use super::*;

pub(super) async fn fetch_gpt_codex_models(
    config: &GaryxConfig,
) -> Result<GptModelDiscovery, String> {
    #[cfg(test)]
    if std::env::var_os("GARYX_ALLOW_REAL_CODEX_MODEL_FETCH").is_none() {
        return Err("Codex model catalog fetch disabled in tests".to_owned());
    }

    let gpt_config = configured_gpt_config(config);
    let auth =
        resolve_codex_auth(&gpt_config, &gpt_config.env).map_err(|error| error.to_string())?;
    let client_version = resolve_codex_models_client_version().await;
    let response = reqwest::Client::new()
        .get(models_endpoint(&auth.base_url, &client_version))
        .bearer_auth(&auth.bearer_token)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(
            reqwest::header::USER_AGENT,
            format!("garyx-provider-models/{}", env!("CARGO_PKG_VERSION")),
        );
    let response = if let Some(account_id) = auth.account_id.as_deref()
        && !account_id.trim().is_empty()
    {
        response.header("ChatGPT-Account-ID", account_id)
    } else {
        response
    };
    let response = timeout(Duration::from_secs(10), response.send())
        .await
        .map_err(|_| "Codex model catalog request timed out".to_owned())?
        .map_err(|error| format!("Codex model catalog request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "Codex model catalog request failed with {status}: {body}"
        ));
    }
    let catalog = response
        .json::<CodexModelsResponse>()
        .await
        .map_err(|error| format!("Codex model catalog response was invalid: {error}"))?;
    let presets = available_codex_model_presets(catalog.models, auth.uses_codex_backend());
    Ok(gpt_discovery_from_presets(presets, "codex_models", None))
}

pub(super) fn gpt_builtin_models(error: Option<String>) -> GptModelDiscovery {
    gpt_discovery_from_presets(codex_builtin_model_presets(), "codex_builtin", error)
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

pub(super) fn apply_default_model_to_gpt_discovery(
    mut discovery: GptModelDiscovery,
    default_model: Option<String>,
) -> GptModelDiscovery {
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

pub(super) fn gpt_discovery_from_presets(
    presets: Vec<CodexModelPreset>,
    source: &'static str,
    error: Option<String>,
) -> GptModelDiscovery {
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

    GptModelDiscovery {
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

pub(super) async fn resolve_codex_models_client_version() -> String {
    // Explicit override: honored verbatim (escape hatch to pin a catalog
    // generation, including one below the floor).
    if let Ok(value) = std::env::var("CODEX_CLIENT_VERSION")
        && let Some(version) = parse_codex_cli_version(&value)
    {
        return version;
    }
    // The local CLI version is only a hint for this direct catalog fetch; an
    // old or missing CLI must not hide models Garyx itself supports, so the
    // detected version is clamped up to the catalog-capability floor.
    let version = timeout(
        Duration::from_secs(2),
        Command::new("codex").arg("--version").output(),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .and_then(|output| String::from_utf8(output.stdout).ok())
    .and_then(|output| parse_codex_cli_version(&output));
    effective_codex_models_client_version(version.as_deref())
}

pub(super) fn configured_gpt_config(config: &GaryxConfig) -> GaryxNativeConfig {
    for key in ["gpt", "openai", "garyx", "garyx_native", "native"] {
        if let Some(value) = config.agents.get(key)
            && let Some(config) = gpt_config_from_agent_config(value)
        {
            return config;
        }
    }
    for value in config.agents.values() {
        if let Some(config) = gpt_config_from_agent_config(value) {
            return config;
        }
    }
    GaryxNativeConfig::default()
}

pub(super) fn gpt_config_from_agent_config(value: &Value) -> Option<GaryxNativeConfig> {
    let config = serde_json::from_value::<AgentProviderConfig>(value.clone()).ok()?;
    if !matches!(
        config.provider_type.as_str(),
        "gpt" | "openai" | "openai_gpt" | "garyx_native" | "garyx" | "native"
    ) {
        return None;
    }
    Some(GaryxNativeConfig {
        default_model: config.default_model,
        model: config.model,
        model_reasoning_effort: config.model_reasoning_effort,
        model_service_tier: config.model_service_tier,
        max_turns: config.max_turns,
        timeout_seconds: config.timeout_seconds,
        workspace_dir: config.workspace_dir,
        env: config.env,
        auth_source: config.auth_source,
        base_url: config.base_url,
        codex_home: config.codex_home,
        max_tool_iterations: config.max_tool_iterations,
        request_timeout_seconds: config.request_timeout_seconds,
        ..Default::default()
    })
}
