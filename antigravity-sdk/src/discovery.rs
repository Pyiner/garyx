use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::transcript::{first_user_input_text, transcript_path};

pub(crate) fn antigravity_base_dir(brain_root: &Path) -> PathBuf {
    if brain_root.file_name().and_then(|value| value.to_str()) == Some("brain")
        && let Some(parent) = brain_root.parent()
    {
        return parent.to_path_buf();
    }
    brain_root
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| brain_root.to_path_buf())
}

fn uuid_candidates(contents: &str) -> Vec<String> {
    contents
        .split(|ch: char| !(ch.is_ascii_hexdigit() || ch == '-'))
        .filter(|candidate| {
            candidate.len() == 36
                && candidate.chars().enumerate().all(|(index, ch)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        ch == '-'
                    } else {
                        ch.is_ascii_hexdigit()
                    }
                })
        })
        .map(ToOwned::to_owned)
        .collect()
}

fn discover_from_run_log(log_path: &Path, brain_root: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(log_path).ok()?;
    uuid_candidates(&contents)
        .into_iter()
        .find(|candidate| transcript_path(brain_root, candidate).exists())
}

fn normalized_contains(haystack: &str, needle: &str) -> bool {
    let haystack = haystack.split_whitespace().collect::<Vec<_>>().join(" ");
    let needle = needle.split_whitespace().collect::<Vec<_>>().join(" ");
    let needle = needle.trim();
    !needle.is_empty() && haystack.contains(needle)
}

fn prompt_matches_text(prompt_text: &str, prompt: &str, discovery_text: &str) -> bool {
    if normalized_contains(prompt_text, prompt) || normalized_contains(prompt, prompt_text) {
        return true;
    }
    let prompt_prefix = prompt.chars().take(512).collect::<String>();
    normalized_contains(prompt_text, &prompt_prefix)
        || normalized_contains(prompt_text, discovery_text)
}

fn conversation_matches_prompt(
    brain_root: &Path,
    conversation_id: &str,
    prompt: &str,
    discovery_text: &str,
) -> bool {
    let path = transcript_path(brain_root, conversation_id);
    first_user_input_text(&path)
        .is_some_and(|text| prompt_matches_text(&text, prompt, discovery_text))
}

fn candidate_conversation_ids(
    conversations_dir: &Path,
    run_start: SystemTime,
) -> Vec<(String, SystemTime)> {
    let threshold = run_start
        .checked_sub(Duration::from_secs(2))
        .unwrap_or(run_start);
    let mut candidates = std::fs::read_dir(conversations_dir)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.flatten())
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("db") {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            if modified < threshold {
                return None;
            }
            let id = path.file_stem()?.to_str()?.trim().to_owned();
            (!id.is_empty()).then_some((id, modified))
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(_, modified)| *modified);
    candidates
}

fn discover_from_conversations(
    conversations_dir: &Path,
    brain_root: &Path,
    run_start: SystemTime,
    prompt: &str,
    discovery_text: &str,
) -> Option<String> {
    let candidates = candidate_conversation_ids(conversations_dir, run_start);
    if candidates.is_empty() {
        return None;
    }
    let matched = candidates
        .iter()
        .filter(|(id, _)| conversation_matches_prompt(brain_root, id, prompt, discovery_text))
        .cloned()
        .collect::<Vec<_>>();
    let selected = if matched.is_empty() && candidates.len() == 1 {
        candidates
    } else {
        matched
    };
    selected
        .into_iter()
        .max_by_key(|(_, modified)| *modified)
        .map(|(id, _)| id)
}

pub(crate) async fn discover_conversation_id(
    run_log: &Path,
    brain_root: &Path,
    run_start: SystemTime,
    prompt: &str,
    discovery_text: &str,
    timeout: Duration,
    poll_interval: Duration,
) -> Option<String> {
    let conversations_dir = antigravity_base_dir(brain_root).join("conversations");
    let started = Instant::now();
    loop {
        if let Some(id) = discover_from_run_log(run_log, brain_root) {
            return Some(id);
        }
        if let Some(id) = discover_from_conversations(
            &conversations_dir,
            brain_root,
            run_start,
            prompt,
            discovery_text,
        ) {
            return Some(id);
        }
        if started.elapsed() >= timeout {
            return None;
        }
        tokio::time::sleep(poll_interval).await;
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn discovery_uses_prompt_match_when_multiple_candidates_exist() {
        let temp = tempfile::tempdir().expect("tempdir");
        let base = temp.path().join(".gemini").join("antigravity-cli");
        let brain = base.join("brain");
        let conversations = base.join("conversations");
        fs::create_dir_all(&conversations).expect("conversations");
        for (id, prompt) in [
            ("synthetic-wrong-session", "other prompt"),
            ("synthetic-right-session", "target prompt"),
        ] {
            fs::write(conversations.join(format!("{id}.db")), "").expect("db");
            let logs = brain.join(id).join(".system_generated").join("logs");
            fs::create_dir_all(&logs).expect("logs");
            fs::write(
                logs.join("transcript.jsonl"),
                format!(r#"{{"type":"USER_INPUT","step_index":1,"content":"{prompt}"}}"#),
            )
            .expect("transcript");
        }

        let discovered = discover_from_conversations(
            &conversations,
            &brain,
            SystemTime::now()
                .checked_sub(Duration::from_secs(1))
                .expect("time"),
            "target prompt",
            "target prompt",
        );

        assert_eq!(discovered.as_deref(), Some("synthetic-right-session"));
    }

    #[test]
    fn single_recent_db_remains_the_discovery_fallback() {
        let temp = tempfile::tempdir().expect("tempdir");
        let base = temp.path().join("antigravity-cli");
        let brain = base.join("brain");
        let conversations = base.join("conversations");
        fs::create_dir_all(&conversations).expect("conversations");
        fs::write(conversations.join("synthetic-session.db"), "").expect("db");

        let discovered = discover_from_conversations(
            &conversations,
            &brain,
            SystemTime::now(),
            "prompt",
            "prompt",
        );

        assert_eq!(discovered.as_deref(), Some("synthetic-session"));
    }

    #[test]
    fn run_log_uuid_candidate_takes_protocol_precedence() {
        let temp = tempfile::tempdir().expect("tempdir");
        let brain = temp.path().join("brain");
        let conversation_id = "10000000-0000-4000-8000-000000000001";
        let logs = brain
            .join(conversation_id)
            .join(".system_generated")
            .join("logs");
        fs::create_dir_all(&logs).expect("logs");
        fs::write(logs.join("transcript.jsonl"), "").expect("transcript");
        let run_log = temp.path().join("run.log");
        fs::write(
            &run_log,
            format!("synthetic log prefix conversation={conversation_id}\n"),
        )
        .expect("run log");

        assert_eq!(
            discover_from_run_log(&run_log, &brain).as_deref(),
            Some(conversation_id)
        );
    }
}
