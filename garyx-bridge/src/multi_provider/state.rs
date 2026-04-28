use std::collections::HashMap;
use std::sync::{Arc, RwLock as StdRwLock};

use garyx_models::thread_logs::ThreadLogSink;
use garyx_models::{AgentTeamProfile, CustomAgentProfile};
use garyx_router::{ThreadHistoryRepository, ThreadStore};
use tokio::sync::{Mutex, RwLock, Semaphore, broadcast, mpsc};
use tokio::task::JoinHandle;

use super::persistence::ThreadPersistenceCommand;
use crate::provider_trait::{AgentLoopProvider, ProviderHealth};

#[derive(Clone)]
pub(super) struct ActiveThreadPersistence {
    pub(super) run_id: String,
    pub(super) tx: mpsc::UnboundedSender<ThreadPersistenceCommand>,
}

/// Shared inner state (cheaply cloneable via Arc).
#[derive(Clone)]
pub(super) struct Inner {
    /// Provider topology and route table (lower churn, shared by most reads).
    pub(super) topology: Arc<RwLock<BridgeTopologyState>>,
    /// Thread-level provider affinity (`thread_id -> provider_key`).
    pub(super) thread_affinity: Arc<RwLock<HashMap<String, String>>>,
    /// Thread-level workspace binding (`thread_id -> workspace_dir`).
    pub(super) thread_workspace_bindings: Arc<RwLock<HashMap<String, String>>>,
    /// Known standalone agent profiles (`agent_id -> profile`).
    pub(super) agent_profiles: Arc<RwLock<HashMap<String, CustomAgentProfile>>>,
    /// Known team profiles (`team_id -> profile`).
    pub(super) team_profiles: Arc<RwLock<HashMap<String, AgentTeamProfile>>>,
    /// Run lifecycle indexes.
    pub(super) run_index: Arc<RwLock<BridgeRunIndex>>,
    /// `run_id -> JoinHandle`
    pub(super) active_tasks: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
    /// Per-thread startup guards to serialize run-vs-queue decisions.
    pub(super) thread_dispatch_guards: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    /// `thread_id -> active streaming persistence handle`
    pub(super) active_thread_persistence: Arc<Mutex<HashMap<String, ActiveThreadPersistence>>>,
    /// Optional thread store for persisting messages after runs.
    pub(super) thread_store: Arc<RwLock<Option<Arc<dyn ThreadStore>>>>,
    /// Optional thread history repository for transcript-backed persistence.
    pub(super) thread_history: Arc<RwLock<Option<Arc<ThreadHistoryRepository>>>>,
    /// Optional broadcast channel for SSE events.
    pub(super) event_tx: Arc<RwLock<Option<broadcast::Sender<String>>>>,
    /// Optional thread log sink for per-thread lifecycle logs.
    pub(super) thread_logs: Arc<StdRwLock<Option<Arc<dyn ThreadLogSink>>>>,
    /// Global admission control for run fan-out.
    pub(super) run_limiter: Arc<Semaphore>,
    pub(super) max_concurrent_runs: usize,
}

impl Inner {
    pub(super) fn new(max_concurrent_runs: usize) -> Self {
        let limit = max_concurrent_runs.max(1);
        Self {
            topology: Arc::new(RwLock::new(BridgeTopologyState::default())),
            thread_affinity: Arc::new(RwLock::new(HashMap::new())),
            thread_workspace_bindings: Arc::new(RwLock::new(HashMap::new())),
            agent_profiles: Arc::new(RwLock::new(HashMap::new())),
            team_profiles: Arc::new(RwLock::new(HashMap::new())),
            run_index: Arc::new(RwLock::new(BridgeRunIndex::default())),
            active_tasks: Arc::new(Mutex::new(HashMap::new())),
            thread_dispatch_guards: Arc::new(Mutex::new(HashMap::new())),
            active_thread_persistence: Arc::new(Mutex::new(HashMap::new())),
            thread_store: Arc::new(RwLock::new(None)),
            thread_history: Arc::new(RwLock::new(None)),
            event_tx: Arc::new(RwLock::new(None)),
            thread_logs: Arc::new(StdRwLock::new(None)),
            run_limiter: Arc::new(Semaphore::new(limit)),
            max_concurrent_runs: limit,
        }
    }
}

#[derive(Default)]
pub(super) struct BridgeTopologyState {
    /// `provider_key -> provider instance`
    pub(super) provider_pool: HashMap<String, Arc<dyn AgentLoopProvider>>,
    /// `(channel, account_id) -> provider_key`
    pub(super) route_cache: HashMap<(String, String), String>,
    /// Default provider key (set during initialize).
    pub(super) default_provider_key: Option<String>,
    /// Per-provider health tracking.
    pub(super) provider_health: HashMap<String, ProviderHealth>,
}

#[derive(Default)]
pub(super) struct BridgeRunIndex {
    /// `run_id -> provider_key`
    pub(super) active_runs: HashMap<String, String>,
    /// `run_id -> thread_id`
    pub(super) run_sessions: HashMap<String, String>,
}

pub(super) fn default_max_concurrent_runs() -> usize {
    const DEFAULT: usize = 32;
    std::env::var("GARYX_BRIDGE_MAX_CONCURRENT_RUNS")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT)
}
