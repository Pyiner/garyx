use std::collections::{HashMap, VecDeque};

use garyx_models::provider::ProviderRateLimit;
use serde_json::Value;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::gary_prompt::task_cli_env;

pub(crate) fn metadata_bool(metadata: &HashMap<String, Value>, key: &str) -> bool {
    metadata.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub(crate) fn metadata_string(metadata: &HashMap<String, Value>, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn metadata_string_map(
    metadata: &HashMap<String, Value>,
    key: &str,
) -> HashMap<String, String> {
    metadata
        .get(key)
        .and_then(Value::as_object)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(entry_key, entry_value)| {
                    entry_value
                        .as_str()
                        .map(|entry_value| (entry_key.clone(), entry_value.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn normalize_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn resolve_run_id_with(
    metadata: &HashMap<String, Value>,
    fallback: impl FnOnce() -> String,
) -> String {
    metadata
        .get("bridge_run_id")
        .and_then(Value::as_str)
        .or_else(|| metadata.get("client_run_id").and_then(Value::as_str))
        .or_else(|| metadata.get("run_id").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .unwrap_or_else(fallback)
}

pub(crate) fn resolve_uuid_run_id(metadata: &HashMap<String, Value>) -> String {
    resolve_run_id_with(metadata, || format!("run_{}", Uuid::new_v4()))
}

pub(crate) fn runtime_env_overlay(
    config_env: &HashMap<String, String>,
    metadata: &HashMap<String, Value>,
    extra_env_key: &str,
) -> HashMap<String, String> {
    let mut env = config_env.clone();
    env.extend(task_cli_env(metadata));
    env.extend(metadata_string_map(metadata, extra_env_key));
    env
}

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
    headers.extend(metadata_string_map(metadata, "garyx_mcp_headers"));

    let encoded_thread = urlencoding::encode(thread_id);
    let encoded_run = urlencoding::encode(run_id);
    let url = metadata_string(metadata, "garyx_mcp_auth_token")
        .map(|token| {
            format!(
                "{base_url}/mcp/auth/{}/{encoded_thread}/{encoded_run}",
                urlencoding::encode(&token)
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
    fn metadata_helpers_keep_existing_coercion_and_trimming_rules() {
        let metadata = HashMap::from([
            ("flag".to_owned(), Value::Bool(true)),
            ("text".to_owned(), Value::String(" value ".to_owned())),
            ("blank".to_owned(), Value::String("   ".to_owned())),
            (
                "map".to_owned(),
                json!({
                    "string": "kept",
                    "number": 42,
                }),
            ),
        ]);

        assert!(metadata_bool(&metadata, "flag"));
        assert!(!metadata_bool(&metadata, "text"));
        assert_eq!(metadata_string(&metadata, "text").as_deref(), Some("value"));
        assert_eq!(metadata_string(&metadata, "blank"), None);
        assert_eq!(
            metadata_string_map(&metadata, "map"),
            HashMap::from([("string".to_owned(), "kept".to_owned())])
        );
        assert_eq!(
            normalize_non_empty(Some(" value ")).as_deref(),
            Some("value")
        );
        assert_eq!(normalize_non_empty(Some("  ")), None);
    }

    #[test]
    fn resolve_run_id_keeps_priority_and_injected_fallback() {
        let metadata = HashMap::from([
            (
                "bridge_run_id".to_owned(),
                Value::String("bridge".to_owned()),
            ),
            (
                "client_run_id".to_owned(),
                Value::String("client".to_owned()),
            ),
            ("run_id".to_owned(), Value::String("run".to_owned())),
        ]);
        assert_eq!(
            resolve_run_id_with(&metadata, || "fallback".to_owned()),
            "bridge"
        );
        assert_eq!(
            resolve_run_id_with(
                &HashMap::from([("bridge_run_id".to_owned(), Value::String(String::new()),)]),
                || "fallback".to_owned(),
            ),
            "",
            "run id extraction intentionally does not normalize present strings"
        );
        assert_eq!(
            resolve_run_id_with(&HashMap::new(), || "fallback".to_owned()),
            "fallback"
        );
    }

    #[test]
    fn runtime_env_overlay_preserves_config_task_and_extra_precedence() {
        let config_env = HashMap::from([
            ("CONFIG_ONLY".to_owned(), "config".to_owned()),
            ("GARYX_CHANNEL".to_owned(), "config".to_owned()),
            ("GARYX_THREAD_ID".to_owned(), "config".to_owned()),
        ]);
        let metadata = HashMap::from([
            (
                "runtime_context".to_owned(),
                json!({
                    "thread_id": "thread::runtime",
                    "channel": "runtime-channel",
                }),
            ),
            (
                "provider_env".to_owned(),
                json!({
                    "GARYX_THREAD_ID": "extra-thread",
                    "EXTRA_ONLY": "extra",
                }),
            ),
        ]);

        let env = runtime_env_overlay(&config_env, &metadata, "provider_env");
        assert_eq!(env.get("CONFIG_ONLY").map(String::as_str), Some("config"));
        assert_eq!(
            env.get("GARYX_CHANNEL").map(String::as_str),
            Some("runtime-channel"),
            "task runtime env must override config env"
        );
        assert_eq!(
            env.get("GARYX_THREAD_ID").map(String::as_str),
            Some("extra-thread"),
            "extra provider env must override task runtime env"
        );
        assert_eq!(env.get("EXTRA_ONLY").map(String::as_str), Some("extra"));
    }

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
