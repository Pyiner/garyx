use super::*;

#[derive(Debug, Clone)]
pub(super) struct GatewayEndpoint {
    pub(super) base_url: String,
    pub(super) auth_token: Option<String>,
}

pub(super) fn gateway_endpoint(
    config_path: &str,
) -> Result<GatewayEndpoint, Box<dyn std::error::Error>> {
    let config = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?.config;
    let public_url = config.gateway.public_url.trim();
    let base_url = if !public_url.is_empty() {
        public_url.trim_end_matches('/').to_owned()
    } else {
        let host = if config.gateway.host == "0.0.0.0" {
            "127.0.0.1".to_owned()
        } else {
            config.gateway.host
        };
        format!("http://{}:{}", host, config.gateway.port)
    };
    let auth_token = (!config.gateway.auth_token.trim().is_empty())
        .then(|| config.gateway.auth_token.trim().to_owned());
    Ok(GatewayEndpoint {
        base_url,
        auth_token,
    })
}

#[cfg(test)]
fn gateway_base_url(config_path: &str) -> Result<String, Box<dyn std::error::Error>> {
    Ok(gateway_endpoint(config_path)?.base_url)
}

pub(super) fn gateway_request(
    mut builder: reqwest::RequestBuilder,
    gateway: &GatewayEndpoint,
) -> reqwest::RequestBuilder {
    if let Some(token) = gateway.auth_token.as_deref() {
        builder = builder.bearer_auth(token);
    }
    builder
}

pub(super) async fn fetch_gateway_json(
    gateway: &GatewayEndpoint,
    path_and_query: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path_and_query);
    let response = gateway_request(reqwest::Client::new().get(&url), gateway)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(format!("gateway request failed: {status} {body}").into());
    }
    Ok(serde_json::from_str(&body)?)
}

pub(super) fn print_pretty_json(value: &Value) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

pub(super) async fn post_gateway_json(
    gateway: &GatewayEndpoint,
    path: &str,
    payload: &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let response = gateway_request(reqwest::Client::new().post(&url), gateway)
        .json(payload)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(format!("gateway request failed: {status} {body}").into());
    }
    Ok(serde_json::from_str(&body)?)
}

pub(super) async fn post_gateway_json_with_timeout(
    gateway: &GatewayEndpoint,
    path: &str,
    payload: &Value,
    timeout_secs: u64,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let response = gateway_request(reqwest::Client::new().post(&url), gateway)
        .json(payload)
        .timeout(std::time::Duration::from_secs(timeout_secs.max(1)))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(format!("gateway request failed: {status} {body}").into());
    }
    Ok(serde_json::from_str(&body)?)
}

pub(super) async fn post_gateway_json_as_cli_actor(
    gateway: &GatewayEndpoint,
    path: &str,
    payload: &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let response = gateway_request(reqwest::Client::new().post(&url), gateway)
        .header("X-Garyx-Actor", cli_actor_header_value())
        .json(payload)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(format!("gateway request failed: {status} {body}").into());
    }
    Ok(serde_json::from_str(&body)?)
}

pub(super) async fn patch_gateway_json(
    gateway: &GatewayEndpoint,
    path: &str,
    payload: &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let response = gateway_request(reqwest::Client::new().patch(&url), gateway)
        .json(payload)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(format!("gateway request failed: {status} {body}").into());
    }
    Ok(serde_json::from_str(&body)?)
}

pub(super) async fn patch_gateway_json_as_cli_actor(
    gateway: &GatewayEndpoint,
    path: &str,
    payload: &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let response = gateway_request(reqwest::Client::new().patch(&url), gateway)
        .header("X-Garyx-Actor", cli_actor_header_value())
        .json(payload)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(format!("gateway request failed: {status} {body}").into());
    }
    Ok(serde_json::from_str(&body)?)
}

pub(super) async fn put_gateway_json(
    gateway: &GatewayEndpoint,
    path: &str,
    payload: &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let response = gateway_request(reqwest::Client::new().put(&url), gateway)
        .json(payload)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(format!("gateway request failed: {status} {body}").into());
    }
    Ok(serde_json::from_str(&body)?)
}

pub(super) async fn delete_gateway_json_as_cli_actor(
    gateway: &GatewayEndpoint,
    path: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let response = gateway_request(reqwest::Client::new().delete(&url), gateway)
        .header("X-Garyx-Actor", cli_actor_header_value())
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(format!("gateway request failed: {status} {body}").into());
    }
    Ok(serde_json::from_str(&body)?)
}

pub(super) async fn delete_gateway_json(
    gateway: &GatewayEndpoint,
    path: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{}{}", gateway.base_url, path);
    let response = gateway_request(reqwest::Client::new().delete(&url), gateway)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(format!("gateway request failed: {status} {body}").into());
    }
    if body.trim().is_empty() {
        return Ok(json!({}));
    }
    Ok(serde_json::from_str(&body)?)
}

pub(super) fn principal_payload(principal: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let principal = principal.trim();
    if principal.is_empty() {
        return Err("principal cannot be empty".into());
    }
    if let Some(user_id) = principal.strip_prefix("human:") {
        let user_id = user_id.trim();
        if user_id.is_empty() {
            return Err("human principal cannot be empty".into());
        }
        return Ok(json!({ "kind": "human", "user_id": user_id }));
    }
    if let Some(agent_id) = principal.strip_prefix("agent:") {
        let agent_id = agent_id.trim();
        if agent_id.is_empty() {
            return Err("agent principal cannot be empty".into());
        }
        return Ok(json!({ "kind": "agent", "agent_id": agent_id }));
    }
    Ok(json!({ "kind": "agent", "agent_id": principal }))
}

fn cli_actor_payload() -> Value {
    if let Some(actor) = env_nonempty("GARYX_ACTOR") {
        return principal_payload(&actor)
            .unwrap_or_else(|_| json!({ "kind": "human", "user_id": cli_actor_user_id() }));
    }
    if let Some(agent_id) = env_nonempty("GARYX_AGENT_ID") {
        return json!({ "kind": "agent", "agent_id": agent_id });
    }
    json!({ "kind": "human", "user_id": cli_actor_user_id() })
}

fn cli_actor_user_id() -> String {
    env_nonempty("GARYX_USER").unwrap_or_else(|| "owner".to_owned())
}

fn cli_actor_header_value() -> String {
    format_principal(&cli_actor_payload())
}

pub(super) fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub(super) fn format_principal(value: &Value) -> String {
    if value.is_null() {
        return "(unassigned)".to_owned();
    }
    match value
        .get("kind")
        .or_else(|| value.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("-")
    {
        "human" => format!(
            "human:{}",
            value.get("user_id").and_then(Value::as_str).unwrap_or("-")
        ),
        "agent" => format!(
            "agent:{}",
            value.get("agent_id").and_then(Value::as_str).unwrap_or("-")
        ),
        other => other.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::*;
    use tempfile::tempdir;

    #[test]
    fn cli_actor_header_uses_agent_identity_from_env() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let _actor = ScopedEnvVar::remove("GARYX_ACTOR");
        let _agent = ScopedEnvVar::set_string("GARYX_AGENT_ID", "codex");
        let _user = ScopedEnvVar::set_string("GARYX_USER", "owner");

        assert_eq!(cli_actor_header_value(), "agent:codex");
        assert_eq!(
            cli_actor_payload(),
            json!({ "kind": "agent", "agent_id": "codex" })
        );
    }

    #[test]
    fn cli_actor_header_prefers_explicit_actor_env() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let _actor = ScopedEnvVar::set_string("GARYX_ACTOR", "human:alice");
        let _agent = ScopedEnvVar::set_string("GARYX_AGENT_ID", "codex");
        let _user = ScopedEnvVar::set_string("GARYX_USER", "owner");

        assert_eq!(cli_actor_header_value(), "human:alice");
        assert_eq!(
            cli_actor_payload(),
            json!({ "kind": "human", "user_id": "alice" })
        );
    }

    #[test]
    fn gateway_base_url_prefers_public_url() {
        let dir = tempdir().expect("tempdir");
        let config_path = dir.path().join("gary.json");
        std::fs::write(
            &config_path,
            serde_json::to_vec_pretty(&json!({
                "gateway": {
                    "host": "0.0.0.0",
                    "port": 3000,
                    "public_url": "http://127.0.0.1:31337"
                }
            }))
            .expect("config json"),
        )
        .expect("write config");
        let base_url =
            gateway_base_url(config_path.to_str().expect("config path")).expect("base url");
        assert_eq!(base_url, "http://127.0.0.1:31337");
    }
}
