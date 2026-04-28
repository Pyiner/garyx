use super::super::*;

pub(crate) async fn run(
    server: &GaryMcpServer,
    ctx: RequestContext<RoleServer>,
) -> Result<String, String> {
    let started = Instant::now();
    let run_ctx = RunContext::from_request_context(&ctx);
    let result = server
        .status_payload(run_ctx)
        .await
        .map(|value| serde_json::to_string(&value).unwrap_or_default());

    server.record_tool_metric(
        "status",
        if result.is_ok() { "ok" } else { "error" },
        started.elapsed(),
    );
    result
}
