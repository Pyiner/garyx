use std::collections::HashMap;

use serde_json::Value;

fn skill_id(metadata: &HashMap<String, Value>) -> Option<&str> {
    metadata
        .get("slash_command_skill_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(crate) fn build_native_skill_prompt(
    message: &str,
    metadata: &HashMap<String, Value>,
) -> Option<String> {
    let skill_id = skill_id(metadata)?;
    let trimmed = message.trim();

    if trimmed.starts_with('/') {
        return Some(message.to_owned());
    }

    if trimmed.is_empty() {
        Some(format!("/{skill_id}"))
    } else {
        Some(format!("/{skill_id}\n\n{message}"))
    }
}

#[cfg(test)]
mod tests;
