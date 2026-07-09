use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::provider::GaryxNativeConfig;

pub const OPENAI_RESPONSES_BASE_URL: &str = "https://api.openai.com/v1";
pub const CHATGPT_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
/// The newest Codex models-catalog shape Garyx itself understands. The backend
/// gates catalog entries by `client_version` (each model carries a
/// `minimal_client_version`), so this floor must be raised whenever Garyx
/// gains support for a newer catalog generation (e.g. `0.144.0` unlocks the
/// GPT-5.6 family and the `max`/`ultra` reasoning levels).
pub const CODEX_MODELS_CLIENT_VERSION_FLOOR: &str = "0.144.0";

#[derive(Debug, Serialize, Deserialize, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum CodexReasoningEffort {
    None,
    Minimal,
    Low,
    #[default]
    Medium,
    High,
    XHigh,
    Max,
    Ultra,
}

impl CodexReasoningEffort {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
            Self::Ultra => "ultra",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Minimal => "Minimal",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::XHigh => "Extra High",
            Self::Max => "Max",
            Self::Ultra => "Ultra",
        }
    }
}

impl fmt::Display for CodexReasoningEffort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CodexReasoningEffort {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_value(Value::String(s.to_owned()))
            .map_err(|_| format!("invalid reasoning_effort: {s}"))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct CodexReasoningEffortPreset {
    pub effort: CodexReasoningEffort,
    pub description: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct CodexModelServiceTier {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CodexModelVisibility {
    List,
    Hide,
    None,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct CodexModelInfo {
    pub slug: String,
    pub display_name: String,
    pub description: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient_reasoning_effort"
    )]
    pub default_reasoning_level: Option<CodexReasoningEffort>,
    #[serde(default, deserialize_with = "lenient_reasoning_levels")]
    pub supported_reasoning_levels: Vec<CodexReasoningEffortPreset>,
    #[serde(default)]
    pub additional_speed_tiers: Vec<String>,
    #[serde(default)]
    pub service_tiers: Vec<CodexModelServiceTier>,
    pub visibility: CodexModelVisibility,
    #[serde(default = "default_true")]
    pub supported_in_api: bool,
    pub priority: i32,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq, Default)]
pub struct CodexModelsResponse {
    pub models: Vec<CodexModelInfo>,
}

// The models catalog is a remote contract that keeps evolving (new reasoning
// levels, new per-model fields). Parse it leniently: an unknown reasoning
// level drops only that level, and an unparseable catalog entry drops only
// that entry, so the rest of the catalog stays dynamic instead of the whole
// response failing and falling back to the builtin snapshot.
impl<'de> Deserialize<'de> for CodexModelsResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // `models` is intentionally required: a response without it is a
        // structurally different payload and must fail parsing (falling back
        // to the builtin catalog) instead of masquerading as an empty catalog.
        #[derive(Deserialize)]
        struct RawCodexModelsResponse {
            models: Vec<Value>,
        }

        let raw = RawCodexModelsResponse::deserialize(deserializer)?;
        Ok(Self {
            models: raw
                .models
                .into_iter()
                .filter_map(|model| serde_json::from_value(model).ok())
                .collect(),
        })
    }
}

fn lenient_reasoning_effort<'de, D>(
    deserializer: D,
) -> Result<Option<CodexReasoningEffort>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<Value>::deserialize(deserializer)?;
    Ok(raw.and_then(|value| serde_json::from_value(value).ok()))
}

fn lenient_reasoning_levels<'de, D>(
    deserializer: D,
) -> Result<Vec<CodexReasoningEffortPreset>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Vec::<Value>::deserialize(deserializer)?;
    Ok(raw
        .into_iter()
        .filter_map(|level| serde_json::from_value(level).ok())
        .collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexModelPreset {
    pub id: String,
    pub model: String,
    pub display_name: String,
    pub description: String,
    pub default_reasoning_effort: CodexReasoningEffort,
    pub supported_reasoning_efforts: Vec<CodexReasoningEffortPreset>,
    pub additional_speed_tiers: Vec<String>,
    pub service_tiers: Vec<CodexModelServiceTier>,
    pub is_default: bool,
    pub show_in_picker: bool,
    pub supported_in_api: bool,
}

impl From<CodexModelInfo> for CodexModelPreset {
    fn from(info: CodexModelInfo) -> Self {
        Self {
            id: info.slug.clone(),
            model: info.slug,
            display_name: info.display_name,
            description: info.description.unwrap_or_default(),
            default_reasoning_effort: info
                .default_reasoning_level
                .unwrap_or(CodexReasoningEffort::None),
            supported_reasoning_efforts: info.supported_reasoning_levels,
            additional_speed_tiers: info.additional_speed_tiers,
            service_tiers: info.service_tiers,
            is_default: false,
            show_in_picker: info.visibility == CodexModelVisibility::List,
            supported_in_api: info.supported_in_api,
        }
    }
}

pub fn available_codex_model_presets(
    mut remote_models: Vec<CodexModelInfo>,
    chatgpt_mode: bool,
) -> Vec<CodexModelPreset> {
    remote_models.sort_by_key(|model| model.priority);

    let mut presets: Vec<CodexModelPreset> = remote_models
        .into_iter()
        .map(CodexModelPreset::from)
        .filter(|preset| chatgpt_mode || preset.supported_in_api)
        .collect();
    mark_default_by_picker_visibility(&mut presets);
    presets
}

fn mark_default_by_picker_visibility(models: &mut [CodexModelPreset]) {
    for preset in models.iter_mut() {
        preset.is_default = false;
    }
    if let Some(default) = models.iter_mut().find(|preset| preset.show_in_picker) {
        default.is_default = true;
    } else if let Some(default) = models.first_mut() {
        default.is_default = true;
    }
}

pub fn codex_builtin_model_presets() -> Vec<CodexModelPreset> {
    available_codex_model_presets(codex_builtin_models(), true)
}

/// Fallback snapshot of the Codex backend models catalog, used when the
/// dynamic `/models` fetch is unavailable. Keep this mirroring the backend
/// response (slugs, display names, descriptions, priorities, reasoning
/// levels) for the current `CODEX_MODELS_CLIENT_VERSION_FLOOR`.
pub fn codex_builtin_models() -> Vec<CodexModelInfo> {
    vec![
        codex_builtin_model(
            "gpt-5.5",
            "GPT-5.5",
            "Frontier model for complex coding, research, and real-world work.",
            0,
            CodexReasoningEffort::Medium,
            codex_default_reasoning_levels(),
            true,
        ),
        codex_builtin_model(
            "gpt-5.6-sol",
            "GPT-5.6-Sol",
            "Latest frontier agentic coding model.",
            1,
            CodexReasoningEffort::Low,
            codex_extended_reasoning_levels(true),
            true,
        ),
        codex_builtin_model(
            "gpt-5.6-terra",
            "GPT-5.6-Terra",
            "Balanced agentic coding model for everyday work.",
            2,
            CodexReasoningEffort::Medium,
            codex_extended_reasoning_levels(true),
            true,
        ),
        codex_builtin_model(
            "gpt-5.6-luna",
            "GPT-5.6-Luna",
            "Fast and affordable agentic coding model.",
            3,
            CodexReasoningEffort::Medium,
            codex_extended_reasoning_levels(false),
            true,
        ),
        codex_builtin_model(
            "gpt-5.4",
            "GPT-5.4",
            "Strong model for everyday coding.",
            16,
            CodexReasoningEffort::Medium,
            codex_default_reasoning_levels(),
            true,
        ),
        codex_builtin_model(
            "gpt-5.4-mini",
            "GPT-5.4-Mini",
            "Small, fast, and cost-efficient model for simpler coding tasks.",
            23,
            CodexReasoningEffort::Medium,
            codex_default_reasoning_levels(),
            false,
        ),
        codex_builtin_model(
            "gpt-5.3-codex-spark",
            "GPT-5.3-Codex-Spark",
            "Ultra-fast coding model.",
            26,
            CodexReasoningEffort::High,
            codex_default_reasoning_levels(),
            false,
        ),
    ]
}

fn codex_builtin_model(
    slug: &str,
    display_name: &str,
    description: &str,
    priority: i32,
    default_reasoning_level: CodexReasoningEffort,
    supported_reasoning_levels: Vec<CodexReasoningEffortPreset>,
    supports_fast_service_tier: bool,
) -> CodexModelInfo {
    CodexModelInfo {
        slug: slug.to_owned(),
        display_name: display_name.to_owned(),
        description: Some(description.to_owned()),
        default_reasoning_level: Some(default_reasoning_level),
        supported_reasoning_levels,
        additional_speed_tiers: if supports_fast_service_tier {
            vec!["fast".to_owned()]
        } else {
            Vec::new()
        },
        service_tiers: if supports_fast_service_tier {
            vec![codex_fast_service_tier()]
        } else {
            Vec::new()
        },
        visibility: CodexModelVisibility::List,
        supported_in_api: true,
        priority,
    }
}

pub fn codex_default_reasoning_levels() -> Vec<CodexReasoningEffortPreset> {
    vec![
        reasoning_effort_preset(
            CodexReasoningEffort::Low,
            "Fast responses with lighter reasoning",
        ),
        reasoning_effort_preset(
            CodexReasoningEffort::Medium,
            "Balances speed and reasoning depth for everyday tasks",
        ),
        reasoning_effort_preset(
            CodexReasoningEffort::High,
            "Greater reasoning depth for complex problems",
        ),
        reasoning_effort_preset(
            CodexReasoningEffort::XHigh,
            "Extra high reasoning depth for complex problems",
        ),
    ]
}

/// The default reasoning levels extended with `max` (and optionally `ultra`),
/// matching the GPT-5.6 family catalog entries.
pub fn codex_extended_reasoning_levels(include_ultra: bool) -> Vec<CodexReasoningEffortPreset> {
    let mut levels = codex_default_reasoning_levels();
    levels.push(reasoning_effort_preset(
        CodexReasoningEffort::Max,
        "Maximum reasoning depth for the hardest problems",
    ));
    if include_ultra {
        levels.push(reasoning_effort_preset(
            CodexReasoningEffort::Ultra,
            "Maximum reasoning with automatic task delegation",
        ));
    }
    levels
}

fn reasoning_effort_preset(
    effort: CodexReasoningEffort,
    description: &str,
) -> CodexReasoningEffortPreset {
    CodexReasoningEffortPreset {
        effort,
        description: description.to_owned(),
    }
}

pub fn codex_fast_service_tier() -> CodexModelServiceTier {
    CodexModelServiceTier {
        id: "priority".to_owned(),
        name: "Fast".to_owned(),
        description: "1.5x speed, increased usage".to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexAuth {
    pub bearer_token: String,
    pub base_url: String,
    pub account_id: Option<String>,
}

impl CodexAuth {
    pub fn uses_codex_backend(&self) -> bool {
        self.base_url
            .trim_end_matches('/')
            .ends_with("/backend-api/codex")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexAuthError {
    message: String,
}

impl CodexAuthError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for CodexAuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CodexAuthError {}

#[derive(Debug, Deserialize)]
struct AuthDotJson {
    #[serde(rename = "OPENAI_API_KEY")]
    openai_api_key: Option<String>,
    tokens: Option<AuthTokens>,
    agent_identity: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AuthTokens {
    access_token: String,
    account_id: Option<String>,
    #[serde(default)]
    id_token: Value,
}

pub fn resolve_codex_auth(
    config: &GaryxNativeConfig,
    env: &HashMap<String, String>,
) -> Result<CodexAuth, CodexAuthError> {
    let auth_source = config.auth_source.trim();
    if !matches!(auth_source, "" | "codex" | "api_key" | "openai_api_key") {
        return Err(CodexAuthError::new(format!(
            "unsupported Native GPT auth_source '{}'",
            config.auth_source
        )));
    }

    if let Some(api_key) =
        env_value(env, "CODEX_API_KEY").or_else(|| env_value(env, "OPENAI_API_KEY"))
    {
        return Ok(CodexAuth {
            bearer_token: api_key,
            base_url: response_base_url(OPENAI_RESPONSES_BASE_URL, config),
            account_id: None,
        });
    }
    if matches!(auth_source, "api_key" | "openai_api_key") {
        return Err(CodexAuthError::new(
            "Native GPT auth_source api_key requires CODEX_API_KEY or OPENAI_API_KEY",
        ));
    }

    let home = codex_home(config, env)
        .ok_or_else(|| CodexAuthError::new("Codex auth not found: CODEX_HOME/HOME is unset"))?;
    let auth_path = home.join("auth.json");
    let contents = std::fs::read_to_string(&auth_path).map_err(|error| {
        CodexAuthError::new(format!(
            "Codex auth not found at {}: {error}",
            auth_path.display()
        ))
    })?;
    let auth: AuthDotJson = serde_json::from_str(&contents).map_err(|error| {
        CodexAuthError::new(format!(
            "Codex auth file {} is invalid: {error}",
            auth_path.display()
        ))
    })?;
    if let Some(api_key) = auth
        .openai_api_key
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    {
        return Ok(CodexAuth {
            bearer_token: api_key,
            base_url: response_base_url(OPENAI_RESPONSES_BASE_URL, config),
            account_id: None,
        });
    }
    if let Some(tokens) = auth.tokens
        && !tokens.access_token.trim().is_empty()
    {
        let account_id = tokens.account_id.or_else(|| {
            tokens
                .id_token
                .get("chatgpt_account_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        });
        return Ok(CodexAuth {
            bearer_token: tokens.access_token,
            base_url: response_base_url(CHATGPT_CODEX_BASE_URL, config),
            account_id,
        });
    }
    if auth.agent_identity.is_some() {
        return Err(CodexAuthError::new(
            "Codex auth contains only agent_identity; Native GPT currently supports CODEX_API_KEY, OPENAI_API_KEY, auth.json OPENAI_API_KEY, or auth.json tokens.access_token",
        ));
    }
    Err(CodexAuthError::new(
        "Codex auth file does not contain a supported credential",
    ))
}

pub fn env_value(env: &HashMap<String, String>, name: &str) -> Option<String> {
    env.get(name)
        .cloned()
        .or_else(|| std::env::var(name).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn codex_home(config: &GaryxNativeConfig, env: &HashMap<String, String>) -> Option<PathBuf> {
    normalize_non_empty(Some(config.codex_home.as_str()))
        .or_else(|| env_value(env, "CODEX_HOME"))
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|home| format!("{home}/.codex"))
        })
        .map(expand_tilde)
}

fn response_base_url(default_base_url: &str, config: &GaryxNativeConfig) -> String {
    normalize_non_empty(Some(config.base_url.as_str()))
        .unwrap_or_else(|| default_base_url.to_owned())
}

fn normalize_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn expand_tilde(value: String) -> PathBuf {
    if value == "~" {
        return std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(value));
    }
    if let Some(rest) = value.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(value)
}

pub fn responses_endpoint(base_url: &str) -> String {
    append_endpoint(base_url, "responses")
}

pub fn models_endpoint(base_url: &str, client_version: &str) -> String {
    format!(
        "{}?client_version={}",
        append_endpoint(base_url, "models"),
        client_version
    )
}

fn append_endpoint(base_url: &str, endpoint: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.ends_with(&format!("/{endpoint}")) {
        trimmed.to_owned()
    } else {
        format!("{trimmed}/{endpoint}")
    }
}

pub fn parse_codex_cli_version(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .rev()
        .find(|token| semver_triple(token).is_some())
        .map(ToOwned::to_owned)
}

/// The `client_version` Garyx declares when fetching the Codex models catalog
/// directly: the detected local CLI version when it is at least the floor,
/// otherwise [`CODEX_MODELS_CLIENT_VERSION_FLOOR`]. The floor is Garyx's own
/// catalog capability, so a missing or older local Codex CLI must not hide
/// catalog entries the native GPT provider fully supports.
pub fn effective_codex_models_client_version(detected: Option<&str>) -> String {
    let floor = CODEX_MODELS_CLIENT_VERSION_FLOOR;
    let Some((detected, detected_triple)) = detected
        .and_then(|version| Some((version, semver_triple(version)?)))
    else {
        return floor.to_owned();
    };
    match semver_triple(floor) {
        Some(floor_triple) if detected_triple < floor_triple => floor.to_owned(),
        _ => detected.to_owned(),
    }
}

fn semver_triple(value: &str) -> Option<(u64, u64, u64)> {
    let mut parts = value.split('.');
    let major = parse_semver_component(parts.next()?)?;
    let minor = parse_semver_component(parts.next()?)?;
    let patch = parse_semver_component(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn parse_semver_component(value: &str) -> Option<u64> {
    if value.is_empty() || !value.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    value.parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn model(slug: &str, priority: i32, visibility: CodexModelVisibility) -> CodexModelInfo {
        CodexModelInfo {
            slug: slug.to_owned(),
            display_name: slug.to_owned(),
            description: None,
            default_reasoning_level: Some(CodexReasoningEffort::Medium),
            supported_reasoning_levels: codex_default_reasoning_levels(),
            additional_speed_tiers: Vec::new(),
            service_tiers: Vec::new(),
            visibility,
            supported_in_api: true,
            priority,
        }
    }

    #[test]
    fn reasoning_effort_matches_codex_wire_values() {
        assert_eq!(CodexReasoningEffort::None.to_string(), "none");
        assert_eq!(CodexReasoningEffort::Minimal.to_string(), "minimal");
        assert_eq!(CodexReasoningEffort::Max.to_string(), "max");
        assert_eq!(CodexReasoningEffort::Ultra.to_string(), "ultra");
        assert_eq!(
            "xhigh".parse::<CodexReasoningEffort>(),
            Ok(CodexReasoningEffort::XHigh)
        );
        assert_eq!(
            "max".parse::<CodexReasoningEffort>(),
            Ok(CodexReasoningEffort::Max)
        );
        assert_eq!(
            "ultra".parse::<CodexReasoningEffort>(),
            Ok(CodexReasoningEffort::Ultra)
        );
    }

    #[test]
    fn catalog_with_max_and_ultra_levels_deserializes() {
        let catalog: CodexModelsResponse = serde_json::from_value(json!({
            "models": [{
                "slug": "gpt-5.6-sol",
                "display_name": "GPT-5.6-Sol",
                "description": "Latest frontier agentic coding model.",
                "default_reasoning_level": "low",
                "supported_reasoning_levels": [
                    { "effort": "low", "description": "Fast" },
                    { "effort": "max", "description": "Maximum reasoning depth" },
                    { "effort": "ultra", "description": "Maximum reasoning with delegation" }
                ],
                "visibility": "list",
                "supported_in_api": true,
                "priority": 1,
                "unknown_future_field": { "ignored": true }
            }]
        }))
        .expect("catalog with max/ultra levels should deserialize");

        let levels: Vec<_> = catalog.models[0]
            .supported_reasoning_levels
            .iter()
            .map(|preset| preset.effort)
            .collect();
        assert_eq!(
            levels,
            vec![
                CodexReasoningEffort::Low,
                CodexReasoningEffort::Max,
                CodexReasoningEffort::Ultra
            ]
        );
    }

    #[test]
    fn catalog_parsing_is_lenient_about_unknown_levels_and_entries() {
        let catalog: CodexModelsResponse = serde_json::from_value(json!({
            "models": [
                {
                    "slug": "gpt-5.6-sol",
                    "display_name": "GPT-5.6-Sol",
                    "description": null,
                    "default_reasoning_level": "hyper",
                    "supported_reasoning_levels": [
                        { "effort": "low", "description": "Fast" },
                        { "effort": "hyper", "description": "A future level Garyx does not know yet" },
                        { "effort": "ultra", "description": "Delegating" }
                    ],
                    "visibility": "list",
                    "priority": 1
                },
                {
                    "slug": "gpt-6-future",
                    "display_name": "GPT-6",
                    "description": null,
                    "visibility": "a-future-visibility-variant",
                    "priority": 0
                }
            ]
        }))
        .expect("catalog should parse leniently");

        assert_eq!(catalog.models.len(), 1);
        let model = &catalog.models[0];
        assert_eq!(model.slug, "gpt-5.6-sol");
        assert_eq!(model.default_reasoning_level, None);
        assert_eq!(
            model
                .supported_reasoning_levels
                .iter()
                .map(|preset| preset.effort)
                .collect::<Vec<_>>(),
            vec![CodexReasoningEffort::Low, CodexReasoningEffort::Ultra]
        );
    }

    #[test]
    fn catalog_without_top_level_models_field_fails_parsing() {
        assert!(serde_json::from_value::<CodexModelsResponse>(json!({ "data": [] })).is_err());
        assert!(serde_json::from_value::<CodexModelsResponse>(json!({})).is_err());
        let empty: CodexModelsResponse = serde_json::from_value(json!({ "models": [] }))
            .expect("an explicit empty models list is a valid catalog");
        assert!(empty.models.is_empty());
    }

    #[test]
    fn effective_client_version_clamps_older_cli_to_floor() {
        assert_eq!(
            effective_codex_models_client_version(Some("0.142.5")),
            CODEX_MODELS_CLIENT_VERSION_FLOOR
        );
        assert_eq!(
            effective_codex_models_client_version(Some("0.150.2")),
            "0.150.2"
        );
        assert_eq!(
            effective_codex_models_client_version(Some(CODEX_MODELS_CLIENT_VERSION_FLOOR)),
            CODEX_MODELS_CLIENT_VERSION_FLOOR
        );
        assert_eq!(
            effective_codex_models_client_version(None),
            CODEX_MODELS_CLIENT_VERSION_FLOOR
        );
        assert_eq!(
            effective_codex_models_client_version(Some("not-a-version")),
            CODEX_MODELS_CLIENT_VERSION_FLOOR
        );
    }

    #[test]
    fn available_presets_sort_by_priority_and_mark_first_picker_model_default() {
        let hidden = model("hidden", 0, CodexModelVisibility::Hide);
        let visible = model("visible", 1, CodexModelVisibility::List);

        let presets = available_codex_model_presets(vec![visible, hidden], true);

        assert_eq!(presets[0].model, "hidden");
        assert!(!presets[0].is_default);
        assert_eq!(presets[1].model, "visible");
        assert!(presets[1].is_default);
    }

    #[test]
    fn builtin_catalog_uses_codex_default_model() {
        let presets = codex_builtin_model_presets();
        let default = presets
            .iter()
            .find(|preset| preset.is_default)
            .expect("builtin catalog should have a default");

        assert_eq!(default.model, "gpt-5.5");
        assert_eq!(
            default
                .supported_reasoning_efforts
                .iter()
                .map(|preset| preset.effort.to_string())
                .collect::<Vec<_>>(),
            vec!["low", "medium", "high", "xhigh"]
        );
        assert_eq!(default.service_tiers[0].id, "priority");
        assert_eq!(default.service_tiers[0].name, "Fast");
    }

    #[test]
    fn builtin_catalog_includes_gpt_5_6_family_with_max_and_ultra() {
        let presets = codex_builtin_model_presets();
        let sol = presets
            .iter()
            .find(|preset| preset.model == "gpt-5.6-sol")
            .expect("builtin catalog should include gpt-5.6-sol");
        assert_eq!(sol.default_reasoning_effort, CodexReasoningEffort::Low);
        assert_eq!(
            sol.supported_reasoning_efforts
                .iter()
                .map(|preset| preset.effort.to_string())
                .collect::<Vec<_>>(),
            vec!["low", "medium", "high", "xhigh", "max", "ultra"]
        );
        let luna = presets
            .iter()
            .find(|preset| preset.model == "gpt-5.6-luna")
            .expect("builtin catalog should include gpt-5.6-luna");
        assert_eq!(
            luna.supported_reasoning_efforts
                .iter()
                .map(|preset| preset.effort.to_string())
                .collect::<Vec<_>>(),
            vec!["low", "medium", "high", "xhigh", "max"]
        );
    }

    #[test]
    fn codex_auth_reads_chatgpt_token_from_auth_file() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("auth.json"),
            serde_json::to_string(&json!({
                "tokens": {
                    "access_token": "test-access-token",
                    "refresh_token": "test-refresh-token",
                    "account_id": "test-account",
                    "id_token": {}
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let config = GaryxNativeConfig {
            codex_home: temp.path().display().to_string(),
            ..Default::default()
        };

        let auth = resolve_codex_auth(&config, &HashMap::new()).unwrap();

        assert_eq!(auth.bearer_token, "test-access-token");
        assert_eq!(auth.base_url, CHATGPT_CODEX_BASE_URL);
        assert_eq!(auth.account_id.as_deref(), Some("test-account"));
        assert!(auth.uses_codex_backend());
    }

    #[test]
    fn codex_auth_api_key_source_requires_explicit_api_key() {
        let config = GaryxNativeConfig {
            auth_source: "api_key".to_owned(),
            ..Default::default()
        };

        let error = resolve_codex_auth(&config, &HashMap::new()).expect_err("missing key");
        assert!(
            error
                .to_string()
                .contains("requires CODEX_API_KEY or OPENAI_API_KEY")
        );

        let auth = resolve_codex_auth(
            &config,
            &HashMap::from([("OPENAI_API_KEY".to_owned(), "test-api-key".to_owned())]),
        )
        .expect("explicit key");
        assert_eq!(auth.bearer_token, "test-api-key");
        assert_eq!(auth.account_id, None);
    }

    #[test]
    fn codex_models_endpoint_includes_client_version() {
        assert_eq!(
            models_endpoint(CHATGPT_CODEX_BASE_URL, "0.124.0"),
            "https://chatgpt.com/backend-api/codex/models?client_version=0.124.0"
        );
    }

    #[test]
    fn parses_codex_cli_version() {
        assert_eq!(
            parse_codex_cli_version("codex-cli 0.130.0\n").as_deref(),
            Some("0.130.0")
        );
    }
}
