use std::collections::HashMap;

use super::super::*;
use chrono::Local;
use garyx_router::{
    ChannelBinding, ThreadEnsureOptions, bindings_from_value, list_known_channel_endpoints,
    workspace_dir_from_value,
};
use serde_json::{Value, json};

fn normalized_nonempty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_scope(value: Option<&str>) -> Option<String> {
    normalized_nonempty(value)
}

fn channel_supports_endpoint_binding(channel: &str) -> bool {
    matches!(channel, "telegram" | "feishu" | "weixin")
}

fn current_thread_origin_fields(
    thread_data: &Value,
) -> (Option<String>, Option<String>, Option<String>) {
    (
        normalized_nonempty(thread_data.get("channel").and_then(Value::as_str)),
        normalized_nonempty(thread_data.get("account_id").and_then(Value::as_str)),
        normalized_nonempty(thread_data.get("from_id").and_then(Value::as_str)),
    )
}

fn binding_matches_context(
    binding: &ChannelBinding,
    channel: &str,
    account_id: &str,
    thread_binding_key: &str,
) -> bool {
    binding.channel == channel
        && binding.account_id == account_id
        && binding.binding_key == thread_binding_key
}

fn binding_from_known_endpoint(value: garyx_router::KnownChannelEndpoint) -> ChannelBinding {
    ChannelBinding {
        channel: value.channel,
        account_id: value.account_id,
        binding_key: value.binding_key,
        chat_id: value.chat_id,
        delivery_target_type: value.delivery_target_type,
        delivery_target_id: value.delivery_target_id,
        display_label: value.display_label,
        last_inbound_at: value.last_inbound_at,
        last_delivery_at: value.last_delivery_at,
    }
}

async fn resolve_current_binding(
    server: &GaryMcpServer,
    thread_id: &str,
    run_ctx_channel: Option<&str>,
    run_ctx_account_id: Option<&str>,
    run_ctx_from_id: Option<&str>,
    thread_scope: Option<&str>,
) -> Result<ChannelBinding, String> {
    let store = &server.app_state.threads.thread_store;
    let current_thread = store
        .get(thread_id)
        .await
        .ok_or_else(|| format!("thread not found: {thread_id}"))?;

    let bindings = bindings_from_value(&current_thread);
    if bindings.is_empty() {
        return Err("current thread has no bound channel endpoint".to_owned());
    }

    let (thread_channel, thread_account_id, thread_from_id) =
        current_thread_origin_fields(&current_thread);
    let candidate_channel = normalized_nonempty(run_ctx_channel).or(thread_channel);
    let candidate_account_id = normalized_nonempty(run_ctx_account_id).or(thread_account_id);
    let candidate_binding_key =
        normalize_scope(thread_scope).or(normalized_nonempty(run_ctx_from_id).or(thread_from_id));

    if let (Some(channel), Some(account_id), Some(thread_binding_key)) = (
        candidate_channel.as_deref(),
        candidate_account_id.as_deref(),
        candidate_binding_key.as_deref(),
    ) {
        let mut matching = bindings
            .iter()
            .filter(|binding| {
                binding_matches_context(binding, channel, account_id, thread_binding_key)
            })
            .cloned()
            .collect::<Vec<_>>();
        if matching.len() == 1 {
            return Ok(matching.remove(0));
        }
    }

    if bindings.len() == 1 {
        return Ok(bindings[0].clone());
    }

    let mut matching_known_endpoints = list_known_channel_endpoints(store)
        .await
        .into_iter()
        .filter(|endpoint| endpoint.thread_id.as_deref() == Some(thread_id))
        .collect::<Vec<_>>();
    if matching_known_endpoints.len() == 1 {
        return Ok(binding_from_known_endpoint(
            matching_known_endpoints.remove(0),
        ));
    }

    Err(
        "current thread resolves to multiple channel bindings; rebind_current_channel cannot disambiguate"
            .to_owned(),
    )
}

pub(crate) async fn payload(
    server: &GaryMcpServer,
    run_ctx: RunContext,
    params: RebindCurrentChannelParams,
) -> Result<Value, String> {
    let thread_id = normalized_nonempty(run_ctx.thread_id.as_deref())
        .ok_or("rebind_current_channel requires a thread_id in MCP context")?;

    let requested_agent_id = params.agent_id.trim();
    if requested_agent_id.is_empty() {
        return Err("agent_id is required".to_owned());
    }
    let requested_workspace_dir = params.workspace_dir.trim();
    if requested_workspace_dir.is_empty() {
        return Err("workspace_dir is required".to_owned());
    }

    let thread_scope = normalize_scope(run_ctx.delivery_thread_id.as_deref());
    let binding = resolve_current_binding(
        server,
        &thread_id,
        run_ctx.channel.as_deref(),
        run_ctx.account_id.as_deref(),
        run_ctx.from_id.as_deref(),
        thread_scope.as_deref(),
    )
    .await?;
    let channel = binding.channel.clone();
    let account_id = binding.account_id.clone();
    let from_id = run_ctx
        .from_id
        .clone()
        .or_else(|| normalized_nonempty(Some(&binding.binding_key)))
        .unwrap_or_else(|| binding.chat_id.clone());
    if !channel_supports_endpoint_binding(&channel) {
        return Err(format!(
            "rebind_current_channel is unsupported for channel '{channel}'"
        ));
    }
    let is_group = binding.chat_id.trim() != binding.binding_key.trim();

    let label = Local::now().format("thread-%Y%m%d-%H%M%S").to_string();
    let options = ThreadEnsureOptions {
        label: Some(label.clone()),
        workspace_dir: Some(requested_workspace_dir.to_owned()),
        agent_id: Some(requested_agent_id.to_owned()),
        metadata: HashMap::new(),
        provider_type: None,
        sdk_session_id: None,
        thread_kind: None,
        origin_channel: Some(channel.clone()),
        origin_account_id: Some(account_id.clone()),
        origin_from_id: Some(from_id.clone()),
        is_group: Some(is_group),
    };

    let (new_thread_id, new_thread_data, previous_thread_id) = {
        let mut router = server.app_state.threads.router.lock().await;
        let (new_thread_id, new_thread_data) = router.create_thread_with_options(options).await?;
        let previous_thread_id = router
            .bind_endpoint_runtime(&new_thread_id, binding.clone())
            .await?;
        (new_thread_id, new_thread_data, previous_thread_id)
    };

    Ok(json!({
        "tool": "rebind_current_channel",
        "status": "ok",
        "thread_id": new_thread_id,
        "previous_thread_id": previous_thread_id,
        "current_thread_id": thread_id,
        "channel": channel,
        "account_id": account_id,
        "from_id": from_id,
        "thread_binding_key": binding.binding_key,
        "thread_scope": thread_scope,
        "endpoint_key": binding.endpoint_key(),
        "label": label,
        "requested_agent_id": requested_agent_id,
        "bound_agent_id": new_thread_data.get("agent_id").and_then(Value::as_str),
        "workspace_dir": workspace_dir_from_value(&new_thread_data)
            .unwrap_or_else(|| requested_workspace_dir.to_owned()),
        "provider_type": new_thread_data.get("provider_type").cloned().unwrap_or(Value::Null),
    }))
}

pub(crate) async fn run(
    server: &GaryMcpServer,
    ctx: RequestContext<RoleServer>,
    params: RebindCurrentChannelParams,
) -> Result<String, String> {
    let started = Instant::now();
    let run_ctx = RunContext::from_request_context(&ctx);
    let result = payload(server, run_ctx, params)
        .await
        .map(|value| serde_json::to_string(&value).unwrap_or_default());
    server.record_tool_metric(
        "rebind_current_channel",
        if result.is_ok() { "ok" } else { "error" },
        started.elapsed(),
    );
    result
}
