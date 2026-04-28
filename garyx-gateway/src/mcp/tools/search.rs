use super::super::*;

pub(crate) async fn run(server: &GaryMcpServer, params: SearchParams) -> Result<String, String> {
    let started = Instant::now();

    let query = params.query.trim().to_owned();
    if query.is_empty() {
        server.record_tool_metric("search", "error", started.elapsed());
        return Err("query is required".to_owned());
    }

    let config_snapshot = server.app_state.config_snapshot();
    let configured_api_key = config_snapshot.gateway.search.api_key.clone();
    let configured_model = config_snapshot.gateway.search.model.clone();

    let search_result =
        match GaryMcpServer::run_search(&query, configured_api_key.trim(), configured_model.trim())
            .await
        {
            Ok(v) => v,
            Err(e) => json!({
                "success": false,
                "error": e,
            }),
        };

    let success = search_result
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let status = if success { "ok" } else { "error" };
    server.record_tool_metric("search", status, started.elapsed());

    Ok(serde_json::to_string(&json!({
        "tool": "search",
        "status": status,
        "query": query,
        "result": search_result,
    }))
    .unwrap_or_default())
}
