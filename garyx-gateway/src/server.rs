use axum::Router;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing;

use crate::server_frontend::mount_frontend_routes;
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
        let router = mount_frontend_routes(route_graph::build_router(state.clone()));

        Self { state, router }
    }

    /// Serve the gateway, blocking until `shutdown_signal` fires.
    pub async fn serve(self, addr: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
        start_gateway_runtime(self.state.clone());
        tracing::info!("Gateway listening on {}", addr);

        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(
            listener,
            self.router
                .into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal())
        .await?;

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
