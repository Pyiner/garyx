//! Thread pins and favorites handlers.

use super::*;

#[derive(Deserialize)]
pub struct ThreadFavoritesSnapshotQuery {
    #[serde(default)]
    pub include_summaries: Option<bool>,
}

#[derive(Deserialize)]
pub struct ThreadFavoritesMutationQuery {
    #[serde(default)]
    pub expected_revision: Option<String>,
    #[serde(default)]
    pub expected_store_incarnation: Option<String>,
}

pub(super) fn thread_pin_ids(page: &ThreadPinsPage) -> Vec<String> {
    page.pins
        .iter()
        .map(|record| record.thread_id.clone())
        .collect()
}

pub(super) fn thread_pins_payload(page: &ThreadPinsPage) -> Value {
    json!({
        "thread_ids": thread_pin_ids(page),
        "pins": page.pins,
        "revision": page.revision,
    })
}

pub(super) const THREAD_FAVORITES_GET_OPERATION: &str = "thread_favorites_get";

pub(super) const THREAD_FAVORITES_PUT_OPERATION: &str = "thread_favorites_put";

pub(super) const THREAD_FAVORITES_DELETE_OPERATION: &str = "thread_favorites_delete";

pub(super) const THREAD_FAVORITES_SNAPSHOT_OPERATION: &str = "thread_favorites_snapshot";

pub(super) fn thread_favorite_ids(page: &ThreadFavoritesPage) -> Vec<String> {
    page.favorites
        .iter()
        .map(|favorite| favorite.thread_id.clone())
        .collect()
}

pub(super) fn thread_favorites_payload(page: &ThreadFavoritesPage, server_boot_id: &str) -> Value {
    json!({
        "store_incarnation_id": page.store_incarnation_id,
        "server_boot_id": server_boot_id,
        "revision": page.revision,
        "thread_ids": thread_favorite_ids(page),
        "favorites": page.favorites,
    })
}

pub(super) fn thread_favorites_tagged_error(
    status: StatusCode,
    operation: &'static str,
    code: &'static str,
    message: impl Into<String>,
    page: Option<&ThreadFavoritesPage>,
    server_boot_id: &str,
    fields: Value,
) -> axum::response::Response {
    let mut payload = page.map_or_else(
        || {
            json!({
                "server_boot_id": server_boot_id,
            })
        },
        |page| thread_favorites_payload(page, server_boot_id),
    );
    extend_json_object(
        &mut payload,
        json!({
            "kind": "garyx_api_error",
            "operation": operation,
            "code": code,
            "message": message.into(),
        }),
    );
    extend_json_object(&mut payload, fields);
    (status, Json(payload)).into_response()
}

pub(super) fn parse_thread_favorites_mutation_query(
    query: Result<Query<ThreadFavoritesMutationQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<(i64, String), String> {
    let Query(query) = query.map_err(|error| error.to_string())?;
    let expected_revision = query
        .expected_revision
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|revision| *revision >= 0)
        .ok_or_else(|| "expected_revision must be a non-negative integer".to_owned())?;
    let expected_store_incarnation = query
        .expected_store_incarnation
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
        .map(|uuid| uuid.to_string())
        .ok_or_else(|| "expected_store_incarnation must be a UUID".to_owned())?;
    Ok((expected_revision, expected_store_incarnation))
}

pub(super) async fn tagged_favorites_invalid_request(
    state: &Arc<AppState>,
    operation: &'static str,
    message: String,
) -> axum::response::Response {
    let page = state
        .ops
        .garyx_db
        .run_blocking(|db| db.list_thread_favorites())
        .await
        .ok();
    thread_favorites_tagged_error(
        StatusCode::BAD_REQUEST,
        operation,
        "invalid_request",
        message,
        page.as_ref(),
        state.server_boot_id(),
        json!({}),
    )
}

pub(super) fn parse_reorder_thread_pins_request(
    payload: &Value,
) -> Result<(Vec<String>, i64), GaryxDbError> {
    let object = payload
        .as_object()
        .ok_or_else(|| GaryxDbError::BadRequest("request body must be a JSON object".to_owned()))?;
    let expected_revision = object
        .get("expected_revision")
        .and_then(Value::as_i64)
        .filter(|revision| *revision >= 0)
        .ok_or_else(|| {
            GaryxDbError::BadRequest("expected_revision must be a non-negative integer".to_owned())
        })?;
    let values = object
        .get("thread_ids")
        .and_then(Value::as_array)
        .filter(|values| !values.is_empty())
        .ok_or_else(|| {
            GaryxDbError::BadRequest("thread_ids must be a non-empty array".to_owned())
        })?;
    let mut thread_ids = Vec::with_capacity(values.len());
    let mut seen = BTreeSet::new();
    for value in values {
        let thread_id = value
            .as_str()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| {
                GaryxDbError::BadRequest(
                    "thread_ids must contain only non-empty strings".to_owned(),
                )
            })?;
        if !seen.insert(thread_id.to_owned()) {
            return Err(GaryxDbError::BadRequest(format!(
                "duplicate thread_id: {thread_id}"
            )));
        }
        thread_ids.push(thread_id.to_owned());
    }
    Ok((thread_ids, expected_revision))
}

/// GET /api/thread-pins - list pinned thread ids in display order.
pub async fn list_thread_pins(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state
        .ops
        .garyx_db
        .run_blocking(|db| db.list_pinned_threads())
        .await
    {
        Ok(page) => (StatusCode::OK, Json(thread_pins_payload(&page))).into_response(),
        Err(error) => garyx_db_error_response(error).into_response(),
    }
}

/// GET /api/thread-favorites - one atomic membership/revision/identity page.
pub async fn list_thread_favorites(State(state): State<Arc<AppState>>) -> axum::response::Response {
    match state
        .ops
        .garyx_db
        .run_blocking(|db| db.list_thread_favorites())
        .await
    {
        Ok(page) => (
            StatusCode::OK,
            Json(thread_favorites_payload(&page, state.server_boot_id())),
        )
            .into_response(),
        Err(error) => thread_favorites_tagged_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            THREAD_FAVORITES_GET_OPERATION,
            "unavailable",
            error.to_string(),
            None,
            state.server_boot_id(),
            json!({}),
        ),
    }
}

/// GET /api/thread-favorites/snapshot - membership and joined recent rows
/// from one SQLite read transaction.
pub async fn thread_favorites_snapshot(
    State(state): State<Arc<AppState>>,
    query: Result<Query<ThreadFavoritesSnapshotQuery>, axum::extract::rejection::QueryRejection>,
) -> axum::response::Response {
    let Query(query) = match query {
        Ok(query) => query,
        Err(error) => {
            return tagged_favorites_invalid_request(
                &state,
                THREAD_FAVORITES_SNAPSHOT_OPERATION,
                error.to_string(),
            )
            .await;
        }
    };
    if query.include_summaries.unwrap_or(false) {
        return match state
            .ops
            .garyx_db
            .run_blocking(|db| db.thread_favorites_snapshot_with_summaries())
            .await
        {
            Ok(enhanced) => {
                let threads = recent_thread_values(&state, &enhanced.snapshot.recent_threads).await;
                let mut payload =
                    thread_favorites_payload(&enhanced.snapshot.page, state.server_boot_id());
                extend_json_object(
                    &mut payload,
                    json!({
                        "recent": {
                            "threads": threads,
                            "total": enhanced.snapshot.recent_total,
                            "truncated": enhanced.snapshot.recent_truncated,
                        },
                        "summaries": enhanced.summaries,
                        "summaries_truncated": enhanced.summaries_truncated,
                    }),
                );
                (StatusCode::OK, Json(payload)).into_response()
            }
            Err(error) => thread_favorites_tagged_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                THREAD_FAVORITES_SNAPSHOT_OPERATION,
                "unavailable",
                error.to_string(),
                None,
                state.server_boot_id(),
                json!({}),
            ),
        };
    }
    match state
        .ops
        .garyx_db
        .run_blocking(|db| db.thread_favorites_snapshot())
        .await
    {
        Ok(snapshot) => {
            let threads = recent_thread_values(&state, &snapshot.recent_threads).await;
            let mut payload = thread_favorites_payload(&snapshot.page, state.server_boot_id());
            extend_json_object(
                &mut payload,
                json!({
                    "recent": {
                        "threads": threads,
                        "total": snapshot.recent_total,
                        "truncated": snapshot.recent_truncated,
                    }
                }),
            );
            (StatusCode::OK, Json(payload)).into_response()
        }
        Err(error) => thread_favorites_tagged_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            THREAD_FAVORITES_SNAPSHOT_OPERATION,
            "unavailable",
            error.to_string(),
            None,
            state.server_boot_id(),
            json!({}),
        ),
    }
}

/// PUT /api/thread-favorites/:key - conditionally favorite one thread.
pub async fn favorite_thread(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    query: Result<Query<ThreadFavoritesMutationQuery>, axum::extract::rejection::QueryRejection>,
) -> axum::response::Response {
    mutate_thread_favorite(state, key, query, true, THREAD_FAVORITES_PUT_OPERATION).await
}

/// DELETE /api/thread-favorites/:key - conditionally unfavorite one thread.
pub async fn unfavorite_thread(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    query: Result<Query<ThreadFavoritesMutationQuery>, axum::extract::rejection::QueryRejection>,
) -> axum::response::Response {
    mutate_thread_favorite(state, key, query, false, THREAD_FAVORITES_DELETE_OPERATION).await
}

pub(super) async fn mutate_thread_favorite(
    state: Arc<AppState>,
    key: String,
    query: Result<Query<ThreadFavoritesMutationQuery>, axum::extract::rejection::QueryRejection>,
    favorited: bool,
    operation: &'static str,
) -> axum::response::Response {
    let (expected_revision, expected_store_incarnation) =
        match parse_thread_favorites_mutation_query(query) {
            Ok(expected) => expected,
            Err(message) => {
                return tagged_favorites_invalid_request(&state, operation, message).await;
            }
        };
    let thread_id = key.trim().to_owned();
    if !is_thread_key(&thread_id) {
        return tagged_favorites_invalid_request(
            &state,
            operation,
            "thread key must use the thread:: prefix".to_owned(),
        )
        .await;
    }
    let mutation_thread_id = thread_id.clone();
    let mutation_expected_store_incarnation = expected_store_incarnation.clone();
    let result = state
        .ops
        .garyx_db
        .run_blocking(move |db| {
            db.set_thread_favorite(
                &mutation_thread_id,
                favorited,
                expected_revision,
                &mutation_expected_store_incarnation,
            )
        })
        .await;
    match result {
        Ok(FavoriteThreadResult::Updated { changed, page }) => {
            let mut payload = thread_favorites_payload(&page, state.server_boot_id());
            extend_json_object(
                &mut payload,
                if favorited {
                    json!({
                        "favorited": true,
                        "changed": changed,
                        "thread_id": thread_id,
                    })
                } else {
                    json!({
                        "favorited": false,
                        "removed": changed,
                        "thread_id": thread_id,
                    })
                },
            );
            (StatusCode::OK, Json(payload)).into_response()
        }
        Ok(FavoriteThreadResult::Conflict(page)) => thread_favorites_tagged_error(
            StatusCode::CONFLICT,
            operation,
            "conflict",
            "favorites revision does not match",
            Some(&page),
            state.server_boot_id(),
            json!({
                "conflict": true,
                "expected_revision": expected_revision,
                "favorited": page.favorites.iter().any(|item| item.thread_id == thread_id),
            }),
        ),
        Ok(FavoriteThreadResult::WrongIncarnation(page)) => thread_favorites_tagged_error(
            StatusCode::CONFLICT,
            operation,
            "wrong_incarnation",
            "store incarnation does not match",
            Some(&page),
            state.server_boot_id(),
            json!({
                "expected_store_incarnation": expected_store_incarnation,
                "favorited": page.favorites.iter().any(|item| item.thread_id == thread_id),
            }),
        ),
        Ok(FavoriteThreadResult::NotFound(page)) => thread_favorites_tagged_error(
            StatusCode::NOT_FOUND,
            operation,
            "not_found",
            format!("thread not found: {thread_id}"),
            Some(&page),
            state.server_boot_id(),
            json!({
                "favorited": false,
                "thread_id": thread_id,
            }),
        ),
        Err(error) => {
            let (status, code) = if matches!(error, GaryxDbError::BadRequest(_)) {
                (StatusCode::BAD_REQUEST, "invalid_request")
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, "unavailable")
            };
            thread_favorites_tagged_error(
                status,
                operation,
                code,
                error.to_string(),
                None,
                state.server_boot_id(),
                json!({}),
            )
        }
    }
}

/// PUT /api/thread-pins - reorder the pinned collection with revision CAS.
pub async fn reorder_thread_pins(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let (thread_ids, expected_revision) = match parse_reorder_thread_pins_request(&payload) {
        Ok(request) => request,
        Err(error) => return garyx_db_error_response(error).into_response(),
    };
    match state
        .ops
        .garyx_db
        .run_blocking(move |db| db.reorder_thread_pins(thread_ids, expected_revision))
        .await
    {
        Ok(ReorderThreadPinsResult::Updated(page)) => {
            (StatusCode::OK, Json(thread_pins_payload(&page))).into_response()
        }
        Ok(ReorderThreadPinsResult::Conflict(page)) => {
            (StatusCode::CONFLICT, Json(thread_pins_payload(&page))).into_response()
        }
        Err(error) => garyx_db_error_response(error).into_response(),
    }
}

/// PUT /api/thread-pins/:key - mark a thread as pinned.
pub async fn pin_thread(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    let thread_id = match ensure_existing_thread_id(&state, &key).await {
        Ok(Some(thread_id)) => thread_id,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"pinned": false, "error": "thread not found"})),
            )
                .into_response();
        }
        Err(response) => return response.into_response(),
    };
    let pin_thread_id = thread_id.clone();
    match state
        .ops
        .garyx_db
        .run_blocking(move |db| db.pin_thread(&pin_thread_id))
        .await
    {
        Ok(page) => {
            let pin = page
                .pins
                .iter()
                .find(|record| record.thread_id == thread_id)
                .cloned();
            (
                StatusCode::OK,
                Json(json!({
                "pinned": true,
                "pin": pin,
                "thread_ids": thread_pin_ids(&page),
                "pins": page.pins,
                "revision": page.revision,
                })),
            )
                .into_response()
        }
        Err(error) => garyx_db_error_response(error).into_response(),
    }
}

/// DELETE /api/thread-pins/:key - remove a thread pin.
pub async fn unpin_thread(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    let thread_id = match ensure_existing_thread_id(&state, &key).await {
        Ok(resolved) => resolved.unwrap_or_else(|| key.trim().to_owned()),
        Err(response) => return response.into_response(),
    };
    let unpin_thread_id = thread_id.clone();
    match state
        .ops
        .garyx_db
        .run_blocking(move |db| db.unpin_thread(&unpin_thread_id))
        .await
    {
        Ok((removed, page)) => (
            StatusCode::OK,
            Json(json!({
                "pinned": false,
                "removed": removed,
                "thread_id": thread_id,
                "thread_ids": thread_pin_ids(&page),
                "pins": page.pins,
                "revision": page.revision,
            })),
        )
            .into_response(),
        Err(error) => garyx_db_error_response(error).into_response(),
    }
}
