use super::*;

pub(super) fn trim_required_cli(
    value: &str,
    field: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} cannot be empty").into());
    }
    Ok(trimmed.to_owned())
}

pub(super) fn trim_optional_cli(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub(super) fn committed_message(event: &Value) -> Option<&Value> {
    (event.get("type").and_then(Value::as_str) == Some("committed_message"))
        .then(|| event.get("message"))
        .flatten()
}

pub(super) fn committed_assistant_text(event: &Value) -> Option<&str> {
    let message = committed_message(event)?;
    (message.get("role").and_then(Value::as_str) == Some("assistant"))
        .then(|| {
            message
                .get("text")
                .and_then(Value::as_str)
                .or_else(|| message.get("content").and_then(Value::as_str))
        })
        .flatten()
}

pub(super) fn committed_control_kind(event: &Value) -> Option<&str> {
    committed_message(event)?
        .get("control")
        .and_then(|control| control.get("kind"))
        .and_then(Value::as_str)
}
