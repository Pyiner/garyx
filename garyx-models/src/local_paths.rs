use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

pub fn gary_home_dir() -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".garyx")
}

/// Previous home dir (~/.gary) — used for automatic migration.
pub fn legacy_gary_home_dir() -> Option<PathBuf> {
    home_dir().map(|home| home.join(".gary"))
}

/// Legacy home dir (~/.garybot) — kept around so existing local state
/// can be migrated into ~/.garyx on first run.
pub fn legacy_garybot_home_dir() -> Option<PathBuf> {
    home_dir().map(|home| home.join(".garybot"))
}

pub fn default_session_data_dir() -> PathBuf {
    gary_home_dir().join("data")
}

pub fn thread_transcripts_dir_for_data_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("transcripts")
}

pub fn default_thread_transcripts_dir() -> PathBuf {
    thread_transcripts_dir_for_data_dir(&default_session_data_dir())
}

pub fn message_ledger_dir_for_data_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("message-ledger")
}

pub fn default_message_ledger_dir() -> PathBuf {
    message_ledger_dir_for_data_dir(&default_session_data_dir())
}

pub fn auto_research_state_path_for_data_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("auto-research-state.json")
}

pub fn default_auto_research_state_path() -> PathBuf {
    auto_research_state_path_for_data_dir(&default_session_data_dir())
}

pub fn custom_agents_state_path_for_data_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("custom-agents.json")
}

pub fn default_custom_agents_state_path() -> PathBuf {
    custom_agents_state_path_for_data_dir(&default_session_data_dir())
}

pub fn agent_teams_state_path_for_data_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("agent-teams.json")
}

pub fn default_agent_teams_state_path() -> PathBuf {
    agent_teams_state_path_for_data_dir(&default_session_data_dir())
}

pub fn wikis_state_path_for_data_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("wikis.json")
}

pub fn default_wikis_state_path() -> PathBuf {
    wikis_state_path_for_data_dir(&default_session_data_dir())
}

/// Directory holding one JSON file per `AgentTeam` group — orchestrator
/// state owned by the AgentTeam provider (sub-agent thread mappings and
/// per-member catch-up offsets, keyed by the group's thread_id).
pub fn agent_team_groups_dir_for_data_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("agent-team-groups")
}

pub fn default_agent_team_groups_dir() -> PathBuf {
    agent_team_groups_dir_for_data_dir(&default_session_data_dir())
}

pub fn run_terminal_dir_for_data_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("run-terminal")
}

pub fn default_run_terminal_dir() -> PathBuf {
    run_terminal_dir_for_data_dir(&default_session_data_dir())
}

pub fn conversation_index_dir_for_data_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("conversation-index")
}

pub fn conversation_index_db_path_for_data_dir(data_dir: &Path) -> PathBuf {
    conversation_index_dir_for_data_dir(data_dir).join("index.sqlite3")
}

pub fn default_skills_dir() -> PathBuf {
    gary_home_dir().join("skills")
}

pub fn default_log_file_path() -> PathBuf {
    gary_home_dir().join("logs").join("gary.log")
}

pub fn default_mcp_sync_state_path() -> PathBuf {
    gary_home_dir().join("mcp-sync-state.json")
}

pub fn skills_sync_state_path_for_gary_home(gary_home: &Path) -> PathBuf {
    gary_home.join("skills-sync-state.json")
}

pub fn default_skills_sync_state_path() -> PathBuf {
    skills_sync_state_path_for_gary_home(&gary_home_dir())
}

pub fn auto_memory_dir_for_gary_home(gary_home: &Path) -> PathBuf {
    gary_home.join("auto-memory")
}

pub fn auto_memory_root_file_for_gary_home(gary_home: &Path) -> PathBuf {
    auto_memory_dir_for_gary_home(gary_home).join("memory.md")
}

pub fn auto_memory_workspaces_dir_for_gary_home(gary_home: &Path) -> PathBuf {
    auto_memory_dir_for_gary_home(gary_home).join("workspaces")
}

pub fn auto_memory_automations_dir_for_gary_home(gary_home: &Path) -> PathBuf {
    auto_memory_dir_for_gary_home(gary_home).join("automations")
}

pub fn auto_memory_workspace_key(workspace_dir: &Path) -> String {
    let normalized_workspace =
        fs::canonicalize(workspace_dir).unwrap_or_else(|_| workspace_dir.to_path_buf());
    let display_name = normalized_workspace
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("workspace");
    let mut sanitized = String::new();
    for ch in display_name.chars() {
        if ch.is_ascii_alphanumeric() {
            sanitized.push(ch.to_ascii_lowercase());
        } else if !sanitized.ends_with('-') {
            sanitized.push('-');
        }
    }
    let sanitized = sanitized.trim_matches('-');
    let sanitized = if sanitized.is_empty() {
        "workspace"
    } else {
        sanitized
    };
    let hash = fnv1a64_hex(normalized_workspace.to_string_lossy().as_bytes());
    format!("{sanitized}-{hash}")
}

pub fn auto_memory_workspace_dir_for_gary_home(gary_home: &Path, workspace_dir: &Path) -> PathBuf {
    auto_memory_workspaces_dir_for_gary_home(gary_home)
        .join(auto_memory_workspace_key(workspace_dir))
}

pub fn default_auto_memory_workspace_dir(workspace_dir: &Path) -> PathBuf {
    auto_memory_workspace_dir_for_gary_home(&gary_home_dir(), workspace_dir)
}

pub fn auto_memory_workspace_root_file_for_gary_home(
    gary_home: &Path,
    workspace_dir: &Path,
) -> PathBuf {
    auto_memory_workspace_dir_for_gary_home(gary_home, workspace_dir).join("memory.md")
}

pub fn default_auto_memory_workspace_root_file(workspace_dir: &Path) -> PathBuf {
    auto_memory_workspace_root_file_for_gary_home(&gary_home_dir(), workspace_dir)
}

pub fn auto_memory_automation_key(automation_id: &str) -> String {
    let trimmed = automation_id.trim();
    let base = if trimmed.is_empty() {
        "automation"
    } else {
        trimmed
    };
    let mut sanitized = String::new();
    for ch in base.chars() {
        if ch.is_ascii_alphanumeric() {
            sanitized.push(ch.to_ascii_lowercase());
        } else if !sanitized.ends_with('-') {
            sanitized.push('-');
        }
    }
    let sanitized = sanitized.trim_matches('-');
    if sanitized.is_empty() {
        "automation".to_owned()
    } else {
        sanitized.to_owned()
    }
}

pub fn auto_memory_automation_dir_for_gary_home(gary_home: &Path, automation_id: &str) -> PathBuf {
    auto_memory_automations_dir_for_gary_home(gary_home)
        .join(auto_memory_automation_key(automation_id))
}

pub fn auto_memory_automation_root_file_for_gary_home(
    gary_home: &Path,
    automation_id: &str,
) -> PathBuf {
    auto_memory_automation_dir_for_gary_home(gary_home, automation_id).join("memory.md")
}

pub fn default_pending_restart_path() -> PathBuf {
    gary_home_dir().join("pending-restart.json")
}

pub fn migrate_legacy_homes() -> Result<(), io::Error> {
    let garyx_home = gary_home_dir();

    if let Some(legacy_home) = legacy_garybot_home_dir() {
        migrate_legacy_home_dir(&legacy_home, &garyx_home)?;
    }

    Ok(())
}

fn migrate_legacy_home_dir(legacy_home: &Path, target_home: &Path) -> Result<(), io::Error> {
    if !legacy_home.exists() {
        return Ok(());
    }

    fs::create_dir_all(target_home)?;

    for name in [
        "skills",
        "data",
        "logs",
        "auto-memory",
        "mcp-sync-state.json",
        "skills-sync-state.json",
        "pending-restart.json",
    ] {
        migrate_path(&legacy_home.join(name), &target_home.join(name))?;
    }

    if legacy_home.exists() && legacy_home.read_dir()?.next().is_none() {
        fs::remove_dir(legacy_home)?;
    }

    Ok(())
}

fn migrate_path(source: &Path, target: &Path) -> Result<(), io::Error> {
    let source_metadata = match fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };

    if !target.exists() {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(source, target)?;
        return Ok(());
    }

    let target_metadata = fs::symlink_metadata(target)?;
    if source_metadata.is_dir() && target_metadata.is_dir() {
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            migrate_path(&entry.path(), &target.join(entry.file_name()))?;
        }
        fs::remove_dir(source)?;
        return Ok(());
    }

    if source_metadata.file_type().is_symlink() || source_metadata.is_file() {
        fs::remove_file(source)?;
    }

    Ok(())
}

fn fnv1a64_hex(bytes: &[u8]) -> String {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    let mut hash = OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }

    format!("{hash:016x}")
}

#[cfg(test)]
mod tests;
