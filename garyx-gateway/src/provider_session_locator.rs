use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use garyx_models::local_paths::home_dir;
use garyx_models::provider::ProviderType;
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
    pub(crate) codex_session_roots: Vec<PathBuf>,
    pub(crate) gemini_tmp_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RecoveredLocalProviderSession {
    pub(crate) binding: LocalProviderSessionBinding,
    pub(crate) messages: Vec<Value>,
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn locate_local_provider_session(
    session_id: &str,
    provider_hint: Option<ProviderType>,
) -> Result<Option<LocalProviderSessionBinding>, String> {
    locate_local_provider_session_with_roots(
        session_id,
        provider_hint,
        &default_provider_session_search_roots(),
    )
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

    if matches_provider_hint(provider_hint, ProviderType::ClaudeCode) {
        if let Some(claude_projects_dir) = roots.claude_projects_dir.as_deref() {
            if let Some(binding) = locate_claude_session_binding(session_id, claude_projects_dir) {
                matches.push(binding);
            }
        }
    }

    if matches_provider_hint(provider_hint, ProviderType::CodexAppServer) {
        if let Some(binding) = locate_codex_session_binding(session_id, &roots.codex_session_roots)
        {
            matches.push(binding);
        }
    }

    if matches_provider_hint(provider_hint, ProviderType::GeminiCli) {
        if let Some(gemini_tmp_dir) = roots.gemini_tmp_dir.as_deref() {
            if let Some(binding) = locate_gemini_session_binding(session_id, gemini_tmp_dir) {
                matches.push(binding);
            }
        }
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

    if matches_provider_hint(provider_hint, ProviderType::ClaudeCode) {
        if let Some(claude_projects_dir) = roots.claude_projects_dir.as_deref() {
            if let Some(recovered) = recover_claude_session(session_id, claude_projects_dir) {
                matches.push(recovered);
            }
        }
    }

    if matches_provider_hint(provider_hint, ProviderType::CodexAppServer) {
        if let Some(recovered) = recover_codex_session(session_id, &roots.codex_session_roots) {
            matches.push(recovered);
        }
    }

    if matches_provider_hint(provider_hint, ProviderType::GeminiCli) {
        if let Some(gemini_tmp_dir) = roots.gemini_tmp_dir.as_deref() {
            if let Some(recovered) = recover_gemini_session(session_id, gemini_tmp_dir) {
                matches.push(recovered);
            }
        }
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

fn default_provider_session_search_roots() -> ProviderSessionSearchRoots {
    let Some(home) = home_dir() else {
        return ProviderSessionSearchRoots::default();
    };

    ProviderSessionSearchRoots {
        claude_projects_dir: existing_dir(home.join(".claude").join("projects")),
        codex_session_roots: [
            home.join(".codex").join("sessions"),
            home.join(".codex").join("archived_sessions"),
        ]
        .into_iter()
        .filter_map(existing_dir)
        .collect(),
        gemini_tmp_dir: existing_dir(home.join(".gemini").join("tmp")),
    }
}

fn existing_dir(path: PathBuf) -> Option<PathBuf> {
    path.is_dir().then_some(path)
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
                provider_type: ProviderType::ClaudeCode,
                agent_id: "claude".to_owned(),
                workspace_dir,
            },
            messages: read_claude_transcript_messages(&session_file, session_id),
        });
    }
    None
}

fn read_claude_transcript_messages(session_file: &Path, session_id: &str) -> Vec<Value> {
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
                    &ProviderType::ClaudeCode,
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
                    &ProviderType::ClaudeCode,
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

fn recover_gemini_session(
    session_id: &str,
    tmp_dir: &Path,
) -> Option<RecoveredLocalProviderSession> {
    for entry in fs::read_dir(tmp_dir).ok()?.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }

        let project_dir = entry.path();
        let Some(workspace_dir) = fs::read_to_string(project_dir.join(".project_root"))
            .ok()
            .and_then(|raw| normalized_existing_path(&raw))
        else {
            continue;
        };
        let chats_dir = project_dir.join("chats");
        if !chats_dir.is_dir() {
            continue;
        }

        for chat_entry in fs::read_dir(chats_dir).ok()?.flatten() {
            let Ok(chat_type) = chat_entry.file_type() else {
                continue;
            };
            if !chat_type.is_file() {
                continue;
            }
            let chat_path = chat_entry.path();
            if chat_path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            if read_gemini_session_id(&chat_path).as_deref() != Some(session_id) {
                continue;
            }

            return Some(RecoveredLocalProviderSession {
                binding: LocalProviderSessionBinding {
                    provider_type: ProviderType::GeminiCli,
                    agent_id: "gemini".to_owned(),
                    workspace_dir,
                },
                messages: read_gemini_transcript_messages(&chat_path, session_id),
            });
        }
    }
    None
}

fn read_gemini_transcript_messages(chat_file: &Path, session_id: &str) -> Vec<Value> {
    let Ok(raw) = fs::read_to_string(chat_file) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return Vec::new();
    };

    value
        .get("messages")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let timestamp = trimmed_string(item.get("timestamp").and_then(Value::as_str))?;
                    let message_type = item.get("type").and_then(Value::as_str)?;
                    match message_type {
                        "user" => {
                            let text = trimmed_string(item.get("content").and_then(Value::as_str))?;
                            Some(build_imported_message(
                                "user",
                                text,
                                timestamp,
                                &ProviderType::GeminiCli,
                                session_id,
                            ))
                        }
                        "gemini" => {
                            let text = trimmed_string(item.get("content").and_then(Value::as_str))?;
                            Some(build_imported_message(
                                "assistant",
                                text,
                                timestamp,
                                &ProviderType::GeminiCli,
                                session_id,
                            ))
                        }
                        _ => None,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
fn locate_claude_session_binding(
    session_id: &str,
    projects_dir: &Path,
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
            provider_type: ProviderType::ClaudeCode,
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
        if let Some(cwd) = value.get("cwd").and_then(Value::as_str) {
            if let Some(path) = normalized_existing_path(cwd) {
                return Some(path);
            }
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
fn locate_gemini_session_binding(
    session_id: &str,
    tmp_dir: &Path,
) -> Option<LocalProviderSessionBinding> {
    for entry in fs::read_dir(tmp_dir).ok()?.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }

        let project_dir = entry.path();
        let Some(workspace_dir) = fs::read_to_string(project_dir.join(".project_root"))
            .ok()
            .and_then(|raw| normalized_existing_path(&raw))
        else {
            continue;
        };
        let chats_dir = project_dir.join("chats");
        if !chats_dir.is_dir() {
            continue;
        }

        for chat_entry in fs::read_dir(chats_dir).ok()?.flatten() {
            let Ok(chat_type) = chat_entry.file_type() else {
                continue;
            };
            if !chat_type.is_file() {
                continue;
            }
            let chat_path = chat_entry.path();
            if chat_path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            if read_gemini_session_id(&chat_path).as_deref() == Some(session_id) {
                return Some(LocalProviderSessionBinding {
                    provider_type: ProviderType::GeminiCli,
                    agent_id: "gemini".to_owned(),
                    workspace_dir,
                });
            }
        }
    }
    None
}

fn read_gemini_session_id(chat_file: &Path) -> Option<String> {
    let raw = fs::read_to_string(chat_file).ok()?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    value
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests;
