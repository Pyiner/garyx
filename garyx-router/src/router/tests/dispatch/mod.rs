use super::*;
use crate::memory_store::InMemoryThreadStore;
use async_trait::async_trait;
use garyx_models::config::SlashCommand;
use garyx_models::provider::ProviderType;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

mod auto_recovery;
mod route_and_dispatch;

type DispatchedRun = (String, String, Option<Vec<ImagePayload>>);

struct MockDispatcher {
    dispatched: tokio::sync::Mutex<Vec<DispatchedRun>>,
    metadata: tokio::sync::Mutex<Vec<HashMap<String, Value>>>,
    workspace_dirs: tokio::sync::Mutex<Vec<Option<String>>>,
    requested_providers: tokio::sync::Mutex<Vec<Option<ProviderType>>>,
    should_fail: bool,
}

impl MockDispatcher {
    fn new() -> Self {
        Self {
            dispatched: tokio::sync::Mutex::new(Vec::new()),
            metadata: tokio::sync::Mutex::new(Vec::new()),
            workspace_dirs: tokio::sync::Mutex::new(Vec::new()),
            requested_providers: tokio::sync::Mutex::new(Vec::new()),
            should_fail: false,
        }
    }

    fn failing() -> Self {
        Self {
            dispatched: tokio::sync::Mutex::new(Vec::new()),
            metadata: tokio::sync::Mutex::new(Vec::new()),
            workspace_dirs: tokio::sync::Mutex::new(Vec::new()),
            requested_providers: tokio::sync::Mutex::new(Vec::new()),
            should_fail: true,
        }
    }
}

#[async_trait]
impl AgentDispatcher for MockDispatcher {
    async fn dispatch(
        &self,
        run: crate::AdmittedRun,
        _response_callback: Option<Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    ) -> Result<garyx_models::provider::AgentDispatchOutcome, String> {
        let (request, _lease) = run.into_dispatch_parts();
        if self.should_fail {
            return Err("mock dispatch failure".to_owned());
        }
        self.dispatched
            .lock()
            .await
            .push((request.thread_id, request.message, request.images));
        self.metadata.lock().await.push(request.metadata);
        self.workspace_dirs.lock().await.push(request.workspace_dir);
        self.requested_providers
            .lock()
            .await
            .push(request.requested_provider);
        Ok(garyx_models::provider::AgentDispatchOutcome::Started)
    }
}
