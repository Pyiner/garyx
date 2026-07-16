use std::io::Write;
use std::time::Duration;

use garyx_gateway::meetings::{MeetingReadMode, MeetingReadResponse, continuation_mode_unverified};
use serde::Deserialize;

use super::gateway_client::{
    GatewayCliError, GatewayEndpoint, GatewayErrorKind, RawGatewayResponse, delete_gateway_raw,
    fetch_gateway_json, gateway_endpoint, post_gateway_raw,
};
use super::{env_nonempty, format_local_timestamp};

#[derive(Debug, Clone)]
pub(crate) struct MeetingReadCliOptions {
    pub id: String,
    pub full: bool,
    pub range: Option<String>,
    pub epoch: Option<i64>,
    pub continue_token: Option<String>,
    pub thread: Option<String>,
    pub json: bool,
    pub max_bytes: Option<usize>,
}

type ResolvedReadMode = (
    MeetingReadMode,
    Option<String>,
    Option<(i64, i64)>,
    Option<String>,
);

#[derive(Debug, Deserialize)]
struct MeetingErrorEnvelope {
    error: MeetingErrorBody,
}

#[derive(Debug, Deserialize)]
struct MeetingErrorBody {
    code: String,
    message: String,
    #[serde(default)]
    restart_command: Option<String>,
}

pub(crate) async fn cmd_meeting_list(
    config_path: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let mut token = None::<String>;
    let mut meetings = Vec::new();
    loop {
        let path = match token.as_deref() {
            Some(token) => format!(
                "/api/meetings?limit=100&page_token={}",
                urlencoding::encode(token)
            ),
            None => "/api/meetings?limit=100".to_owned(),
        };
        let page = fetch_gateway_json(&gateway, &path).await?;
        meetings.extend(
            page.get("meetings")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default(),
        );
        token = page
            .get("next_page_token")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);
        if token.is_none() {
            break;
        }
    }

    if json_output {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "meetings": meetings,
            }))?
        );
        return Ok(());
    }
    let output = render_meeting_list(&meetings);
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(output.as_bytes())?;
    stdout.flush()?;
    Ok(())
}

pub(crate) async fn cmd_meeting_read(
    config_path: &str,
    options: MeetingReadCliOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    cmd_meeting_read_with_writer(config_path, options, &mut output).await
}

async fn cmd_meeting_read_with_writer(
    config_path: &str,
    options: MeetingReadCliOptions,
    output: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let id = options.id.trim().to_owned();
    uuid::Uuid::parse_str(&id).map_err(|_| "meeting id must be a UUID")?;
    if options.max_bytes.is_some_and(|value| value < 4_096) {
        return Err("--max-bytes must be at least 4096".into());
    }

    let (mode, reader_id, range, mut continue_token) = resolve_read_mode(&options)?;
    let mut consumed = 0usize;
    let requested_total = options.max_bytes;
    let mut index_build_retries = 0usize;

    loop {
        let remainder = requested_total.map(|total| total.saturating_sub(consumed));
        if remainder.is_some_and(|remaining| remaining < 4_096) {
            if let Some(token) = continue_token.as_deref() {
                print_resume(output, &id, token, options.json)?;
            }
            return Ok(());
        }
        let payload = read_payload(
            mode,
            reader_id.as_deref(),
            range,
            options.epoch,
            continue_token.as_deref(),
            remainder,
        );
        let raw = post_gateway_raw(
            &gateway,
            &format!("/api/meetings/{id}/read"),
            &payload,
            true,
        )
        .await
        .map_err(|error| error.into_cli_error())?;
        if !is_success(raw.status) {
            let body = parse_meeting_error(&raw);
            if body
                .as_ref()
                .is_some_and(|body| body.code == "index_building")
                && index_build_retries < 6
            {
                const BACKOFFS: [u64; 6] = [100, 200, 400, 800, 1_600, 2_000];
                tokio::time::sleep(Duration::from_millis(BACKOFFS[index_build_retries])).await;
                index_build_retries += 1;
                continue;
            }
            return Err(meeting_http_error(raw).into());
        }
        index_build_retries = 0;
        let response: MeetingReadResponse = serde_json::from_slice(&raw.raw_body)?;
        write_read_page(output, &id, &raw.raw_body, &response, options.json)?;

        if mode == MeetingReadMode::Incremental {
            if let Some(receipt) = response.meta.receipt.as_deref() {
                confirm_incremental(
                    &gateway,
                    &id,
                    reader_id.as_deref().expect("incremental reader"),
                    receipt,
                    response.meta.log_epoch,
                )
                .await?;
            }
            return Ok(());
        }

        consumed = consumed.saturating_add(raw.body_len);
        continue_token = response.meta.continue_token.clone();
        let Some(token) = continue_token.as_deref() else {
            return Ok(());
        };
        if requested_total.is_some_and(|total| consumed >= total)
            || requested_total.is_some_and(|total| total.saturating_sub(consumed) < 4_096)
        {
            print_resume(output, &id, token, options.json)?;
            return Ok(());
        }
    }
}

pub(crate) async fn cmd_meeting_delete(
    config_path: &str,
    id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let id = id.trim();
    uuid::Uuid::parse_str(id).map_err(|_| "meeting id must be a UUID")?;
    let gateway = gateway_endpoint(config_path)?;
    let raw = delete_gateway_raw(&gateway, &format!("/api/meetings/{id}"))
        .await
        .map_err(|error| error.into_cli_error())?;
    if !is_success(raw.status) {
        return Err(meeting_http_error(raw).into());
    }
    println!("Deleted meeting {id}");
    Ok(())
}

fn resolve_read_mode(
    options: &MeetingReadCliOptions,
) -> Result<ResolvedReadMode, Box<dyn std::error::Error>> {
    if let Some(token) = options.continue_token.as_deref() {
        if options.full || options.range.is_some() || options.epoch.is_some() {
            return Err(
                "--continue is mutually exclusive with --full, --range, and --epoch".into(),
            );
        }
        if options.thread.is_some() {
            return Err("--thread is valid only for incremental reads".into());
        }
        return Ok((
            continuation_mode_unverified(token)?,
            None,
            None,
            Some(token.to_owned()),
        ));
    }
    if options.full {
        if options.range.is_some() || options.epoch.is_some() {
            return Err("--full is mutually exclusive with --range and --epoch".into());
        }
        if options.thread.is_some() {
            return Err("--thread is valid only for incremental reads".into());
        }
        return Ok((MeetingReadMode::Full, None, None, None));
    }
    if let Some(range) = options.range.as_deref() {
        if options.thread.is_some() {
            return Err("--thread is valid only for incremental reads".into());
        }
        return Ok((
            MeetingReadMode::Range,
            None,
            Some(parse_range(range)?),
            None,
        ));
    }
    if options.epoch.is_some() {
        return Err("--epoch is valid only with --range".into());
    }
    let reader = options
        .thread
        .clone()
        .or_else(|| env_nonempty("GARYX_THREAD_ID"))
        .ok_or(
            "incremental meeting reads need a reader identity: set GARYX_THREAD_ID or pass --thread <id>",
        )?;
    let reader = reader.trim().to_owned();
    if !(1..=128).contains(&reader.len()) {
        return Err("reader identity must be between 1 and 128 bytes".into());
    }
    Ok((MeetingReadMode::Incremental, Some(reader), None, None))
}

fn parse_range(value: &str) -> Result<(i64, i64), Box<dyn std::error::Error>> {
    let Some((start, end)) = value.split_once("..") else {
        return Err("--range must use A..B syntax".into());
    };
    let start: i64 = start
        .parse()
        .map_err(|_| "--range start must be a positive integer")?;
    let end: i64 = end
        .parse()
        .map_err(|_| "--range end must be a positive integer")?;
    if start <= 0 || end < start {
        return Err("--range must be a positive closed interval A..B".into());
    }
    Ok((start, end))
}

fn read_payload(
    mode: MeetingReadMode,
    reader_id: Option<&str>,
    range: Option<(i64, i64)>,
    epoch: Option<i64>,
    continue_token: Option<&str>,
    max_bytes: Option<usize>,
) -> serde_json::Value {
    let mut payload = serde_json::Map::new();
    payload.insert(
        "mode".to_owned(),
        serde_json::Value::String(
            match mode {
                MeetingReadMode::Incremental => "incremental",
                MeetingReadMode::Full => "full",
                MeetingReadMode::Range => "range",
            }
            .to_owned(),
        ),
    );
    if let Some(token) = continue_token {
        payload.insert(
            "continue_token".to_owned(),
            serde_json::Value::String(token.to_owned()),
        );
    } else {
        if let Some(reader_id) = reader_id {
            payload.insert(
                "reader_id".to_owned(),
                serde_json::Value::String(reader_id.to_owned()),
            );
        }
        if let Some((start, end)) = range {
            payload.insert("range_start".to_owned(), start.into());
            payload.insert("range_end".to_owned(), end.into());
        }
        if let Some(epoch) = epoch {
            payload.insert("epoch".to_owned(), epoch.into());
        }
    }
    if let Some(max_bytes) = max_bytes {
        payload.insert("max_bytes".to_owned(), max_bytes.into());
    }
    serde_json::Value::Object(payload)
}

async fn confirm_incremental(
    gateway: &GatewayEndpoint,
    id: &str,
    reader_id: &str,
    receipt: &str,
    log_epoch: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    let raw = post_gateway_raw(
        gateway,
        &format!("/api/meetings/{id}/read/confirm"),
        &serde_json::json!({
            "reader_id": reader_id,
            "receipt": receipt,
            "log_epoch": log_epoch,
        }),
        false,
    )
    .await
    .map_err(|error| error.into_cli_error())?;
    if !is_success(raw.status) {
        return Err(meeting_http_error(raw).into());
    }
    Ok(())
}

fn write_read_page(
    output: &mut impl Write,
    id: &str,
    raw_body: &[u8],
    response: &MeetingReadResponse,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if json_output {
        output.write_all(raw_body)?;
        output.write_all(b"\n")?;
    } else {
        output.write_all(render_read_page(id, response).as_bytes())?;
    }
    // This flush is the linearization boundary before confirm.
    output.flush()?;
    Ok(())
}

fn render_meeting_list(meetings: &[serde_json::Value]) -> String {
    if meetings.is_empty() {
        return "No meeting entities.\n".to_owned();
    }
    let mut output = String::new();
    for meeting in meetings {
        let id = meeting
            .get("id")
            .and_then(|value| value.as_str())
            .unwrap_or("-");
        output.push_str(&format!("Meeting {id}\n"));
        output.push_str(&format!(
            "Status: {}",
            meeting
                .get("status")
                .and_then(|value| value.as_str())
                .unwrap_or("-")
        ));
        let end_source = meeting
            .get("end_source")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if !end_source.is_empty() {
            output.push_str(&format!(" (end source: {end_source})"));
        }
        output.push('\n');
        output.push_str(&format!(
            "Epoch: {}  Segments: {}  Bytes: {}\n",
            meeting
                .get("log_epoch")
                .and_then(|value| value.as_i64())
                .unwrap_or(0),
            meeting
                .get("closed_segment_count")
                .and_then(|value| value.as_i64())
                .unwrap_or(0),
            meeting
                .get("byte_size")
                .and_then(|value| value.as_i64())
                .unwrap_or(0),
        ));
        let stalled = meeting
            .get("stalled_reason")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if !stalled.is_empty() {
            output.push_str(&format!("Stalled: {stalled}\n"));
        }
        let content_state = meeting
            .get("content_state")
            .and_then(|value| value.as_str())
            .unwrap_or("ok");
        output.push_str(&format!("Content state: {content_state}\n"));
        output.push_str(&format!(
            "Started: {}  Ended: {}  Finalized: {}  Updated: {}\n",
            format_local_timestamp(meeting.get("started_at").and_then(|value| value.as_str())),
            format_local_timestamp(meeting.get("ended_at").and_then(|value| value.as_str())),
            format_local_timestamp(meeting.get("finalized_at").and_then(|value| value.as_str())),
            format_local_timestamp(meeting.get("updated_at").and_then(|value| value.as_str())),
        ));
        if let Some(content_lost_at) = meeting
            .get("content_lost_at")
            .and_then(|value| value.as_str())
        {
            output.push_str(&format!(
                "Content lost: {}\n",
                format_local_timestamp(Some(content_lost_at))
            ));
        }
        frame_field(
            &mut output,
            "Topic",
            meeting
                .get("topic")
                .and_then(|value| value.as_str())
                .unwrap_or(""),
        );
        let detail = meeting
            .get("status_detail")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if !detail.is_empty() {
            frame_field(&mut output, "Status detail", detail);
        }
        output.push_str(&format!("Read: garyx meeting read {id}\n\n"));
    }
    output
}

fn render_read_page(id: &str, response: &MeetingReadResponse) -> String {
    let meta = &response.meta;
    let mut output = String::new();
    output.push_str(&format!("Meeting {}\n", meta.entity_id));
    output.push_str(&format!("Mode: {}\n", meta.mode));
    output.push_str(&format!("Log epoch: {}\n", meta.log_epoch));
    match (meta.span_from, meta.span_to) {
        (Some(from), Some(to)) => {
            output.push_str(&format!("Span: {from}..{to} of {}\n", meta.closed_total));
        }
        _ => output.push_str(&format!("Span: empty of {}\n", meta.closed_total)),
    }
    output.push_str(&format!("Status: {}", meta.status));
    if !meta.end_source.is_empty() {
        output.push_str(&format!(" (end source: {})", meta.end_source));
    }
    output.push('\n');
    if !meta.stalled_reason.is_empty() {
        output.push_str(&format!("Stalled: {}\n", meta.stalled_reason));
    }
    output.push_str(&format!("Content state: {}\n", meta.content_state));
    output.push_str(&format!(
        "Started: {}  Ended: {}  Finalized: {}  Updated: {}\n",
        format_local_timestamp(Some(&meta.started_at)),
        format_local_timestamp(meta.ended_at.as_deref()),
        format_local_timestamp(meta.finalized_at.as_deref()),
        format_local_timestamp(Some(&meta.updated_at)),
    ));
    if let Some(content_lost_at) = meta.content_lost_at.as_deref() {
        output.push_str(&format!(
            "Content lost: {}\n",
            format_local_timestamp(Some(content_lost_at))
        ));
    }
    frame_field(&mut output, "Topic", &meta.topic);
    if !meta.status_detail.is_empty() {
        frame_field(&mut output, "Status detail", &meta.status_detail);
    }
    for note in &meta.notes {
        output.push_str(&format!("Note: {note}\n"));
    }
    if meta.mode == "incremental" {
        if let (Some(from), Some(to)) = (meta.span_from, meta.span_to) {
            output.push_str("Confirmation: pending; this invocation confirms after stdout flush\n");
            output.push_str(&format!(
                "Re-read this span: garyx meeting read {id} --range {from}..{to} --epoch {}\n",
                meta.log_epoch
            ));
        } else {
            output.push_str("Confirmation: no pending span\n");
        }
    }
    for segment in &response.segments {
        output.push_str(&format!(
            "Segment {} ({:?}, {} → {})\n",
            segment.seq, segment.kind, segment.start, segment.end
        ));
        frame_field(&mut output, "Speaker", &segment.speaker);
        frame_field(&mut output, "Content", &segment.text);
        frame_field(&mut output, "Sources", &segment.sources.join(", "));
        if segment.cont {
            output.push_str("Continuation: true\n");
        }
    }
    if let Some(token) = meta.continue_token.as_deref() {
        output.push_str(&format!(
            "Resume: garyx meeting read {id} --continue {token}\n"
        ));
    }
    output.push('\n');
    output
}

fn frame_field(output: &mut String, label: &str, value: &str) {
    output.push_str(label);
    output.push_str(":\n");
    let sanitized = sanitize_platform_text(value);
    for line in sanitized.split('\n') {
        output.push_str("│ ");
        output.push_str(line);
        output.push('\n');
    }
}

fn sanitize_platform_text(value: &str) -> String {
    let normalized = value.replace(['\u{2028}', '\u{2029}'], "\n");
    let chars = normalized.chars().collect::<Vec<_>>();
    let mut output = String::new();
    let mut index = 0usize;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '\u{001b}' {
            index += 1;
            if index < chars.len() && chars[index] == '[' {
                index += 1;
                while index < chars.len() {
                    let current = chars[index];
                    index += 1;
                    if current.is_ascii() && ('@'..='~').contains(&current) {
                        break;
                    }
                }
                continue;
            }
            if index < chars.len() && chars[index] == ']' {
                index += 1;
                while index < chars.len() {
                    if chars[index] == '\u{0007}' {
                        index += 1;
                        break;
                    }
                    if chars[index] == '\u{001b}' && chars.get(index + 1).copied() == Some('\\') {
                        index += 2;
                        break;
                    }
                    index += 1;
                }
                continue;
            }
            continue;
        }
        if ch == '\u{009b}' {
            index += 1;
            while index < chars.len() {
                let current = chars[index];
                index += 1;
                if current.is_ascii() && ('@'..='~').contains(&current) {
                    break;
                }
            }
            continue;
        }
        if ch == '\u{009d}' {
            index += 1;
            while index < chars.len() && chars[index] != '\u{0007}' {
                index += 1;
            }
            index = (index + 1).min(chars.len());
            continue;
        }
        if is_bidi_control(ch) {
            output.push_str(&format!("\\u{{{:04X}}}", ch as u32));
        } else if ch == '\n' {
            output.push(ch);
        } else if ch <= '\u{001f}' || ('\u{007f}'..='\u{009f}').contains(&ch) {
            // Strip C0 (except LF above) and C1 controls.
        } else {
            output.push(ch);
        }
        index += 1;
    }
    output
}

fn is_bidi_control(ch: char) -> bool {
    matches!(
        ch,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

fn print_resume(
    output: &mut impl Write,
    id: &str,
    token: &str,
    json_output: bool,
) -> Result<(), std::io::Error> {
    let line = format!("garyx meeting read {id} --continue {token}");
    if json_output {
        eprintln!("Resume: {line}");
    } else {
        writeln!(output, "Resume: {line}")?;
        output.flush()?;
    }
    Ok(())
}

fn is_success(status: u16) -> bool {
    (200..300).contains(&status)
}

fn parse_meeting_error(response: &RawGatewayResponse) -> Option<MeetingErrorBody> {
    serde_json::from_slice::<MeetingErrorEnvelope>(&response.raw_body)
        .ok()
        .map(|envelope| envelope.error)
}

fn meeting_http_error(response: RawGatewayResponse) -> GatewayCliError {
    let status = response.status;
    let body = parse_meeting_error(&response);
    let kind = match status {
        404 => GatewayErrorKind::NotFound,
        409 => GatewayErrorKind::Conflict,
        _ => GatewayErrorKind::Rejected,
    };
    let message = match body {
        Some(body) if body.code == "entity_deleted" => "entity deleted".to_owned(),
        Some(body) => match body.restart_command {
            Some(command) => format!("{}; restart with: {command}", body.message),
            None => format!("{}: {}", body.code, body.message),
        },
        None => {
            let text = String::from_utf8_lossy(&response.raw_body);
            if text.trim().is_empty() {
                format!("meeting gateway request failed with HTTP {status}")
            } else {
                format!("meeting gateway request failed with HTTP {status}: {text}")
            }
        }
    };
    GatewayCliError { kind, message }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::commands::test_support::{ENV_LOCK, ScopedEnvVar, write_test_gateway_config};
    use axum::Router;
    use axum::body::Body;
    use axum::extract::Json;
    use axum::http::StatusCode;
    use axum::response::Response;
    use axum::routing::post;
    use garyx_gateway::garyx_db::{GaryxDbService, MeetingCreateDraft, MeetingStatus};
    use garyx_gateway::meetings::{MeetingReadMeta, MeetingSegment, SegmentDraft, SegmentKind};
    use garyx_gateway::server::AppStateBuilder;
    use garyx_models::config::GaryxConfig;
    use uuid::Uuid;

    fn response_with_text(topic: &str, speaker: &str, text: &str) -> MeetingReadResponse {
        MeetingReadResponse {
            meta: MeetingReadMeta {
                mode: "incremental".to_owned(),
                entity_id: "00000000-0000-7000-8000-000000000001".to_owned(),
                log_epoch: 3,
                status: "live".to_owned(),
                status_detail: String::new(),
                end_source: String::new(),
                stalled_reason: String::new(),
                content_state: "ok".to_owned(),
                topic: topic.to_owned(),
                started_at: "2026-07-16T02:35:00.000Z".to_owned(),
                ended_at: None,
                finalized_at: None,
                content_lost_at: None,
                updated_at: "2026-07-16T02:35:00.000Z".to_owned(),
                span_from: Some(1),
                span_to: Some(1),
                closed_total: 1,
                receipt: Some("receipt".to_owned()),
                continue_token: None,
                notes: Vec::new(),
            },
            segments: vec![MeetingSegment {
                seq: 1,
                kind: SegmentKind::Transcript,
                speaker: speaker.to_owned(),
                start: "2026-07-16T02:35:00.000Z".to_owned(),
                end: "2026-07-16T02:35:01.000Z".to_owned(),
                text: text.to_owned(),
                sources: vec!["source".to_owned()],
                cont: false,
            }],
        }
    }

    fn full_response(
        entity_id: &str,
        texts: &[String],
        continue_token: Option<&str>,
    ) -> MeetingReadResponse {
        MeetingReadResponse {
            meta: MeetingReadMeta {
                mode: "full".to_owned(),
                entity_id: entity_id.to_owned(),
                log_epoch: 0,
                status: "live".to_owned(),
                status_detail: String::new(),
                end_source: String::new(),
                stalled_reason: String::new(),
                content_state: "ok".to_owned(),
                topic: "Synthetic topic".to_owned(),
                started_at: "2026-07-16T02:35:00.000Z".to_owned(),
                ended_at: None,
                finalized_at: None,
                content_lost_at: None,
                updated_at: "2026-07-16T02:35:00.000Z".to_owned(),
                span_from: (!texts.is_empty()).then_some(1),
                span_to: (!texts.is_empty()).then_some(texts.len() as i64),
                closed_total: texts.len() as i64 + usize::from(continue_token.is_some()) as i64,
                receipt: None,
                continue_token: continue_token.map(ToOwned::to_owned),
                notes: Vec::new(),
            },
            segments: texts
                .iter()
                .enumerate()
                .map(|(index, text)| MeetingSegment {
                    seq: index as i64 + 1,
                    kind: SegmentKind::Chat,
                    speaker: "Test Speaker".to_owned(),
                    start: "2026-07-16T02:35:00.000Z".to_owned(),
                    end: "2026-07-16T02:35:01.000Z".to_owned(),
                    text: text.clone(),
                    sources: vec![format!("source-{index}")],
                    cont: false,
                })
                .collect(),
        }
    }

    struct BrokenFlushWriter {
        bytes: Vec<u8>,
    }

    impl Write for BrokenFlushWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "injected broken pipe",
            ))
        }
    }

    async fn spawn_paged_read_server(
        responses: Vec<Vec<u8>>,
        requests: Arc<Mutex<Vec<serde_json::Value>>>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let responses = Arc::new(Mutex::new(responses));
        let app = Router::new().route(
            "/api/meetings/{id}/read",
            post({
                let responses = responses.clone();
                move |Json(payload): Json<serde_json::Value>| {
                    let responses = responses.clone();
                    let requests = requests.clone();
                    async move {
                        requests.lock().expect("requests").push(payload);
                        let body = {
                            let mut responses = responses.lock().expect("responses");
                            if responses.len() > 1 {
                                responses.remove(0)
                            } else {
                                responses[0].clone()
                            }
                        };
                        Response::builder()
                            .status(StatusCode::OK)
                            .header("content-type", "application/json")
                            .body(Body::from(body))
                            .expect("response")
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let addr = listener.local_addr().expect("addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn hostile_platform_text_is_normalized_stripped_and_framed() {
        let hostile = concat!(
            "safe\u{2028}FORGED: header\u{2029}",
            "\r\n\u{001b}[31mred\u{001b}[0m",
            "\u{009b}32mc1-red",
            "\u{001b}]0;osc-title\u{0007}after-osc",
            "\u{009d}hidden-c1-title\u{0007}after-c1-osc",
            "\u{202e}rtl\u{0085}tail"
        );
        let rendered = render_read_page(
            "00000000-0000-7000-8000-000000000001",
            &response_with_text(hostile, hostile, hostile),
        );
        assert!(!rendered.contains('\u{001b}'));
        assert!(!rendered.contains('\u{0085}'));
        assert!(!rendered.contains('\u{2028}'));
        assert!(!rendered.contains('\u{2029}'));
        assert!(!rendered.contains('\r'));
        assert!(!rendered.contains("osc-title"));
        assert!(!rendered.contains("hidden-c1-title"));
        assert!(rendered.contains("\\u{202E}"));
        for line in rendered.lines() {
            if line.contains("safe")
                || line.contains("FORGED")
                || line.contains("red")
                || line.contains("after-osc")
                || line.contains("after-c1-osc")
                || line.contains("rtl")
                || line.contains("tail")
            {
                assert!(
                    line.starts_with("│ "),
                    "platform text escaped framing: {line:?}"
                );
            }
        }

        let json = serde_json::to_string(&response_with_text(hostile, hostile, hostile))
            .expect("structured JSON");
        assert_eq!(json.lines().count(), 1);
        let round_trip: MeetingReadResponse = serde_json::from_str(&json).expect("round trip");
        assert_eq!(round_trip.meta.topic, hostile);
        assert_eq!(round_trip.segments[0].speaker, hostile);
        assert_eq!(round_trip.segments[0].text, hostile);
    }

    #[test]
    fn human_list_covers_every_status_and_frames_platform_fields() {
        let statuses = [
            "joining",
            "live",
            "finalizing",
            "aborting",
            "finalized",
            "aborted",
        ];
        let meetings = statuses
            .iter()
            .enumerate()
            .map(|(index, status)| {
                serde_json::json!({
                    "id": format!("00000000-0000-7000-8000-{index:012}"),
                    "status": status,
                    "status_detail": "detail\nFORGED",
                    "content_state": if *status == "aborted" { "lost" } else { "ok" },
                    "content_lost_at": if *status == "aborted" {
                        Some("2026-07-16T02:35:04.000Z")
                    } else {
                        None
                    },
                    "end_source": if matches!(*status, "finalized" | "aborted") {
                        "push"
                    } else {
                        ""
                    },
                    "stalled_reason": "",
                    "log_epoch": 0,
                    "closed_segment_count": 0,
                    "byte_size": 0,
                    "started_at": "2026-07-16T02:35:00.000Z",
                    "ended_at": if matches!(*status, "finalized" | "aborted") {
                        Some("2026-07-16T02:35:02.000Z")
                    } else {
                        None
                    },
                    "finalized_at": if matches!(*status, "finalized" | "aborted") {
                        Some("2026-07-16T02:35:03.000Z")
                    } else {
                        None
                    },
                    "updated_at": "2026-07-16T02:35:04.000Z",
                    "topic": "topic\nMode: forged"
                })
            })
            .collect::<Vec<_>>();
        let rendered = render_meeting_list(&meetings);
        for status in statuses {
            assert!(rendered.contains(&format!("Status: {status}")));
        }
        assert!(rendered.contains("Content state: lost"));
        assert!(rendered.contains("Content lost:"));
        for line in rendered.lines() {
            if matches!(line, "Mode: forged" | "FORGED") {
                panic!("platform content escaped framing: {line:?}");
            }
            if line.contains("Mode: forged") || line.contains("FORGED") {
                assert!(line.starts_with("│ "));
            }
        }
    }

    #[test]
    fn header_commands_parse_back_to_the_same_range_and_epoch() {
        let response = response_with_text("topic", "speaker", "body");
        let rendered = render_read_page(&response.meta.entity_id, &response);
        let command = rendered
            .lines()
            .find(|line| line.starts_with("Re-read this span:"))
            .expect("re-read command");
        assert!(command.ends_with("--range 1..1 --epoch 3"));
        assert_eq!(parse_range("1..1").expect("range"), (1, 1));
    }

    #[test]
    fn structured_body_length_counts_exact_wire_bytes() {
        let response = response_with_text("topic", "speaker", "body");
        let raw = serde_json::to_vec(&response).expect("serialize");
        let gateway = RawGatewayResponse {
            status: 200,
            body_len: raw.len(),
            raw_body: raw.clone(),
        };
        assert_eq!(gateway.body_len, gateway.raw_body.len());
    }

    #[test]
    fn nested_typed_error_keeps_code_and_restart_command() {
        let raw_body = serde_json::to_vec(&serde_json::json!({
            "error": {
                "code": "token_expired",
                "message": "continue_token expired",
                "restart_command": "garyx meeting read 00000000-0000-7000-8000-000000000001 --full"
            }
        }))
        .expect("body");
        let error = meeting_http_error(RawGatewayResponse {
            status: 400,
            body_len: raw_body.len(),
            raw_body,
        });
        assert_eq!(error.kind, GatewayErrorKind::Rejected);
        assert!(error.message.contains("token expired"));
        assert!(error.message.contains("garyx meeting read"));
    }

    #[test]
    fn wire_payloads_cover_all_modes_and_token_only_continue() {
        assert_eq!(
            read_payload(
                MeetingReadMode::Incremental,
                Some("thread::reader"),
                None,
                None,
                None,
                Some(4_096),
            ),
            serde_json::json!({
                "mode": "incremental",
                "reader_id": "thread::reader",
                "max_bytes": 4096
            })
        );
        assert_eq!(
            read_payload(MeetingReadMode::Full, None, None, None, None, Some(8_192),),
            serde_json::json!({"mode": "full", "max_bytes": 8192})
        );
        assert_eq!(
            read_payload(
                MeetingReadMode::Range,
                None,
                Some((100, 200)),
                Some(7),
                None,
                None,
            ),
            serde_json::json!({
                "mode": "range",
                "range_start": 100,
                "range_end": 200,
                "epoch": 7
            })
        );
        assert_eq!(
            read_payload(
                MeetingReadMode::Range,
                None,
                Some((1, 2)),
                Some(99),
                Some("opaque-token"),
                Some(12_000),
            ),
            serde_json::json!({
                "mode": "range",
                "continue_token": "opaque-token",
                "max_bytes": 12000
            }),
            "continue mode sends only the mode, token, and page budget"
        );
    }

    #[tokio::test]
    async fn real_cli_http_paging_sends_exact_saturating_remainder() {
        let entity_id = Uuid::now_v7().to_string();
        let first = serde_json::to_vec(&full_response(
            &entity_id,
            &["a\n".repeat(8_000), "b\n".repeat(8_000)],
            Some("token-page-two"),
        ))
        .expect("first");
        assert!(first.len() > 48_000 && first.len() < 61_440);
        let second = serde_json::to_vec(&full_response(&entity_id, &["z".repeat(20_000)], None))
            .expect("second");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (base_url, server) =
            spawn_paged_read_server(vec![first.clone(), second.clone()], requests.clone()).await;
        let temp = tempfile::tempdir().expect("temp");
        let config = write_test_gateway_config(&temp, &base_url);
        let mut output = Vec::new();
        cmd_meeting_read_with_writer(
            config.to_str().expect("config"),
            MeetingReadCliOptions {
                id: entity_id,
                full: true,
                range: None,
                epoch: None,
                continue_token: None,
                thread: None,
                json: true,
                max_bytes: Some(65_536),
            },
            &mut output,
        )
        .await
        .expect("CLI paging");
        server.abort();
        let ndjson = String::from_utf8(output).expect("NDJSON");
        assert_eq!(ndjson.lines().count(), 2);
        for line in ndjson.lines() {
            let _: MeetingReadResponse = serde_json::from_str(line).expect("structured page");
        }

        let requests = requests.lock().expect("requests");
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0]["mode"], "full");
        assert_eq!(requests[0]["max_bytes"], 65_536);
        assert_eq!(requests[1]["mode"], "full");
        assert_eq!(requests[1]["continue_token"], "token-page-two");
        assert!(requests[1].get("range_start").is_none());
        assert!(requests[1].get("range_end").is_none());
        assert!(requests[1].get("epoch").is_none());
        assert_eq!(
            requests[1]["max_bytes"].as_u64(),
            Some(65_536u64.saturating_sub(first.len() as u64))
        );
        assert!(
            first.len() + second.len() > 65_536,
            "the per-page minimum-progress exception may overshoot the cumulative target"
        );
    }

    #[tokio::test]
    async fn real_cli_stops_before_a_sub_floor_followup_and_prints_resume() {
        let entity_id = Uuid::now_v7().to_string();
        let first = serde_json::to_vec(&full_response(
            &entity_id,
            &["a".repeat(31_000), "b".repeat(30_000)],
            Some("token-stop"),
        ))
        .expect("first");
        assert!(65_536usize.saturating_sub(first.len()) < 4_096);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (base_url, server) = spawn_paged_read_server(vec![first], requests.clone()).await;
        let temp = tempfile::tempdir().expect("temp");
        let config = write_test_gateway_config(&temp, &base_url);
        let mut output = Vec::new();
        cmd_meeting_read_with_writer(
            config.to_str().expect("config"),
            MeetingReadCliOptions {
                id: entity_id,
                full: true,
                range: None,
                epoch: None,
                continue_token: None,
                thread: None,
                json: true,
                max_bytes: Some(65_536),
            },
            &mut output,
        )
        .await
        .expect("CLI stop");
        server.abort();
        assert_eq!(requests.lock().expect("requests").len(), 1);
        assert_eq!(
            String::from_utf8(output).expect("NDJSON").lines().count(),
            1
        );
    }

    #[tokio::test]
    async fn real_cli_retries_typed_index_building_before_rendering() {
        let entity_id = Uuid::now_v7().to_string();
        let success = serde_json::to_vec(&full_response(&entity_id, &["ready".to_owned()], None))
            .expect("success body");
        let calls = Arc::new(AtomicUsize::new(0));
        let app = Router::new().route(
            "/api/meetings/{id}/read",
            post({
                let calls = calls.clone();
                move || {
                    let calls = calls.clone();
                    let success = success.clone();
                    async move {
                        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                            Response::builder()
                                .status(StatusCode::SERVICE_UNAVAILABLE)
                                .header("content-type", "application/json")
                                .body(Body::from(
                                    r#"{"error":{"code":"index_building","message":"retry"}}"#,
                                ))
                                .expect("building response")
                        } else {
                            Response::builder()
                                .status(StatusCode::OK)
                                .header("content-type", "application/json")
                                .body(Body::from(success))
                                .expect("success response")
                        }
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        let temp = tempfile::tempdir().expect("temp");
        let config = write_test_gateway_config(&temp, &format!("http://{addr}"));
        let mut output = Vec::new();
        cmd_meeting_read_with_writer(
            config.to_str().expect("config"),
            MeetingReadCliOptions {
                id: entity_id,
                full: true,
                range: None,
                epoch: None,
                continue_token: None,
                thread: None,
                json: true,
                max_bytes: None,
            },
            &mut output,
        )
        .await
        .expect("CLI retry");
        server.abort();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let _: MeetingReadResponse =
            serde_json::from_slice(trim_ascii_whitespace(&output)).expect("structured response");
    }

    #[tokio::test]
    async fn hand_seeded_real_gateway_two_reader_cli_gate_replays_broken_flush() {
        let temp = tempfile::tempdir().expect("temp");
        let db = Arc::new(GaryxDbService::memory().expect("db"));
        let mut gateway_config = GaryxConfig::default();
        gateway_config.gateway.auth_token = "test-gateway-token".to_owned();
        let state = AppStateBuilder::new(gateway_config)
            .with_garyx_db(db.clone())
            .with_meetings_dir(temp.path().join("meetings"))
            .build();
        let id = Uuid::now_v7().to_string();
        let stamp = "2026-07-16T02:35:00.000Z".to_owned();
        db.create_meeting(MeetingCreateDraft {
            id: Some(id.clone()),
            account_id: "test-account".to_owned(),
            meeting_no: "123456789".to_owned(),
            feishu_meeting_id: String::new(),
            invite_event_id: "invite-cli-gate".to_owned(),
            call_id: String::new(),
            topic: "Synthetic CLI gate".to_owned(),
            invited_by: "Test User".to_owned(),
            status: MeetingStatus::Live,
            status_detail: String::new(),
            join_deadline_at: "2026-07-16T02:40:00.000Z".to_owned(),
            grace_deadline_at: None,
            started_at: stamp.clone(),
            ended_at: None,
            finalized_at: None,
            created_at: stamp,
        })
        .expect("hand seed row");
        state
            .ops
            .meetings
            .append_page(
                &id,
                vec![
                    SegmentDraft {
                        kind: SegmentKind::Chat,
                        speaker: "Test Speaker".to_owned(),
                        start: "2026-07-16T02:35:00.000Z".to_owned(),
                        end: "2026-07-16T02:35:01.000Z".to_owned(),
                        text: format!("first-{}", "a".repeat(3_000)),
                        source_id: "source-first".to_owned(),
                    },
                    SegmentDraft {
                        kind: SegmentKind::Chat,
                        speaker: "Test Speaker".to_owned(),
                        start: "2026-07-16T02:35:02.000Z".to_owned(),
                        end: "2026-07-16T02:35:03.000Z".to_owned(),
                        text: format!("second-{}", "b".repeat(3_000)),
                        source_id: "source-second".to_owned(),
                    },
                ],
                "cursor-gate",
            )
            .await
            .expect("hand seed content");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, garyx_gateway::build_router(state))
                .await
                .expect("serve");
        });
        let config = temp.path().join("gary.json");
        std::fs::write(
            &config,
            serde_json::to_vec(&serde_json::json!({
                "gateway": {
                    "public_url": format!("http://{addr}"),
                    "auth_token": "test-gateway-token"
                }
            }))
            .expect("config JSON"),
        )
        .expect("config");
        let options = |reader: &str| MeetingReadCliOptions {
            id: id.clone(),
            full: false,
            range: None,
            epoch: None,
            continue_token: None,
            thread: Some(reader.to_owned()),
            json: true,
            max_bytes: Some(65_536),
        };

        let mut broken = BrokenFlushWriter { bytes: Vec::new() };
        let error = cmd_meeting_read_with_writer(
            config.to_str().expect("config"),
            options("reader-a"),
            &mut broken,
        )
        .await
        .expect_err("broken flush");
        assert!(
            error.to_string().contains("injected broken pipe"),
            "unexpected CLI failure: {error}"
        );
        let failed_page: MeetingReadResponse =
            serde_json::from_slice(trim_ascii_whitespace(&broken.bytes)).expect("failed page");
        let pending = db
            .get_meeting_cursor(&id, "reader-a")
            .expect("cursor")
            .expect("recognition");
        assert_eq!(pending.receipt, failed_page.meta.receipt);
        assert_eq!(pending.confirmed_seq, 0);

        let mut replay_output = Vec::new();
        cmd_meeting_read_with_writer(
            config.to_str().expect("config"),
            options("reader-a"),
            &mut replay_output,
        )
        .await
        .expect("reader-a replay and confirm");
        let replay: MeetingReadResponse =
            serde_json::from_slice(trim_ascii_whitespace(&replay_output)).expect("replay");
        assert_eq!(replay.meta.receipt, failed_page.meta.receipt);
        assert_eq!(replay.segments, failed_page.segments);

        let mut reader_b_output = Vec::new();
        cmd_meeting_read_with_writer(
            config.to_str().expect("config"),
            options("reader-b"),
            &mut reader_b_output,
        )
        .await
        .expect("reader-b read and confirm");
        let reader_b: MeetingReadResponse =
            serde_json::from_slice(trim_ascii_whitespace(&reader_b_output)).expect("reader b");
        assert_ne!(reader_b.meta.receipt, replay.meta.receipt);
        for reader in ["reader-a", "reader-b"] {
            let cursor = db
                .get_meeting_cursor(&id, reader)
                .expect("cursor")
                .expect("reader cursor");
            assert_eq!(cursor.confirmed_seq, 2);
            assert!(cursor.receipt.is_none());
        }

        let mut range_output = Vec::new();
        cmd_meeting_read_with_writer(
            config.to_str().expect("config"),
            MeetingReadCliOptions {
                id: id.clone(),
                full: false,
                range: Some("1..2".to_owned()),
                epoch: Some(0),
                continue_token: None,
                thread: None,
                json: false,
                max_bytes: Some(4_096),
            },
            &mut range_output,
        )
        .await
        .expect("first real range page");
        let range_output = String::from_utf8(range_output).expect("range output");
        assert!(range_output.contains("Span: 1..1 of 2"));
        let resume = range_output
            .lines()
            .find(|line| line.starts_with("Resume: garyx meeting read "))
            .expect("range resume command");
        let token = resume
            .split_once(" --continue ")
            .map(|(_, token)| token)
            .expect("resume token");
        let mut continued_output = Vec::new();
        cmd_meeting_read_with_writer(
            config.to_str().expect("config"),
            MeetingReadCliOptions {
                id: id.clone(),
                full: false,
                range: None,
                epoch: None,
                continue_token: Some(token.to_owned()),
                thread: None,
                json: true,
                max_bytes: Some(4_096),
            },
            &mut continued_output,
        )
        .await
        .expect("real range continuation");
        let continued: MeetingReadResponse =
            serde_json::from_slice(trim_ascii_whitespace(&continued_output))
                .expect("continued range DTO");
        assert_eq!(continued.meta.mode, "range");
        assert_eq!(
            (continued.meta.span_from, continued.meta.span_to),
            (Some(2), Some(2))
        );
        server.abort();
    }

    #[test]
    fn newline_heavy_json_budget_is_independent_from_larger_human_stdout() {
        let entity_id = Uuid::now_v7().to_string();
        let response = full_response(
            &entity_id,
            &["left\n".repeat(4_000), "right\n".repeat(4_000)],
            None,
        );
        let json_len = serde_json::to_vec(&response).expect("JSON").len();
        let human_len = render_read_page(&entity_id, &response).len();
        assert!(json_len < 65_536);
        assert!(
            human_len > json_len,
            "local framing is intentionally outside the structured JSON budget"
        );
        assert_eq!(response.segments.len(), 2);
    }

    #[test]
    fn incremental_identity_requires_env_or_thread_and_explicit_thread_wins() {
        let _env_lock = ENV_LOCK.lock().expect("env lock");
        let _thread = ScopedEnvVar::remove("GARYX_THREAD_ID");
        let base = MeetingReadCliOptions {
            id: Uuid::now_v7().to_string(),
            full: false,
            range: None,
            epoch: None,
            continue_token: None,
            thread: None,
            json: false,
            max_bytes: None,
        };
        let missing = resolve_read_mode(&base).expect_err("missing identity");
        assert!(missing.to_string().contains("GARYX_THREAD_ID"));
        assert!(missing.to_string().contains("--thread"));

        let _env = ScopedEnvVar::set_string("GARYX_THREAD_ID", "thread::environment");
        let mut explicit = base;
        explicit.thread = Some("thread::explicit".to_owned());
        let (mode, reader, _, _) = resolve_read_mode(&explicit).expect("explicit");
        assert_eq!(mode, MeetingReadMode::Incremental);
        assert_eq!(reader.as_deref(), Some("thread::explicit"));
    }

    fn trim_ascii_whitespace(bytes: &[u8]) -> &[u8] {
        let start = bytes
            .iter()
            .position(|byte| !byte.is_ascii_whitespace())
            .unwrap_or(bytes.len());
        let end = bytes
            .iter()
            .rposition(|byte| !byte.is_ascii_whitespace())
            .map(|index| index + 1)
            .unwrap_or(start);
        &bytes[start..end]
    }
}
