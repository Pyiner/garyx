mod agent_identity;
mod agent_team_provider;
mod agent_teams;
pub mod api;
mod application;
mod auto_research;
pub mod automation;
mod channel_catalog;
pub mod chat;
pub mod commands;
mod composition;
pub mod cron;
mod custom_agents;
pub mod dashboard;
mod delivery_target;
pub mod gateway_auth;
pub mod health;
mod internal_inbound;
mod loop_continuation;
mod managed_mcp_metadata;
pub mod mcp;
pub mod mcp_config;
mod provider_models;
mod provider_session_locator;
pub mod restart;
pub mod restart_wake;
mod route_graph;
pub mod routes;
mod runtime_diagnostics;
pub mod server;
pub mod skills;
mod task_notifications;
pub mod tasks;
pub mod thread_logs;
mod wikis;
pub mod workspace_files;

#[cfg(all(test, feature = "real-provider-tests"))]
mod downstream_real_tests;
#[cfg(all(test, feature = "real-provider-tests"))]
mod managed_mcp_real_tests;

pub use cron::CronService;
pub use route_graph::build_router;
pub use server::{AppState, Gateway};
pub use thread_logs::{ThreadFileLogger, default_thread_log_dir};

pub(crate) use application::chat::control as chat_control;
pub(crate) use application::chat::delivery as chat_delivery;
pub(crate) use application::chat::prepare as chat_application;
pub(crate) use application::chat::shared as chat_shared;
pub(crate) use composition::app_bootstrap;
pub(crate) use composition::app_state;
pub(crate) use composition::event_stream_hub;
pub(crate) use composition::frontend as server_frontend;
pub(crate) use composition::lifecycle as server_lifecycle;
pub(crate) use composition::mcp_metrics;
pub(crate) use composition::runtime_cells;

#[cfg(test)]
pub(crate) mod test_support {
    use axum::http::{Request, header, request};
    use garyx_models::config::GaryxConfig;

    pub(crate) const TEST_GATEWAY_TOKEN: &str = "test-gateway-token";
    pub(crate) const TEST_GATEWAY_AUTHORIZATION: &str = "Bearer test-gateway-token";

    pub(crate) fn with_gateway_auth(mut config: GaryxConfig) -> GaryxConfig {
        config.gateway.auth_token = TEST_GATEWAY_TOKEN.to_owned();
        config
    }

    pub(crate) fn authed_request() -> request::Builder {
        Request::builder().header(header::AUTHORIZATION, TEST_GATEWAY_AUTHORIZATION)
    }
}
