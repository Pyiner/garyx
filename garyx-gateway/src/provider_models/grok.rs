use super::*;
use grok_agent_sdk::{GrokClient, GrokClientConfig};

pub(super) async fn fetch_grok_models(
    config: &GaryxConfig,
) -> Result<ProviderModelDiscovery, String> {
    let agent_config = configured_agent_provider_config(
        config,
        ProviderType::GrokBuild,
        &["grok", "grok_build", "grok-build"],
    )
    .unwrap_or_else(|| AgentProviderConfig {
        provider_type: ProviderType::GrokBuild.as_slug().to_owned(),
        ..Default::default()
    });
    let binary = if agent_config.grok_bin.trim().is_empty() {
        "grok".to_owned()
    } else {
        agent_config.grok_bin.clone()
    };
    let client = GrokClient::new(GrokClientConfig {
        binary,
        environment: agent_config.env,
        startup_timeout: Duration::from_secs(10),
        request_timeout: Duration::from_secs(15),
    });
    let cwd = std::env::current_dir()
        .map_err(|error| format!("failed to resolve Grok discovery cwd: {error}"))?;
    let catalog = client
        .discover_models(&cwd)
        .await
        .map_err(|error| error.to_string())?;
    let models = catalog
        .models
        .into_iter()
        .map(|model| ProviderModelOption {
            id: model.id,
            label: model.label,
            description: model.description,
            recommended: model.recommended,
            default_reasoning_effort: model.default_reasoning_effort,
            supported_reasoning_efforts: model
                .reasoning_efforts
                .into_iter()
                .map(|effort| ProviderReasoningEffortOption {
                    id: effort.id,
                    label: effort.label,
                    description: effort.description,
                    recommended: effort.recommended,
                })
                .collect(),
            service_tiers: Vec::new(),
        })
        .collect();
    Ok(ProviderModelDiscovery {
        models,
        default_model: catalog.current_model_id,
        reasoning_efforts: Vec::new(),
        service_tiers: Vec::new(),
        source: "grok_acp",
        error: None,
    })
}

pub(super) fn grok_unavailable_models(error: String) -> ProviderModelDiscovery {
    ProviderModelDiscovery {
        models: Vec::new(),
        default_model: None,
        reasoning_efforts: Vec::new(),
        service_tiers: Vec::new(),
        source: "grok_acp",
        error: Some(error),
    }
}
