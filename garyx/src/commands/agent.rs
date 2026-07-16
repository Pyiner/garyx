use super::*;

// ---------------------------------------------------------------------------
// Custom Agent commands
// ---------------------------------------------------------------------------
pub(crate) async fn cmd_agent_list(
    config_path: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(&gateway, "/api/custom-agents").await?;
    if json {
        return print_agent_json(decorate_agent_list_json(payload));
    }
    let mut agents = payload["agents"].as_array().cloned().unwrap_or_default();
    sort_agents_builtin_first(&mut agents);
    if agents.is_empty() {
        println!("Agents: (none)");
        return Ok(());
    }
    let raw_default_agent_id = payload["default_agent_id"].as_str();
    let effective_default_agent_id = payload["effective_default_agent_id"].as_str();
    for a in &agents {
        print_agent_summary(a);
        if let Some(marker) =
            agent_default_marker(a, raw_default_agent_id, effective_default_agent_id)
        {
            println!("Default: {marker}");
        }
        println!();
    }
    Ok(())
}

fn agent_default_marker(
    agent: &Value,
    raw_default_agent_id: Option<&str>,
    effective_default_agent_id: Option<&str>,
) -> Option<&'static str> {
    let agent_id = agent["agent_id"].as_str()?;
    let enabled = agent["enabled"].as_bool().unwrap_or(true);
    if raw_default_agent_id == Some(agent_id) {
        return if enabled && effective_default_agent_id == Some(agent_id) {
            Some("Default")
        } else {
            Some("Default (inactive)")
        };
    }
    if effective_default_agent_id == Some(agent_id) {
        return if raw_default_agent_id.is_none() {
            Some("Default (auto)")
        } else {
            Some("Acting default")
        };
    }
    None
}

fn sort_agents_builtin_first(agents: &mut [Value]) {
    agents.sort_by(|a, b| {
        let a_builtin = a["built_in"].as_bool().unwrap_or(false);
        let b_builtin = b["built_in"].as_bool().unwrap_or(false);
        // Reversed: builtin (true) should sort before custom (false).
        b_builtin.cmp(&a_builtin).then_with(|| {
            let a_id = a["agent_id"].as_str().unwrap_or("");
            let b_id = b["agent_id"].as_str().unwrap_or("");
            a_id.cmp(b_id)
        })
    });
}

fn decorate_agent_list_json(mut payload: Value) -> Value {
    if let Some(agents) = payload
        .get_mut("agents")
        .and_then(|value| value.as_array_mut())
    {
        sort_agents_builtin_first(agents);
        for agent in agents {
            let is_builtin = agent["built_in"].as_bool().unwrap_or(false);
            if let Some(obj) = agent.as_object_mut() {
                obj.insert(
                    "kind".to_string(),
                    Value::String(if is_builtin { "builtin" } else { "custom" }.to_string()),
                );
            }
        }
    }
    payload
}

/// Keep opaque image bodies out of terminal-oriented agent JSON while leaving
/// the authoritative Gateway response unchanged for app clients.
fn omit_agent_avatar_data_urls(value: &mut Value) {
    match value {
        Value::Array(values) => {
            for value in values {
                omit_agent_avatar_data_urls(value);
            }
        }
        Value::Object(object) => {
            object.remove("avatar_data_url");
            for value in object.values_mut() {
                omit_agent_avatar_data_urls(value);
            }
        }
        _ => {}
    }
}

fn project_agent_json(mut payload: Value) -> Value {
    omit_agent_avatar_data_urls(&mut payload);
    payload
}

fn print_agent_json(payload: Value) -> Result<(), Box<dyn std::error::Error>> {
    print_pretty_json(&project_agent_json(payload))
}

pub(crate) async fn cmd_agent_get(
    config_path: &str,
    agent_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(
        &gateway,
        &format!("/api/custom-agents/{}", urlencoding::encode(agent_id)),
    )
    .await?;
    if json {
        return print_agent_json(payload);
    }
    print_agent_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_agent_set_enabled(
    config_path: &str,
    agent_id: &str,
    enabled: bool,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let agent_id = agent_id.trim();
    if agent_id.is_empty() {
        return Err("agent_id cannot be empty".into());
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = patch_gateway_json(
        &gateway,
        &format!(
            "/api/custom-agents/{}/toggle",
            urlencoding::encode(agent_id)
        ),
        &json!({ "enabled": enabled }),
    )
    .await?;
    if json {
        return print_agent_json(payload);
    }
    print_agent_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_agent_default(
    config_path: &str,
    agent_id: Option<&str>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    if let Some(agent_id) = agent_id {
        let agent_id = agent_id.trim();
        if agent_id.is_empty() {
            return Err("agent_id cannot be empty".into());
        }
        let payload = patch_gateway_json(
            &gateway,
            &format!(
                "/api/custom-agents/{}/default",
                urlencoding::encode(agent_id)
            ),
            &json!({}),
        )
        .await?;
        if json {
            return print_agent_json(payload);
        }
        print_agent_summary(&payload);
        println!("Default: Default");
        return Ok(());
    }

    let payload = fetch_gateway_json(&gateway, "/api/custom-agents").await?;
    if json {
        return print_agent_json(decorate_agent_list_json(payload));
    }
    let raw = payload["default_agent_id"].as_str().unwrap_or("(auto)");
    let effective = payload["effective_default_agent_id"]
        .as_str()
        .unwrap_or("(none available)");
    println!("Configured default: {raw}");
    println!("Effective default: {effective}");
    Ok(())
}

/// Parse a `KEY=VALUE` CLI env pair, splitting on the first `=`. The value may
/// contain `=` and may be empty; the key must be a valid env name.
pub(super) fn parse_env_pair(pair: &str) -> Result<(String, String), Box<dyn std::error::Error>> {
    let (key, value) = pair
        .split_once('=')
        .ok_or_else(|| format!("--env must be KEY=VALUE, got: {pair}"))?;
    let key = key.trim();
    if !garyx_models::custom_agent::is_valid_env_key(key) {
        return Err(format!("invalid env key '{key}': must match [A-Za-z_][A-Za-z0-9_]*").into());
    }
    Ok((key.to_owned(), value.to_owned()))
}

/// Compute the final `provider_env` value for an agent mutation from the CLI env
/// flags, merged onto the existing env (client-side read-modify-write).
///
/// Returns `Ok(None)` when no env-affecting flag was given, so the caller omits
/// `provider_env` and the gateway preserves the stored value. Returns
/// `Ok(Some(map))` with the full desired map otherwise (`{}` clears all env).
/// Merge order: start from `{}` when `env_clear` else the existing map, then
/// remove `--unset-env` keys, then apply `--env` upserts.
fn resolve_cli_provider_env(
    existing: &Map<String, Value>,
    env_sets: &[String],
    unset_env: &[String],
    env_clear: bool,
) -> Result<Option<Value>, Box<dyn std::error::Error>> {
    let touched = !env_sets.is_empty() || !unset_env.is_empty() || env_clear;
    if !touched {
        return Ok(None);
    }
    let mut map = if env_clear {
        Map::new()
    } else {
        existing.clone()
    };
    for key in unset_env {
        map.remove(key.trim());
    }
    for pair in env_sets {
        let (key, value) = parse_env_pair(pair)?;
        map.insert(key, Value::String(value));
    }
    Ok(Some(Value::Object(map)))
}

/// Fetch the full current agent for a read-modify-write mutation.
///
/// Returns `Ok(Some(agent))` when the agent exists, `Ok(None)` on a 404 (so
/// `upsert` can fall through to create), and `Err` on any other failure
/// (unreachable, 5xx, malformed body). Distinguishing these prevents a failed
/// read from silently rebuilding the agent out of CLI-provided fields only.
async fn fetch_existing_agent(
    gateway: &GatewayEndpoint,
    agent_id: &str,
) -> Result<Option<Value>, Box<dyn std::error::Error>> {
    let path = format!(
        "/api/custom-agents/{}",
        urlencoding::encode(agent_id.trim())
    );
    match fetch_gateway_json(gateway, &path).await {
        Ok(agent) => Ok(Some(agent)),
        Err(error) => match error.downcast_ref::<GatewayCliError>() {
            Some(gateway_error) if gateway_error.kind == GatewayErrorKind::NotFound => Ok(None),
            _ => Err(error),
        },
    }
}

/// Copy the concurrency token out of a freshly fetched profile into the
/// mutation body: the gateway only applies the write when the stored
/// `updated_at` still matches, so this GET->PUT pair cannot overwrite a
/// concurrent edit or resurrect a deleted profile (#TASK-1761).
fn attach_expected_updated_at(
    body: &mut Value,
    existing: &Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let updated_at = existing["updated_at"]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(
            "gateway returned a profile without updated_at — cannot build a conditional update",
        )?;
    body["expected_updated_at"] = Value::String(updated_at.to_owned());
    Ok(())
}

/// An explicitly passed flag must not be blank: blank is neither a valid
/// value nor "omit" (omit = don't pass the flag). Silently preserving the
/// stored value would make scripts believe their write took effect.
fn reject_blank_flag(
    value: Option<String>,
    flag: &str,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    match value {
        Some(value) => {
            let trimmed = value.trim().to_owned();
            if trimmed.is_empty() {
                return Err(format!(
                    "{flag} cannot be blank — omit the flag to keep the current value"
                )
                .into());
            }
            Ok(Some(trimmed))
        }
        None => Ok(None),
    }
}

/// Resolve the identity fields of an agent mutation, preserving the stored
/// values when the flags are omitted. The gateway upsert payload requires
/// `display_name` and `provider_type`, so an update/upsert that omits them must
/// fill them from the existing agent instead of overwriting with defaults.
fn merge_agent_identity(
    existing: Option<&Value>,
    display_name: Option<String>,
    provider: Option<String>,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    let display_name = reject_blank_flag(display_name, "--display-name")?
        .or_else(|| {
            existing.and_then(|agent| {
                agent["display_name"]
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
            })
        })
        .ok_or("--display-name is required when creating a new agent")?;
    let provider = reject_blank_flag(provider, "--provider")?
        .or_else(|| {
            existing.and_then(|agent| {
                agent["provider_type"]
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
            })
        })
        .unwrap_or_else(|| "claude_code".to_owned());
    Ok((display_name, provider))
}

fn existing_provider_env(existing: Option<&Value>) -> Map<String, Value> {
    existing
        .and_then(|agent| agent.get("provider_env"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn build_agent_mutation_body(
    agent_id: String,
    display_name: String,
    provider: String,
    model: Option<String>,
    clear_model: bool,
    model_reasoning_effort: Option<String>,
    model_service_tier: Option<String>,
    provider_env: Option<Value>,
    default_workspace_dir: Option<String>,
    system_prompt: Option<String>,
) -> Result<Value, Box<dyn std::error::Error>> {
    let agent_id = agent_id.trim().to_owned();
    if agent_id.is_empty() {
        return Err("agent_id cannot be empty".into());
    }
    let mut body = json!({
        "agent_id": agent_id,
        "display_name": display_name.trim(),
        "provider_type": provider.trim(),
    });
    if let Some(system_prompt) = system_prompt.as_deref().map(str::trim) {
        body["system_prompt"] = Value::String(system_prompt.to_owned());
    }
    if clear_model {
        body["model"] = Value::String(String::new());
    } else if let Some(model) = model.as_deref().map(str::trim) {
        body["model"] = Value::String(model.to_owned());
    }
    if let Some(effort) = model_reasoning_effort.as_deref().map(str::trim) {
        body["model_reasoning_effort"] = Value::String(effort.to_owned());
    }
    if let Some(tier) = model_service_tier.as_deref().map(str::trim) {
        body["model_service_tier"] = Value::String(tier.to_owned());
    }
    if let Some(provider_env) = provider_env {
        body["provider_env"] = provider_env;
    }
    if let Some(default_workspace_dir) = default_workspace_dir {
        body["default_workspace_dir"] = Value::String(default_workspace_dir.trim().to_owned());
    }
    Ok(body)
}

pub(crate) async fn cmd_agent_create(
    config_path: &str,
    agent_id: String,
    display_name: String,
    provider: String,
    model: Option<String>,
    model_reasoning_effort: Option<String>,
    model_service_tier: Option<String>,
    env: Vec<String>,
    unset_env: Vec<String>,
    env_clear: bool,
    default_workspace_dir: Option<String>,
    system_prompt: Option<String>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    // New agents start from an empty env, so no existing-env fetch is needed.
    let provider_env = resolve_cli_provider_env(&Map::new(), &env, &unset_env, env_clear)?;
    let body = build_agent_mutation_body(
        agent_id,
        display_name,
        provider,
        model,
        false,
        model_reasoning_effort,
        model_service_tier,
        provider_env,
        default_workspace_dir,
        system_prompt,
    )?;
    let payload = post_gateway_json(&gateway, "/api/custom-agents", &body).await?;
    if json {
        return print_agent_json(payload);
    }
    print_agent_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_agent_update(
    config_path: &str,
    agent_id: String,
    display_name: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    clear_model: bool,
    model_reasoning_effort: Option<String>,
    model_service_tier: Option<String>,
    env: Vec<String>,
    unset_env: Vec<String>,
    env_clear: bool,
    default_workspace_dir: Option<String>,
    system_prompt: Option<String>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    // Read-modify-write: the gateway upsert payload requires identity fields,
    // so update always starts from the stored agent and only overwrites what
    // the invocation explicitly passed.
    let existing = fetch_existing_agent(&gateway, &agent_id).await?.ok_or_else(|| {
        // Keep the NotFound class so `--json` reports kind=not_found and the
        // process exits 4, same as a direct 404.
        GatewayCliError {
            kind: GatewayErrorKind::NotFound,
            message: format!(
                "custom agent '{}' not found — list agents with `garyx agent list`, or create it with `garyx agent create` / `garyx agent upsert`",
                agent_id.trim()
            ),
        }
    })?;
    let (display_name, provider) = merge_agent_identity(Some(&existing), display_name, provider)?;
    let provider_env = resolve_cli_provider_env(
        &existing_provider_env(Some(&existing)),
        &env,
        &unset_env,
        env_clear,
    )?;
    let mut body = build_agent_mutation_body(
        agent_id.clone(),
        display_name,
        provider,
        model,
        clear_model,
        model_reasoning_effort,
        model_service_tier,
        provider_env,
        default_workspace_dir,
        system_prompt,
    )?;
    attach_expected_updated_at(&mut body, &existing)?;
    let url = format!(
        "/api/custom-agents/{}",
        urlencoding::encode(agent_id.trim())
    );
    let payload = put_gateway_json(&gateway, &url, &body).await?;
    if json {
        return print_agent_json(payload);
    }
    print_agent_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_agent_upsert(
    config_path: &str,
    agent_id: String,
    display_name: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    clear_model: bool,
    model_reasoning_effort: Option<String>,
    model_service_tier: Option<String>,
    env: Vec<String>,
    unset_env: Vec<String>,
    env_clear: bool,
    default_workspace_dir: Option<String>,
    system_prompt: Option<String>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let existing = fetch_existing_agent(&gateway, &agent_id).await?;
    let (display_name, provider) = merge_agent_identity(existing.as_ref(), display_name, provider)?;
    let provider_env = resolve_cli_provider_env(
        &existing_provider_env(existing.as_ref()),
        &env,
        &unset_env,
        env_clear,
    )?;
    let mut body = build_agent_mutation_body(
        agent_id.clone(),
        display_name,
        provider,
        model,
        clear_model,
        model_reasoning_effort,
        model_service_tier,
        provider_env,
        default_workspace_dir,
        system_prompt,
    )?;
    let payload = if let Some(existing) = existing.as_ref() {
        attach_expected_updated_at(&mut body, existing)?;
        let url = format!(
            "/api/custom-agents/{}",
            urlencoding::encode(agent_id.trim())
        );
        put_gateway_json(&gateway, &url, &body).await?
    } else {
        post_gateway_json(&gateway, "/api/custom-agents", &body).await?
    };
    if json {
        return print_agent_json(payload);
    }
    print_agent_summary(&payload);
    Ok(())
}

pub(crate) async fn cmd_agent_delete(
    config_path: &str,
    agent_id: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let agent_id = agent_id.trim();
    if agent_id.is_empty() {
        return Err("agent_id cannot be empty".into());
    }
    let gateway = gateway_endpoint(config_path)?;
    let payload = delete_gateway_json(
        &gateway,
        &format!("/api/custom-agents/{}", urlencoding::encode(agent_id)),
    )
    .await?;
    if json {
        return print_agent_json(payload);
    }
    println!("Deleted agent: {agent_id}");
    Ok(())
}

fn print_agent_summary(a: &Value) {
    let agent_id = a["agent_id"].as_str().unwrap_or("-");
    let name = a["display_name"].as_str().unwrap_or("-");
    let provider = a["provider_type"].as_str().unwrap_or("-");
    let model = a["model"].as_str().unwrap_or("").trim();
    let model_reasoning_effort = a["model_reasoning_effort"].as_str().unwrap_or("").trim();
    let model_service_tier = a["model_service_tier"].as_str().unwrap_or("").trim();
    let builtin = a["built_in"].as_bool().unwrap_or(false);
    println!(
        "Agent: {agent_id}{}",
        if builtin { " (built-in)" } else { "" }
    );
    println!("Name: {name}");
    println!("Provider: {provider}");
    println!(
        "Status: {}",
        if a["enabled"].as_bool().unwrap_or(true) {
            "enabled"
        } else {
            "disabled"
        }
    );
    if !model.is_empty() {
        println!("Model: {model}");
    }
    if !model_reasoning_effort.is_empty() {
        println!("Reasoning effort: {model_reasoning_effort}");
    }
    if !model_service_tier.is_empty() {
        println!("Service tier: {model_service_tier}");
    }
    if let Some(default_workspace_dir) = a["default_workspace_dir"].as_str()
        && !default_workspace_dir.trim().is_empty()
    {
        println!("Default workspace: {}", default_workspace_dir.trim());
    }
    if let Some(env) = a["provider_env"].as_object()
        && !env.is_empty()
    {
        println!("Environment:");
        let mut keys: Vec<&String> = env.keys().collect();
        keys.sort();
        for key in keys {
            let value = env[key].as_str().unwrap_or("");
            // Values print verbatim: agent env is the user's own local config,
            // and masked output hides exactly what someone inspecting an agent
            // needs to verify (e.g. whether a numeric value picked up quotes).
            println!("  {key}={value}");
        }
    }
    if let Some(prompt) = a["system_prompt"].as_str()
        && !prompt.trim().is_empty()
    {
        let preview: String = prompt.chars().take(120).collect();
        let ellipsis = if prompt.len() > 120 { "…" } else { "" };
        println!("Prompt: {preview}{ellipsis}");
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]

    use super::*;
    use crate::commands::test_support::*;
    use axum::{
        Json, Router,
        extract::Path as AxumPath,
        http::StatusCode,
        routing::{get, patch, post},
    };
    use std::sync::{Arc as StdArc, Mutex};
    use tempfile::tempdir;
    use tokio::{net::TcpListener, task::JoinHandle};

    /// Agent test server: records POST/PUT mutations into `requests` (GET
    /// reads are not recorded). `get_status` controls whether the existing-
    /// agent read succeeds (200 with a canned codex agent) or 404s, which is
    /// how tests select the update (existing) vs create (missing) paths.
    async fn spawn_agent_http_test_server(
        requests: StdArc<Mutex<Vec<RecordedRequest>>>,
        put_status: StatusCode,
        get_status: StatusCode,
    ) -> (String, JoinHandle<()>) {
        let post_requests = requests.clone();
        let put_requests = requests.clone();
        let app = Router::new()
            .route(
                "/api/custom-agents",
                post(move |Json(payload): Json<Value>| {
                    let requests = post_requests.clone();
                    async move {
                        requests
                            .lock()
                            .expect("request lock")
                            .push(RecordedRequest {
                                method: "POST".to_owned(),
                                path: "/api/custom-agents".to_owned(),
                                body: payload.clone(),
                            });
                        (
                            StatusCode::CREATED,
                            Json(json!({
                                "agent_id": payload["agent_id"],
                                "display_name": payload["display_name"],
                                "provider_type": payload["provider_type"],
                                "model": payload["model"],
                                "system_prompt": payload["system_prompt"],
                                "built_in": false,
                            })),
                        )
                    }
                }),
            )
            .route(
                "/api/custom-agents/{agent_id}",
                get(move |AxumPath(agent_id): AxumPath<String>| async move {
                    if get_status.is_success() {
                        (
                            get_status,
                            Json(json!({
                                "agent_id": agent_id,
                                "display_name": "Existing Agent",
                                "provider_type": "codex_app_server",
                                "model": "existing-model",
                                "system_prompt": "Existing prompt.",
                                "updated_at": "2026-01-01T00:00:00Z",
                                "built_in": false,
                            })),
                        )
                    } else {
                        (
                            get_status,
                            Json(json!({ "error": "custom agent not found" })),
                        )
                    }
                })
                .put(
                    move |AxumPath(agent_id): AxumPath<String>, Json(payload): Json<Value>| {
                        let requests = put_requests.clone();
                        async move {
                            let path = format!("/api/custom-agents/{agent_id}");
                            requests
                                .lock()
                                .expect("request lock")
                                .push(RecordedRequest {
                                    method: "PUT".to_owned(),
                                    path,
                                    body: payload.clone(),
                                });
                            if put_status.is_success() {
                                (
                                    put_status,
                                    Json(json!({
                                        "agent_id": agent_id,
                                        "display_name": payload["display_name"],
                                        "provider_type": payload["provider_type"],
                                        "model": payload["model"],
                                        "system_prompt": payload["system_prompt"],
                                        "built_in": false,
                                    })),
                                )
                            } else {
                                (
                                    put_status,
                                    Json(json!({ "error": "custom agent not found" })),
                                )
                            }
                        }
                    },
                ),
            );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test router");
        });
        (format!("http://{addr}"), handle)
    }

    /// Agent test server that also answers GET with a canned agent carrying
    /// `existing_env`, so read-modify-write env merges can be exercised end to end.
    /// Only PUT requests are recorded into `requests`.
    async fn spawn_agent_http_test_server_with_env(
        requests: StdArc<Mutex<Vec<RecordedRequest>>>,
        existing_env: Value,
    ) -> (String, JoinHandle<()>) {
        let put_requests = requests.clone();
        let get_env = existing_env;
        let app = Router::new().route(
            "/api/custom-agents/{agent_id}",
            get(move |AxumPath(agent_id): AxumPath<String>| {
                let env = get_env.clone();
                async move {
                    (
                        StatusCode::OK,
                        Json(json!({
                            "agent_id": agent_id,
                            "display_name": "Existing Agent",
                            "provider_type": "codex_app_server",
                            "model": "",
                            "system_prompt": "",
                            "provider_env": env,
                            "updated_at": "2026-01-01T00:00:00Z",
                            "built_in": false,
                        })),
                    )
                }
            })
            .put(
                move |AxumPath(agent_id): AxumPath<String>, Json(payload): Json<Value>| {
                    let requests = put_requests.clone();
                    async move {
                        requests
                            .lock()
                            .expect("request lock")
                            .push(RecordedRequest {
                                method: "PUT".to_owned(),
                                path: format!("/api/custom-agents/{agent_id}"),
                                body: payload.clone(),
                            });
                        (
                            StatusCode::OK,
                            Json(json!({
                                "agent_id": agent_id,
                                "display_name": payload["display_name"],
                                "provider_type": payload["provider_type"],
                                "model": payload["model"],
                                "system_prompt": payload["system_prompt"],
                                "built_in": false,
                            })),
                        )
                    }
                },
            ),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test router");
        });
        (format!("http://{addr}"), handle)
    }

    /// Agent test server whose GET returns a fixed (failure) status, to prove that a
    /// failed existing-env read aborts the mutation instead of merging onto `{}`.
    /// PUT requests are still recorded so tests can assert none were sent.
    async fn spawn_agent_http_test_server_get_status(
        requests: StdArc<Mutex<Vec<RecordedRequest>>>,
        get_status: StatusCode,
    ) -> (String, JoinHandle<()>) {
        let put_requests = requests.clone();
        let app = Router::new().route(
            "/api/custom-agents/{agent_id}",
            get(move |AxumPath(_agent_id): AxumPath<String>| async move {
                (get_status, Json(json!({ "error": "boom" })))
            })
            .put(
                move |AxumPath(agent_id): AxumPath<String>, Json(payload): Json<Value>| {
                    let requests = put_requests.clone();
                    async move {
                        requests
                            .lock()
                            .expect("request lock")
                            .push(RecordedRequest {
                                method: "PUT".to_owned(),
                                path: format!("/api/custom-agents/{agent_id}"),
                                body: payload.clone(),
                            });
                        (
                            StatusCode::OK,
                            Json(json!({ "agent_id": agent_id, "built_in": false })),
                        )
                    }
                },
            ),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test router");
        });
        (format!("http://{addr}"), handle)
    }

    async fn spawn_agent_availability_http_test_server(
        requests: StdArc<Mutex<Vec<RecordedRequest>>>,
    ) -> (String, JoinHandle<()>) {
        let list_requests = requests.clone();
        let toggle_requests = requests.clone();
        let default_requests = requests;
        let app = Router::new()
            .route(
                "/api/custom-agents",
                get(move || {
                    let requests = list_requests.clone();
                    async move {
                        requests
                            .lock()
                            .expect("request lock")
                            .push(RecordedRequest {
                                method: "GET".to_owned(),
                                path: "/api/custom-agents".to_owned(),
                                body: Value::Null,
                            });
                        Json(json!({
                            "agents": [
                                {
                                    "agent_id": "claude",
                                    "display_name": "Claude",
                                    "provider_type": "claude_code",
                                    "built_in": true,
                                    "standalone": true,
                                    "enabled": true
                                },
                                {
                                    "agent_id": "reviewer",
                                    "display_name": "Reviewer",
                                    "provider_type": "codex_app_server",
                                    "built_in": false,
                                    "standalone": true,
                                    "enabled": true
                                }
                            ],
                            "default_agent_id": "reviewer",
                            "effective_default_agent_id": "reviewer"
                        }))
                    }
                }),
            )
            .route(
                "/api/custom-agents/{agent_id}/toggle",
                patch(
                    move |AxumPath(agent_id): AxumPath<String>, Json(payload): Json<Value>| {
                        let requests = toggle_requests.clone();
                        async move {
                            requests
                                .lock()
                                .expect("request lock")
                                .push(RecordedRequest {
                                    method: "PATCH".to_owned(),
                                    path: format!("/api/custom-agents/{agent_id}/toggle"),
                                    body: payload.clone(),
                                });
                            Json(json!({
                                "agent_id": agent_id,
                                "display_name": "Reviewer",
                                "provider_type": "codex_app_server",
                                "built_in": false,
                                "standalone": true,
                                "enabled": payload["enabled"]
                            }))
                        }
                    },
                ),
            )
            .route(
                "/api/custom-agents/{agent_id}/default",
                patch(
                    move |AxumPath(agent_id): AxumPath<String>, Json(payload): Json<Value>| {
                        let requests = default_requests.clone();
                        async move {
                            requests
                                .lock()
                                .expect("request lock")
                                .push(RecordedRequest {
                                    method: "PATCH".to_owned(),
                                    path: format!("/api/custom-agents/{agent_id}/default"),
                                    body: payload,
                                });
                            if agent_id == "disabled" {
                                (
                                    StatusCode::BAD_REQUEST,
                                    Json(json!({"error": "agent is disabled: disabled"})),
                                )
                            } else {
                                (
                                    StatusCode::OK,
                                    Json(json!({
                                        "agent_id": agent_id,
                                        "display_name": "Reviewer",
                                        "provider_type": "codex_app_server",
                                        "built_in": false,
                                        "standalone": true,
                                        "enabled": true
                                    })),
                                )
                            }
                        }
                    },
                ),
            );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test router");
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn agent_list_reads_enabled_raw_and_effective_contract() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_agent_availability_http_test_server(requests.clone()).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_agent_list(config_path.to_str().expect("config path"), true)
            .await
            .expect("agent list should succeed");

        handle.abort();
        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].method, "GET");
        assert_eq!(records[0].path, "/api/custom-agents");
    }

    #[tokio::test]
    async fn agent_enable_and_disable_patch_exact_enabled_state() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_agent_availability_http_test_server(requests.clone()).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_agent_set_enabled(
            config_path.to_str().expect("config path"),
            "reviewer",
            false,
            true,
        )
        .await
        .expect("disable should succeed");
        cmd_agent_set_enabled(
            config_path.to_str().expect("config path"),
            "reviewer",
            true,
            true,
        )
        .await
        .expect("enable should succeed");

        handle.abort();
        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].path, "/api/custom-agents/reviewer/toggle");
        assert_eq!(records[0].body, json!({"enabled": false}));
        assert_eq!(records[1].path, "/api/custom-agents/reviewer/toggle");
        assert_eq!(records[1].body, json!({"enabled": true}));
    }

    #[tokio::test]
    async fn agent_default_sets_when_id_present_and_only_reads_when_omitted() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_agent_availability_http_test_server(requests.clone()).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_agent_default(
            config_path.to_str().expect("config path"),
            Some("reviewer"),
            true,
        )
        .await
        .expect("set default should succeed");
        cmd_agent_default(config_path.to_str().expect("config path"), None, true)
            .await
            .expect("read default should succeed");

        handle.abort();
        let records = requests.lock().expect("request lock");
        assert_eq!(
            records
                .iter()
                .filter(|record| record.method == "PATCH")
                .count(),
            1
        );
        assert_eq!(records[0].path, "/api/custom-agents/reviewer/default");
        assert_eq!(records[0].body, json!({}));
        assert_eq!(
            records
                .iter()
                .filter(|record| record.method == "GET")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn agent_default_preserves_disabled_rejection_without_fallback_read() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_agent_availability_http_test_server(requests.clone()).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        let error = cmd_agent_default(
            config_path.to_str().expect("config path"),
            Some("disabled"),
            true,
        )
        .await
        .expect_err("disabled agent cannot become default");

        handle.abort();
        let gateway_error = error
            .downcast_ref::<GatewayCliError>()
            .expect("gateway rejection must remain typed");
        assert_eq!(gateway_error.kind, GatewayErrorKind::Rejected);
        assert!(
            gateway_error
                .message
                .ends_with("agent is disabled: disabled")
        );
        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 1, "rejection must not fall through to GET");
        assert_eq!(records[0].path, "/api/custom-agents/disabled/default");
    }

    #[tokio::test]
    async fn cmd_agent_create_posts_model_payload() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) =
            spawn_agent_http_test_server(requests.clone(), StatusCode::OK, StatusCode::NOT_FOUND)
                .await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_agent_create(
            config_path.to_str().expect("config path"),
            "spec-review".to_owned(),
            "Spec Review".to_owned(),
            "codex_app_server".to_owned(),
            Some("gpt-5".to_owned()),
            Some("high".to_owned()),
            Some("priority".to_owned()),
            Vec::new(),
            Vec::new(),
            false,
            Some("/tmp/spec-review".to_owned()),
            Some("Review specs carefully.".to_owned()),
            false,
        )
        .await
        .expect("agent create should succeed");

        handle.abort();

        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].method, "POST");
        assert_eq!(records[0].path, "/api/custom-agents");
        assert_eq!(records[0].body["agent_id"], "spec-review");
        assert_eq!(records[0].body["model"], "gpt-5");
        assert_eq!(records[0].body["model_reasoning_effort"], "high");
        assert_eq!(records[0].body["model_service_tier"], "priority");
        assert_eq!(records[0].body["default_workspace_dir"], "/tmp/spec-review");
        assert_eq!(records[0].body["system_prompt"], "Review specs carefully.");
    }

    #[tokio::test]
    async fn cmd_agent_create_omits_system_prompt_when_omitted() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) =
            spawn_agent_http_test_server(requests.clone(), StatusCode::OK, StatusCode::NOT_FOUND)
                .await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_agent_create(
            config_path.to_str().expect("config path"),
            "plain-claude".to_owned(),
            "Plain Claude".to_owned(),
            "claude_code".to_owned(),
            None,
            None,
            None,
            Vec::new(),
            Vec::new(),
            false,
            None,
            None,
            false,
        )
        .await
        .expect("agent create should succeed");

        handle.abort();

        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].method, "POST");
        assert!(records[0].body.get("system_prompt").is_none());
    }

    #[tokio::test]
    async fn cmd_agent_update_omits_model_fields_when_omitted() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) =
            spawn_agent_http_test_server(requests.clone(), StatusCode::OK, StatusCode::OK).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_agent_update(
            config_path.to_str().expect("config path"),
            "spec-review".to_owned(),
            None,
            None,
            None,
            false,
            None,
            None,
            Vec::new(),
            Vec::new(),
            false,
            None,
            None,
            false,
        )
        .await
        .expect("agent update should succeed");

        handle.abort();

        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].method, "PUT");
        assert_eq!(records[0].path, "/api/custom-agents/spec-review");
        // Omitted identity fields are preserved from the stored agent instead
        // of being overwritten with defaults (the old behavior silently reset
        // provider_type to claude_code).
        assert_eq!(records[0].body["display_name"], "Existing Agent");
        assert_eq!(records[0].body["provider_type"], "codex_app_server");
        // The conditional-update token from the fetched agent rides along.
        assert_eq!(
            records[0].body["expected_updated_at"],
            "2026-01-01T00:00:00Z"
        );
        assert!(records[0].body.get("model").is_none());
        assert!(records[0].body.get("model_reasoning_effort").is_none());
        assert!(records[0].body.get("model_service_tier").is_none());
        assert!(records[0].body.get("system_prompt").is_none());
    }

    #[tokio::test]
    async fn cmd_agent_update_sends_empty_model_when_clear_model_is_set() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) =
            spawn_agent_http_test_server(requests.clone(), StatusCode::OK, StatusCode::OK).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_agent_update(
            config_path.to_str().expect("config path"),
            "spec-review".to_owned(),
            Some("Spec Review".to_owned()),
            Some("codex_app_server".to_owned()),
            None,
            true,
            None,
            None,
            Vec::new(),
            Vec::new(),
            false,
            None,
            Some("Review specs carefully.".to_owned()),
            false,
        )
        .await
        .expect("agent update should succeed");

        handle.abort();

        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].method, "PUT");
        assert_eq!(records[0].path, "/api/custom-agents/spec-review");
        assert_eq!(records[0].body["model"], "");
        assert!(records[0].body.get("model_reasoning_effort").is_none());
        assert!(records[0].body.get("model_service_tier").is_none());
    }

    #[tokio::test]
    async fn cmd_agent_update_sends_empty_system_prompt_when_explicitly_blank() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) =
            spawn_agent_http_test_server(requests.clone(), StatusCode::OK, StatusCode::OK).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_agent_update(
            config_path.to_str().expect("config path"),
            "spec-review".to_owned(),
            Some("Spec Review".to_owned()),
            Some("codex_app_server".to_owned()),
            None,
            false,
            None,
            None,
            Vec::new(),
            Vec::new(),
            false,
            None,
            Some("  ".to_owned()),
            false,
        )
        .await
        .expect("agent update should succeed");

        handle.abort();

        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].method, "PUT");
        assert_eq!(records[0].body["system_prompt"], "");
    }

    #[tokio::test]
    async fn cmd_agent_upsert_creates_via_post_when_agent_missing() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) =
            spawn_agent_http_test_server(requests.clone(), StatusCode::OK, StatusCode::NOT_FOUND)
                .await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_agent_upsert(
            config_path.to_str().expect("config path"),
            "spec-review".to_owned(),
            Some("Spec Review".to_owned()),
            Some("antigravity".to_owned()),
            Some("Claude Opus 4.6 (Thinking)".to_owned()),
            false,
            None,
            None,
            Vec::new(),
            Vec::new(),
            false,
            None,
            Some("Review specs carefully.".to_owned()),
            false,
        )
        .await
        .expect("agent upsert should succeed");

        handle.abort();

        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].method, "POST");
        assert_eq!(records[0].path, "/api/custom-agents");
        assert_eq!(records[0].body["model"], "Claude Opus 4.6 (Thinking)");
        assert_eq!(records[0].body["provider_type"], "antigravity");
    }

    #[tokio::test]
    async fn cmd_agent_upsert_updates_existing_and_preserves_omitted_identity() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) =
            spawn_agent_http_test_server(requests.clone(), StatusCode::OK, StatusCode::OK).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_agent_upsert(
            config_path.to_str().expect("config path"),
            "spec-review".to_owned(),
            None,
            None,
            Some("gpt-5".to_owned()),
            false,
            None,
            None,
            Vec::new(),
            Vec::new(),
            false,
            None,
            None,
            false,
        )
        .await
        .expect("agent upsert should succeed");

        handle.abort();

        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].method, "PUT");
        assert_eq!(records[0].body["display_name"], "Existing Agent");
        assert_eq!(records[0].body["provider_type"], "codex_app_server");
        assert_eq!(records[0].body["model"], "gpt-5");
    }

    #[tokio::test]
    async fn cmd_agent_upsert_requires_display_name_when_creating() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) =
            spawn_agent_http_test_server(requests.clone(), StatusCode::OK, StatusCode::NOT_FOUND)
                .await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        let result = cmd_agent_upsert(
            config_path.to_str().expect("config path"),
            "brand-new".to_owned(),
            None,
            None,
            None,
            false,
            None,
            None,
            Vec::new(),
            Vec::new(),
            false,
            None,
            None,
            false,
        )
        .await;

        handle.abort();
        assert!(
            result.is_err(),
            "creating a new agent without --display-name must fail"
        );
        let records = requests.lock().expect("request lock");
        assert!(records.is_empty(), "no mutation may be sent: {records:?}");
    }

    #[tokio::test]
    async fn cmd_agent_update_fails_when_agent_missing() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) =
            spawn_agent_http_test_server(requests.clone(), StatusCode::OK, StatusCode::NOT_FOUND)
                .await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        let result = cmd_agent_update(
            config_path.to_str().expect("config path"),
            "missing-agent".to_owned(),
            Some("Name".to_owned()),
            None,
            None,
            false,
            None,
            None,
            Vec::new(),
            Vec::new(),
            false,
            None,
            None,
            false,
        )
        .await;

        handle.abort();
        assert!(result.is_err(), "update of a missing agent must fail");
        let records = requests.lock().expect("request lock");
        assert!(records.is_empty(), "no mutation may be sent: {records:?}");
    }

    #[test]
    fn resolve_cli_provider_env_returns_none_when_untouched() {
        let existing = Map::new();
        let out = resolve_cli_provider_env(&existing, &[], &[], false).expect("ok");
        assert!(
            out.is_none(),
            "no env flags must omit provider_env (preserve)"
        );
    }

    #[test]
    fn resolve_cli_provider_env_merges_sets_onto_existing() {
        let mut existing = Map::new();
        existing.insert("OLD_KEY".to_owned(), Value::String("old".to_owned()));
        let sets = vec!["NEW_KEY=new".to_owned(), "OLD_KEY=updated".to_owned()];
        let out = resolve_cli_provider_env(&existing, &sets, &[], false)
            .expect("ok")
            .expect("some");
        assert_eq!(out["OLD_KEY"], "updated");
        assert_eq!(out["NEW_KEY"], "new");
    }

    #[test]
    fn resolve_cli_provider_env_unset_and_clear() {
        let mut existing = Map::new();
        existing.insert("A".to_owned(), Value::String("1".to_owned()));
        existing.insert("B".to_owned(), Value::String("2".to_owned()));

        let out = resolve_cli_provider_env(&existing, &[], &["A".to_owned()], false)
            .expect("ok")
            .expect("some");
        assert!(out.get("A").is_none());
        assert_eq!(out["B"], "2");

        let cleared = resolve_cli_provider_env(&existing, &[], &[], true)
            .expect("ok")
            .expect("some");
        assert_eq!(cleared, json!({}));

        let cleared_set = resolve_cli_provider_env(&existing, &["C=3".to_owned()], &[], true)
            .expect("ok")
            .expect("some");
        assert_eq!(cleared_set, json!({ "C": "3" }));
    }

    #[test]
    fn parse_env_pair_splits_on_first_equals_and_validates_key() {
        assert_eq!(
            parse_env_pair("KEY=a=b=c").expect("ok"),
            ("KEY".to_owned(), "a=b=c".to_owned())
        );
        assert_eq!(
            parse_env_pair("EMPTY=").expect("ok"),
            ("EMPTY".to_owned(), String::new())
        );
        assert!(parse_env_pair("NO_EQUALS").is_err());
        assert!(parse_env_pair("1BAD=x").is_err());
        assert!(parse_env_pair("HAS SPACE=x").is_err());
    }

    #[tokio::test]
    async fn cmd_agent_create_sends_multiple_env_vars() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) =
            spawn_agent_http_test_server(requests.clone(), StatusCode::OK, StatusCode::NOT_FOUND)
                .await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_agent_create(
            config_path.to_str().expect("config path"),
            "envy".to_owned(),
            "Envy".to_owned(),
            "claude_code".to_owned(),
            None,
            None,
            None,
            vec!["A=1".to_owned(), "B=two".to_owned()],
            Vec::new(),
            false,
            None,
            Some("Prompt.".to_owned()),
            false,
        )
        .await
        .expect("agent create should succeed");

        handle.abort();
        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].method, "POST");
        assert_eq!(records[0].body["provider_env"]["A"], "1");
        assert_eq!(records[0].body["provider_env"]["B"], "two");
    }

    #[tokio::test]
    async fn cmd_agent_update_without_env_flags_omits_provider_env() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) =
            spawn_agent_http_test_server(requests.clone(), StatusCode::OK, StatusCode::OK).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_agent_update(
            config_path.to_str().expect("config path"),
            "spec-review".to_owned(),
            Some("Spec Review".to_owned()),
            Some("claude_code".to_owned()),
            None,
            false,
            None,
            None,
            Vec::new(),
            Vec::new(),
            false,
            None,
            Some("Prompt.".to_owned()),
            false,
        )
        .await
        .expect("agent update should succeed");

        handle.abort();
        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].method, "PUT");
        assert!(
            records[0].body.get("provider_env").is_none(),
            "unchanged env must omit provider_env so the gateway preserves it"
        );
    }

    #[tokio::test]
    async fn cmd_agent_update_merges_env_onto_existing_via_read_modify_write() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_agent_http_test_server_with_env(
            requests.clone(),
            json!({ "EXISTING_KEY": "keep-me" }),
        )
        .await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_agent_update(
            config_path.to_str().expect("config path"),
            "spec-review".to_owned(),
            Some("Spec Review".to_owned()),
            Some("claude_code".to_owned()),
            None,
            false,
            None,
            None,
            vec!["NEW_KEY=new-value".to_owned()],
            Vec::new(),
            false,
            None,
            Some("Prompt.".to_owned()),
            false,
        )
        .await
        .expect("agent update should succeed");

        handle.abort();
        let records = requests.lock().expect("request lock");
        let put = records
            .iter()
            .find(|record| record.method == "PUT")
            .expect("put recorded");
        assert_eq!(put.body["provider_env"]["EXISTING_KEY"], "keep-me");
        assert_eq!(put.body["provider_env"]["NEW_KEY"], "new-value");
    }

    #[tokio::test]
    async fn cmd_agent_update_fails_before_put_when_env_fetch_errors() {
        // A failed existing-agent read must abort the update, not merge env flags
        // onto an empty map (which would drop the agent's stored env).
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_agent_http_test_server_get_status(
            requests.clone(),
            StatusCode::INTERNAL_SERVER_ERROR,
        )
        .await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        let result = cmd_agent_update(
            config_path.to_str().expect("config path"),
            "spec-review".to_owned(),
            Some("Spec Review".to_owned()),
            Some("claude_code".to_owned()),
            None,
            false,
            None,
            None,
            vec!["NEW=1".to_owned()],
            Vec::new(),
            false,
            None,
            Some("Prompt.".to_owned()),
            false,
        )
        .await;

        handle.abort();
        assert!(
            result.is_err(),
            "env-merge update must fail when the existing-env read fails"
        );
        let records = requests.lock().expect("request lock");
        assert!(
            records.iter().all(|record| record.method != "PUT"),
            "must not PUT after a failed read (would risk dropping existing env)"
        );
    }

    #[test]
    fn merge_agent_identity_rejects_blank_explicit_values() {
        let existing = json!({
            "display_name": "Existing Agent",
            "provider_type": "codex_app_server",
        });
        // Blank display name: error, not silent preserve.
        let blank_name = merge_agent_identity(Some(&existing), Some("   ".to_owned()), None);
        assert!(blank_name.is_err(), "blank --display-name must fail");
        // Blank provider: error, not silent preserve.
        let blank_provider = merge_agent_identity(Some(&existing), None, Some(String::new()));
        assert!(blank_provider.is_err(), "blank --provider must fail");
        // Omitted flags still preserve stored values.
        let (name, provider) = merge_agent_identity(Some(&existing), None, None).expect("merge");
        assert_eq!(name, "Existing Agent");
        assert_eq!(provider, "codex_app_server");
    }

    fn agent_value(agent_id: &str, built_in: bool) -> Value {
        json!({
            "agent_id": agent_id,
            "display_name": agent_id,
            "provider_type": "claude_code",
            "built_in": built_in,
            "enabled": true,
        })
    }

    #[test]
    fn default_markers_cover_raw_inactive_acting_and_auto_states() {
        let enabled_raw = agent_value("reviewer", false);
        assert_eq!(
            agent_default_marker(&enabled_raw, Some("reviewer"), Some("reviewer")),
            Some("Default")
        );

        let mut disabled_raw = enabled_raw.clone();
        disabled_raw["enabled"] = Value::Bool(false);
        assert_eq!(
            agent_default_marker(&disabled_raw, Some("reviewer"), Some("codex")),
            Some("Default (inactive)")
        );
        let acting = agent_value("codex", true);
        assert_eq!(
            agent_default_marker(&acting, Some("reviewer"), Some("codex")),
            Some("Acting default")
        );
        let automatic = agent_value("claude", true);
        assert_eq!(
            agent_default_marker(&automatic, None, Some("claude")),
            Some("Default (auto)")
        );
        assert_eq!(
            agent_default_marker(&automatic, None, None),
            None,
            "all-disabled catalogs have no default marker"
        );
    }

    #[test]
    fn sort_agents_builtin_first_groups_builtins_then_alphabetical() {
        let mut agents = vec![
            agent_value("novelist", false),
            agent_value("codex", true),
            agent_value("gary", false),
            agent_value("claude", true),
            agent_value("antigravity", true),
        ];

        sort_agents_builtin_first(&mut agents);

        let order: Vec<&str> = agents
            .iter()
            .map(|a| a["agent_id"].as_str().unwrap())
            .collect();
        assert_eq!(
            order,
            vec!["antigravity", "claude", "codex", "gary", "novelist"]
        );
    }

    #[test]
    fn sort_agents_builtin_first_treats_missing_flag_as_custom() {
        let mut agents = vec![
            json!({ "agent_id": "zeta" }),
            agent_value("alpha", true),
            json!({ "agent_id": "alpha-custom" }),
        ];

        sort_agents_builtin_first(&mut agents);

        let order: Vec<&str> = agents
            .iter()
            .map(|a| a["agent_id"].as_str().unwrap())
            .collect();
        assert_eq!(order, vec!["alpha", "alpha-custom", "zeta"]);
    }

    #[test]
    fn sort_agents_builtin_first_handles_empty_slice() {
        let mut agents: Vec<Value> = Vec::new();
        sort_agents_builtin_first(&mut agents);
        assert!(agents.is_empty());
    }

    #[test]
    fn decorate_agent_list_json_adds_kind_and_sorts_in_place() {
        let payload = json!({
            "agents": [
                { "agent_id": "novelist", "display_name": "Novelist", "built_in": false, "enabled": false, "model": "gpt-5" },
                { "agent_id": "codex", "display_name": "Codex", "built_in": true, "enabled": true },
                { "agent_id": "gary", "display_name": "Gary", "built_in": false, "enabled": true },
                { "agent_id": "claude", "display_name": "Claude", "built_in": true, "enabled": true },
            ],
            "default_agent_id": "novelist",
            "effective_default_agent_id": "claude",
        });

        let decorated = decorate_agent_list_json(payload);

        let agents = decorated["agents"].as_array().expect("agents array");
        let order: Vec<&str> = agents
            .iter()
            .map(|a| a["agent_id"].as_str().unwrap())
            .collect();
        assert_eq!(order, vec!["claude", "codex", "gary", "novelist"]);

        assert_eq!(agents[0]["kind"], "builtin");
        assert_eq!(agents[1]["kind"], "builtin");
        assert_eq!(agents[2]["kind"], "custom");
        assert_eq!(agents[3]["kind"], "custom");

        // Original fields survive untouched.
        assert_eq!(agents[0]["display_name"], "Claude");
        assert_eq!(agents[0]["built_in"], true);
        assert_eq!(agents[3]["model"], "gpt-5");
        assert_eq!(agents[3]["built_in"], false);
        assert_eq!(agents[3]["enabled"], false);
        assert_eq!(decorated["default_agent_id"], "novelist");
        assert_eq!(decorated["effective_default_agent_id"], "claude");
    }

    #[test]
    fn decorate_agent_list_json_preserves_top_level_shape_when_agents_missing() {
        let payload = json!({ "other": "value" });
        let decorated = decorate_agent_list_json(payload);
        assert_eq!(decorated, json!({ "other": "value" }));
    }

    #[test]
    fn decorate_agent_list_json_handles_empty_array() {
        let payload = json!({ "agents": [] });
        let decorated = decorate_agent_list_json(payload);
        assert_eq!(decorated, json!({ "agents": [] }));
    }

    #[test]
    fn project_agent_json_omits_avatar_data_urls_at_every_depth() {
        assert_eq!(project_agent_json(Value::Null), Value::Null);
        assert_eq!(
            project_agent_json(json!({ "agents": [] })),
            json!({ "agents": [] })
        );

        let payload = json!({
            "agent_id": "test-agent",
            "avatar_data_url": "data:image/png;base64,AAAA",
            "avatar_url": "https://example.test/avatar.png",
            "nested": {
                "avatar_data_url": null,
                "avatar_data_url_metadata": "preserve-me",
            },
            "agents": [
                {
                    "agent_id": "nested-agent",
                    "avatar_data_url": "data:image/jpeg;base64,BBBB",
                    "system_prompt": "Keep this prompt.",
                }
            ],
        });

        let projected = project_agent_json(payload);

        assert_eq!(projected["agent_id"], "test-agent");
        assert!(projected.get("avatar_data_url").is_none());
        assert_eq!(projected["avatar_url"], "https://example.test/avatar.png");
        assert!(projected["nested"].get("avatar_data_url").is_none());
        assert_eq!(
            projected["nested"]["avatar_data_url_metadata"],
            "preserve-me"
        );
        assert!(projected["agents"][0].get("avatar_data_url").is_none());
        assert_eq!(projected["agents"][0]["system_prompt"], "Keep this prompt.");
    }
}
