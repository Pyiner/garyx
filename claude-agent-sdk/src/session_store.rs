//! Claude Agent SDK SessionStore support.
//!
//! The behavioral contract in this module tracks the official TypeScript SDK
//! `@anthropic-ai/claude-agent-sdk@0.3.217`. Garyx intentionally ships one
//! production adapter: a local Claude-compatible `projects` directory.

use std::collections::{BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::error::{ClaudeSDKError, Result};

pub(crate) const SESSION_STORE_MAX_PENDING_ENTRIES: usize = 500;
pub(crate) const SESSION_STORE_MAX_PENDING_BYTES: usize = 1024 * 1024;
pub(crate) const SESSION_STORE_APPEND_TIMEOUT: Duration = Duration::from_secs(60);
pub(crate) const SESSION_STORE_LOAD_TIMEOUT: Duration = Duration::from_secs(60);
const SESSION_STORE_RETRY_BACKOFF: [Duration; 2] =
    [Duration::from_millis(200), Duration::from_millis(800)];

/// Identifies one main or nested Claude transcript in a SessionStore.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionKey {
    pub project_key: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subpath: Option<String>,
}

impl SessionKey {
    pub fn main(project_key: impl Into<String>, session_id: impl Into<String>) -> Self {
        Self {
            project_key: project_key.into(),
            session_id: session_id.into(),
            subpath: None,
        }
    }
}

/// One opaque JSONL transcript line.
pub type SessionStoreEntry = Value;

/// Session identifier and storage modification time returned by listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStoreSession {
    pub session_id: String,
    /// Integer Unix epoch milliseconds.
    pub mtime: u64,
}

/// Result of reconciling native Claude transcript copies into the canonical
/// local SessionStore.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionReconcileSummary {
    pub sessions_scanned: usize,
    pub sessions_failed: usize,
    pub keys_promoted: usize,
    pub keys_unchanged: usize,
}

impl SessionReconcileSummary {
    fn record(&mut self, outcome: ReconcileKeyOutcome) {
        match outcome {
            ReconcileKeyOutcome::Missing => {}
            ReconcileKeyOutcome::Unchanged => self.keys_unchanged += 1,
            ReconcileKeyOutcome::Promoted => self.keys_promoted += 1,
        }
    }

    fn merge(&mut self, other: Self) {
        self.sessions_scanned += other.sessions_scanned;
        self.sessions_failed += other.sessions_failed;
        self.keys_promoted += other.keys_promoted;
        self.keys_unchanged += other.keys_unchanged;
    }
}

/// Transcript-mirror flush policy from the TypeScript SDK.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SessionStoreFlush {
    #[default]
    Batched,
    Eager,
}

/// Async SessionStore protocol.
///
/// Optional TypeScript methods return `None` from their Rust counterparts
/// when an adapter does not implement them. `LocalDirectorySessionStore`
/// implements the complete resume-relevant surface.
#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn append(&self, key: &SessionKey, entries: &[SessionStoreEntry]) -> Result<()>;

    async fn load(&self, key: &SessionKey) -> Result<Option<Vec<SessionStoreEntry>>>;

    async fn list_sessions(&self, _project_key: &str) -> Result<Option<Vec<SessionStoreSession>>> {
        Ok(None)
    }

    async fn delete(&self, _key: &SessionKey) -> Result<bool> {
        Ok(false)
    }

    async fn list_subkeys(&self, _key: &SessionKey) -> Result<Option<Vec<String>>> {
        Ok(None)
    }

    /// Native Claude projects root, when the backend directly exposes one.
    fn native_projects_root(&self) -> Option<&Path> {
        None
    }
}

/// Default canonical transcript root shared with terminal Claude Code.
pub fn default_claude_projects_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("projects")
}

fn absolute_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn normalized_existing_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| absolute_path(path))
}

pub(crate) fn same_path(left: &Path, right: &Path) -> bool {
    normalized_existing_path(left) == normalized_existing_path(right)
}

/// Derive the exact project key used by the TypeScript SDK at v0.3.217.
pub fn session_project_key(cwd: impl AsRef<Path>) -> String {
    let cwd = normalized_existing_path(cwd.as_ref());
    let text = cwd.to_string_lossy();
    // JavaScript's non-`u` regular expression walks UTF-16 code units. An
    // astral code point therefore becomes two dashes, not one.
    let mapped = text
        .encode_utf16()
        .map(|unit| {
            if unit <= u16::from(u8::MAX) && (unit as u8).is_ascii_alphanumeric() {
                char::from(unit as u8)
            } else {
                '-'
            }
        })
        .collect::<String>();
    if mapped.len() <= 200 {
        return mapped;
    }

    // The TS implementation applies its signed 32-bit hash to JavaScript
    // UTF-16 code units, then Math.abs(...).toString(36).
    let mut hash = 0_i32;
    for code_unit in text.encode_utf16() {
        hash = hash.wrapping_mul(31).wrapping_add(i32::from(code_unit));
    }
    let magnitude = i64::from(hash).abs();
    format!("{}-{}", &mapped[..200], base36(magnitude as u64))
}

fn base36(mut value: u64) -> String {
    if value == 0 {
        return "0".to_owned();
    }
    let mut out = Vec::new();
    while value > 0 {
        let digit = (value % 36) as u8;
        out.push(if digit < 10 {
            b'0' + digit
        } else {
            b'a' + (digit - 10)
        });
        value /= 36;
    }
    out.reverse();
    String::from_utf8(out).expect("base36 only emits ASCII")
}

fn validate_single_segment(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains('\0')
    {
        return Err(ClaudeSDKError::SessionStore(format!(
            "invalid {label} path segment"
        )));
    }
    Ok(())
}

fn normalized_subpath(value: &str) -> Result<PathBuf> {
    if value.is_empty() || value.contains('\\') {
        return Err(ClaudeSDKError::SessionStore(
            "invalid empty SessionStore subpath".to_owned(),
        ));
    }
    let mut normalized = PathBuf::new();
    for component in Path::new(value).components() {
        match component {
            Component::Normal(segment) if !segment.is_empty() => normalized.push(segment),
            _ => {
                return Err(ClaudeSDKError::SessionStore(
                    "unsafe SessionStore subpath".to_owned(),
                ));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(ClaudeSDKError::SessionStore(
            "invalid empty SessionStore subpath".to_owned(),
        ));
    }
    Ok(normalized)
}

/// A SessionStore backed by a native Claude `projects` directory.
#[derive(Clone)]
pub struct LocalDirectorySessionStore {
    root: PathBuf,
    legacy_roots: Arc<Vec<PathBuf>>,
    path_locks: Arc<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReconcileKeyOutcome {
    Missing,
    Unchanged,
    Promoted,
}

#[derive(Debug)]
struct SessionCandidate {
    root: PathBuf,
    modified: SystemTime,
    canonical: bool,
}

impl std::fmt::Debug for LocalDirectorySessionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalDirectorySessionStore")
            .field("root", &self.root)
            .field("legacy_roots", &self.legacy_roots)
            .finish_non_exhaustive()
    }
}

impl LocalDirectorySessionStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: absolute_path(root.into()),
            legacy_roots: Arc::new(Vec::new()),
            path_locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_legacy_roots(mut self, roots: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut unique: Vec<PathBuf> = Vec::new();
        for root in roots {
            let root = absolute_path(root);
            if same_path(&root, &self.root) || unique.iter().any(|known| same_path(known, &root)) {
                continue;
            }
            unique.push(root);
        }
        unique.sort();
        self.legacy_roots = Arc::new(unique);
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path_for_key_at(root: &Path, key: &SessionKey) -> Result<PathBuf> {
        validate_single_segment(&key.project_key, "projectKey")?;
        validate_single_segment(&key.session_id, "sessionId")?;
        let project_dir = root.join(&key.project_key);
        match key.subpath.as_deref() {
            None => Ok(project_dir.join(format!("{}.jsonl", key.session_id))),
            Some(subpath) => Ok(with_suffix(
                project_dir
                    .join(&key.session_id)
                    .join(normalized_subpath(subpath)?),
                ".jsonl",
            )?),
        }
    }

    pub fn path_for_key(&self, key: &SessionKey) -> Result<PathBuf> {
        Self::path_for_key_at(&self.root, key)
    }

    fn metadata_path_for_key_at(root: &Path, key: &SessionKey) -> Result<Option<PathBuf>> {
        if key.subpath.is_none() {
            return Ok(None);
        }
        Ok(Some(
            Self::path_for_key_at(root, key)?.with_extension("meta.json"),
        ))
    }

    async fn path_lock(&self, path: &Path) -> Arc<Mutex<()>> {
        let mut locks = self.path_locks.lock().await;
        locks
            .entry(path.to_path_buf())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    async fn ensure_safe_parent(&self, path: &Path) -> Result<()> {
        let parent = path.parent().ok_or_else(|| {
            ClaudeSDKError::SessionStore("SessionStore path has no parent".to_owned())
        })?;
        tokio::fs::create_dir_all(&self.root).await?;
        tokio::fs::create_dir_all(parent).await?;
        let canonical_root = tokio::fs::canonicalize(&self.root).await?;
        let canonical_parent = tokio::fs::canonicalize(parent).await?;
        if !canonical_parent.starts_with(&canonical_root) {
            return Err(ClaudeSDKError::SessionStore(
                "SessionStore path escaped its configured root".to_owned(),
            ));
        }
        Ok(())
    }

    async fn read_entries_at(root: &Path, key: &SessionKey) -> Result<Option<Vec<Value>>> {
        let path = Self::path_for_key_at(root, key)?;
        let raw = match tokio::fs::read_to_string(&path).await {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error.into()),
        };
        let mut entries = Vec::new();
        if !raw.is_empty() {
            for (index, line) in raw.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let entry = serde_json::from_str::<Value>(line).map_err(|error| {
                    ClaudeSDKError::SessionStore(format!(
                        "invalid JSONL entry at {}:{}: {error}",
                        path.display(),
                        index + 1
                    ))
                })?;
                entries.push(entry);
            }
        }

        if let Some(metadata_path) = Self::metadata_path_for_key_at(root, key)? {
            match tokio::fs::read_to_string(&metadata_path).await {
                Ok(raw) => {
                    let mut metadata = serde_json::from_str::<Value>(&raw).map_err(|error| {
                        ClaudeSDKError::SessionStore(format!(
                            "invalid subagent metadata at {}: {error}",
                            metadata_path.display()
                        ))
                    })?;
                    let object = metadata.as_object_mut().ok_or_else(|| {
                        ClaudeSDKError::SessionStore(format!(
                            "subagent metadata at {} is not an object",
                            metadata_path.display()
                        ))
                    })?;
                    object.insert(
                        "type".to_owned(),
                        Value::String("agent_metadata".to_owned()),
                    );
                    entries.push(metadata);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }

        if entries.is_empty() {
            Ok(None)
        } else {
            Ok(Some(entries))
        }
    }

    async fn candidate_at(
        root: &Path,
        key: &SessionKey,
        canonical: bool,
    ) -> Result<Option<SessionCandidate>> {
        let transcript_path = Self::path_for_key_at(root, key)?;
        let (mut exists, mut modified) = match tokio::fs::symlink_metadata(&transcript_path).await {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                if canonical {
                    return Err(ClaudeSDKError::SessionStore(format!(
                        "canonical SessionStore transcript is a symlink: {}",
                        transcript_path.display()
                    )));
                }
                return Ok(None);
            }
            Ok(metadata) if metadata.is_file() => (true, metadata.modified().unwrap_or(UNIX_EPOCH)),
            Ok(_) => return Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (false, UNIX_EPOCH),
            Err(error) => return Err(error.into()),
        };

        if let Some(metadata_path) = Self::metadata_path_for_key_at(root, key)? {
            match tokio::fs::symlink_metadata(&metadata_path).await {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    if canonical {
                        return Err(ClaudeSDKError::SessionStore(format!(
                            "canonical SessionStore metadata is a symlink: {}",
                            metadata_path.display()
                        )));
                    }
                    return Ok(None);
                }
                Ok(metadata) if metadata.is_file() => {
                    exists = true;
                    modified = modified.max(metadata.modified().unwrap_or(UNIX_EPOCH));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }

        Ok(exists.then(|| SessionCandidate {
            root: root.to_path_buf(),
            modified,
            canonical,
        }))
    }

    fn candidate_wins(candidate: &SessionCandidate, current: &SessionCandidate) -> bool {
        candidate.modified > current.modified
            || (candidate.modified == current.modified
                && ((candidate.canonical && !current.canonical)
                    || (candidate.canonical == current.canonical && candidate.root < current.root)))
    }

    async fn replace_canonical_entries(
        &self,
        key: &SessionKey,
        entries: &[SessionStoreEntry],
    ) -> Result<()> {
        let path = self.path_for_key(key)?;
        self.ensure_safe_parent(&path).await?;

        let (transcript_entries, metadata_entry): (Vec<&Value>, Option<&Value>) =
            if key.subpath.is_some() {
                let mut transcript = Vec::new();
                let mut metadata = None;
                for entry in entries {
                    if entry.get("type").and_then(Value::as_str) == Some("agent_metadata") {
                        metadata = Some(entry);
                    } else {
                        transcript.push(entry);
                    }
                }
                (transcript, metadata)
            } else {
                (entries.iter().collect(), None)
            };

        let transcript_entries = transcript_entries
            .into_iter()
            .cloned()
            .collect::<Vec<Value>>();
        write_entries_atomic(&path, &transcript_entries).await?;

        if key.subpath.is_some() {
            let metadata_path = path.with_extension("meta.json");
            if let Some(metadata_entry) = metadata_entry {
                let mut metadata = metadata_entry.clone();
                let object = metadata.as_object_mut().ok_or_else(|| {
                    ClaudeSDKError::SessionStore("agent_metadata entry is not an object".to_owned())
                })?;
                object.remove("type");
                atomic_write_private(&metadata_path, &serde_json::to_vec(&metadata)?).await?;
            } else {
                remove_file_if_present(&metadata_path).await?;
            }
        }

        Ok(())
    }

    /// Reconcile one main or subagent transcript. Parsed entry equality is
    /// checked before mtime so a prior materialization cannot make an
    /// unchanged managed copy feed back into the canonical store.
    async fn reconcile_key(&self, key: &SessionKey) -> Result<ReconcileKeyOutcome> {
        let canonical_path = self.path_for_key(key)?;
        let lock = self.path_lock(&canonical_path).await;
        let _guard = lock.lock().await;

        let mut candidates = Vec::new();
        if let Some(candidate) = Self::candidate_at(&self.root, key, true).await? {
            candidates.push(candidate);
        }
        for root in self.legacy_roots.iter() {
            if let Some(candidate) = Self::candidate_at(root, key, false).await? {
                candidates.push(candidate);
            }
        }

        let Some(winner) = candidates.iter().reduce(|current, candidate| {
            if Self::candidate_wins(candidate, current) {
                candidate
            } else {
                current
            }
        }) else {
            return Ok(ReconcileKeyOutcome::Missing);
        };

        if winner.canonical {
            // Validate the winning transcript now. Stale corrupt profile
            // copies do not poison a healthy, newer canonical lineage.
            Self::read_entries_at(&self.root, key).await?;
            return Ok(ReconcileKeyOutcome::Unchanged);
        }

        let Some(winner_entries) = Self::read_entries_at(&winner.root, key).await? else {
            return Ok(ReconcileKeyOutcome::Missing);
        };
        if Self::read_entries_at(&self.root, key)
            .await
            .is_ok_and(|canonical| canonical.as_ref() == Some(&winner_entries))
        {
            return Ok(ReconcileKeyOutcome::Unchanged);
        }

        self.replace_canonical_entries(key, &winner_entries).await?;
        Ok(ReconcileKeyOutcome::Promoted)
    }

    async fn session_subkeys_across_roots(&self, key: &SessionKey) -> Result<Vec<String>> {
        let mut subkeys = BTreeSet::new();
        for root in std::iter::once(&self.root).chain(self.legacy_roots.iter()) {
            let session_dir = root.join(&key.project_key).join(&key.session_id);
            subkeys.extend(session_subpaths(&session_dir).await?);
        }
        Ok(subkeys.into_iter().collect())
    }

    /// Reconcile one native Claude session, treating the main transcript and
    /// every subagent transcript as independent LWW keys.
    pub async fn reconcile_session(
        &self,
        project_key: &str,
        session_id: &str,
    ) -> Result<SessionReconcileSummary> {
        validate_single_segment(project_key, "projectKey")?;
        validate_single_segment(session_id, "sessionId")?;
        let main = SessionKey::main(project_key, session_id);
        let mut summary = SessionReconcileSummary {
            sessions_scanned: 1,
            ..SessionReconcileSummary::default()
        };
        summary.record(self.reconcile_key(&main).await?);
        for subpath in self.session_subkeys_across_roots(&main).await? {
            let subkey = SessionKey {
                project_key: project_key.to_owned(),
                session_id: session_id.to_owned(),
                subpath: Some(subpath),
            };
            summary.record(self.reconcile_key(&subkey).await?);
        }
        Ok(summary)
    }

    /// Startup/account-switch repair pass. This only promotes the newest
    /// durable copy into the canonical root; managed profile copies are never
    /// rewritten by the sweep.
    pub async fn reconcile_all(&self) -> Result<SessionReconcileSummary> {
        if self.legacy_roots.is_empty() {
            return Ok(SessionReconcileSummary::default());
        }
        let mut sessions = BTreeSet::new();
        for root in std::iter::once(&self.root).chain(self.legacy_roots.iter()) {
            sessions.extend(session_ids_below_projects_root(root).await?);
        }

        let mut summary = SessionReconcileSummary::default();
        for (project_key, session_id) in sessions {
            match self.reconcile_session(&project_key, &session_id).await {
                Ok(session_summary) => summary.merge(session_summary),
                Err(error) => {
                    summary.sessions_scanned += 1;
                    summary.sessions_failed += 1;
                    tracing::warn!(
                        project_key,
                        session_id,
                        error = %error,
                        "skipping failed Claude session during reconciliation sweep"
                    );
                }
            }
        }
        Ok(summary)
    }
}

async fn session_ids_below_projects_root(root: &Path) -> Result<BTreeSet<(String, String)>> {
    let mut sessions = BTreeSet::new();
    let mut projects = match tokio::fs::read_dir(root).await {
        Ok(projects) => projects,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(sessions),
        Err(error) => return Err(error.into()),
    };
    while let Some(project) = projects.next_entry().await? {
        let file_type = project.file_type().await?;
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }
        let Some(project_key) = project.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };
        if validate_single_segment(&project_key, "projectKey").is_err() {
            continue;
        }
        let mut entries = tokio::fs::read_dir(project.path()).await?;
        while let Some(entry) = entries.next_entry().await? {
            let file_type = entry.file_type().await?;
            if file_type.is_symlink() {
                continue;
            }
            let session_id = if file_type.is_file()
                && entry.path().extension().and_then(|value| value.to_str()) == Some("jsonl")
            {
                entry
                    .path()
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .map(ToOwned::to_owned)
            } else if file_type.is_dir() {
                entry.file_name().to_str().map(ToOwned::to_owned)
            } else {
                None
            };
            if let Some(session_id) = session_id.filter(|value| valid_session_id(value)) {
                sessions.insert((project_key.clone(), session_id));
            }
        }
    }
    Ok(sessions)
}

fn with_suffix(mut path: PathBuf, suffix: &str) -> Result<PathBuf> {
    let name = path.file_name().ok_or_else(|| {
        ClaudeSDKError::SessionStore("SessionStore subpath has no filename".to_owned())
    })?;
    let mut suffixed = name.to_os_string();
    suffixed.push(suffix);
    path.set_file_name(suffixed);
    Ok(path)
}

#[async_trait]
impl SessionStore for LocalDirectorySessionStore {
    async fn append(&self, key: &SessionKey, entries: &[SessionStoreEntry]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let path = self.path_for_key(key)?;
        let lock = self.path_lock(&path).await;
        let _guard = lock.lock().await;
        self.ensure_safe_parent(&path).await?;

        let (transcript_entries, metadata_entry): (Vec<&Value>, Option<&Value>) =
            if key.subpath.is_some() {
                let mut transcript = Vec::new();
                let mut metadata = None;
                for entry in entries {
                    if entry.get("type").and_then(Value::as_str) == Some("agent_metadata") {
                        metadata = Some(entry);
                    } else {
                        transcript.push(entry);
                    }
                }
                (transcript, metadata)
            } else {
                (entries.iter().collect(), None)
            };

        if !transcript_entries.is_empty() {
            let mut payload = Vec::new();
            for entry in transcript_entries {
                serde_json::to_writer(&mut payload, entry)?;
                payload.push(b'\n');
            }
            append_private_file(&path, &payload).await?;
        }

        if let Some(metadata_entry) = metadata_entry {
            let mut metadata = metadata_entry.clone();
            let object = metadata.as_object_mut().ok_or_else(|| {
                ClaudeSDKError::SessionStore("agent_metadata entry is not an object".to_owned())
            })?;
            object.remove("type");
            let metadata_path = path.with_extension("meta.json");
            let payload = serde_json::to_vec(&metadata)?;
            atomic_write_private(&metadata_path, &payload).await?;
        }

        Ok(())
    }

    async fn load(&self, key: &SessionKey) -> Result<Option<Vec<SessionStoreEntry>>> {
        self.reconcile_key(key).await?;
        Self::read_entries_at(&self.root, key).await
    }

    async fn list_sessions(&self, project_key: &str) -> Result<Option<Vec<SessionStoreSession>>> {
        validate_single_segment(project_key, "projectKey")?;
        let project_dir = self.root.join(project_key);
        let mut reader = match tokio::fs::read_dir(&project_dir).await {
            Ok(reader) => reader,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Some(Vec::new()));
            }
            Err(error) => return Err(error.into()),
        };
        let mut sessions = Vec::new();
        while let Some(entry) = reader.next_entry().await? {
            let file_type = entry.file_type().await?;
            if !file_type.is_file()
                || entry.path().extension().and_then(|v| v.to_str()) != Some("jsonl")
            {
                continue;
            }
            let Some(session_id) = entry
                .path()
                .file_stem()
                .and_then(|v| v.to_str())
                .map(ToOwned::to_owned)
            else {
                continue;
            };
            let modified = entry.metadata().await?.modified().unwrap_or(UNIX_EPOCH);
            let mtime = modified
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            sessions.push(SessionStoreSession { session_id, mtime });
        }
        Ok(Some(sessions))
    }

    async fn delete(&self, key: &SessionKey) -> Result<bool> {
        let path = self.path_for_key(key)?;
        remove_file_if_present(&path).await?;
        if key.subpath.is_some() {
            remove_file_if_present(&path.with_extension("meta.json")).await?;
        } else {
            remove_dir_if_present(&self.root.join(&key.project_key).join(&key.session_id)).await?;
        }
        Ok(true)
    }

    async fn list_subkeys(&self, key: &SessionKey) -> Result<Option<Vec<String>>> {
        validate_single_segment(&key.project_key, "projectKey")?;
        validate_single_segment(&key.session_id, "sessionId")?;
        let subkeys = self.session_subkeys_across_roots(key).await?;
        for subpath in &subkeys {
            let subkey = SessionKey {
                project_key: key.project_key.clone(),
                session_id: key.session_id.clone(),
                subpath: Some(subpath.clone()),
            };
            self.reconcile_key(&subkey).await?;
        }
        Ok(Some(subkeys))
    }

    fn native_projects_root(&self) -> Option<&Path> {
        Some(&self.root)
    }
}

async fn session_subpaths(session_dir: &Path) -> Result<Vec<String>> {
    let mut subpaths = BTreeSet::new();
    let mut pending = vec![session_dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let mut reader = match tokio::fs::read_dir(&dir).await {
            Ok(reader) => reader,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        while let Some(entry) = reader.next_entry().await? {
            let file_type = entry.file_type().await?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                pending.push(entry.path());
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let relative = match entry.path().strip_prefix(session_dir) {
                Ok(relative) => relative.to_path_buf(),
                Err(_) => continue,
            };
            let text = relative.to_string_lossy();
            let subpath = text
                .strip_suffix(".jsonl")
                .or_else(|| text.strip_suffix(".meta.json"));
            if let Some(subpath) = subpath.filter(|value| !value.is_empty()) {
                subpaths.insert(subpath.replace(std::path::MAIN_SEPARATOR, "/"));
            }
        }
    }
    Ok(subpaths.into_iter().collect())
}

#[cfg(unix)]
async fn append_private_file(path: &Path, payload: &[u8]) -> Result<()> {
    let mut options = tokio::fs::OpenOptions::new();
    options.create(true).append(true).mode(0o600);
    let mut file = options.open(path).await?;
    file.write_all(payload).await?;
    file.flush().await?;
    Ok(())
}

#[cfg(not(unix))]
async fn append_private_file(path: &Path, payload: &[u8]) -> Result<()> {
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    file.write_all(payload).await?;
    file.flush().await?;
    Ok(())
}

async fn atomic_write_private(path: &Path, payload: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        ClaudeSDKError::SessionStore("SessionStore path has no parent".to_owned())
    })?;
    tokio::fs::create_dir_all(parent).await?;
    let temp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("session-store"),
        Uuid::new_v4()
    ));
    let write_result = write_private_new_file(&temp, payload).await;
    if let Err(error) = write_result {
        let _ = tokio::fs::remove_file(&temp).await;
        return Err(error);
    }
    if let Err(error) = tokio::fs::rename(&temp, path).await {
        let _ = tokio::fs::remove_file(&temp).await;
        return Err(error.into());
    }
    Ok(())
}

#[cfg(unix)]
async fn write_private_new_file(path: &Path, payload: &[u8]) -> Result<()> {
    let mut options = tokio::fs::OpenOptions::new();
    options.create_new(true).write(true).mode(0o600);
    let mut file = options.open(path).await?;
    file.write_all(payload).await?;
    file.flush().await?;
    file.sync_all().await?;
    Ok(())
}

#[cfg(not(unix))]
async fn write_private_new_file(path: &Path, payload: &[u8]) -> Result<()> {
    let mut file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .await?;
    file.write_all(payload).await?;
    file.flush().await?;
    file.sync_all().await?;
    Ok(())
}

async fn remove_file_if_present(path: &Path) -> Result<()> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

async fn remove_dir_if_present(path: &Path) -> Result<()> {
    match tokio::fs::remove_dir_all(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn valid_session_id(value: &str) -> bool {
    if value.len() != 36 {
        return false;
    }
    value.bytes().enumerate().all(|(index, byte)| {
        if matches!(index, 8 | 13 | 18 | 23) {
            byte == b'-'
        } else {
            byte.is_ascii_hexdigit()
        }
    })
}

/// Resolve the config projects root used by a Claude subprocess.
pub(crate) fn launched_projects_root(env: &HashMap<String, String>) -> PathBuf {
    launched_projects_root_with_inherited(
        env,
        std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from),
    )
}

fn launched_projects_root_with_inherited(
    env: &HashMap<String, String>,
    inherited_config_dir: Option<PathBuf>,
) -> PathBuf {
    env.get("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or(inherited_config_dir)
        .unwrap_or_else(|| {
            default_claude_projects_dir()
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf()
        })
        .join("projects")
}

pub(crate) fn session_mirror_required(
    store: &Arc<dyn SessionStore>,
    launched_projects_root: &Path,
) -> bool {
    !store
        .native_projects_root()
        .is_some_and(|root| same_path(root, launched_projects_root))
}

/// Load and materialize a native session before spawning Claude Code.
pub(crate) async fn materialize_session_for_resume(
    store: &Arc<dyn SessionStore>,
    cwd: &Path,
    session_id: &str,
    target_projects_root: &Path,
    timeout: Duration,
) -> Result<bool> {
    if !valid_session_id(session_id) {
        return Ok(false);
    }
    let project_key = session_project_key(cwd);
    let key = SessionKey::main(project_key.clone(), session_id.to_owned());
    let entries = timeout_store_call(
        store.load(&key),
        timeout,
        format!(
            "SessionStore.load() timed out after {}ms for session {session_id}",
            timeout.as_millis()
        ),
    )
    .await?;
    let Some(entries) = entries.filter(|entries| !entries.is_empty()) else {
        return Ok(false);
    };

    let subkeys = timeout_store_call(
        store.list_subkeys(&key),
        timeout,
        format!(
            "SessionStore.listSubkeys() timed out after {}ms for session {session_id}",
            timeout.as_millis()
        ),
    )
    .await?;

    if store
        .native_projects_root()
        .is_some_and(|root| same_path(root, target_projects_root))
    {
        return Ok(true);
    }

    let project_dir = target_projects_root.join(&project_key);
    tokio::fs::create_dir_all(&project_dir).await?;
    let main_path = project_dir.join(format!("{session_id}.jsonl"));
    write_entries_atomic(&main_path, &entries).await?;

    let session_dir = project_dir.join(session_id);
    remove_dir_if_present(&session_dir).await?;
    let Some(subkeys) = subkeys else {
        return Ok(true);
    };

    for subpath in subkeys {
        let relative = match normalized_subpath(&subpath) {
            Ok(relative) => relative,
            Err(error) => {
                tracing::warn!(subpath, error = %error, "skipping unsafe SessionStore subpath");
                continue;
            }
        };
        let subkey = SessionKey {
            project_key: project_key.clone(),
            session_id: session_id.to_owned(),
            subpath: Some(subpath.clone()),
        };
        let loaded = timeout_store_call(
            store.load(&subkey),
            timeout,
            format!(
                "SessionStore.load() timed out after {}ms for session {session_id} subpath {subpath}",
                timeout.as_millis()
            ),
        )
        .await?;
        let Some(loaded) = loaded.filter(|entries| !entries.is_empty()) else {
            continue;
        };
        let mut transcript = Vec::new();
        let mut metadata = None;
        for entry in loaded {
            if entry.get("type").and_then(Value::as_str) == Some("agent_metadata") {
                metadata = Some(entry);
            } else {
                transcript.push(entry);
            }
        }
        let transcript_path = with_suffix(session_dir.join(&relative), ".jsonl")?;
        if !transcript.is_empty() {
            write_entries_atomic(&transcript_path, &transcript).await?;
        }
        if let Some(mut metadata) = metadata {
            let object = metadata.as_object_mut().ok_or_else(|| {
                ClaudeSDKError::SessionStore("agent_metadata entry is not an object".to_owned())
            })?;
            object.remove("type");
            atomic_write_private(
                &with_suffix(session_dir.join(&relative), ".meta.json")?,
                &serde_json::to_vec(&metadata)?,
            )
            .await?;
        }
    }
    Ok(true)
}

async fn write_entries_atomic(path: &Path, entries: &[Value]) -> Result<()> {
    let mut payload = Vec::new();
    for entry in entries {
        serde_json::to_writer(&mut payload, entry)?;
        payload.push(b'\n');
    }
    atomic_write_private(path, &payload).await
}

async fn timeout_store_call<F, T>(future: F, timeout: Duration, message: String) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    match tokio::time::timeout(timeout, future).await {
        Ok(result) => result,
        Err(_) => Err(ClaudeSDKError::Timeout(message)),
    }
}

/// Translate a CLI transcript-mirror file path into a SessionStore key.
pub(crate) fn session_key_from_mirror_path(
    file_path: &Path,
    projects_root: &Path,
) -> Option<SessionKey> {
    let relative = file_path.strip_prefix(projects_root).ok()?;
    let parts = relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => value.to_string_lossy().into_owned(),
            _ => String::new(),
        })
        .collect::<Vec<_>>();
    if parts.iter().any(String::is_empty) || parts.len() < 2 {
        return None;
    }
    let file = parts.last()?;
    if parts.len() == 2 && file.ends_with(".jsonl") {
        return Some(SessionKey::main(
            parts[0].clone(),
            file.strip_suffix(".jsonl")?.to_owned(),
        ));
    }
    // Official v0.3.217 requires project/session plus at least two nested
    // components for subagent transcript paths.
    if parts.len() < 4 {
        return None;
    }
    let mut nested = parts[2..].to_vec();
    *nested.last_mut()? = file.strip_suffix(".jsonl").unwrap_or(file).to_owned();
    Some(SessionKey {
        project_key: parts[0].clone(),
        session_id: parts[1].clone(),
        subpath: Some(nested.join("/")),
    })
}

#[derive(Debug, Clone)]
pub(crate) struct MirrorFailure {
    pub key: SessionKey,
    pub error: String,
}

#[derive(Debug)]
struct PendingMirrorFrame {
    file_path: PathBuf,
    entries: Vec<Value>,
    bytes: usize,
}

#[derive(Debug, Default)]
struct PendingMirrorState {
    frames: Vec<PendingMirrorFrame>,
    entries: usize,
    bytes: usize,
}

/// Batches `transcript_mirror` protocol frames with the official thresholds
/// and retry policy.
pub(crate) struct TranscriptMirrorBatcher {
    store: Arc<dyn SessionStore>,
    projects_root: PathBuf,
    pending: Mutex<PendingMirrorState>,
    flush_lock: Mutex<()>,
    max_pending_entries: usize,
    max_pending_bytes: usize,
    append_timeout: Duration,
}

impl TranscriptMirrorBatcher {
    pub(crate) fn new(
        store: Arc<dyn SessionStore>,
        projects_root: PathBuf,
        flush: SessionStoreFlush,
    ) -> Self {
        let eager = flush == SessionStoreFlush::Eager;
        Self {
            store,
            projects_root,
            pending: Mutex::new(PendingMirrorState::default()),
            flush_lock: Mutex::new(()),
            max_pending_entries: if eager {
                0
            } else {
                SESSION_STORE_MAX_PENDING_ENTRIES
            },
            max_pending_bytes: if eager {
                0
            } else {
                SESSION_STORE_MAX_PENDING_BYTES
            },
            append_timeout: SESSION_STORE_APPEND_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn with_limits(
        store: Arc<dyn SessionStore>,
        projects_root: PathBuf,
        max_pending_entries: usize,
        max_pending_bytes: usize,
        append_timeout: Duration,
    ) -> Self {
        Self {
            store,
            projects_root,
            pending: Mutex::new(PendingMirrorState::default()),
            flush_lock: Mutex::new(()),
            max_pending_entries,
            max_pending_bytes,
            append_timeout,
        }
    }

    pub(crate) async fn enqueue(
        &self,
        file_path: PathBuf,
        entries: Vec<Value>,
    ) -> Vec<MirrorFailure> {
        let bytes = serde_json::to_string(&entries)
            .map(|json| json.encode_utf16().count())
            .unwrap_or_default();
        let should_flush = {
            let mut pending = self.pending.lock().await;
            pending.entries += entries.len();
            pending.bytes += bytes;
            pending.frames.push(PendingMirrorFrame {
                file_path,
                entries,
                bytes,
            });
            pending.entries > self.max_pending_entries || pending.bytes > self.max_pending_bytes
        };
        if should_flush {
            self.flush().await
        } else {
            Vec::new()
        }
    }

    pub(crate) async fn flush(&self) -> Vec<MirrorFailure> {
        let _flush_guard = self.flush_lock.lock().await;
        let frames = {
            let mut pending = self.pending.lock().await;
            pending.entries = 0;
            pending.bytes = 0;
            std::mem::take(&mut pending.frames)
        };
        if frames.is_empty() {
            return Vec::new();
        }

        let mut groups: Vec<(PathBuf, Vec<Value>)> = Vec::new();
        let mut indexes = HashMap::<PathBuf, usize>::new();
        for frame in frames {
            let _ = frame.bytes;
            if let Some(index) = indexes.get(&frame.file_path).copied() {
                groups[index].1.extend(frame.entries);
            } else {
                indexes.insert(frame.file_path.clone(), groups.len());
                groups.push((frame.file_path, frame.entries));
            }
        }

        let mut failures = Vec::new();
        for (file_path, entries) in groups {
            let Some(key) = session_key_from_mirror_path(&file_path, &self.projects_root) else {
                tracing::warn!(
                    file_path = %file_path.display(),
                    projects_root = %self.projects_root.display(),
                    "dropping transcript mirror frame outside Claude projects root"
                );
                continue;
            };
            if let Err(error) = self.send_with_retry(&key, &entries).await {
                tracing::error!(
                    file_path = %file_path.display(),
                    error = %error,
                    "SessionStore mirror batch failed after bounded retry"
                );
                failures.push(MirrorFailure {
                    key,
                    error: error.to_string(),
                });
            }
        }
        failures
    }

    async fn send_with_retry(&self, key: &SessionKey, entries: &[Value]) -> Result<()> {
        let timeout_message = format!(
            "SessionStore.append() timed out after {}ms for {}/{}",
            self.append_timeout.as_millis(),
            key.project_key,
            key.session_id
        );
        for attempt in 0..=SESSION_STORE_RETRY_BACKOFF.len() {
            match tokio::time::timeout(self.append_timeout, self.store.append(key, entries)).await {
                Ok(Ok(())) => return Ok(()),
                Err(_) => return Err(ClaudeSDKError::Timeout(timeout_message)),
                Ok(Err(error)) => {
                    let Some(backoff) = SESSION_STORE_RETRY_BACKOFF.get(attempt) else {
                        return Err(error);
                    };
                    tokio::time::sleep(*backoff).await;
                }
            }
        }
        unreachable!("bounded retry loop always returns")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use filetime::{FileTime, set_file_mtime};
    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    fn key() -> SessionKey {
        SessionKey::main("proj", "sess")
    }

    fn set_mtime(path: impl AsRef<Path>, seconds: i64) {
        set_file_mtime(path, FileTime::from_unix_time(seconds, 0)).unwrap();
    }

    #[test]
    fn launched_projects_root_falls_back_to_inherited_config_dir() {
        let inherited = PathBuf::from("/tmp/inherited-claude");
        assert_eq!(
            launched_projects_root_with_inherited(&HashMap::new(), Some(inherited.clone())),
            inherited.join("projects")
        );
        assert_eq!(
            launched_projects_root_with_inherited(
                &HashMap::from([(
                    "CLAUDE_CONFIG_DIR".to_owned(),
                    "/tmp/explicit-claude".to_owned()
                )]),
                Some(inherited),
            ),
            PathBuf::from("/tmp/explicit-claude/projects")
        );
    }

    #[tokio::test]
    async fn local_store_passes_core_conformance() {
        let temp = tempdir().unwrap();
        let store = LocalDirectorySessionStore::new(temp.path());
        let entries = vec![json!({"type":"a","n":1}), json!({"type":"b","n":2})];
        assert_eq!(store.load(&key()).await.unwrap(), None);
        store.append(&key(), &entries).await.unwrap();
        store.append(&key(), &[]).await.unwrap();
        store.append(&key(), &[json!({"type":"c"})]).await.unwrap();
        assert_eq!(
            store.load(&key()).await.unwrap().unwrap(),
            vec![
                json!({"type":"a","n":1}),
                json!({"type":"b","n":2}),
                json!({"type":"c"})
            ]
        );

        let sub_a = SessionKey {
            subpath: Some("subagents/a".to_owned()),
            ..key()
        };
        let sub_b = SessionKey {
            subpath: Some("subagents/b".to_owned()),
            ..key()
        };
        store.append(&sub_a, &[json!({"type":"sa"})]).await.unwrap();
        store.append(&sub_b, &[json!({"type":"sb"})]).await.unwrap();
        let mut subkeys = store.list_subkeys(&key()).await.unwrap().unwrap();
        subkeys.sort();
        assert_eq!(subkeys, vec!["subagents/a", "subagents/b"]);
        store.delete(&sub_a).await.unwrap();
        assert_eq!(store.load(&sub_a).await.unwrap(), None);
        assert!(store.load(&sub_b).await.unwrap().is_some());
        store.delete(&key()).await.unwrap();
        assert_eq!(store.load(&key()).await.unwrap(), None);
        assert_eq!(store.load(&sub_b).await.unwrap(), None);
    }

    #[tokio::test]
    async fn local_store_lists_main_sessions_but_not_subkeys() {
        let temp = tempdir().unwrap();
        let store = LocalDirectorySessionStore::new(temp.path());
        store
            .append(&SessionKey::main("P", "s1"), &[json!({"type":"a"})])
            .await
            .unwrap();
        store
            .append(&SessionKey::main("P", "s2"), &[json!({"type":"b"})])
            .await
            .unwrap();
        store
            .append(
                &SessionKey {
                    project_key: "P".into(),
                    session_id: "s3".into(),
                    subpath: Some("subagents/a".into()),
                },
                &[json!({"type":"c"})],
            )
            .await
            .unwrap();
        let mut listed = store.list_sessions("P").await.unwrap().unwrap();
        listed.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        assert_eq!(
            listed
                .iter()
                .map(|entry| entry.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["s1", "s2"]
        );
        assert!(listed.iter().all(|entry| entry.mtime > 1_000_000_000_000));
    }

    #[tokio::test]
    async fn subagent_metadata_round_trips_through_native_sidecar() {
        let temp = tempdir().unwrap();
        let store = LocalDirectorySessionStore::new(temp.path());
        let subkey = SessionKey {
            project_key: "proj".into(),
            session_id: "sess".into(),
            subpath: Some("subagents/agent-x".into()),
        };
        let entries = vec![
            json!({"type":"user","uuid":"u"}),
            json!({"type":"agent_metadata","toolUseId":"tool"}),
        ];
        store.append(&subkey, &entries).await.unwrap();
        assert_eq!(store.load(&subkey).await.unwrap().unwrap(), entries);
        assert!(
            temp.path()
                .join("proj/sess/subagents/agent-x.meta.json")
                .is_file()
        );
    }

    #[tokio::test]
    async fn canonical_miss_imports_the_newest_legacy_session() {
        let temp = tempdir().unwrap();
        let canonical = temp.path().join("canonical");
        let old_a = temp.path().join("old-a");
        let old_b = temp.path().join("old-b");
        let a = LocalDirectorySessionStore::new(&old_a);
        let b = LocalDirectorySessionStore::new(&old_b);
        a.append(&key(), &[json!({"type":"a"})]).await.unwrap();
        b.append(&key(), &[json!({"type":"a"}), json!({"type":"b"})])
            .await
            .unwrap();
        set_mtime(old_a.join("proj/sess.jsonl"), 10);
        set_mtime(old_b.join("proj/sess.jsonl"), 20);
        let store = LocalDirectorySessionStore::new(&canonical).with_legacy_roots([old_a, old_b]);
        assert_eq!(store.load(&key()).await.unwrap().unwrap().len(), 2);
        assert!(canonical.join("proj/sess.jsonl").is_file());
    }

    #[tokio::test]
    async fn newer_profile_copy_wins_per_key_and_preserves_source() {
        let temp = tempdir().unwrap();
        let canonical = temp.path().join("canonical");
        let profile = temp.path().join("profile");
        let canonical_store = LocalDirectorySessionStore::new(&canonical);
        let profile_store = LocalDirectorySessionStore::new(&profile);
        let main = key();
        let sub = SessionKey {
            subpath: Some("subagents/agent-a".to_owned()),
            ..main.clone()
        };
        canonical_store
            .append(&main, &[json!({"entry":"canonical"})])
            .await
            .unwrap();
        canonical_store
            .append(&sub, &[json!({"entry":"canonical-sub"})])
            .await
            .unwrap();
        let profile_main = vec![json!({"entry":"canonical"}), json!({"entry":"new-tail"})];
        let profile_sub = vec![
            json!({"entry":"new-sub"}),
            json!({"type":"agent_metadata","toolUseId":"new-tool"}),
        ];
        profile_store.append(&main, &profile_main).await.unwrap();
        profile_store.append(&sub, &profile_sub).await.unwrap();

        set_mtime(canonical.join("proj/sess.jsonl"), 10);
        set_mtime(canonical.join("proj/sess/subagents/agent-a.jsonl"), 10);
        set_mtime(profile.join("proj/sess.jsonl"), 20);
        set_mtime(profile.join("proj/sess/subagents/agent-a.jsonl"), 20);
        set_mtime(profile.join("proj/sess/subagents/agent-a.meta.json"), 20);

        let store =
            LocalDirectorySessionStore::new(&canonical).with_legacy_roots([profile.clone()]);
        let summary = store.reconcile_session("proj", "sess").await.unwrap();
        assert_eq!(summary.keys_promoted, 2);
        assert_eq!(store.load(&main).await.unwrap(), Some(profile_main.clone()));
        assert_eq!(store.load(&sub).await.unwrap(), Some(profile_sub.clone()));
        assert_eq!(
            LocalDirectorySessionStore::new(&profile)
                .load(&main)
                .await
                .unwrap(),
            Some(profile_main)
        );
        assert_eq!(
            LocalDirectorySessionStore::new(&profile)
                .load(&sub)
                .await
                .unwrap(),
            Some(profile_sub)
        );
    }

    #[tokio::test]
    async fn deep_equal_newer_profile_does_not_rewrite_canonical() {
        let temp = tempdir().unwrap();
        let canonical = temp.path().join("canonical");
        let profile = temp.path().join("profile");
        let entries = [json!({"same":true})];
        LocalDirectorySessionStore::new(&canonical)
            .append(&key(), &entries)
            .await
            .unwrap();
        LocalDirectorySessionStore::new(&profile)
            .append(&key(), &entries)
            .await
            .unwrap();
        let canonical_path = canonical.join("proj/sess.jsonl");
        set_mtime(&canonical_path, 10);
        set_mtime(profile.join("proj/sess.jsonl"), 20);

        let store = LocalDirectorySessionStore::new(&canonical).with_legacy_roots([profile]);
        let summary = store.reconcile_session("proj", "sess").await.unwrap();
        assert_eq!(summary.keys_promoted, 0);
        assert_eq!(summary.keys_unchanged, 1);
        assert_eq!(
            std::fs::metadata(canonical_path)
                .unwrap()
                .modified()
                .unwrap(),
            UNIX_EPOCH + Duration::from_secs(10)
        );
    }

    #[tokio::test]
    async fn exact_mtime_tie_keeps_canonical_copy() {
        let temp = tempdir().unwrap();
        let canonical = temp.path().join("canonical");
        let profile = temp.path().join("profile");
        let canonical_entries = vec![json!({"winner":"canonical"})];
        LocalDirectorySessionStore::new(&canonical)
            .append(&key(), &canonical_entries)
            .await
            .unwrap();
        LocalDirectorySessionStore::new(&profile)
            .append(&key(), &[json!({"winner":"profile"})])
            .await
            .unwrap();
        set_mtime(canonical.join("proj/sess.jsonl"), 10);
        set_mtime(profile.join("proj/sess.jsonl"), 10);

        let store = LocalDirectorySessionStore::new(&canonical).with_legacy_roots([profile]);
        let summary = store.reconcile_session("proj", "sess").await.unwrap();
        assert_eq!(summary.keys_promoted, 0);
        assert_eq!(store.load(&key()).await.unwrap(), Some(canonical_entries));
    }

    #[tokio::test]
    async fn startup_sweep_promotes_legacy_only_main_and_subagent() {
        let temp = tempdir().unwrap();
        let canonical = temp.path().join("canonical");
        let profile = temp.path().join("profile");
        let session_id = "11111111-2222-4333-8444-555555555555";
        let main = SessionKey::main("project", session_id);
        let sub = SessionKey {
            subpath: Some("subagents/agent-a".to_owned()),
            ..main.clone()
        };
        let profile_store = LocalDirectorySessionStore::new(&profile);
        profile_store
            .append(&main, &[json!({"legacy":true})])
            .await
            .unwrap();
        profile_store
            .append(
                &sub,
                &[
                    json!({"subagent":true}),
                    json!({"type":"agent_metadata","toolUseId":"legacy-tool"}),
                ],
            )
            .await
            .unwrap();

        let store = LocalDirectorySessionStore::new(&canonical).with_legacy_roots([profile]);
        let summary = store.reconcile_all().await.unwrap();
        assert_eq!(summary.sessions_scanned, 1);
        assert_eq!(summary.sessions_failed, 0);
        assert_eq!(summary.keys_promoted, 2);
        assert!(store.load(&main).await.unwrap().is_some());
        assert!(store.load(&sub).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn newer_corrupt_profile_never_destroys_canonical_copy() {
        let temp = tempdir().unwrap();
        let canonical = temp.path().join("canonical");
        let profile = temp.path().join("profile");
        let canonical_entries = vec![json!({"safe":"canonical"})];
        LocalDirectorySessionStore::new(&canonical)
            .append(&key(), &canonical_entries)
            .await
            .unwrap();
        tokio::fs::create_dir_all(profile.join("proj"))
            .await
            .unwrap();
        tokio::fs::write(profile.join("proj/sess.jsonl"), b"{broken\n")
            .await
            .unwrap();
        set_mtime(canonical.join("proj/sess.jsonl"), 10);
        set_mtime(profile.join("proj/sess.jsonl"), 20);

        let store = LocalDirectorySessionStore::new(&canonical).with_legacy_roots([profile]);
        assert!(store.reconcile_session("proj", "sess").await.is_err());
        assert_eq!(
            LocalDirectorySessionStore::new(&canonical)
                .load(&key())
                .await
                .unwrap(),
            Some(canonical_entries)
        );
    }

    #[tokio::test]
    async fn stale_corrupt_profile_does_not_poison_newer_canonical_copy() {
        let temp = tempdir().unwrap();
        let canonical = temp.path().join("canonical");
        let profile = temp.path().join("profile");
        let canonical_entries = vec![json!({"safe":"canonical"})];
        LocalDirectorySessionStore::new(&canonical)
            .append(&key(), &canonical_entries)
            .await
            .unwrap();
        tokio::fs::create_dir_all(profile.join("proj"))
            .await
            .unwrap();
        tokio::fs::write(profile.join("proj/sess.jsonl"), b"{broken\n")
            .await
            .unwrap();
        set_mtime(profile.join("proj/sess.jsonl"), 10);
        set_mtime(canonical.join("proj/sess.jsonl"), 20);

        let store = LocalDirectorySessionStore::new(&canonical).with_legacy_roots([profile]);
        assert_eq!(
            store.reconcile_session("proj", "sess").await.unwrap(),
            SessionReconcileSummary {
                sessions_scanned: 1,
                keys_unchanged: 1,
                ..SessionReconcileSummary::default()
            }
        );
        assert_eq!(store.load(&key()).await.unwrap(), Some(canonical_entries));
    }

    #[tokio::test]
    async fn resume_reconciles_newer_profile_tail_before_materializing() {
        let temp = tempdir().unwrap();
        let cwd = temp.path().join("workspace");
        tokio::fs::create_dir_all(&cwd).await.unwrap();
        let project_key = session_project_key(&cwd);
        let session_id = "11111111-2222-4333-8444-555555555555";
        let main = SessionKey::main(&project_key, session_id);
        let canonical = temp.path().join("canonical");
        let profile = temp.path().join("profile");
        LocalDirectorySessionStore::new(&canonical)
            .append(&main, &[json!({"n":1})])
            .await
            .unwrap();
        LocalDirectorySessionStore::new(&profile)
            .append(&main, &[json!({"n":1}), json!({"n":2,"tail":true})])
            .await
            .unwrap();
        set_mtime(
            canonical
                .join(&project_key)
                .join(format!("{session_id}.jsonl")),
            10,
        );
        set_mtime(
            profile
                .join(&project_key)
                .join(format!("{session_id}.jsonl")),
            20,
        );

        let concrete =
            LocalDirectorySessionStore::new(&canonical).with_legacy_roots([profile.clone()]);
        let store: Arc<dyn SessionStore> = Arc::new(concrete.clone());
        materialize_session_for_resume(&store, &cwd, session_id, &profile, Duration::from_secs(1))
            .await
            .unwrap();

        assert_eq!(concrete.load(&main).await.unwrap().unwrap().len(), 2);
        assert_eq!(
            LocalDirectorySessionStore::new(&profile)
                .load(&main)
                .await
                .unwrap()
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn legacy_profile_session_is_imported_then_materialized_into_new_profile() {
        let temp = tempdir().unwrap();
        let cwd = temp.path().join("workspace");
        let legacy_root = temp.path().join("profile-a/projects");
        let canonical = temp.path().join("canonical");
        let target = temp.path().join("profile-b/projects");
        tokio::fs::create_dir_all(&cwd).await.unwrap();
        let project_key = session_project_key(&cwd);
        let session_id = "11111111-2222-4333-8444-555555555555";
        let key = SessionKey::main(&project_key, session_id);
        LocalDirectorySessionStore::new(&legacy_root)
            .append(&key, &[json!({"type":"user","from":"profile-a"})])
            .await
            .unwrap();
        let store: Arc<dyn SessionStore> =
            Arc::new(LocalDirectorySessionStore::new(&canonical).with_legacy_roots([legacy_root]));

        assert!(
            materialize_session_for_resume(
                &store,
                &cwd,
                session_id,
                &target,
                Duration::from_secs(1),
            )
            .await
            .unwrap()
        );
        assert!(
            canonical
                .join(&project_key)
                .join(format!("{session_id}.jsonl"))
                .is_file()
        );
        assert!(
            target
                .join(&project_key)
                .join(format!("{session_id}.jsonl"))
                .is_file()
        );
    }

    #[test]
    fn mirror_path_mapping_matches_typescript_shape() {
        let root = Path::new("/tmp/config/projects");
        assert_eq!(
            session_key_from_mirror_path(Path::new("/tmp/config/projects/proj/sess.jsonl"), root),
            Some(SessionKey::main("proj", "sess"))
        );
        assert_eq!(
            session_key_from_mirror_path(
                Path::new("/tmp/config/projects/proj/sess/subagents/agent-a.jsonl"),
                root
            ),
            Some(SessionKey {
                project_key: "proj".into(),
                session_id: "sess".into(),
                subpath: Some("subagents/agent-a".into())
            })
        );
        assert_eq!(
            session_key_from_mirror_path(Path::new("/tmp/other/sess.jsonl"), root),
            None
        );
        assert_eq!(
            session_key_from_mirror_path(
                Path::new("/tmp/config/projects/proj/sess/nested.jsonl"),
                root
            ),
            None
        );
        assert_eq!(
            session_key_from_mirror_path(
                Path::new("/tmp/config/projects/proj/sess/subagents/agent-a.trace"),
                root
            ),
            Some(SessionKey {
                project_key: "proj".into(),
                session_id: "sess".into(),
                subpath: Some("subagents/agent-a.trace".into())
            })
        );
        assert_eq!(
            session_key_from_mirror_path(Path::new("/tmp/config/projects/proj/sess.trace"), root),
            None
        );
    }

    #[tokio::test]
    async fn materialization_restores_main_and_subagent_before_spawn() {
        let temp = tempdir().unwrap();
        let store_root = temp.path().join("store");
        let target = temp.path().join("profile/projects");
        let cwd = temp.path().join("workspace");
        tokio::fs::create_dir_all(&cwd).await.unwrap();
        let project_key = session_project_key(&cwd);
        let session_id = "11111111-2222-4333-8444-555555555555";
        let store = LocalDirectorySessionStore::new(&store_root);
        let main = SessionKey::main(&project_key, session_id);
        let sub = SessionKey {
            project_key,
            session_id: session_id.into(),
            subpath: Some("subagents/agent.a".into()),
        };
        store
            .append(&main, &[json!({"type":"user","uuid":"u"})])
            .await
            .unwrap();
        store
            .append(
                &sub,
                &[
                    json!({"type":"assistant","uuid":"a"}),
                    json!({"type":"agent_metadata","toolUseId":"t"}),
                ],
            )
            .await
            .unwrap();
        let store: Arc<dyn SessionStore> = Arc::new(store);
        assert!(
            materialize_session_for_resume(
                &store,
                &cwd,
                session_id,
                &target,
                Duration::from_secs(1)
            )
            .await
            .unwrap()
        );
        assert!(
            target
                .join(&main.project_key)
                .join(format!("{session_id}.jsonl"))
                .is_file()
        );
        assert!(
            target
                .join(&main.project_key)
                .join(session_id)
                .join("subagents/agent.a.jsonl")
                .is_file()
        );
        assert!(
            target
                .join(&main.project_key)
                .join(session_id)
                .join("subagents/agent.a.meta.json")
                .is_file()
        );
    }

    struct UnsafeSubkeyStore;

    #[async_trait]
    impl SessionStore for UnsafeSubkeyStore {
        async fn append(&self, _key: &SessionKey, _entries: &[Value]) -> Result<()> {
            Ok(())
        }

        async fn load(&self, key: &SessionKey) -> Result<Option<Vec<Value>>> {
            Ok(match key.subpath.as_deref() {
                None => Some(vec![json!({"type":"user"})]),
                Some("subagents/safe") => Some(vec![json!({"type":"assistant"})]),
                Some(_) => Some(vec![json!({"type":"unsafe"})]),
            })
        }

        async fn list_subkeys(&self, _key: &SessionKey) -> Result<Option<Vec<String>>> {
            Ok(Some(vec![
                "../escape".to_owned(),
                "/absolute".to_owned(),
                "subagents/safe".to_owned(),
            ]))
        }
    }

    #[tokio::test]
    async fn materialization_skips_unsafe_subkeys() {
        let temp = tempdir().unwrap();
        let cwd = temp.path().join("workspace");
        let target = temp.path().join("profile/projects");
        tokio::fs::create_dir_all(&cwd).await.unwrap();
        let session_id = "11111111-2222-4333-8444-555555555555";
        let project_key = session_project_key(&cwd);
        let store: Arc<dyn SessionStore> = Arc::new(UnsafeSubkeyStore);

        materialize_session_for_resume(&store, &cwd, session_id, &target, Duration::from_secs(1))
            .await
            .unwrap();

        let session_dir = target.join(&project_key).join(session_id);
        assert!(session_dir.join("subagents/safe.jsonl").is_file());
        assert!(!target.join(&project_key).join("escape.jsonl").exists());
        assert!(!temp.path().join("absolute.jsonl").exists());
    }

    struct FailingStore {
        calls: AtomicUsize,
        failures: usize,
    }

    #[async_trait]
    impl SessionStore for FailingStore {
        async fn append(&self, _key: &SessionKey, _entries: &[Value]) -> Result<()> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call < self.failures {
                Err(ClaudeSDKError::SessionStore("probe".into()))
            } else {
                Ok(())
            }
        }

        async fn load(&self, _key: &SessionKey) -> Result<Option<Vec<Value>>> {
            Ok(None)
        }
    }

    #[tokio::test(start_paused = true)]
    async fn mirror_retries_three_times_with_official_backoff() {
        let store = Arc::new(FailingStore {
            calls: AtomicUsize::new(0),
            failures: 2,
        });
        let batcher = TranscriptMirrorBatcher::with_limits(
            store.clone(),
            PathBuf::from("/projects"),
            0,
            usize::MAX,
            Duration::from_secs(60),
        );
        let task = tokio::spawn(async move {
            batcher
                .enqueue(
                    PathBuf::from("/projects/proj/sess.jsonl"),
                    vec![json!({"type":"a"})],
                )
                .await
        });
        tokio::time::advance(Duration::from_millis(200)).await;
        tokio::time::advance(Duration::from_millis(800)).await;
        assert!(task.await.unwrap().is_empty());
        assert_eq!(store.calls.load(Ordering::SeqCst), 3);
    }

    struct HangingStore {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl SessionStore for HangingStore {
        async fn append(&self, _key: &SessionKey, _entries: &[Value]) -> Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            std::future::pending().await
        }

        async fn load(&self, _key: &SessionKey) -> Result<Option<Vec<Value>>> {
            Ok(None)
        }
    }

    #[tokio::test(start_paused = true)]
    async fn mirror_timeout_is_not_retried() {
        let store = Arc::new(HangingStore {
            calls: AtomicUsize::new(0),
        });
        let batcher = TranscriptMirrorBatcher::with_limits(
            store.clone(),
            PathBuf::from("/projects"),
            0,
            usize::MAX,
            Duration::from_secs(2),
        );
        let task = tokio::spawn(async move {
            batcher
                .enqueue(
                    PathBuf::from("/projects/proj/sess.jsonl"),
                    vec![json!({"type":"a"})],
                )
                .await
        });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(2)).await;
        let failures = task.await.unwrap();
        assert_eq!(failures.len(), 1);
        assert!(failures[0].error.contains("timed out"));
        assert_eq!(store.calls.load(Ordering::SeqCst), 1);
    }
}
