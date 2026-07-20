mod agent_identity;
mod application;
mod atomic_write;
pub mod automation;
pub mod capsules;
mod channel_catalog;
pub mod chat;
mod claude_oauth;
pub mod coding_usage;
pub mod commands;
mod composition;
mod conversation_admission;
mod create_dispatch;
mod create_resources;
pub mod cron;
mod custom_agents;
pub mod dashboard;
mod delivery_target;
mod endpoint_binding_mutator;
mod endpoint_projection;
pub mod garyx_db;
pub mod gateway_auth;
pub mod health;
mod internal_inbound;
mod legacy_boot_import;
pub use legacy_boot_import::{
    LegacyBootImportError, LegacyBootImportOutcome, ThreadRecordImportSummary,
    run_legacy_boot_import,
};
mod managed_mcp_metadata;
pub mod mcp;
pub mod mcp_config;
pub mod meetings;
mod optimistic_write;
mod prompt_attachment_lifecycle;
mod provider_auth;
mod provider_models;
mod provider_session_locator;
mod quota_resend;
mod recent_thread_projection;
mod recent_thread_reader;
pub mod restart;
pub mod restart_wake;
mod route_graph;
pub mod routes;
mod runtime_diagnostics;
pub mod server;
pub mod skills;
mod sqlite_thread_store;
pub use sqlite_thread_store::{SqliteThreadStoreHandle, assemble_sqlite_thread_store};
mod task_notifications;
mod task_projection;
pub use task_projection::seed_task_counter_from_legacy;
mod task_tree;
pub mod tasks;
mod thread_lifecycle;
pub mod thread_logs;
mod thread_meta_projection;
mod thread_record_normalization;
mod thread_runtime;
mod thread_type;
mod tool_image;
mod transcript_run_projection;
pub mod workspace_files;
mod workspace_mode;
pub mod workspaces;

#[cfg(all(test, feature = "real-provider-tests"))]
mod downstream_real_tests;
#[cfg(all(test, feature = "real-provider-tests"))]
mod managed_mcp_real_tests;
#[cfg(test)]
mod source_guard_tests;

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
pub(crate) use composition::automation_wiring;
pub(crate) use composition::event_stream_hub;
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
