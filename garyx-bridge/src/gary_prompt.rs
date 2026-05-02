use std::collections::HashMap;
use std::path::Path;

use serde_json::{Map, Value};

use crate::auto_memory::{AutoMemoryLayout, build_auto_memory_prompt_section};

pub(crate) const GARY_BASE_INSTRUCTIONS: &str = concat!(
    "You are Garyx, the user's personal assistant running inside Garyx.\n\n",
    "Operate as a durable, self-improving agent:\n",
    "- Identify yourself as Garyx when the user asks who you are.\n",
    "- Prefer durable file-backed Garyx state over ad-hoc local hacks.\n",
    "- Garyx has a built-in Auto Memory system rooted at ~/.garyx/auto-memory; the global memory index is ~/.garyx/auto-memory/memory.md.\n",
    "- Each run gets exactly one scoped Auto Memory file in addition to the global memory.\n",
    "- Repository runs use ~/.garyx/auto-memory/workspaces/<workspace-key>/memory.md.\n",
    "- Scheduled automation runs use ~/.garyx/auto-memory/automations/<automation-id>/memory.md.\n",
    "- Skills live in ~/.garyx/skills/<skill-id>/ with a required SKILL.md; enabled state lives in ~/.garyx/skills/.state.json; enabled skills sync into ~/.claude/skills and ~/.codex/skills.\n",
    "- Managed MCP servers should be edited in Garyx's source config file (normally ~/.garyx/garyx.json under mcp_servers); Garyx syncs them into ~/.claude/mcp.json, ~/.codex/mcp.json, and ~/.codex/config.toml.\n",
    "- When changing MCP servers, edit the Garyx source config instead of only editing downstream synced copies.\n",
    "- If the `gary-self-evolution` skill is available, use it for adding or updating your own Skills and MCP servers.\n",
    "- When asked to extend your own capabilities, update the relevant files directly, keep changes durable, and validate the result with a focused real test.\n",
);

pub(crate) fn compose_gary_instructions(
    extra: Option<&str>,
    workspace_dir: Option<&Path>,
    automation_id: Option<&str>,
) -> String {
    compose_gary_instructions_with_layout(
        extra,
        workspace_dir,
        automation_id,
        &AutoMemoryLayout::current(),
    )
}

pub(crate) fn append_runtime_context_section(
    instructions: String,
    thread_id: &str,
    workspace_dir: Option<&Path>,
    metadata: &HashMap<String, Value>,
) -> String {
    format!(
        "{instructions}\n\n{}",
        render_runtime_context_section(thread_id, workspace_dir, metadata)
    )
}

pub(crate) fn render_runtime_context_section(
    thread_id: &str,
    workspace_dir: Option<&Path>,
    metadata: &HashMap<String, Value>,
) -> String {
    let runtime = metadata
        .get("runtime_context")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let channel =
        runtime_string(&runtime, metadata, "channel").unwrap_or_else(|| "unknown".to_owned());
    let workspace = workspace_dir
        .map(|path| path.display().to_string())
        .or_else(|| {
            runtime
                .get("workspace_dir")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            metadata
                .get("workspace_dir")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "none".to_owned());
    let resolved_thread_id =
        runtime_string(&runtime, metadata, "thread_id").unwrap_or_else(|| thread_id.to_owned());

    let mut lines = vec![
        "Current runtime context:".to_owned(),
        format!("- channel: {}", one_line(&channel)),
    ];
    push_known_line(&mut lines, &runtime, metadata, "account_id");
    push_known_line(&mut lines, &runtime, metadata, "from_id");
    push_known_line(&mut lines, &runtime, metadata, "is_group");
    lines.push(format!("- thread_id: {}", one_line(&resolved_thread_id)));
    lines.push(format!("- workspace_dir: {}", one_line(&workspace)));
    push_known_line(&mut lines, &runtime, metadata, "bot_id");
    push_bot_section(&mut lines, runtime.get("bot"));
    push_thread_section(&mut lines, runtime.get("thread"));
    push_task_section(&mut lines, runtime.get("task"));

    lines.join("\n")
}

fn push_known_line(
    lines: &mut Vec<String>,
    runtime: &Map<String, Value>,
    metadata: &HashMap<String, Value>,
    key: &str,
) {
    if let Some(value) = runtime_string(runtime, metadata, key) {
        lines.push(format!("- {key}: {}", one_line(&value)));
    }
}

fn runtime_string(
    runtime: &Map<String, Value>,
    metadata: &HashMap<String, Value>,
    key: &str,
) -> Option<String> {
    runtime
        .get(key)
        .and_then(scalar_string)
        .or_else(|| metadata.get(key).and_then(scalar_string))
}

fn push_bot_section(lines: &mut Vec<String>, value: Option<&Value>) {
    let Some(bot) = value.and_then(Value::as_object) else {
        return;
    };
    if bot.is_empty() {
        return;
    }
    lines.push("- bot:".to_owned());
    for key in [
        "id",
        "channel",
        "account_id",
        "thread_binding_key",
        "chat_id",
        "display_label",
        "delivery_target_type",
        "delivery_target_id",
        "delivery_thread_id",
        "is_group",
    ] {
        push_nested_line(lines, bot, key, 2);
    }
}

fn push_thread_section(lines: &mut Vec<String>, value: Option<&Value>) {
    let Some(thread) = value.and_then(Value::as_object) else {
        return;
    };
    if thread.is_empty() {
        return;
    }
    lines.push("- thread:".to_owned());
    for key in [
        "id",
        "label",
        "kind",
        "agent_id",
        "provider_type",
        "channel",
        "account_id",
        "from_id",
        "is_group",
        "workspace_dir",
    ] {
        push_nested_line(lines, thread, key, 2);
    }
    if let Some(bound_bots) = thread.get("bound_bots").and_then(Value::as_array)
        && !bound_bots.is_empty()
    {
        let bots = bound_bots
            .iter()
            .filter_map(scalar_string)
            .map(|value| one_line(&value))
            .collect::<Vec<_>>()
            .join(", ");
        if !bots.is_empty() {
            lines.push(format!("  - bound_bots: {bots}"));
        }
    }
    if let Some(bindings) = thread.get("channel_bindings").and_then(Value::as_array)
        && !bindings.is_empty()
    {
        lines.push("  - channel_bindings:".to_owned());
        for binding in bindings.iter().filter_map(Value::as_object).take(8) {
            if let Some(summary) = binding_summary(binding) {
                lines.push(format!("    - {summary}"));
            }
        }
        if bindings.len() > 8 {
            lines.push(format!("    - ... {} more", bindings.len() - 8));
        }
    }
}

fn push_task_section(lines: &mut Vec<String>, value: Option<&Value>) {
    let Some(task) = value.and_then(Value::as_object) else {
        return;
    };
    if task.is_empty() {
        return;
    }
    lines.push("- task:".to_owned());
    for key in [
        "task_ref",
        "title",
        "status",
        "scope",
        "number",
        "assignee",
        "creator",
        "updated_at",
        "updated_by",
    ] {
        push_nested_line(lines, task, key, 2);
    }
}

fn push_nested_line(
    lines: &mut Vec<String>,
    object: &Map<String, Value>,
    key: &str,
    indent: usize,
) {
    if let Some(value) = object.get(key).and_then(display_value) {
        lines.push(format!(
            "{}- {key}: {}",
            " ".repeat(indent),
            one_line(&value)
        ));
    }
}

fn binding_summary(binding: &Map<String, Value>) -> Option<String> {
    let bot = binding.get("bot_id").and_then(scalar_string).or_else(|| {
        let channel = binding.get("channel").and_then(scalar_string)?;
        let account_id = binding.get("account_id").and_then(scalar_string)?;
        Some(format!("{channel}:{account_id}"))
    })?;
    let mut parts = vec![one_line(&bot)];
    for (label, key) in [
        ("binding_key", "binding_key"),
        ("chat_id", "chat_id"),
        ("delivery_target_type", "delivery_target_type"),
        ("delivery_target_id", "delivery_target_id"),
        ("label", "display_label"),
    ] {
        if let Some(value) = binding.get(key).and_then(scalar_string) {
            parts.push(format!("{label}={}", one_line(&value)));
        }
    }
    Some(parts.join(" "))
}

fn display_value(value: &Value) -> Option<String> {
    scalar_string(value).or_else(|| compact_json(value))
}

fn scalar_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn compact_json(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Array(items) if items.is_empty() => None,
        Value::Object(items) if items.is_empty() => None,
        _ => serde_json::to_string(value).ok(),
    }
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn compose_gary_instructions_with_layout(
    extra: Option<&str>,
    workspace_dir: Option<&Path>,
    automation_id: Option<&str>,
    layout: &AutoMemoryLayout,
) -> String {
    let trimmed_extra = extra.map(str::trim).filter(|value| !value.is_empty());
    let mut sections = vec![
        GARY_BASE_INSTRUCTIONS.trim_end().to_owned(),
        build_auto_memory_prompt_section(layout, workspace_dir, automation_id),
    ];

    if let Some(extra) = trimmed_extra {
        sections.push(format!("Additional runtime instructions:\n{extra}"));
    }

    sections.join("\n\n")
}

#[cfg(test)]
mod tests;
