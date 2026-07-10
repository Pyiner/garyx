use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use crate::memory_context::build_memory_context_user_message;

pub(crate) const GARY_BASE_INSTRUCTIONS: &str = concat!(
    "Garyx runtime guidance:\n",
    "\n",
    "Self-evolution:\n",
    "- When you learn something durable — a user preference, a recurring workflow, a project convention, or a better way to do a task — decide where it belongs and write it down before the run ends. Don't let the learning evaporate.\n",
    "- Agent memory is what you carry on your back every run, so keep it small: this agent's behavior tuning, the requester's stable preferences, and facts you genuinely need every time you and this user work together. When Garyx provides a wrapped `<garyx_memory_context>`, update only the memory.md files explicitly referenced there.\n",
    "- Skills are procedural knowledge loaded on demand, so they can be detailed: a reusable workflow, a domain recipe, the right sequence of CLI calls for a recurring task. When you discover one — not every-time knowledge, but a real capability — create or refine the relevant skill in ~/.garyx/skills/<skill-id>/SKILL.md (synced into ~/.claude/skills and ~/.codex/skills) and validate it with a focused test. If an existing skill is wrong, stale, or incomplete, fix it the moment you notice.\n",
    "- Project-scoped knowledge (codebase conventions, build/test commands, architectural facts, repo-specific gotchas) goes into the current project's AGENTS.md, and mirror the same change into CLAUDE.md in the same commit so every coding agent sees the same guidance.\n",
    "- If the user surfaces a fundamental gap in this agent's own system prompt — baseline behavior that should always have been there, not a single preference (→ agent memory) and not a reusable procedure (→ skill) — patch the agent definition itself: read the current prompt with `garyx agent get <agent_id>`, then write the smallest patch back via `garyx agent update` (or `upsert`). Run `garyx agent --help` if you don't remember the flags.\n",
    "\n",
    "System capabilities:\n",
    "- Delegate work with tasks, for example: `garyx task create --title \"...\" --body \"...\" --agent <agent_id>` (or `--workflow <workflow_id>`); inspect with `garyx task get <task_id>`. Notifications default to the current thread so the requester sees the final review message; pass `--notify none` only when silence is intentional, or `--notify bot <channel:account_id>` to reach a different surface. Garyx moves an in-progress task to review when its agent run stops. Only after a user, reviewer, or task creator explicitly approves the result should the task be marked done with `garyx task update <task_id> --status done --note \"approved by <name>\"`.\n",
    "- Manage scheduled automations with the CLI, for example: `garyx automation create --label \"Daily triage\" --prompt \"...\" --workspace-dir /path --every-hours 24`; then use `garyx automation list|get|update|pause|resume|run|delete`.\n",
    "- To resume the current thread after a delay (for example while waiting on background work), call the `mcp__garyx__schedule_followup` MCP tool with `delay_seconds` and a `prompt` to inject as a synthetic user turn. Do not rely on the harness `ScheduleWakeup` tool: it is disabled in this runtime because the single-turn provider mode has no firing mechanism for it.\n",
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
    if let Some(memory_context) = build_memory_context_user_message(metadata) {
        blocks.push(memory_context);
    }
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
    match build_memory_context_user_message(metadata) {
        Some(memory) if message.trim().is_empty() => memory,
        Some(memory) => format!("{memory}\n\n{message}"),
        None => message.to_owned(),
    }
}

fn build_runtime_metadata_user_message(metadata: &HashMap<String, Value>) -> Option<String> {
    let runtime = metadata.get("runtime_context").and_then(Value::as_object)?;
    let mut lines = Vec::new();
    push_runtime_line(&mut lines, "thread_id", runtime.get("thread_id"));
    push_runtime_line(&mut lines, "bot_id", runtime.get("bot_id"));
    push_runtime_line(&mut lines, "workspace_dir", runtime.get("workspace_dir"));
    push_runtime_line(&mut lines, "channel", runtime.get("channel"));
    push_runtime_line(&mut lines, "account_id", runtime.get("account_id"));
    push_runtime_line(&mut lines, "from_id", runtime.get("from_id"));
    push_runtime_line(&mut lines, "is_group", runtime.get("is_group"));
    if let Some(bot) = runtime.get("bot").and_then(Value::as_object) {
        push_runtime_line(&mut lines, "endpoint_key", bot.get("endpoint_key"));
        push_runtime_line(
            &mut lines,
            "thread_binding_key",
            bot.get("thread_binding_key"),
        );
        push_runtime_line(&mut lines, "chat_id", bot.get("chat_id"));
        push_runtime_line(
            &mut lines,
            "delivery_target_type",
            bot.get("delivery_target_type"),
        );
        push_runtime_line(
            &mut lines,
            "delivery_target_id",
            bot.get("delivery_target_id"),
        );
        push_runtime_line(
            &mut lines,
            "delivery_thread_id",
            bot.get("delivery_thread_id"),
        );
    }
    if let Some(task) = runtime.get("task").and_then(Value::as_object) {
        push_runtime_line(&mut lines, "task_id", task.get("task_id"));
    }
    if lines.is_empty() {
        return None;
    }
    // Stamp the gateway machine's local wall-clock time, with the IANA zone
    // as one-off context, so the agent can reason about "now" in the user's
    // timezone. Placed first so it reads as session context rather than
    // thread identity.
    lines.insert(
        0,
        format!(
            "current_time: {} ({})",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            iana_time_zone::get_timezone().unwrap_or_else(|_| "unknown-timezone".to_owned())
        ),
    );
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
