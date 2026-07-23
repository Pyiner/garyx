use serde_json::{Value, json};

use super::{gateway_endpoint, post_gateway_json};

pub(crate) fn push_send_request(
    title: String,
    body: String,
    thread_id: Option<String>,
) -> Result<Value, Box<dyn std::error::Error>> {
    if title.trim().is_empty() {
        return Err("--title must not be empty".into());
    }
    if body.trim().is_empty() {
        return Err("--body must not be empty".into());
    }
    let thread_id = thread_id
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let mut request = json!({
        "title": title,
        "body": body,
    });
    if let Some(thread_id) = thread_id {
        request["thread_id"] = Value::String(thread_id);
    }
    Ok(request)
}

pub(crate) async fn cmd_push_send(
    config_path: &str,
    title: String,
    body: String,
    thread_id: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let request = push_send_request(title, body, thread_id)?;
    let gateway = gateway_endpoint(config_path)?;
    let response = post_gateway_json(&gateway, "/api/push/send", &request).await?;
    let sent = response.get("sent").and_then(Value::as_u64).unwrap_or(0);
    let failed = response.get("failed").and_then(Value::as_u64).unwrap_or(0);
    if response
        .get("no_devices")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        println!("No registered iOS devices; nothing was sent.");
    } else {
        println!("Push delivery: {sent} sent, {failed} failed.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_mapping_preserves_content_and_omits_missing_thread() {
        assert_eq!(
            push_send_request("  Build done  ".to_owned(), "Open Garyx".to_owned(), None,).unwrap(),
            json!({"title": "  Build done  ", "body": "Open Garyx"})
        );
    }

    #[test]
    fn request_mapping_includes_trimmed_thread_id() {
        assert_eq!(
            push_send_request(
                "Build done".to_owned(),
                "Open Garyx".to_owned(),
                Some(" thread::synthetic ".to_owned()),
            )
            .unwrap(),
            json!({
                "title": "Build done",
                "body": "Open Garyx",
                "thread_id": "thread::synthetic"
            })
        );
    }
}
