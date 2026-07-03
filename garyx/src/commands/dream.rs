use super::*;

pub(crate) async fn cmd_dream_list(
    config_path: &str,
    from: Option<&str>,
    to: Option<&str>,
    since_hours: i64,
    limit: usize,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let query = dream_query(from, to, since_hours, Some(limit), None)?;
    let gateway = gateway_endpoint(config_path)?;
    let payload = fetch_gateway_json(&gateway, &format!("/api/dreams?{query}")).await?;
    if json_output {
        return print_pretty_json(&payload);
    }
    print_dream_list(&payload);
    Ok(())
}

pub(crate) async fn cmd_dream_scan(
    config_path: &str,
    from: Option<&str>,
    to: Option<&str>,
    since_hours: i64,
    mode: &str,
    limit: usize,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let payload = dream_scan_payload(from, to, since_hours, mode, limit)?;
    let gateway = gateway_endpoint(config_path)?;
    let response =
        post_gateway_json_with_timeout(&gateway, "/api/dreams/scan", &payload, 180).await?;
    if json_output {
        return print_pretty_json(&response);
    }
    if let Some(scan) = response.get("scan") {
        let status = scan
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let topics = scan
            .get("topics_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let spans = scan.get("spans_count").and_then(Value::as_u64).unwrap_or(0);
        println!("Dream scan: {status} ({topics} topics, {spans} spans)");
        if let Some(error) = scan.get("error").and_then(Value::as_str) {
            println!("Extractor note: {error}");
        }
        println!();
    }
    print_dream_list(&response);
    Ok(())
}

pub(crate) async fn cmd_dream_auto(
    config_path: &str,
    state: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let normalized = state.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "on" => {
            put_gateway_json(
                &gateway,
                "/api/settings?merge=true",
                &json!({"dreams": {"enabled": true}}),
            )
            .await?;
        }
        "off" => {
            put_gateway_json(
                &gateway,
                "/api/settings?merge=true",
                &json!({"dreams": {"enabled": false}}),
            )
            .await?;
        }
        "status" => {}
        _ => return Err("dream auto state must be status, on, or off".into()),
    }

    let settings = fetch_gateway_json(&gateway, "/api/settings").await?;
    let dreams = settings.get("dreams").unwrap_or(&Value::Null);
    let enabled = dreams
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let scan_interval_secs = dreams
        .get("scan_interval_secs")
        .and_then(Value::as_u64)
        .unwrap_or(3600);
    let scan_since_hours = dreams
        .get("scan_since_hours")
        .and_then(Value::as_i64)
        .unwrap_or(1);
    let payload = json!({
        "enabled": enabled,
        "scan_interval_secs": scan_interval_secs,
        "scan_since_hours": scan_since_hours,
    });
    if json_output {
        return print_pretty_json(&payload);
    }

    println!(
        "Dream auto scan: {}",
        if enabled { "enabled" } else { "disabled" }
    );
    println!("Interval: {scan_interval_secs}s");
    println!("Lookback: {scan_since_hours}h");
    Ok(())
}

pub(crate) async fn cmd_dream_show(
    config_path: &str,
    dream_id: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let encoded = urlencoding::encode(dream_id);
    let payload = fetch_gateway_json(&gateway, &format!("/api/dreams/{encoded}")).await?;
    if json_output {
        return print_pretty_json(&payload);
    }
    if let Some(dream) = payload.get("dream") {
        print_dream_topic(dream);
        return Ok(());
    }
    println!("Dream: (not found)");
    Ok(())
}

fn dream_query(
    from: Option<&str>,
    to: Option<&str>,
    since_hours: i64,
    limit: Option<usize>,
    mode: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    if since_hours <= 0 {
        return Err("since-hours must be greater than zero".into());
    }
    let mut params = Vec::new();
    if let Some(from) = from.map(str::trim).filter(|value| !value.is_empty()) {
        params.push(("from".to_owned(), from.to_owned()));
    } else {
        params.push(("since_hours".to_owned(), since_hours.to_string()));
    }
    if let Some(to) = to.map(str::trim).filter(|value| !value.is_empty()) {
        params.push(("to".to_owned(), to.to_owned()));
    }
    if let Some(limit) = limit {
        params.push(("limit".to_owned(), limit.to_string()));
    }
    if let Some(mode) = mode.map(str::trim).filter(|value| !value.is_empty()) {
        params.push(("mode".to_owned(), mode.to_owned()));
    }
    Ok(params
        .iter()
        .map(|(key, value)| format!("{key}={}", urlencoding::encode(value)))
        .collect::<Vec<_>>()
        .join("&"))
}

fn dream_scan_payload(
    from: Option<&str>,
    to: Option<&str>,
    since_hours: i64,
    mode: &str,
    limit: usize,
) -> Result<Value, Box<dyn std::error::Error>> {
    if since_hours <= 0 {
        return Err("since-hours must be greater than zero".into());
    }
    let mut obj = serde_json::Map::new();
    if let Some(from) = from.map(str::trim).filter(|value| !value.is_empty()) {
        obj.insert("from".to_owned(), Value::String(from.to_owned()));
    } else {
        obj.insert("since_hours".to_owned(), json!(since_hours));
    }
    if let Some(to) = to.map(str::trim).filter(|value| !value.is_empty()) {
        obj.insert("to".to_owned(), Value::String(to.to_owned()));
    }
    obj.insert("mode".to_owned(), Value::String(mode.trim().to_owned()));
    obj.insert("limit".to_owned(), json!(limit));
    Ok(Value::Object(obj))
}

fn print_dream_list(payload: &Value) {
    let dreams = payload
        .get("dreams")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if dreams.is_empty() {
        println!("Dreams: (none)");
        return;
    }
    for (index, dream) in dreams.iter().enumerate() {
        if index > 0 {
            println!();
        }
        print_dream_topic(dream);
    }
}

fn print_dream_topic(dream: &Value) {
    let dream_id = dream
        .get("dream_id")
        .or_else(|| dream.get("dreamId"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let title = dream
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Untitled Dream");
    println!("{title}");
    if !dream_id.is_empty() {
        println!("  id: {dream_id}");
    }
    let source = dream
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let confidence = dream
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let message_count = dream
        .get("message_count")
        .or_else(|| dream.get("messageCount"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let span_count = dream
        .get("span_count")
        .or_else(|| dream.get("spanCount"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let last_message_at = dream
        .get("last_message_at")
        .or_else(|| dream.get("lastMessageAt"))
        .and_then(Value::as_str)
        .unwrap_or("");
    println!(
        "  {message_count} messages, {span_count} spans, source={source}, confidence={confidence:.2}"
    );
    if !last_message_at.is_empty() {
        println!("  last: {last_message_at}");
    }
    if let Some(summary) = dream
        .get("summary")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        println!("  {summary}");
    }
    if let Some(spans) = dream.get("spans").and_then(Value::as_array) {
        for span in spans {
            let thread_id = span
                .get("thread_id")
                .or_else(|| span.get("threadId"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let start_seq = span
                .get("start_seq")
                .or_else(|| span.get("startSeq"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let end_seq = span
                .get("end_seq")
                .or_else(|| span.get("endSeq"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let excerpt = span
                .get("excerpt")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("");
            println!("  - {thread_id} #{start_seq}-{end_seq}: {excerpt}");
        }
    }
}
