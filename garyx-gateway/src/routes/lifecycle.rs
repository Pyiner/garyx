//! Thread archive/delete lifecycle handlers and the owner-run coordination.

use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveThreadBody {
    #[serde(alias = "operation_id")]
    pub operation_id: String,
    #[serde(alias = "expected_store_incarnation")]
    pub expected_store_incarnation: String,
    #[serde(default, alias = "endpoint_keys")]
    pub endpoint_keys: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteThreadBody {
    #[serde(alias = "operation_id")]
    pub operation_id: String,
    #[serde(alias = "expected_store_incarnation")]
    pub expected_store_incarnation: String,
}

pub(super) fn cron_target_thread_id(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if is_thread_key(trimmed) {
        return Some(trimmed.to_owned());
    }
    let stripped = trimmed.strip_prefix("thread:")?;
    is_thread_key(stripped).then(|| stripped.to_owned())
}

pub(super) fn cron_job_references_thread(job: &crate::cron::CronJob, thread_id: &str) -> bool {
    job.thread_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some_and(|value| value == thread_id)
        || job
            .target
            .as_deref()
            .and_then(cron_target_thread_id)
            .is_some_and(|target| target == thread_id)
}

pub(super) async fn automation_job_for_archive_conflict(
    state: &Arc<AppState>,
    thread_id: &str,
) -> Option<String> {
    let service = state.ops.cron_service.as_ref()?;
    service
        .list_all()
        .await
        .into_iter()
        .find(|job| cron_job_references_thread(job, thread_id))
        .map(|job| job.id)
}

#[derive(Clone)]
pub(super) struct LifecycleRequest {
    kind: LifecycleOperationKind,
    thread_id: String,
    operation_id: String,
    expected_store_incarnation: String,
    fingerprint: String,
    endpoint_keys: Vec<String>,
}

pub(super) enum LifecycleChildResult {
    Drain(Result<(), CoordinationError>),
    Transaction(GaryxDbResult<LifecycleTransactionResult>),
}

pub(super) type LifecycleMutationSupervisor = MutationSupervisor<LifecycleChildResult>;

pub(super) fn lifecycle_operation_name(kind: LifecycleOperationKind) -> &'static str {
    match kind {
        LifecycleOperationKind::Archive => "thread_archive",
        LifecycleOperationKind::Delete => "thread_delete",
    }
}

pub(super) fn lifecycle_tagged_error(
    status: StatusCode,
    kind: LifecycleOperationKind,
    code: &'static str,
    message: impl Into<String>,
    fields: Value,
) -> axum::response::Response {
    let mut payload = json!({
        "kind": "garyx_api_error",
        "operation": lifecycle_operation_name(kind),
        "code": code,
        "message": message.into(),
    });
    extend_json_object(&mut payload, fields);
    (status, Json(payload)).into_response()
}

pub(super) fn parse_lifecycle_request(
    kind: LifecycleOperationKind,
    key: &str,
    operation_id: &str,
    expected_store_incarnation: &str,
    endpoint_keys: Vec<String>,
) -> Result<LifecycleRequest, axum::response::Response> {
    let thread_id = key.trim();
    if !is_thread_key(thread_id) {
        return Err(lifecycle_tagged_error(
            StatusCode::BAD_REQUEST,
            kind,
            "invalid_request",
            "thread key must use the thread:: prefix",
            json!({}),
        ));
    }
    let operation_id = uuid::Uuid::parse_str(operation_id.trim())
        .map(|value| value.to_string())
        .map_err(|_| {
            lifecycle_tagged_error(
                StatusCode::BAD_REQUEST,
                kind,
                "invalid_request",
                "operation_id must be a UUID",
                json!({ "thread_id": thread_id }),
            )
        })?;
    let expected_store_incarnation = uuid::Uuid::parse_str(expected_store_incarnation.trim())
        .map(|value| value.to_string())
        .map_err(|_| {
            lifecycle_tagged_error(
                StatusCode::BAD_REQUEST,
                kind,
                "invalid_request",
                "expected_store_incarnation must be a UUID",
                json!({
                    "thread_id": thread_id,
                    "operation_id": operation_id,
                }),
            )
        })?;
    let endpoint_keys = endpoint_keys
        .into_iter()
        .map(|key| normalize_endpoint_lookup_key(&key))
        .filter(|key| !key.is_empty())
        .collect::<Vec<_>>();
    let fingerprint =
        canonical_lifecycle_fingerprint(kind, thread_id, endpoint_keys).map_err(|error| {
            lifecycle_tagged_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                kind,
                "unavailable",
                format!("failed to canonicalize lifecycle request: {error}"),
                json!({
                    "thread_id": thread_id,
                    "operation_id": operation_id.clone(),
                }),
            )
        })?;
    Ok(LifecycleRequest {
        kind,
        thread_id: thread_id.to_owned(),
        operation_id,
        expected_store_incarnation,
        fingerprint: fingerprint.canonical,
        endpoint_keys: fingerprint.endpoint_keys,
    })
}

pub(super) fn operation_matches_request(
    operation: &LifecycleOperationRecord,
    request: &LifecycleRequest,
) -> bool {
    operation.kind == request.kind
        && operation.thread_id == request.thread_id
        && operation.fingerprint == request.fingerprint
}

pub(super) fn completed_lifecycle_response(
    operation: &LifecycleOperationRecord,
) -> axum::response::Response {
    match operation.outcome {
        LifecycleOperationOutcome::AppliedChanged | LifecycleOperationOutcome::AppliedNoop => {
            let detached_endpoint_keys = operation
                .result_payload
                .as_ref()
                .and_then(|payload| payload.get("detached_endpoint_keys"))
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new()));
            let mut payload = json!({
                "operation_id": operation.operation_id,
                "outcome": operation.outcome,
                "thread_id": operation.thread_id,
                "changed": operation.outcome == LifecycleOperationOutcome::AppliedChanged,
                "detached_endpoint_keys": detached_endpoint_keys,
            });
            match operation.kind {
                LifecycleOperationKind::Archive => {
                    extend_json_object(&mut payload, json!({ "archived": true, "deleted": true }));
                }
                LifecycleOperationKind::Delete => {
                    extend_json_object(&mut payload, json!({ "deleted": true }));
                }
            }
            (StatusCode::OK, Json(payload)).into_response()
        }
        LifecycleOperationOutcome::RejectedConflict
        | LifecycleOperationOutcome::RejectedNotFound => {
            let status = if operation.outcome == LifecycleOperationOutcome::RejectedConflict {
                StatusCode::CONFLICT
            } else {
                StatusCode::NOT_FOUND
            };
            let message = operation
                .detail
                .as_ref()
                .and_then(|detail| detail.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("thread lifecycle request was rejected");
            let reason_code = operation
                .detail
                .as_ref()
                .and_then(|detail| detail.get("code"))
                .cloned();
            let mut fields = json!({
                "operation_id": operation.operation_id,
                "outcome": operation.outcome,
                "thread_id": operation.thread_id,
                "error": message,
                "detail": operation.detail,
                "reason_code": reason_code,
            });
            match operation.kind {
                LifecycleOperationKind::Archive => {
                    extend_json_object(&mut fields, json!({ "archived": false }));
                }
                LifecycleOperationKind::Delete => {
                    extend_json_object(&mut fields, json!({ "deleted": false }));
                }
            }
            if let Some(detail) = operation.detail.as_ref().and_then(Value::as_object) {
                for (key, value) in detail {
                    if key != "code" && key != "message" {
                        fields[key] = value.clone();
                    }
                }
            }
            lifecycle_tagged_error(
                status,
                operation.kind,
                match operation.outcome {
                    LifecycleOperationOutcome::RejectedConflict => "rejected_conflict",
                    LifecycleOperationOutcome::RejectedNotFound => "rejected_not_found",
                    _ => unreachable!(),
                },
                message,
                fields,
            )
        }
    }
}

pub(super) fn lifecycle_cell_response(
    request: &LifecycleRequest,
    result: &OperationCellResult,
) -> axum::response::Response {
    match result {
        OperationCellResult::Completed(operation) => {
            if operation_matches_request(operation, request) {
                completed_lifecycle_response(operation)
            } else {
                lifecycle_tagged_error(
                    StatusCode::CONFLICT,
                    request.kind,
                    "operation_id_conflict",
                    "operation_id was reused with a different lifecycle request",
                    json!({
                        "thread_id": request.thread_id,
                        "operation_id": request.operation_id,
                    }),
                )
            }
        }
        OperationCellResult::OperationIdConflict => lifecycle_tagged_error(
            StatusCode::CONFLICT,
            request.kind,
            "operation_id_conflict",
            "operation_id was reused with a different lifecycle request",
            json!({
                "thread_id": request.thread_id,
                "operation_id": request.operation_id,
            }),
        ),
        OperationCellResult::WrongIncarnation {
            current_store_incarnation,
        } => lifecycle_tagged_error(
            StatusCode::CONFLICT,
            request.kind,
            "wrong_incarnation",
            "store incarnation does not match",
            json!({
                "thread_id": request.thread_id,
                "operation_id": request.operation_id,
                "expected_store_incarnation": request.expected_store_incarnation,
                "current_store_incarnation": current_store_incarnation,
            }),
        ),
        OperationCellResult::InProgress => lifecycle_tagged_error(
            StatusCode::CONFLICT,
            request.kind,
            "operation_in_progress",
            "thread lifecycle operation is still in progress",
            json!({
                "thread_id": request.thread_id,
                "operation_id": request.operation_id,
            }),
        ),
        OperationCellResult::TransientFailure => lifecycle_tagged_error(
            StatusCode::SERVICE_UNAVAILABLE,
            request.kind,
            "unavailable",
            "thread lifecycle result is temporarily unavailable",
            json!({
                "thread_id": request.thread_id,
                "operation_id": request.operation_id,
            }),
        ),
    }
}

pub(super) async fn wait_for_lifecycle_result(
    request: &LifecycleRequest,
    waiter: OperationJoinHandle,
) -> axum::response::Response {
    match waiter.wait(LIFECYCLE_JOIN_WINDOW).await {
        Ok(result) => lifecycle_cell_response(request, &result),
        Err(OperationWaitError::InProgress) => {
            lifecycle_cell_response(request, &OperationCellResult::InProgress)
        }
        Err(OperationWaitError::TransientFailure) => {
            lifecycle_cell_response(request, &OperationCellResult::TransientFailure)
        }
    }
}

pub(super) fn coordination_failure(error: CoordinationError) -> OperationCellResult {
    match error {
        CoordinationError::Unavailable => OperationCellResult::InProgress,
        CoordinationError::Store(_) | CoordinationError::Abort(_) => {
            OperationCellResult::TransientFailure
        }
    }
}

pub(super) enum LifecycleDbCommand {
    Mutation(LifecycleMutationInput),
    Decision(LifecycleDecisionInput),
}

pub(super) async fn execute_lifecycle_db_command(
    state: &Arc<AppState>,
    request: &LifecycleRequest,
    supervisor: &mut LifecycleMutationSupervisor,
    reservation: garyx_router::LifecycleReservation,
    command: LifecycleDbCommand,
) -> OperationCellResult {
    let witness = reservation.commit_witness();
    supervisor.insert_guard(reservation);
    let db = state.ops.garyx_db.clone();
    supervisor.spawn_blocking_child(move || {
        let result = match command {
            LifecycleDbCommand::Mutation(input) => db.execute_lifecycle_mutation(input),
            LifecycleDbCommand::Decision(input) => db.execute_lifecycle_decision(input),
        };
        if let Ok(
            LifecycleTransactionResult::Completed {
                durable_terminal, ..
            }
            | LifecycleTransactionResult::Existing {
                durable_terminal, ..
            },
        ) = &result
        {
            witness.mark_committed(*durable_terminal);
        }
        LifecycleChildResult::Transaction(result)
    });
    let joined = supervisor.join_child().await;
    let mut reservation = supervisor
        .take_guard::<garyx_router::LifecycleReservation>()
        .expect("lifecycle supervisor lost its reservation");
    match joined {
        Ok(LifecycleChildResult::Transaction(Ok(
            LifecycleTransactionResult::Completed {
                operation,
                durable_terminal,
            }
            | LifecycleTransactionResult::Existing {
                operation,
                durable_terminal,
            },
        ))) => {
            if matches!(
                operation.outcome,
                LifecycleOperationOutcome::AppliedChanged | LifecycleOperationOutcome::AppliedNoop
            ) {
                reservation.settle_committed(durable_terminal);
            } else {
                reservation.settle_decision(durable_terminal);
            }
            state.ops.lifecycle.wake_outbox();
            if operation_matches_request(&operation, request) {
                OperationCellResult::Completed(operation)
            } else {
                OperationCellResult::OperationIdConflict
            }
        }
        Ok(LifecycleChildResult::Transaction(Ok(
            LifecycleTransactionResult::WrongIncarnation {
                current_store_incarnation,
            },
        ))) => {
            let prior = reservation.prior_terminal();
            reservation.settle_transient(prior);
            OperationCellResult::WrongIncarnation {
                current_store_incarnation,
            }
        }
        Ok(LifecycleChildResult::Transaction(Err(_))) | Err(_) => {
            let prior = reservation.prior_terminal();
            reservation.settle_transient(prior);
            OperationCellResult::TransientFailure
        }
        Ok(LifecycleChildResult::Drain(_)) => {
            unreachable!("transaction child returned a drain result")
        }
    }
}

pub(super) async fn run_lifecycle_owner_inner(
    state: &Arc<AppState>,
    request: &LifecycleRequest,
    supervisor: &mut LifecycleMutationSupervisor,
) -> OperationCellResult {
    // Owner double-check closes completed-lookup → registration races. The
    // DB helper repeats identity first under the same read transaction.
    let db = state.ops.garyx_db.clone();
    let expected = request.expected_store_incarnation.clone();
    let operation_id = request.operation_id.clone();
    let lookup = db
        .run_blocking(move |db| db.lookup_lifecycle_operation(&expected, &operation_id))
        .await;
    match lookup {
        Ok(LifecycleOperationLookup::WrongIncarnation {
            current_store_incarnation,
        }) => {
            return OperationCellResult::WrongIncarnation {
                current_store_incarnation,
            };
        }
        Ok(LifecycleOperationLookup::Current(Some(operation))) => {
            return if operation_matches_request(&operation, request) {
                OperationCellResult::Completed(operation)
            } else {
                OperationCellResult::OperationIdConflict
            };
        }
        Ok(LifecycleOperationLookup::Current(None)) => {}
        Err(_) => return OperationCellResult::TransientFailure,
    }

    let coordinator = state.threads.thread_store.run_coordinator();
    if request.kind == LifecycleOperationKind::Archive {
        let thread_exists = match state.threads.thread_store.get(&request.thread_id).await {
            Ok(record) => record.is_some(),
            Err(_) => return OperationCellResult::TransientFailure,
        };
        if thread_exists
            && let Some(automation_id) =
                automation_job_for_archive_conflict(state, &request.thread_id).await
        {
            let reservation = match coordinator
                .reserve_decision(state.threads.thread_store.as_ref(), &request.thread_id)
                .await
            {
                Ok(reservation) => reservation,
                Err(error) => return coordination_failure(error),
            };
            return execute_lifecycle_db_command(
                state,
                request,
                supervisor,
                reservation,
                LifecycleDbCommand::Decision(LifecycleDecisionInput {
                    expected_store_incarnation: request.expected_store_incarnation.clone(),
                    operation_id: request.operation_id.clone(),
                    kind: request.kind,
                    thread_id: request.thread_id.clone(),
                    fingerprint: request.fingerprint.clone(),
                    outcome: LifecycleOperationOutcome::RejectedConflict,
                    detail: json!({
                        "code": "automation_target",
                        "message": "cannot archive thread targeted by automation",
                        "automation_id": automation_id,
                    }),
                }),
            )
            .await;
        }

        let (reservation, command) = match coordinator
            .reserve_archive(state.threads.thread_store.as_ref(), &request.thread_id)
            .await
        {
            Ok(ArchiveBarrier::Ready(reservation)) => (
                reservation,
                LifecycleDbCommand::Mutation(LifecycleMutationInput {
                    expected_store_incarnation: request.expected_store_incarnation.clone(),
                    operation_id: request.operation_id.clone(),
                    kind: request.kind,
                    thread_id: request.thread_id.clone(),
                    fingerprint: request.fingerprint.clone(),
                    endpoint_keys: request.endpoint_keys.clone(),
                    enabled_channel_accounts: BTreeSet::new(),
                }),
            ),
            Ok(ArchiveBarrier::ActiveLease(reservation)) => (
                reservation,
                LifecycleDbCommand::Decision(LifecycleDecisionInput {
                    expected_store_incarnation: request.expected_store_incarnation.clone(),
                    operation_id: request.operation_id.clone(),
                    kind: request.kind,
                    thread_id: request.thread_id.clone(),
                    fingerprint: request.fingerprint.clone(),
                    outcome: LifecycleOperationOutcome::RejectedConflict,
                    detail: json!({
                        "code": "active_run",
                        "message": "cannot archive thread with active run",
                        "active_run_id": Value::Null,
                    }),
                }),
            ),
            Err(error) => return coordination_failure(error),
        };
        return execute_lifecycle_db_command(state, request, supervisor, reservation, command)
            .await;
    }

    let preflight = state
        .ops
        .endpoint_binding_mutator
        .preflight_and_freeze(&request.thread_id, || state.config_snapshot())
        .await;
    match preflight {
        Ok(DeleteBindingPreflight::InProgress) => OperationCellResult::InProgress,
        Ok(DeleteBindingPreflight::RejectedEnabledBinding) => {
            let reservation = match coordinator
                .reserve_decision(state.threads.thread_store.as_ref(), &request.thread_id)
                .await
            {
                Ok(reservation) => reservation,
                Err(error) => return coordination_failure(error),
            };
            execute_lifecycle_db_command(
                state,
                request,
                supervisor,
                reservation,
                LifecycleDbCommand::Decision(LifecycleDecisionInput {
                    expected_store_incarnation: request.expected_store_incarnation.clone(),
                    operation_id: request.operation_id.clone(),
                    kind: request.kind,
                    thread_id: request.thread_id.clone(),
                    fingerprint: request.fingerprint.clone(),
                    outcome: LifecycleOperationOutcome::RejectedConflict,
                    detail: json!({
                        "code": "active_channel_binding",
                        "message": "cannot delete thread with active channel bindings",
                    }),
                }),
            )
            .await
        }
        Ok(DeleteBindingPreflight::Frozen {
            guard,
            enabled_channel_accounts,
        }) => {
            supervisor.insert_guard(guard);
            let reservation = match coordinator
                .reserve_delete(state.threads.thread_store.as_ref(), &request.thread_id)
                .await
            {
                Ok(reservation) => reservation,
                Err(error) => return coordination_failure(error),
            };
            let drain = reservation.abort_and_drain_future();
            supervisor.insert_guard(reservation);
            supervisor.spawn_child(async move { LifecycleChildResult::Drain(drain.await) });
            let drain_result = supervisor.join_child().await;
            match drain_result {
                Ok(LifecycleChildResult::Drain(Ok(()))) => {}
                Ok(LifecycleChildResult::Drain(Err(error))) => {
                    let mut reservation = supervisor
                        .take_guard::<garyx_router::LifecycleReservation>()
                        .expect("lifecycle supervisor lost its reservation");
                    let prior = reservation.prior_terminal();
                    reservation.settle_transient(prior);
                    return coordination_failure(error);
                }
                Err(_) => {
                    let mut reservation = supervisor
                        .take_guard::<garyx_router::LifecycleReservation>()
                        .expect("lifecycle supervisor lost its reservation");
                    let prior = reservation.prior_terminal();
                    reservation.settle_transient(prior);
                    return OperationCellResult::TransientFailure;
                }
                Ok(LifecycleChildResult::Transaction(_)) => {
                    unreachable!("drain child returned a transaction result")
                }
            }
            let reservation = supervisor
                .take_guard::<garyx_router::LifecycleReservation>()
                .expect("lifecycle supervisor lost its reservation after drain");
            execute_lifecycle_db_command(
                state,
                request,
                supervisor,
                reservation,
                LifecycleDbCommand::Mutation(LifecycleMutationInput {
                    expected_store_incarnation: request.expected_store_incarnation.clone(),
                    operation_id: request.operation_id.clone(),
                    kind: request.kind,
                    thread_id: request.thread_id.clone(),
                    fingerprint: request.fingerprint.clone(),
                    endpoint_keys: Vec::new(),
                    enabled_channel_accounts,
                }),
            )
            .await
        }
        Err(_) => OperationCellResult::TransientFailure,
    }
}

pub(super) async fn run_lifecycle_owner(
    state: Arc<AppState>,
    request: LifecycleRequest,
    owner: OperationOwnerGuard,
) {
    let mut supervisor = LifecycleMutationSupervisor::new();
    supervisor.insert_guard(owner);
    #[cfg(any(test, feature = "test-seams"))]
    if state.ops.lifecycle.take_owner_panic() {
        panic!("injected lifecycle owner panic");
    }
    let result = run_lifecycle_owner_inner(&state, &request, &mut supervisor).await;
    let owner = supervisor
        .take_guard::<OperationOwnerGuard>()
        .expect("lifecycle supervisor lost its operation owner");
    owner.publish(result);
}

pub(super) async fn handle_lifecycle_request(
    state: Arc<AppState>,
    request: LifecycleRequest,
) -> axum::response::Response {
    let db = state.ops.garyx_db.clone();
    let expected = request.expected_store_incarnation.clone();
    let operation_id = request.operation_id.clone();
    let lookup = db
        .run_blocking(move |db| db.lookup_lifecycle_operation(&expected, &operation_id))
        .await;
    match lookup {
        Ok(LifecycleOperationLookup::WrongIncarnation {
            current_store_incarnation,
        }) => {
            return lifecycle_cell_response(
                &request,
                &OperationCellResult::WrongIncarnation {
                    current_store_incarnation,
                },
            );
        }
        Ok(LifecycleOperationLookup::Current(Some(operation))) => {
            return lifecycle_cell_response(&request, &OperationCellResult::Completed(operation));
        }
        Ok(LifecycleOperationLookup::Current(None)) => {}
        Err(_) => {
            return lifecycle_cell_response(&request, &OperationCellResult::TransientFailure);
        }
    }

    #[cfg(any(test, feature = "test-seams"))]
    state
        .ops
        .lifecycle
        .pause_after_initial_lookup_if_configured()
        .await;

    let key = OperationKey {
        store_incarnation: request.expected_store_incarnation.clone(),
        operation_id: request.operation_id.clone(),
    };
    match state
        .ops
        .lifecycle
        .registry
        .register(key, &request.fingerprint)
    {
        Ok(OperationRegistration::Join(waiter)) => {
            wait_for_lifecycle_result(&request, waiter).await
        }
        Ok(OperationRegistration::Owner(owner)) => {
            let waiter = owner.join_handle();
            tokio::spawn(run_lifecycle_owner(state, request.clone(), owner));
            wait_for_lifecycle_result(&request, waiter).await
        }
        Err(OperationRegistrationError::FingerprintConflict) => {
            lifecycle_cell_response(&request, &OperationCellResult::OperationIdConflict)
        }
    }
}

/// POST /api/threads/:key/archive - idempotent product archive.
pub async fn archive_thread(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    Json(body): Json<ArchiveThreadBody>,
) -> axum::response::Response {
    let request = match parse_lifecycle_request(
        LifecycleOperationKind::Archive,
        &key,
        &body.operation_id,
        &body.expected_store_incarnation,
        body.endpoint_keys,
    ) {
        Ok(request) => request,
        Err(response) => return response,
    };
    handle_lifecycle_request(state, request).await
}

/// DELETE /api/threads/:key - idempotent destructive delete.
pub async fn delete_thread(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    Json(body): Json<DeleteThreadBody>,
) -> axum::response::Response {
    let request = match parse_lifecycle_request(
        LifecycleOperationKind::Delete,
        &key,
        &body.operation_id,
        &body.expected_store_incarnation,
        Vec::new(),
    ) {
        Ok(request) => request,
        Err(response) => return response,
    };
    handle_lifecycle_request(state, request).await
}
