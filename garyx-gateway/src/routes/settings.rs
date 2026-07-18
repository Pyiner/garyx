//! Settings update/reload, channel plugin listing, account validation, auth flows.

use super::*;
use crate::server::AppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use garyx_channels::builtin_catalog::builtin_channel_descriptor;
use garyx_models::config_loader::{
    ConfigLoadOptions, ConfigWriteOptions, load_config, write_config_value_atomic,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Shared state for restart cooldown
// ---------------------------------------------------------------------------

/// Deep-merge two JSON values.  For objects, keys from `overlay` override
/// keys in `base`; keys only in `base` are preserved.  All other types are
/// replaced outright by `overlay`.
pub(super) fn deep_merge_json(base: Value, overlay: Value) -> Value {
    match (base, overlay) {
        (Value::Object(mut base_map), Value::Object(overlay_map)) => {
            for (key, overlay_val) in overlay_map {
                let merged = if let Some(base_val) = base_map.remove(&key) {
                    deep_merge_json(base_val, overlay_val)
                } else {
                    overlay_val
                };
                base_map.insert(key, merged);
            }
            Value::Object(base_map)
        }
        // Non-object overlay replaces base entirely (arrays, scalars, null).
        (_base, overlay) => overlay,
    }
}

pub(super) fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

pub(super) fn collect_channel_account_config_errors(
    body: &Value,
    channel_schemas: &HashMap<String, Value>,
    scoped_accounts: Option<&HashSet<(String, String)>>,
) -> Vec<String> {
    let mut errors = Vec::new();
    let Some(channels) = body.get("channels").and_then(Value::as_object) else {
        return errors;
    };

    for (channel_id, channel_value) in channels {
        if channel_id == "api" {
            continue;
        }

        let Some(accounts) = channel_value.get("accounts").and_then(Value::as_object) else {
            continue;
        };

        for (account_id, account_value) in accounts {
            if let Some(scoped_accounts) = scoped_accounts {
                let key = (channel_id.clone(), account_id.clone());
                if !scoped_accounts.contains(&key) {
                    continue;
                }
            }
            let Some(account) = account_value.as_object() else {
                continue;
            };
            let path = format!("$.channels.{channel_id}.accounts.{account_id}.config");
            match account.get("config") {
                None => errors.push(format!("{path} is required for channel accounts")),
                Some(Value::Object(config)) => {
                    if let Some(schema) = channel_schemas.get(channel_id) {
                        collect_required_account_config_errors(
                            channel_id,
                            &path,
                            config,
                            schema,
                            &mut errors,
                        );
                    }
                }
                Some(value) => errors.push(format!(
                    "{path} must be a JSON object, got {}",
                    json_type_name(value)
                )),
            }
        }
    }

    errors
}

pub(super) fn collect_touched_channel_accounts(body: &Value) -> HashSet<(String, String)> {
    let mut accounts = HashSet::new();
    let Some(channels) = body.get("channels").and_then(Value::as_object) else {
        return accounts;
    };

    for (channel_id, channel_value) in channels {
        if channel_id == "api" {
            continue;
        }

        let Some(channel_accounts) = channel_value.get("accounts").and_then(Value::as_object)
        else {
            continue;
        };

        for account_id in channel_accounts.keys() {
            accounts.insert((channel_id.clone(), account_id.clone()));
        }
    }

    accounts
}

pub(super) fn collect_required_account_config_errors(
    channel_id: &str,
    config_path: &str,
    config: &serde_json::Map<String, Value>,
    schema: &Value,
    errors: &mut Vec<String>,
) {
    let Some(required) = schema.get("required").and_then(Value::as_array) else {
        return;
    };
    let properties = schema.get("properties").and_then(Value::as_object);

    for required_field in required.iter().filter_map(Value::as_str) {
        let field_path = format!("{config_path}.{required_field}");
        let field_schema = properties.and_then(|props| props.get(required_field));
        match config.get(required_field) {
            None => errors.push(format!("{field_path} is required by channel schema")),
            Some(Value::Null) => errors.push(format!("{field_path} must not be null")),
            Some(Value::String(value))
                if required_string_field_rejects_blank(channel_id, field_schema)
                    && value.trim().is_empty() =>
            {
                errors.push(format!("{field_path} must not be blank"));
            }
            Some(_) => {}
        }
    }
}

pub(super) fn required_string_field_rejects_blank(
    channel_id: &str,
    field_schema: Option<&Value>,
) -> bool {
    if matches!(channel_id, "telegram" | "feishu" | "weixin") {
        return true;
    }

    matches!(
        field_schema.and_then(|schema| schema.get("type")),
        Some(Value::String(kind)) if kind == "string"
    )
}

pub(super) fn builtin_channel_account_schemas() -> HashMap<String, Value> {
    ["telegram", "feishu", "weixin"]
        .into_iter()
        .filter_map(|plugin_id| {
            builtin_channel_descriptor(plugin_id)
                .map(|descriptor| (plugin_id.to_owned(), descriptor.schema()))
        })
        .collect()
}

#[derive(Debug, Default, Deserialize)]
pub struct SettingsUpdateQuery {
    /// When false, the incoming body fully replaces the stored config instead
    /// of being deep-merged onto it. Needed so callers sending the full doc
    /// (desktop app) can actually delete keys (e.g. a channel account);
    /// additive merge can never express deletion. Defaults to true to stay
    /// safe for partial-payload callers.
    #[serde(default)]
    pub merge: Option<bool>,
}

/// PUT /api/settings - validate, persist, and apply configuration.
pub async fn settings_update(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SettingsUpdateQuery>,
    Json(mut body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    // Validate that the body is a JSON object
    if !body.is_object() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "errors": ["request body must be a JSON object"],
            })),
        );
    }

    // Serialize settings updates to prevent TOCTOU races between concurrent
    // PUT requests (read snapshot → modify → persist → apply).
    let _settings_guard = state.ops.settings_mutex.lock().await;

    let merge_settings = params.merge.unwrap_or(true);
    let mut patch_body = body.clone();
    let existing_config = serde_json::to_value(state.config_snapshot()).unwrap_or_default();

    // Deep-merge by default: prevents partial payloads (e.g. `{"commands":[]}`)
    // from wiping unrelated sections. Callers sending the full document opt
    // out with `?merge=false` so deletions (e.g. removing a channel account)
    // actually take effect — additive merge has no delete semantics.
    if merge_settings {
        body = deep_merge_json(existing_config.clone(), body);
    }

    garyx_models::config_loader::strip_redundant_config_fields(&mut body);
    garyx_models::config_loader::strip_redundant_config_fields(&mut patch_body);

    let mut channel_schemas = builtin_channel_account_schemas();
    {
        let manager = state.channel_plugin_manager();
        let guard = manager.lock().await;
        for entry in guard.subprocess_plugin_catalog() {
            channel_schemas.insert(entry.id, entry.schema);
        }
    }

    let touched_channel_accounts = if merge_settings {
        Some(collect_touched_channel_accounts(&patch_body))
    } else {
        None
    };
    let account_config_errors = collect_channel_account_config_errors(
        &body,
        &channel_schemas,
        touched_channel_accounts.as_ref(),
    );
    if !account_config_errors.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "errors": account_config_errors,
            })),
        );
    }

    // Attempt to deserialize as GaryxConfig for validation.
    let config = match serde_json::from_value::<garyx_models::config::GaryxConfig>(body.clone()) {
        Ok(config) => config,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "ok": false,
                    "errors": [format!("invalid configuration: {e}")],
                })),
            );
        }
    };
    // Strict unknown-field validation: compare user input with normalized schema output.
    let normalized = match serde_json::to_value(&config) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "ok": false,
                    "errors": [format!("failed to normalize config for validation: {e}")],
                })),
            );
        }
    };
    let mut unknown_fields = Vec::new();
    let unknown_field_input = if merge_settings { &patch_body } else { &body };
    collect_unknown_fields("$", unknown_field_input, &normalized, &mut unknown_fields);
    if !unknown_fields.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "errors": unknown_fields,
            })),
        );
    }

    let mcp_servers = config.mcp_servers.clone();
    body = normalized.clone();

    // Apply runtime config FIRST so we can detect errors before persisting.
    // This prevents writing a broken config to disk that would survive restarts.
    if let Err(error) = state.apply_runtime_config(config).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "ok": false,
                "errors": [format!("failed to apply runtime config: {error}")],
            })),
        );
    }

    // Persist to disk only after successful runtime apply.
    if let Some(path) = state.ops.config_path.clone() {
        let body = body.clone();
        let write_result = tokio::task::spawn_blocking(move || {
            let write_opts = ConfigWriteOptions {
                backup_keep: 3,
                mode: Some(0o600),
            };
            write_config_value_atomic(&path, &body, &write_opts)
        })
        .await;
        match write_result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "ok": false,
                        "errors": [format!("failed to persist config file: {e}")],
                    })),
                );
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({
                        "ok": false,
                        "errors": [format!("failed to persist config file: {e}")],
                    })),
                );
            }
        }
    }

    let mut warnings: Vec<String> = Vec::new();
    if let Err(error) = crate::mcp_config::sync_external_configs_from_servers(&mcp_servers).await {
        tracing::warn!("MCP external config sync failed (non-fatal): {error}");
        warnings.push(format!("MCP config sync: {error}"));
    }

    let mut result = json!({
        "ok": true,
        "message": "settings validated, persisted, and applied",
    });
    if !warnings.is_empty() {
        result["warnings"] = json!(warnings);
    }

    (StatusCode::OK, Json(result))
}

/// POST /api/settings/reload - reload config from disk and apply it.
pub async fn settings_reload(State(state): State<Arc<AppState>>) -> (StatusCode, Json<Value>) {
    let _settings_guard = state.ops.settings_mutex.lock().await;

    let Some(path) = state.ops.config_path.clone() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "errors": ["runtime config path is unavailable"],
            })),
        );
    };

    let loaded = match tokio::task::spawn_blocking({
        let path = path.clone();
        move || {
            load_config(
                &path,
                &ConfigLoadOptions {
                    default_path: path.clone(),
                    runtime_overrides: Default::default(),
                },
            )
        }
    })
    .await
    {
        Ok(Ok(loaded)) => loaded,
        Ok(Err(error)) => {
            let errors = if error.diagnostics.errors.is_empty() {
                vec![error.to_string()]
            } else {
                error
                    .diagnostics
                    .errors
                    .iter()
                    .map(|item| item.message.clone())
                    .collect::<Vec<_>>()
            };
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "ok": false,
                    "errors": errors,
                    "config_path": path.display().to_string(),
                })),
            );
        }
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "ok": false,
                    "errors": [format!("failed to load config file: {error}")],
                    "config_path": path.display().to_string(),
                })),
            );
        }
    };

    if let Err(error) = state.apply_runtime_config(loaded.config).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "ok": false,
                "errors": [format!("failed to apply runtime config: {error}")],
                "config_path": loaded.path.display().to_string(),
            })),
        );
    }

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "message": "config reloaded",
            "config_path": loaded.path.display().to_string(),
            "warnings": loaded
                .diagnostics
                .warnings
                .iter()
                .map(|item| item.message.clone())
                .collect::<Vec<_>>(),
        })),
    )
}

// ---------------------------------------------------------------------------
// POST /api/restart
// ---------------------------------------------------------------------------

/// POST /api/restart - restart the service with auth and cooldown protection.
/// `GET /api/channels/plugins` — list of every channel the host
/// knows about (built-in AND subprocess-plugin), with the full
/// metadata the desktop UI needs to render a schema-driven account
/// configuration form (§11).
///
/// Returns `[{ id, display_name, version, description, state,
/// last_error?, capabilities, schema, auth_flows, accounts[] }]`.
/// Built-in channels (telegram / feishu / weixin) are synthesized
/// from the live `ChannelsConfig` via [`crate::channel_catalog`];
/// subprocess plugins come from
/// [`garyx_channels::ChannelPluginManager::subprocess_plugin_catalog`].
/// The UI treats both identically — no hardcoded per-channel
/// branching.
pub async fn list_channel_plugins(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let manager = state.channel_plugin_manager();
    let subprocess = manager.lock().await.subprocess_plugin_catalog();
    let config = state.config_snapshot();
    let builtin = crate::channel_catalog::builtin_channel_catalog(&config.channels);

    // Built-in entries come first (stable display order the UI can
    // rely on without a secondary sort). Subprocess plugins append
    // in discovery order. A plugin id colliding with a built-in name
    // is prevented upstream by `ChannelDispatcherImpl::register_plugin`.
    let mut combined = builtin;
    combined.extend(subprocess);
    for entry in &mut combined {
        entry.project_account_configs_through_schema();
    }

    Json(json!({
        "ok": true,
        "plugins": combined,
    }))
}

/// Body of `POST /api/channels/plugins/:id/auth_flow/start`.
///
/// `form_state` carries whatever the user has typed into the
/// JSON-Schema form so far — the plugin picks the fields it needs,
/// applies its own defaults for the rest, and kicks off its
/// internal state machine. Sending `{}` is valid: the plugin runs
/// with full defaults (this is how a pristine "Click to auto-login"
/// button works).
#[derive(Debug, Clone, Deserialize)]
pub struct AuthFlowStartBody {
    #[serde(default)]
    pub form_state: Value,
}

/// Body of `POST /api/channels/plugins/:id/auth_flow/poll`.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthFlowPollBody {
    pub session_id: String,
}

/// Body of `POST /api/channels/plugins/:id/validate_account`.
#[derive(Debug, Clone, Deserialize)]
pub struct ChannelAccountValidationBody {
    pub account_id: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub config: Value,
}

pub(super) fn default_enabled() -> bool {
    true
}

/// `POST /api/channels/plugins/{id}/validate_account`.
///
/// Validates one account payload before the desktop persists it into
/// `~/.garyx/garyx.json`. Built-ins perform real provider probes when
/// safe; plugins without a validator return `validated=false` so callers
/// can distinguish a real check from a deliberate skip.
pub async fn channel_account_validate(
    State(state): State<Arc<AppState>>,
    Path(plugin_id): Path<String>,
    Json(body): Json<ChannelAccountValidationBody>,
) -> impl IntoResponse {
    let account_id = body.account_id.trim();
    if account_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "reason": "invalid_account",
                "message": "account_id is required",
            })),
        );
    }

    let plugin = {
        let manager = state.channel_plugin_manager();
        let guard = manager.lock().await;
        guard.plugin(&plugin_id)
    };
    let Some(plugin) = plugin else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "ok": false,
                "reason": "unknown_plugin",
                "message": format!("plugin '{plugin_id}' is not registered"),
            })),
        );
    };

    let account = garyx_channels::plugin_host::AccountDescriptor {
        id: account_id.to_owned(),
        enabled: body.enabled,
        config: body.config,
    };
    match plugin.validate_account_config(account).await {
        Ok(result) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "validated": result.validated,
                "message": result.message,
            })),
        ),
        Err(message) => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "ok": false,
                "reason": "validation_failed",
                "message": message,
            })),
        ),
    }
}

/// `POST /api/channels/plugins/{id}/auth_flow/start`.
///
/// Starts an auto-login session against the plugin identified by
/// `{id}` (canonical id or alias — feishu accepts `lark` etc.). On
/// success returns the plugin's `AuthSession` with a rendered
/// display list, session id, TTL, and poll cadence; the desktop
/// client then polls `/poll` at `poll_interval_secs` until Confirmed
/// or Failed.
///
/// Returns 404 when the plugin is unknown or doesn't support an
/// auto-login path (its `config_methods` didn't include
/// `AutoLogin`). Returns 400 on transport / protocol failures the
/// executor couldn't recover from so the UI stops polling.
pub async fn channel_auth_flow_start(
    State(state): State<Arc<AppState>>,
    Path(plugin_id): Path<String>,
    Json(body): Json<AuthFlowStartBody>,
) -> impl IntoResponse {
    let executor = {
        let manager = state.channel_plugin_manager();
        let guard = manager.lock().await;
        guard.auth_flow_executor(&plugin_id)
    };
    let Some(executor) = executor else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "ok": false,
                "reason": "no_auth_flow",
                "message": format!(
                    "plugin '{plugin_id}' does not expose an auto-login flow"
                ),
            })),
        );
    };

    match executor.start(body.form_state).await {
        Ok(session) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "session_id": session.session_id,
                "display": session.display,
                "expires_in_secs": session.expires_in_secs,
                "poll_interval_secs": session.poll_interval_secs,
            })),
        ),
        Err(err) => (
            auth_flow_err_status(&err),
            Json(json!({
                "ok": false,
                "reason": "start_failed",
                "message": err.to_string(),
            })),
        ),
    }
}

/// `POST /api/channels/plugins/{id}/auth_flow/poll`.
///
/// Advances the named session by one tick. Returns the executor's
/// 3-state outcome verbatim (`pending` / `confirmed` / `failed`).
/// Unknown session_id surfaces as 404.
pub async fn channel_auth_flow_poll(
    State(state): State<Arc<AppState>>,
    Path(plugin_id): Path<String>,
    Json(body): Json<AuthFlowPollBody>,
) -> impl IntoResponse {
    let executor = {
        let manager = state.channel_plugin_manager();
        let guard = manager.lock().await;
        guard.auth_flow_executor(&plugin_id)
    };
    let Some(executor) = executor else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "ok": false,
                "reason": "no_auth_flow",
                "message": format!(
                    "plugin '{plugin_id}' does not expose an auto-login flow"
                ),
            })),
        );
    };

    match executor.poll(&body.session_id).await {
        Ok(result) => (
            StatusCode::OK,
            Json(
                serde_json::to_value(&result)
                    .map(|mut v| {
                        if let Value::Object(map) = &mut v {
                            map.insert("ok".into(), Value::Bool(true));
                        }
                        v
                    })
                    .unwrap_or_else(|_| json!({ "ok": true })),
            ),
        ),
        Err(err) => (
            auth_flow_err_status(&err),
            Json(json!({
                "ok": false,
                "reason": "poll_failed",
                "message": err.to_string(),
            })),
        ),
    }
}

/// Map an `AuthFlowError` to the right HTTP status so the desktop
/// can tell "I sent a bad session id" (404) from "the plugin died"
/// (502) from "the plugin's reply didn't parse" (500). Kept local
/// so the two handlers share exactly one mapping.
pub(super) fn auth_flow_err_status(err: &garyx_channels::auth_flow::AuthFlowError) -> StatusCode {
    use garyx_channels::auth_flow::AuthFlowError;
    match err {
        AuthFlowError::UnknownSession(_) => StatusCode::NOT_FOUND,
        AuthFlowError::InvalidArgs(_) => StatusCode::BAD_REQUEST,
        AuthFlowError::Transport(_) => StatusCode::BAD_GATEWAY,
        AuthFlowError::Protocol(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
