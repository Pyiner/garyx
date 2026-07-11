use std::path::{Path, PathBuf};
use std::sync::Arc;

use garyx_bridge::MultiProviderBridge;
use garyx_gateway::server::{AppState, AppStateBuilder};
use garyx_gateway::{CronService, ThreadFileLogger, default_thread_log_dir};
use garyx_models::config::GaryxConfig;
use garyx_models::local_paths::{
    default_session_data_dir, message_ledger_dir_for_data_dir, thread_transcripts_dir_for_data_dir,
};
use garyx_router::{
    FileThreadStore, InMemoryThreadStore, MessageLedgerStore, ThreadHistoryRepository, ThreadStore,
    ThreadTranscriptStore,
};

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
        let file_store: Arc<dyn ThreadStore> = match FileThreadStore::new(&session_data_dir).await
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

        let bridge = Arc::new(MultiProviderBridge::new());

        // Thread records live in SQLite, full stop (#TASK-1864). The file
        // archive survives only as the one-shot boot-import source for
        // upgrades from pre-SQLite installs; there is no runtime file mode
        // and no dual-write mirror. Emergency recovery = the archived
        // backups plus a fresh boot import, not a mode switch.
        let garyx_db = Arc::new(garyx_gateway::garyx_db::GaryxDbService::open(
            garyx_models::local_paths::garyx_database_path_for_data_dir(Path::new(
                &session_data_dir,
            )),
        )?);
        let assembled_garyx_db = Some(garyx_db.clone());
        tracing::info!("thread store backend: sqlite");
        let thread_store: Arc<dyn ThreadStore> = garyx_gateway::assemble_sqlite_thread_store(
            garyx_db,
            transcript_store.clone(),
            &bridge,
            file_store.clone(),
        )
        .await?;
        let thread_history = Arc::new(ThreadHistoryRepository::new(
            thread_store.clone(),
            transcript_store,
        ));

        let (event_tx, _) = tokio::sync::broadcast::channel(128);

        let mut cron_service_raw = CronService::new(PathBuf::from(&session_data_dir));
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

        let mut builder = AppStateBuilder::new(self.config.clone()).with_persistent_local_stores();
        if let Some(garyx_db) = assembled_garyx_db {
            // The sqlite thread-store backend already opened the garyx
            // database; share that instance instead of letting the builder
            // open a second one (single-writer discipline, D4).
            builder = builder.with_garyx_db(garyx_db);
        }
        let state = builder
            .with_thread_store(thread_store.clone())
            .with_thread_history(thread_history.clone())
            .with_message_ledger(message_ledger)
            .with_bridge(bridge.clone())
            .with_provider_runtime_ready(false)
            .with_event_tx(event_tx)
            .with_cron_service(cron_service.clone())
            .with_config_path(self.config_path)
            .with_restart_tokens(restart_tokens)
            .with_thread_log_sink(thread_logs.clone())
            .build();

        // AppStateBuilder wraps the raw file store in the recent-thread
        // projecting store. Bind the bridge to that final store so provider
        // persistence updates the projection instead of bypassing it.
        bridge
            .set_thread_store(state.threads.thread_store.clone())
            .await;
        bridge.set_thread_history(state.threads.history.clone());
        bridge.set_event_tx(state.ops.events.sender()).await;
        cron_service
            .set_dispatch_runtime(
                state.threads.thread_store.clone(),
                state.threads.router.clone(),
                bridge.clone(),
                state.channel_dispatcher(),
                state.ops.thread_logs.clone(),
                self.config.mcp_servers.clone(),
                state.ops.custom_agents.clone(),
            )
            .await;
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

#[cfg(test)]
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
