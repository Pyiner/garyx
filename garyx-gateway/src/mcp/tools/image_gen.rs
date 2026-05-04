use super::super::*;

pub(crate) async fn run(server: &GaryMcpServer, params: ImageGenParams) -> Result<String, String> {
    let started = Instant::now();
    if let Some(size) = params.size.as_deref()
        && !["256x256", "512x512", "1024x1024"].contains(&size)
    {
        server.record_tool_metric("image_gen", "error", started.elapsed());
        return Err(format!("invalid size: {size}"));
    }

    let aspect_ratio = params.aspect_ratio.as_deref().unwrap_or("1:1");
    let image_size = params.image_size.as_deref().unwrap_or("2K");
    let reference_images = params.reference_images.clone().unwrap_or_default();
    let config_snapshot = server.app_state.config_snapshot();
    let configured_api_key = config_snapshot.gateway.image_gen.api_key.clone();
    let configured_model = config_snapshot.gateway.image_gen.model.clone();

    let image_gen_result = match GaryMcpServer::run_image_gen(
        &params.prompt,
        aspect_ratio,
        image_size,
        &reference_images,
        configured_api_key.trim(),
        configured_model.trim(),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => json!({
            "success": false,
            "error": e,
        }),
    };

    let success = image_gen_result
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let status = if success { "ok" } else { "error" };
    server.record_tool_metric("image_gen", status, started.elapsed());

    Ok(serde_json::to_string(&json!({
        "tool": "image_gen",
        "status": status,
        "prompt": params.prompt,
        "aspect_ratio": aspect_ratio,
        "image_size": image_size,
        "reference_images": reference_images,
        "result": image_gen_result,
    }))
    .unwrap_or_default())
}
