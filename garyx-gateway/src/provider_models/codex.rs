use super::*;

pub(super) fn codex_builtin_models(error: Option<String>) -> CodexModelDiscovery {
    codex_discovery_from_presets(codex_builtin_model_presets(), "codex_builtin", error)
}

pub(super) fn traex_unavailable_models(error: String) -> ProviderModelDiscovery {
    ProviderModelDiscovery {
        models: Vec::new(),
        default_model: None,
        reasoning_efforts: Vec::new(),
        service_tiers: Vec::new(),
        source: "traex_app_server",
        error: Some(error),
    }
}

pub(super) fn apply_default_model_to_codex_discovery(
    mut discovery: CodexModelDiscovery,
    default_model: Option<String>,
) -> CodexModelDiscovery {
    let Some(default_model) = default_model else {
        return discovery;
    };
    let trimmed = default_model.trim();
    if trimmed.is_empty() {
        return discovery;
    }
    discovery.default_model = Some(trimmed.to_owned());
    if let Some(model) = discovery.models.iter().find(|model| model.id == trimmed) {
        discovery.reasoning_efforts = model.supported_reasoning_efforts.clone();
        discovery.service_tiers = model.service_tiers.clone();
    } else {
        // A configured model can be newer than the built-in catalog. Avoid
        // showing reasoning or tier options that belong to the previous default.
        discovery.reasoning_efforts.clear();
        discovery.service_tiers.clear();
    }
    discovery
}

pub(super) fn codex_discovery_from_presets(
    presets: Vec<CodexModelPreset>,
    source: &'static str,
    error: Option<String>,
) -> CodexModelDiscovery {
    let default_model = presets
        .iter()
        .find(|preset| preset.is_default)
        .or_else(|| presets.iter().find(|preset| preset.show_in_picker));
    let reasoning_efforts = default_model
        .map(|preset| {
            provider_reasoning_effort_options(
                &preset.supported_reasoning_efforts,
                preset.default_reasoning_effort,
            )
        })
        .unwrap_or_default();
    let service_tiers = default_model
        .map(|preset| provider_service_tier_options(&preset.service_tiers))
        .unwrap_or_default();
    let default_model = default_model.map(|preset| preset.model.clone());
    let models = presets
        .into_iter()
        .filter(|preset| preset.show_in_picker)
        .map(provider_model_option_from_codex_preset)
        .collect();

    CodexModelDiscovery {
        models,
        default_model,
        reasoning_efforts,
        service_tiers,
        source,
        error,
    }
}

pub(super) fn provider_model_option_from_codex_preset(
    preset: CodexModelPreset,
) -> ProviderModelOption {
    ProviderModelOption {
        id: preset.model.clone(),
        label: preset.display_name,
        description: (!preset.description.trim().is_empty()).then_some(preset.description),
        recommended: preset.is_default,
        default_reasoning_effort: Some(preset.default_reasoning_effort.to_string()),
        supported_reasoning_efforts: provider_reasoning_effort_options(
            &preset.supported_reasoning_efforts,
            preset.default_reasoning_effort,
        ),
        service_tiers: provider_service_tier_options(&preset.service_tiers),
    }
}

pub(super) fn provider_reasoning_effort_options(
    presets: &[CodexReasoningEffortPreset],
    default_effort: CodexReasoningEffort,
) -> Vec<ProviderReasoningEffortOption> {
    presets
        .iter()
        .map(|preset| ProviderReasoningEffortOption {
            id: preset.effort.to_string(),
            label: preset.effort.label().to_owned(),
            description: (!preset.description.trim().is_empty())
                .then(|| preset.description.clone()),
            recommended: preset.effort == default_effort,
        })
        .collect()
}

pub(super) fn provider_service_tier_options(
    tiers: &[CodexModelServiceTier],
) -> Vec<ProviderModelOption> {
    tiers
        .iter()
        .filter_map(|tier| {
            let id = tier.id.trim();
            if id.is_empty() {
                return None;
            }
            let label = if tier.name.trim().is_empty() {
                id.to_owned()
            } else {
                tier.name.trim().to_owned()
            };
            Some(ProviderModelOption {
                id: id.to_owned(),
                label,
                description: (!tier.description.trim().is_empty())
                    .then(|| tier.description.clone()),
                recommended: false,
                default_reasoning_effort: None,
                supported_reasoning_efforts: Vec::new(),
                service_tiers: Vec::new(),
            })
        })
        .collect()
}
