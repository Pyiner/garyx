//! Read seam over the SQL channel-endpoint projections.
//!
//! Thread condition queries must go through SQL projections (repository
//! contract): `SqliteThreadStore` exposes a SQL-backed implementation over
//! `thread_channel_endpoints` and `thread_meta`
//! through [`crate::ThreadStore::channel_endpoint_projection`]. Projections
//! derive in the same transaction as every record write, so readers are
//! structurally current — there is no staleness gate and no repair path.
//!
//! Stores without their own projection (in-memory embedders, unit tests)
//! fall back to [`ScanChannelEndpointProjection`], which answers the same
//! queries by scanning the store. For an in-memory store the whole store
//! already lives in memory, so the scan is the structural equivalent of a
//! projection read.

use std::sync::Arc;

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

#[async_trait]
pub trait ChannelEndpointProjection: Send + Sync {
    /// Every bound endpoint with its holder-thread metadata. The
    /// endpoint table holds one row per endpoint (single-owner model;
    /// duplicates in legacy record bodies are settled by the one-shot
    /// holder dedup migration).
    async fn endpoints(&self) -> Result<Vec<KnownChannelEndpoint>, String>;

    /// Point lookup of one endpoint's owner entry — the same truth
    /// source as [`Self::endpoints`], narrowed to a single key. The SQL
    /// projection overrides this with an indexed point query; the scan
    /// fallback reduces duplicates (legacy record bodies) to the
    /// preferred holder.
    async fn endpoint_owner(
        &self,
        endpoint_key: &str,
    ) -> Result<Option<KnownChannelEndpoint>, String> {
        Ok(self
            .endpoints()
            .await?
            .into_iter()
            .filter(|candidate| candidate.endpoint_key == endpoint_key)
            .reduce(|current, candidate| {
                if crate::threads::is_preferred_thread_binding(
                    candidate.thread_id.as_deref().unwrap_or_default(),
                    candidate.thread_updated_at.as_deref(),
                    current.thread_id.as_deref().unwrap_or_default(),
                    current.thread_updated_at.as_deref(),
                ) {
                    candidate
                } else {
                    current
                }
            }))
    }

    /// Every thread with a persisted delivery context.
    async fn delivery_contexts(&self) -> Result<Vec<DeliveryContextRow>, String>;
}

/// The projection for this store: the store's own SQL projection when the
/// backend maintains one (SQLite), else the scan fallback — the structural
/// equivalent for in-memory stores. The projection's lifetime is tied to
/// the store; there is no process-global registry.
pub fn channel_endpoint_projection_for(
    store: &Arc<dyn ThreadStore>,
) -> Arc<dyn ChannelEndpointProjection> {
    store
        .channel_endpoint_projection()
        .unwrap_or_else(|| Arc::new(ScanChannelEndpointProjection::new(store.clone())))
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
}

fn string_field(obj: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}
