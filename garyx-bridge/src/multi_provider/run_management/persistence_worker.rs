use super::*;
use garyx_router::{ThreadRecordPatch, ThreadStoreExt};

pub(super) fn insert_snapshot_field(
    metadata: &mut Map<String, Value>,
    snapshot_key: &str,
    override_key: &str,
    value: Option<&str>,
) -> bool {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    let has_snapshot = metadata
        .get(snapshot_key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some();
    let has_override = metadata
        .get(override_key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some();
    if has_snapshot || has_override {
        return false;
    }
    metadata.insert(snapshot_key.to_owned(), Value::String(value.to_owned()));
    true
}

pub(super) async fn persist_thread_runtime_snapshot(
    store: Option<Arc<dyn ThreadStore>>,
    thread_id: &str,
    selection: &ProviderRuntimeSelection,
) {
    let Some(store) = store else {
        return;
    };
    let Some(mut thread_data) = store.get_logged(thread_id).await else {
        return;
    };
    let observed = thread_data.clone();
    let existing_top_level_overrides = [
        MODEL_OVERRIDE_METADATA_KEY,
        MODEL_REASONING_EFFORT_OVERRIDE_METADATA_KEY,
        MODEL_SERVICE_TIER_OVERRIDE_METADATA_KEY,
    ]
    .map(|key| has_thread_value(&thread_data, key));
    let existing_snapshot_fields = [
        MODEL_METADATA_KEY,
        MODEL_REASONING_EFFORT_METADATA_KEY,
        MODEL_SERVICE_TIER_METADATA_KEY,
    ]
    .map(|key| has_thread_value(&thread_data, key));
    let Some(thread_object) = thread_data.as_object_mut() else {
        return;
    };
    let metadata_value = thread_object
        .entry("metadata".to_owned())
        .or_insert_with(|| Value::Object(Map::new()));
    if !metadata_value.is_object() {
        *metadata_value = Value::Object(Map::new());
    }
    let Some(metadata) = metadata_value.as_object_mut() else {
        return;
    };

    let mut changed = false;
    if !existing_top_level_overrides[0] && !existing_snapshot_fields[0] {
        changed |= insert_snapshot_field(
            metadata,
            MODEL_METADATA_KEY,
            MODEL_OVERRIDE_METADATA_KEY,
            selection.model.as_deref(),
        );
    }
    if !existing_top_level_overrides[1] && !existing_snapshot_fields[1] {
        changed |= insert_snapshot_field(
            metadata,
            MODEL_REASONING_EFFORT_METADATA_KEY,
            MODEL_REASONING_EFFORT_OVERRIDE_METADATA_KEY,
            selection.model_reasoning_effort.as_deref(),
        );
    }
    if !existing_top_level_overrides[2] && !existing_snapshot_fields[2] {
        changed |= insert_snapshot_field(
            metadata,
            MODEL_SERVICE_TIER_METADATA_KEY,
            MODEL_SERVICE_TIER_OVERRIDE_METADATA_KEY,
            selection.model_service_tier.as_deref(),
        );
    }
    if !changed {
        return;
    }
    thread_object.insert(
        "updated_at".to_owned(),
        Value::String(Utc::now().to_rfc3339()),
    );
    let patch =
        match ThreadRecordPatch::from_diff(&observed, &thread_data, &["metadata", "updated_at"]) {
            Ok(patch) => patch,
            Err(error) => {
                tracing::warn!(thread_id, error = %error, "invalid runtime snapshot patch");
                return;
            }
        };
    if let Err(error) = store.patch(thread_id, patch).await {
        tracing::warn!(thread_id, error = %error, "runtime snapshot patch did not persist");
    }
}

pub(super) fn build_pending_input_content(
    message: &str,
    images: &[ImagePayload],
    attachments: &[garyx_models::provider::PromptAttachment],
) -> Value {
    build_user_content_from_parts(message, attachments, images)
}

pub(super) fn is_persistent_control_stream_event(event: &StreamEvent) -> bool {
    matches!(event, StreamEvent::Boundary { .. } | StreamEvent::Done)
}

pub(super) fn emit_committed_records(
    event_tx: &Option<tokio::sync::broadcast::Sender<String>>,
    thread_id: &str,
    run_id: Option<&str>,
    committed: Vec<(u64, Value)>,
) {
    let event_run_id = run_id.map(str::to_owned);
    for (seq, message) in committed {
        emit_gateway_event(
            event_tx,
            serde_json::json!({
                "type": "committed_message",
                "thread_id": thread_id,
                "run_id": event_run_id.clone(),
                "seq": seq,
                "message": message,
            }),
        );
    }
}

pub(super) fn control_record_for_stream_event(
    thread_id: &str,
    run_id: &str,
    event: &StreamEvent,
    after_content_count: usize,
) -> Option<RunControlRecord> {
    let mut payload = serde_json::Map::new();
    let kind = match event {
        StreamEvent::Boundary {
            kind: garyx_models::provider::StreamBoundaryKind::AssistantSegment,
            pending_input_id,
        } => {
            if let Some(pending_input_id) = pending_input_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                payload.insert(
                    "pending_input_id".to_owned(),
                    Value::String(pending_input_id.to_owned()),
                );
            }
            "assistant_boundary"
        }
        StreamEvent::Boundary {
            kind: garyx_models::provider::StreamBoundaryKind::UserAck,
            pending_input_id,
        } => {
            if let Some(pending_input_id) = pending_input_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                payload.insert(
                    "pending_input_id".to_owned(),
                    Value::String(pending_input_id.to_owned()),
                );
            }
            "user_ack"
        }
        StreamEvent::Done => "done",
        _ => return None,
    };
    Some(RunControlRecord::new(
        kind,
        thread_id,
        run_id,
        Utc::now().to_rfc3339(),
        payload,
        after_content_count,
    ))
}

pub(super) fn abort_terminal_control_record(
    thread_id: &str,
    run_id: &str,
    after_content_count: usize,
    error: Option<&str>,
) -> RunControlRecord {
    let mut payload = serde_json::Map::new();
    payload.insert("status".to_owned(), Value::String("interrupted".to_owned()));
    if let Some(error) = error.map(str::trim).filter(|value| !value.is_empty()) {
        payload.insert("error".to_owned(), Value::String(error.to_owned()));
    }
    RunControlRecord::new(
        "run_complete",
        thread_id,
        run_id,
        Utc::now().to_rfc3339(),
        payload,
        after_content_count,
    )
}

pub(super) fn maybe_push_capsule_attachment_control(
    thread_id: &str,
    run_id: &str,
    snapshot: &mut StreamingRunSnapshot,
    message: &garyx_models::provider::ProviderMessage,
    transcript_controls: &mut Vec<RunControlRecord>,
) -> bool {
    let Some(attachment) = snapshot.capsule_attachment_for_tool_result(message) else {
        return false;
    };
    let after_content_count = 1 + snapshot.session_messages.len();
    let key = attachment.marker_key(message.tool_use_id.as_deref(), after_content_count);
    if !snapshot.emitted_capsule_markers.insert(key) {
        return false;
    }
    transcript_controls.push(capsule_attached_control_record(
        thread_id,
        run_id,
        &attachment,
        after_content_count,
    ));
    true
}

pub(super) fn take_pending_input_for_ack(
    pending_user_inputs: &mut Vec<PendingUserInput>,
    pending_input_id: Option<&str>,
) -> Option<PendingUserInput> {
    let target_id = pending_input_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(target_id) = target_id
        && let Some(index) = pending_user_inputs
            .iter()
            .position(|input| input.id == target_id)
    {
        return Some(pending_user_inputs.remove(index));
    }

    if pending_user_inputs.is_empty() {
        None
    } else {
        Some(pending_user_inputs.remove(0))
    }
}

#[allow(clippy::too_many_arguments)]
fn process_thread_persistence_command(
    command: ThreadPersistenceCommand,
    snapshot: &mut StreamingRunSnapshot,
    pending_user_inputs: &mut Vec<PendingUserInput>,
    committed_event_run_id: Option<&str>,
    thread_id: &str,
    transcript_controls: &mut Vec<RunControlRecord>,
    abort_terminal_ack: &mut Option<tokio::sync::oneshot::Sender<()>>,
    dirty: &mut bool,
    finish: &mut bool,
    after_commit_callbacks: &mut Vec<(Arc<dyn Fn(StreamEvent) + Send + Sync>, StreamEvent)>,
) {
    match command {
        ThreadPersistenceCommand::Stream {
            event,
            after_commit,
        } => {
            if let Some(callback) = after_commit {
                after_commit_callbacks.push((callback, event.clone()));
            }
            match event {
                StreamEvent::Boundary {
                    kind: garyx_models::provider::StreamBoundaryKind::UserAck,
                    ref pending_input_id,
                } => {
                    snapshot.apply_stream_event(&event);
                    if let Some(pending_input) =
                        take_pending_input_for_ack(pending_user_inputs, pending_input_id.as_deref())
                    {
                        *dirty |= snapshot.acknowledge_pending_input(&pending_input);
                    }
                    if let Some(run_id) = committed_event_run_id
                        && let Some(control) = control_record_for_stream_event(
                            thread_id,
                            run_id,
                            &event,
                            1 + snapshot.session_messages.len(),
                        )
                    {
                        transcript_controls.push(control);
                        *dirty = true;
                    }
                }
                other => {
                    *dirty |= snapshot.apply_stream_event(&other);
                    if let Some(run_id) = committed_event_run_id
                        && let StreamEvent::ToolResult { message } = &other
                    {
                        *dirty |= maybe_push_capsule_attachment_control(
                            thread_id,
                            run_id,
                            snapshot,
                            message,
                            transcript_controls,
                        );
                    }
                    if let Some(run_id) = committed_event_run_id
                        && let Some(control) = control_record_for_stream_event(
                            thread_id,
                            run_id,
                            &other,
                            1 + snapshot.session_messages.len(),
                        )
                    {
                        transcript_controls.push(control);
                        *dirty = true;
                    }
                }
            }
        }
        ThreadPersistenceCommand::QueuePendingInput(pending_input) => {
            pending_user_inputs.push(pending_input);
            *dirty = true;
        }
        ThreadPersistenceCommand::DropPendingInput { pending_input_id } => {
            let before = pending_user_inputs.len();
            pending_user_inputs.retain(|input| input.id != pending_input_id);
            *dirty |= pending_user_inputs.len() != before;
        }
        ThreadPersistenceCommand::AbortTerminal { error, ack } => {
            if let Some(run_id) = committed_event_run_id {
                transcript_controls.push(abort_terminal_control_record(
                    thread_id,
                    run_id,
                    1 + snapshot.session_messages.len(),
                    error.as_deref(),
                ));
                *dirty = true;
            }
            *abort_terminal_ack = Some(ack);
            *finish = true;
        }
        ThreadPersistenceCommand::Finish => {
            *finish = true;
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_partial_thread_persistence_worker(
    store: Arc<dyn ThreadStore>,
    history: Arc<ThreadHistoryRepository>,
    thread_id: String,
    user_message: String,
    user_timestamp: String,
    user_images: Vec<ImagePayload>,
    provider_key: String,
    provider_type: ProviderType,
    metadata: HashMap<String, Value>,
    gateway_event_tx: Option<tokio::sync::broadcast::Sender<String>>,
) -> (
    mpsc::UnboundedSender<ThreadPersistenceCommand>,
    JoinHandle<StreamingPersistenceWorkerResult>,
) {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<ThreadPersistenceCommand>();
    let task = tokio::spawn(async move {
        let mut snapshot = StreamingRunSnapshot::default();
        let mut pending_user_inputs = Vec::<PendingUserInput>::new();
        // Running count of finalized rows already appended to the committed
        // transcript for this run (F1 real-time append cursor).
        let mut appended_finalized: usize = 0;
        // Run id stamped on the live `committed_message` events (S5). The
        // per-thread stream filters by thread_id; run_id is informational.
        let committed_event_run_id = metadata
            .get("bridge_run_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut transcript_controls = Vec::<RunControlRecord>::new();
        if let Some(run_id) = committed_event_run_id.as_deref() {
            let mut payload = serde_json::Map::new();
            if !provider_key.trim().is_empty() {
                payload.insert(
                    "provider_key".to_owned(),
                    Value::String(provider_key.clone()),
                );
            }
            payload.insert(
                "provider_type".to_owned(),
                serde_json::to_value(&provider_type).unwrap_or(Value::Null),
            );
            transcript_controls.push(RunControlRecord::new(
                "run_start",
                &thread_id,
                run_id,
                user_timestamp.clone(),
                payload,
                0,
            ));
        }
        // Publish each row this flush committed to the jsonl as a seq'd
        // `committed_message` on the gateway bus, AFTER the append flushed
        // (write-then-emit): the in-memory event never references a seq the file
        // does not yet have, so a reconnect replay can never miss it.
        let emit_committed = |committed: Vec<(u64, Value)>| {
            emit_committed_records(
                &gateway_event_tx,
                &thread_id,
                committed_event_run_id.as_deref(),
                committed,
            );
        };

        let (cursor, committed) = save_streaming_partial(
            &store,
            &history,
            PersistedRun {
                thread_id: &thread_id,
                user_message: &user_message,
                user_timestamp: Some(&user_timestamp),
                user_images: &user_images,
                assistant_response: "",
                sdk_session_id: None,
                provider_key: &provider_key,
                provider_type: provider_type.clone(),
                session_messages: &[],
                metadata: &metadata,
            },
            &pending_user_inputs,
            &transcript_controls,
            0,
            appended_finalized,
        )
        .await;
        appended_finalized = cursor;
        emit_committed(committed);

        let mut abort_terminal_ack: Option<tokio::sync::oneshot::Sender<()>> = None;
        while let Some(command) = event_rx.recv().await {
            let mut dirty = false;
            let mut finish = false;
            let mut after_commit_callbacks = Vec::new();
            process_thread_persistence_command(
                command,
                &mut snapshot,
                &mut pending_user_inputs,
                committed_event_run_id.as_deref(),
                &thread_id,
                &mut transcript_controls,
                &mut abort_terminal_ack,
                &mut dirty,
                &mut finish,
                &mut after_commit_callbacks,
            );
            while let Ok(pending) = event_rx.try_recv() {
                process_thread_persistence_command(
                    pending,
                    &mut snapshot,
                    &mut pending_user_inputs,
                    committed_event_run_id.as_deref(),
                    &thread_id,
                    &mut transcript_controls,
                    &mut abort_terminal_ack,
                    &mut dirty,
                    &mut finish,
                    &mut after_commit_callbacks,
                );
            }
            if dirty {
                let (cursor, committed) = save_streaming_partial(
                    &store,
                    &history,
                    PersistedRun {
                        thread_id: &thread_id,
                        user_message: &user_message,
                        user_timestamp: Some(&user_timestamp),
                        user_images: &user_images,
                        assistant_response: &snapshot.assistant_response,
                        sdk_session_id: snapshot.sdk_session_id.as_deref(),
                        provider_key: &provider_key,
                        provider_type: provider_type.clone(),
                        session_messages: &snapshot.session_messages,
                        metadata: &metadata,
                    },
                    &pending_user_inputs,
                    &transcript_controls,
                    snapshot.finalized_len(),
                    appended_finalized,
                )
                .await;
                appended_finalized = cursor;
                emit_committed(committed);
            }
            for (callback, event) in after_commit_callbacks {
                callback(event);
            }
            if finish {
                break;
            }
        }

        for pending_input in &mut pending_user_inputs {
            if pending_input.status == PendingUserInputStatus::Queued {
                pending_input.status = PendingUserInputStatus::Abandoned;
            }
        }
        {
            // Final flush at run end. The whole session is finalized now (the run
            // ended, so the trailing assistant is no longer in-flight), so commit
            // the FULL length — this commits + emits the last segment that the
            // periodic streaming flush has not finalized yet, closing the crash window AND
            // delivering it as a seq'd `committed_message` so the client's cursor
            // advances continuously rather than waiting for the next reconnect.
            let (_, committed) = save_streaming_partial(
                &store,
                &history,
                PersistedRun {
                    thread_id: &thread_id,
                    user_message: &user_message,
                    user_timestamp: Some(&user_timestamp),
                    user_images: &user_images,
                    assistant_response: &snapshot.assistant_response,
                    sdk_session_id: snapshot.sdk_session_id.as_deref(),
                    provider_key: &provider_key,
                    provider_type: provider_type.clone(),
                    session_messages: &snapshot.session_messages,
                    metadata: &metadata,
                },
                &pending_user_inputs,
                &transcript_controls,
                snapshot.session_messages.len(),
                appended_finalized,
            )
            .await;
            emit_committed(committed);
        }
        if let Some(ack) = abort_terminal_ack {
            let _ = ack.send(());
        }

        StreamingPersistenceWorkerResult {
            assistant_response: snapshot.assistant_response,
            session_messages: snapshot.session_messages,
            transcript_controls,
        }
    });

    (event_tx, task)
}
