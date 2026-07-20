use std::sync::Arc;

use axum::{Router, extract::DefaultBodyLimit};
use tower::ServiceBuilder;
use tower_http::limit::RequestBodyLimitLayer;

use crate::server::AppState;
use crate::{
    automation, capsules, chat, coding_usage, commands, create_dispatch, dashboard, gateway_auth,
    mcp, mcp_config, meetings, provider_auth, restart_wake, routes, tasks, tool_image,
    workspace_files, workspaces,
};

pub fn build_router(state: Arc<AppState>) -> Router {
    let protected = Router::new()
        .merge(protected_runtime_routes())
        .merge(thread_routes())
        .merge(usage_routes())
        .merge(chat_routes())
        .merge(observability_routes())
        .merge(operations_routes())
        .merge(mcp_routes(state.clone()))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            gateway_auth::enforce_gateway_auth,
        ));

    Router::new()
        .merge(public_runtime_routes())
        .merge(protected)
        .fallback(routes::fallback)
        .with_state(state)
}

fn public_runtime_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/health", axum::routing::get(routes::health))
        .route(
            "/health/detailed",
            axum::routing::get(routes::health_detailed),
        )
}

fn protected_runtime_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/runtime", axum::routing::get(routes::runtime_info))
        .route(
            "/api/store-identity",
            axum::routing::get(routes::store_identity),
        )
}

fn usage_routes() -> Router<Arc<AppState>> {
    Router::new().route(
        "/api/usage/coding",
        axum::routing::get(coding_usage::get_coding_usage),
    )
}

fn thread_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/status", axum::routing::get(routes::system_status))
        .route(
            "/api/threads",
            axum::routing::get(routes::list_threads).post(routes::create_thread),
        )
        .route(
            "/api/threads/create-and-dispatch",
            axum::routing::post(create_dispatch::create_and_dispatch),
        )
        .route(
            "/api/threads/by-create-intent",
            axum::routing::get(create_dispatch::get_by_create_intent),
        )
        .route(
            "/api/recent-threads",
            axum::routing::get(routes::list_recent_threads),
        )
        .route(
            "/api/thread-summaries",
            axum::routing::get(routes::list_thread_summaries),
        )
        .route(
            restart_wake::RESTART_WAKE_ALL_SNAPSHOT_PATH,
            axum::routing::get(restart_wake::restart_wake_all_snapshot_endpoint),
        )
        .route(
            "/api/provider-sessions/recent",
            axum::routing::get(routes::list_recent_provider_sessions),
        )
        .route(
            "/api/threads/history",
            axum::routing::get(routes::thread_history),
        )
        .route(
            "/api/thread-pins",
            axum::routing::get(routes::list_thread_pins).put(routes::reorder_thread_pins),
        )
        .route(
            "/api/thread-pins/{key}",
            axum::routing::put(routes::pin_thread).delete(routes::unpin_thread),
        )
        .route(
            "/api/thread-favorites",
            axum::routing::get(routes::list_thread_favorites),
        )
        .route(
            "/api/thread-favorites/snapshot",
            axum::routing::get(routes::thread_favorites_snapshot),
        )
        .route(
            "/api/thread-favorites/{key}",
            axum::routing::put(routes::favorite_thread).delete(routes::unfavorite_thread),
        )
        .route(
            "/api/threads/{key}",
            axum::routing::get(routes::get_thread)
                .patch(routes::update_thread)
                .delete(routes::delete_thread),
        )
        .route(
            "/api/threads/{key}/archive",
            axum::routing::post(routes::archive_thread),
        )
        .route(
            "/api/threads/{key}/logs",
            axum::routing::get(routes::get_thread_logs),
        )
        .route(
            "/api/threads/{key}/stream",
            axum::routing::get(routes::thread_stream),
        )
        .route(
            "/api/tasks",
            axum::routing::get(tasks::list_tasks).post(tasks::create_task),
        )
        .route(
            "/api/tasks/forest",
            axum::routing::get(tasks::list_task_forest),
        )
        .route(
            "/api/tasks/{task_id}",
            axum::routing::get(tasks::get_task).delete(tasks::delete_task),
        )
        .route(
            "/api/tasks/{task_id}/history",
            axum::routing::get(tasks::task_history),
        )
        .route(
            "/api/tasks/{task_id}/stop",
            axum::routing::post(tasks::stop_task),
        )
        .route(
            "/api/tasks/{task_id}/assign",
            axum::routing::patch(tasks::assign_task).delete(tasks::unassign_task),
        )
        .route(
            "/api/tasks/{task_id}/status",
            axum::routing::patch(tasks::update_task_status),
        )
        .route(
            "/api/tasks/{task_id}/title",
            axum::routing::patch(tasks::set_task_title),
        )
        .route(
            "/api/channel-endpoints",
            axum::routing::get(routes::list_channel_endpoints),
        )
        .route(
            "/api/workspaces/git-status",
            axum::routing::get(routes::workspace_git_status),
        )
        .route(
            "/api/workspaces/directories",
            axum::routing::get(workspaces::list_workspace_directories),
        )
        .route(
            "/api/workspaces",
            axum::routing::get(workspaces::list_workspaces)
                .post(workspaces::upsert_workspace)
                .delete(workspaces::delete_workspace),
        )
        .route("/api/capsules", axum::routing::get(capsules::list_capsules))
        .route(
            "/api/capsules/{id}",
            axum::routing::get(capsules::get_capsule).delete(capsules::delete_capsule),
        )
        .route(
            "/api/capsules/{id}/favorite",
            axum::routing::put(capsules::favorite_capsule).delete(capsules::unfavorite_capsule),
        )
        .route(
            "/api/capsules/{id}/serve",
            axum::routing::get(capsules::serve_capsule),
        )
        .route("/api/meetings", axum::routing::get(meetings::list_meetings))
        .route(
            "/api/meetings/{id}",
            axum::routing::get(meetings::get_meeting).delete(meetings::delete_meeting),
        )
        .route(
            "/api/meetings/{id}/read",
            axum::routing::post(meetings::read_meeting),
        )
        .route(
            "/api/meetings/{id}/read/confirm",
            axum::routing::post(meetings::confirm_meeting_read),
        )
        .route(
            "/api/meetings/{id}/abort",
            axum::routing::post(meetings::abort_meeting),
        )
        .route(
            "/api/configured-bots",
            axum::routing::get(routes::list_configured_bots),
        )
        .route(
            "/api/bot-consoles",
            axum::routing::get(routes::list_bot_consoles),
        )
        .route(
            "/api/channel-bindings/bind",
            axum::routing::post(routes::bind_channel_endpoint),
        )
        .route(
            "/api/channel-bindings/detach",
            axum::routing::post(routes::detach_channel_endpoint),
        )
        .route(
            "/api/automations",
            axum::routing::get(automation::list_automations).post(automation::create_automation),
        )
        .route(
            "/api/automations/{id}",
            axum::routing::get(automation::get_automation)
                .patch(automation::update_automation)
                .delete(automation::delete_automation),
        )
        .route(
            "/api/automations/{id}/run-now",
            axum::routing::post(automation::run_automation_now),
        )
        .route(
            "/api/automations/{id}/threads",
            axum::routing::get(automation::automation_threads),
        )
        .route(
            "/api/automations/{id}/activity",
            axum::routing::get(automation::automation_activity),
        )
        .route(
            "/api/custom-agents",
            axum::routing::get(routes::list_custom_agents).post(routes::create_custom_agent),
        )
        .route(
            "/api/provider-models/{provider_type}",
            axum::routing::get(routes::list_provider_models),
        )
        .route(
            "/api/custom-agents/{agent_id}",
            axum::routing::get(routes::get_custom_agent)
                .put(routes::update_custom_agent)
                .delete(routes::delete_custom_agent),
        )
        .route(
            "/api/custom-agents/{agent_id}/toggle",
            axum::routing::patch(routes::toggle_custom_agent),
        )
        .route(
            "/api/custom-agents/{agent_id}/default",
            axum::routing::patch(routes::set_default_custom_agent),
        )
        .route(
            "/api/skills",
            axum::routing::get(routes::list_skills).post(routes::create_skill),
        )
        .route(
            "/api/skills/{id}",
            axum::routing::patch(routes::update_skill).delete(routes::delete_skill),
        )
        .route(
            "/api/skills/{id}/tree",
            axum::routing::get(routes::skill_tree),
        )
        .route(
            "/api/skills/{id}/file",
            axum::routing::get(routes::read_skill_file).put(routes::write_skill_file),
        )
        .route(
            "/api/skills/{id}/entries",
            axum::routing::post(routes::create_skill_entry).delete(routes::delete_skill_entry),
        )
        .route(
            "/api/skills/{id}/toggle",
            axum::routing::patch(routes::toggle_skill),
        )
        .route(
            "/api/workspace-files",
            axum::routing::get(workspace_files::list_workspace_files),
        )
        .route(
            "/api/workspace-files/preview",
            axum::routing::get(workspace_files::preview_workspace_file),
        )
        .route(
            "/api/workspace-files/upload",
            axum::routing::post(workspace_files::upload_workspace_files).layer(
                DefaultBodyLimit::max(workspace_files::MAX_UPLOAD_BODY_BYTES),
            ),
        )
        .route(
            "/api/chat/attachments/upload",
            axum::routing::post(workspace_files::upload_chat_attachments).layer(
                DefaultBodyLimit::max(workspace_files::MAX_UPLOAD_BODY_BYTES),
            ),
        )
        .route(
            "/api/tools/image",
            axum::routing::post(tool_image::generate_image),
        )
        .route(
            "/api/commands/shortcuts",
            axum::routing::get(commands::list_shortcuts).post(commands::create_shortcut),
        )
        .route(
            "/api/commands/shortcuts/{name}",
            axum::routing::put(commands::update_shortcut).delete(commands::delete_shortcut),
        )
        .route(
            "/api/mcp-servers",
            axum::routing::get(mcp_config::list_mcp_servers).post(mcp_config::create_mcp_server),
        )
        .route(
            "/api/mcp-servers/{name}",
            axum::routing::put(mcp_config::update_mcp_server).delete(mcp_config::delete_mcp_server),
        )
        .route(
            "/api/mcp-servers/{name}/toggle",
            axum::routing::patch(mcp_config::toggle_mcp_server),
        )
}

fn chat_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/chat/start", axum::routing::post(chat::chat_start))
        .route("/api/chat/ws", axum::routing::get(chat::chat_ws))
        .route(
            "/api/chat/interrupt",
            axum::routing::post(chat::chat_interrupt),
        )
        .route(
            "/api/chat/stream-input",
            axum::routing::post(chat::chat_stream_input),
        )
        .route("/api/chat/health", axum::routing::get(chat::chat_health))
}

fn observability_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/overview", axum::routing::get(dashboard::overview))
        .route("/api/agent-view", axum::routing::get(dashboard::agent_view))
        .route("/api/logs/tail", axum::routing::get(dashboard::logs_tail))
        .route(
            "/api/threads/diagnostics",
            axum::routing::get(routes::thread_diagnostics),
        )
        .route("/api/bot/status", axum::routing::get(routes::bot_status))
        .route("/api/bot/bind", axum::routing::post(routes::bot_bind))
        .route("/api/bot/unbind", axum::routing::post(routes::bot_unbind))
        .route(
            "/api/settings",
            axum::routing::get(dashboard::settings).put(routes::settings_update),
        )
        .route(
            "/api/settings/reload",
            axum::routing::post(routes::settings_reload),
        )
}

fn operations_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/cron/jobs", axum::routing::get(routes::cron_jobs))
        .route("/api/cron/runs", axum::routing::get(routes::cron_runs))
        // Debug observability for system-managed cron jobs (AXON-692). Lives in
        // the protected router so `enforce_gateway_auth` gates it: loopback
        // passes, everything else needs a valid gateway token.
        .route(
            "/api/debug/system-cron-jobs",
            axum::routing::get(routes::debug_system_cron_jobs),
        )
        .route(
            "/api/debug/system-cron-jobs/{id}/run",
            axum::routing::post(routes::debug_run_system_cron_job),
        )
        .route(
            "/api/channels/plugins",
            axum::routing::get(routes::list_channel_plugins),
        )
        // Auto-login flow. The desktop UI calls `start` with the
        // current form state, then polls `poll` at the cadence the
        // executor returned until it sees Confirmed or Failed. The
        // Mac App's code is plugin-blind — these endpoints route
        // built-in and subprocess plugins through the same
        // `AuthFlowExecutor` trait on the gateway side.
        .route(
            "/api/channels/plugins/{plugin_id}/auth_flow/start",
            axum::routing::post(routes::channel_auth_flow_start),
        )
        .route(
            "/api/channels/plugins/{plugin_id}/auth_flow/poll",
            axum::routing::post(routes::channel_auth_flow_poll),
        )
        .route(
            "/api/channels/plugins/{plugin_id}/validate_account",
            axum::routing::post(routes::channel_account_validate),
        )
        .route(
            "/api/providers/claude_code/auth/start",
            axum::routing::post(provider_auth::start_claude_code_auth),
        )
        .route(
            "/api/providers/claude_code/auth/{login_id}",
            axum::routing::get(provider_auth::get_claude_code_auth),
        )
        .route(
            "/api/providers/claude_code/auth/{login_id}/submit",
            axum::routing::post(provider_auth::submit_claude_code_auth),
        )
        .route("/api/restart", axum::routing::post(routes::restart))
        .route("/api/send", axum::routing::post(routes::send_message))
}

fn mcp_routes(state: Arc<AppState>) -> Router<Arc<AppState>> {
    let mcp_service = mcp::create_mcp_service(state, tokio_util::sync::CancellationToken::new());
    let mcp_service = ServiceBuilder::new()
        .layer(RequestBodyLimitLayer::new(
            capsules::CAPSULE_MCP_BODY_LIMIT_BYTES,
        ))
        .service(mcp_service);
    // Claude Code CLI strips custom headers and query params from MCP tool
    // call requests, so we encode `{thread_id}` and `{run_id}` directly in
    // the URL path: clients call `/mcp/{thread_id}/{run_id}` (Claude Code),
    // `/mcp/{thread_id}`, or just `/mcp` (Codex, which still uses headers).
    //
    // The inner rmcp `StreamableHttpService` only serves at its root, so the
    // ID segments must be removed before the request reaches it.  We mount a
    // single `nest_service("/mcp", …)` (avoiding the matchit conflict from
    // overlapping nests) and use a middleware that:
    //   1. Saves the original URI to `OriginalMcpUri` for downstream parsing
    //      in `RunContext::from_request_context`.
    //   2. Rewrites the URI path to plain `/mcp`, so `nest_service`'s
    //      `StripPrefix` cleanly drops it and the inner service sees `/`.
    Router::new()
        .nest_service("/mcp", mcp_service)
        .layer(axum::middleware::from_fn(rewrite_mcp_uri))
}

/// Middleware: capture the original `/mcp/...` URI (with optional thread_id
/// and run_id segments) into request extensions, then rewrite the URI down
/// to `/mcp` so `nest_service`'s `StripPrefix` produces `/` for the inner
/// `StreamableHttpService`.
async fn rewrite_mcp_uri(
    mut req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let uri = req.uri().clone();
    let path = uri.path();
    if path == "/mcp" || path.starts_with("/mcp/") {
        // Always record the original URI; downstream RunContext parsing
        // tolerates the no-extra-segments case.
        req.extensions_mut()
            .insert(mcp::OriginalMcpUri(uri.clone()));

        if path != "/mcp" {
            // Collapse `/mcp/{thread_id}[/{run_id}[/...]]` → `/mcp` so
            // nest_service can strip it cleanly.
            let new_pq = match uri.query() {
                Some(q) => format!("/mcp?{q}"),
                None => "/mcp".to_string(),
            };
            let mut parts = uri.into_parts();
            if let Ok(pq) = new_pq.parse::<axum::http::uri::PathAndQuery>() {
                parts.path_and_query = Some(pq);
                if let Ok(new_uri) = axum::http::Uri::from_parts(parts) {
                    *req.uri_mut() = new_uri;
                }
            }
        }
    }
    next.run(req).await
}
