use std::collections::{HashMap, VecDeque};

use garyx_models::provider::ProviderRateLimit;
use serde_json::Value;
use tokio::sync::Mutex;

pub(crate) struct GaryxMcpServer {
    pub(crate) url: String,
    pub(crate) headers: HashMap<String, String>,
}

// Carry thread/run context in the path because some provider clients strip
// custom MCP headers. The gateway decodes these exact path segments.
pub(crate) fn garyx_mcp_server(
    base_url: &str,
    thread_id: &str,
    run_id: &str,
    metadata: &HashMap<String, Value>,
) -> Option<GaryxMcpServer> {
    let base_url = base_url.trim().trim_end_matches('/');
    if base_url.is_empty() {
        return None;
    }

    let mut headers = HashMap::from([
        ("X-Run-Id".to_owned(), run_id.to_owned()),
        ("X-Thread-Id".to_owned(), thread_id.to_owned()),
        ("X-Session-Key".to_owned(), thread_id.to_owned()),
    ]);
    if let Some(overrides) = metadata.get("garyx_mcp_headers").and_then(Value::as_object) {
        headers.extend(overrides.iter().filter_map(|(key, value)| {
            value.as_str().map(|value| (key.clone(), value.to_owned()))
        }));
    }

    let encoded_thread = urlencoding::encode(thread_id);
    let encoded_run = urlencoding::encode(run_id);
    let url = metadata
        .get("garyx_mcp_auth_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|token| {
            format!(
                "{base_url}/mcp/auth/{}/{encoded_thread}/{encoded_run}",
                urlencoding::encode(token)
            )
        })
        .unwrap_or_else(|| format!("{base_url}/mcp/{encoded_thread}/{encoded_run}"));

    Some(GaryxMcpServer { url, headers })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingAckMarker {
    // Marks the provider echo for the run's initial user message. It must be
    // consumed without acknowledging a queued follow-up.
    RootUserMessage,
    QueuedInput(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PendingAckQueue {
    markers: VecDeque<PendingAckMarker>,
}

impl PendingAckQueue {
    pub(crate) fn with_root_user_message() -> Self {
        Self {
            markers: VecDeque::from([PendingAckMarker::RootUserMessage]),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_root_user_messages(count: usize) -> Self {
        Self {
            markers: std::iter::repeat_n(PendingAckMarker::RootUserMessage, count).collect(),
        }
    }

    pub(crate) fn enqueue(&mut self, pending_input_id: String) {
        self.markers
            .push_back(PendingAckMarker::QueuedInput(pending_input_id));
    }

    pub(crate) fn rollback(&mut self, pending_input_id: &str) {
        if let Some(index) = self.markers.iter().position(
            |marker| matches!(marker, PendingAckMarker::QueuedInput(candidate) if candidate == pending_input_id),
        ) {
            self.markers.remove(index);
        }
    }

    pub(crate) fn acknowledge_next(&mut self, prefer_queued_input: bool) -> Option<String> {
        // Claude can emit assistant/tool activity before echoing the root user
        // message. In that case its next echo belongs to a queued follow-up;
        // Codex passes `false` and consumes the root marker strictly in order.
        if prefer_queued_input
            && matches!(
                self.markers.front(),
                Some(PendingAckMarker::RootUserMessage)
            )
            && self.has_queued_input()
        {
            self.markers.pop_front();
        }
        match self.markers.pop_front() {
            Some(PendingAckMarker::QueuedInput(pending_input_id)) => Some(pending_input_id),
            Some(PendingAckMarker::RootUserMessage) | None => None,
        }
    }

    pub(crate) fn has_queued_input(&self) -> bool {
        self.markers
            .iter()
            .any(|marker| matches!(marker, PendingAckMarker::QueuedInput(_)))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.markers.is_empty()
    }
}

#[derive(Default)]
pub(crate) struct PendingRateLimits {
    by_thread: Mutex<HashMap<String, ProviderRateLimit>>,
}

impl PendingRateLimits {
    pub(crate) async fn clear(&self, thread_id: &str) {
        self.by_thread.lock().await.remove(thread_id);
    }

    pub(crate) async fn stage(&self, thread_id: String, rate_limit: ProviderRateLimit) {
        self.by_thread.lock().await.insert(thread_id, rate_limit);
    }

    pub(crate) async fn take(&self, thread_id: &str) -> Option<ProviderRateLimit> {
        self.by_thread.lock().await.remove(thread_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn garyx_mcp_server_normalizes_base_url_and_encodes_context() {
        let server = garyx_mcp_server(
            " http://127.0.0.1:31337/// ",
            "thread::alpha",
            "run/1",
            &HashMap::new(),
        )
        .expect("server config");

        assert_eq!(
            server.url,
            "http://127.0.0.1:31337/mcp/thread%3A%3Aalpha/run%2F1"
        );
        assert_eq!(
            server.headers.get("X-Run-Id").map(String::as_str),
            Some("run/1")
        );
        assert_eq!(
            server.headers.get("X-Session-Key").map(String::as_str),
            Some("thread::alpha")
        );
    }

    #[test]
    fn garyx_mcp_server_encodes_auth_token_and_applies_header_overrides() {
        let metadata = HashMap::from([
            (
                "garyx_mcp_auth_token".to_owned(),
                Value::String(" secret/token ".to_owned()),
            ),
            (
                "garyx_mcp_headers".to_owned(),
                json!({
                    "X-Run-Id": "override",
                    "X-Test": "value",
                    "X-Ignored": 42,
                }),
            ),
        ]);
        let server = garyx_mcp_server(
            "http://127.0.0.1:31337",
            "thread::alpha",
            "run-1",
            &metadata,
        )
        .expect("server config");

        assert_eq!(
            server.url,
            "http://127.0.0.1:31337/mcp/auth/secret%2Ftoken/thread%3A%3Aalpha/run-1"
        );
        assert_eq!(
            server.headers.get("X-Run-Id").map(String::as_str),
            Some("override")
        );
        assert_eq!(
            server.headers.get("X-Test").map(String::as_str),
            Some("value")
        );
        assert!(!server.headers.contains_key("X-Ignored"));
    }

    #[test]
    fn garyx_mcp_server_skips_blank_base_url() {
        assert!(garyx_mcp_server("  /// ", "thread", "run", &HashMap::new()).is_none());
    }
}
