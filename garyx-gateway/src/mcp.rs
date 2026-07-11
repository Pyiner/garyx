//! MCP (Model Context Protocol) server using the official rmcp SDK.
//!
//! Provides a Streamable HTTP MCP endpoint at `/mcp` via rmcp's
//! `StreamableHttpService`. Replaces the hand-rolled JSON-RPC dispatch
//! with proper MCP protocol support.

#[cfg(test)]
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(test)]
use garyx_channels::OutboundMessage;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{RoleServer, ServerHandler, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
#[cfg(test)]
use uuid::Uuid;

#[cfg(test)]
use crate::delivery_target::resolve_delivery_target_with_recovery;
use crate::server::AppState;

mod helpers;
#[cfg(test)]
mod tests;
// `pub(crate)` (not `pub`) so other gateway modules — currently
// `cron::tests` reaching into `schedule_followup::followup_job_id` — can
// share the same helpers without re-exporting them on a public API
// surface.
pub(crate) mod tools;

// ---------------------------------------------------------------------------
// Parameter types (JsonSchema enables auto tool discovery)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[cfg(test)]
pub struct MessageParams {
    /// Message text to send
    #[serde(default)]
    pub text: Option<String>,
    /// Optional local image path. Supported for telegram/weixin/feishu targets.
    #[serde(default)]
    pub image: Option<String>,
    /// Optional local file path. Deprecated; MCP no longer exposes message sending.
    #[serde(default)]
    pub file: Option<String>,
    /// Bot selector as `channel:account_id`, e.g. `telegram:main`.
    #[serde(default, alias = "botId")]
    pub bot: Option<String>,
    // -- fields below are accepted but hidden from the schema --
    #[serde(default)]
    #[schemars(skip)]
    pub action: Option<String>,
    #[serde(default)]
    #[schemars(skip)]
    pub target: Option<String>,
    #[serde(default)]
    #[schemars(skip)]
    pub channel: Option<String>,
    #[serde(default, alias = "accountId")]
    #[schemars(skip)]
    pub account_id: Option<String>,
    #[serde(default, alias = "replyTo")]
    #[schemars(skip)]
    pub reply_to: Option<String>,
    #[serde(default, alias = "runId")]
    #[schemars(skip)]
    pub run_id: Option<String>,
    #[serde(default)]
    #[schemars(skip)]
    pub token: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchParams {
    /// The search query to look up using Google Search
    pub query: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScheduleFollowupParams {
    /// Wall-clock delay in seconds before the assistant is re-woken on the
    /// current thread. Must be in `60..=86400`; out-of-range requests are
    /// rejected with `out_of_range` rather than silently clamped.
    #[serde(alias = "delaySeconds")]
    pub delay_seconds: u64,
    /// Prompt text that will be injected back into the thread when the
    /// delay elapses. Echoed verbatim after a `<garyx_followup_metadata>`
    /// header so the resumed agent can correlate the turn.
    pub prompt: String,
    /// Optional free-text reason recorded in the metadata block; intended
    /// for the agent's own bookkeeping and surfaced in telemetry.
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CapsuleCreateParams {
    /// User-facing capsule title.
    pub title: String,
    /// Optional short description shown in Capsule lists.
    #[serde(default)]
    pub description: Option<String>,
    /// Inline self-contained HTML. Exactly one of `html` or `html_path` is required.
    #[serde(default)]
    pub html: Option<String>,
    /// Absolute path to a self-contained HTML file. Exactly one of `html` or `html_path` is required.
    #[serde(default, alias = "htmlPath")]
    pub html_path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CapsuleUpdateParams {
    /// Capsule UUID returned by `capsule_create` or `capsule_list`.
    #[serde(alias = "capsuleId")]
    pub capsule_id: String,
    /// New title. Omit to keep the existing title.
    #[serde(default)]
    pub title: Option<String>,
    /// New description. Omit to keep the existing description.
    #[serde(default)]
    pub description: Option<String>,
    /// Replacement self-contained HTML. At most one of `html` or `html_path` may be set.
    #[serde(default)]
    pub html: Option<String>,
    /// Absolute path to replacement self-contained HTML. At most one of `html` or `html_path` may be set.
    #[serde(default, alias = "htmlPath")]
    pub html_path: Option<String>,
}

// ---------------------------------------------------------------------------
// Original URI extension (set by middleware before nest_service strips prefix)
// ---------------------------------------------------------------------------

/// Holds the original request URI before axum's `nest_service` strips the
/// matched prefix.  Injected by a `map_request` layer in `route_graph`.
#[derive(Debug, Clone)]
pub struct OriginalMcpUri(pub axum::http::Uri);

fn decode_mcp_path_context(path: &str) -> (Option<String>, Option<String>) {
    let decode = |segment: &str| -> Option<String> {
        let trimmed = segment.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(
            urlencoding::decode(trimmed)
                .map(|value| value.into_owned())
                .unwrap_or_else(|_| trimmed.to_owned()),
        )
    };

    let mut segments: Vec<&str> = path
        .strip_prefix("/mcp/")
        .unwrap_or("")
        .split('/')
        .collect();
    if segments.first() == Some(&crate::gateway_auth::MCP_AUTH_SEGMENT) {
        segments = segments.into_iter().skip(2).collect();
    }
    let thread_id = segments.first().and_then(|segment| decode(segment));
    let run_id = segments.get(1).and_then(|segment| decode(segment));
    (thread_id, run_id)
}

// ---------------------------------------------------------------------------
// RunContext (extracted from HTTP headers via RequestContext)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct RunContext {
    run_id: Option<String>,
    thread_id: Option<String>,
    channel: Option<String>,
    account_id: Option<String>,
    #[allow(dead_code)]
    from_id: Option<String>,
    #[allow(dead_code)]
    delivery_thread_id: Option<String>,
    #[allow(dead_code)]
    auth_token: Option<String>,
}

impl RunContext {
    fn from_request_context(ctx: &RequestContext<RoleServer>) -> Self {
        let Some(parts) = ctx.extensions.get::<axum::http::request::Parts>() else {
            tracing::warn!(
                "MCP RunContext: no HTTP request parts in extensions — headers unavailable"
            );
            return Self::default();
        };

        let headers = &parts.headers;
        let h = |name: &str| -> Option<String> {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_owned())
        };

        // Extract thread_id and run_id from the original URI path.
        // The OriginalMcpUri extension is set by a middleware layer in route_graph
        // *before* axum's nest_service strips the prefix.
        // Path formats:
        //   /mcp/{thread_id}
        //   /mcp/{thread_id}/{run_id}
        //   /mcp/auth/{token}/{thread_id}/{run_id}
        let (path_thread_id, path_run_id) = parts
            .extensions
            .get::<OriginalMcpUri>()
            .map(|orig| decode_mcp_path_context(orig.0.path()))
            .unwrap_or((None, None));

        let auth_token = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.strip_prefix("Bearer ").unwrap_or(s).to_owned())
            .or_else(|| h("x-mcp-token"))
            .or_else(|| {
                parts
                    .extensions
                    .get::<OriginalMcpUri>()
                    .and_then(|orig| crate::gateway_auth::token_from_mcp_path(orig.0.path()))
            });
        let ctx = Self {
            run_id: h("x-run-id").or(path_run_id),
            thread_id: h("x-thread-id").or(path_thread_id),
            channel: h("x-channel"),
            account_id: h("x-account-id"),
            from_id: h("x-from-id"),
            delivery_thread_id: h("x-thread-scope"),
            auth_token,
        };
        tracing::info!(
            run_id = ?ctx.run_id,
            thread_id = ?ctx.thread_id,
            "MCP RunContext: resolved"
        );
        ctx
    }
}

#[derive(Debug, Clone)]
#[cfg(test)]
struct ResolvedMessageTarget {
    channel: String,
    account_id: String,
    chat_id: String,
    delivery_target_type: String,
    delivery_target_id: String,
    delivery_thread_id: Option<String>,
    thread_id: Option<String>,
}

// ---------------------------------------------------------------------------
// MCP Server
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct GaryMcpServer {
    app_state: Arc<AppState>,
    tool_router: ToolRouter<Self>,
}

impl GaryMcpServer {
    pub fn new(app_state: Arc<AppState>) -> Self {
        let tool_router = Self::tool_router();
        Self {
            app_state,
            tool_router,
        }
    }

}

#[tool_router]
impl GaryMcpServer {
    #[tool(
        description = "Get bot status: uptime, active threads, provider and channel info, plus current/available bots for the current thread when available"
    )]
    async fn status(&self, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        tools::status::run(self, ctx).await
    }

    #[tool(
        description = "Search the web using Google Search grounding via Gemini. Returns grounded answers with source citations."
    )]
    async fn search(&self, Parameters(params): Parameters<SearchParams>) -> Result<String, String> {
        tools::search::run(self, params).await
    }

    #[tool(
        description = "Schedule a delayed re-wake of the current thread. After `delay_seconds` (60..=86400) elapses, the gateway injects a synthetic user turn carrying the supplied `prompt` so the agent can continue work that depends on background progress. Multiple calls from the same (thread, run) replace each other and the response reports `replaced_previous` so the agent can see if it just bumped its own earlier schedule."
    )]
    async fn schedule_followup(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(params): Parameters<ScheduleFollowupParams>,
    ) -> Result<String, String> {
        tools::schedule_followup::run(self, ctx, params).await
    }

    #[tool(
        description = "Create a Capsule: a self-contained single-file HTML explanation, visualization, or demo for the current thread. Provide exactly one of `html` or absolute `html_path`; HTML must be UTF-8, <=5 MiB, and self-contained (resources inline or via data:/blob:/https: only). Returns the new capsule's `capsule_id` (UUID) and `serve_path`; pass that `capsule_id` to `capsule_update` to revise it later."
    )]
    async fn capsule_create(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(params): Parameters<CapsuleCreateParams>,
    ) -> Result<String, String> {
        tools::capsule::create(self, ctx, params).await
    }

    #[tool(
        description = "Update an existing Capsule by `capsule_id` (UUID from `capsule_create` or `capsule_list`). Provide at least one of title, description, html, or html_path; omit a field to leave it unchanged. Replacement HTML must be UTF-8, <=5 MiB, and self-contained (resources inline or via data:/blob:/https: only). Returns the capsule's `capsule_id` (UUID) and updated metadata."
    )]
    async fn capsule_update(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(params): Parameters<CapsuleUpdateParams>,
    ) -> Result<String, String> {
        tools::capsule::update(self, ctx, params).await
    }

    #[tool(
        description = "List Capsules created by the current thread, returning each `capsule_id` (UUID), title, revision, timestamp, and `serve_path` so a later run can pick the right one and revise it with `capsule_update`."
    )]
    async fn capsule_list(&self, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        tools::capsule::list(self, ctx).await
    }
}

// ---------------------------------------------------------------------------
// ServerHandler (rmcp generates call_tool / list_tools)
// ---------------------------------------------------------------------------

impl ServerHandler for GaryMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: Default::default(),
            capabilities: ServerCapabilities {
                tools: Some(Default::default()),
                ..Default::default()
            },
            server_info: rmcp::model::Implementation {
                name: "gary-mcp".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                title: None,
                description: None,
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Garyx MCP server. Tools: status, search, schedule_followup, capsule_create, capsule_update, capsule_list."
                    .to_owned(),
            ),
        }
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let tcc = ToolCallContext::new(self, request, context);
        self.tool_router.call(tcc).await
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        Ok(ListToolsResult {
            tools: self.tool_router.list_all(),
            meta: None,
            next_cursor: None,
        })
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tool_router.get(name).cloned()
    }
}

// ---------------------------------------------------------------------------
// Service factory (axum integration via nest_service)
// ---------------------------------------------------------------------------

pub fn create_mcp_service(
    app_state: Arc<AppState>,
    cancellation_token: CancellationToken,
) -> StreamableHttpService<GaryMcpServer, LocalSessionManager> {
    StreamableHttpService::new(
        move || Ok(GaryMcpServer::new(app_state.clone())),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig {
            stateful_mode: false,
            json_response: true,
            cancellation_token,
            ..Default::default()
        },
    )
}
