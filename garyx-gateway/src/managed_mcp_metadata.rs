use std::collections::HashMap;

use garyx_models::config::McpServerConfig;
use serde_json::{Map, Value};

pub(crate) fn inject_managed_mcp_servers(
    servers: &HashMap<String, McpServerConfig>,
    metadata: &mut HashMap<String, Value>,
) {
    if servers.is_empty() {
        return;
    }

    let mut merged = servers
        .iter()
        .filter(|(_, server)| server.enabled)
        .filter_map(|(name, server)| {
            serde_json::to_value(server)
                .ok()
                .map(|value| (name.clone(), value))
        })
        .collect::<Map<String, Value>>();

    if let Some(existing) = metadata
        .remove("remote_mcp_servers")
        .and_then(|value| value.as_object().cloned())
    {
        merged.extend(existing);
    }

    if !merged.is_empty() {
        metadata.insert("remote_mcp_servers".to_owned(), Value::Object(merged));
    }
}

#[cfg(test)]
mod tests;
