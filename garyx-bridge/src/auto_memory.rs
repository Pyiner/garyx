use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use garyx_models::local_paths::{
    auto_memory_agent_dir_for_gary_home, auto_memory_agent_root_file_for_gary_home,
    auto_memory_automation_dir_for_gary_home, auto_memory_automation_root_file_for_gary_home,
    gary_home_dir,
};
use serde_json::Value;

const MAX_PROMPT_CHARS_PER_FILE: usize = 6_000;
const DEFAULT_AGENT_ID: &str = "garyx";

const MEMORY_CONTEXT_GUIDANCE: &str = concat!(
    "This is durable Garyx memory context for the current run.\n",
    "- Treat it as background context, not as a user request.\n",
    "- Agent memory belongs to the current agent. Update it when you learn durable facts, user preferences, recurring workflows, or improvements to your own behavior.\n",
    "- Automation memory belongs to the current scheduled automation when present. Update it with durable outcomes, status, or lessons from recurring runs.\n",
    "- Keep memory concise; correct stale entries instead of appending contradictions.\n",
);

#[derive(Debug, Clone)]
pub(crate) struct AutoMemoryLayout {
    gary_home: PathBuf,
}

impl AutoMemoryLayout {
    pub(crate) fn current() -> Self {
        Self {
            gary_home: gary_home_dir(),
        }
    }

    #[cfg(test)]
    pub(crate) fn from_gary_home(gary_home: impl Into<PathBuf>) -> Self {
        Self {
            gary_home: gary_home.into(),
        }
    }

    fn agent_memory_dir(&self, agent_id: &str) -> PathBuf {
        auto_memory_agent_dir_for_gary_home(&self.gary_home, agent_id)
    }

    fn agent_memory_file(&self, agent_id: &str) -> PathBuf {
        auto_memory_agent_root_file_for_gary_home(&self.gary_home, agent_id)
    }

    fn automation_memory_dir(&self, automation_id: &str) -> PathBuf {
        auto_memory_automation_dir_for_gary_home(&self.gary_home, automation_id)
    }

    fn automation_memory_file(&self, automation_id: &str) -> PathBuf {
        auto_memory_automation_root_file_for_gary_home(&self.gary_home, automation_id)
    }
}

pub(crate) fn build_auto_memory_user_message(metadata: &HashMap<String, Value>) -> String {
    build_auto_memory_user_message_with_layout(metadata, &AutoMemoryLayout::current())
}

fn build_auto_memory_user_message_with_layout(
    metadata: &HashMap<String, Value>,
    layout: &AutoMemoryLayout,
) -> String {
    let agent_id = current_agent_id(metadata).unwrap_or_else(|| DEFAULT_AGENT_ID.to_owned());
    let automation_id = current_automation_id(metadata);
    let _ = ensure_auto_memory_scaffold(layout, &agent_id, automation_id.as_deref());

    let mut blocks = vec![format!(
        "<instructions>\n{}</instructions>",
        escape_xml_text(MEMORY_CONTEXT_GUIDANCE.trim_end())
    )];
    blocks.push(render_memory_xml_block(
        "agent_memory",
        "agent_id",
        &agent_id,
        &layout.agent_memory_file(&agent_id),
    ));
    if let Some(automation_id) = automation_id.as_deref() {
        blocks.push(render_memory_xml_block(
            "automation_memory",
            "automation_id",
            automation_id,
            &layout.automation_memory_file(automation_id),
        ));
    }

    format!(
        "<garyx_memory_context>\n{}\n</garyx_memory_context>",
        blocks.join("\n\n")
    )
}

fn ensure_auto_memory_scaffold(
    layout: &AutoMemoryLayout,
    agent_id: &str,
    automation_id: Option<&str>,
) -> Result<(), io::Error> {
    fs::create_dir_all(layout.agent_memory_dir(agent_id))?;
    ensure_markdown_file(
        &layout.agent_memory_file(agent_id),
        &agent_memory_template(agent_id),
    )?;

    if let Some(automation_id) = automation_id {
        fs::create_dir_all(layout.automation_memory_dir(automation_id))?;
        ensure_markdown_file(
            &layout.automation_memory_file(automation_id),
            &automation_memory_template(automation_id),
        )?;
    }

    Ok(())
}

fn ensure_markdown_file(path: &Path, contents: &str) -> Result<(), io::Error> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)
}

fn agent_memory_template(agent_id: &str) -> String {
    format!(
        concat!(
            "# Agent Memory\n\n",
            "Agent ID: `{}`\n\n",
            "## Durable Notes\n",
            "- Add stable user preferences, durable facts, recurring workflows, and self-improvement notes for this agent here.\n",
            "- Keep this file concise.\n",
        ),
        agent_id.trim()
    )
}

fn automation_memory_template(automation_id: &str) -> String {
    format!(
        concat!(
            "# Automation Memory\n\n",
            "Automation ID: `{}`\n\n",
            "## Durable Notes\n",
            "- Add durable facts, outcomes, status, and lessons for this recurring automation here.\n",
            "- Keep this file concise.\n",
        ),
        automation_id.trim()
    )
}

fn render_memory_xml_block(tag: &str, attr_name: &str, attr_value: &str, path: &Path) -> String {
    let contents = fs::read_to_string(path).unwrap_or_default();
    let contents = trim_for_prompt(&contents);
    format!(
        "<{tag} {attr_name}=\"{}\" path=\"{}\">\n{}\n</{tag}>",
        escape_xml_attr(attr_value),
        escape_xml_attr(&path.display().to_string()),
        escape_xml_text(&contents)
    )
}

fn trim_for_prompt(contents: &str) -> String {
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        return "(empty)".to_owned();
    }

    let mut result = String::new();
    let mut used = 0usize;
    for ch in trimmed.chars() {
        let char_len = ch.len_utf8();
        if used + char_len > MAX_PROMPT_CHARS_PER_FILE {
            result.push_str(
                "\n\n[Auto Memory truncated for prompt; read the file directly if you need more detail.]",
            );
            return result;
        }
        result.push(ch);
        used += char_len;
    }

    result
}

fn current_agent_id(metadata: &HashMap<String, Value>) -> Option<String> {
    metadata
        .get("agent_id")
        .and_then(scalar_string)
        .or_else(|| {
            metadata
                .get("runtime_context")
                .and_then(Value::as_object)
                .and_then(|runtime| runtime.get("agent_id").and_then(scalar_string))
        })
        .or_else(|| {
            metadata
                .get("runtime_context")
                .and_then(Value::as_object)
                .and_then(|runtime| {
                    runtime
                        .get("thread")
                        .and_then(Value::as_object)
                        .and_then(|thread| thread.get("agent_id").and_then(scalar_string))
                })
        })
}

fn current_automation_id(metadata: &HashMap<String, Value>) -> Option<String> {
    metadata
        .get("automation_id")
        .and_then(scalar_string)
        .or_else(|| {
            metadata
                .get("runtime_context")
                .and_then(Value::as_object)
                .and_then(|runtime| runtime.get("automation_id").and_then(scalar_string))
        })
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

fn escape_xml_attr(value: &str) -> String {
    escape_xml_text(value).replace('"', "&quot;")
}

#[cfg(test)]
mod tests;
