use garyx_models::provider::ProviderMessage;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const COMPACTION_SUMMARY_PREFIX: &str =
    "The conversation history before this point was compacted into the following summary:";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextCompactionConfig {
    pub enabled: bool,
    pub context_window_tokens: usize,
    pub reserve_tokens: usize,
    pub keep_recent_tokens: usize,
}

impl ContextCompactionConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            context_window_tokens: 0,
            reserve_tokens: 0,
            keep_recent_tokens: 0,
        }
    }
}

impl Default for ContextCompactionConfig {
    fn default() -> Self {
        Self::disabled()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextUsageEstimate {
    pub tokens: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContextCompactionPlan {
    pub tokens_before: usize,
    pub first_kept_index: usize,
    pub messages_to_summarize: Vec<ProviderMessage>,
    pub kept_messages: Vec<ProviderMessage>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContextCompactionResult {
    pub summary: String,
    pub tokens_before: usize,
    pub summarized_message_count: usize,
    pub kept_message_count: usize,
    pub messages: Vec<ProviderMessage>,
}

pub fn estimate_message_tokens(message: &ProviderMessage) -> usize {
    let chars = message
        .text
        .as_ref()
        .map(|value| value.chars().count())
        .or_else(|| message.content.as_str().map(|value| value.chars().count()))
        .unwrap_or_else(|| message.content.to_string().chars().count());
    chars.div_ceil(4).max(1)
}

pub fn estimate_context_tokens(messages: &[ProviderMessage]) -> ContextUsageEstimate {
    ContextUsageEstimate {
        tokens: messages.iter().map(estimate_message_tokens).sum(),
    }
}

pub fn should_compact(tokens: usize, config: &ContextCompactionConfig) -> bool {
    config.enabled
        && config.context_window_tokens > config.reserve_tokens
        && tokens
            > config
                .context_window_tokens
                .saturating_sub(config.reserve_tokens)
}

pub fn build_compaction_plan(
    messages: &[ProviderMessage],
    config: &ContextCompactionConfig,
) -> Option<ContextCompactionPlan> {
    let tokens_before = estimate_context_tokens(messages).tokens;
    if !should_compact(tokens_before, config) || messages.len() < 2 {
        return None;
    }

    let mut accumulated = 0usize;
    let mut first_kept_index = messages.len().saturating_sub(1);
    for index in (0..messages.len()).rev() {
        accumulated = accumulated.saturating_add(estimate_message_tokens(&messages[index]));
        first_kept_index = index;
        if accumulated >= config.keep_recent_tokens.max(1) {
            break;
        }
    }

    while first_kept_index > 0 && messages[first_kept_index].role_str() == "tool_result" {
        first_kept_index -= 1;
    }

    if first_kept_index == 0 {
        return None;
    }

    Some(ContextCompactionPlan {
        tokens_before,
        first_kept_index,
        messages_to_summarize: messages[..first_kept_index].to_vec(),
        kept_messages: messages[first_kept_index..].to_vec(),
    })
}

pub fn compaction_summary_message(
    summary: impl Into<String>,
    tokens_before: usize,
) -> ProviderMessage {
    let summary = summary.into();
    let mut message = ProviderMessage::system_text(format!(
        "{COMPACTION_SUMMARY_PREFIX}\n\n<summary>\n{summary}\n</summary>"
    ));
    message
        .metadata
        .insert("garyx_compaction".to_owned(), json!(true));
    message
        .metadata
        .insert("tokens_before".to_owned(), json!(tokens_before));
    message
}

pub fn compact_messages_with_summary(
    messages: &[ProviderMessage],
    summary: impl Into<String>,
    config: &ContextCompactionConfig,
) -> Option<ContextCompactionResult> {
    let plan = build_compaction_plan(messages, config)?;
    let summary = summary.into();
    let summary_message = compaction_summary_message(summary.clone(), plan.tokens_before);
    let summarized_message_count = plan.messages_to_summarize.len();
    let kept_message_count = plan.kept_messages.len();
    let mut compacted = Vec::with_capacity(1 + kept_message_count);
    compacted.push(summary_message);
    compacted.extend(plan.kept_messages);

    Some(ContextCompactionResult {
        summary,
        tokens_before: plan.tokens_before,
        summarized_message_count,
        kept_message_count,
        messages: compacted,
    })
}

pub fn serialize_messages_for_summary(messages: &[ProviderMessage]) -> String {
    messages
        .iter()
        .map(|message| {
            let content = message
                .text
                .as_deref()
                .or_else(|| message.content.as_str())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| message.content.to_string());
            format!("{}: {content}", message.role_str())
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn compact_policy() -> ContextCompactionConfig {
        ContextCompactionConfig {
            enabled: true,
            context_window_tokens: 16,
            reserve_tokens: 4,
            keep_recent_tokens: 5,
        }
    }

    #[test]
    fn estimates_provider_message_tokens() {
        let message = ProviderMessage::user_text("abcdefghijkl");

        assert_eq!(estimate_message_tokens(&message), 3);
    }

    #[test]
    fn builds_compaction_plan_when_context_exceeds_budget() {
        let messages = vec![
            ProviderMessage::user_text("old user message that is long"),
            ProviderMessage::assistant_text("old assistant message that is long"),
            ProviderMessage::user_text("recent question"),
            ProviderMessage::assistant_text("recent answer"),
        ];

        let plan = build_compaction_plan(&messages, &compact_policy()).unwrap();

        assert!(plan.tokens_before > 12);
        assert!(!plan.messages_to_summarize.is_empty());
        assert!(!plan.kept_messages.is_empty());
        assert_eq!(messages[plan.first_kept_index].role_str(), "user");
    }

    #[test]
    fn compaction_result_prepends_summary_message_and_keeps_recent_messages() {
        let messages = vec![
            ProviderMessage::user_text("old user message that is long"),
            ProviderMessage::assistant_text("old assistant message that is long"),
            ProviderMessage::user_text("recent question"),
            ProviderMessage::assistant_text("recent answer"),
        ];

        let result =
            compact_messages_with_summary(&messages, "important old facts", &compact_policy())
                .unwrap();

        assert_eq!(result.messages[0].role_str(), "system");
        assert_eq!(
            result.messages[0].metadata.get("garyx_compaction"),
            Some(&json!(true))
        );
        assert!(
            result.messages[0]
                .text
                .as_deref()
                .unwrap()
                .contains("important old facts")
        );
        assert_eq!(result.kept_message_count, 2);
    }

    #[test]
    fn compaction_cut_does_not_start_with_tool_result() {
        let messages = vec![
            ProviderMessage::user_text("old user message that is long"),
            ProviderMessage::assistant_text("old assistant message that is long"),
            ProviderMessage::tool_use(
                json!({"name": "read_file", "arguments": {}}),
                Some("call-1".to_owned()),
                Some("read_file".to_owned()),
            ),
            ProviderMessage::tool_result(
                json!({"content": "recent result"}),
                Some("call-1".to_owned()),
                Some("read_file".to_owned()),
                Some(false),
            ),
        ];
        let config = ContextCompactionConfig {
            keep_recent_tokens: 2,
            ..compact_policy()
        };

        let plan = build_compaction_plan(&messages, &config).unwrap();

        assert_ne!(messages[plan.first_kept_index].role_str(), "tool_result");
    }
}
