use super::*;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn maps_codex_presets_with_model_specific_reasoning() {
    let discovery = gpt_builtin_models(None);

    assert_eq!(discovery.source, "codex_builtin");
    assert_eq!(discovery.default_model.as_deref(), Some("gpt-5.5"));
    assert_eq!(discovery.models[0].id, "gpt-5.5");
    assert!(discovery.models[0].recommended);
    assert_eq!(discovery.models[0].service_tiers[0].id, "priority");
    assert_eq!(discovery.models[0].service_tiers[0].label, "Fast");
    assert_eq!(
        discovery.models[0].default_reasoning_effort.as_deref(),
        Some("medium")
    );
    assert_eq!(discovery.models[0].supported_reasoning_efforts[0].id, "low");
    assert_eq!(discovery.service_tiers[0].id, "priority");
    assert_eq!(discovery.reasoning_efforts[1].id, "medium");
    assert!(discovery.reasoning_efforts[1].recommended);
}

#[test]
fn gpt_configured_unknown_default_model_does_not_reuse_previous_options() {
    let discovery = apply_default_model_to_gpt_discovery(
        gpt_builtin_models(None),
        Some("gpt-6-turbo".to_owned()),
    );

    assert_eq!(discovery.default_model.as_deref(), Some("gpt-6-turbo"));
    assert!(discovery.reasoning_efforts.is_empty());
    assert!(discovery.service_tiers.is_empty());
}

#[test]
fn configured_default_model_does_not_scan_non_default_agent_keys() {
    let mut config = GaryxConfig::default();
    config.agents.insert(
        "custom-gpt".to_owned(),
        json!({
            "provider_type": "gpt",
            "default_model": "gpt-custom-shadow"
        }),
    );

    let default_model = configured_default_model(
        &config,
        ProviderType::Gpt,
        &["gpt", "openai", "garyx", "garyx_native", "native"],
    );

    assert_eq!(default_model, None);
}

#[tokio::test]
async fn claude_code_catalog_ignores_empty_configured_provider_default_reasoning_effort() {
    let mut config = GaryxConfig::default();
    config.agents.insert(
        "claude".to_owned(),
        json!({
            "provider_type": "claude_code",
            "default_model": "claude-opus-4-8",
            "model_reasoning_effort": "  "
        }),
    );

    let response = list_provider_models(&config, ProviderType::ClaudeCode).await;
    let payload = serde_json::to_value(response).expect("provider models response");

    assert_eq!(payload["default_model"], "claude-opus-4-8");
    assert!(payload.get("default_reasoning_effort").is_none());
}

#[tokio::test]
async fn antigravity_model_catalog_defaults_to_claude_opus() {
    let response =
        list_provider_models(&GaryxConfig::default(), ProviderType::AntigravityCli).await;

    assert_eq!(response.provider_type, ProviderType::AntigravityCli);
    assert!(response.supports_model_selection);
    assert_eq!(response.source, "antigravity_builtin");
    assert_eq!(
        response.default_model.as_deref(),
        Some("Claude Opus 4.6 (Thinking)")
    );
    assert!(
        response
            .models
            .iter()
            .any(|model| model.id == "Claude Sonnet 4.6 (Thinking)")
    );
}

#[tokio::test]
async fn claude_code_model_catalog_supports_selection_and_reasoning() {
    let response = list_provider_models(&GaryxConfig::default(), ProviderType::ClaudeCode).await;

    assert_eq!(response.provider_type, ProviderType::ClaudeCode);
    assert!(response.supports_model_selection);
    assert!(response.supports_reasoning_effort_selection);
    assert_eq!(response.source, "claude_code_builtin");
    // The CLI's account default is unknowable, so no default is claimed and
    // the model-less effort list is the intersection every model supports.
    assert_eq!(response.default_model, None);
    assert_eq!(
        response
            .reasoning_efforts
            .iter()
            .map(|effort| effort.id.as_str())
            .collect::<Vec<_>>(),
        vec!["low", "medium", "high"]
    );
    assert_eq!(
        response
            .models
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "claude-fable-5",
            "claude-opus-4-8",
            "claude-sonnet-4-6",
            "claude-haiku-4-5",
        ]
    );
    for deep_model in ["claude-fable-5", "claude-opus-4-8"] {
        assert_eq!(
            response
                .models
                .iter()
                .find(|model| model.id == deep_model)
                .expect("deep model")
                .supported_reasoning_efforts
                .iter()
                .map(|effort| effort.id.as_str())
                .collect::<Vec<_>>(),
            vec!["low", "medium", "high", "xhigh", "max"]
        );
    }
    assert_eq!(
        response
            .models
            .iter()
            .find(|model| model.id == "claude-haiku-4-5")
            .expect("haiku model")
            .supported_reasoning_efforts
            .iter()
            .map(|effort| effort.id.as_str())
            .collect::<Vec<_>>(),
        vec!["low", "medium", "high"]
    );
    assert!(!response.supports_service_tier_selection);
}

#[tokio::test]
async fn claude_code_dynamic_catalog_maps_models_and_efforts() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", "Bearer test-claude-token"))
        .and(header("anthropic-version", "2023-06-01"))
        .and(header("anthropic-beta", "oauth-2025-04-20"))
        .and(header("user-agent", crate::claude_oauth::CLAUDE_USER_AGENT))
        .and(header("accept", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [
                {
                    "id": "claude-empty-effort-20260101",
                    "display_name": "Claude Empty Effort",
                    "created_at": "2026-01-01T00:00:00Z",
                    "capabilities": {
                        "effort": {
                            "supported": true,
                            "low": { "supported": false },
                            "medium": { "supported": false }
                        }
                    }
                },
                {
                    "id": "claude-opus-4-8-20260201",
                    "display_name": "  Claude Opus 4.8  ",
                    "created_at": "2026-02-01T00:00:00Z",
                    "capabilities": {
                        "effort": {
                            "supported": true,
                            "low": { "supported": true },
                            "medium": { "supported": true },
                            "high": { "supported": true },
                            "xhigh": { "supported": true },
                            "max": { "supported": true }
                        }
                    }
                },
                {
                    "id": "claude-sonnet-4-6-20260115",
                    "display_name": "",
                    "created_at": "2026-01-15T00:00:00Z",
                    "capabilities": {
                        "effort": {
                            "supported": true,
                            "low": { "supported": true },
                            "medium": { "supported": true },
                            "high": { "supported": true }
                        }
                    }
                },
                {
                    "id": "claude-haiku-4-5-20251215",
                    "display_name": "Claude Haiku 4.5",
                    "created_at": "2025-12-15T00:00:00Z",
                    "capabilities": {
                        "effort": {
                            "supported": true,
                            "low": { "supported": true },
                            "medium": { "supported": true },
                            "high": { "supported": true }
                        }
                    }
                },
                {
                    "id": "claude-fable-5-20251201",
                    "display_name": "Claude Fable 5",
                    "created_at": "2025-12-01T00:00:00Z",
                    "capabilities": {
                        "effort": {
                            "supported": true,
                            "low": { "supported": true },
                            "medium": { "supported": true },
                            "high": { "supported": true },
                            "xhigh": { "supported": true },
                            "max": { "supported": true }
                        }
                    }
                },
                {
                    "id": "claude-opus-4-7-20251101",
                    "display_name": "Claude Opus 4.7",
                    "created_at": "2025-11-01T00:00:00Z",
                    "capabilities": {
                        "effort": {
                            "supported": true,
                            "low": { "supported": true },
                            "medium": { "supported": true },
                            "high": { "supported": true },
                            "xhigh": { "supported": true }
                        }
                    }
                },
                {
                    "id": "claude-sonnet-4-5-20251001",
                    "display_name": "Claude Sonnet 4.5",
                    "created_at": "2025-10-01T00:00:00Z",
                    "capabilities": {
                        "effort": {
                            "supported": true,
                            "low": { "supported": true },
                            "medium": { "supported": true },
                            "high": { "supported": true },
                            "max": { "supported": true }
                        }
                    }
                },
                {
                    "id": "claude-no-effort",
                    "display_name": "Claude No Effort",
                    "created_at": null,
                    "capabilities": { "effort": { "supported": false } }
                },
                {
                    "id": "claude-missing-created-b",
                    "display_name": "Claude Missing Created B",
                    "created_at": null,
                    "capabilities": { "effort": { "supported": false } }
                },
                {
                    "id": "",
                    "display_name": "Skipped"
                }
            ]
        })))
        .mount(&server)
        .await;

    let discovery = fetch_claude_code_models_from_endpoint(
        &server.uri(),
        "test-claude-token",
        Duration::from_secs(5),
    )
    .await
    .expect("mock Claude model catalog");

    assert_eq!(discovery.source, "claude_code_api");
    assert_eq!(
        discovery
            .models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "claude-opus-4-8-20260201",
            "claude-sonnet-4-6-20260115",
            "claude-empty-effort-20260101",
            "claude-haiku-4-5-20251215",
            "claude-fable-5-20251201",
            "claude-opus-4-7-20251101",
            "claude-sonnet-4-5-20251001",
            "claude-no-effort",
            "claude-missing-created-b",
        ]
    );
    let opus = &discovery.models[0];
    assert_eq!(opus.label, "Claude Opus 4.8");
    assert_eq!(opus.default_reasoning_effort, None);
    assert_eq!(
        opus.supported_reasoning_efforts
            .iter()
            .map(|effort| effort.id.as_str())
            .collect::<Vec<_>>(),
        vec!["low", "medium", "high", "xhigh", "max"]
    );
    let sonnet = &discovery.models[1];
    assert_eq!(sonnet.label, "Claude Sonnet 4 6");
    assert_eq!(
        sonnet
            .supported_reasoning_efforts
            .iter()
            .map(|effort| effort.id.as_str())
            .collect::<Vec<_>>(),
        vec!["low", "medium", "high"]
    );
    assert_eq!(discovery.models.len(), 9);
    assert!(discovery.models[2].supported_reasoning_efforts.is_empty());
    assert!(discovery.models[7].supported_reasoning_efforts.is_empty());
    assert!(discovery.models[8].supported_reasoning_efforts.is_empty());
    assert!(discovery.default_model.is_none());
    assert!(discovery.reasoning_efforts.is_empty());
}

#[tokio::test]
async fn claude_code_dynamic_catalog_non_200_and_timeout_are_errors() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(503).set_body_string("upstream details stay short"))
        .mount(&server)
        .await;

    let error = fetch_claude_code_models_from_endpoint(
        &server.uri(),
        "test-claude-token",
        Duration::from_secs(5),
    )
    .await
    .expect_err("non-200 should error");
    assert!(error.contains("HTTP 503"), "unexpected error: {error}");
    assert!(!error.contains("upstream details stay short"));

    let slow_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(50))
                .set_body_json(json!({ "data": [] })),
        )
        .mount(&slow_server)
        .await;
    let error = fetch_claude_code_models_from_endpoint(
        &slow_server.uri(),
        "test-claude-token",
        Duration::from_millis(1),
    )
    .await
    .expect_err("timeout should error");
    assert!(error.contains("timed out"), "unexpected error: {error}");
}

#[test]
fn claude_code_fallback_preserves_nonempty_builtin_catalog() {
    clear_provider_model_discovery_cache_for_tests();

    let discovery = discover_or_fallback(
        "test_claude_fallback",
        Err("Claude OAuth token unavailable".to_owned()),
        |error| claude_code_builtin_models(Some(error)),
    );

    assert_eq!(discovery.source, "claude_code_builtin");
    assert!(discovery.error.as_deref().unwrap_or("").contains("OAuth"));
    assert!(!discovery.models.is_empty());
    assert!(provider_supports_reasoning_effort_selection(
        &discovery.models
    ));
}

#[test]
fn discover_or_fallback_prefers_stale_success_before_builtin_preset() {
    clear_provider_model_discovery_cache_for_tests();
    let cached = ProviderModelDiscovery {
        models: vec![ProviderModelOption {
            id: "claude-stale".to_owned(),
            label: "Claude Stale".to_owned(),
            description: None,
            recommended: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: native_reasoning_efforts("low", &["low"]),
            service_tiers: Vec::new(),
        }],
        default_model: None,
        reasoning_efforts: Vec::new(),
        service_tiers: Vec::new(),
        source: "claude_code_api",
        error: None,
    };
    let success = discover_or_fallback("test_claude_stale", Ok(cached), |error| {
        claude_code_builtin_models(Some(error))
    });
    assert_eq!(success.source, "claude_code_api");

    let stale = discover_or_fallback(
        "test_claude_stale",
        Err("network down".to_owned()),
        |error| claude_code_builtin_models(Some(error)),
    );

    assert_eq!(stale.source, "claude_code_api");
    assert_eq!(stale.models[0].id, "claude-stale");
    assert_eq!(stale.error.as_deref(), Some("network down"));
}

#[tokio::test]
async fn claude_code_catalog_uses_configured_provider_default_model() {
    let mut config = GaryxConfig::default();
    config.agents.insert(
        "claude".to_owned(),
        json!({
            "provider_type": "claude_code",
            "default_model": "claude-opus-4-8",
            "model_reasoning_effort": "max"
        }),
    );

    let response = list_provider_models(&config, ProviderType::ClaudeCode).await;

    assert_eq!(response.default_model.as_deref(), Some("claude-opus-4-8"));
}

#[tokio::test]
async fn claude_code_catalog_exposes_configured_provider_default_reasoning_effort() {
    let mut config = GaryxConfig::default();
    config.agents.insert(
        "claude".to_owned(),
        json!({
            "provider_type": "claude_code",
            "default_model": "claude-opus-4-8",
            "model_reasoning_effort": "max"
        }),
    );

    let response = list_provider_models(&config, ProviderType::ClaudeCode).await;
    let payload = serde_json::to_value(response).expect("provider models response");

    assert_eq!(payload["default_reasoning_effort"], "max");
}

#[tokio::test]
async fn codex_app_server_model_catalog_supports_selection_and_reasoning() {
    let response =
        list_provider_models(&GaryxConfig::default(), ProviderType::CodexAppServer).await;

    assert_eq!(response.provider_type, ProviderType::CodexAppServer);
    assert!(response.supports_model_selection);
    assert!(response.supports_reasoning_effort_selection);
    assert_eq!(response.source, "codex_builtin");
    assert!(response.default_model.is_none());
    assert!(!response.models.is_empty());
    assert!(!response.reasoning_efforts.is_empty());
}

#[tokio::test]
async fn codex_app_server_catalog_uses_configured_provider_default_model() {
    let mut config = GaryxConfig::default();
    config.agents.insert(
        "codex".to_owned(),
        json!({
            "provider_type": "codex_app_server",
            "default_model": "gpt-5.4"
        }),
    );

    let response = list_provider_models(&config, ProviderType::CodexAppServer).await;

    assert_eq!(response.default_model.as_deref(), Some("gpt-5.4"));
}

#[test]
fn parse_app_server_models_maps_catalog_and_reasoning() {
    let result = json!({
        "data": [
            {
                "id": "glm-5", "model": "glm-5", "displayName": "GLM-5",
                "description": "smart", "defaultReasoningEffort": "medium",
                "isDefault": false, "hidden": false,
                "supportedReasoningEfforts": [
                    {"reasoningEffort": "low", "description": "lo"},
                    {"reasoningEffort": "medium", "description": "med"}
                ]
            },
            {
                "id": "gpt-5.5", "model": "gpt-5.5", "displayName": "GPT-5.5",
                "description": "", "defaultReasoningEffort": "high",
                "isDefault": true, "hidden": false,
                "supportedReasoningEfforts": [
                    {"reasoningEffort": "high", "description": "hi"}
                ]
            },
            {
                "id": "secret", "model": "secret", "displayName": "Secret",
                "hidden": true, "defaultReasoningEffort": "low",
                "isDefault": false, "supportedReasoningEfforts": []
            }
        ]
    });

    let discovery = parse_app_server_models(&result, "traex_app_server");

    assert_eq!(discovery.source, "traex_app_server");
    // Hidden models are filtered out.
    assert_eq!(discovery.models.len(), 2);
    assert!(!discovery.models.iter().any(|model| model.id == "secret"));
    // Default comes from the `isDefault` flag.
    assert_eq!(discovery.default_model.as_deref(), Some("gpt-5.5"));
    // Top-level reasoning efforts come from the default model.
    assert_eq!(discovery.reasoning_efforts.len(), 1);
    assert_eq!(discovery.reasoning_efforts[0].id, "high");
    assert!(discovery.reasoning_efforts[0].recommended);
    // Per-model reasoning options are mapped with the default marked.
    let glm = discovery
        .models
        .iter()
        .find(|model| model.id == "glm-5")
        .expect("glm-5 present");
    assert_eq!(glm.label, "GLM-5");
    assert_eq!(glm.default_reasoning_effort.as_deref(), Some("medium"));
    assert_eq!(glm.supported_reasoning_efforts.len(), 2);
    assert!(
        glm.supported_reasoning_efforts
            .iter()
            .any(|effort| effort.id == "medium" && effort.recommended)
    );
}

#[test]
fn provider_reasoning_support_uses_any_model_effort() {
    let result = json!({
        "data": [
            {
                "id": "doubao-empty", "model": "doubao-empty", "displayName": "Doubao",
                "isDefault": true, "hidden": false, "supportedReasoningEfforts": []
            },
            {
                "id": "openrouter-3o", "model": "openrouter-3o", "displayName": null,
                "isDefault": false, "hidden": false,
                "supportedReasoningEfforts": [
                    {"reasoningEffort": "low"},
                    {"reasoningEffort": "medium"},
                    {"reasoningEffort": "high"},
                    {"reasoningEffort": "xhigh"},
                    {"reasoningEffort": "max"}
                ]
            }
        ]
    });

    let discovery = parse_app_server_models(&result, "traex_app_server");

    assert!(discovery.reasoning_efforts.is_empty());
    assert!(provider_supports_reasoning_effort_selection(
        &discovery.models
    ));
    let openrouter = discovery
        .models
        .iter()
        .find(|model| model.id == "openrouter-3o")
        .expect("openrouter model");
    assert_eq!(openrouter.label, "openrouter-3o");
    assert_eq!(
        openrouter
            .supported_reasoning_efforts
            .iter()
            .map(|effort| effort.id.as_str())
            .collect::<Vec<_>>(),
        vec!["low", "medium", "high", "xhigh", "max"]
    );
}

#[test]
fn parse_app_server_models_expands_context_window_variants() {
    let result = json!({
        "data": [
            {
                "id": "gpt-5.5", "model": "GPT-5.5", "displayName": "GPT-5.5",
                "description": "frontier", "defaultReasoningEffort": "medium",
                "isDefault": false, "hidden": false, "supportedReasoningEfforts": [],
                "businessMetadata": { "variants": {
                    "standard_key": "gpt-5.5__dev", "standard_context_window": 272000,
                    "max_key": "gpt-5.5__max", "max_context_window": 1000000
                }}
            },
            {
                "id": "glm-5", "model": "glm-5", "displayName": "GLM-5",
                "description": "", "defaultReasoningEffort": "medium",
                "isDefault": false, "hidden": false, "supportedReasoningEfforts": [],
                "businessMetadata": { "variants": {
                    "standard_key": "glm-5__dev", "standard_context_window": 200000,
                    "max_key": null, "max_context_window": null
                }}
            }
        ]
    });

    let discovery = parse_app_server_models(&result, "traex_app_server");
    let ids: Vec<&str> = discovery.models.iter().map(|m| m.id.as_str()).collect();
    // gpt-5.5 has a Max variant -> two options; glm-5 has only Standard -> one.
    assert_eq!(ids, vec!["gpt-5.5__dev", "gpt-5.5__max", "glm-5"]);
    let std = &discovery.models[0];
    assert_eq!(std.label, "GPT-5.5 / Standard");
    assert_eq!(std.description.as_deref(), Some("272K context window"));
    let max = &discovery.models[1];
    assert_eq!(max.label, "GPT-5.5 / Max");
    assert_eq!(max.description.as_deref(), Some("1M context window"));
    // Single-variant model keeps its plain display name.
    assert_eq!(discovery.models[2].label, "GLM-5");
}

// Real end-to-end discovery against the local `traex` binary. Opt-in via
// GARYX_ALLOW_REAL_APP_SERVER_MODEL_FETCH=1 (mirrors the Codex fetch guard)
// because it spawns `traex app-server`.
#[tokio::test]
async fn traex_app_server_real_discovery_lists_models() {
    if std::env::var_os("GARYX_ALLOW_REAL_APP_SERVER_MODEL_FETCH").is_none() {
        return;
    }
    let response = list_provider_models(&GaryxConfig::default(), ProviderType::Traex).await;
    assert_eq!(response.provider_type, ProviderType::Traex);
    assert_eq!(response.source, "traex_app_server");
    assert!(
        !response.models.is_empty(),
        "expected dynamically discovered traex models"
    );
    assert!(response.supports_model_selection);
    // The picker should be available when any discovered Traex model
    // advertises selectable reasoning efforts, even if the default model does
    // not.
    assert!(response.supports_reasoning_effort_selection);
}

#[tokio::test]
async fn codex_app_server_real_discovery_lists_models_with_reasoning() {
    if std::env::var_os("GARYX_ALLOW_REAL_APP_SERVER_MODEL_FETCH").is_none() {
        return;
    }
    let response =
        list_provider_models(&GaryxConfig::default(), ProviderType::CodexAppServer).await;
    assert_eq!(response.provider_type, ProviderType::CodexAppServer);
    assert_eq!(response.source, "codex_app_server");
    assert!(!response.models.is_empty());
    // Codex models advertise reasoning efforts; the picker should be on.
    assert!(response.supports_reasoning_effort_selection);
    // Service tiers are now plumbed to thread/start, so Codex advertises
    // them (e.g. Fast/priority).
    assert!(response.supports_service_tier_selection);
    assert!(!response.service_tiers.is_empty());
}

#[tokio::test]
async fn native_claude_model_catalog_supports_selection_and_reasoning() {
    let response = list_provider_models(&GaryxConfig::default(), ProviderType::ClaudeLlm).await;

    assert_eq!(response.provider_type, ProviderType::ClaudeLlm);
    assert!(response.supports_model_selection);
    assert!(response.supports_reasoning_effort_selection);
    assert_eq!(response.default_model.as_deref(), Some("claude-sonnet-4-6"));
    assert_eq!(response.models[0].id, "claude-sonnet-4-6");
    assert!(response.models[0].recommended);
    assert_eq!(
        response.models[0]
            .supported_reasoning_efforts
            .iter()
            .map(|effort| effort.id.as_str())
            .collect::<Vec<_>>(),
        vec!["off", "minimal", "low", "medium", "high"]
    );
    assert!(
        response
            .models
            .iter()
            .find(|model| model.id == "claude-opus-4-7")
            .expect("opus model")
            .supported_reasoning_efforts
            .iter()
            .any(|effort| effort.id == "xhigh")
    );
    assert_eq!(
        response
            .reasoning_efforts
            .last()
            .map(|effort| effort.id.as_str()),
        Some("high")
    );
}

#[tokio::test]
async fn native_claude_catalog_uses_configured_provider_default_model() {
    let mut config = GaryxConfig::default();
    config.agents.insert(
        "anthropic".to_owned(),
        json!({
            "provider_type": "anthropic",
            "default_model": "claude-opus-4-7"
        }),
    );

    let response = list_provider_models(&config, ProviderType::ClaudeLlm).await;

    assert_eq!(response.default_model.as_deref(), Some("claude-opus-4-7"));
    assert_eq!(
        response
            .reasoning_efforts
            .last()
            .map(|effort| effort.id.as_str()),
        Some("xhigh")
    );
}

#[tokio::test]
async fn native_gemini_model_catalog_supports_selection_and_reasoning() {
    let response = list_provider_models(&GaryxConfig::default(), ProviderType::GeminiLlm).await;

    assert_eq!(response.provider_type, ProviderType::GeminiLlm);
    assert!(response.supports_model_selection);
    assert!(response.supports_reasoning_effort_selection);
    assert_eq!(
        response.default_model.as_deref(),
        Some("gemini-3-flash-preview")
    );
    assert_eq!(response.models[0].id, "gemini-3-flash-preview");
    assert!(response.models[0].recommended);
    assert_eq!(
        response.models[0]
            .supported_reasoning_efforts
            .iter()
            .map(|effort| effort.id.as_str())
            .collect::<Vec<_>>(),
        vec!["minimal", "low", "medium", "high"]
    );
    assert_eq!(
        response
            .models
            .iter()
            .find(|model| model.id == "gemini-3.1-pro-preview")
            .expect("gemini pro preview model")
            .supported_reasoning_efforts
            .iter()
            .map(|effort| effort.id.as_str())
            .collect::<Vec<_>>(),
        vec!["low", "high"]
    );
}

#[test]
fn reads_configured_gpt_codex_home() {
    let mut config = GaryxConfig::default();
    config.agents.insert(
        "custom-gpt".to_owned(),
        json!({
            "provider_type": "gpt",
            "codex_home": "/tmp/test-codex-home",
            "base_url": "https://example.invalid/codex"
        }),
    );

    let gpt = configured_gpt_config(&config);

    assert_eq!(gpt.codex_home, "/tmp/test-codex-home");
    assert_eq!(gpt.base_url, "https://example.invalid/codex");
}
