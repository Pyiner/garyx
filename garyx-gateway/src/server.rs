use axum::Router;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tracing;

use crate::server_lifecycle::start_gateway_runtime;

/// Post-signal drain deadline. Axum's graceful shutdown waits for every
/// in-flight connection to finish, but the gateway serves infinite SSE
/// streams (`/api/events`, per-thread streams) that never finish on their
/// own — without a deadline SIGTERM hangs until launchd's SIGKILL (every
/// observed shutdown was force-killed). After the signal, real
/// request/response traffic gets this window to complete; remaining
/// connections are dropped on process exit. SSE clients are
/// reconnect-driven, so a dropped stream is an ordinary resume for them.
const GRACEFUL_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

pub use crate::app_bootstrap::{AppStateBuilder, create_app_state, create_app_state_with_bridge};
pub use crate::app_state::{AppState, IntegrationState, OpsState, RuntimeState, ThreadState};
pub use crate::automation_wiring::start_automation_scheduler;
pub use crate::event_stream_hub::EventStreamHub;
pub use crate::mcp_metrics::{
    McpToolCallCount, McpToolDurationStat, McpToolMetrics, McpToolMetricsSnapshot,
};
use crate::route_graph;
pub use crate::runtime_cells::{ChannelDispatcherCell, HotSwapCell, LiveConfigCell};

/// The axum HTTP gateway server.
pub struct Gateway {
    state: Arc<AppState>,
    router: Router,
}

impl Gateway {
    /// Create a new gateway from application state.
    pub fn new(state: Arc<AppState>) -> Self {
        let router = route_graph::build_router(state.clone());

        Self { state, router }
    }

    /// Serve the gateway, blocking until `shutdown_signal` fires.
    pub async fn serve(self, addr: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
        self.serve_with_lifecycle_hooks(addr, || {}, || {}).await
    }

    /// Serve the gateway and invoke `on_listening` immediately after the TCP
    /// listener is bound. This lets the CLI boot path keep non-critical
    /// provider/channel/plugin startup out of the port-listening critical path.
    pub async fn serve_with_listening_hook(
        self,
        addr: SocketAddr,
        on_listening: impl FnOnce() + Send + 'static,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.serve_with_lifecycle_hooks(addr, on_listening, || {})
            .await
    }

    /// Serve the gateway with hooks for the two process lifecycle boundaries
    /// the CLI needs to coordinate with deferred startup work.
    pub async fn serve_with_lifecycle_hooks(
        self,
        addr: SocketAddr,
        on_listening: impl FnOnce() + Send + 'static,
        on_shutdown_started: impl FnOnce() + Send + 'static,
    ) -> Result<(), Box<dyn std::error::Error>> {
        start_gateway_runtime(self.state.clone());
        let listener = tokio::net::TcpListener::bind(addr).await?;
        self.state.spawn_gateway_sync_cache_warmup();
        tracing::info!("Gateway listening on {}", addr);
        on_listening();

        let shutdown_state = self.state.clone();
        let meeting_shutdown_state = self.state.clone();
        let (drain_started_tx, drain_started_rx) = tokio::sync::oneshot::channel();
        let shutdown = async move {
            shutdown_signal().await;
            meeting_shutdown_state.ops.meetings.shutdown_ingestion();
            on_shutdown_started();
            let _ = drain_started_tx.send(());
        };
        let serve = axum::serve(
            listener,
            self.router
                .into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown);
        serve_with_bounded_drain(serve, drain_started_rx, GRACEFUL_DRAIN_TIMEOUT).await?;

        // On graceful shutdown, abort in-flight runs so a restart does not leave
        // orphaned `running` projections behind. Bounded so a stuck abort cannot
        // hang shutdown; the startup reconcile backs up anything not closed here.
        match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            shutdown_state.integration.bridge.abort_all_active_runs(),
        )
        .await
        {
            Ok(aborted) if !aborted.is_empty() => {
                tracing::info!(
                    count = aborted.len(),
                    "aborted in-flight runs on graceful shutdown"
                );
            }
            Ok(_) => {}
            Err(_) => tracing::warn!("timed out aborting in-flight runs on shutdown"),
        }

        tracing::info!("Gateway shut down gracefully");
        Ok(())
    }

    /// Get a reference to the shared state.
    pub fn state(&self) -> &Arc<AppState> {
        &self.state
    }
}

/// Await `serve`, but once the shutdown signal has fired (`drain_started`)
/// give in-flight connections at most `drain_timeout` to finish instead of
/// waiting on them forever. `serve` completing first — the fully graceful
/// path — always wins the race; before the signal there is no deadline at
/// all, so this can never cut short a healthy server.
async fn serve_with_bounded_drain(
    serve: impl std::future::IntoFuture<Output = std::io::Result<()>>,
    drain_started: tokio::sync::oneshot::Receiver<()>,
    drain_timeout: Duration,
) -> std::io::Result<()> {
    let serve = serve.into_future();
    tokio::pin!(serve);
    let drain_deadline = async move {
        // A dropped sender means the shutdown future never fired; in that
        // case there is no drain phase to bound, so never trigger.
        if drain_started.await.is_ok() {
            tokio::time::sleep(drain_timeout).await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    tokio::select! {
        result = &mut serve => result,
        _ = drain_deadline => {
            tracing::warn!(
                timeout_secs = drain_timeout.as_secs(),
                "graceful drain timed out; shutting down with connections still open"
            );
            Ok(())
        }
    }
}

/// Wait for SIGINT (Ctrl+C) or SIGTERM for graceful shutdown.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %err, "failed to install Ctrl+C handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sigterm) => {
                sigterm.recv().await;
            }
            Err(err) => {
                tracing::error!(error = %err, "failed to install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => { tracing::info!("Received SIGINT"); }
        _ = terminate => { tracing::info!("Received SIGTERM"); }
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod server_tests;
