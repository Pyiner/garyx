use axum::Router;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing;

use crate::server_lifecycle::start_gateway_runtime;

pub use crate::app_bootstrap::{AppStateBuilder, create_app_state, create_app_state_with_bridge};
pub use crate::app_state::{AppState, IntegrationState, OpsState, RuntimeState, ThreadState};
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
        start_gateway_runtime(self.state.clone());
        let listener = tokio::net::TcpListener::bind(addr).await?;
        self.state.spawn_gateway_sync_cache_warmup();
        tracing::info!("Gateway listening on {}", addr);

        let shutdown_state = self.state.clone();
        axum::serve(
            listener,
            self.router
                .into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal())
        .await?;

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
