use super::*;

pub(super) async fn send_process_request(
    stdin: &mut ChildStdin,
    id: u64,
    method: &str,
    params: Value,
) -> Result<(), String> {
    let payload = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    stdin
        .write_all(payload.to_string().as_bytes())
        .await
        .map_err(|error| format!("failed to write provider process request: {error}"))?;
    stdin
        .write_all(b"\n")
        .await
        .map_err(|error| format!("failed to finish provider process request: {error}"))?;
    stdin
        .flush()
        .await
        .map_err(|error| format!("failed to flush provider process request: {error}"))
}

pub(super) async fn send_process_notification(
    stdin: &mut ChildStdin,
    method: &str,
    params: Value,
) -> Result<(), String> {
    let payload = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    });
    stdin
        .write_all(payload.to_string().as_bytes())
        .await
        .map_err(|error| format!("failed to write {method} notification: {error}"))?;
    stdin
        .write_all(b"\n")
        .await
        .map_err(|error| format!("failed to finish {method} notification: {error}"))?;
    stdin
        .flush()
        .await
        .map_err(|error| format!("failed to flush {method} notification: {error}"))
}

pub(super) async fn read_process_response(
    lines: &mut Lines<BufReader<ChildStdout>>,
    expected_id: u64,
    duration: Duration,
) -> Result<Value, String> {
    let future = async {
        loop {
            let Some(line) = lines
                .next_line()
                .await
                .map_err(|error| format!("failed to read provider process response: {error}"))?
            else {
                return Err("provider process closed before responding".to_owned());
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let value = serde_json::from_str::<Value>(trimmed)
                .map_err(|error| format!("invalid provider process JSON response: {error}"))?;
            if value.get("id").and_then(Value::as_u64) == Some(expected_id) {
                return Ok(value);
            }
        }
    };
    timeout(duration, future)
        .await
        .map_err(|_| format!("timed out waiting for provider process request {expected_id}"))?
}

pub(super) async fn shutdown_child(child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

pub(super) fn process_error_code(response: &Value) -> Option<i64> {
    response
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_i64)
}

pub(super) fn process_error_message(response: &Value) -> Option<String> {
    let error = response.get("error")?;
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown error");
    let details = error
        .get("data")
        .and_then(|data| data.get("details"))
        .and_then(Value::as_str);
    Some(match details {
        Some(details) if !details.is_empty() => format!("{message} ({details})"),
        _ => message.to_owned(),
    })
}
