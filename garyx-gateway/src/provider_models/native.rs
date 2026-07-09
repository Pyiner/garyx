use super::*;

pub(super) fn native_model_catalog_response(
    provider_type: ProviderType,
    source: &'static str,
    models: Vec<ProviderModelOption>,
    default_model: &str,
    default_reasoning_effort: Option<String>,
) -> ProviderModelsResponse {
    let reasoning_efforts = models
        .iter()
        .find(|model| model.id == default_model)
        .or_else(|| models.first())
        .map(|model| model.supported_reasoning_efforts.clone())
        .unwrap_or_default();
    let supports_reasoning_effort_selection = provider_supports_reasoning_effort_selection(&models);
    ProviderModelsResponse {
        provider_type,
        supports_model_selection: true,
        models,
        supports_reasoning_effort_selection,
        reasoning_efforts,
        supports_service_tier_selection: false,
        service_tiers: Vec::new(),
        default_model: Some(default_model.to_owned()),
        default_reasoning_effort,
        source,
        error: None,
    }
}

pub(super) fn native_reasoning_efforts(
    default: &str,
    supported: &[&str],
) -> Vec<ProviderReasoningEffortOption> {
    supported
        .iter()
        .copied()
        .filter_map(|id| {
            native_reasoning_effort_metadata(id).map(|(id, label, description)| {
                ProviderReasoningEffortOption {
                    id: id.to_owned(),
                    label: label.to_owned(),
                    description: Some(description.to_owned()),
                    recommended: id == default,
                }
            })
        })
        .collect()
}

pub(super) fn native_reasoning_effort_metadata(
    id: &str,
) -> Option<(&'static str, &'static str, &'static str)> {
    match id {
        "off" => Some(("off", "Off", "Disable explicit thinking controls.")),
        "minimal" => Some((
            "minimal",
            "Minimal",
            "Use the smallest explicit reasoning budget.",
        )),
        "low" => Some((
            "low",
            "Low",
            "Prefer lower latency and lower reasoning budget.",
        )),
        "medium" => Some((
            "medium",
            "Medium",
            "Balanced reasoning budget for normal coding tasks.",
        )),
        "high" => Some((
            "high",
            "High",
            "Use a larger reasoning budget for harder tasks.",
        )),
        "xhigh" => Some((
            "xhigh",
            "Extra High",
            "Use the highest supported adaptive reasoning effort.",
        )),
        "max" => Some(("max", "Max", "Maximum capability with deepest reasoning.")),
        "ultra" => Some((
            "ultra",
            "Ultra",
            "Maximum reasoning with automatic task delegation.",
        )),
        _ => None,
    }
}

pub(super) fn native_model_option(
    id: &str,
    label: &str,
    description: &str,
    recommended: bool,
    default_reasoning_effort: &str,
    reasoning_efforts: Vec<ProviderReasoningEffortOption>,
) -> ProviderModelOption {
    ProviderModelOption {
        id: id.to_owned(),
        label: label.to_owned(),
        description: Some(description.to_owned()),
        recommended,
        default_reasoning_effort: Some(default_reasoning_effort.to_owned()),
        supported_reasoning_efforts: reasoning_efforts,
        service_tiers: Vec::new(),
    }
}

/// Reasoning levels supported by every model in the catalog, preserving the
/// first model's ordering. Used when no model is chosen so any selectable
/// level stays valid regardless of which model the CLI resolves.
pub(super) fn common_reasoning_efforts(
    models: &[ProviderModelOption],
) -> Vec<ProviderReasoningEffortOption> {
    let Some(first) = models.first() else {
        return Vec::new();
    };
    first
        .supported_reasoning_efforts
        .iter()
        .filter(|effort| {
            models.iter().all(|model| {
                model
                    .supported_reasoning_efforts
                    .iter()
                    .any(|candidate| candidate.id == effort.id)
            })
        })
        .cloned()
        .collect()
}

// Mirrors the Claude Code CLI model picker; effort levels map to the CLI's
// `--effort` values. Per the CLI's own gating, `xhigh` is available on
// Fable 5 and Opus 4.8 only, and `max` additionally on Sonnet 4.6.
pub(super) fn claude_code_models() -> Vec<ProviderModelOption> {
    let claude_haiku_efforts = native_reasoning_efforts("high", &["low", "medium", "high"]);
    let claude_sonnet_efforts = native_reasoning_efforts("high", &["low", "medium", "high", "max"]);
    let claude_deep_efforts =
        native_reasoning_efforts("high", &["low", "medium", "high", "xhigh", "max"]);
    vec![
        native_model_option(
            "claude-fable-5",
            "Fable 5",
            "Newest model for complex, long-running work.",
            false,
            "high",
            claude_deep_efforts.clone(),
        ),
        native_model_option(
            "claude-opus-4-8",
            "Claude Opus 4.8",
            "Most capable for the hardest and longest-running tasks.",
            false,
            "high",
            claude_deep_efforts,
        ),
        native_model_option(
            "claude-sonnet-4-6",
            "Claude Sonnet 4.6",
            "Best for everyday, complex tasks.",
            false,
            "high",
            claude_sonnet_efforts,
        ),
        native_model_option(
            "claude-haiku-4-5",
            "Claude Haiku 4.5",
            "Fastest for quick answers.",
            false,
            "high",
            claude_haiku_efforts,
        ),
    ]
}

pub(super) fn native_claude_models() -> Vec<ProviderModelOption> {
    let claude_standard_efforts =
        native_reasoning_efforts("high", &["off", "minimal", "low", "medium", "high"]);
    let claude_opus_efforts = native_reasoning_efforts(
        "xhigh",
        &["off", "minimal", "low", "medium", "high", "xhigh"],
    );
    vec![
        native_model_option(
            "claude-sonnet-4-6",
            "Claude Sonnet 4.6",
            "Default Claude model backend for Garyx's native agent loop.",
            true,
            "high",
            claude_standard_efforts.clone(),
        ),
        native_model_option(
            "claude-opus-4-7",
            "Claude Opus 4.7",
            "Highest-depth Claude model option.",
            false,
            "xhigh",
            claude_opus_efforts,
        ),
        native_model_option(
            "claude-haiku-4-5",
            "Claude Haiku 4.5",
            "Lower-latency Claude model option.",
            false,
            "high",
            claude_standard_efforts,
        ),
    ]
}

pub(super) fn native_gemini_models() -> Vec<ProviderModelOption> {
    let gemini_25_efforts =
        native_reasoning_efforts("high", &["off", "minimal", "low", "medium", "high"]);
    let gemini_3_flash_efforts =
        native_reasoning_efforts("high", &["minimal", "low", "medium", "high"]);
    let gemini_31_pro_efforts = native_reasoning_efforts("high", &["low", "high"]);
    vec![
        native_model_option(
            "gemini-3-flash-preview",
            "Gemini 3 Flash Preview",
            "Default Gemini model backend for Garyx's native agent loop.",
            true,
            "high",
            gemini_3_flash_efforts,
        ),
        native_model_option(
            "gemini-3.1-pro-preview",
            "Gemini 3.1 Pro Preview",
            "Higher-depth Gemini model option.",
            false,
            "high",
            gemini_31_pro_efforts,
        ),
        native_model_option(
            "gemini-2.5-pro",
            "Gemini 2.5 Pro",
            "Stable pro Gemini model option.",
            false,
            "high",
            gemini_25_efforts.clone(),
        ),
        native_model_option(
            "gemini-2.5-flash",
            "Gemini 2.5 Flash",
            "Lower-latency Gemini model option.",
            false,
            "high",
            gemini_25_efforts,
        ),
    ]
}

pub(super) fn antigravity_models() -> Vec<ProviderModelOption> {
    vec![
        ProviderModelOption {
            id: "Claude Opus 4.6 (Thinking)".to_owned(),
            label: "Claude Opus 4.6 (Thinking)".to_owned(),
            description: Some(
                "Default Antigravity model; uses the Claude backend through the local agy CLI."
                    .to_owned(),
            ),
            recommended: true,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        },
        ProviderModelOption {
            id: "Claude Sonnet 4.6 (Thinking)".to_owned(),
            label: "Claude Sonnet 4.6 (Thinking)".to_owned(),
            description: Some("Claude option exposed by the local agy CLI.".to_owned()),
            recommended: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        },
        ProviderModelOption {
            id: "Gemini 3.5 Flash (Low)".to_owned(),
            label: "Gemini 3.5 Flash (Low)".to_owned(),
            description: Some(
                "Antigravity Gemini option; may be subject to Google AI Platform location limits."
                    .to_owned(),
            ),
            recommended: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        },
        ProviderModelOption {
            id: "Gemini 3.5 Flash (Medium)".to_owned(),
            label: "Gemini 3.5 Flash (Medium)".to_owned(),
            description: Some(
                "Antigravity Gemini option; may be subject to Google AI Platform location limits."
                    .to_owned(),
            ),
            recommended: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        },
        ProviderModelOption {
            id: "Gemini 3.5 Flash (High)".to_owned(),
            label: "Gemini 3.5 Flash (High)".to_owned(),
            description: Some(
                "Antigravity Gemini option; may be subject to Google AI Platform location limits."
                    .to_owned(),
            ),
            recommended: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        },
        ProviderModelOption {
            id: "Gemini 3.1 Pro (Low)".to_owned(),
            label: "Gemini 3.1 Pro (Low)".to_owned(),
            description: Some(
                "Antigravity Gemini option; may be subject to Google AI Platform location limits."
                    .to_owned(),
            ),
            recommended: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        },
        ProviderModelOption {
            id: "Gemini 3.1 Pro (High)".to_owned(),
            label: "Gemini 3.1 Pro (High)".to_owned(),
            description: Some(
                "Antigravity Gemini option; may be subject to Google AI Platform location limits."
                    .to_owned(),
            ),
            recommended: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        },
        ProviderModelOption {
            id: "GPT-OSS 120B (Medium)".to_owned(),
            label: "GPT-OSS 120B (Medium)".to_owned(),
            description: Some(
                "Antigravity open model option exposed by the local agy CLI.".to_owned(),
            ),
            recommended: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        },
    ]
}
