use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use chrono::{DateTime, Utc};
use garyx_models::local_paths::home_dir;
use garyx_models::provider::ProviderType;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalProviderSessionBinding {
    pub(crate) provider_type: ProviderType,
    pub(crate) agent_id: String,
    pub(crate) workspace_dir: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProviderSessionSearchRoots {
    pub(crate) claude_projects_dir: Option<PathBuf>,
    pub(crate) codex_state_db: Option<PathBuf>,
    pub(crate) codex_session_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RecoveredLocalProviderSession {
    pub(crate) binding: LocalProviderSessionBinding,
    pub(crate) messages: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecentLocalProviderSession {
    pub(crate) provider_type: ProviderType,
    pub(crate) provider_hint: &'static str,
    pub(crate) session_id: String,
    pub(crate) title: String,
    pub(crate) workspace_dir: String,
    pub(crate) updated_at: String,
    pub(crate) path: String,
}

pub(crate) fn recover_local_provider_session(
    session_id: &str,
    provider_hint: Option<ProviderType>,
) -> Result<Option<RecoveredLocalProviderSession>, String> {
    recover_local_provider_session_with_roots(
        session_id,
        provider_hint,
        &default_provider_session_search_roots(),
    )
}

pub(crate) fn list_recent_local_provider_sessions(
    provider_hint: Option<ProviderType>,
    limit: usize,
) -> Vec<RecentLocalProviderSession> {
    list_recent_local_provider_sessions_with_roots(
        provider_hint,
        limit,
        &default_provider_session_search_roots(),
    )
}

pub(crate) fn list_recent_local_provider_sessions_with_roots(
    provider_hint: Option<ProviderType>,
    limit: usize,
    roots: &ProviderSessionSearchRoots,
) -> Vec<RecentLocalProviderSession> {
    if limit == 0 {
        return Vec::new();
    }

    let mut sessions = Vec::new();
    let provider_hint_ref = provider_hint.as_ref();

    if let Some(claude_provider_type) = claude_session_provider_type(provider_hint_ref)
        && let Some(claude_projects_dir) = roots.claude_projects_dir.as_deref()
    {
        sessions.extend(list_recent_claude_sessions(
            claude_projects_dir,
            claude_provider_type,
            limit,
        ));
    }

    if matches_provider_hint(provider_hint_ref, ProviderType::CodexAppServer) {
        sessions.extend(list_recent_codex_sessions(
            roots.codex_state_db.as_deref(),
            limit,
        ));
    }

    let mut seen = HashSet::new();
    sessions.retain(|session| {
        seen.insert((
            session.provider_type.clone(),
            session.session_id.to_ascii_lowercase(),
        ))
    });
    sessions.sort_by(|left, right| {
        recent_sort_key(&right.updated_at).cmp(&recent_sort_key(&left.updated_at))
    });
    sessions.truncate(limit);
    sessions
}

#[cfg(test)]
pub(crate) fn locate_local_provider_session_with_roots(
    session_id: &str,
    provider_hint: Option<ProviderType>,
    roots: &ProviderSessionSearchRoots,
) -> Result<Option<LocalProviderSessionBinding>, String> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Ok(None);
    }

    let mut matches = Vec::new();
    let provider_hint = provider_hint.as_ref();

    if let Some(claude_provider_type) = claude_session_provider_type(provider_hint)
        && let Some(claude_projects_dir) = roots.claude_projects_dir.as_deref()
        && let Some(binding) =
            locate_claude_session_binding(session_id, claude_projects_dir, claude_provider_type)
    {
        matches.push(binding);
    }

    if matches_provider_hint(provider_hint, ProviderType::CodexAppServer)
        && let Some(binding) = locate_codex_session_binding(session_id, &roots.codex_session_roots)
    {
        matches.push(binding);
    }

    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.into_iter().next()),
        _ => Err(format!(
            "session id '{session_id}' matches multiple local providers"
        )),
    }
}

pub(crate) fn recover_local_provider_session_with_roots(
    session_id: &str,
    provider_hint: Option<ProviderType>,
    roots: &ProviderSessionSearchRoots,
) -> Result<Option<RecoveredLocalProviderSession>, String> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Ok(None);
    }

    let mut matches = Vec::new();
    let provider_hint = provider_hint.as_ref();

    if let Some(claude_provider_type) = claude_session_provider_type(provider_hint)
        && let Some(claude_projects_dir) = roots.claude_projects_dir.as_deref()
        && let Some(recovered) =
            recover_claude_session(session_id, claude_projects_dir, claude_provider_type)
    {
        matches.push(recovered);
    }

    if matches_provider_hint(provider_hint, ProviderType::CodexAppServer)
        && let Some(recovered) = recover_codex_session(session_id, &roots.codex_session_roots)
    {
        matches.push(recovered);
    }

    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.into_iter().next()),
        _ => Err(format!(
            "session id '{session_id}' matches multiple local providers"
        )),
    }
}

fn matches_provider_hint(
    provider_hint: Option<&ProviderType>,
    provider_type: ProviderType,
) -> bool {
    match provider_hint {
        None => true,
        Some(value) => value == &provider_type,
    }
}

fn claude_session_provider_type(provider_hint: Option<&ProviderType>) -> Option<ProviderType> {
    match provider_hint {
        None | Some(ProviderType::ClaudeCode) => Some(ProviderType::ClaudeCode),
        Some(_) => None,
    }
}

fn build_imported_message(
    role: &str,
    content: String,
    timestamp: String,
    provider_type: &ProviderType,
    session_id: &str,
) -> Value {
    json!({
        "role": role,
        "content": content,
        "timestamp": timestamp,
        "metadata": {
            "imported_from_provider_session": true,
            "imported_provider_type": provider_type,
            "imported_session_id": session_id,
        }
    })
}

fn provider_resume_hint(provider_type: &ProviderType) -> &'static str {
    match provider_type {
        ProviderType::ClaudeCode => "claude",
        ProviderType::CodexAppServer => "codex",
        _ => provider_type.as_slug(),
    }
}

fn trimmed_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn text_from_content_blocks(content: &Value, allowed_types: &[&str]) -> Option<String> {
    match content {
        Value::String(value) => trimmed_string(Some(value)),
        Value::Array(items) => {
            let parts: Vec<String> = items
                .iter()
                .filter_map(|item| {
                    let item_type = item.get("type").and_then(Value::as_str)?;
                    if !allowed_types.iter().any(|allowed| allowed == &item_type) {
                        return None;
                    }
                    trimmed_string(item.get("text").and_then(Value::as_str))
                })
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n\n"))
            }
        }
        _ => None,
    }
}

fn title_from_messages(messages: &[Value]) -> Option<String> {
    messages
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .filter_map(|message| message.get("content").and_then(Value::as_str))
        .filter(|text| !text.trim_start().starts_with("<environment_context"))
        .filter_map(compact_title)
        .next()
        .or_else(|| {
            messages
                .iter()
                .filter_map(|message| message.get("content").and_then(Value::as_str))
                .filter_map(compact_title)
                .next()
        })
}

fn compact_title(text: &str) -> Option<String> {
    let compact = strip_leading_metadata_block(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if compact.is_empty() {
        return None;
    }
    const MAX_TITLE_CHARS: usize = 96;
    let mut chars = compact.chars();
    let title: String = chars.by_ref().take(MAX_TITLE_CHARS).collect();
    if chars.next().is_some() {
        Some(format!("{title}..."))
    } else {
        Some(title)
    }
}

fn strip_leading_metadata_block(text: &str) -> &str {
    let mut trimmed = text.trim_start();
    loop {
        let mut stripped = false;
        for (open, close) in [
            ("<garyx_thread_metadata>", "</garyx_thread_metadata>"),
            ("<garyx_memory_context>", "</garyx_memory_context>"),
            ("<system_instruction>", "</system_instruction>"),
            ("<environment_context>", "</environment_context>"),
        ] {
            if let Some(rest) = trimmed.strip_prefix(open)
                && let Some((_, after)) = rest.split_once(close)
            {
                trimmed = after.trim_start();
                stripped = true;
                break;
            }
        }
        if !stripped {
            break;
        }
    }
    trimmed
}

fn latest_message_timestamp(messages: &[Value]) -> Option<String> {
    messages
        .iter()
        .filter_map(|message| message.get("timestamp").and_then(Value::as_str))
        .filter_map(|timestamp| trimmed_string(Some(timestamp)))
        .max_by_key(|timestamp| recent_sort_key(timestamp))
}

fn file_modified_at(path: &Path) -> Option<String> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()
        .map(DateTime::<Utc>::from)
        .map(|timestamp| timestamp.to_rfc3339())
}

fn file_modified_sort_key(path: &Path) -> i64 {
    fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|timestamp| timestamp.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn recent_sort_key(timestamp: &str) -> i64 {
    DateTime::parse_from_rfc3339(timestamp)
        .map(|value| value.timestamp_millis())
        .unwrap_or(0)
}

fn workspace_fallback_title(workspace_dir: &str, provider_label: &str) -> String {
    Path::new(workspace_dir)
        .file_name()
        .and_then(|value| value.to_str())
        .map(|name| format!("{provider_label}: {name}"))
        .unwrap_or_else(|| provider_label.to_owned())
}

fn default_provider_session_search_roots() -> ProviderSessionSearchRoots {
    let Some(home) = home_dir() else {
        return ProviderSessionSearchRoots::default();
    };

    ProviderSessionSearchRoots {
        claude_projects_dir: existing_dir(home.join(".claude").join("projects")),
        codex_state_db: existing_file(home.join(".codex").join("state_5.sqlite")),
        codex_session_roots: [
            home.join(".codex").join("sessions"),
            home.join(".codex").join("archived_sessions"),
        ]
        .into_iter()
        .filter_map(existing_dir)
        .collect(),
    }
}

fn existing_dir(path: PathBuf) -> Option<PathBuf> {
    path.is_dir().then_some(path)
}

fn existing_file(path: PathBuf) -> Option<PathBuf> {
    path.is_file().then_some(path)
}

fn normalized_existing_path(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let path = PathBuf::from(trimmed);
    if !path.exists() {
        return None;
    }

    let canonical = fs::canonicalize(&path).unwrap_or(path);
    Some(canonical.to_string_lossy().to_string())
}

fn recover_claude_session(
    session_id: &str,
    projects_dir: &Path,
    provider_type: ProviderType,
) -> Option<RecoveredLocalProviderSession> {
    let session_file_name = format!("{session_id}.jsonl");
    for entry in fs::read_dir(projects_dir).ok()?.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }

        let project_dir = entry.path();
        let session_file = project_dir.join(&session_file_name);
        if !session_file.is_file() {
            continue;
        }

        let Some(workspace_dir) =
            read_claude_session_cwd(&session_file, session_id).or_else(|| {
                read_claude_project_path_from_index(
                    &project_dir.join("sessions-index.json"),
                    session_id,
                )
            })
        else {
            continue;
        };

        return Some(RecoveredLocalProviderSession {
            binding: LocalProviderSessionBinding {
                provider_type: provider_type.clone(),
                agent_id: "claude".to_owned(),
                workspace_dir,
            },
            messages: read_claude_transcript_messages(&session_file, session_id, &provider_type),
        });
    }
    None
}

fn list_recent_claude_sessions(
    projects_dir: &Path,
    provider_type: ProviderType,
    limit: usize,
) -> Vec<RecentLocalProviderSession> {
    let mut candidates = Vec::new();
    for entry in fs::read_dir(projects_dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
    {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }

        let project_dir = entry.path();
        for session_entry in fs::read_dir(&project_dir)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
        {
            let Ok(session_type) = session_entry.file_type() else {
                continue;
            };
            if !session_type.is_file() {
                continue;
            }
            let session_file = session_entry.path();
            if session_file.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }
            candidates.push((
                file_modified_sort_key(&session_file),
                project_dir.clone(),
                session_file,
            ));
        }
    }
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));
    candidates.truncate(limit);

    let mut sessions = Vec::new();
    for (_, project_dir, session_file) in candidates {
        let Some(session_id) = session_file
            .file_stem()
            .and_then(|value| value.to_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
        else {
            continue;
        };
        let Some(workspace_dir) =
            read_claude_session_cwd(&session_file, &session_id).or_else(|| {
                read_claude_project_path_from_index(
                    &project_dir.join("sessions-index.json"),
                    &session_id,
                )
            })
        else {
            continue;
        };

        let messages = read_claude_transcript_messages(&session_file, &session_id, &provider_type);
        let updated_at = latest_message_timestamp(&messages)
            .or_else(|| file_modified_at(&session_file))
            .unwrap_or_else(|| Utc::now().to_rfc3339());
        let title = title_from_messages(&messages)
            .unwrap_or_else(|| workspace_fallback_title(&workspace_dir, "Claude"));
        sessions.push(RecentLocalProviderSession {
            provider_type: provider_type.clone(),
            provider_hint: provider_resume_hint(&provider_type),
            session_id,
            title,
            workspace_dir,
            updated_at,
            path: session_file.display().to_string(),
        });
    }
    sessions
}

fn read_claude_transcript_messages(
    session_file: &Path,
    session_id: &str,
    provider_type: &ProviderType,
) -> Vec<Value> {
    let Ok(file) = fs::File::open(session_file) else {
        return Vec::new();
    };

    let mut messages = Vec::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("sessionId").and_then(Value::as_str) != Some(session_id) {
            continue;
        }

        let timestamp = trimmed_string(value.get("timestamp").and_then(Value::as_str));
        match value.get("type").and_then(Value::as_str) {
            Some("user") => {
                let content = value
                    .get("message")
                    .and_then(|message| message.get("content"));
                let Some(text) =
                    content.and_then(|content| text_from_content_blocks(content, &["text"]))
                else {
                    continue;
                };
                let Some(timestamp) = timestamp.clone() else {
                    continue;
                };
                messages.push(build_imported_message(
                    "user",
                    text,
                    timestamp,
                    provider_type,
                    session_id,
                ));
            }
            Some("assistant") => {
                let content = value
                    .get("message")
                    .and_then(|message| message.get("content"));
                let Some(text) =
                    content.and_then(|content| text_from_content_blocks(content, &["text"]))
                else {
                    continue;
                };
                let Some(timestamp) = timestamp.clone() else {
                    continue;
                };
                messages.push(build_imported_message(
                    "assistant",
                    text,
                    timestamp,
                    provider_type,
                    session_id,
                ));
            }
            _ => {}
        }
    }

    messages
}

fn recover_codex_session(
    session_id: &str,
    session_roots: &[PathBuf],
) -> Option<RecoveredLocalProviderSession> {
    for root in session_roots {
        if let Some(session_file) = find_codex_session_file(root, session_id) {
            let Some(workspace_dir) = read_codex_session_cwd(&session_file, session_id) else {
                continue;
            };
            return Some(RecoveredLocalProviderSession {
                binding: LocalProviderSessionBinding {
                    provider_type: ProviderType::CodexAppServer,
                    agent_id: "codex".to_owned(),
                    workspace_dir,
                },
                messages: read_codex_transcript_messages(&session_file, session_id),
            });
        }
    }
    None
}

fn list_recent_codex_sessions(
    state_db: Option<&Path>,
    limit: usize,
) -> Vec<RecentLocalProviderSession> {
    if let Some(state_db) = state_db {
        return list_recent_codex_sessions_from_state_db(state_db, limit);
    }
    Vec::new()
}

fn list_recent_codex_sessions_from_state_db(
    state_db: &Path,
    limit: usize,
) -> Vec<RecentLocalProviderSession> {
    let Ok(connection) = Connection::open_with_flags(state_db, OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return Vec::new();
    };
    let Ok(mut statement) = connection.prepare(
        r#"
        SELECT id, title, first_user_message, preview, cwd, rollout_path, updated_at_ms, updated_at
        FROM threads
        WHERE archived = 0
        ORDER BY updated_at_ms DESC, updated_at DESC, id DESC
        LIMIT ?1
        "#,
    ) else {
        return Vec::new();
    };
    let Ok(rows) = statement.query_map([limit as i64], |row| {
        let session_id: String = row.get(0)?;
        let title: String = row.get(1)?;
        let first_user_message: String = row.get(2)?;
        let preview: String = row.get(3)?;
        let cwd: String = row.get(4)?;
        let rollout_path: String = row.get(5)?;
        let updated_at_ms: Option<i64> = row.get(6)?;
        let updated_at_seconds: i64 = row.get(7)?;
        Ok((
            session_id,
            title,
            first_user_message,
            preview,
            cwd,
            rollout_path,
            updated_at_ms,
            updated_at_seconds,
        ))
    }) else {
        return Vec::new();
    };

    rows.filter_map(Result::ok)
        .filter_map(
            |(
                session_id,
                title,
                first_user_message,
                preview,
                cwd,
                rollout_path,
                updated_at_ms,
                updated_at_seconds,
            )| {
                let workspace_dir = normalized_existing_path(&cwd)?;
                let title = compact_title(&title)
                    .or_else(|| compact_title(&first_user_message))
                    .or_else(|| compact_title(&preview))
                    .unwrap_or_else(|| workspace_fallback_title(&workspace_dir, "Codex"));
                let updated_at = DateTime::<Utc>::from_timestamp_millis(
                    updated_at_ms.unwrap_or(updated_at_seconds.saturating_mul(1000)),
                )
                .unwrap_or_else(Utc::now)
                .to_rfc3339();
                Some(RecentLocalProviderSession {
                    provider_type: ProviderType::CodexAppServer,
                    provider_hint: provider_resume_hint(&ProviderType::CodexAppServer),
                    session_id,
                    title,
                    workspace_dir,
                    updated_at,
                    path: rollout_path,
                })
            },
        )
        .collect()
}

fn read_codex_transcript_messages(session_file: &Path, session_id: &str) -> Vec<Value> {
    let Ok(file) = fs::File::open(session_file) else {
        return Vec::new();
    };

    let mut messages = Vec::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(timestamp) = trimmed_string(value.get("timestamp").and_then(Value::as_str)) else {
            continue;
        };

        match value.get("type").and_then(Value::as_str) {
            Some("event_msg")
                if value
                    .get("payload")
                    .and_then(|payload| payload.get("type"))
                    .and_then(Value::as_str)
                    == Some("user_message") =>
            {
                let Some(text) = trimmed_string(
                    value
                        .get("payload")
                        .and_then(|payload| payload.get("message"))
                        .and_then(Value::as_str),
                ) else {
                    continue;
                };
                messages.push(build_imported_message(
                    "user",
                    text,
                    timestamp,
                    &ProviderType::CodexAppServer,
                    session_id,
                ));
            }
            Some("response_item")
                if value
                    .get("payload")
                    .and_then(|payload| payload.get("type"))
                    .and_then(Value::as_str)
                    == Some("message")
                    && value
                        .get("payload")
                        .and_then(|payload| payload.get("role"))
                        .and_then(Value::as_str)
                        == Some("assistant") =>
            {
                let Some(text) = value
                    .get("payload")
                    .and_then(|payload| payload.get("content"))
                    .and_then(|content| {
                        text_from_content_blocks(content, &["output_text", "text"])
                    })
                else {
                    continue;
                };
                messages.push(build_imported_message(
                    "assistant",
                    text,
                    timestamp,
                    &ProviderType::CodexAppServer,
                    session_id,
                ));
            }
            _ => {}
        }
    }

    messages
}

#[cfg(test)]
fn locate_claude_session_binding(
    session_id: &str,
    projects_dir: &Path,
    provider_type: ProviderType,
) -> Option<LocalProviderSessionBinding> {
    let session_file_name = format!("{session_id}.jsonl");
    for entry in fs::read_dir(projects_dir).ok()?.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }

        let project_dir = entry.path();
        let session_file = project_dir.join(&session_file_name);
        if !session_file.is_file() {
            continue;
        }

        let Some(workspace_dir) =
            read_claude_session_cwd(&session_file, session_id).or_else(|| {
                read_claude_project_path_from_index(
                    &project_dir.join("sessions-index.json"),
                    session_id,
                )
            })
        else {
            continue;
        };

        return Some(LocalProviderSessionBinding {
            provider_type,
            agent_id: "claude".to_owned(),
            workspace_dir,
        });
    }
    None
}

fn read_claude_session_cwd(session_file: &Path, session_id: &str) -> Option<String> {
    let file = fs::File::open(session_file).ok()?;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("sessionId").and_then(Value::as_str) != Some(session_id) {
            continue;
        }
        if let Some(cwd) = value.get("cwd").and_then(Value::as_str)
            && let Some(path) = normalized_existing_path(cwd)
        {
            return Some(path);
        }
    }
    None
}

fn read_claude_project_path_from_index(index_file: &Path, session_id: &str) -> Option<String> {
    let raw = fs::read_to_string(index_file).ok()?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    let entries = value.get("entries").and_then(Value::as_array)?;
    for entry in entries {
        if entry.get("sessionId").and_then(Value::as_str) != Some(session_id) {
            continue;
        }
        if let Some(project_path) = entry
            .get("projectPath")
            .and_then(Value::as_str)
            .and_then(normalized_existing_path)
        {
            return Some(project_path);
        }
        if let Some(project_path) = value
            .get("originalPath")
            .and_then(Value::as_str)
            .and_then(normalized_existing_path)
        {
            return Some(project_path);
        }
    }
    None
}

#[cfg(test)]
fn locate_codex_session_binding(
    session_id: &str,
    session_roots: &[PathBuf],
) -> Option<LocalProviderSessionBinding> {
    for root in session_roots {
        if let Some(session_file) = find_codex_session_file(root, session_id) {
            let Some(workspace_dir) = read_codex_session_cwd(&session_file, session_id) else {
                continue;
            };
            return Some(LocalProviderSessionBinding {
                provider_type: ProviderType::CodexAppServer,
                agent_id: "codex".to_owned(),
                workspace_dir,
            });
        }
    }
    None
}

fn find_codex_session_file(root: &Path, session_id: &str) -> Option<PathBuf> {
    for entry in fs::read_dir(root).ok()?.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            if let Some(found) = find_codex_session_file(&path, session_id) {
                return Some(found);
            }
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let file_name = path.file_name().and_then(|value| value.to_str())?;
        if file_name.ends_with(&format!("{session_id}.jsonl")) {
            return Some(path);
        }
    }
    None
}

fn read_codex_session_cwd(session_file: &Path, session_id: &str) -> Option<String> {
    let file = fs::File::open(session_file).ok()?;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("session_meta") {
            continue;
        }
        let payload = value.get("payload")?;
        if payload.get("id").and_then(Value::as_str) != Some(session_id) {
            continue;
        }
        let cwd = payload.get("cwd").and_then(Value::as_str)?;
        return normalized_existing_path(cwd);
    }
    None
}

#[cfg(test)]
mod tests;
