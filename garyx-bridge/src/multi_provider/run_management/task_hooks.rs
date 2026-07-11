use super::*;

pub(super) fn is_task_work_run_wake(run_id: &str, metadata: &HashMap<String, Value>) -> bool {
    !run_id.starts_with("task-notify-")
        && !metadata_bool(metadata, "task_notification")
        && !metadata_bool(metadata, "internal_dispatch")
        && !metadata_bool(metadata, "system")
}

pub(super) async fn mark_task_in_progress_on_work_run_wake(
    inner: &super::state::Inner,
    thread_id: &str,
    run_id: &str,
    metadata: &HashMap<String, Value>,
    thread_logs: Option<Arc<dyn ThreadLogSink>>,
    thread_log_id: Option<&str>,
) {
    if !is_task_work_run_wake(run_id, metadata) {
        return;
    }
    let Some(store) = inner.thread_store.read().await.clone() else {
        return;
    };
    match mark_thread_task_in_progress_on_wake(
        &store,
        thread_id,
        Principal::Agent {
            agent_id: "garyx".to_owned(),
        },
    )
    .await
    {
        Ok(Some(task)) => {
            let task_id = garyx_router::tasks::canonical_task_id(&task);
            record_thread_log(
                thread_logs,
                thread_log_id,
                ThreadLogEvent::info("", "task", "task moved to in progress after work run wake")
                    .with_run_id(run_id.to_owned())
                    .with_field("task_id", json!(task_id)),
            )
            .await;
        }
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(
                thread_id = %thread_id,
                run_id = %run_id,
                error = %error,
                "failed to revive task for work run wake"
            );
            record_thread_log(
                thread_logs,
                thread_log_id,
                ThreadLogEvent::warn("", "task", "failed to revive task for work run wake")
                    .with_run_id(run_id.to_owned())
                    .with_field("error", json!(error.to_string())),
            )
            .await;
        }
    }
}

pub(super) async fn emit_task_ready_for_review_event(
    inner: &super::state::Inner,
    thread_id: &str,
    run_id: &str,
    task_id: &str,
    notification_text: Option<&str>,
) {
    if let Some(tx) = &*inner.event_tx.read().await {
        let event = serde_json::json!({
            "type": "task_ready_for_review",
            "thread_id": thread_id,
            "run_id": run_id,
            "task_id": task_id,
            "handoff": notification_text
                .map(str::trim)
                .filter(|value| !value.is_empty()),
        });
        let _ = tx.send(event.to_string());
    }
}

pub(super) async fn mark_task_ready_for_review_after_stopped_run(
    inner: &super::state::Inner,
    thread_id: &str,
    run_id: &str,
    gate_response: Option<&str>,
    allow_transition: bool,
    thread_logs: Option<Arc<dyn ThreadLogSink>>,
    thread_log_id: Option<&str>,
) {
    let has_gate_response = gate_response
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some();
    if allow_transition && !has_gate_response {
        record_thread_log(
            thread_logs.clone(),
            thread_log_id,
            ThreadLogEvent::info(
                "",
                "task",
                "task run stopped without final response; leaving task in progress",
            )
            .with_run_id(run_id.to_owned())
            .with_field("thread_id", json!(thread_id)),
        )
        .await;
    }
    let Some(store) = inner.thread_store.read().await.clone() else {
        return;
    };

    if allow_transition && has_gate_response {
        let handoff = gate_response
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        match mark_thread_task_in_review_if_in_progress(
            &store,
            thread_id,
            Principal::Agent {
                agent_id: "garyx".to_owned(),
            },
            Some("agent run stopped".to_owned()),
            handoff,
        )
        .await
        {
            Ok(Some(transition)) => {
                let task_id = garyx_router::tasks::canonical_task_id(&transition.task);
                emit_task_ready_for_review_event(
                    inner,
                    thread_id,
                    run_id,
                    &task_id,
                    transition.handoff.as_deref(),
                )
                .await;
                record_thread_log(
                    thread_logs.clone(),
                    thread_log_id,
                    ThreadLogEvent::info("", "task", "task moved to review after run stopped")
                        .with_run_id(run_id.to_owned())
                        .with_field("task_id", json!(task_id)),
                )
                .await;
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(
                    thread_id = %thread_id,
                    run_id = %run_id,
                    error = %error,
                    "failed to move stopped task to review"
                );
                record_thread_log(
                    thread_logs.clone(),
                    thread_log_id,
                    ThreadLogEvent::warn("", "task", "failed to move stopped task to review")
                        .with_run_id(run_id.to_owned())
                        .with_field("error", json!(error.to_string())),
                )
                .await;
            }
        }
    }
}

pub(super) async fn final_task_handoff_for_stopped_run(
    inner: &super::state::Inner,
    thread_id: &str,
    run_id: &str,
    fallback_response: &str,
) -> Option<String> {
    final_answer_from_committed_run_tail(inner, thread_id, run_id)
        .await
        .or_else(|| non_empty_trimmed_owned(fallback_response))
}

pub(super) async fn final_answer_from_committed_run_tail(
    inner: &super::state::Inner,
    thread_id: &str,
    run_id: &str,
) -> Option<String> {
    let trimmed_run_id = run_id.trim();
    if trimmed_run_id.is_empty() {
        return None;
    }
    let history = inner.thread_history.read().await.clone()?;
    let records = history.transcript_store().records(thread_id).await.ok()?;
    let mut run_tail = Vec::new();
    for record in records.into_iter().rev() {
        if record.run_id.as_deref().map(str::trim) == Some(trimmed_run_id) {
            run_tail.push(record);
        } else if !run_tail.is_empty() {
            break;
        }
    }
    if run_tail.is_empty() {
        return None;
    }
    run_tail.reverse();
    let values = run_tail
        .iter()
        .filter_map(|record| serde_json::to_value(record).ok())
        .collect::<Vec<_>>();
    final_assistant_text_from_render_records(&values)
}
