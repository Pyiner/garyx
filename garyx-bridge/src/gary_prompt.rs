use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use crate::auto_memory::{AutoMemoryLayout, build_auto_memory_prompt_section};

pub(crate) const GARY_BASE_INSTRUCTIONS: &str = concat!(
    "Operate as a durable, self-improving agent:\n",
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
    "- Scheduled automations are managed with the `garyx automation` CLI. Do not use MCP tools for automation scheduling or management.\n",
    "\n",
    "Task workflow:\n",
    "- Garyx has one human user plus multiple agent principals. Agent principals are written as `agent:<agent_id>`; the human user is written as `human:<user_id>`.\n",
    "- In task threads, user messages may end with a live task snapshot like `[task #TASK-12 status=in_progress assignee=agent:reviewer]`. Treat that suffix as the current task state for this turn.\n",
    "- Assigned tasks are already started; do not manually move a task to in_progress just because it has been assigned to you.\n",
    "- Use the `garyx task` CLI for task state changes: `garyx task update <task_ref> --status in_review`, `garyx task update <task_ref> --status done`, `garyx task claim <task_ref>` for unassigned work, or `garyx task release <task_ref>`.\n",
    "- Do not use Garyx MCP task tools for normal task state changes; the CLI is the task control path for agents.\n",
    "- If you restart the managed gateway while working as an agent, you must queue a wake before restarting so this thread can resume: `garyx gateway restart --wake thread <thread_id> --wake-message \"continue\"`. Use `--no-wake` only when the user explicitly wants a restart with no agent continuation.\n",
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

pub(crate) fn append_task_suffix_to_user_message(
    message: &str,
    metadata: &HashMap<String, Value>,
) -> String {
    let Some(suffix) = task_suffix(metadata) else {
        return message.to_owned();
    };
    let message = message.trim_end();
    if message.is_empty() {
        suffix
    } else {
        format!("{message} {suffix}")
    }
}

pub(crate) fn task_cli_env(metadata: &HashMap<String, Value>) -> HashMap<String, String> {
    let Some(runtime) = metadata.get("runtime_context").and_then(Value::as_object) else {
        return HashMap::new();
    };
    let mut env = HashMap::new();
    if let Some(thread_id) = runtime.get("thread_id").and_then(scalar_string) {
        env.insert("GARYX_THREAD_ID".to_owned(), thread_id);
    }
    let agent_id = metadata
        .get("agent_id")
        .and_then(scalar_string)
        .or_else(|| {
            runtime
                .get("thread")
                .and_then(Value::as_object)
                .and_then(|thread| thread.get("agent_id"))
                .and_then(scalar_string)
        });
    if let Some(agent_id) = agent_id {
        env.insert("GARYX_AGENT_ID".to_owned(), agent_id.clone());
        env.insert("GARYX_ACTOR".to_owned(), format!("agent:{agent_id}"));
    }
    if let Some(task) = runtime.get("task").and_then(Value::as_object) {
        if let Some(task_ref) = task.get("task_ref").and_then(scalar_string) {
            env.insert("GARYX_TASK_REF".to_owned(), task_ref);
        }
        if let Some(scope) = task.get("scope").and_then(scalar_string) {
            env.insert("GARYX_TASK_SCOPE".to_owned(), scope);
        }
        if let Some(status) = task.get("status").and_then(scalar_string) {
            env.insert("GARYX_TASK_STATUS".to_owned(), status);
        }
    }
    env
}

fn task_suffix(metadata: &HashMap<String, Value>) -> Option<String> {
    let runtime = metadata.get("runtime_context").and_then(Value::as_object)?;
    let task = runtime.get("task").and_then(Value::as_object)?;
    let task_ref = task.get("task_ref").and_then(scalar_string).or_else(|| {
        task.get("number")
            .and_then(scalar_string)
            .map(|number| format!("#{number}"))
    })?;
    let status = task.get("status").and_then(scalar_string)?;

    let mut suffix = format!("[task {} status={}", one_line(&task_ref), one_line(&status));
    if let Some(assignee) = task.get("assignee").and_then(principal_label) {
        suffix.push_str(&format!(" assignee={}", one_line(&assignee)));
    }
    suffix.push(']');
    Some(suffix)
}

fn principal_label(value: &Value) -> Option<String> {
    if let Some(label) = scalar_string(value) {
        return Some(label);
    }
    let principal = value.as_object()?;
    match principal
        .get("kind")
        .or_else(|| principal.get("type"))
        .and_then(Value::as_str)?
    {
        "human" => principal
            .get("user_id")
            .and_then(scalar_string)
            .map(|value| format!("human:{value}")),
        "agent" => principal
            .get("agent_id")
            .and_then(scalar_string)
            .map(|value| format!("agent:{value}")),
        other => Some(other.to_owned()),
    }
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
