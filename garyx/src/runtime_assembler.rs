use std::path::{Path, PathBuf};
use std::sync::Arc;

use garyx_bridge::MultiProviderBridge;
use garyx_gateway::server::{AppState, AppStateBuilder};
use garyx_gateway::{CronService, ThreadFileLogger, default_thread_log_dir};
use garyx_models::config::GaryxConfig;
use garyx_models::local_paths::{
    conversation_index_db_path_for_data_dir, default_session_data_dir,
    message_ledger_dir_for_data_dir, thread_transcripts_dir_for_data_dir,
};
use garyx_router::{
    ConversationIndexManager, FileThreadStore, InMemoryThreadStore, MessageLedgerStore,
    ThreadHistoryRepository, ThreadStore, ThreadTranscriptStore, is_thread_key,
    workspace_dir_from_value,
};
use std::collections::HashMap;

pub struct RuntimeAssembly {
    pub state: Arc<AppState>,
    pub bridge: Arc<MultiProviderBridge>,
    pub cron_service: Arc<CronService>,
}

pub struct RuntimeAssembler {
    config_path: PathBuf,
    config: GaryxConfig,
}

impl RuntimeAssembler {
    pub fn new(config_path: impl AsRef<Path>, config: GaryxConfig) -> Self {
        Self {
            config_path: config_path.as_ref().to_path_buf(),
            config,
        }
    }

    pub async fn assemble(self) -> Result<RuntimeAssembly, Box<dyn std::error::Error>> {
        let session_data_dir = self
            .config
            .sessions
            .data_dir
            .clone()
            .unwrap_or_else(|| default_session_data_dir().to_string_lossy().to_string());
        let thread_store: Arc<dyn ThreadStore> = match FileThreadStore::new(&session_data_dir).await
        {
            Ok(store) => {
                tracing::info!(data_dir = %session_data_dir, "FileThreadStore initialized");
                Arc::new(store)
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "Failed to create FileThreadStore, falling back to in-memory"
                );
                Arc::new(InMemoryThreadStore::new())
            }
        };
        let transcript_root = thread_transcripts_dir_for_data_dir(Path::new(&session_data_dir));
        let transcript_store = Arc::new(ThreadTranscriptStore::file(&transcript_root).await?);
        let message_ledger = Arc::new(
            MessageLedgerStore::file(message_ledger_dir_for_data_dir(Path::new(
                &session_data_dir,
            )))
            .await?,
        );
        let conversation_index = match ConversationIndexManager::new(
            thread_store.clone(),
            transcript_store.clone(),
            conversation_index_db_path_for_data_dir(Path::new(&session_data_dir)),
            self.config.gateway.conversation_index.clone(),
        )
        .await
        {
            Ok(index) => Some(index),
            Err(error) => {
                tracing::warn!(error = %error, "Conversation index init failed, continuing without vector recall");
                None
            }
        };
        let mut thread_history =
            ThreadHistoryRepository::new(thread_store.clone(), transcript_store);
        if let Some(conversation_index) = conversation_index {
            thread_history = thread_history.with_conversation_index(conversation_index);
        }
        let thread_history = Arc::new(thread_history);

        let bridge = Arc::new(MultiProviderBridge::new());
        match bridge.initialize_from_config(&self.config).await {
            Ok(()) => tracing::info!("MultiProviderBridge initialized"),
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "Bridge init failed, starting with empty provider pool"
                );
            }
        }

        let (event_tx, _) = tokio::sync::broadcast::channel(128);

        let mut cron_service_raw = CronService::new(PathBuf::from(&session_data_dir));
        cron_service_raw.set_event_tx(event_tx.clone());
        let cron_boot_config = self.config.cron.clone();
        match cron_service_raw.load(&cron_boot_config).await {
            Ok(()) => {
                cron_service_raw.start();
                tracing::info!(
                    configured_jobs = cron_boot_config.jobs.len(),
                    "Cron service started"
                );
            }
            Err(error) => {
                tracing::warn!(error = %error, "Failed to initialize cron service");
            }
        }
        let cron_service = Arc::new(cron_service_raw);

        let restart_tokens = parse_restart_tokens_from_env();
        if !restart_tokens.is_empty() {
            tracing::info!(
                count = restart_tokens.len(),
                "Restart auth tokens loaded from GARYX_RESTART_TOKENS"
            );
        }
        let thread_logs = Arc::new(ThreadFileLogger::new(default_thread_log_dir()));

        let state = AppStateBuilder::new(self.config.clone())
            .with_thread_store(thread_store.clone())
            .with_thread_history(thread_history.clone())
            .with_message_ledger(message_ledger)
            .with_bridge(bridge.clone())
            .with_event_tx(event_tx)
            .with_cron_service(cron_service.clone())
            .with_config_path(self.config_path)
            .with_restart_tokens(restart_tokens)
            .with_thread_log_sink(thread_logs.clone())
            .build();

        bridge
            .replace_agent_profiles(state.ops.custom_agents.list_agents().await)
            .await;
        bridge
            .replace_team_profiles(state.ops.agent_teams.list_teams().await)
            .await;
        if let Err(error) = bridge.reload_from_config(&self.config).await {
            tracing::warn!(
                error = %error,
                "Bridge reload after agent-profile sync failed"
            );
        }

        rebuild_routing_caches(&state, &self.config).await;

        bridge.set_thread_store(thread_store).await;
        bridge.set_thread_history(thread_history);
        bridge.set_event_tx(state.ops.events.sender()).await;
        bridge
            .replace_thread_workspace_bindings(
                collect_thread_workspace_bindings(&state.threads.thread_store).await,
            )
            .await;
        cron_service
            .set_dispatch_runtime(
                state.threads.thread_store.clone(),
                state.threads.router.clone(),
                bridge.clone(),
                state.channel_dispatcher(),
                state.ops.thread_logs.clone(),
                self.config.mcp_servers.clone(),
                state.ops.custom_agents.clone(),
                state.ops.agent_teams.clone(),
            )
            .await;
        state
            .threads
            .history
            .schedule_full_conversation_index_backfill();

        Ok(RuntimeAssembly {
            state,
            bridge,
            cron_service,
        })
    }
}

fn parse_restart_tokens_from_env() -> Vec<String> {
    std::env::var("GARYX_RESTART_TOKENS")
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<String>>()
        })
        .unwrap_or_default()
}

async fn rebuild_routing_caches(state: &Arc<AppState>, config: &GaryxConfig) {
    let mut router = state.threads.router.lock().await;
    let rebuild_channels = routing_rebuild_channels(config);
    let thread_index_stats = router.rebuild_thread_indexes().await;
    let mut routing_entries = 0usize;
    for channel in &rebuild_channels {
        routing_entries += router.rebuild_routing_index(channel).await;
    }
    let delivery_entries = router.rebuild_last_delivery_cache().await;
    tracing::info!(
        channels = ?rebuild_channels,
        endpoint_bindings = thread_index_stats.endpoint_bindings,
        routing_entries,
        delivery_entries,
        "Rebuilt thread, routing, and delivery caches from thread store"
    );
}

async fn collect_thread_workspace_bindings(
    store: &Arc<dyn ThreadStore>,
) -> HashMap<String, String> {
    let mut bindings = HashMap::new();
    for key in store.list_keys(None).await {
        if !is_thread_key(&key) {
            continue;
        }
        let Some(value) = store.get(&key).await else {
            continue;
        };
        let Some(workspace_dir) = workspace_dir_from_value(&value) else {
            continue;
        };
        bindings.insert(key, workspace_dir);
    }
    bindings
}

pub(crate) fn routing_rebuild_channels(config: &GaryxConfig) -> Vec<String> {
    let mut channels: Vec<String> = Vec::new();

    if config.channels.api.accounts.values().any(|acc| acc.enabled) {
        channels.push("api".to_owned());
    }
    for (plugin_id, plugin_cfg) in &config.channels.plugins {
        if plugin_cfg.accounts.values().any(|entry| entry.enabled) {
            channels.push(plugin_id.clone());
        }
    }

    channels
}
