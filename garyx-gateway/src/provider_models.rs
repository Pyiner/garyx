use std::collections::HashSet;
use std::process::Stdio;
use std::time::Duration;

use garyx_models::config::{AgentProviderConfig, GaryxConfig};
use garyx_models::provider::ProviderType;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::timeout;

const GEMINI_ACP_MODEL_METHODS: &[&str] = &[
    "model/list",
    "models/list",
    "model/list_models",
    "models/list_models",
    "session/list_models",
    "session/models",
    "list_models",
];

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProviderModelOption {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub recommended: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProviderModelsResponse {
    pub provider_type: ProviderType,
    pub supports_model_selection: bool,
    pub models: Vec<ProviderModelOption>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    pub source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

struct GeminiModelDiscovery {
    models: Vec<ProviderModelOption>,
    default_model: Option<String>,
}

pub(crate) async fn list_provider_models(
    config: &GaryxConfig,
    provider_type: ProviderType,
) -> ProviderModelsResponse {
    match provider_type {
        ProviderType::GeminiCli => match fetch_gemini_acp_models(config).await {
            Ok(discovery) if !discovery.models.is_empty() => ProviderModelsResponse {
                provider_type,
                supports_model_selection: true,
                models: discovery.models,
                default_model: discovery.default_model,
                source: "gemini_acp",
                error: None,
            },
            Ok(_) => unsupported(
                provider_type,
                "gemini_acp",
                Some("local Gemini ACP returned no models".to_owned()),
            ),
            Err(error) => unsupported(provider_type, "gemini_acp", Some(error)),
        },
        ProviderType::ClaudeCode | ProviderType::CodexAppServer => {
            unsupported(provider_type, "provider", None)
        }
        ProviderType::AgentTeam => unsupported(provider_type, "provider", None),
    }
}

fn unsupported(
    provider_type: ProviderType,
    source: &'static str,
    error: Option<String>,
) -> ProviderModelsResponse {
    ProviderModelsResponse {
        provider_type,
        supports_model_selection: false,
        models: Vec::new(),
        default_model: None,
        source,
        error,
    }
}

async fn fetch_gemini_acp_models(config: &GaryxConfig) -> Result<GeminiModelDiscovery, String> {
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

fn configured_gemini_bin(config: &GaryxConfig) -> String {
    for key in ["gemini", "gemini_cli"] {
        if let Some(value) = config.agents.get(key) {
            if let Some(bin) = gemini_bin_from_agent_config(value) {
                return bin;
            }
        }
    }
    for value in config.agents.values() {
        if let Some(bin) = gemini_bin_from_agent_config(value) {
            return bin;
        }
    }
    "gemini".to_owned()
}

fn gemini_bin_from_agent_config(value: &Value) -> Option<String> {
    let config = serde_json::from_value::<AgentProviderConfig>(value.clone()).ok()?;
    if config.provider_type != "gemini_cli" {
        return None;
    }
    let bin = config.gemini_bin.trim();
    (!bin.is_empty()).then(|| bin.to_owned())
}

async fn send_acp_request(
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

async fn read_acp_response(
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

async fn shutdown_child(child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

fn acp_error_code(response: &Value) -> Option<i64> {
    response
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_i64)
}

fn acp_error_message(response: &Value) -> Option<String> {
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

fn parse_gemini_models_result(result: &Value) -> GeminiModelDiscovery {
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

fn parse_model_array(values: &[Value]) -> Vec<ProviderModelOption> {
    let mut seen = HashSet::new();
    let mut models = Vec::new();
    for value in values {
        let Some(option) = parse_model_option(value) else {
            continue;
        };
        if seen.insert(option.id.clone()) {
            models.push(option);
        }
    }
    models
}

fn parse_model_option(value: &Value) -> Option<ProviderModelOption> {
    if let Some(id) = value.as_str().map(str::trim).filter(|id| !id.is_empty()) {
        return Some(ProviderModelOption {
            id: id.to_owned(),
            label: model_label(id, None),
            description: None,
            recommended: false,
        });
    }

    let object = value.as_object()?;
    let id = ["id", "name", "model", "model_id", "modelId"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|id| !id.is_empty())?;
    let label = ["label", "display_name", "displayName", "title"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|label| !label.is_empty());
    let description = ["description", "summary"]
        .iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .map(str::to_owned);

    Some(ProviderModelOption {
        id: id.to_owned(),
        label: model_label(id, label),
        description,
        recommended: object
            .get("recommended")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn model_label(id: &str, label: Option<&str>) -> String {
    label
        .map(str::to_owned)
        .unwrap_or_else(|| id.strip_prefix("models/").unwrap_or(id).to_owned())
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flexible_model_payloads() {
        let discovery = parse_gemini_models_result(&json!({
            "models": [
                "gemini-2.5-pro",
                {
                    "name": "models/gemini-2.5-flash",
                    "displayName": "Gemini 2.5 Flash",
                    "description": "Fast model",
                    "recommended": true
                },
                { "id": "" },
                "gemini-2.5-pro"
            ],
            "defaultModel": "gemini-2.5-flash"
        }));

        assert_eq!(discovery.default_model.as_deref(), Some("gemini-2.5-flash"));
        assert_eq!(discovery.models.len(), 2);
        assert_eq!(discovery.models[0].id, "gemini-2.5-pro");
        assert_eq!(discovery.models[0].label, "gemini-2.5-pro");
        assert_eq!(discovery.models[1].id, "models/gemini-2.5-flash");
        assert_eq!(discovery.models[1].label, "Gemini 2.5 Flash");
        assert!(discovery.models[1].recommended);
    }

    #[test]
    fn reads_configured_gemini_binary() {
        let mut config = GaryxConfig::default();
        config.agents.insert(
            "custom-gemini".to_owned(),
            json!({
                "provider_type": "gemini_cli",
                "gemini_bin": "/tmp/gemini-acp"
            }),
        );

        assert_eq!(configured_gemini_bin(&config), "/tmp/gemini-acp");
    }
}
