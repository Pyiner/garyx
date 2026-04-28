use garyx_models::config::{FeishuAccount, TopicSessionMode};

/// The simplified Feishu policy is intentionally fixed:
/// - direct messages are accepted
/// - group messages are accepted
/// - group replies require an @mention only when `require_mention` is true
pub fn is_dm_message_allowed() -> bool {
    true
}

pub fn is_group_message_allowed() -> bool {
    true
}

pub fn requires_group_mention(account: &FeishuAccount) -> bool {
    account.require_mention
}

pub fn resolve_topic_session_mode(account: &FeishuAccount) -> TopicSessionMode {
    account.topic_session_mode.clone()
}

pub fn apply_mention_context_limit(history: &mut Vec<String>, limit: i64) {
    if limit <= 0 {
        return;
    }
    let limit = limit as usize;
    if history.len() > limit {
        let drain_count = history.len() - limit;
        history.drain(..drain_count);
    }
}
