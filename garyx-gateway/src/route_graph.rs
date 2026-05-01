use std::sync::Arc;

use axum::Router;

use crate::server::AppState;
use crate::{
    api, automation, chat, commands, dashboard, gateway_auth, mcp, mcp_config, routes, tasks,
    workspace_files,
};

pub fn build_router(state: Arc<AppState>) -> Router {
    let protected = Router::new()
        .merge(protected_runtime_routes())
        .merge(thread_routes())
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
        .route("/agent", axum::routing::get(routes::redirect_legacy_status))
        .route(
            "/status",
            axum::routing::get(routes::redirect_legacy_status),
        )
        .route(
            "/settings",
            axum::routing::get(routes::redirect_legacy_settings),
        )
        .route("/logs", axum::routing::get(routes::redirect_legacy_logs))
        .route("/cron", axum::routing::get(routes::redirect_legacy_cron))
        .route(
            "/threads",
            axum::routing::get(routes::redirect_legacy_threads),
        )
        .route("/runtime", axum::routing::get(routes::runtime_info))
}

fn thread_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/status", axum::routing::get(routes::system_status))
        .route(
            "/api/threads",
            axum::routing::get(routes::list_threads).post(routes::create_thread),
        )
        .route(
            "/api/threads/history",
            axum::routing::get(api::thread_history),
        )
        .route(
            "/api/threads/{key}",
            axum::routing::get(routes::get_thread)
                .patch(routes::update_thread)
                .delete(routes::delete_thread),
        )
        .route(
            "/api/threads/{key}/logs",
            axum::routing::get(routes::get_thread_logs),
        )
        .route(
            "/api/tasks",
            axum::routing::get(tasks::list_tasks).post(tasks::create_task),
        )
        .route(
            "/api/tasks/batch",
            axum::routing::post(tasks::create_tasks_batch),
        )
        .route(
            "/api/tasks/promote",
            axum::routing::post(tasks::promote_task),
        )
        .route("/api/tasks/{task_ref}", axum::routing::get(tasks::get_task))
        .route(
            "/api/tasks/{task_ref}/history",
            axum::routing::get(tasks::task_history),
        )
        .route(
            "/api/tasks/{task_ref}/assign",
            axum::routing::patch(tasks::assign_task).delete(tasks::unassign_task),
        )
        .route(
            "/api/tasks/{task_ref}/status",
            axum::routing::patch(tasks::update_task_status),
        )
        .route(
            "/api/tasks/{task_ref}/title",
            axum::routing::patch(tasks::set_task_title),
        )
        .route(
            "/api/channel-endpoints",
            axum::routing::get(routes::list_channel_endpoints),
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
            "/api/automations/{id}/activity",
            axum::routing::get(automation::automation_activity),
        )
        .route(
            "/api/auto-research/runs",
            axum::routing::get(api::list_auto_research_runs).post(api::create_auto_research_run),
        )
        .route(
            "/api/auto-research/runs/{run_id}",
            axum::routing::get(api::get_auto_research_run)
                .patch(api::patch_auto_research_run)
                .delete(api::delete_auto_research_run),
        )
        .route(
            "/api/auto-research/runs/{run_id}/iterations",
            axum::routing::get(api::list_auto_research_iterations),
        )
        .route(
            "/api/auto-research/runs/{run_id}/stop",
            axum::routing::post(api::stop_auto_research_run),
        )
        .route(
            "/api/auto-research/runs/{run_id}/candidates",
            axum::routing::get(api::list_auto_research_candidates),
        )
        .route(
            "/api/auto-research/runs/{run_id}/select/{candidate_id}",
            axum::routing::post(api::select_auto_research_candidate),
        )
        .route(
            "/api/auto-research/runs/{run_id}/feedback",
            axum::routing::post(api::inject_auto_research_feedback),
        )
        .route(
            "/api/auto-research/runs/{run_id}/reverify",
            axum::routing::post(api::reverify_auto_research_candidate),
        )
        .route(
            "/api/custom-agents",
            axum::routing::get(api::list_custom_agents).post(api::create_custom_agent),
        )
        .route(
            "/api/provider-models/{provider_type}",
            axum::routing::get(api::list_provider_models),
        )
        .route(
            "/api/teams",
            axum::routing::get(api::list_agent_teams).post(api::create_agent_team),
        )
        .route(
            "/api/teams/{team_id}",
            axum::routing::get(api::get_agent_team)
                .put(api::update_agent_team)
                .delete(api::delete_agent_team),
        )
        .route(
            "/api/custom-agents/{agent_id}",
            axum::routing::get(api::get_custom_agent)
                .put(api::update_custom_agent)
                .delete(api::delete_custom_agent),
        )
        .route(
            "/api/wikis",
            axum::routing::get(api::list_wikis).post(api::create_wiki),
        )
        .route(
            "/api/wikis/{wiki_id}",
            axum::routing::get(api::get_wiki)
                .put(api::update_wiki)
                .delete(api::delete_wiki),
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
            axum::routing::post(workspace_files::upload_workspace_files),
        )
        .route(
            "/api/chat/attachments/upload",
            axum::routing::post(workspace_files::upload_chat_attachments),
        )
        .route("/api/commands", axum::routing::get(commands::list_commands))
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
        .route("/api/chat/ws", axum::routing::get(chat::chat_ws))
        .route("/api/chat/health", axum::routing::get(chat::chat_health))
}

fn observability_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/overview", axum::routing::get(dashboard::overview))
        .route("/api/agent-view", axum::routing::get(dashboard::agent_view))
        .route("/api/logs/tail", axum::routing::get(dashboard::logs_tail))
        .route("/api/debug/thread", axum::routing::get(api::debug_thread))
        .route("/api/debug/bot", axum::routing::get(api::debug_bot))
        .route(
            "/api/debug/bot/threads",
            axum::routing::get(api::debug_bot_threads),
        )
        .route(
            "/api/settings",
            axum::routing::get(dashboard::settings).put(api::settings_update),
        )
        .route(
            "/api/settings/reload",
            axum::routing::post(api::settings_reload),
        )
        .route("/api/stream", axum::routing::get(dashboard::event_stream))
}

fn operations_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/cron/jobs", axum::routing::get(api::cron_jobs))
        .route("/api/cron/runs", axum::routing::get(api::cron_runs))
        .route(
            "/api/channels/plugins",
            axum::routing::get(api::list_channel_plugins),
        )
        // Auto-login flow. The desktop UI calls `start` with the
        // current form state, then polls `poll` at the cadence the
        // executor returned until it sees Confirmed or Failed. The
        // Mac App's code is plugin-blind — these endpoints route
        // built-in and subprocess plugins through the same
        // `AuthFlowExecutor` trait on the gateway side.
        .route(
            "/api/channels/plugins/{plugin_id}/auth_flow/start",
            axum::routing::post(api::channel_auth_flow_start),
        )
        .route(
            "/api/channels/plugins/{plugin_id}/auth_flow/poll",
            axum::routing::post(api::channel_auth_flow_poll),
        )
        .route(
            "/api/channels/plugins/{plugin_id}/validate_account",
            axum::routing::post(api::channel_account_validate),
        )
        .route("/api/restart", axum::routing::post(api::restart))
        .route("/api/send", axum::routing::post(api::send_message))
}

fn mcp_routes(state: Arc<AppState>) -> Router<Arc<AppState>> {
    let mcp_service = mcp::create_mcp_service(state, tokio_util::sync::CancellationToken::new());
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
