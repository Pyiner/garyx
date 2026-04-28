use super::super::*;
use garyx_models::{MessageLifecycleStatus, MessageTerminalReason};

const RESTART_COOLDOWN_SECS: u64 = 30;

pub(crate) async fn run(
    server: &GaryMcpServer,
    ctx: RequestContext<RoleServer>,
    params: RestartParams,
) -> Result<String, String> {
    let started = Instant::now();
    let result = async {
        let state = &server.app_state;
        let run_ctx = RunContext::from_request_context(&ctx);
        GaryMcpServer::require_auth(state, &run_ctx, params.token.as_deref())?;

        let action = params
            .action
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("restart");
        let normalized_action =
            match action {
                "build" => "build",
                "restart" => "restart",
                // Backward-compatible alias. Semantics are the same as `restart`.
                "build_and_restart" => "restart",
                _ => return Err(
                    "action must be one of: build, restart (build_and_restart accepted as alias)"
                        .to_owned(),
                ),
            };

        let reason = params.reason.as_deref().unwrap_or("no reason provided");
        if normalized_action == "build" {
            crate::restart::build_backend()
                .await
                .map_err(|e| format!("build failed: {e}"))?;
        } else {
            let mut tracker = state.ops.restart_tracker.lock().await;
            if let Some(remaining) = tracker.cooldown_remaining_secs(RESTART_COOLDOWN_SECS) {
                return Err(format!(
                    "restart cooldown active, try again in {remaining}s"
                ));
            }
            tracker.mark_restart_now();
            drop(tracker);

            let continue_thread_id = run_ctx
                .thread_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);

            crate::runtime_diagnostics::record_message_ledger_event(
                state,
                MessageLifecycleStatus::RunInterrupted,
                crate::runtime_diagnostics::RuntimeDiagnosticContext {
                    thread_id: continue_thread_id.clone(),
                    run_id: run_ctx.run_id.clone(),
                    channel: run_ctx.channel.clone(),
                    account_id: run_ctx.account_id.clone(),
                    from_id: run_ctx.from_id.clone(),
                    text_excerpt: Some(format!("restart: {reason}")),
                    terminal_reason: Some(MessageTerminalReason::SelfRestart),
                    metadata: Some(json!({
                        "source": "mcp_restart",
                        "continue_message": crate::restart::default_restart_continuation_message(),
                    })),
                    ..Default::default()
                },
            )
            .await;

            crate::restart::request_restart_with_options(crate::restart::RestartOptions {
                reason: reason.to_owned(),
                build_before_restart: true,
                continue_thread_id,
                continue_run_id: run_ctx.run_id.clone(),
            })
            .await
            .map_err(|e| format!("failed to initiate restart: {e}"))?;
        }

        Ok(serde_json::to_string(&json!({
            "tool": "restart",
            "action": normalized_action,
            "reason": reason,
            "status": "ok",
            "message": if normalized_action == "build" {
                "build completed"
            } else {
                "build completed and restart initiated"
            },
            "run_id": run_ctx.run_id,
            "thread_id": run_ctx.thread_id,
        }))
        .unwrap_or_default())
    }
    .await;

    server.record_tool_metric(
        "restart",
        if result.is_ok() { "ok" } else { "error" },
        started.elapsed(),
    );
    result
}
