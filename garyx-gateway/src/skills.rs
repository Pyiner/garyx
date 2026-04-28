use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use garyx_models::local_paths::default_skills_dir;
use serde::{Deserialize, Serialize};
use tracing::warn;

const MAX_SKILL_IMAGE_PREVIEW_BYTES: u64 = 12 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub installed: bool,
    pub enabled: bool,
    pub source_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillEntryNode {
    pub path: String,
    pub name: String,
    pub entry_type: String,
    #[serde(default)]
    pub children: Vec<SkillEntryNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillEditorState {
    pub skill: SkillInfo,
    pub entries: Vec<SkillEntryNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillFileDocument {
    pub skill: SkillInfo,
    pub path: String,
    pub content: String,
    pub media_type: String,
    pub preview_kind: String,
    pub data_base64: Option<String>,
    pub editable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkillFilePreviewKind {
    Markdown,
    Text,
    Image,
    Unsupported,
}

#[derive(Debug)]
pub enum SkillStoreError {
    Validation(String),
    AlreadyExists(String),
    NotFound(String),
    Io(io::Error),
}

impl std::fmt::Display for SkillStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(message) | Self::AlreadyExists(message) | Self::NotFound(message) => {
                f.write_str(message)
            }
            Self::Io(error) => write!(f, "{}", error),
        }
    }
}

impl std::error::Error for SkillStoreError {}

impl From<io::Error> for SkillStoreError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct SkillFrontmatter {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
}

mod sync;

pub struct SkillsService {
    user_dir: PathBuf,
    project_dir: Option<PathBuf>,
    io_lock: Mutex<()>,
}

impl SkillsService {
    pub fn new(user_dir: PathBuf, project_dir: Option<PathBuf>) -> Self {
        Self {
            user_dir,
            project_dir,
            io_lock: Mutex::new(()),
        }
    }

    pub fn default_user_dir() -> PathBuf {
        let _ = home_dir();
        default_skills_dir()
    }

    pub fn default_project_dir() -> Option<PathBuf> {
        env::current_dir()
            .ok()
            .map(|dir| dir.join(".claude").join("skills"))
    }

    pub fn default_builtin_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("builtin-skills")
    }

    pub fn list_skills(&self) -> Result<Vec<SkillInfo>, SkillStoreError> {
        let _guard = self
            .io_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let state = self.load_state_map()?;
        let mut skills = BTreeMap::new();

        if let Some(project_dir) = self.project_dir.as_deref() {
            self.scan_root(project_dir, &state, &mut skills)?;
        }
        self.scan_root(&self.user_dir, &state, &mut skills)?;

        Ok(skills.into_values().collect())
    }

    pub fn sync_external_user_skills(&self) -> Result<(), SkillStoreError> {
        let _guard = self
            .io_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let state = self.load_state_map()?;
        sync::sync_external_user_skills(&self.user_dir, &state)
    }

    pub fn seed_builtin_skills(&self) -> Result<(), SkillStoreError> {
        let _guard = self
            .io_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let builtin_dir = Self::default_builtin_dir();
        if !builtin_dir.is_dir() {
            return Ok(());
        }

        fs::create_dir_all(&self.user_dir)?;
        let mut state = self.load_state_map()?;

        for entry in fs::read_dir(&builtin_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let id = entry.file_name().to_string_lossy().to_string();
            if id.starts_with('.') || !is_valid_skill_id(&id) {
                continue;
            }
            let source_dir = entry.path();
            if !source_dir.join("SKILL.md").is_file() {
                continue;
            }

            let target_dir = self.user_dir.join(&id);
            if !target_dir.exists() {
                copy_dir_recursive(&source_dir, &target_dir)?;
            }
            state.entry(id).or_insert(true);
        }

        self.save_state_map(&state)?;
        sync::sync_external_user_skills(&self.user_dir, &state)
    }

    pub fn create_skill(
        &self,
        id: &str,
        name: &str,
        description: &str,
        body: &str,
    ) -> Result<SkillInfo, SkillStoreError> {
        let _guard = self
            .io_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        validate_skill_creation_fields(id, name, description, body)?;

        if self.find_skill_dir(id).is_some() {
            return Err(SkillStoreError::AlreadyExists(format!(
                "skill '{}' already exists",
                id
            )));
        }

        fs::create_dir_all(&self.user_dir)?;
        let skill_dir = self.user_dir.join(id);
        fs::create_dir_all(&skill_dir)?;

        let skill_markdown = render_skill_markdown(name.trim(), description.trim(), body)?;
        if let Err(error) = fs::write(skill_dir.join("SKILL.md"), skill_markdown) {
            let _ = fs::remove_dir_all(&skill_dir);
            return Err(SkillStoreError::Io(error));
        }

        let mut state = self.load_state_map()?;
        state.insert(id.to_owned(), true);
        self.save_state_map(&state)?;
        sync::sync_external_user_skills(&self.user_dir, &state)?;

        self.load_skill_from_dir(id, &skill_dir, true)
    }

    pub fn update_skill(
        &self,
        id: &str,
        name: &str,
        description: &str,
    ) -> Result<SkillInfo, SkillStoreError> {
        let _guard = self
            .io_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        validate_skill_metadata_fields(id, name, description)?;

        let skill_dir = self
            .find_skill_dir(id)
            .ok_or_else(|| SkillStoreError::NotFound(format!("skill '{}' not found", id)))?;
        let skill_md = skill_dir.join("SKILL.md");
        let current = fs::read_to_string(&skill_md)?;
        let updated = rewrite_skill_markdown(&current, id, name.trim(), description.trim())?;
        fs::write(&skill_md, updated)?;

        let state = self.load_state_map()?;
        let enabled = state.get(id).copied().unwrap_or(true);
        sync::sync_external_user_skills(&self.user_dir, &state)?;
        self.load_skill_from_dir(id, &skill_dir, enabled)
    }

    pub fn toggle_skill(&self, id: &str) -> Result<SkillInfo, SkillStoreError> {
        let _guard = self
            .io_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        validate_skill_id(id)?;

        let skill_dir = self
            .find_skill_dir(id)
            .ok_or_else(|| SkillStoreError::NotFound(format!("skill '{}' not found", id)))?;

        let mut state = self.load_state_map()?;
        let enabled = !state.get(id).copied().unwrap_or(true);
        state.insert(id.to_owned(), enabled);
        self.save_state_map(&state)?;
        sync::sync_external_user_skills(&self.user_dir, &state)?;

        self.load_skill_from_dir(id, &skill_dir, enabled)
    }

    pub fn delete_skill(&self, id: &str) -> Result<(), SkillStoreError> {
        let _guard = self
            .io_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        validate_skill_id(id)?;

        let skill_dir = self
            .find_skill_dir(id)
            .ok_or_else(|| SkillStoreError::NotFound(format!("skill '{}' not found", id)))?;

        fs::remove_dir_all(&skill_dir)?;

        let mut state = self.load_state_map()?;
        state.remove(id);
        self.save_state_map(&state)?;
        sync::sync_external_user_skills(&self.user_dir, &state)?;

        Ok(())
    }

    pub fn skill_editor_state(&self, id: &str) -> Result<SkillEditorState, SkillStoreError> {
        let _guard = self
            .io_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let state = self.load_state_map()?;
        let (skill_dir, enabled) = self.resolve_skill_dir(id, &state)?;
        self.load_editor_state(id, &skill_dir, enabled)
    }

    pub fn read_skill_file(
        &self,
        id: &str,
        relative_path: &str,
    ) -> Result<SkillFileDocument, SkillStoreError> {
        let _guard = self
            .io_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let state = self.load_state_map()?;
        let (skill_dir, enabled) = self.resolve_skill_dir(id, &state)?;
        let (absolute_path, normalized_path) = resolve_skill_entry_path(&skill_dir, relative_path)?;
        reject_symlink_path(&skill_dir, &normalized_path)?;

        if !absolute_path.is_file() {
            return Err(SkillStoreError::NotFound(format!(
                "skill file '{}' not found",
                normalized_path
            )));
        }

        let media_type = detect_skill_media_type(&normalized_path);
        let preview_kind = detect_skill_preview_kind(&normalized_path, &media_type);
        let (content, data_base64, editable) =
            read_skill_file_contents(&absolute_path, &normalized_path, preview_kind)?;
        let skill = self.load_skill_from_dir(id, &skill_dir, enabled)?;
        Ok(SkillFileDocument {
            skill,
            path: normalized_path,
            content,
            media_type,
            preview_kind: skill_preview_kind_label(preview_kind).to_owned(),
            data_base64,
            editable,
        })
    }

    pub fn write_skill_file(
        &self,
        id: &str,
        relative_path: &str,
        content: &str,
    ) -> Result<SkillFileDocument, SkillStoreError> {
        let _guard = self
            .io_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let state = self.load_state_map()?;
        let (skill_dir, enabled) = self.resolve_skill_dir(id, &state)?;
        let (absolute_path, normalized_path) = resolve_skill_entry_path(&skill_dir, relative_path)?;
        reject_symlink_path(&skill_dir, &normalized_path)?;

        if !absolute_path.is_file() {
            return Err(SkillStoreError::NotFound(format!(
                "skill file '{}' not found",
                normalized_path
            )));
        }

        let media_type = detect_skill_media_type(&normalized_path);
        let preview_kind = detect_skill_preview_kind(&normalized_path, &media_type);
        if !skill_file_is_editable(preview_kind) {
            return Err(SkillStoreError::Validation(format!(
                "skill file '{}' is not editable text",
                normalized_path
            )));
        }

        fs::write(&absolute_path, content)?;
        let skill = self.load_skill_from_dir(id, &skill_dir, enabled)?;
        sync::sync_external_user_skills(&self.user_dir, &state)?;
        Ok(SkillFileDocument {
            skill,
            path: normalized_path,
            content: content.to_owned(),
            media_type,
            preview_kind: skill_preview_kind_label(preview_kind).to_owned(),
            data_base64: None,
            editable: true,
        })
    }

    pub fn create_skill_entry(
        &self,
        id: &str,
        relative_path: &str,
        entry_type: &str,
    ) -> Result<SkillEditorState, SkillStoreError> {
        let _guard = self
            .io_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let state = self.load_state_map()?;
        let (skill_dir, enabled) = self.resolve_skill_dir(id, &state)?;
        let (absolute_path, normalized_path) = resolve_skill_entry_path(&skill_dir, relative_path)?;
        reject_symlink_path(&skill_dir, &normalized_path)?;

        if absolute_path.exists() {
            return Err(SkillStoreError::AlreadyExists(format!(
                "skill entry '{}' already exists",
                normalized_path
            )));
        }

        match entry_type {
            "directory" => {
                fs::create_dir_all(&absolute_path)?;
            }
            "file" => {
                if let Some(parent) = absolute_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&absolute_path, "")?;
            }
            _ => {
                return Err(SkillStoreError::Validation(
                    "skill entry type must be 'file' or 'directory'".to_owned(),
                ));
            }
        }

        sync::sync_external_user_skills(&self.user_dir, &state)?;
        self.load_editor_state(id, &skill_dir, enabled)
    }

    pub fn delete_skill_entry(
        &self,
        id: &str,
        relative_path: &str,
    ) -> Result<SkillEditorState, SkillStoreError> {
        let _guard = self
            .io_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let state = self.load_state_map()?;
        let (skill_dir, enabled) = self.resolve_skill_dir(id, &state)?;
        let (absolute_path, normalized_path) = resolve_skill_entry_path(&skill_dir, relative_path)?;
        reject_symlink_path(&skill_dir, &normalized_path)?;

        if normalized_path == "SKILL.md" {
            return Err(SkillStoreError::Validation(
                "SKILL.md cannot be deleted".to_owned(),
            ));
        }

        let metadata = fs::metadata(&absolute_path).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                SkillStoreError::NotFound(format!("skill entry '{}' not found", normalized_path))
            } else {
                SkillStoreError::Io(error)
            }
        })?;

        if metadata.is_dir() {
            fs::remove_dir_all(&absolute_path)?;
        } else {
            fs::remove_file(&absolute_path)?;
        }

        sync::sync_external_user_skills(&self.user_dir, &state)?;
        self.load_editor_state(id, &skill_dir, enabled)
    }

    fn scan_root(
        &self,
        root: &Path,
        state: &HashMap<String, bool>,
        skills: &mut BTreeMap<String, SkillInfo>,
    ) -> Result<(), SkillStoreError> {
        if !root.is_dir() {
            return Ok(());
        }

        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }

            let id = entry.file_name().to_string_lossy().to_string();
            if id.starts_with('.') || !is_valid_skill_id(&id) {
                continue;
            }

            let skill_dir = entry.path();
            let skill_md = skill_dir.join("SKILL.md");
            if !skill_md.is_file() {
                continue;
            }

            let enabled = state.get(&id).copied().unwrap_or(true);
            let skill = self.load_skill_from_dir(&id, &skill_dir, enabled)?;
            skills.insert(id, skill);
        }

        Ok(())
    }

    fn find_skill_dir(&self, id: &str) -> Option<PathBuf> {
        let user_dir = self.user_dir.join(id);
        if user_dir.join("SKILL.md").is_file() {
            return Some(user_dir);
        }

        let project_dir = self.project_dir.as_ref()?.join(id);
        if project_dir.join("SKILL.md").is_file() {
            return Some(project_dir);
        }

        None
    }

    fn resolve_skill_dir(
        &self,
        id: &str,
        state: &HashMap<String, bool>,
    ) -> Result<(PathBuf, bool), SkillStoreError> {
        validate_skill_id(id)?;
        let skill_dir = self
            .find_skill_dir(id)
            .ok_or_else(|| SkillStoreError::NotFound(format!("skill '{}' not found", id)))?;
        let enabled = state.get(id).copied().unwrap_or(true);
        Ok((skill_dir, enabled))
    }

    fn load_skill_from_dir(
        &self,
        id: &str,
        skill_dir: &Path,
        enabled: bool,
    ) -> Result<SkillInfo, SkillStoreError> {
        let skill_md = skill_dir.join("SKILL.md");
        let contents = fs::read_to_string(&skill_md)?;
        let frontmatter = parse_skill_frontmatter(&contents);

        Ok(SkillInfo {
            id: id.to_owned(),
            name: if frontmatter.name.trim().is_empty() {
                id.to_owned()
            } else {
                frontmatter.name.trim().to_owned()
            },
            description: frontmatter.description.trim().to_owned(),
            installed: true,
            enabled,
            source_path: skill_dir.to_string_lossy().to_string(),
        })
    }

    fn load_editor_state(
        &self,
        id: &str,
        skill_dir: &Path,
        enabled: bool,
    ) -> Result<SkillEditorState, SkillStoreError> {
        Ok(SkillEditorState {
            skill: self.load_skill_from_dir(id, skill_dir, enabled)?,
            entries: list_skill_entries(skill_dir, skill_dir)?,
        })
    }

    fn load_state_map(&self) -> Result<HashMap<String, bool>, SkillStoreError> {
        let state_path = self.state_path();
        let raw = match fs::read_to_string(&state_path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(HashMap::new()),
            Err(error) => return Err(SkillStoreError::Io(error)),
        };

        match serde_json::from_str::<HashMap<String, bool>>(&raw) {
            Ok(state) => Ok(state),
            Err(error) => {
                warn!(
                    state_path = %state_path.display(),
                    error = %error,
                    "failed to parse skills state file; continuing with defaults"
                );
                Ok(HashMap::new())
            }
        }
    }

    fn save_state_map(&self, state: &HashMap<String, bool>) -> Result<(), SkillStoreError> {
        fs::create_dir_all(&self.user_dir)?;
        let serialized = serde_json::to_string_pretty(state).map_err(io::Error::other)?;
        fs::write(self.state_path(), serialized)?;
        Ok(())
    }

    fn state_path(&self) -> PathBuf {
        self.user_dir.join(".state.json")
    }
}

pub fn sync_default_external_user_skills() -> Result<(), SkillStoreError> {
    let service = SkillsService::new(
        SkillsService::default_user_dir(),
        SkillsService::default_project_dir(),
    );
    service.seed_builtin_skills()?;
    service.sync_external_user_skills()
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<(), SkillStoreError> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &target_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &target_path)?;
        }
    }
    Ok(())
}

fn validate_skill_metadata_fields(
    id: &str,
    name: &str,
    description: &str,
) -> Result<(), SkillStoreError> {
    validate_skill_id(id)?;

    if name.trim().is_empty() {
        return Err(SkillStoreError::Validation(
            "skill name is required".to_owned(),
        ));
    }

    if description.trim().is_empty() {
        return Err(SkillStoreError::Validation(
            "skill description is required".to_owned(),
        ));
    }

    Ok(())
}

fn validate_skill_creation_fields(
    id: &str,
    name: &str,
    description: &str,
    body: &str,
) -> Result<(), SkillStoreError> {
    validate_skill_metadata_fields(id, name, description)?;

    if body.trim().is_empty() {
        return Err(SkillStoreError::Validation(
            "skill content is required".to_owned(),
        ));
    }

    Ok(())
}

fn validate_skill_id(id: &str) -> Result<(), SkillStoreError> {
    if !is_valid_skill_id(id) {
        return Err(SkillStoreError::Validation(
            "skill id must match [a-z0-9-]".to_owned(),
        ));
    }
    Ok(())
}

fn read_utf8_file(path: &Path) -> Result<String, SkillStoreError> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(error) if error.kind() == io::ErrorKind::InvalidData => {
            Err(SkillStoreError::Validation(format!(
                "skill file '{}' is not valid UTF-8 text",
                path.display()
            )))
        }
        Err(error) => Err(SkillStoreError::Io(error)),
    }
}

fn read_skill_file_contents(
    path: &Path,
    normalized_path: &str,
    preview_kind: SkillFilePreviewKind,
) -> Result<(String, Option<String>, bool), SkillStoreError> {
    match preview_kind {
        SkillFilePreviewKind::Markdown | SkillFilePreviewKind::Text => {
            let content = read_utf8_file(path)?;
            Ok((content, None, true))
        }
        SkillFilePreviewKind::Image => {
            let metadata = fs::metadata(path)?;
            if metadata.len() > MAX_SKILL_IMAGE_PREVIEW_BYTES {
                return Ok((String::new(), None, false));
            }
            let raw = fs::read(path)?;
            Ok((String::new(), Some(BASE64.encode(raw)), false))
        }
        SkillFilePreviewKind::Unsupported => {
            if is_likely_text_path(normalized_path) {
                let content = read_utf8_file(path)?;
                return Ok((content, None, true));
            }
            Ok((String::new(), None, false))
        }
    }
}

fn skill_file_is_editable(preview_kind: SkillFilePreviewKind) -> bool {
    matches!(
        preview_kind,
        SkillFilePreviewKind::Markdown | SkillFilePreviewKind::Text
    )
}

fn skill_preview_kind_label(kind: SkillFilePreviewKind) -> &'static str {
    match kind {
        SkillFilePreviewKind::Markdown => "markdown",
        SkillFilePreviewKind::Text => "text",
        SkillFilePreviewKind::Image => "image",
        SkillFilePreviewKind::Unsupported => "unsupported",
    }
}

fn detect_skill_preview_kind(path: &str, media_type: &str) -> SkillFilePreviewKind {
    let lower_name = path.trim().to_ascii_lowercase();
    if lower_name.ends_with(".md")
        || lower_name.ends_with(".markdown")
        || media_type == "text/markdown"
    {
        return SkillFilePreviewKind::Markdown;
    }
    if media_type.starts_with("image/") {
        return SkillFilePreviewKind::Image;
    }
    if media_type.starts_with("text/") || is_likely_text_path(&lower_name) {
        return SkillFilePreviewKind::Text;
    }
    SkillFilePreviewKind::Unsupported
}

fn is_likely_text_path(path: &str) -> bool {
    matches!(
        path.rsplit('.').next(),
        Some(
            "txt"
                | "json"
                | "jsonl"
                | "yaml"
                | "yml"
                | "toml"
                | "csv"
                | "tsv"
                | "log"
                | "rs"
                | "ts"
                | "tsx"
                | "js"
                | "mjs"
                | "cjs"
                | "jsx"
                | "mts"
                | "cts"
                | "css"
                | "scss"
                | "py"
                | "go"
                | "java"
                | "kt"
                | "swift"
                | "sh"
                | "sql"
                | "xml"
                | "html"
                | "htm"
        )
    )
}

fn detect_skill_media_type(path: &str) -> String {
    let lower_name = path.trim().to_ascii_lowercase();
    if lower_name.ends_with(".md") || lower_name.ends_with(".markdown") {
        return "text/markdown".to_owned();
    }
    if lower_name.ends_with(".html") || lower_name.ends_with(".htm") {
        return "text/html".to_owned();
    }
    if lower_name.ends_with(".png") {
        return "image/png".to_owned();
    }
    if lower_name.ends_with(".jpg") || lower_name.ends_with(".jpeg") {
        return "image/jpeg".to_owned();
    }
    if lower_name.ends_with(".gif") {
        return "image/gif".to_owned();
    }
    if lower_name.ends_with(".webp") {
        return "image/webp".to_owned();
    }
    if lower_name.ends_with(".svg") {
        return "image/svg+xml".to_owned();
    }
    if lower_name.ends_with(".json") || lower_name.ends_with(".jsonl") {
        return "application/json".to_owned();
    }
    if lower_name.ends_with(".yaml") || lower_name.ends_with(".yml") {
        return "application/yaml".to_owned();
    }
    if lower_name.ends_with(".xml") {
        return "application/xml".to_owned();
    }
    if lower_name.ends_with(".csv") || lower_name.ends_with(".tsv") {
        return "text/csv".to_owned();
    }
    if is_likely_text_path(&lower_name) {
        return "text/plain".to_owned();
    }
    "application/octet-stream".to_owned()
}

fn resolve_skill_entry_path(
    skill_dir: &Path,
    relative_path: &str,
) -> Result<(PathBuf, String), SkillStoreError> {
    let trimmed = relative_path.trim();
    if trimmed.is_empty() {
        return Err(SkillStoreError::Validation(
            "skill path is required".to_owned(),
        ));
    }

    let mut normalized = PathBuf::new();
    let mut segments = Vec::new();
    for component in Path::new(trimmed).components() {
        match component {
            Component::Normal(segment) => {
                let segment = segment.to_str().ok_or_else(|| {
                    SkillStoreError::Validation("skill path must be valid UTF-8".to_owned())
                })?;
                if segment.is_empty() {
                    continue;
                }
                normalized.push(segment);
                segments.push(segment.to_owned());
            }
            _ => {
                return Err(SkillStoreError::Validation(
                    "skill path must stay inside the skill directory".to_owned(),
                ));
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(SkillStoreError::Validation(
            "skill path is required".to_owned(),
        ));
    }

    Ok((skill_dir.join(&normalized), segments.join("/")))
}

fn reject_symlink_path(skill_dir: &Path, relative_path: &str) -> Result<(), SkillStoreError> {
    let mut current = skill_dir.to_path_buf();
    for segment in relative_path.split('/') {
        current.push(segment);
        if let Ok(metadata) = fs::symlink_metadata(&current) {
            if metadata.file_type().is_symlink() {
                return Err(SkillStoreError::Validation(
                    "skill symlinks are not supported in the editor".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn list_skill_entries(root: &Path, current: &Path) -> Result<Vec<SkillEntryNode>, SkillStoreError> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }

        let path = entry.path();
        let relative = path.strip_prefix(root).map_err(io::Error::other)?;
        let relative_path = relative
            .components()
            .filter_map(|component| match component {
                Component::Normal(segment) => Some(segment.to_string_lossy().to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/");
        let name = entry.file_name().to_string_lossy().to_string();

        let mut node = SkillEntryNode {
            path: relative_path,
            name,
            entry_type: if file_type.is_dir() {
                "directory".to_owned()
            } else {
                "file".to_owned()
            },
            children: Vec::new(),
        };

        if file_type.is_dir() {
            node.children = list_skill_entries(root, &path)?;
        }

        entries.push(node);
    }

    entries.sort_by(
        |left, right| match (left.entry_type.as_str(), right.entry_type.as_str()) {
            ("directory", "file") => std::cmp::Ordering::Less,
            ("file", "directory") => std::cmp::Ordering::Greater,
            _ => left
                .name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
                .then_with(|| left.name.cmp(&right.name)),
        },
    );

    Ok(entries)
}

fn is_valid_skill_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn render_skill_markdown(
    name: &str,
    description: &str,
    body: &str,
) -> Result<String, SkillStoreError> {
    render_skill_document(name, description, body)
}

fn rewrite_skill_markdown(
    contents: &str,
    id: &str,
    name: &str,
    description: &str,
) -> Result<String, SkillStoreError> {
    let previous = parse_skill_frontmatter(contents);
    let previous_name = if previous.name.trim().is_empty() {
        id
    } else {
        previous.name.trim()
    };
    let body = extract_skill_body(contents).unwrap_or(contents);
    let body = rewrite_primary_heading(body, previous_name, id, name);
    render_skill_document(name, description, &body)
}

fn render_skill_document(
    name: &str,
    description: &str,
    body: &str,
) -> Result<String, SkillStoreError> {
    let yaml = serde_yaml::to_string(&SkillFrontmatter {
        name: name.to_owned(),
        description: description.to_owned(),
    })
    .map_err(io::Error::other)?;
    let yaml = yaml.strip_prefix("---\n").unwrap_or(&yaml);
    let body = normalize_skill_body(body, name);

    Ok(format!("---\n{}---\n\n{}", yaml, body))
}

fn default_skill_body(name: &str) -> String {
    format!("# {}\n\nDescribe how this skill should behave.\n", name)
}

fn normalize_skill_body(body: &str, name: &str) -> String {
    let trimmed = body.trim_start_matches('\n').trim_end();
    if trimmed.is_empty() {
        return default_skill_body(name);
    }

    let mut normalized = trimmed.to_owned();
    normalized.push('\n');
    normalized
}

fn extract_skill_body(contents: &str) -> Option<&str> {
    let normalized = contents.strip_prefix("---\n")?;
    let end = normalized.find("\n---\n")?;
    Some(&normalized[end + "\n---\n".len()..])
}

fn rewrite_primary_heading(body: &str, previous_name: &str, id: &str, next_name: &str) -> String {
    let mut lines = body.lines().map(str::to_owned).collect::<Vec<_>>();

    for line in &mut lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let expected_name_heading = format!("# {}", previous_name);
        let expected_id_heading = format!("# {}", id);
        if trimmed == expected_name_heading || trimmed == expected_id_heading {
            let indent = line.len() - line.trim_start().len();
            let prefix = &line[..indent];
            *line = format!("{}# {}", prefix, next_name);
        }
        break;
    }

    let mut rewritten = lines.join("\n");
    if body.ends_with('\n') {
        rewritten.push('\n');
    }
    rewritten
}

fn parse_skill_frontmatter(contents: &str) -> SkillFrontmatter {
    let Some(yaml) = extract_frontmatter(contents) else {
        return SkillFrontmatter::default();
    };

    match serde_yaml::from_str::<SkillFrontmatter>(yaml) {
        Ok(frontmatter) => frontmatter,
        Err(error) => {
            warn!(error = %error, "failed to parse skill frontmatter");
            SkillFrontmatter::default()
        }
    }
}

fn extract_frontmatter(contents: &str) -> Option<&str> {
    let normalized = contents.strip_prefix("---\n")?;
    let end = normalized.find("\n---\n")?;
    Some(&normalized[..end])
}

fn home_dir() -> Option<PathBuf> {
    garyx_models::local_paths::home_dir()
}

#[cfg(test)]
mod tests;
