//! SQL implementation of the router's channel-endpoint projection seam.
//!
//! Answers endpoint condition queries from the projection tables
//! (`thread_channel_endpoints`, `thread_meta`, `thread_message_routes`),
//! which derive in the same transaction as every record write — so the
//! answers are structurally current and no store scan is ever needed.

use std::sync::Arc;

use async_trait::async_trait;
use garyx_router::{
    ChannelEndpointProjection, DeliveryContextRow, KnownChannelEndpoint, OutboundRouteRow,
};

use crate::garyx_db::GaryxDbService;

pub(crate) struct SqlChannelEndpointProjection {
    garyx_db: Arc<GaryxDbService>,
}

impl SqlChannelEndpointProjection {
    pub(crate) fn new(garyx_db: Arc<GaryxDbService>) -> Self {
        Self { garyx_db }
    }
}

#[async_trait]
impl ChannelEndpointProjection for SqlChannelEndpointProjection {
    async fn endpoint_holders(&self, endpoint_key: &str) -> Result<Vec<String>, String> {
        let endpoint_key = endpoint_key.to_owned();
        self.garyx_db
            .run_blocking(move |db| db.thread_ids_for_channel_endpoint(&endpoint_key))
            .await
            .map_err(|error| error.to_string())
    }

    async fn endpoints(&self) -> Result<Vec<KnownChannelEndpoint>, String> {
        self.garyx_db
            .run_blocking(|db| db.list_thread_channel_endpoints())
            .await
            .map_err(|error| error.to_string())
    }

    async fn delivery_contexts(&self) -> Result<Vec<DeliveryContextRow>, String> {
        self.garyx_db
            .run_blocking(|db| db.list_thread_delivery_contexts())
            .await
            .map(|rows| {
                rows.into_iter()
                    .map(|(thread_id, context_json, updated_at)| DeliveryContextRow {
                        thread_id,
                        context_json,
                        updated_at,
                    })
                    .collect()
            })
            .map_err(|error| error.to_string())
    }

    async fn outbound_routes(&self) -> Result<Vec<OutboundRouteRow>, String> {
        self.garyx_db
            .run_blocking(|db| db.list_thread_message_routes())
            .await
            .map(|rows| {
                rows.into_iter()
                    .map(|row| OutboundRouteRow {
                        thread_id: row.thread_id,
                        channel: Some(row.channel),
                        account_id: row.account_id,
                        chat_id: row.chat_id,
                        thread_binding_key: row.thread_binding_key,
                        message_id: row.message_id,
                    })
                    .collect()
            })
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use garyx_router::{ThreadStore, channel_endpoint_projection_for};
    use serde_json::json;

    use crate::composition::app_bootstrap::AppStateBuilder;
    use crate::server::AppState;

    fn state() -> Arc<AppState> {
        AppStateBuilder::new(garyx_models::GaryxConfig::default()).build()
    }

    fn bound_thread_record(thread_id: &str, chat_id: &str) -> serde_json::Value {
        json!({
            "thread_id": thread_id,
            "label": "Bound",
            "updated_at": "2026-07-01T00:00:00.000Z",
            "channel_bindings": [{
                "channel": "telegram",
                "account_id": "main",
                "binding_key": chat_id,
                "chat_id": chat_id,
            }],
            "outbound_message_ids": [{
                "channel": "telegram",
                "account_id": "main",
                "chat_id": chat_id,
                "message_id": "900",
            }],
            "delivery_context": {
                "channel": "telegram",
                "account_id": "main",
                "chat_id": chat_id,
                "user_id": chat_id,
            },
        })
    }

    /// End-to-end over the real write path: a record write derives the
    /// endpoint projections in the same transaction, and every endpoint
    /// condition query is answered by SQL — no store scan.
    #[tokio::test]
    async fn sql_endpoint_projection_answers_condition_queries_from_record_writes() {
        let state = state();
        let store: Arc<dyn ThreadStore> = state.threads.thread_store.clone();
        store
            .set("thread::bound", bound_thread_record("thread::bound", "42"))
            .await
            .unwrap();

        let projection = channel_endpoint_projection_for(&store);
        assert_eq!(
            projection
                .endpoint_holders("telegram::main::42")
                .await
                .expect("holders"),
            vec!["thread::bound".to_owned()],
        );

        let endpoints = projection.endpoints().await.expect("endpoints");
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].endpoint_key, "telegram::main::42");
        assert_eq!(endpoints[0].thread_id.as_deref(), Some("thread::bound"));

        let contexts = projection
            .delivery_contexts()
            .await
            .expect("delivery contexts");
        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0].thread_id, "thread::bound");
        assert!(contexts[0].context_json.contains("\"chat_id\":\"42\""));

        let routes = projection.outbound_routes().await.expect("routes");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].message_id, "900");
        assert_eq!(routes[0].channel.as_deref(), Some("telegram"));

        // Deleting the record removes the projection rows with it.
        store.delete("thread::bound").await.unwrap();
        assert!(
            projection
                .endpoint_holders("telegram::main::42")
                .await
                .expect("holders after delete")
                .is_empty()
        );
    }

    /// Moving a binding between threads goes through the projection to find
    /// the previous holder and strips the binding from that record only.
    #[tokio::test]
    async fn bind_endpoint_moves_binding_via_projection_holder_lookup() {
        let state = state();
        let store: Arc<dyn ThreadStore> = state.threads.thread_store.clone();
        store
            .set("thread::old", bound_thread_record("thread::old", "42"))
            .await
            .unwrap();
        store
            .set("thread::new", json!({ "thread_id": "thread::new" }))
            .await
            .unwrap();

        let binding = garyx_router::ChannelBinding {
            channel: "telegram".to_owned(),
            account_id: "main".to_owned(),
            binding_key: "42".to_owned(),
            chat_id: "42".to_owned(),
            ..Default::default()
        };
        let previous = garyx_router::bind_endpoint_to_thread(&store, "thread::new", binding)
            .await
            .expect("bind");
        assert_eq!(previous.as_deref(), Some("thread::old"));

        let projection = channel_endpoint_projection_for(&store);
        assert_eq!(
            projection
                .endpoint_holders("telegram::main::42")
                .await
                .expect("holders"),
            vec!["thread::new".to_owned()],
        );
        let old_record = store.get("thread::old").await.unwrap().expect("old record");
        assert!(garyx_router::bindings_from_value(&old_record).is_empty());
    }

    /// Injected non-SQL stores must resolve to the scan projection over
    /// the SAME store — never to an unrelated SQL database (#TASK-2099
    /// root review finding 4). The projection accessor lives on the store
    /// itself, so there is no pointer-keyed registry to leak or mismatch.
    #[tokio::test]
    async fn injected_in_memory_store_resolves_scan_projection_over_itself() {
        let injected: Arc<dyn ThreadStore> = Arc::new(garyx_router::InMemoryThreadStore::new());
        let state = AppStateBuilder::new(garyx_models::GaryxConfig::default())
            .with_thread_store(injected.clone())
            .build();
        let store = state.threads.thread_store.clone();
        store
            .set("thread::bound", bound_thread_record("thread::bound", "42"))
            .await
            .unwrap();

        let projection = channel_endpoint_projection_for(&store);
        assert_eq!(
            projection
                .endpoint_holders("telegram::main::42")
                .await
                .expect("holders on injected store"),
            vec!["thread::bound".to_owned()],
            "the injected store must be answered by its own scan projection"
        );

        // Task condition queries fall back to the scan reader the same way.
        let service = garyx_router::TaskService::new(
            store.clone(),
            Arc::new(garyx_router::InMemoryTaskCounterStore::new()),
        );
        let (thread_id, task) = service
            .create_task(garyx_router::CreateTaskInput {
                title: Some("Injected".to_owned()),
                body: None,
                assignee: None,
                notification_target: None,
                source: None,
                executor: None,
                start: false,
                actor: None,
                workspace_dir: None,
                runtime: None,
            })
            .await
            .expect("create task on injected store");
        let (resolved_thread_id, _, resolved) = service
            .get_task(&format!("#TASK-{}", task.number))
            .await
            .expect("resolve task by number over the scan reader");
        assert_eq!(resolved_thread_id, thread_id);
        assert_eq!(resolved.number, task.number);
    }

    /// Two thread records holding the SAME endpoint (legacy import or a
    /// mid-bind race) must both be visible as holders and both be stripped
    /// on the next bind — the projection stores one row per holder
    /// (#TASK-2107 P1 regression).
    #[tokio::test]
    async fn duplicate_endpoint_holders_are_all_visible_and_all_removed_on_bind() {
        let state = state();
        let store: Arc<dyn ThreadStore> = state.threads.thread_store.clone();
        store
            .set("thread::old-a", bound_thread_record("thread::old-a", "42"))
            .await
            .unwrap();
        store
            .set("thread::old-b", bound_thread_record("thread::old-b", "42"))
            .await
            .unwrap();
        store
            .set("thread::new", json!({ "thread_id": "thread::new" }))
            .await
            .unwrap();

        let projection = channel_endpoint_projection_for(&store);
        let mut holders = projection
            .endpoint_holders("telegram::main::42")
            .await
            .expect("holders before bind");
        holders.sort();
        assert_eq!(
            holders,
            vec!["thread::old-a".to_owned(), "thread::old-b".to_owned()],
            "both duplicate holders must be visible in the projection"
        );

        let binding = garyx_router::ChannelBinding {
            channel: "telegram".to_owned(),
            account_id: "main".to_owned(),
            binding_key: "42".to_owned(),
            chat_id: "42".to_owned(),
            ..Default::default()
        };
        garyx_router::bind_endpoint_to_thread(&store, "thread::new", binding)
            .await
            .expect("bind");

        for old_thread in ["thread::old-a", "thread::old-b"] {
            let record = store.get(old_thread).await.unwrap().expect("old record");
            assert!(
                garyx_router::bindings_from_value(&record).is_empty(),
                "{old_thread} still holds a duplicate binding"
            );
        }
        assert_eq!(
            projection
                .endpoint_holders("telegram::main::42")
                .await
                .expect("holders after bind"),
            vec!["thread::new".to_owned()],
        );
    }
}
