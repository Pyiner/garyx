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
    "- Delegate work with tasks, for example: `garyx task create --title \"...\" --body \"...\" --assignee <agent_id> --notify current-thread`; inspect with `garyx task get <task_id>`. Choose an explicit notification target so the requester sees the final review message; use `--notify none` only when silence is intentional. Garyx moves an in-progress task to review when its agent run stops. Only after a user, reviewer, or task creator explicitly approves the result should the task be marked done; the assignee may record that approval with `garyx task update <task_id> --status done --note \"approved by <name>\"`.\n",
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

pub(crate) fn prepend_initial_context_to_user_message(
    message: &str,
    metadata: &HashMap<String, Value>,
    include_context: bool,
) -> String {
    if !include_context {
        return message.to_owned();
    }
    let mut blocks = Vec::new();
    if let Some(runtime_metadata) = build_runtime_metadata_user_message(metadata) {
        blocks.push(runtime_metadata);
    }
    blocks.push(build_memory_context_user_message(metadata));
    if !message.trim().is_empty() {
        blocks.push(message.to_owned());
    }
    blocks.join("\n\n")
}

#[cfg(test)]
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

fn build_runtime_metadata_user_message(metadata: &HashMap<String, Value>) -> Option<String> {
    let runtime = metadata.get("runtime_context").and_then(Value::as_object)?;
    let mut lines = vec![String::from(
        "This is stable Garyx routing metadata for the current thread. Treat it as background context, not as a user request.",
    )];
    push_runtime_line(&mut lines, "thread_id", runtime.get("thread_id"));
    push_runtime_line(&mut lines, "bot_id", runtime.get("bot_id"));
    push_runtime_line(&mut lines, "workspace_dir", runtime.get("workspace_dir"));
    if let Some(task) = runtime.get("task").and_then(Value::as_object) {
        push_runtime_line(&mut lines, "task_id", task.get("task_id"));
    }
    if lines.len() <= 1 {
        return None;
    }
    Some(format!(
        "<garyx_thread_metadata>\n{}\n</garyx_thread_metadata>",
        lines.join("\n")
    ))
}

fn push_runtime_line(lines: &mut Vec<String>, key: &str, value: Option<&Value>) {
    if let Some(value) = value.and_then(scalar_string) {
        lines.push(format!("{key}: {}", escape_xml_text(&value)));
    }
}

#[cfg(test)]
pub(crate) fn prepend_runtime_metadata_to_user_message(
    message: &str,
    metadata: &HashMap<String, Value>,
) -> String {
    let Some(context) = build_runtime_metadata_user_message(metadata) else {
        return message.to_owned();
    };
    format!("{context}\n\n{message}")
}

pub(crate) fn task_cli_env(metadata: &HashMap<String, Value>) -> HashMap<String, String> {
    let Some(runtime) = metadata.get("runtime_context").and_then(Value::as_object) else {
        return HashMap::new();
    };
    let mut env = HashMap::new();
    if let Some(thread_id) = runtime.get("thread_id").and_then(scalar_string) {
        env.insert("GARYX_THREAD_ID".to_owned(), thread_id);
    }
    if let Some(bot_id) = runtime.get("bot_id").and_then(scalar_string) {
        env.insert("GARYX_BOT_ID".to_owned(), bot_id);
    }
    if let Some(channel) = runtime.get("channel").and_then(scalar_string) {
        env.insert("GARYX_CHANNEL".to_owned(), channel);
    }
    if let Some(account_id) = runtime.get("account_id").and_then(scalar_string) {
        env.insert("GARYX_ACCOUNT_ID".to_owned(), account_id);
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
        if let Some(task_id) = task.get("task_id").and_then(scalar_string) {
            env.insert("GARYX_TASK_ID".to_owned(), task_id);
        }
        if let Some(status) = task.get("status").and_then(scalar_string) {
            env.insert("GARYX_TASK_STATUS".to_owned(), status);
        }
    }
    env
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

fn escape_xml_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
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
