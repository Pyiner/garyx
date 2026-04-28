use std::collections::HashSet;
use std::sync::Arc;

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use garyx_models::command_catalog::{
    CommandCatalogOptions, is_valid_shortcut_command_name, normalize_shortcut_command_name,
};
use garyx_models::config::SlashCommand;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::mcp_config::persist_and_apply_config;
use crate::server::AppState;

#[derive(Debug, Deserialize)]
pub struct UpsertShortcutBody {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub prompt: Option<String>,
}

fn normalize_shortcut(body: UpsertShortcutBody) -> Result<SlashCommand, (StatusCode, Json<Value>)> {
    let name = normalize_shortcut_command_name(&body.name);
    if !is_valid_shortcut_command_name(&name) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "shortcut name must match [a-z0-9_] and be at most 32 characters",
            })),
        ));
    }
    if garyx_router::reserved_command_names().contains(name.as_str()) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "reserved_command_name",
                "message": format!("command '/{}' is built in and cannot be redefined", name),
            })),
        ));
    }

    let description = body.description.trim().to_owned();
    if description.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "description is required",
            })),
        ));
    }
    if description.chars().count() > 256 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "description must be at most 256 characters",
            })),
        ));
    }

    let prompt = body
        .prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    if prompt.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "prompt is required",
            })),
        ));
    }

    Ok(SlashCommand {
        name,
        description,
        prompt,
        skill_id: None,
    })
}

pub async fn list_commands(
    State(state): State<Arc<AppState>>,
    Query(options): Query<CommandCatalogOptions>,
) -> impl IntoResponse {
    let config = state.config_snapshot();
    let catalog = garyx_router::command_catalog_for_config(&config, options);
    (StatusCode::OK, Json(catalog))
}

fn shortcut_value(command: &SlashCommand) -> Value {
    json!({
        "name": command.name.clone(),
        "description": command.description.clone(),
        "prompt": command.prompt.clone(),
    })
}

pub async fn list_shortcuts(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.config_snapshot();
    let reserved = garyx_router::reserved_command_names();
    let mut seen = HashSet::new();
    let commands = config
        .commands
        .iter()
        .filter_map(|command| {
            let name = normalize_shortcut_command_name(&command.name);
            if !is_valid_shortcut_command_name(&name)
                || reserved.contains(name.as_str())
                || !seen.insert(name.clone())
            {
                return None;
            }
            let prompt = command
                .prompt
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())?;
            Some(SlashCommand {
                name,
                description: command.description.trim().to_owned(),
                prompt: Some(prompt.to_owned()),
                skill_id: None,
            })
        })
        .map(|command| shortcut_value(&command))
        .collect::<Vec<_>>();
    (
        StatusCode::OK,
        Json(json!({
            "commands": commands,
        })),
    )
}

pub async fn create_shortcut(
    State(state): State<Arc<AppState>>,
    Json(body): Json<UpsertShortcutBody>,
) -> impl IntoResponse {
    let command = match normalize_shortcut(body) {
        Ok(command) => command,
        Err(error) => return error.into_response(),
    };

    let mut config = (*state.config_snapshot()).clone();
    if config
        .commands
        .iter()
        .any(|existing| normalize_shortcut_command_name(&existing.name) == command.name)
    {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!("command '/{}' already exists", command.name),
            })),
        )
            .into_response();
    }

    config.commands.push(command.clone());
    match persist_and_apply_config(&state, &config).await {
        Ok(()) => (StatusCode::CREATED, Json(shortcut_value(&command))).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        )
            .into_response(),
    }
}

pub async fn update_shortcut(
    State(state): State<Arc<AppState>>,
    AxumPath(current_name): AxumPath<String>,
    Json(body): Json<UpsertShortcutBody>,
) -> impl IntoResponse {
    let command = match normalize_shortcut(body) {
        Ok(command) => command,
        Err(error) => return error.into_response(),
    };

    let current_name = normalize_shortcut_command_name(&current_name);
    let mut config = (*state.config_snapshot()).clone();
    let Some(index) = config
        .commands
        .iter()
        .position(|existing| normalize_shortcut_command_name(&existing.name) == current_name)
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": format!("command '/{}' not found", current_name),
            })),
        )
            .into_response();
    };

    if command.name != current_name
        && config
            .commands
            .iter()
            .any(|existing| normalize_shortcut_command_name(&existing.name) == command.name)
    {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!("command '/{}' already exists", command.name),
            })),
        )
            .into_response();
    }

    config.commands[index] = command.clone();
    match persist_and_apply_config(&state, &config).await {
        Ok(()) => (StatusCode::OK, Json(shortcut_value(&command))).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        )
            .into_response(),
    }
}

pub async fn delete_shortcut(
    State(state): State<Arc<AppState>>,
    AxumPath(name): AxumPath<String>,
) -> impl IntoResponse {
    let name = normalize_shortcut_command_name(&name);
    let mut config = (*state.config_snapshot()).clone();
    let Some(index) = config
        .commands
        .iter()
        .position(|command| normalize_shortcut_command_name(&command.name) == name)
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": format!("command '/{}' not found", name),
            })),
        )
            .into_response();
    };

    config.commands.remove(index);
    match persist_and_apply_config(&state, &config).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({
                "deleted": true,
                "name": name,
            })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests;
