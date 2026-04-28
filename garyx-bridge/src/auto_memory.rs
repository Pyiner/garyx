use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use garyx_models::local_paths::{
    auto_memory_automation_dir_for_gary_home, auto_memory_automation_root_file_for_gary_home,
    auto_memory_dir_for_gary_home, auto_memory_root_file_for_gary_home,
    auto_memory_workspace_dir_for_gary_home, auto_memory_workspace_root_file_for_gary_home,
    gary_home_dir,
};

const MAX_PROMPT_CHARS_PER_FILE: usize = 6_000;
const AUTO_MEMORY_GUIDANCE: &str = concat!(
    "Garyx has a built-in Auto Memory system.\n",
    "- Source of truth lives under ~/.garyx/auto-memory.\n",
    "- Global memory file: ~/.garyx/auto-memory/memory.md\n",
    "- Each run gets exactly one scoped memory file in addition to the global memory.\n",
    "- Repository runs use ~/.garyx/auto-memory/workspaces/<workspace-key>/memory.md.\n",
    "- Scheduled automation runs use ~/.garyx/auto-memory/automations/<automation-id>/memory.md.\n",
    "- When a run reveals a durable fact that should persist, update the relevant memory.md file before you finish.\n",
    "- Keep memory.md concise. Link out to other markdown files when you need more detail.\n",
    "- If a fact becomes stale, edit or remove it instead of appending contradictions.\n",
);

const AUTOMATION_MEMORY_GUIDANCE: &str = concat!(
    "Automation Memory Protocol (IMPORTANT for scheduled tasks):\n",
    "- BEFORE starting the task, read the scoped automation memory file to recall context from previous runs.\n",
    "- AFTER completing the task, update the scoped automation memory file with any new durable findings, status changes, or lessons learned.\n",
    "- Remove or correct stale entries instead of appending contradictions.\n",
);

const GLOBAL_MEMORY_TEMPLATE: &str = concat!(
    "# Auto Memory\n\n",
    "## Durable Notes\n",
    "- Add stable user preferences, durable facts, and recurring workflows here.\n",
    "- Keep this file concise.\n",
);

#[derive(Debug, Clone, Copy)]
pub(crate) enum AutoMemoryScope<'a> {
    Workspace(&'a Path),
    Automation(&'a str),
}

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

    fn global_memory_dir(&self) -> PathBuf {
        auto_memory_dir_for_gary_home(&self.gary_home)
    }

    fn global_memory_file(&self) -> PathBuf {
        auto_memory_root_file_for_gary_home(&self.gary_home)
    }

    fn scoped_memory_dir(&self, scope: AutoMemoryScope<'_>) -> PathBuf {
        match scope {
            AutoMemoryScope::Workspace(workspace_dir) => {
                auto_memory_workspace_dir_for_gary_home(&self.gary_home, workspace_dir)
            }
            AutoMemoryScope::Automation(automation_id) => {
                auto_memory_automation_dir_for_gary_home(&self.gary_home, automation_id)
            }
        }
    }

    fn scoped_memory_file(&self, scope: AutoMemoryScope<'_>) -> PathBuf {
        match scope {
            AutoMemoryScope::Workspace(workspace_dir) => {
                auto_memory_workspace_root_file_for_gary_home(&self.gary_home, workspace_dir)
            }
            AutoMemoryScope::Automation(automation_id) => {
                auto_memory_automation_root_file_for_gary_home(&self.gary_home, automation_id)
            }
        }
    }
}

pub(crate) fn build_auto_memory_prompt_section(
    layout: &AutoMemoryLayout,
    workspace_dir: Option<&Path>,
    automation_id: Option<&str>,
) -> String {
    let normalized_workspace = workspace_dir.map(normalize_workspace_dir);
    let scope = automation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(AutoMemoryScope::Automation)
        .or_else(|| {
            normalized_workspace
                .as_deref()
                .map(AutoMemoryScope::Workspace)
        });
    let _ = ensure_auto_memory_scaffold(layout, scope);

    let mut sections = vec![AUTO_MEMORY_GUIDANCE.to_owned()];
    if matches!(scope, Some(AutoMemoryScope::Automation(_))) {
        sections.push(AUTOMATION_MEMORY_GUIDANCE.to_owned());
    }
    sections.push(render_memory_block(
        "Global Auto Memory",
        &layout.global_memory_file(),
    ));
    if let Some(scope) = scope {
        let label = match scope {
            AutoMemoryScope::Workspace(_) => "Scoped Auto Memory (Workspace)",
            AutoMemoryScope::Automation(_) => "Scoped Auto Memory (Automation)",
        };
        sections.push(render_memory_block(
            label,
            &layout.scoped_memory_file(scope),
        ));
    }

    sections.join("\n\n")
}

fn ensure_auto_memory_scaffold(
    layout: &AutoMemoryLayout,
    scope: Option<AutoMemoryScope<'_>>,
) -> Result<(), io::Error> {
    fs::create_dir_all(layout.global_memory_dir())?;
    ensure_markdown_file(&layout.global_memory_file(), GLOBAL_MEMORY_TEMPLATE)?;

    if let Some(scope) = scope {
        fs::create_dir_all(layout.scoped_memory_dir(scope))?;
        ensure_markdown_file(
            &layout.scoped_memory_file(scope),
            &scoped_auto_memory_template(scope),
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

fn normalize_workspace_dir(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn scoped_auto_memory_template(scope: AutoMemoryScope<'_>) -> String {
    match scope {
        AutoMemoryScope::Workspace(workspace_dir) => format!(
            concat!(
                "# Auto Memory\n\n",
                "Scope: workspace\n",
                "Workspace: `{}`\n\n",
                "## Durable Notes\n",
                "- Add durable repository facts, build and test commands, coding conventions, and workflow notes here.\n",
                "- Keep this file concise.\n",
            ),
            workspace_dir.display()
        ),
        AutoMemoryScope::Automation(automation_id) => format!(
            concat!(
                "# Auto Memory\n\n",
                "Scope: automation\n",
                "Automation ID: `{}`\n\n",
                "## Durable Notes\n",
                "- Add durable facts for this recurring automation here.\n",
                "- Keep this file concise.\n",
            ),
            automation_id.trim()
        ),
    }
}

fn render_memory_block(label: &str, path: &Path) -> String {
    let contents = fs::read_to_string(path).unwrap_or_default();
    let contents = trim_for_prompt(&contents);
    format!("{label} ({})\n{contents}", path.display())
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

#[cfg(test)]
mod tests;
