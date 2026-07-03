use super::*;

pub(super) fn summarize_text(value: &str, limit: usize) -> String {
    let sanitized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = sanitized.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.chars().count() <= limit {
        return trimmed.to_owned();
    }
    let mut clipped = trimmed
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    clipped.push('…');
    clipped
}

pub(super) fn normalize_provider_thread_title(value: &str) -> Option<String> {
    let title = summarize_text(value, 80);
    (!title.is_empty()).then_some(title)
}

pub(super) fn api_route_placeholder_label(existing: &Value) -> Option<String> {
    let channel = existing.get("channel").and_then(Value::as_str)?.trim();
    let account_id = existing.get("account_id").and_then(Value::as_str)?.trim();
    let from_id = existing.get("from_id").and_then(Value::as_str)?.trim();
    if channel != "api" || account_id.is_empty() || from_id.is_empty() {
        return None;
    }
    Some(format!("{channel}/{account_id}/{from_id}"))
}

pub(super) fn should_apply_provider_thread_title(existing: &Value) -> bool {
    if existing
        .get("thread_title_source")
        .and_then(Value::as_str)
        .map(str::trim)
        == Some(PROMPT_THREAD_TITLE_SOURCE)
    {
        return true;
    }

    let Some(label) = existing.get("label").and_then(Value::as_str) else {
        return true;
    };
    let trimmed = label.trim();
    trimmed.is_empty()
        || trimmed == LEGACY_DEFAULT_THREAD_LABEL
        || api_route_placeholder_label(existing).as_deref() == Some(trimmed)
}

pub(super) async fn persist_provider_thread_title_if_missing(
    store: &Arc<dyn ThreadStore>,
    thread_id: &str,
    title: Option<&str>,
) -> Option<String> {
    let title = title.and_then(normalize_provider_thread_title)?;
    let mut value = store.get(thread_id).await?;
    if !should_apply_provider_thread_title(&value) {
        return None;
    }
    let obj = value.as_object_mut()?;
    obj.insert("label".to_owned(), Value::String(title.clone()));
    obj.insert(
        "provider_thread_title".to_owned(),
        Value::String(title.clone()),
    );
    obj.insert(
        "thread_title_source".to_owned(),
        Value::String(PROVIDER_THREAD_TITLE_SOURCE.to_owned()),
    );
    obj.insert(
        "updated_at".to_owned(),
        Value::String(chrono::Utc::now().to_rfc3339()),
    );
    store.set(thread_id, value).await;
    Some(title)
}

pub(super) fn forward_applied_thread_title_update(
    external_callback: Option<&Arc<dyn Fn(StreamEvent) + Send + Sync>>,
    applied_thread_title: Option<&str>,
) {
    if let Some(title) = applied_thread_title
        && let Some(callback) = external_callback
    {
        callback(StreamEvent::ThreadTitleUpdated {
            title: title.to_owned(),
        });
    }
}
