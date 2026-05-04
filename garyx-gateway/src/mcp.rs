//! MCP (Model Context Protocol) server using the official rmcp SDK.
//!
//! Provides a Streamable HTTP MCP endpoint at `/mcp` via rmcp's
//! `StreamableHttpService`. Replaces the hand-rolled JSON-RPC dispatch
//! with proper MCP protocol support.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use garyx_channels::OutboundMessage;
use garyx_models::Verdict;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{RoleServer, ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::delivery_target::resolve_delivery_target_with_recovery;
use crate::server::AppState;

mod helpers;
#[cfg(test)]
mod tests;
mod tools;

// ---------------------------------------------------------------------------
// Parameter types (JsonSchema enables auto tool discovery)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MessageParams {
    /// Message text to send
    #[serde(default)]
    pub text: Option<String>,
    /// Optional local image path. Supported for telegram/weixin/feishu targets.
    #[serde(default)]
    pub image: Option<String>,
    /// Optional local file path. Supported for telegram/weixin/feishu targets.
    #[serde(default)]
    pub file: Option<String>,
    /// Bot selector as `channel:account_id`, e.g. `telegram:main`. If omitted, reply via the current thread.
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
pub struct ConversationHistoryParams {
    /// Optional thread id to restrict the history search to a single conversation
    #[serde(default, alias = "threadId")]
    pub thread_id: Option<String>,
    /// Optional workspace path to restrict matching threads
    #[serde(default, alias = "workspaceDir")]
    pub workspace_dir: Option<String>,
    /// Inclusive lower time bound. Accepts RFC3339, YYYY-MM-DD, YYYY-MM-DD HH:MM, or YYYY-MM-DDTHH:MM
    #[serde(default)]
    pub from: Option<String>,
    /// Inclusive upper time bound. Accepts RFC3339, YYYY-MM-DD, YYYY-MM-DD HH:MM, or YYYY-MM-DDTHH:MM
    #[serde(default)]
    pub to: Option<String>,
    /// Maximum number of text messages to return after filtering. Defaults to 200.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ConversationSearchParams {
    /// Search query for recalling relevant past conversations
    pub query: String,
    /// Optional thread id to restrict search to a single conversation
    #[serde(default, alias = "threadId")]
    pub thread_id: Option<String>,
    /// Optional workspace path to restrict matching threads
    #[serde(default, alias = "workspaceDir")]
    pub workspace_dir: Option<String>,
    /// Inclusive lower time bound. Accepts RFC3339, YYYY-MM-DD, YYYY-MM-DD HH:MM, or YYYY-MM-DDTHH:MM
    #[serde(default)]
    pub from: Option<String>,
    /// Inclusive upper time bound. Accepts RFC3339, YYYY-MM-DD, YYYY-MM-DD HH:MM, or YYYY-MM-DDTHH:MM
    #[serde(default)]
    pub to: Option<String>,
    /// Maximum number of search results to return. Defaults to 5.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RebindCurrentChannelParams {
    /// Agent or team ID to bind to the new thread.
    #[serde(alias = "agentId")]
    pub agent_id: String,
    /// Workspace directory for the new thread.
    #[serde(alias = "workspaceDir")]
    pub workspace_dir: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AutoResearchVerdictParams {
    /// Score from 0 to 10
    pub score: f32,
    /// Free-text evaluation: what's good, what's bad, suggestions for next iteration.
    /// Required — the verifier must provide qualitative guidance.
    pub feedback: String,
}

impl From<AutoResearchVerdictParams> for Verdict {
    fn from(value: AutoResearchVerdictParams) -> Self {
        Self {
            score: value.score,
            feedback: value.feedback,
        }
    }
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
    from_id: Option<String>,
    delivery_thread_id: Option<String>,
    auth_token: Option<String>,
    auto_research_role: Option<String>,
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
            auto_research_role: h("x-gary-auto-research-role"),
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
        description = "Send a message to another channel/target, or send a local image/file reply to the current user. Do not use this tool for ordinary text replies to the current user; reply directly in the assistant response by default. Use this tool when you need to reply to the current user with an image/file, or when messaging another bot/channel/target. Provide `bot` (e.g. `telegram:main`) to send to that bot's main endpoint; omit `bot` to reply in the current thread."
    )]
    async fn message(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(params): Parameters<MessageParams>,
    ) -> Result<String, String> {
        tools::message::run(self, ctx, params).await
    }

    #[tool(
        description = "Search the web using Google Search grounding via Gemini. Returns grounded answers with source citations."
    )]
    async fn search(&self, Parameters(params): Parameters<SearchParams>) -> Result<String, String> {
        tools::search::run(self, params).await
    }

    #[tool(
        description = "Fetch user/assistant text transcript lines from stored conversations. Use this for questions like '最近我们聊了啥', '这个线程里聊了啥', or '这个 workspace 我们聊了啥'. Supports filtering by thread_id, workspace_dir, from, to, and limit. Tool messages are removed."
    )]
    async fn conversation_history(
        &self,
        Parameters(params): Parameters<ConversationHistoryParams>,
    ) -> Result<String, String> {
        tools::history::run(self, params).await
    }

    #[tool(
        description = "Search stored conversations for relevant user/assistant transcript snippets. Use this for semantic recall like '我们之前聊过 once 协议吗' or '找一下 workspace 里关于自动化的讨论'. Supports filtering by thread_id, workspace_dir, from, to, and limit."
    )]
    async fn conversation_search(
        &self,
        Parameters(params): Parameters<ConversationSearchParams>,
    ) -> Result<String, String> {
        tools::conversation_search::run(self, params).await
    }

    #[tool(
        description = "Create a new thread for the current bound channel conversation using the requested agent_id and workspace_dir, then rebind the current endpoint to that new thread. Requires current MCP thread/channel context and sends no message."
    )]
    async fn rebind_current_channel(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(params): Parameters<RebindCurrentChannelParams>,
    ) -> Result<String, String> {
        tools::rebind_current_channel::run(self, ctx, params).await
    }

    #[tool(
        description = "Internal AutoResearch verifier tool. Submit a structured verdict for the current verifier thread. Only callable when the request carries the AutoResearch verifier header."
    )]
    async fn auto_research_verdict(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(params): Parameters<AutoResearchVerdictParams>,
    ) -> Result<String, String> {
        tools::auto_research::run_verdict(self, ctx, params).await
    }

    #[tool(
        description = "Stop loop mode for the current thread. Call this when you have completed all pending tasks and there is no more work to do."
    )]
    async fn stop_loop(&self, ctx: RequestContext<RoleServer>) -> Result<String, String> {
        tools::stop_loop::run(self, ctx).await
    }
}

// ---------------------------------------------------------------------------
// ServerHandler (rmcp generates call_tool / list_tools)
// ---------------------------------------------------------------------------

#[tool_handler]
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
                "Garyx MCP server. Tools: status, message, search, conversation_history, conversation_search, rebind_current_channel, stop_loop."
                    .to_owned(),
            ),
        }
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
