use super::*;

pub(super) fn resolve_sdk_session_id_for_persistence(
    metadata: &HashMap<String, Value>,
    result_sdk_session_id: Option<&str>,
) -> Option<String> {
    result_sdk_session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            metadata
                .get(SDK_SESSION_ID_METADATA_KEY)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
}

pub(super) fn non_empty_value_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn persisted_provider_type(session_data: &Value) -> Option<ProviderType> {
    let raw = session_data.get("provider_type")?.clone();
    serde_json::from_value(raw.clone())
        .map_err(
            |e| tracing::debug!(raw = %raw, error = %e, "failed to parse persisted provider_type"),
        )
        .ok()
}

pub(super) fn provider_types_share_native_session(
    left: &ProviderType,
    right: &ProviderType,
) -> bool {
    left == right
}

pub(super) fn resolve_persisted_sdk_session_id_for_provider(
    session_data: &Value,
    provider_key: &str,
    provider_type: Option<&ProviderType>,
) -> Option<String> {
    let object = session_data.as_object()?;

    if let Some(expected_provider_type) = provider_type
        && persisted_provider_type(session_data)
            .as_ref()
            .is_some_and(|persisted| {
                provider_types_share_native_session(persisted, expected_provider_type)
            })
        && let Some(sdk_session_id) = non_empty_value_string(object.get("sdk_session_id"))
    {
        return Some(sdk_session_id);
    }

    let trimmed_provider_key = provider_key.trim();
    if trimmed_provider_key.is_empty() {
        return None;
    }

    if let Some(provider_scoped_session_id) = object
        .get("provider_sdk_session_ids")
        .and_then(Value::as_object)
        .and_then(|map| map.get(trimmed_provider_key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(provider_scoped_session_id.to_owned());
    }

    let stored_provider_key = non_empty_value_string(object.get("provider_key"));
    if stored_provider_key.as_deref() != Some(trimmed_provider_key) {
        return None;
    }

    non_empty_value_string(object.get("sdk_session_id"))
}

pub(super) fn provider_type_from_metadata_value(value: Option<&Value>) -> Option<ProviderType> {
    let raw = value?.clone();
    serde_json::from_value(raw.clone())
        .map_err(|e| tracing::debug!(raw = %raw, error = %e, "failed to parse fork provider_type"))
        .ok()
}

pub(super) fn resolve_fork_sdk_session_id_for_provider(
    session_data: &Value,
    provider_type: &ProviderType,
) -> Option<String> {
    let metadata = session_data.get("metadata").and_then(Value::as_object)?;
    if metadata
        .get(SDK_SESSION_FORK_METADATA_KEY)
        .and_then(Value::as_bool)
        != Some(true)
    {
        return None;
    }
    let fork_provider_type =
        provider_type_from_metadata_value(metadata.get(FORK_FROM_PROVIDER_TYPE_METADATA_KEY))?;
    if !provider_types_share_native_session(&fork_provider_type, provider_type) {
        return None;
    }
    non_empty_value_string(metadata.get(FORK_FROM_SDK_SESSION_ID_METADATA_KEY))
}

pub(super) fn attach_provider_sdk_session_metadata(
    options: &mut ProviderRunOptions,
    session_data: &Value,
    provider_key: &str,
    provider_type: &ProviderType,
) {
    // Thread metadata is copied into dispatch metadata before this point. Clear
    // fork mode first so a child thread that has already bound its own provider
    // session resumes normally instead of forking from the parent every turn.
    options.metadata.remove(SDK_SESSION_FORK_METADATA_KEY);

    if let Some(sid) = resolve_persisted_sdk_session_id_for_provider(
        session_data,
        provider_key,
        Some(provider_type),
    ) {
        options
            .metadata
            .insert(SDK_SESSION_ID_METADATA_KEY.to_owned(), Value::String(sid));
        return;
    }

    if let Some(parent_sid) = resolve_fork_sdk_session_id_for_provider(session_data, provider_type)
    {
        options.metadata.insert(
            SDK_SESSION_ID_METADATA_KEY.to_owned(),
            Value::String(parent_sid),
        );
        options
            .metadata
            .insert(SDK_SESSION_FORK_METADATA_KEY.to_owned(), Value::Bool(true));
    }
}

pub(super) fn persisted_provider_messages_from_thread(
    session_data: &Value,
) -> Vec<ProviderMessage> {
    let committed = session_data
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut messages = Vec::with_capacity(committed.len());
    for value in &committed {
        if let Some(message) = ProviderMessage::from_value(value) {
            messages.push(message);
        }
    }
    messages
}

pub(super) fn attach_native_session_messages(
    options: &mut ProviderRunOptions,
    session_data: &Value,
    provider_type: &ProviderType,
) {
    if !matches!(
        provider_type,
        ProviderType::Gpt | ProviderType::ClaudeLlm | ProviderType::GeminiLlm
    ) {
        return;
    }
    let messages = persisted_provider_messages_from_thread(session_data);
    if messages.is_empty() {
        return;
    }
    options.metadata.insert(
        SESSION_MESSAGES_METADATA_KEY.to_owned(),
        serde_json::to_value(messages).unwrap_or(Value::Array(Vec::new())),
    );
}
