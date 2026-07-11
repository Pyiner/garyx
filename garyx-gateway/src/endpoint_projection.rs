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
    /// Holder thread ids for one endpoint, derived from the public
    /// listing (the dedicated point lookup was retired with its last
    /// production consumer).
    async fn holders_of(
        projection: &std::sync::Arc<dyn garyx_router::ChannelEndpointProjection>,
        endpoint_key: &str,
    ) -> Vec<String> {
        projection
            .endpoints()
            .await
            .expect("endpoints listing")
            .into_iter()
            .filter(|endpoint| endpoint.endpoint_key == endpoint_key)
            .filter_map(|endpoint| endpoint.thread_id)
            .collect()
    }

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
            holders_of(&projection, "telegram::main::42").await,
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
            holders_of(&projection, "telegram::main::42")
                .await
                .is_empty()
        );
    }

    /// Moving a binding between threads goes through the production
    /// EndpointBindingMutator; the projection follows the record writes in
    /// the same transaction, so the new holder is the only one visible.
    #[tokio::test]
    async fn bind_endpoint_moves_binding_and_projection_follows() {
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
        let mutator = crate::endpoint_binding_mutator::SqlEndpointBindingMutator::new(
            store.clone(),
            state.ops.garyx_db.clone(),
        );
        let result =
            garyx_router::EndpointBindingMutator::bind_endpoint(&mutator, "thread::new", binding)
                .await
                .expect("bind");
        assert_eq!(result.previous_thread_id.as_deref(), Some("thread::old"));

        let projection = channel_endpoint_projection_for(&store);
        assert_eq!(
            holders_of(&projection, "telegram::main::42").await,
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
            holders_of(&projection, "telegram::main::42").await,
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
}
