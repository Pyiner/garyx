use super::*;

/// Resolve the app-server binary for a Codex-family provider (Codex or Traex).
/// Mirrors the binary defaulting in the bridge provider factory.
pub(super) fn app_server_model_bin(provider_type: &ProviderType) -> &'static str {
    match provider_type {
        ProviderType::Traex => "traex",
        _ => "codex",
    }
}

/// Dynamically discover models from a Codex-family app-server (Codex or Traex)
/// by spawning `<bin> app-server` and calling the `model/list` JSON-RPC method.
/// This reflects the real backend catalog (e.g. Traex exposes Doubao/GLM/Gemini/
/// GPT/etc.) instead of a hardcoded preset list.
pub(super) async fn fetch_app_server_models(
    codex_bin: &str,
    source: &'static str,
) -> Result<GptModelDiscovery, String> {
    #[cfg(test)]
    if std::env::var_os("GARYX_ALLOW_REAL_APP_SERVER_MODEL_FETCH").is_none() {
        return Err("app-server model fetch disabled in tests".to_owned());
    }

    let mut child = Command::new(codex_bin)
        // Mirror the SDK transport invocation (codex-sdk transport.rs) so we
        // pin the stdio transport instead of relying on the default, which a
        // user config or future build could change.
        .args(["app-server", "--listen", "stdio://"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("failed to start `{codex_bin} app-server`: {error}"))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "app-server stdin was unavailable".to_owned())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "app-server stdout was unavailable".to_owned())?;
    let mut lines = BufReader::new(stdout).lines();

    let result = async {
        send_acp_request(
            &mut stdin,
            1,
            "initialize",
            json!({
                "clientInfo": {
                    "name": "garyx-provider-models",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": { "experimentalApi": true },
            }),
        )
        .await?;
        let initialize = read_acp_response(&mut lines, 1, Duration::from_secs(10)).await?;
        if let Some(message) = acp_error_message(&initialize) {
            return Err(format!("app-server initialize failed: {message}"));
        }
        send_jsonrpc_notification(&mut stdin, "initialized", json!({})).await?;

        send_acp_request(&mut stdin, 2, "model/list", json!({})).await?;
        let response = read_acp_response(&mut lines, 2, Duration::from_secs(15)).await?;
        if acp_error_code(&response) == Some(-32601) {
            return Err("app-server does not expose model/list".to_owned());
        }
        if let Some(message) = acp_error_message(&response) {
            return Err(format!("app-server model/list failed: {message}"));
        }
        let result = response
            .get("result")
            .ok_or_else(|| "app-server model/list returned no result".to_owned())?;
        let discovery = parse_app_server_models(result, source);
        if discovery.models.is_empty() {
            return Err("app-server model/list returned no models".to_owned());
        }
        Ok(discovery)
    }
    .await;

    shutdown_child(&mut child).await;
    result
}

/// Parse a `model/list` response (`{ data: [Model] }`) into a model discovery.
pub(super) fn parse_app_server_models(result: &Value, source: &'static str) -> GptModelDiscovery {
    let entries = result
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut models = Vec::new();
    let mut default_model: Option<String> = None;
    let mut default_reasoning: Vec<ProviderReasoningEffortOption> = Vec::new();
    let mut first_reasoning: Vec<ProviderReasoningEffortOption> = Vec::new();
    let mut default_service_tiers: Vec<ProviderModelOption> = Vec::new();
    let mut first_service_tiers: Vec<ProviderModelOption> = Vec::new();
    let mut saw_default_model = false;

    for entry in entries {
        if entry
            .get("hidden")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        let Some(id) = entry
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let label = entry
            .get("displayName")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(id)
            .to_owned();
        let description = entry
            .get("description")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let default_effort = entry
            .get("defaultReasoningEffort")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let supported = parse_app_server_reasoning_efforts(&entry, default_effort.as_deref());
        // Service tiers are plumbed through to thread/start (serviceTier), so we
        // advertise whatever the backend reports per model.
        let service_tiers = parse_app_server_service_tiers(&entry);
        let is_default = entry
            .get("isDefault")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        if first_reasoning.is_empty() && !supported.is_empty() {
            first_reasoning = supported.clone();
        }
        if first_service_tiers.is_empty() && !service_tiers.is_empty() {
            first_service_tiers = service_tiers.clone();
        }
        if is_default {
            saw_default_model = true;
            default_reasoning = supported.clone();
            default_service_tiers = service_tiers.clone();
        }

        // Expand context-window variants the backend advertises under
        // businessMetadata.variants (e.g. TRAE's "Standard" 272K vs "Max" 1M)
        // into separate selectable models, matching the TRAE picker. Only expand
        // when a distinct Max variant exists; single-variant models stay as the
        // plain base model.
        let variants = entry
            .get("businessMetadata")
            .and_then(|meta| meta.get("variants"));
        let standard_key = variants
            .and_then(|v| v.get("standard_key"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let max_key = variants
            .and_then(|v| v.get("max_key"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());

        if let (Some(standard_key), Some(max_key)) = (standard_key, max_key) {
            let context_label = |key: &str| {
                variants
                    .and_then(|v| v.get(key))
                    .and_then(Value::as_u64)
                    .map(|ctx| format!("{} context window", format_context_window(ctx)))
            };
            if is_default {
                default_model = Some(standard_key.to_owned());
            }
            models.push(ProviderModelOption {
                id: standard_key.to_owned(),
                label: format!("{label} / Standard"),
                description: context_label("standard_context_window"),
                recommended: is_default,
                default_reasoning_effort: default_effort.clone(),
                supported_reasoning_efforts: supported.clone(),
                service_tiers: service_tiers.clone(),
            });
            models.push(ProviderModelOption {
                id: max_key.to_owned(),
                label: format!("{label} / Max"),
                description: context_label("max_context_window"),
                recommended: false,
                default_reasoning_effort: default_effort,
                supported_reasoning_efforts: supported,
                service_tiers,
            });
        } else {
            if is_default {
                default_model = Some(id.to_owned());
            }
            models.push(ProviderModelOption {
                id: id.to_owned(),
                label,
                description,
                recommended: is_default,
                default_reasoning_effort: default_effort,
                supported_reasoning_efforts: supported,
                service_tiers,
            });
        }
    }

    let reasoning_efforts = if saw_default_model {
        default_reasoning
    } else {
        first_reasoning
    };
    let service_tiers = if saw_default_model {
        default_service_tiers
    } else {
        first_service_tiers
    };

    GptModelDiscovery {
        models,
        default_model,
        reasoning_efforts,
        service_tiers,
        source,
        error: None,
    }
}

/// Render a token count as a compact context-window label (e.g. 272000 -> "272K",
/// 1000000 -> "1M").
pub(super) fn format_context_window(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        let millions = tokens as f64 / 1_000_000.0;
        if millions.fract().abs() < f64::EPSILON {
            format!("{}M", millions as u64)
        } else {
            format!("{millions:.1}M")
        }
    } else {
        format!("{}K", (tokens as f64 / 1000.0).round() as u64)
    }
}

pub(super) fn parse_app_server_service_tiers(entry: &Value) -> Vec<ProviderModelOption> {
    entry
        .get("serviceTiers")
        .and_then(Value::as_array)
        .map(|tiers| {
            tiers
                .iter()
                .filter_map(|tier| {
                    let id = tier
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())?;
                    let label = tier
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .unwrap_or(id)
                        .to_owned();
                    let description = tier
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned);
                    Some(ProviderModelOption {
                        id: id.to_owned(),
                        label,
                        description,
                        recommended: false,
                        default_reasoning_effort: None,
                        supported_reasoning_efforts: Vec::new(),
                        service_tiers: Vec::new(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn parse_app_server_reasoning_efforts(
    entry: &Value,
    default_effort: Option<&str>,
) -> Vec<ProviderReasoningEffortOption> {
    entry
        .get("supportedReasoningEfforts")
        .and_then(Value::as_array)
        .map(|options| {
            options
                .iter()
                .filter_map(|option| {
                    let effort = option
                        .get("reasoningEffort")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())?;
                    let metadata = native_reasoning_effort_metadata(effort);
                    let label = metadata
                        .map(|(_, label, _)| label.to_owned())
                        .unwrap_or_else(|| effort.to_owned());
                    let description = option
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned)
                        .or_else(|| metadata.map(|(_, _, description)| description.to_owned()));
                    Some(ProviderReasoningEffortOption {
                        id: effort.to_owned(),
                        label,
                        description,
                        recommended: Some(effort) == default_effort,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}
