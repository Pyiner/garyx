//! Read seam over the SQL channel-endpoint projections.
//!
//! Thread condition queries must go through SQL projections (repository
//! contract): the gateway registers a SQL-backed implementation over
//! `thread_channel_endpoints`, `thread_meta`, and `thread_message_routes`
//! for its `SqliteThreadStore` at bootstrap. Projections derive in the
//! same transaction as every record write, so readers are structurally
//! current — there is no staleness gate and no repair path.
//!
//! Stores without a registered projection (in-memory embedders, unit
//! tests) fall back to [`ScanChannelEndpointProjection`], which answers
//! the same queries by scanning the store. For an in-memory store the
//! whole store already lives in memory, so the scan is the structural
//! equivalent of a projection read; SQLite-backed stores must register
//! the SQL implementation instead.

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use async_trait::async_trait;
use serde_json::Value;

use crate::store::ThreadStore;
use crate::threads::{
    KnownChannelEndpoint, bindings_from_value, is_thread_key, label_from_value, value_updated_at,
    workspace_dir_from_value,
};

/// One persisted delivery context: the projection row behind
/// `thread_meta.last_delivery_context_json`.
#[derive(Debug, Clone)]
pub struct DeliveryContextRow {
    pub thread_id: String,
    pub context_json: String,
    pub updated_at: Option<String>,
}

/// One outbound message-id route: the projection row behind
/// `thread_message_routes`.
#[derive(Debug, Clone)]
pub struct OutboundRouteRow {
    pub thread_id: String,
    /// `None` when the persisted record carried no channel (legacy rows);
    /// callers supply their own fallback.
    pub channel: Option<String>,
    pub account_id: String,
    pub chat_id: String,
    pub thread_binding_key: Option<String>,
    pub message_id: String,
}

#[async_trait]
pub trait ChannelEndpointProjection: Send + Sync {
    /// Thread ids currently holding a channel binding for `endpoint_key`.
    async fn endpoint_holders(&self, endpoint_key: &str) -> Result<Vec<String>, String>;

    /// Every bound endpoint with its holder-thread metadata — one entry
    /// per (endpoint, holder) pair, so duplicate holders stay visible.
    /// Callers wanting one row per endpoint pick their preferred holder
    /// (see `list_known_channel_endpoints`).
    async fn endpoints(&self) -> Result<Vec<KnownChannelEndpoint>, String>;

    /// Every thread with a persisted delivery context.
    async fn delivery_contexts(&self) -> Result<Vec<DeliveryContextRow>, String>;

    /// Every persisted outbound message-id route.
    async fn outbound_routes(&self) -> Result<Vec<OutboundRouteRow>, String>;
}

static CHANNEL_ENDPOINT_PROJECTIONS: OnceLock<
    StdMutex<HashMap<usize, Arc<dyn ChannelEndpointProjection>>>,
> = OnceLock::new();

fn projection_registry() -> &'static StdMutex<HashMap<usize, Arc<dyn ChannelEndpointProjection>>> {
    CHANNEL_ENDPOINT_PROJECTIONS.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn store_id(store: &Arc<dyn ThreadStore>) -> usize {
    Arc::as_ptr(store) as *const () as usize
}

/// Register the SQL projection for a store. The gateway calls this once at
/// bootstrap for its `SqliteThreadStore`.
pub fn register_channel_endpoint_projection(
    store: &Arc<dyn ThreadStore>,
    projection: Arc<dyn ChannelEndpointProjection>,
) {
    let mut registry = projection_registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    registry.insert(store_id(store), projection);
}

pub fn remove_channel_endpoint_projection(store: &Arc<dyn ThreadStore>) {
    let mut registry = projection_registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    registry.remove(&store_id(store));
}

/// The projection registered for this store, or the scan fallback for
/// stores without one (in-memory embedders and unit tests).
pub fn channel_endpoint_projection_for(
    store: &Arc<dyn ThreadStore>,
) -> Arc<dyn ChannelEndpointProjection> {
    let registered = {
        let registry = projection_registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        registry.get(&store_id(store)).cloned()
    };
    registered.unwrap_or_else(|| Arc::new(ScanChannelEndpointProjection::new(store.clone())))
}

/// Scan-backed projection for stores without SQL projections. Answers the
/// same queries by walking the store; only correct for in-memory stores,
/// where the walk is a hash-map iteration.
pub struct ScanChannelEndpointProjection {
    store: Arc<dyn ThreadStore>,
}

impl ScanChannelEndpointProjection {
    pub fn new(store: Arc<dyn ThreadStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl ChannelEndpointProjection for ScanChannelEndpointProjection {
    async fn endpoint_holders(&self, endpoint_key: &str) -> Result<Vec<String>, String> {
        let mut holders = Vec::new();
        let keys = self
            .store
            .list_keys(None)
            .await
            .map_err(|error| error.to_string())?;
        for key in keys {
            if !is_thread_key(&key) {
                continue;
            }
            let Some(value) = self
                .store
                .get(&key)
                .await
                .map_err(|error| error.to_string())?
            else {
                continue;
            };
            if bindings_from_value(&value)
                .iter()
                .any(|binding| binding.endpoint_key() == endpoint_key)
            {
                holders.push(key);
            }
        }
        Ok(holders)
    }

    async fn endpoints(&self) -> Result<Vec<KnownChannelEndpoint>, String> {
        let mut endpoints = Vec::new();
        let keys = self
            .store
            .list_keys(None)
            .await
            .map_err(|error| error.to_string())?;
        for key in keys {
            if !is_thread_key(&key) {
                continue;
            }
            let Some(value) = self
                .store
                .get(&key)
                .await
                .map_err(|error| error.to_string())?
            else {
                continue;
            };
            let updated_at = value_updated_at(&value);
            for binding in bindings_from_value(&value) {
                let endpoint_key = binding.endpoint_key();
                let delivery_target_type = binding.resolved_delivery_target_type();
                let delivery_target_id = binding.resolved_delivery_target_id();
                endpoints.push(KnownChannelEndpoint {
                    endpoint_key,
                    channel: binding.channel,
                    account_id: binding.account_id,
                    binding_key: binding.binding_key,
                    chat_id: binding.chat_id,
                    delivery_target_type,
                    delivery_target_id,
                    display_label: binding.display_label,
                    thread_id: Some(key.clone()),
                    thread_label: label_from_value(&value),
                    workspace_dir: workspace_dir_from_value(&value),
                    thread_updated_at: updated_at.clone(),
                    last_inbound_at: binding.last_inbound_at,
                    last_delivery_at: binding.last_delivery_at,
                });
            }
        }
        Ok(endpoints)
    }

    async fn delivery_contexts(&self) -> Result<Vec<DeliveryContextRow>, String> {
        let mut rows = Vec::new();
        let keys = self
            .store
            .list_keys(None)
            .await
            .map_err(|error| error.to_string())?;
        for key in keys {
            let Some(value) = self
                .store
                .get(&key)
                .await
                .map_err(|error| error.to_string())?
            else {
                continue;
            };
            let Some(obj) = value.as_object() else {
                continue;
            };
            let Some(context) = crate::MessageRouter::extract_delivery_context(obj) else {
                continue;
            };
            let Ok(context_json) = serde_json::to_string(&context) else {
                continue;
            };
            let updated_at = string_field(obj, "lastUpdatedAt")
                .or_else(|| string_field(obj, "updated_at"))
                .or_else(|| string_field(obj, "last_updated_at"));
            rows.push(DeliveryContextRow {
                thread_id: key,
                context_json,
                updated_at,
            });
        }
        Ok(rows)
    }

    async fn outbound_routes(&self) -> Result<Vec<OutboundRouteRow>, String> {
        let mut rows = Vec::new();
        let keys = self
            .store
            .list_keys(None)
            .await
            .map_err(|error| error.to_string())?;
        for key in keys {
            let Some(value) = self
                .store
                .get(&key)
                .await
                .map_err(|error| error.to_string())?
            else {
                continue;
            };
            let Some(records) = value.get("outbound_message_ids").and_then(Value::as_array) else {
                continue;
            };
            for record in records {
                let Some(obj) = record.as_object() else {
                    continue;
                };
                let Some(message_id) = obj.get("message_id").and_then(Value::as_str) else {
                    continue;
                };
                rows.push(OutboundRouteRow {
                    thread_id: key.clone(),
                    channel: obj
                        .get("channel")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    account_id: obj
                        .get("account_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    chat_id: obj
                        .get("chat_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    thread_binding_key: obj
                        .get("thread_binding_key")
                        .or_else(|| obj.get("thread_scope"))
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    message_id: message_id.to_owned(),
                });
            }
        }
        Ok(rows)
    }
}

fn string_field(obj: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}
