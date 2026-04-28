use super::super::*;
use serde_json::json;

const AUTO_RESEARCH_VERIFIER_ROLE: &str = "verifier";

pub(crate) async fn verdict_payload(
    server: &GaryMcpServer,
    run_ctx: RunContext,
    params: AutoResearchVerdictParams,
) -> Result<serde_json::Value, String> {
    let thread_id = run_ctx
        .thread_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "auto_research_verdict requires a thread_id in MCP context".to_owned())?;

    let thread_authorized = server
        .app_state
        .ops
        .auto_research
        .is_authorized_verifier_thread(thread_id)
        .await;
    if run_ctx.auto_research_role.as_deref() != Some(AUTO_RESEARCH_VERIFIER_ROLE)
        && !thread_authorized
    {
        return Err(
            "auto_research_verdict is only available to AutoResearch verifier runs".to_owned(),
        );
    }

    if !(thread_id.contains("thread::auto-research::")
        && (thread_id.contains("::verify::") || thread_id.contains("::reverify::")))
    {
        return Err("auto_research_verdict requires an AutoResearch verify thread".to_owned());
    }

    let verdict = crate::auto_research::validate_verdict(params.into())?;
    let score = verdict.score;
    server
        .app_state
        .ops
        .auto_research
        .submit_verifier_verdict(thread_id, verdict)
        .await;

    Ok(json!({
        "tool": "auto_research_verdict",
        "status": "ok",
        "stored_for_thread_id": thread_id,
        "score": score,
    }))
}

pub(crate) async fn run_verdict(
    server: &GaryMcpServer,
    ctx: RequestContext<RoleServer>,
    params: AutoResearchVerdictParams,
) -> Result<String, String> {
    let payload = verdict_payload(server, RunContext::from_request_context(&ctx), params).await?;
    serde_json::to_string(&payload).map_err(|error| error.to_string())
}
