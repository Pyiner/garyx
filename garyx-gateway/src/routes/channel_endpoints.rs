//! Channel-endpoint bind/detach/list handlers.

use super::*;

#[derive(Debug, Clone)]
pub(crate) struct ChannelEndpointBindResult {
    pub(crate) thread_id: String,
    pub(crate) previous_thread_id: Option<String>,
    pub(crate) endpoint_key: String,
    pub(crate) binding: ChannelBinding,
}

#[derive(Debug, Clone)]
pub(crate) struct ChannelEndpointDetachResult {
    pub(crate) previous_thread_id: Option<String>,
    pub(crate) endpoint_key: String,
    pub(crate) binding: Option<ChannelBinding>,
}

#[derive(Debug, Clone)]
pub(crate) struct ChannelEndpointMutationError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
}

impl ChannelEndpointMutationError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

pub(crate) fn binding_delivery_thread_id(binding_key: &str, chat_id: &str) -> Option<String> {
    let binding_key = binding_key.trim();
    let chat_id = chat_id.trim();
    if binding_key.is_empty() || binding_key == chat_id {
        None
    } else {
        Some(binding_key.to_owned())
    }
}

pub(super) fn normalize_endpoint_lookup_key(endpoint_key: &str) -> String {
    let trimmed = endpoint_key.trim();
    let parts: Vec<&str> = trimmed.split("::").collect();
    if parts.len() >= 4 {
        format!("{}::{}::{}", parts[0], parts[1], parts[parts.len() - 1])
    } else {
        trimmed.to_owned()
    }
}

pub(super) fn endpoint_key_matches(candidate: &str, requested: &str) -> bool {
    let requested = requested.trim();
    candidate == requested || candidate == normalize_endpoint_lookup_key(requested)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BindChannelEndpointBody {
    pub endpoint_key: String,
    pub thread_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetachChannelEndpointBody {
    pub endpoint_key: String,
}

/// Resolve an existing thread id at a request boundary: `Ok(None)` is a
/// plain 404, while a store backend failure surfaces as `Err` so handlers
/// answer 500 instead of a misleading not-found.
pub(super) async fn ensure_existing_thread_id(
    state: &Arc<AppState>,
    key: &str,
) -> Result<Option<String>, (StatusCode, Json<Value>)> {
    let trimmed = key.trim();
    if trimmed.is_empty() || !is_thread_key(trimmed) {
        return Ok(None);
    }
    match state.threads.thread_store.exists(trimmed).await {
        Ok(true) => Ok(Some(trimmed.to_owned())),
        Ok(false) => Ok(None),
        Err(error) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        )),
    }
}

pub(super) fn binding_from_known_endpoint(endpoint: &KnownChannelEndpoint) -> ChannelBinding {
    ChannelBinding {
        channel: endpoint.channel.clone(),
        account_id: endpoint.account_id.clone(),
        binding_key: endpoint.binding_key.clone(),
        chat_id: endpoint.chat_id.clone(),
        delivery_target_type: endpoint.delivery_target_type.clone(),
        delivery_target_id: endpoint.delivery_target_id.clone(),
        display_label: endpoint.display_label.clone(),
        last_inbound_at: endpoint.last_inbound_at.clone(),
        last_delivery_at: endpoint.last_delivery_at.clone(),
    }
}

pub(crate) async fn bind_channel_endpoint_key_to_thread(
    state: &Arc<AppState>,
    endpoint_key: &str,
    thread_id: &str,
) -> Result<ChannelEndpointBindResult, ChannelEndpointMutationError> {
    let requested_endpoint_key = normalize_endpoint_lookup_key(endpoint_key);
    let Some(thread_id) =
        ensure_existing_thread_id(state, thread_id)
            .await
            .map_err(|(status, body)| {
                ChannelEndpointMutationError::new(
                    status,
                    body.0["error"].as_str().unwrap_or("thread store failed"),
                )
            })?
    else {
        return Err(ChannelEndpointMutationError::new(
            StatusCode::NOT_FOUND,
            "target thread not found",
        ));
    };

    let known_endpoint = state
        .cached_channel_endpoints()
        .await
        .map_err(|error| {
            ChannelEndpointMutationError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("thread store error: {error}"),
            )
        })?
        .into_iter()
        .find(|endpoint| endpoint_key_matches(&endpoint.endpoint_key, &requested_endpoint_key));

    let binding = if let Some(endpoint) = known_endpoint.as_ref() {
        binding_from_known_endpoint(endpoint)
    } else if let Some(binding) = resolve_main_endpoint_by_key(state, &requested_endpoint_key)
        .await
        .map_err(|error| {
            ChannelEndpointMutationError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("thread store error: {error}"),
            )
        })?
        .map(|endpoint| endpoint.to_binding())
    {
        binding
    } else {
        return Err(ChannelEndpointMutationError::new(
            StatusCode::NOT_FOUND,
            "endpoint not found",
        ));
    };

    let bind_result = {
        let mut router = state.threads.router.lock().await;
        router
            .bind_endpoint_runtime(&thread_id, binding.clone())
            .await
    };

    match bind_result {
        Ok(mutation) => {
            // bind_endpoint_runtime upserts the endpoint index entry itself;
            // no full index rebuild is needed here.
            state.invalidate_gateway_sync_caches().await;
            Ok(ChannelEndpointBindResult {
                thread_id,
                previous_thread_id: mutation.previous_thread_id,
                endpoint_key: requested_endpoint_key,
                binding: mutation.binding,
            })
        }
        Err(error) => Err(endpoint_mutation_error_response(error)),
    }
}

/// Structured status mapping for endpoint binding mutations: a storage
/// outage inside the mutator surfaces as 500, never as a client error
/// (#TASK-2147).
pub(super) fn endpoint_mutation_error_response(
    error: garyx_router::EndpointBindingMutationError,
) -> ChannelEndpointMutationError {
    use garyx_router::EndpointBindingMutationError as MutationError;
    let status = match &error {
        MutationError::TargetNotFound(_) => StatusCode::NOT_FOUND,
        MutationError::TargetArchived(_) => StatusCode::GONE,
        MutationError::ThreadLifecycleInProgress(_) => StatusCode::CONFLICT,
        MutationError::Incompatible(_) => StatusCode::BAD_REQUEST,
        MutationError::Unavailable
        | MutationError::Projection(_)
        | MutationError::PreviousOwnerUnavailable(_)
        | MutationError::WriteFailed { .. } => StatusCode::INTERNAL_SERVER_ERROR,
    };
    ChannelEndpointMutationError::new(status, error.to_string())
}

pub(crate) async fn detach_channel_endpoint_key(
    state: &Arc<AppState>,
    endpoint_key: &str,
) -> Result<ChannelEndpointDetachResult, ChannelEndpointMutationError> {
    let requested_endpoint_key = normalize_endpoint_lookup_key(endpoint_key);
    let mutation = {
        let mut router = state.threads.router.lock().await;
        router
            .detach_endpoint_runtime(&requested_endpoint_key)
            .await
    };
    match mutation {
        Ok(mutation) => {
            let previous_thread_id = mutation.previous_thread_id;
            state.invalidate_channel_endpoint_cache().await;
            if let (Some(thread_id), Some(binding)) =
                (previous_thread_id.as_deref(), mutation.binding.as_ref())
            {
                let delivery_thread_id =
                    binding_delivery_thread_id(&binding.binding_key, &binding.chat_id);
                let mut router = state.threads.router.lock().await;
                router
                    .clear_last_delivery_for_chat_with_known_thread_persistence(
                        thread_id,
                        &binding.channel,
                        &binding.account_id,
                        &binding.chat_id,
                        delivery_thread_id.as_deref(),
                    )
                    .await;
            }
            state.invalidate_gateway_sync_caches().await;
            Ok(ChannelEndpointDetachResult {
                previous_thread_id,
                endpoint_key: requested_endpoint_key,
                binding: mutation.binding,
            })
        }
        Err(error) => Err(endpoint_mutation_error_response(error)),
    }
}

/// GET /api/channel-endpoints - list known channel endpoints
pub async fn list_channel_endpoints(
    State(state): State<Arc<AppState>>,
) -> axum::response::Response {
    // A store/projection failure must surface as 500, never as an empty
    // endpoint listing (#TASK-2128) — and never be satisfied from the
    // snapshot cache during a live outage (#TASK-2134).
    match state.channel_endpoints_fresh().await {
        Ok(endpoints) => Json(json!({
            "endpoints": endpoints.iter().map(channel_endpoint_response_value).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(error) => thread_store_error_response(&error).into_response(),
    }
}

/// POST /api/channel-bindings/bind - move endpoint to another thread
pub async fn bind_channel_endpoint(
    State(state): State<Arc<AppState>>,
    Json(body): Json<BindChannelEndpointBody>,
) -> impl IntoResponse {
    match bind_channel_endpoint_key_to_thread(&state, &body.endpoint_key, &body.thread_id).await {
        Ok(result) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "thread_id": result.thread_id,
                "previous_thread_id": result.previous_thread_id,
                "endpoint_key": result.endpoint_key,
            })),
        ),
        Err(error) => (error.status, Json(json!({ "error": error.message }))),
    }
}

/// POST /api/channel-bindings/detach - detach endpoint from current thread
pub async fn detach_channel_endpoint(
    State(state): State<Arc<AppState>>,
    Json(body): Json<DetachChannelEndpointBody>,
) -> impl IntoResponse {
    match detach_channel_endpoint_key(&state, &body.endpoint_key).await {
        Ok(result) => (
            StatusCode::OK,
            Json(json!({
                "ok": result.previous_thread_id.is_some(),
                "previous_thread_id": result.previous_thread_id,
                "endpoint_key": result.endpoint_key,
            })),
        ),
        Err(error) => (error.status, Json(json!({ "error": error.message }))),
    }
}
