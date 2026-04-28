use super::super::*;
use garyx_models::config::AutomationScheduleView;
use garyx_models::config::{CronJobConfig, CronSchedule};
use serde_json::Value;

fn trimmed_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn automation_schedule_value(
    schedule: &CronSchedule,
    ui_schedule: &Option<AutomationScheduleView>,
) -> Value {
    if let Some(schedule) = ui_schedule {
        return serde_json::to_value(schedule).unwrap_or(Value::Null);
    }
    crate::automation::infer_schedule_view(schedule)
        .ok()
        .and_then(|schedule| serde_json::to_value(schedule).ok())
        .unwrap_or(Value::Null)
}

fn runtime_next_run_value(job: &crate::cron::CronJob) -> String {
    match &job.schedule {
        CronSchedule::Once { at } => crate::cron::parse_once_timestamp(at)
            .map(|timestamp| timestamp.to_rfc3339())
            .unwrap_or_else(|| job.next_run.to_rfc3339()),
        _ => job.next_run.to_rfc3339(),
    }
}

fn runtime_job_json(job: &crate::cron::CronJob) -> Value {
    json!({
        "id": job.id,
        "kind": job.kind,
        "label": job.label,
        "schedule": job.schedule,
        "schedule_view": automation_schedule_value(&job.schedule, &job.ui_schedule),
        "job_action": job.action,
        "enabled": job.enabled,
        "target": job.target,
        "message": job.message,
        "prompt": job.message,
        "agent_id": crate::automation::automation_agent_id(job),
        "workspace_dir": job.workspace_dir,
        "delete_after_run": job.delete_after_run,
        "next_run": runtime_next_run_value(job),
        "last_status": job.last_status,
        "run_count": job.run_count,
    })
}

fn config_job_json(job: &CronJobConfig) -> Value {
    json!({
        "id": job.id,
        "kind": job.kind,
        "label": job.label,
        "schedule": job.schedule,
        "schedule_view": automation_schedule_value(&job.schedule, &job.ui_schedule),
        "job_action": job.action,
        "enabled": job.enabled,
        "target": job.target,
        "message": job.message,
        "prompt": job.message,
        "agent_id": trimmed_non_empty(job.agent_id.as_deref())
            .unwrap_or_else(|| crate::automation::DEFAULT_AUTOMATION_AGENT_ID.to_owned()),
        "workspace_dir": job.workspace_dir,
        "delete_after_run": job.delete_after_run,
    })
}

fn ensure_automation_compatible_params(params: &CronParams) -> Result<(), String> {
    if let Some(action) = params
        .job_action
        .as_deref()
        .or(params.cron_action.as_deref())
        .filter(|value| !value.trim().is_empty())
    {
        if GaryMcpServer::parse_cron_action(params)? != CronAction::AgentTurn {
            return Err(format!(
                "MCP cron add/update now creates automation jobs; unsupported job_action: {action}"
            ));
        }
    }
    if trimmed_non_empty(params.target.as_deref()).is_some() {
        return Err(
            "MCP cron add/update now creates automation jobs; `target` is not supported".to_owned(),
        );
    }
    if params.delete_after_run == Some(true) {
        return Err(
            "MCP cron add/update now creates automation jobs; `delete_after_run` is not supported"
                .to_owned(),
        );
    }
    Ok(())
}

fn automation_prompt_from_params(
    params: &CronParams,
    current: Option<&str>,
) -> Result<String, String> {
    trimmed_non_empty(params.prompt.as_deref())
        .or_else(|| trimmed_non_empty(params.message.as_deref()))
        .or_else(|| trimmed_non_empty(current))
        .ok_or_else(|| "missing required parameter: prompt".to_owned())
}

fn automation_label_from_params(
    params: &CronParams,
    job_id: &str,
    current: Option<&str>,
) -> String {
    trimmed_non_empty(params.label.as_deref())
        .or_else(|| trimmed_non_empty(current))
        .unwrap_or_else(|| job_id.to_owned())
}

fn automation_workspace_from_params(
    params: &CronParams,
    current: Option<&str>,
) -> Result<String, String> {
    trimmed_non_empty(params.workspace_dir.as_deref())
        .or_else(|| trimmed_non_empty(current))
        .ok_or_else(|| "missing required parameter: workspace_dir".to_owned())
}

async fn automation_agent_from_params(
    server: &GaryMcpServer,
    params: &CronParams,
    current: Option<&str>,
) -> Result<String, String> {
    crate::automation::resolve_automation_agent_id(
        &server.app_state,
        params.agent_id.as_deref(),
        current,
    )
    .await
}

fn has_schedule_override(params: &CronParams) -> bool {
    params.schedule_view.is_some()
        || params.schedule.is_some()
        || params.interval_secs.is_some()
        || params.at.is_some()
}

fn automation_schedule_from_params(params: &CronParams) -> Result<AutomationScheduleView, String> {
    if let Some(raw) = &params.schedule_view {
        return serde_json::from_value::<AutomationScheduleView>(raw.clone())
            .map_err(|error| format!("invalid schedule_view: {error}"));
    }
    if let Some(raw) = &params.schedule {
        if let Ok(schedule) = serde_json::from_value::<AutomationScheduleView>(raw.clone()) {
            return Ok(schedule);
        }
    }
    let schedule = GaryMcpServer::parse_schedule(params)?;
    crate::automation::infer_schedule_view(&schedule)
}

fn current_automation_schedule(
    job: &crate::cron::CronJob,
) -> Result<AutomationScheduleView, String> {
    if let Some(schedule) = job.ui_schedule.clone() {
        return Ok(schedule);
    }
    crate::automation::infer_schedule_view(&job.schedule)
}

pub(crate) async fn run(server: &GaryMcpServer, params: CronParams) -> Result<String, String> {
    let started = Instant::now();
    let result = async {
        let state = &server.app_state;
        let cfg = state.config_snapshot();
        let action = params.action.trim().to_ascii_lowercase();
        match action.as_str() {
            "list" => {
                if let Some(svc) = &state.ops.cron_service {
                    let jobs = svc.list().await;
                    let jobs: Vec<Value> =
                        jobs.into_iter().map(|job| runtime_job_json(&job)).collect();
                    Ok(serde_json::to_string(&json!({
                        "tool": "cron", "action": "list", "status": "ok",
                        "service_available": true, "count": jobs.len(), "jobs": jobs,
                    }))
                    .unwrap_or_default())
                } else {
                    let jobs: Vec<Value> = cfg.cron.jobs.iter().map(config_job_json).collect();
                    Ok(serde_json::to_string(&json!({
                        "tool": "cron", "action": "list", "status": "ok",
                        "service_available": false, "count": jobs.len(), "jobs": jobs,
                    }))
                    .unwrap_or_default())
                }
            }
            "status" => {
                let id = params
                    .job_id
                    .as_deref()
                    .ok_or("missing required parameter: job_id")?;
                if let Some(svc) = &state.ops.cron_service {
                    let job = svc
                        .list()
                        .await
                        .into_iter()
                        .find(|job| job.id == id)
                        .ok_or_else(|| format!("Cron job not found: {id}"))?;
                    Ok(serde_json::to_string(&json!({
                        "tool": "cron", "action": "status", "status": "ok",
                        "service_available": true, "job": runtime_job_json(&job),
                    }))
                    .unwrap_or_default())
                } else {
                    let job = cfg
                        .cron
                        .jobs
                        .iter()
                        .find(|job| job.id == id)
                        .ok_or_else(|| format!("Cron job not found: {id}"))?;
                    Ok(serde_json::to_string(&json!({
                        "tool": "cron", "action": "status", "status": "ok",
                        "service_available": false, "job": config_job_json(job),
                    }))
                    .unwrap_or_default())
                }
            }
            "add" => {
                let svc = state
                    .ops
                    .cron_service
                    .as_ref()
                    .ok_or("cron service unavailable")?;
                let id = params
                    .job_id
                    .as_deref()
                    .ok_or("missing required parameter: job_id")?;
                ensure_automation_compatible_params(&params)?;
                let label = automation_label_from_params(&params, id, None);
                let prompt = automation_prompt_from_params(&params, None)?;
                let agent_id = automation_agent_from_params(server, &params, None).await?;
                let workspace_dir = automation_workspace_from_params(&params, None)?;
                let schedule = automation_schedule_from_params(&params)?;
                let cfg = crate::automation::build_automation_job(
                    id,
                    &label,
                    &prompt,
                    &agent_id,
                    &workspace_dir,
                    schedule,
                    params.enabled.unwrap_or(true),
                )?;
                let created = svc
                    .add(cfg)
                    .await
                    .map_err(|error| format!("cron add failed: {error}"))?;
                Ok(serde_json::to_string(&json!({
                    "tool": "cron", "action": "add", "status": "ok",
                    "job": runtime_job_json(&created),
                }))
                .unwrap_or_default())
            }
            "update" => {
                let svc = state
                    .ops
                    .cron_service
                    .as_ref()
                    .ok_or("cron service unavailable")?;
                let id = params
                    .job_id
                    .as_deref()
                    .ok_or("missing required parameter: job_id")?;
                let current = svc
                    .list()
                    .await
                    .into_iter()
                    .find(|job| job.id == id)
                    .ok_or_else(|| format!("Cron job not found: {id}"))?;

                ensure_automation_compatible_params(&params)?;
                let label = automation_label_from_params(&params, id, current.label.as_deref());
                let prompt = automation_prompt_from_params(&params, current.message.as_deref())?;
                let agent_id =
                    automation_agent_from_params(server, &params, current.agent_id.as_deref())
                        .await?;
                let workspace_dir =
                    automation_workspace_from_params(&params, current.workspace_dir.as_deref())?;
                let schedule = if has_schedule_override(&params) {
                    automation_schedule_from_params(&params)?
                } else {
                    current_automation_schedule(&current)?
                };
                let cfg = crate::automation::build_automation_job(
                    id,
                    &label,
                    &prompt,
                    &agent_id,
                    &workspace_dir,
                    schedule,
                    params.enabled.unwrap_or(current.enabled),
                )?;
                let updated = svc
                    .update(id, cfg)
                    .await
                    .map_err(|error| format!("cron update failed: {error}"))?
                    .ok_or_else(|| format!("Cron job not found: {id}"))?;
                Ok(serde_json::to_string(&json!({
                    "tool": "cron", "action": "update", "status": "ok",
                    "job": runtime_job_json(&updated),
                }))
                .unwrap_or_default())
            }
            "remove" | "delete" => {
                let svc = state
                    .ops
                    .cron_service
                    .as_ref()
                    .ok_or("cron service unavailable")?;
                let id = params
                    .job_id
                    .as_deref()
                    .ok_or("missing required parameter: job_id")?;
                let deleted = svc
                    .delete(id)
                    .await
                    .map_err(|error| format!("cron remove failed: {error}"))?;
                Ok(serde_json::to_string(&json!({
                    "tool": "cron", "action": "remove", "status": "ok",
                    "job_id": id, "deleted": deleted,
                }))
                .unwrap_or_default())
            }
            "run" | "run_now" => {
                let svc = state
                    .ops
                    .cron_service
                    .as_ref()
                    .ok_or("cron service unavailable")?;
                let id = params
                    .job_id
                    .as_deref()
                    .ok_or("missing required parameter: job_id")?;
                let record = svc
                    .run_now(id)
                    .await
                    .ok_or_else(|| format!("Cron job not found: {id}"))?;
                Ok(serde_json::to_string(&json!({
                    "tool": "cron", "action": action, "status": "ok",
                    "run": {
                        "run_id": record.run_id, "job_id": record.job_id,
                        "started_at": record.started_at.to_rfc3339(),
                        "finished_at": record.finished_at.map(|value| value.to_rfc3339()),
                        "duration_ms": record.duration_ms,
                        "result": record.status, "error": record.error,
                    }
                }))
                .unwrap_or_default())
            }
            other => Err(format!("invalid cron action: {other}")),
        }
    }
    .await;

    server.record_tool_metric(
        "cron",
        if result.is_ok() { "ok" } else { "error" },
        started.elapsed(),
    );
    result
}
