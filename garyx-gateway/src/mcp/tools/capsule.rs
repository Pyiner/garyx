use garyx_models::provider::ProviderType;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::super::*;
use crate::capsules::{
    atomic_write_capsule_file, read_capsule_html_path, validate_capsule_html_bytes,
};
use crate::garyx_db::{CapsuleCreateDraft, CapsuleRecord, CapsuleUpdateDraft};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CapsuleThreadSnapshot {
    agent_id: Option<String>,
    provider_type: Option<String>,
}

pub(crate) async fn create(
    server: &GaryMcpServer,
    ctx: RequestContext<RoleServer>,
    params: CapsuleCreateParams,
) -> Result<String, String> {
    let started = Instant::now();
    let run_ctx = RunContext::from_request_context(&ctx);
    let result = create_inner(server, run_ctx, params).await;
    server.record_tool_metric(
        "capsule_create",
        if result.is_ok() { "ok" } else { "error" },
        started.elapsed(),
    );
    result.map(|value| serde_json::to_string(&value).unwrap_or_default())
}

pub(crate) async fn update(
    server: &GaryMcpServer,
    ctx: RequestContext<RoleServer>,
    params: CapsuleUpdateParams,
) -> Result<String, String> {
    let started = Instant::now();
    let run_ctx = RunContext::from_request_context(&ctx);
    let result = update_inner(server, run_ctx, params).await;
    server.record_tool_metric(
        "capsule_update",
        if result.is_ok() { "ok" } else { "error" },
        started.elapsed(),
    );
    result.map(|value| serde_json::to_string(&value).unwrap_or_default())
}

pub(crate) async fn list(
    server: &GaryMcpServer,
    ctx: RequestContext<RoleServer>,
) -> Result<String, String> {
    let started = Instant::now();
    let run_ctx = RunContext::from_request_context(&ctx);
    let result = list_inner(server, run_ctx).await;
    server.record_tool_metric(
        "capsule_list",
        if result.is_ok() { "ok" } else { "error" },
        started.elapsed(),
    );
    result.map(|value| serde_json::to_string(&value).unwrap_or_default())
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) async fn create_inner(
    server: &GaryMcpServer,
    run_ctx: RunContext,
    params: CapsuleCreateParams,
) -> Result<Value, String> {
    let thread_id = required_thread_id(&run_ctx, "capsule_create")?;
    let title = required_title(&params.title)?;
    let description = normalize_optional_text(params.description.as_deref()).unwrap_or_default();
    let html = html_from_params(params.html.as_deref(), params.html_path.as_deref(), true)
        .await?
        .ok_or_else(|| "provide exactly one of html or html_path".to_owned())?;
    let capsule_id = Uuid::now_v7().to_string();
    let snapshot = capsule_thread_snapshot(server, &thread_id).await?;
    atomic_write_capsule_file(&capsule_id, html.as_bytes())
        .await
        .map_err(|error| format!("failed to write capsule HTML: {}", error_message(error)))?;
    let record = server
        .app_state
        .ops
        .garyx_db
        .create_capsule(CapsuleCreateDraft {
            id: capsule_id.clone(),
            title,
            description,
            thread_id: Some(thread_id),
            run_id: normalize_optional_text(run_ctx.run_id.as_deref()),
            agent_id: snapshot.agent_id,
            provider_type: snapshot.provider_type,
            html_sha256: sha256_hex(html.as_bytes()),
            byte_size: html.len() as i64,
        })
        .map_err(|error| error.to_string())?;
    Ok(tool_response("capsule_create", &record))
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) async fn update_inner(
    server: &GaryMcpServer,
    _run_ctx: RunContext,
    params: CapsuleUpdateParams,
) -> Result<Value, String> {
    let capsule_id = crate::capsules::parse_capsule_uuid(&params.capsule_id)
        .map_err(|error| format!("invalid capsule_id: {}", error_message(error)))?;
    let title = params.title.as_deref().map(required_title).transpose()?;
    let description = params
        .description
        .as_deref()
        .map(|value| value.trim().to_owned());
    let has_html = params
        .html
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let has_html_path = params
        .html_path
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    if title.is_none() && description.is_none() && !has_html && !has_html_path {
        return Err(
            "capsule_update requires at least one of title, description, html, or html_path"
                .to_owned(),
        );
    }

    let existing = server
        .app_state
        .ops
        .garyx_db
        .get_capsule(&capsule_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("capsule not found: {capsule_id}"))?;

    let html = html_from_params(params.html.as_deref(), params.html_path.as_deref(), false).await?;
    let (html_sha256, byte_size) = if let Some(html) = html.as_ref() {
        atomic_write_capsule_file(&capsule_id, html.as_bytes())
            .await
            .map_err(|error| format!("failed to write capsule HTML: {}", error_message(error)))?;
        (Some(sha256_hex(html.as_bytes())), Some(html.len() as i64))
    } else {
        (None, None)
    };

    let updated = server
        .app_state
        .ops
        .garyx_db
        .update_capsule(
            &existing.id,
            CapsuleUpdateDraft {
                title,
                description,
                html_sha256,
                byte_size,
            },
        )
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("capsule not found: {capsule_id}"))?;
    Ok(tool_response("capsule_update", &updated))
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) async fn list_inner(
    server: &GaryMcpServer,
    run_ctx: RunContext,
) -> Result<Value, String> {
    let thread_id = required_thread_id(&run_ctx, "capsule_list")?;
    let records = server
        .app_state
        .ops
        .garyx_db
        .list_capsules_for_thread(&thread_id)
        .map_err(|error| error.to_string())?;
    Ok(json!({
        "tool": "capsule_list",
        "status": "ok",
        "thread_id": thread_id,
        "capsules": records.into_iter().map(summary_json).collect::<Vec<_>>(),
    }))
}

async fn capsule_thread_snapshot(
    server: &GaryMcpServer,
    thread_id: &str,
) -> Result<CapsuleThreadSnapshot, String> {
    let thread = server
        .app_state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .ok_or_else(|| format!("thread not found for capsule context: {thread_id}"))?;
    Ok(CapsuleThreadSnapshot {
        agent_id: garyx_router::agent_id_from_value(&thread),
        provider_type: provider_type_from_thread_value(&thread),
    })
}

fn provider_type_from_thread_value(thread: &Value) -> Option<String> {
    thread
        .get("provider_type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            ProviderType::from_slug(value)
                .map(|provider| provider.as_slug().to_owned())
                .unwrap_or_else(|| value.to_owned())
        })
}

async fn html_from_params(
    html: Option<&str>,
    html_path: Option<&str>,
    require_html: bool,
) -> Result<Option<String>, String> {
    let html = html.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(value)
    });
    let html_path = html_path.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    });
    match (html, html_path) {
        (Some(_), Some(_)) => Err("provide exactly one of html or html_path".to_owned()),
        (None, None) if require_html => Err("provide exactly one of html or html_path".to_owned()),
        (None, None) => Ok(None),
        (Some(html), None) => validate_capsule_html_bytes(html.as_bytes())
            .map(Some)
            .map_err(|error| error_message(error)),
        (None, Some(path)) => {
            let bytes = read_capsule_html_path(path).await.map_err(error_message)?;
            validate_capsule_html_bytes(&bytes)
                .map(Some)
                .map_err(error_message)
        }
    }
}

fn required_thread_id(run_ctx: &RunContext, tool: &str) -> Result<String, String> {
    run_ctx
        .thread_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{tool} requires a thread_id in the MCP request context"))
}

fn required_title(title: &str) -> Result<String, String> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err("title must not be empty".to_owned());
    }
    Ok(trimmed.to_owned())
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn tool_response(tool: &str, record: &CapsuleRecord) -> Value {
    let mut value = summary_json(record.clone());
    if let Some(object) = value.as_object_mut() {
        object.insert("tool".to_owned(), Value::String(tool.to_owned()));
        object.insert("status".to_owned(), Value::String("ok".to_owned()));
        object.insert("capsule_id".to_owned(), Value::String(record.id.clone()));
    }
    value
}

fn summary_json(record: CapsuleRecord) -> Value {
    json!({
        "id": record.id,
        "title": record.title,
        "description": record.description,
        "thread_id": record.thread_id,
        "run_id": record.run_id,
        "agent_id": record.agent_id,
        "provider_type": record.provider_type,
        "html_sha256": record.html_sha256,
        "byte_size": record.byte_size,
        "revision": record.revision,
        "created_at": record.created_at,
        "updated_at": record.updated_at,
        "open_url": format!("garyx://capsules/{}", record.id),
        "serve_path": format!("/api/capsules/{}/serve", record.id),
    })
}

fn error_message(error: impl std::fmt::Display) -> String {
    error.to_string()
}
