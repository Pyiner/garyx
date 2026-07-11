use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
}
