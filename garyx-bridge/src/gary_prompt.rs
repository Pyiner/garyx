use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

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
    let runtime = metadata
        .get("runtime_context")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let channel = runtime
        .get("channel")
        .and_then(Value::as_str)
        .or_else(|| metadata.get("channel").and_then(Value::as_str))
        .unwrap_or("unknown");
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

    format!(
        "{instructions}\n\nCurrent runtime context:\n- channel: {channel}\n- thread_id: {thread_id}\n- workspace_dir: {workspace}"
    )
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
