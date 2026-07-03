use super::*;

pub(super) const GEMINI_ACP_MODEL_METHODS: &[&str] = &[
    "model/list",
    "models/list",
    "model/list_models",
    "models/list_models",
    "session/list_models",
    "session/models",
    "list_models",
];

pub(super) fn gemini_cli_preset_models(error: String) -> ProviderModelDiscovery {
    let models = vec![
        ProviderModelOption {
            id: "gemini-3-flash-preview".to_owned(),
            label: "Gemini 3 Flash Preview".to_owned(),
            description: Some("Default Gemini CLI model fallback.".to_owned()),
            recommended: true,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        },
        ProviderModelOption {
            id: "gemini-2.5-pro".to_owned(),
            label: "Gemini 2.5 Pro".to_owned(),
            description: Some("Stable pro Gemini CLI model fallback.".to_owned()),
            recommended: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        },
        ProviderModelOption {
            id: "gemini-2.5-flash".to_owned(),
            label: "Gemini 2.5 Flash".to_owned(),
            description: Some("Lower-latency Gemini CLI model fallback.".to_owned()),
            recommended: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            service_tiers: Vec::new(),
        },
    ];
    ProviderModelDiscovery {
        models,
        default_model: Some("gemini-3-flash-preview".to_owned()),
        reasoning_efforts: Vec::new(),
        service_tiers: Vec::new(),
        source: "gemini_cli_builtin",
        error: Some(error),
    }
}

pub(super) async fn fetch_gemini_acp_models(
    config: &GaryxConfig,
) -> Result<GeminiModelDiscovery, String> {
    let gemini_bin = configured_gemini_bin(config);
    let mut child = Command::new(&gemini_bin)
        .arg("--acp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("failed to start local Gemini ACP `{gemini_bin}`: {error}"))?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "local Gemini ACP stdin was unavailable".to_owned())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "local Gemini ACP stdout was unavailable".to_owned())?;
    let mut lines = BufReader::new(stdout).lines();

    let result = async {
        send_acp_request(
            &mut stdin,
            1,
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {
                    "fs": {
                        "readTextFile": false,
                        "writeTextFile": false,
                    }
                },
                "clientInfo": {
                    "name": "garyx-provider-models",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }),
        )
        .await?;
        let initialize = read_acp_response(&mut lines, 1, Duration::from_secs(10)).await?;
        if let Some(message) = acp_error_message(&initialize) {
            return Err(format!("local Gemini ACP initialize failed: {message}"));
        }

        let mut missing_method_count = 0usize;
        let mut other_errors = Vec::new();
        for (index, method) in GEMINI_ACP_MODEL_METHODS.iter().enumerate() {
            let request_id = (index as u64) + 2;
            send_acp_request(&mut stdin, request_id, method, json!({})).await?;
            let response =
                match read_acp_response(&mut lines, request_id, Duration::from_secs(2)).await {
                    Ok(response) => response,
                    Err(error) => {
                        other_errors.push(format!("{method}: {error}"));
                        continue;
                    }
                };
            if acp_error_code(&response) == Some(-32601) {
                missing_method_count += 1;
                continue;
            }
            if let Some(message) = acp_error_message(&response) {
                other_errors.push(format!("{method}: {message}"));
                continue;
            }
            if let Some(result) = response.get("result") {
                let discovery = parse_gemini_models_result(result);
                if !discovery.models.is_empty() {
                    return Ok(discovery);
                }
            }
        }

        if missing_method_count == GEMINI_ACP_MODEL_METHODS.len() {
            return Err("local Gemini ACP does not expose a model list method".to_owned());
        }
        if !other_errors.is_empty() {
            return Err(format!(
                "local Gemini ACP did not return a model list: {}",
                other_errors.join("; ")
            ));
        }
        Err("local Gemini ACP returned no model list".to_owned())
    }
    .await;

    shutdown_child(&mut child).await;
    result
}

pub(super) fn configured_gemini_bin(config: &GaryxConfig) -> String {
    for key in ["gemini", "gemini_cli"] {
        if let Some(value) = config.agents.get(key)
            && let Some(bin) = gemini_bin_from_agent_config(value)
        {
            return bin;
        }
    }
    for value in config.agents.values() {
        if let Some(bin) = gemini_bin_from_agent_config(value) {
            return bin;
        }
    }
    "gemini".to_owned()
}

pub(super) fn gemini_bin_from_agent_config(value: &Value) -> Option<String> {
    let config = serde_json::from_value::<AgentProviderConfig>(value.clone()).ok()?;
    if config.provider_type != "gemini_cli" {
        return None;
    }
    let bin = config.gemini_bin.trim();
    (!bin.is_empty()).then(|| bin.to_owned())
}

pub(super) async fn send_acp_request(
    stdin: &mut ChildStdin,
    id: u64,
    method: &str,
    params: Value,
) -> Result<(), String> {
    let payload = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    stdin
        .write_all(payload.to_string().as_bytes())
        .await
        .map_err(|error| format!("failed to write Gemini ACP request: {error}"))?;
    stdin
        .write_all(b"\n")
        .await
        .map_err(|error| format!("failed to finish Gemini ACP request: {error}"))?;
    stdin
        .flush()
        .await
        .map_err(|error| format!("failed to flush Gemini ACP request: {error}"))
}

pub(super) async fn send_jsonrpc_notification(
    stdin: &mut ChildStdin,
    method: &str,
    params: Value,
) -> Result<(), String> {
    let payload = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    });
    stdin
        .write_all(payload.to_string().as_bytes())
        .await
        .map_err(|error| format!("failed to write {method} notification: {error}"))?;
    stdin
        .write_all(b"\n")
        .await
        .map_err(|error| format!("failed to finish {method} notification: {error}"))?;
    stdin
        .flush()
        .await
        .map_err(|error| format!("failed to flush {method} notification: {error}"))
}

pub(super) async fn read_acp_response(
    lines: &mut Lines<BufReader<ChildStdout>>,
    expected_id: u64,
    duration: Duration,
) -> Result<Value, String> {
    let future = async {
        loop {
            let Some(line) = lines
                .next_line()
                .await
                .map_err(|error| format!("failed to read Gemini ACP response: {error}"))?
            else {
                return Err("local Gemini ACP closed before responding".to_owned());
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let value = serde_json::from_str::<Value>(trimmed)
                .map_err(|error| format!("invalid Gemini ACP JSON response: {error}"))?;
            if value.get("id").and_then(Value::as_u64) == Some(expected_id) {
                return Ok(value);
            }
        }
    };
    timeout(duration, future)
        .await
        .map_err(|_| format!("timed out waiting for Gemini ACP request {expected_id}"))?
}

pub(super) async fn shutdown_child(child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

pub(super) fn acp_error_code(response: &Value) -> Option<i64> {
    response
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_i64)
}

pub(super) fn acp_error_message(response: &Value) -> Option<String> {
    let error = response.get("error")?;
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown error");
    let details = error
        .get("data")
        .and_then(|data| data.get("details"))
        .and_then(Value::as_str);
    Some(match details {
        Some(details) if !details.is_empty() => format!("{message} ({details})"),
        _ => message.to_owned(),
    })
}

pub(super) fn parse_gemini_models_result(result: &Value) -> GeminiModelDiscovery {
    let candidates = result
        .get("models")
        .or_else(|| result.get("modelOptions"))
        .or_else(|| result.get("model_options"))
        .or_else(|| result.get("data").and_then(|data| data.get("models")))
        .unwrap_or(result);

    let models = candidates
        .as_array()
        .map(|values| parse_model_array(values))
        .unwrap_or_default();
    let default_model = string_field(result, &["default_model", "defaultModel", "default"]);

    GeminiModelDiscovery {
        models,
        default_model,
    }
}
