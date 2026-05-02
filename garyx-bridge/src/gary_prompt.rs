use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use crate::memory_context::build_memory_context_user_message;

pub(crate) const GARY_BASE_INSTRUCTIONS: &str = concat!(
    "Garyx runtime guidance:\n",
    "\n",
    "Self-evolution:\n",
    "- Durable memory is delivered as a wrapped user message at the start of a thread. Use it as background context and update the referenced memory.md files when durable facts or better workflows emerge.\n",
    "- Skills live in ~/.garyx/skills/<skill-id>/SKILL.md and sync into ~/.claude/skills and ~/.codex/skills. If you solve a recurring problem or discover a better workflow, improve the relevant skill and validate it with a focused test.\n",
    "\n",
    "System capabilities:\n",
    "- Delegate work with tasks, for example: `garyx task create --title \"...\" --body \"...\" --assignee <agent_id>`; inspect with `garyx task get <task_ref>` and update with `garyx task update <task_ref> --status in_review|done`.\n",
    "- Manage scheduled automations with the CLI, for example: `garyx automation create --label \"Daily triage\" --prompt \"...\" --workspace-dir /path --every-hours 24`; then use `garyx automation list|get|update|pause|resume|run|delete`.\n",
    "- Inspect runtime issues with product-domain commands such as `garyx thread history <thread_id> --limit 200 --json`, `garyx bot status`, and `garyx logs tail`.\n",
    "- If you restart the managed gateway while working as an agent, queue a wake: `garyx gateway restart --wake thread <thread_id> --wake-message \"continue\"`. Use `--no-wake` only when continuation is intentionally unnecessary.\n",
);

pub(crate) fn compose_gary_instructions(
    extra: Option<&str>,
    _workspace_dir: Option<&Path>,
    _automation_id: Option<&str>,
) -> String {
    compose_gary_instructions_with_layout(extra)
}

pub(crate) fn prepend_memory_context_to_user_message(
    message: &str,
    metadata: &HashMap<String, Value>,
    include_memory: bool,
) -> String {
    if !include_memory {
        return message.to_owned();
    }
    let memory = build_memory_context_user_message(metadata);
    if message.trim().is_empty() {
        memory
    } else {
        format!("{memory}\n\n{message}")
    }
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

fn compose_gary_instructions_with_layout(extra: Option<&str>) -> String {
    let trimmed_extra = extra.map(str::trim).filter(|value| !value.is_empty());
    let mut sections = vec![GARY_BASE_INSTRUCTIONS.trim_end().to_owned()];

    if let Some(extra) = trimmed_extra {
        sections.push(format!("Additional runtime instructions:\n{extra}"));
    }

    sections.join("\n\n")
}

#[cfg(test)]
mod tests;
