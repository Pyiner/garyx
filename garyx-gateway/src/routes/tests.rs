use super::*;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::body::Body;
use futures_util::StreamExt;
use garyx_bridge::MultiProviderBridge;
use garyx_bridge::provider_trait::{AgentLoopProvider, BridgeError, StreamCallback};
use garyx_models::config::{
    ApiAccount, CronAction, CronJobConfig, CronJobKind, CronSchedule, GaryxConfig,
    PluginAccountEntry,
};
use garyx_models::provider::{
    FORK_FROM_PROVIDER_TYPE_METADATA_KEY, FORK_FROM_SDK_SESSION_ID_METADATA_KEY,
    FORK_FROM_THREAD_ID_METADATA_KEY, ProviderMessage, ProviderRunOptions, ProviderRunResult,
    ProviderType, SDK_SESSION_FORK_METADATA_KEY, StreamEvent,
};
use garyx_models::thread_logs::{ThreadLogEvent, ThreadLogSink};
use garyx_router::{MessageRouter, RunTranscriptRecordDraft};
use std::path::Path;
use std::process::Command;
use tempfile::{TempDir, tempdir};
use tower::ServiceExt;

use crate::cron::CronService;
use crate::garyx_db::{
    DreamSpanDraft, DreamTopicDraft, RecentThreadDraft, ThreadMetaDraft, ThreadMetaProjectionDraft,
    WorkspaceDraft,
};
use crate::route_graph::build_router;
use crate::server::AppStateBuilder;
use crate::thread_logs::ThreadFileLogger;

fn test_config() -> GaryxConfig {
    crate::test_support::with_gateway_auth(GaryxConfig::default())
}

fn authed_request() -> axum::http::request::Builder {
    crate::test_support::authed_request()
}

async fn append_dangling_run_start(state: &Arc<AppState>, thread_id: &str, run_id: &str) {
    state
        .threads
        .history
        .transcript_store()
        .append_run_records(
            thread_id,
            Some(run_id),
            &[RunTranscriptRecordDraft::with_timestamp(
                json!({
                    "role": "system",
                    "kind": "control",
                    "internal": true,
                    "internal_kind": "control",
                    "control": {
                        "kind": "run_start",
                        "thread_id": thread_id,
                        "run_id": run_id,
                        "at": "2026-06-18T12:00:00Z"
                    }
                }),
                "2026-06-18T12:00:00Z",
            )],
        )
        .await
        .expect("dangling run_start should append");
}

#[test]
fn committed_stream_dedupe_allows_same_seq_overwrite() {
    let mut sent_payloads = HashMap::from([(
        3,
        json!({
            "type": "committed_message",
            "thread_id": "thread::stream-dedupe",
            "seq": 3,
            "message": {"role": "assistant", "content": "old"}
        })
        .to_string(),
    )]);
    let mut last_sent_seq = 3;

    let duplicate = json!({
        "type": "committed_message",
        "thread_id": "thread::stream-dedupe",
        "seq": 3,
        "message": {"role": "assistant", "content": "old"}
    })
    .to_string();
    assert_eq!(
        should_forward_committed_payload(&mut sent_payloads, &mut last_sent_seq, 3, &duplicate),
        CommittedPayloadAction::Skip
    );
    assert_eq!(last_sent_seq, 3);

    let gap = json!({
        "type": "committed_message",
        "thread_id": "thread::stream-dedupe",
        "seq": 5,
        "message": {"role": "assistant", "content": "gap"}
    })
    .to_string();
    assert_eq!(
        should_forward_committed_payload(&mut sent_payloads, &mut last_sent_seq, 5, &gap),
        CommittedPayloadAction::Reconnect
    );
    assert_eq!(last_sent_seq, 3);

    let stale = json!({
        "type": "committed_message",
        "thread_id": "thread::stream-dedupe",
        "seq": 2,
        "message": {"role": "assistant", "content": "stale"}
    })
    .to_string();
    assert_eq!(
        should_forward_committed_payload(&mut sent_payloads, &mut last_sent_seq, 2, &stale),
        CommittedPayloadAction::Skip
    );
    assert_eq!(last_sent_seq, 3);

    let overwrite = json!({
        "type": "committed_message",
        "thread_id": "thread::stream-dedupe",
        "seq": 3,
        "message": {
            "role": "system",
            "kind": "control",
            "internal": true,
            "internal_kind": "control",
            "control": {"kind": "range_rewrite", "tombstone": true}
        }
    })
    .to_string();
    assert_eq!(
        should_forward_committed_payload(&mut sent_payloads, &mut last_sent_seq, 3, &overwrite),
        CommittedPayloadAction::Forward
    );
    assert_eq!(last_sent_seq, 3);

    let suffix = json!({
        "type": "committed_message",
        "thread_id": "thread::stream-dedupe",
        "seq": 4,
        "message": {
            "role": "system",
            "kind": "control",
            "internal": true,
            "internal_kind": "control",
            "control": {"kind": "range_rewrite", "tombstone": false}
        }
    })
    .to_string();
    assert_eq!(
        should_forward_committed_payload(&mut sent_payloads, &mut last_sent_seq, 4, &suffix),
        CommittedPayloadAction::Forward
    );
    assert_eq!(last_sent_seq, 4);
}

#[test]
fn thread_stream_replay_options_last_event_id_forces_resume() {
    let params = ThreadStreamParams {
        after_seq: 1,
        replay_scope: Some(ThreadStreamReplayScope::Initial),
        initial_user_turns: Some(1),
        render_floor: Some(7),
    };

    let (after_seq, options) = thread_stream_replay_options(&params, Some(9), true);

    assert_eq!(after_seq, 9);
    assert_eq!(options.replay_scope, ThreadStreamReplayScope::Resume);
    assert_eq!(options.initial_user_turns, None);
    assert_eq!(options.render_floor, 7);
}

#[test]
fn committed_stream_gap_forces_reconnect() {
    let mut sent_payloads = HashMap::new();
    let mut last_sent_seq = 1;
    let gap = json!({
        "type": "committed_message",
        "thread_id": "thread::stream-gap",
        "seq": 3,
        "message": {"role": "assistant", "content": "gap"}
    })
    .to_string();

    let error = committed_thread_stream_live_payload(
        &gap,
        "thread::stream-gap",
        &mut sent_payloads,
        &mut last_sent_seq,
    )
    .expect_err("non-contiguous live seq should terminate stream");
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    assert_eq!(last_sent_seq, 1);
}

#[test]
fn thread_stream_live_payload_only_forwards_committed_messages() {
    let mut sent_payloads = HashMap::new();
    let mut last_sent_seq = 0;

    let noise = json!({
        "type": "ignored_noise",
        "thread_id": "thread::stream-filter",
        "run_id": "run::stream-filter",
        "reason": "not a committed transcript payload"
    })
    .to_string();
    assert_eq!(
        committed_thread_stream_live_payload(
            &noise,
            "thread::stream-filter",
            &mut sent_payloads,
            &mut last_sent_seq,
        )
        .expect("noise should not force reconnect"),
        None
    );
    assert_eq!(last_sent_seq, 0);

    let other_thread = json!({
        "type": "committed_message",
        "thread_id": "thread::other",
        "seq": 1,
        "message": {"role": "assistant", "content": "other"}
    })
    .to_string();
    assert_eq!(
        committed_thread_stream_live_payload(
            &other_thread,
            "thread::stream-filter",
            &mut sent_payloads,
            &mut last_sent_seq,
        )
        .expect("other thread should not force reconnect"),
        None
    );
    assert_eq!(last_sent_seq, 0);

    let committed = json!({
        "type": "committed_message",
        "thread_id": "thread::stream-filter",
        "seq": 1,
        "message": {"role": "assistant", "content": "ok"}
    })
    .to_string();
    let committed_value: Value = serde_json::from_str(&committed).unwrap();
    assert_eq!(
        committed_thread_stream_live_payload(
            &committed,
            "thread::stream-filter",
            &mut sent_payloads,
            &mut last_sent_seq,
        )
        .expect("committed payload should forward"),
        Some((1, committed_value))
    );
    assert_eq!(last_sent_seq, 1);
}

#[tokio::test]
async fn thread_stream_replay_pages_when_tail_cap_overflows() {
    let state = AppStateBuilder::new(test_config()).build();
    let (thread_id, _) = create_thread_record(
        &state.threads.thread_store,
        ThreadEnsureOptions {
            label: Some("Replay cap".to_owned()),
            workspace_dir: None,
            workspace_mode: Default::default(),
            worktree_base_dir: None,
            agent_id: None,
            metadata: HashMap::new(),
            provider_type: None,
            sdk_session_id: None,
            thread_kind: None,
            origin_channel: None,
            origin_account_id: None,
            origin_from_id: None,
            is_group: None,
        },
    )
    .await
    .unwrap();
    let messages: Vec<Value> = (1..=THREAD_TRANSCRIPT_REPLAY_CAP + 2)
        .map(|seq| json!({"role": "assistant", "content": format!("m{seq}")}))
        .collect();
    state
        .threads
        .history
        .transcript_store()
        .append_committed_messages(&thread_id, Some("run::replay-cap"), &messages)
        .await
        .unwrap();

    let replay =
        build_thread_stream_replay(&state, &thread_id, 0, ThreadStreamReplayOptions::resume(0))
            .await;
    assert_eq!(replay.events.len(), 1);
    assert_eq!(replay.sent_payloads.len(), THREAD_TRANSCRIPT_REPLAY_CAP + 2);
    assert_eq!(replay.max_seq, (THREAD_TRANSCRIPT_REPLAY_CAP + 2) as u64);
    let event = replay.events[0].as_ref().unwrap();
    assert_eq!(event.id, replay.max_seq);
    let frame: Value = serde_json::from_str(&event.payload).unwrap();
    assert_eq!(
        frame.get("type").and_then(Value::as_str),
        Some("thread_render_frame")
    );
    assert_eq!(
        frame
            .get("render_state")
            .and_then(|state| state.get("based_on_seq"))
            .and_then(Value::as_u64),
        Some(replay.max_seq)
    );
    let events = frame.get("events").and_then(Value::as_array).unwrap();
    assert_eq!(events.len(), THREAD_TRANSCRIPT_REPLAY_CAP + 2);
    assert_eq!(events[0].get("seq").and_then(Value::as_u64), Some(1));
    assert_eq!(
        events
            .last()
            .and_then(|event| event.get("seq"))
            .and_then(Value::as_u64),
        Some(replay.max_seq)
    );
    assert!(
        replay.sent_payloads.contains_key(&1),
        "overflow replay must include the oldest missing page, not only the newest tail"
    );
    assert!(
        replay
            .sent_payloads
            .contains_key(&u64::try_from(THREAD_TRANSCRIPT_REPLAY_CAP + 2).unwrap())
    );
}

#[tokio::test]
async fn thread_stream_replay_after_seq_emits_one_aligned_render_frame() {
    let state = AppStateBuilder::new(test_config()).build();
    let thread_id = "thread::render-replay";
    state
        .threads
        .history
        .transcript_store()
        .append_run_records(
            thread_id,
            Some("run::render-replay"),
            &[
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "one"}),
                    "2026-06-18T12:00:00Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "two"}),
                    "2026-06-18T12:00:01Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "three"}),
                    "2026-06-18T12:00:02Z",
                ),
            ],
        )
        .await
        .unwrap();

    let replay =
        build_thread_stream_replay(&state, thread_id, 1, ThreadStreamReplayOptions::resume(0))
            .await;

    assert_eq!(replay.events.len(), 1);
    assert_eq!(replay.max_seq, 3);
    assert_eq!(replay.sent_payloads.len(), 2);
    assert!(replay.sent_payloads.contains_key(&2));
    assert!(replay.sent_payloads.contains_key(&3));
    let event = replay.events[0].as_ref().unwrap();
    assert_eq!(event.id, 3);
    let frame: Value = serde_json::from_str(&event.payload).unwrap();
    let events = frame.get("events").and_then(Value::as_array).unwrap();
    assert_eq!(
        events
            .iter()
            .map(|event| event.get("seq").and_then(Value::as_u64).unwrap())
            .collect::<Vec<_>>(),
        vec![2, 3]
    );
    assert_eq!(
        frame
            .get("render_state")
            .and_then(|state| state.get("based_on_seq"))
            .and_then(Value::as_u64),
        Some(3)
    );
    assert_eq!(
        frame
            .get("render_state")
            .and_then(|state| state.get("visibleMessageIds"))
            .and_then(Value::as_array)
            .map(|items| { items.iter().filter_map(Value::as_str).collect::<Vec<_>>() })
            .unwrap(),
        vec!["seq:1", "seq:2", "seq:3"]
    );
    assert!(
        frame
            .get("render_state")
            .and_then(|state| state.get("window"))
            .is_none(),
        "default replay must not emit window metadata"
    );
}

#[tokio::test]
async fn thread_stream_replay_render_floor_windows_event_frame() {
    let state = AppStateBuilder::new(test_config()).build();
    let thread_id = "thread::render-replay-floor-events";
    state
        .threads
        .history
        .transcript_store()
        .append_run_records(
            thread_id,
            Some("run::render-replay-floor-events"),
            &[
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "older question"}),
                    "2026-06-18T12:00:00Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "older answer"}),
                    "2026-06-18T12:00:01Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "new question"}),
                    "2026-06-18T12:00:02Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "new answer"}),
                    "2026-06-18T12:00:03Z",
                ),
            ],
        )
        .await
        .unwrap();

    let replay =
        build_thread_stream_replay(&state, thread_id, 2, ThreadStreamReplayOptions::resume(3))
            .await;

    assert_eq!(replay.events.len(), 1);
    assert_eq!(replay.max_seq, 4);
    let event = replay.events[0].as_ref().unwrap();
    assert_eq!(event.id, 4);
    let frame: Value = serde_json::from_str(&event.payload).unwrap();
    let events = frame.get("events").and_then(Value::as_array).unwrap();
    assert_eq!(
        events
            .iter()
            .map(|event| event.get("seq").and_then(Value::as_u64).unwrap())
            .collect::<Vec<_>>(),
        vec![3, 4]
    );
    let render_state = frame.get("render_state").unwrap();
    assert_eq!(
        render_state
            .get("visibleMessageIds")
            .and_then(Value::as_array)
            .map(|items| { items.iter().filter_map(Value::as_str).collect::<Vec<_>>() })
            .unwrap(),
        vec!["seq:3", "seq:4"]
    );
    assert_eq!(
        render_state
            .get("window")
            .and_then(|window| window.get("floor_seq"))
            .and_then(Value::as_u64),
        Some(3)
    );
    assert_eq!(
        render_state
            .get("window")
            .and_then(|window| window.get("has_more_above"))
            .and_then(Value::as_bool),
        Some(true)
    );
}

#[tokio::test]
async fn thread_stream_replay_initial_user_turn_window_trims_and_carries_bodies() {
    let state = AppStateBuilder::new(test_config()).build();
    let thread_id = "thread::render-replay-initial-window";
    state
        .threads
        .history
        .transcript_store()
        .append_run_records(
            thread_id,
            Some("run::render-replay-initial-window"),
            &[
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "older question"}),
                    "2026-06-18T12:00:00Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "older answer"}),
                    "2026-06-18T12:00:01Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "new question"}),
                    "2026-06-18T12:00:02Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "new answer"}),
                    "2026-06-18T12:00:03Z",
                ),
            ],
        )
        .await
        .unwrap();

    let replay = build_thread_stream_replay(
        &state,
        thread_id,
        4,
        ThreadStreamReplayOptions {
            replay_scope: ThreadStreamReplayScope::Initial,
            initial_user_turns: Some(1),
            render_floor: 0,
        },
    )
    .await;

    assert_eq!(replay.events.len(), 1);
    assert_eq!(replay.max_seq, 4);
    assert_eq!(
        replay
            .sent_payloads
            .keys()
            .copied()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([3, 4])
    );
    let event = replay.events[0].as_ref().unwrap();
    assert_eq!(event.id, 4);
    let frame: Value = serde_json::from_str(&event.payload).unwrap();
    let events = frame.get("events").and_then(Value::as_array).unwrap();
    assert_eq!(
        events
            .iter()
            .map(|event| event.get("seq").and_then(Value::as_u64).unwrap())
            .collect::<Vec<_>>(),
        vec![3, 4],
        "initial replay must carry window bodies even when after_seq is already caught up"
    );
    let render_state = frame.get("render_state").unwrap();
    assert_eq!(
        render_state
            .get("visibleMessageIds")
            .and_then(Value::as_array)
            .map(|items| { items.iter().filter_map(Value::as_str).collect::<Vec<_>>() })
            .unwrap(),
        vec!["seq:3", "seq:4"]
    );
    assert_eq!(
        render_state
            .get("window")
            .and_then(|window| window.get("floor_seq"))
            .and_then(Value::as_u64),
        Some(3)
    );
    assert_eq!(
        replay.render_floor, 3,
        "same SSE connection live frames must keep the initial render window"
    );

    let live_append = state
        .threads
        .history
        .transcript_store()
        .append_run_records(
            thread_id,
            Some("run::render-replay-initial-window"),
            &[RunTranscriptRecordDraft::with_timestamp(
                json!({"role": "assistant", "content": "live continuation"}),
                "2026-06-18T12:00:04Z",
            )],
        )
        .await
        .unwrap();
    let live_record = live_append.appended_records.last().unwrap();
    let live_payload = committed_thread_stream_replay_payload_value(thread_id, live_record);
    let live_event = committed_thread_stream_live_event(
        &state,
        thread_id,
        live_record.seq,
        live_payload,
        replay.render_floor,
    )
    .await
    .unwrap();
    let live_frame: Value = serde_json::from_str(&live_event.payload).unwrap();
    let live_render_state = live_frame.get("render_state").unwrap();
    assert_eq!(
        live_render_state
            .get("visibleMessageIds")
            .and_then(Value::as_array)
            .map(|items| { items.iter().filter_map(Value::as_str).collect::<Vec<_>>() })
            .unwrap(),
        vec!["seq:3", "seq:4", "seq:5"],
        "live frame after initial replay must not widen back to the full transcript"
    );
    assert_eq!(
        live_render_state
            .get("window")
            .and_then(|window| window.get("floor_seq"))
            .and_then(Value::as_u64),
        Some(3)
    );
}

#[tokio::test]
async fn thread_stream_handler_keeps_initial_floor_for_live_frames() {
    let state = AppStateBuilder::new(test_config()).build();
    let (thread_id, _) = create_thread_record(
        &state.threads.thread_store,
        ThreadEnsureOptions {
            label: Some("Initial floor live".to_owned()),
            workspace_dir: None,
            workspace_mode: Default::default(),
            worktree_base_dir: None,
            agent_id: None,
            metadata: HashMap::new(),
            provider_type: None,
            sdk_session_id: None,
            thread_kind: None,
            origin_channel: None,
            origin_account_id: None,
            origin_from_id: None,
            is_group: None,
        },
    )
    .await
    .unwrap();
    state
        .threads
        .history
        .transcript_store()
        .append_run_records(
            &thread_id,
            Some("run::initial-floor-live"),
            &[
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "older question"}),
                    "2026-06-18T12:00:00Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "older answer"}),
                    "2026-06-18T12:00:01Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "new question"}),
                    "2026-06-18T12:00:02Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "new answer"}),
                    "2026-06-18T12:00:03Z",
                ),
            ],
        )
        .await
        .unwrap();

    let router = build_router(state.clone());
    let request = authed_request()
        .uri(format!(
            "/api/threads/{thread_id}/stream?after_seq=4&replay_scope=initial&initial_user_turns=1"
        ))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body();

    let live_append = state
        .threads
        .history
        .transcript_store()
        .append_run_records(
            &thread_id,
            Some("run::initial-floor-live"),
            &[RunTranscriptRecordDraft::with_timestamp(
                json!({"role": "assistant", "content": "live continuation"}),
                "2026-06-18T12:00:04Z",
            )],
        )
        .await
        .unwrap();
    let live_record = live_append.appended_records.last().unwrap();
    let live_payload = committed_thread_stream_replay_payload_value(&thread_id, live_record);
    state
        .ops
        .events
        .sender()
        .send(live_payload.to_string())
        .unwrap();

    let frames = read_sse_data_frames(body, 2).await;
    assert_eq!(
        frames[0]
            .get("render_state")
            .and_then(|state| state.get("window"))
            .and_then(|window| window.get("floor_seq"))
            .and_then(Value::as_u64),
        Some(3)
    );
    assert_eq!(
        frames[1]
            .get("render_state")
            .and_then(|state| state.get("window"))
            .and_then(|window| window.get("floor_seq"))
            .and_then(Value::as_u64),
        Some(3),
        "thread_stream handler must wire initial replay's effective floor into live frames"
    );
    assert_eq!(
        frames[1]
            .get("render_state")
            .and_then(|state| state.get("visibleMessageIds"))
            .and_then(Value::as_array)
            .map(|items| { items.iter().filter_map(Value::as_str).collect::<Vec<_>>() })
            .unwrap(),
        vec!["seq:3", "seq:4", "seq:5"]
    );
}

async fn read_sse_data_frames(body: Body, count: usize) -> Vec<Value> {
    let mut stream = body.into_data_stream();
    let mut buffer = String::new();
    let mut frames = Vec::new();
    while frames.len() < count {
        let chunk = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .expect("timed out reading SSE chunk")
            .expect("SSE stream ended before expected frame")
            .expect("SSE chunk should be ok");
        buffer.push_str(std::str::from_utf8(&chunk).expect("SSE should be utf8"));
        while let Some(frame_end) = buffer.find("\n\n") {
            let frame = buffer[..frame_end].to_owned();
            buffer = buffer[frame_end + 2..].to_owned();
            for line in frame.lines() {
                if let Some(data) = line.strip_prefix("data:") {
                    frames.push(serde_json::from_str(data.trim_start()).expect("SSE data json"));
                    if frames.len() == count {
                        return frames;
                    }
                }
            }
        }
    }
    frames
}

#[tokio::test]
async fn thread_stream_replay_caught_up_emits_snapshot_only_frame() {
    let state = AppStateBuilder::new(test_config()).build();
    let thread_id = "thread::render-replay-caught-up";
    state
        .threads
        .history
        .transcript_store()
        .append_run_records(
            thread_id,
            Some("run::render-replay-caught-up"),
            &[
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "one"}),
                    "2026-06-18T12:00:00Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "two"}),
                    "2026-06-18T12:00:01Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "three"}),
                    "2026-06-18T12:00:02Z",
                ),
            ],
        )
        .await
        .unwrap();

    let replay =
        build_thread_stream_replay(&state, thread_id, 3, ThreadStreamReplayOptions::resume(0))
            .await;

    assert_eq!(replay.events.len(), 1);
    assert_eq!(replay.max_seq, 3);
    assert!(replay.sent_payloads.is_empty());
    let event = replay.events[0].as_ref().unwrap();
    assert_eq!(event.id, 3);
    let frame: Value = serde_json::from_str(&event.payload).unwrap();
    let events = frame.get("events").and_then(Value::as_array).unwrap();
    assert!(events.is_empty());
    assert_eq!(
        frame
            .get("render_state")
            .and_then(|state| state.get("based_on_seq"))
            .and_then(Value::as_u64),
        Some(3)
    );
    assert_eq!(
        frame
            .get("render_state")
            .and_then(|state| state.get("visibleMessageIds"))
            .and_then(Value::as_array)
            .map(|items| { items.iter().filter_map(Value::as_str).collect::<Vec<_>>() })
            .unwrap(),
        vec!["seq:1", "seq:2", "seq:3"]
    );
    assert!(
        frame
            .get("render_state")
            .and_then(|state| state.get("window"))
            .is_none(),
        "default caught-up snapshot must remain full-history"
    );
}

#[tokio::test]
async fn thread_stream_replay_render_floor_windows_snapshot_only_frame() {
    let state = AppStateBuilder::new(test_config()).build();
    let thread_id = "thread::render-replay-floor-snapshot";
    state
        .threads
        .history
        .transcript_store()
        .append_run_records(
            thread_id,
            Some("run::render-replay-floor-snapshot"),
            &[
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "older question"}),
                    "2026-06-18T12:00:00Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "older answer"}),
                    "2026-06-18T12:00:01Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "new question"}),
                    "2026-06-18T12:00:02Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "new answer"}),
                    "2026-06-18T12:00:03Z",
                ),
            ],
        )
        .await
        .unwrap();

    let replay =
        build_thread_stream_replay(&state, thread_id, 4, ThreadStreamReplayOptions::resume(3))
            .await;

    assert_eq!(replay.events.len(), 1);
    assert_eq!(replay.max_seq, 4);
    assert!(replay.sent_payloads.is_empty());
    let event = replay.events[0].as_ref().unwrap();
    assert_eq!(event.id, 4);
    let frame: Value = serde_json::from_str(&event.payload).unwrap();
    assert!(
        frame
            .get("events")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
    );
    let render_state = frame.get("render_state").unwrap();
    assert_eq!(
        render_state
            .get("visibleMessageIds")
            .and_then(Value::as_array)
            .map(|items| { items.iter().filter_map(Value::as_str).collect::<Vec<_>>() })
            .unwrap(),
        vec!["seq:3", "seq:4"]
    );
    assert_eq!(
        render_state
            .get("window")
            .and_then(|window| window.get("floor_seq"))
            .and_then(Value::as_u64),
        Some(3)
    );
}

#[tokio::test]
async fn thread_stream_replay_caught_up_clamps_overlarge_cursor_to_snapshot_seq() {
    let state = AppStateBuilder::new(test_config()).build();
    let thread_id = "thread::render-replay-overlarge-cursor";
    state
        .threads
        .history
        .transcript_store()
        .append_run_records(
            thread_id,
            Some("run::render-replay-overlarge-cursor"),
            &[
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "one"}),
                    "2026-06-18T12:00:00Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "two"}),
                    "2026-06-18T12:00:01Z",
                ),
            ],
        )
        .await
        .unwrap();

    let replay =
        build_thread_stream_replay(&state, thread_id, 99, ThreadStreamReplayOptions::resume(0))
            .await;

    assert_eq!(replay.events.len(), 1);
    assert_eq!(replay.max_seq, 2);
    assert!(replay.sent_payloads.is_empty());
    let event = replay.events[0].as_ref().unwrap();
    assert_eq!(event.id, 2);
    let frame: Value = serde_json::from_str(&event.payload).unwrap();
    assert_eq!(
        frame
            .get("render_state")
            .and_then(|state| state.get("based_on_seq"))
            .and_then(Value::as_u64),
        Some(2)
    );
    assert!(
        frame
            .get("events")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
    );
}

#[tokio::test]
async fn thread_stream_live_event_carries_committed_payload_and_render_snapshot() {
    let state = AppStateBuilder::new(test_config()).build();
    let thread_id = "thread::render-live-frame";
    let append = state
        .threads
        .history
        .transcript_store()
        .append_run_records(
            thread_id,
            Some("run::render-live-frame"),
            &[
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "question"}),
                    "2026-06-18T12:00:00Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "answer"}),
                    "2026-06-18T12:00:01Z",
                ),
            ],
        )
        .await
        .unwrap();
    let live_record = append.appended_records.last().unwrap();
    let payload = committed_thread_stream_replay_payload_value(thread_id, live_record);

    let event = committed_thread_stream_live_event(&state, thread_id, live_record.seq, payload, 0)
        .await
        .unwrap();

    assert_eq!(event.id, live_record.seq);
    let frame: Value = serde_json::from_str(&event.payload).unwrap();
    let events = frame.get("events").and_then(Value::as_array).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].get("type").and_then(Value::as_str),
        Some("committed_message")
    );
    assert_eq!(
        events[0].get("seq").and_then(Value::as_u64),
        Some(live_record.seq)
    );
    assert_eq!(
        frame
            .get("render_state")
            .and_then(|state| state.get("based_on_seq"))
            .and_then(Value::as_u64),
        Some(live_record.seq)
    );
    assert_eq!(
        frame
            .get("render_state")
            .and_then(|state| state.get("visibleMessageIds"))
            .and_then(Value::as_array)
            .map(|items| { items.iter().filter_map(Value::as_str).collect::<Vec<_>>() })
            .unwrap(),
        vec!["seq:1", "seq:2"]
    );
}

#[tokio::test]
async fn thread_stream_live_event_respects_render_floor() {
    let state = AppStateBuilder::new(test_config()).build();
    let thread_id = "thread::render-live-frame-floor";
    let append = state
        .threads
        .history
        .transcript_store()
        .append_run_records(
            thread_id,
            Some("run::render-live-frame-floor"),
            &[
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "older question"}),
                    "2026-06-18T12:00:00Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "older answer"}),
                    "2026-06-18T12:00:01Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "user", "content": "new question"}),
                    "2026-06-18T12:00:02Z",
                ),
                RunTranscriptRecordDraft::with_timestamp(
                    json!({"role": "assistant", "content": "new answer"}),
                    "2026-06-18T12:00:03Z",
                ),
            ],
        )
        .await
        .unwrap();
    let live_record = append.appended_records.last().unwrap();
    let payload = committed_thread_stream_replay_payload_value(thread_id, live_record);

    let event = committed_thread_stream_live_event(&state, thread_id, live_record.seq, payload, 3)
        .await
        .unwrap();

    assert_eq!(event.id, live_record.seq);
    let frame: Value = serde_json::from_str(&event.payload).unwrap();
    let render_state = frame.get("render_state").unwrap();
    assert_eq!(
        render_state
            .get("visibleMessageIds")
            .and_then(Value::as_array)
            .map(|items| { items.iter().filter_map(Value::as_str).collect::<Vec<_>>() })
            .unwrap(),
        vec!["seq:3", "seq:4"]
    );
    assert_eq!(
        render_state
            .get("window")
            .and_then(|window| window.get("floor_seq"))
            .and_then(Value::as_u64),
        Some(3)
    );
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git -C {} {} failed: {}",
        repo.display(),
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git -C {} {} failed: {}",
        repo.display(),
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn init_test_git_repo(repo: &Path) {
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.name", "Test User"]);
    run_git(repo, &["config", "user.email", "test@example.com"]);
    std::fs::write(repo.join("README.md"), "test repo\n").expect("write readme");
    run_git(repo, &["add", "README.md"]);
    run_git(repo, &["commit", "-m", "initial"]);
}

struct SlowDeleteProvider {
    ready: AtomicBool,
    delay_ms: u64,
    clear_succeeds: bool,
    cleared_sessions: std::sync::Mutex<Vec<String>>,
}

impl SlowDeleteProvider {
    fn new(delay_ms: u64) -> Self {
        Self::with_clear_result(delay_ms, true)
    }

    fn with_clear_result(delay_ms: u64, clear_succeeds: bool) -> Self {
        Self {
            ready: AtomicBool::new(true),
            delay_ms,
            clear_succeeds,
            cleared_sessions: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn cleared_sessions(&self) -> Vec<String> {
        self.cleared_sessions.lock().unwrap().clone()
    }
}

#[derive(Debug, Clone)]
struct RecordedProviderRun {
    thread_id: String,
    message: String,
    metadata: HashMap<String, Value>,
    workspace_dir: Option<String>,
}

struct RecordingTaskProvider {
    ready: AtomicBool,
    provider_type: ProviderType,
    runs: std::sync::Mutex<Vec<RecordedProviderRun>>,
}

impl RecordingTaskProvider {
    fn new() -> Self {
        Self::with_provider_type(ProviderType::CodexAppServer)
    }

    fn with_provider_type(provider_type: ProviderType) -> Self {
        Self {
            ready: AtomicBool::new(true),
            provider_type,
            runs: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn runs(&self) -> Vec<RecordedProviderRun> {
        self.runs.lock().unwrap().clone()
    }
}

#[test]
fn endpoint_conversation_details_marks_feishu_group_from_scope() {
    let endpoint = garyx_router::KnownChannelEndpoint {
        endpoint_key: "feishu::main::oc_group::oc_group".to_owned(),
        channel: "feishu".to_owned(),
        account_id: "main".to_owned(),
        binding_key: "oc_group".to_owned(),
        chat_id: "oc_group".to_owned(),
        delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_owned(),
        delivery_target_id: "oc_group".to_owned(),
        display_label: "garyx".to_owned(),
        thread_id: Some("thread::group".to_owned()),
        thread_label: Some("garyx".to_owned()),
        workspace_dir: None,
        thread_updated_at: None,
        last_inbound_at: None,
        last_delivery_at: None,
    };

    let details = endpoint_conversation_details(&endpoint);

    assert_eq!(details.kind, "group");
    assert_eq!(details.label, "garyx");
}

#[test]
fn endpoint_conversation_details_marks_feishu_topic_from_scope() {
    let endpoint = garyx_router::KnownChannelEndpoint {
        endpoint_key: "feishu::main::ou_user::om_topic".to_owned(),
        channel: "feishu".to_owned(),
        account_id: "main".to_owned(),
        binding_key: "om_topic".to_owned(),
        chat_id: "oc_group".to_owned(),
        delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_owned(),
        delivery_target_id: "oc_group".to_owned(),
        display_label: "garyx".to_owned(),
        thread_id: Some("thread::topic".to_owned()),
        thread_label: Some("garyx".to_owned()),
        workspace_dir: None,
        thread_updated_at: None,
        last_inbound_at: None,
        last_delivery_at: None,
    };

    let details = endpoint_conversation_details(&endpoint);

    assert_eq!(details.kind, "topic");
    assert_eq!(details.label, "garyx");
}

#[test]
fn endpoint_conversation_details_keeps_feishu_private_as_private() {
    let endpoint = garyx_router::KnownChannelEndpoint {
        endpoint_key: "feishu::main::ou_user".to_owned(),
        channel: "feishu".to_owned(),
        account_id: "main".to_owned(),
        binding_key: "ou_user".to_owned(),
        chat_id: "oc_private".to_owned(),
        delivery_target_type: DELIVERY_TARGET_TYPE_OPEN_ID.to_owned(),
        delivery_target_id: "ou_user".to_owned(),
        display_label: "garyx".to_owned(),
        thread_id: Some("thread::private".to_owned()),
        thread_label: Some("garyx".to_owned()),
        workspace_dir: None,
        thread_updated_at: None,
        last_inbound_at: None,
        last_delivery_at: None,
    };

    let details = endpoint_conversation_details(&endpoint);

    assert_eq!(details.kind, "private");
    assert_eq!(details.label, "garyx");
}

#[test]
fn endpoint_conversation_details_marks_discord_dm_as_private() {
    let endpoint = garyx_router::KnownChannelEndpoint {
        endpoint_key: "discord::main::1000000001::2000000001".to_owned(),
        channel: "discord".to_owned(),
        account_id: "main".to_owned(),
        binding_key: "1000000001".to_owned(),
        chat_id: "2000000001".to_owned(),
        delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_owned(),
        delivery_target_id: "2000000001".to_owned(),
        display_label: "Test User".to_owned(),
        thread_id: Some("thread::discord-dm".to_owned()),
        thread_label: Some("Test User".to_owned()),
        workspace_dir: None,
        thread_updated_at: None,
        last_inbound_at: None,
        last_delivery_at: None,
    };

    let details = endpoint_conversation_details(&endpoint);

    assert_eq!(details.kind, "private");
    assert_eq!(details.label, "Test User");
}

#[test]
fn endpoint_conversation_details_marks_discord_channel_as_group() {
    let endpoint = garyx_router::KnownChannelEndpoint {
        endpoint_key: "discord::main::3000000001".to_owned(),
        channel: "discord".to_owned(),
        account_id: "main".to_owned(),
        binding_key: "3000000001".to_owned(),
        chat_id: "3000000001".to_owned(),
        delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_owned(),
        delivery_target_id: "3000000001".to_owned(),
        display_label: "general".to_owned(),
        thread_id: Some("thread::discord-channel".to_owned()),
        thread_label: Some("general".to_owned()),
        workspace_dir: None,
        thread_updated_at: None,
        last_inbound_at: None,
        last_delivery_at: None,
    };

    let details = endpoint_conversation_details(&endpoint);

    assert_eq!(details.kind, "group");
    assert_eq!(details.label, "general");
}

#[async_trait::async_trait]
impl AgentLoopProvider for SlowDeleteProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::ClaudeCode
    }

    fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        self.ready.store(true, Ordering::Relaxed);
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        self.ready.store(false, Ordering::Relaxed);
        Ok(())
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        on_chunk(StreamEvent::Delta {
            text: "slow-delete".to_owned(),
        });
        on_chunk(StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id: "slow-delete-run".to_owned(),
            thread_id: options.thread_id.clone(),
            response: "slow-delete".to_owned(),
            session_messages: vec![],
            sdk_session_id: None,
            actual_model: None,
            thread_title: None,
            success: true,
            error: None,
            input_tokens: 1,
            output_tokens: 1,
            cost: 0.0,
            duration_ms: self.delay_ms as i64,
        })
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{session_key}"))
    }

    async fn clear_session(&self, session_key: &str) -> bool {
        self.cleared_sessions
            .lock()
            .unwrap()
            .push(session_key.to_owned());
        self.clear_succeeds
    }
}

#[async_trait::async_trait]
impl AgentLoopProvider for RecordingTaskProvider {
    fn provider_type(&self) -> ProviderType {
        self.provider_type.clone()
    }

    fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    async fn initialize(&mut self) -> Result<(), BridgeError> {
        self.ready.store(true, Ordering::Relaxed);
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<(), BridgeError> {
        self.ready.store(false, Ordering::Relaxed);
        Ok(())
    }

    async fn run_streaming(
        &self,
        options: &ProviderRunOptions,
        on_chunk: StreamCallback,
    ) -> Result<ProviderRunResult, BridgeError> {
        self.runs.lock().unwrap().push(RecordedProviderRun {
            thread_id: options.thread_id.clone(),
            message: options.message.clone(),
            metadata: options.metadata.clone(),
            workspace_dir: options.workspace_dir.clone(),
        });
        on_chunk(StreamEvent::Delta {
            text: "task recorded".to_owned(),
        });
        on_chunk(StreamEvent::Done);
        Ok(ProviderRunResult {
            run_id: "recording-task-run".to_owned(),
            thread_id: options.thread_id.clone(),
            response: "task recorded".to_owned(),
            session_messages: vec![],
            sdk_session_id: None,
            actual_model: None,
            thread_title: None,
            success: true,
            error: None,
            input_tokens: 1,
            output_tokens: 1,
            cost: 0.0,
            duration_ms: 1,
        })
    }

    async fn get_or_create_session(&self, session_key: &str) -> Result<String, BridgeError> {
        Ok(format!("sdk-{session_key}"))
    }
}

async fn test_state() -> (Arc<AppState>, Arc<ThreadFileLogger>, TempDir) {
    let dir = tempdir().unwrap();
    let logger = Arc::new(ThreadFileLogger::new(dir.path()));
    let state = AppStateBuilder::new(test_config())
        .with_custom_agent_store(Arc::new(crate::custom_agents::CustomAgentStore::new()))
        .with_agent_team_store(Arc::new(crate::agent_teams::AgentTeamStore::new()))
        .with_thread_log_sink(logger.clone())
        .build();
    (state, logger, dir)
}

#[tokio::test]
async fn thread_summary_does_not_fetch_transcript_when_snapshot_cache_is_empty() {
    let (state, _logger, _dir) = test_state().await;
    let thread_id = "thread::summary-transcript";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "messages": [],
                "message_count": 2,
                "history": {
                    "source": "transcript_v1",
                    "message_count": 2
                }
            }),
        )
        .await;
    state
        .threads
        .history
        .transcript_store()
        .rewrite_from_messages(
            thread_id,
            &[
                json!({"role": "user", "content": "hello from transcript"}),
                json!({"role": "assistant", "content": "reply from transcript"}),
            ],
        )
        .await
        .unwrap();

    let data = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("thread data");
    let summary = thread_summary(thread_id, &data);
    assert!(summary["last_user_message"].is_null());
    assert!(summary["last_assistant_message"].is_null());
}

#[tokio::test]
async fn thread_summary_omits_active_run_id_from_runtime_summary() {
    let (_state, _logger, _dir) = test_state().await;
    let thread_id = "thread::inactive-run-summary";
    let summary = thread_summary(thread_id, &json!({ "history": {} }));

    assert_eq!(summary["active_run_id"], json!(null));
}

#[tokio::test]
async fn thread_logs_route_returns_full_and_delta_chunks() {
    let (state, logger, _dir) = test_state().await;
    let (thread_id, _) = create_thread_record(
        &state.threads.thread_store,
        ThreadEnsureOptions {
            label: Some("Logs".to_owned()),
            workspace_dir: None,
            workspace_mode: Default::default(),
            worktree_base_dir: None,
            agent_id: None,
            metadata: HashMap::new(),
            provider_type: None,
            sdk_session_id: None,
            thread_kind: None,
            origin_channel: None,
            origin_account_id: None,
            origin_from_id: None,
            is_group: None,
        },
    )
    .await
    .unwrap();
    logger
        .record_event(ThreadLogEvent::info(&thread_id, "run", "hello"))
        .await;

    let router = build_router(state.clone());
    let request = authed_request()
        .uri(format!("/api/threads/{thread_id}/logs"))
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let cursor = payload["cursor"].as_u64().unwrap();
    assert_eq!(payload["reset"], true);
    assert!(payload["text"].as_str().unwrap().contains("hello"));

    logger
        .record_event(ThreadLogEvent::info(&thread_id, "run", "world"))
        .await;
    let request = authed_request()
        .uri(format!("/api/threads/{thread_id}/logs?cursor={cursor}"))
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["reset"], false);
    assert!(payload["text"].as_str().unwrap().contains("world"));
}

#[tokio::test]
async fn thread_logs_route_alias_returns_full_chunk() {
    let (state, logger, _dir) = test_state().await;
    let (thread_id, _) = create_thread_record(
        &state.threads.thread_store,
        ThreadEnsureOptions {
            label: Some("Logs".to_owned()),
            workspace_dir: None,
            workspace_mode: Default::default(),
            worktree_base_dir: None,
            agent_id: None,
            metadata: HashMap::new(),
            provider_type: None,
            sdk_session_id: None,
            thread_kind: None,
            origin_channel: None,
            origin_account_id: None,
            origin_from_id: None,
            is_group: None,
        },
    )
    .await
    .unwrap();
    logger
        .record_event(ThreadLogEvent::info(&thread_id, "run", "hello"))
        .await;

    let router = build_router(state.clone());
    let request = authed_request()
        .uri(format!("/api/threads/{thread_id}/logs"))
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["reset"], true);
    assert!(payload["text"].as_str().unwrap().contains("hello"));
}

#[tokio::test]
async fn create_thread_seeds_sdk_session_id() {
    let (state, _logger, _dir) = test_state().await;
    let workspace = tempdir().unwrap();
    let workspace_dir = workspace.path().to_string_lossy().to_string();
    let session_id = format!("claude-session-{}", uuid::Uuid::new_v4());
    let (thread_id, data, resolved) = create_thread_for_agent_reference(
        state.threads.thread_store.clone(),
        state.integration.bridge.clone(),
        state.ops.custom_agents.clone(),
        state.ops.agent_teams.clone(),
        ThreadEnsureOptions {
            label: Some("Resume Claude".to_owned()),
            workspace_dir: Some(workspace_dir),
            workspace_mode: Default::default(),
            worktree_base_dir: None,
            agent_id: Some("claude".to_owned()),
            metadata: HashMap::new(),
            provider_type: None,
            sdk_session_id: Some(session_id.clone()),
            thread_kind: None,
            origin_channel: None,
            origin_account_id: None,
            origin_from_id: None,
            is_group: None,
        },
    )
    .await
    .expect("thread created");
    let stored = state
        .threads
        .thread_store
        .get(&thread_id)
        .await
        .expect("stored thread");
    assert_eq!(resolved.provider_type(), ProviderType::ClaudeCode);
    assert_eq!(data["sdk_session_id"], session_id);
    assert_eq!(stored["provider_type"], "claude_code");
    assert_eq!(stored["sdk_session_id"], session_id);
}

#[tokio::test]
async fn create_thread_forks_provider_session_without_importing_visible_history() {
    let (state, _logger, _dir) = test_state().await;
    let workspace = tempdir().unwrap();
    let workspace_dir = workspace.path().to_string_lossy().to_string();
    let parent_session_id = "parent-claude-session";
    let (parent_thread_id, mut parent_data, _resolved) = create_thread_for_agent_reference(
        state.threads.thread_store.clone(),
        state.integration.bridge.clone(),
        state.ops.custom_agents.clone(),
        state.ops.agent_teams.clone(),
        ThreadEnsureOptions {
            label: Some("Main thread".to_owned()),
            workspace_dir: Some(workspace_dir.clone()),
            workspace_mode: Default::default(),
            worktree_base_dir: None,
            agent_id: Some("claude".to_owned()),
            metadata: HashMap::new(),
            provider_type: None,
            sdk_session_id: Some(parent_session_id.to_owned()),
            thread_kind: None,
            origin_channel: None,
            origin_account_id: None,
            origin_from_id: None,
            is_group: None,
        },
    )
    .await
    .expect("parent thread created");
    parent_data["messages"] = json!([
        {"role": "user", "content": "parent question"},
        {"role": "assistant", "content": "parent answer"}
    ]);
    parent_data["history"]["message_count"] = json!(2);
    state
        .threads
        .thread_store
        .set(&parent_thread_id, parent_data)
        .await;

    let router = build_router(state.clone());
    let request = authed_request()
        .method("POST")
        .uri("/api/threads")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "label": "Side chat",
                "forkFromThreadId": parent_thread_id
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let child_thread_id = payload["thread_id"].as_str().expect("child thread id");
    let child_data = state
        .threads
        .thread_store
        .get(child_thread_id)
        .await
        .expect("child thread stored");

    assert_eq!(child_data["label"], "Side chat");
    assert_eq!(child_data["workspace_dir"], workspace_dir);
    assert_eq!(child_data["agent_id"], "claude");
    assert_eq!(child_data["provider_type"], "claude_code");
    assert!(child_data.get("sdk_session_id").is_none());
    assert_eq!(
        child_data["metadata"][FORK_FROM_THREAD_ID_METADATA_KEY],
        parent_thread_id
    );
    assert_eq!(
        child_data["metadata"][FORK_FROM_SDK_SESSION_ID_METADATA_KEY],
        parent_session_id
    );
    assert_eq!(
        child_data["metadata"][FORK_FROM_PROVIDER_TYPE_METADATA_KEY],
        "claude_code"
    );
    assert!(
        child_data["metadata"][SDK_SESSION_FORK_METADATA_KEY]
            .as_bool()
            .unwrap_or(false)
    );
    assert_eq!(history_message_count(&child_data), 0);
    assert!(
        child_data
            .get("messages")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty),
        "fork child should not import parent messages into visible transcript"
    );
}

#[tokio::test]
async fn create_thread_rejects_fork_source_without_provider_session_id() {
    let (state, _logger, _dir) = test_state().await;
    let workspace = tempdir().unwrap();
    let workspace_dir = workspace.path().to_string_lossy().to_string();
    let (parent_thread_id, _parent_data, _resolved) = create_thread_for_agent_reference(
        state.threads.thread_store.clone(),
        state.integration.bridge.clone(),
        state.ops.custom_agents.clone(),
        state.ops.agent_teams.clone(),
        ThreadEnsureOptions {
            label: Some("Main thread".to_owned()),
            workspace_dir: Some(workspace_dir),
            workspace_mode: Default::default(),
            worktree_base_dir: None,
            agent_id: Some("claude".to_owned()),
            metadata: HashMap::new(),
            provider_type: None,
            sdk_session_id: None,
            thread_kind: None,
            origin_channel: None,
            origin_account_id: None,
            origin_from_id: None,
            is_group: None,
        },
    )
    .await
    .expect("parent thread created");

    let router = build_router(state);
    let request = authed_request()
        .method("POST")
        .uri("/api/threads")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "label": "Side chat",
                "forkFromThreadId": parent_thread_id
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert!(
        payload["error"]
            .as_str()
            .unwrap_or_default()
            .contains("fork source thread has no provider session id yet")
    );
}

#[tokio::test]
async fn create_thread_rejects_unknown_sdk_session_id() {
    let (state, _logger, _dir) = test_state().await;
    let router = build_router(state);
    let request = authed_request()
        .method("POST")
        .uri("/api/threads")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "sdkSessionId": "missing-local-provider-session-for-gateway-test"
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_thread_rejects_invalid_sdk_session_provider_hint() {
    let (state, _logger, _dir) = test_state().await;
    let router = build_router(state);
    let request = authed_request()
        .method("POST")
        .uri("/api/threads")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "sdkSessionId": "missing-local-provider-session-for-gateway-test",
                "sdkSessionProviderHint": "wat"
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert!(
        payload["error"]
            .as_str()
            .unwrap_or_default()
            .contains("Unsupported sdkSessionProviderHint")
    );
}

#[tokio::test]
async fn create_thread_persists_model_and_reasoning_overrides() {
    let (state, _logger, _dir) = test_state().await;
    let workspace = tempdir().unwrap();
    let router = build_router(state.clone());
    let request = authed_request()
        .method("POST")
        .uri("/api/threads")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "agentId": "claude",
                "workspaceDir": workspace.path().to_string_lossy(),
                "model": "claude-opus-4-7",
                "modelReasoningEffort": "xhigh",
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let thread_id = payload["thread_id"].as_str().expect("thread id");

    let stored = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("stored thread");
    assert_eq!(stored["metadata"]["model_override"], "claude-opus-4-7");
    assert_eq!(
        stored["metadata"]["model_reasoning_effort_override"],
        "xhigh"
    );
    assert!(
        stored["metadata"]
            .get("model_service_tier_override")
            .is_none()
    );
}

#[tokio::test]
async fn thread_pin_routes_persist_state_in_garyx_db() {
    let state = AppStateBuilder::new(test_config()).build();
    let thread_id = "thread::pin-route";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Pin Route",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z"
            }),
        )
        .await;
    let router = build_router(state.clone());

    let request = authed_request()
        .method("PUT")
        .uri(format!("/api/thread-pins/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let request = authed_request()
        .uri("/api/thread-pins")
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["thread_ids"], json!([thread_id]));

    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/thread-pins/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let request = authed_request()
        .uri("/api/thread-pins")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["thread_ids"], json!([]));
}

#[tokio::test]
async fn delete_thread_removes_garyx_db_pin() {
    let state = AppStateBuilder::new(test_config()).build();
    let thread_id = "thread::delete-pinned-route";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Delete Pinned Route",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z"
            }),
        )
        .await;
    state
        .ops
        .garyx_db
        .pin_thread(thread_id)
        .expect("pin test thread");
    let router = build_router(state.clone());

    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/threads/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        state
            .ops
            .garyx_db
            .list_pinned_threads()
            .expect("list pins")
            .is_empty()
    );
}

#[tokio::test]
async fn recent_threads_route_syncs_router_summary_to_garyx_db() {
    // The running fixture below seeds a dangling run_start; inject a probe that
    // confirms it as live so the projection treats it as a real running run.
    // Crash-orphan handling (dangling run with no live bridge run) is covered in
    // recent_thread_projection::tests.
    let state = AppStateBuilder::new(test_config())
        .with_active_run_probe(Arc::new(
            crate::recent_thread_projection::AlwaysActiveRunProbe,
        ))
        .build();
    state
        .threads
        .thread_store
        .set(
            "thread::recent-older",
            json!({
                "thread_id": "thread::recent-older",
                "label": "Recent Older",
                "created_at": "2026-05-23T08:00:00.000Z",
                "updated_at": "2026-05-23T09:00:00.000Z",
                "workspace_dir": "/work/test-older",
                "provider_type": "claude",
                "agent_id": "agent::test",
                "messages": [
                    {
                        "role": "user",
                        "content": "older user preview"
                    }
                ],
                "history": {
                    "message_count": 3,
                    "recent_committed_run_ids": ["run::older"]
                }
            }),
        )
        .await;
    append_dangling_run_start(&state, "thread::recent-running", "run::active").await;
    state
        .threads
        .thread_store
        .set(
            "thread::recent-running",
            json!({
                "thread_id": "thread::recent-running",
                "label": "Recent Running",
                "created_at": "2026-05-23T08:30:00.000Z",
                "updated_at": "2026-05-23T10:00:00.000Z",
                "workspace_dir": "/work/test-running",
                "provider_type": "codex",
                "agent_id": "agent::running",
                "messages": [
                    {
                        "role": "assistant",
                        "content": "running assistant preview"
                    }
                ],
                "history": {
                    "message_count": 4
                }
            }),
        )
        .await;
    let running_state = state
        .threads
        .history
        .transcript_store()
        .run_state("thread::recent-running")
        .await
        .expect("run state should reduce");
    assert!(running_state.busy);
    assert_eq!(running_state.active_run_id.as_deref(), Some("run::active"));
    let eager_projection = state
        .ops
        .garyx_db
        .list_recent_threads(10, 0)
        .expect("list eager projection")
        .into_iter()
        .find(|record| record.thread_id == "thread::recent-running")
        .expect("running row should project eagerly");
    assert_eq!(
        eager_projection.active_run_id.as_deref(),
        Some("run::active")
    );
    state
        .threads
        .thread_store
        .set(
            "thread::recent-no-timestamp",
            json!({
                "thread_id": "thread::recent-no-timestamp",
                "label": "No Timestamp"
            }),
        )
        .await;
    let router = build_router(state.clone());

    let request = authed_request()
        .uri("/api/recent-threads?limit=10&offset=0")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["count"], 3);
    assert_eq!(payload["total"], 3);
    assert_eq!(payload["offset"], 0);
    assert_eq!(payload["has_more"], false);
    assert_eq!(payload["threads"][0]["thread_id"], "thread::recent-running");
    assert_eq!(payload["threads"][0]["title"], "Recent Running");
    assert_eq!(payload["threads"][0]["workspace_dir"], "/work/test-running");
    assert_eq!(payload["threads"][0]["provider_type"], "codex");
    assert_eq!(payload["threads"][0]["agent_id"], "agent::running");
    assert_eq!(payload["threads"][0]["message_count"], 4);
    assert_eq!(
        payload["threads"][0]["last_message_preview"],
        "running assistant preview"
    );
    assert_eq!(payload["threads"][0]["active_run_id"], "run::active");
    assert_eq!(payload["threads"][0]["run_state"], "running");
    assert_eq!(payload["threads"][1]["thread_id"], "thread::recent-older");
    assert_eq!(payload["threads"][1]["provider_type"], "claude");
    assert_eq!(payload["threads"][1]["agent_id"], "agent::test");
    assert_eq!(payload["threads"][1]["message_count"], 3);
    assert_eq!(
        payload["threads"][1]["last_message_preview"],
        "older user preview"
    );
    assert_eq!(payload["threads"][1]["recent_run_id"], "run::older");
    assert_eq!(payload["threads"][1]["run_state"], "completed");
    assert_eq!(
        payload["threads"][2]["thread_id"],
        "thread::recent-no-timestamp"
    );
    assert_eq!(
        payload["threads"][2]["last_active_at"],
        "1970-01-01T00:00:00.000Z"
    );

    let persisted = state
        .ops
        .garyx_db
        .list_recent_threads(10, 0)
        .expect("list persisted recent threads");
    assert_eq!(
        persisted
            .iter()
            .map(|thread| thread.thread_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "thread::recent-running",
            "thread::recent-older",
            "thread::recent-no-timestamp"
        ],
    );
}

#[tokio::test]
async fn recent_threads_route_reads_persistent_projection_without_router_resync() {
    let state = AppStateBuilder::new(test_config()).build();
    let thread_id = "thread::recent-projection-only";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Canonical Thread Title",
                "created_at": "2026-05-23T08:00:00.000Z",
                "updated_at": "2026-05-23T09:00:00.000Z"
            }),
        )
        .await;
    state
        .ops
        .garyx_db
        .upsert_recent_thread(crate::garyx_db::RecentThreadDraft {
            thread_id: thread_id.to_owned(),
            title: "Projected Thread Title".to_owned(),
            workspace_dir: None,
            thread_type: "chat".to_owned(),
            provider_type: None,
            agent_id: None,
            message_count: 0,
            last_message_preview: String::new(),
            recent_run_id: None,
            active_run_id: None,
            run_state: "idle".to_owned(),
            updated_at: Some("2026-05-23T09:00:00.000Z".to_owned()),
            last_active_at: "2026-05-23T09:00:00.000Z".to_owned(),
        })
        .expect("overwrite recent projection");
    let router = build_router(state.clone());

    let request = authed_request()
        .uri("/api/recent-threads?limit=10")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["threads"][0]["thread_id"], thread_id);
    assert_eq!(payload["threads"][0]["title"], "Projected Thread Title");
}

#[tokio::test]
async fn recent_threads_route_defaults_to_thirty_threads() {
    let state = AppStateBuilder::new(test_config()).build();
    for index in 0..35 {
        let thread_id = format!("thread::recent-default-limit-{index:02}");
        state
            .ops
            .garyx_db
            .upsert_recent_thread(crate::garyx_db::RecentThreadDraft {
                thread_id: thread_id.clone(),
                title: format!("Recent Default Limit {index:02}"),
                workspace_dir: Some("/workspace/test".to_owned()),
                thread_type: "chat".to_owned(),
                provider_type: None,
                agent_id: None,
                message_count: index,
                last_message_preview: String::new(),
                recent_run_id: None,
                active_run_id: None,
                run_state: "idle".to_owned(),
                updated_at: Some(format!("2026-05-23T10:{index:02}:00.000Z")),
                last_active_at: format!("2026-05-23T10:{index:02}:00.000Z"),
            })
            .expect("seed recent projection");
    }
    let router = build_router(state.clone());

    let request = authed_request()
        .uri("/api/recent-threads")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["count"], 30);
    assert_eq!(payload["limit"], 30);
    assert_eq!(payload["offset"], 0);
    assert_eq!(payload["total"], 35);
    assert_eq!(payload["has_more"], true);
    assert_eq!(payload["threads"].as_array().unwrap().len(), 30);
}

#[tokio::test]
async fn recent_threads_route_removes_hidden_threads_from_projection() {
    let state = AppStateBuilder::new(test_config()).build();
    let thread_id = "thread::hidden-recent-route";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Hidden Recent Route",
                "created_at": "2026-05-23T08:00:00.000Z",
                "updated_at": "2026-05-23T09:00:00.000Z"
            }),
        )
        .await;
    let router = build_router(state.clone());

    let request = authed_request()
        .uri("/api/recent-threads?limit=10")
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        state
            .ops
            .garyx_db
            .list_recent_threads(10, 0)
            .expect("list synced recent threads")
            .len(),
        1,
    );

    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Hidden Recent Route",
                "hidden": true
            }),
        )
        .await;
    state.invalidate_thread_list_cache().await;

    let request = authed_request()
        .uri("/api/recent-threads?limit=10")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        state
            .ops
            .garyx_db
            .list_recent_threads(10, 0)
            .expect("list hidden-cleaned recent threads")
            .is_empty()
    );
}

#[tokio::test]
async fn delete_thread_removes_garyx_db_recent_thread() {
    let state = AppStateBuilder::new(test_config()).build();
    let thread_id = "thread::delete-recent-route";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Delete Recent Route",
                "created_at": "2026-05-23T08:00:00.000Z",
                "updated_at": "2026-05-23T09:00:00.000Z"
            }),
        )
        .await;
    state
        .ops
        .garyx_db
        .upsert_recent_thread(crate::garyx_db::RecentThreadDraft {
            thread_id: thread_id.to_owned(),
            title: "Delete Recent Route".to_owned(),
            workspace_dir: None,
            thread_type: "chat".to_owned(),
            provider_type: None,
            agent_id: None,
            message_count: 0,
            last_message_preview: String::new(),
            recent_run_id: None,
            active_run_id: None,
            run_state: "idle".to_owned(),
            updated_at: Some("2026-05-23T09:00:00.000Z".to_owned()),
            last_active_at: "2026-05-23T09:00:00.000Z".to_owned(),
        })
        .expect("seed recent thread");
    let router = build_router(state.clone());

    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/threads/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        state
            .ops
            .garyx_db
            .list_recent_threads(10, 0)
            .expect("list recent threads")
            .is_empty()
    );

    let request = authed_request()
        .uri("/api/recent-threads?limit=10")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert!(payload["threads"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn delete_thread_cleans_stale_projected_workflow_thread() {
    let state = AppStateBuilder::new(test_config()).build();
    let thread_id = "thread::stale-workflow-route";
    state
        .ops
        .garyx_db
        .replace_thread_meta_projection(ThreadMetaProjectionDraft {
            thread_id: thread_id.to_owned(),
            thread_meta: ThreadMetaDraft {
                thread_id: thread_id.to_owned(),
                workspace_dir: Some("/Users/test/project".to_owned()),
                thread_type: "workflow_run".to_owned(),
                thread_label: Some("Workflow Run".to_owned()),
                agent_id: Some("deep-research".to_owned()),
                provider_type: Some("workflow".to_owned()),
                created_at: Some("2026-06-05T08:00:00.000Z".to_owned()),
                updated_at: Some("2026-06-05T08:10:00.000Z".to_owned()),
                message_count: 1,
                last_user_message: Some("start workflow".to_owned()),
                last_assistant_message: None,
                last_message_preview: Some("start workflow".to_owned()),
                recent_run_id: None,
                active_run_id: None,
                worktree_json: None,
                last_delivery_context_json: None,
                last_delivery_updated_at: None,
                default_list_hidden: false,
            },
            channel_endpoints: vec![],
            message_routes: vec![],
        })
        .expect("seed stale workflow projection");
    state
        .ops
        .garyx_db
        .upsert_recent_thread(RecentThreadDraft {
            thread_id: thread_id.to_owned(),
            title: "Workflow Run".to_owned(),
            workspace_dir: Some("/Users/test/project".to_owned()),
            thread_type: "workflow_run".to_owned(),
            provider_type: Some("workflow".to_owned()),
            agent_id: Some("deep-research".to_owned()),
            message_count: 1,
            last_message_preview: "start workflow".to_owned(),
            recent_run_id: None,
            active_run_id: None,
            run_state: "idle".to_owned(),
            updated_at: Some("2026-06-05T08:10:00.000Z".to_owned()),
            last_active_at: "2026-06-05T08:10:00.000Z".to_owned(),
        })
        .expect("seed stale recent workflow");
    state
        .ops
        .garyx_db
        .pin_thread(thread_id)
        .expect("pin stale workflow thread");
    assert!(state.threads.thread_store.get(thread_id).await.is_none());

    let router = build_router(state.clone());
    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/threads/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["deleted"], true);
    assert_eq!(payload["thread_id"], thread_id);
    assert_eq!(payload["stale_projection"], true);
    assert!(
        state
            .ops
            .garyx_db
            .list_thread_meta()
            .expect("list thread meta")
            .is_empty()
    );
    assert!(
        state
            .ops
            .garyx_db
            .list_recent_threads(10, 0)
            .expect("list recent threads")
            .is_empty()
    );
    assert!(
        state
            .ops
            .garyx_db
            .list_pinned_threads()
            .expect("list pinned threads")
            .is_empty()
    );
}

#[tokio::test]
async fn threads_route_reads_full_thread_meta_projection_not_recent_subset() {
    let state = AppStateBuilder::new(test_config()).build();
    let thread_id = "thread::workspace-projection-only";
    append_dangling_run_start(&state, thread_id, "run::active-projection").await;
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Workspace Projection Only",
                "workspace_dir": "/Users/test/project",
                "created_at": "2026-05-23T08:30:00.000Z",
                "updated_at": "2026-05-23T09:00:00.000Z",
                "message_count": 2,
                "history": {
                    "recent_committed_run_ids": ["run::workspace-projection"]
                },
                "messages": [
                    {"role": "user", "content": "hello projection"},
                    {"role": "assistant", "content": "active answer"}
                ],
                "worktree": {
                    "path": "/Users/test/project/.garyx/worktree"
                }
            }),
        )
        .await;
    state
        .ops
        .garyx_db
        .remove_recent_thread(thread_id)
        .expect("remove from recent projection");
    state
        .ops
        .garyx_db
        .remove_thread_meta_projection(thread_id)
        .expect("remove from thread meta projection");
    let router = build_router(state);

    let request = authed_request()
        .uri("/api/threads?limit=1000")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["count"], 1);
    assert_eq!(payload["threads"][0]["thread_id"], thread_id);
    assert_eq!(
        payload["threads"][0]["workspace_dir"],
        "/Users/test/project"
    );
    assert_eq!(
        payload["threads"][0]["created_at"],
        "2026-05-23T08:30:00.000Z"
    );
    assert_eq!(payload["threads"][0]["message_count"], 2);
    assert_eq!(
        payload["threads"][0]["last_user_message"],
        "hello projection"
    );
    assert_eq!(
        payload["threads"][0]["last_assistant_message"],
        "active answer"
    );
    assert_eq!(
        payload["threads"][0]["last_message_preview"],
        "active answer"
    );
    assert_eq!(
        payload["threads"][0]["recent_run_id"],
        "run::workspace-projection"
    );
    assert_eq!(
        payload["threads"][0]["active_run_id"],
        "run::active-projection"
    );
    assert_eq!(
        payload["threads"][0]["worktree"]["path"],
        "/Users/test/project/.garyx/worktree"
    );
}

#[tokio::test]
async fn threads_route_filters_default_hidden_threads_from_meta_projection() {
    let state = AppStateBuilder::new(test_config()).build();
    state
        .threads
        .thread_store
        .set(
            "thread::visible-meta",
            json!({
                "thread_id": "thread::visible-meta",
                "label": "Visible",
                "updated_at": "2026-05-23T09:00:00.000Z"
            }),
        )
        .await;
    state
        .threads
        .thread_store
        .set(
            "thread::hidden-meta",
            json!({
                "thread_id": "thread::hidden-meta",
                "label": "Hidden",
                "workflow_child_run_id": "workflow-child::1",
                "updated_at": "2026-05-23T10:00:00.000Z"
            }),
        )
        .await;
    let router = build_router(state);

    let request = authed_request()
        .uri("/api/threads")
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["total"], 1);
    assert_eq!(payload["threads"][0]["thread_id"], "thread::visible-meta");

    let request = authed_request()
        .uri("/api/threads?include_hidden=true")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["total"], 2);
}

#[tokio::test]
async fn dream_scan_route_persists_thread_topic_spans() {
    let state = AppStateBuilder::new(test_config()).build();
    let thread_id = "thread::dream-route";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Dream Route",
                "created_at": "2026-05-21T09:00:00Z",
                "updated_at": "2026-05-21T10:10:00Z",
                "workspace_dir": "/workspace/test"
            }),
        )
        .await;
    let mut first = ProviderMessage::user_text("Review pinned threads in the gateway");
    first.timestamp = Some("2026-05-21T10:00:00Z".to_owned());
    let mut second = ProviderMessage::user_text("另外实现梦境的一天主题列表");
    second.timestamp = Some("2026-05-21T10:06:00Z".to_owned());
    state
        .threads
        .history
        .transcript_store()
        .append_committed_messages(
            thread_id,
            Some("run::dream-route"),
            &[
                serde_json::to_value(first).expect("first message serializes"),
                serde_json::to_value(second).expect("second message serializes"),
            ],
        )
        .await
        .expect("append transcript");
    let router = build_router(state.clone());

    let request = authed_request()
        .method("POST")
        .uri("/api/dreams/scan")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "from": "2026-05-21T00:00:00Z",
                "to": "2026-05-21T23:59:59Z",
                "mode": "heuristic"
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["matched_messages"], json!(2));
    assert_eq!(payload["count"], json!(2));
    assert_eq!(payload["dreams"][0]["spans"][0]["thread_id"], thread_id);

    let dream_id = payload["dreams"][0]["dream_id"].as_str().unwrap();
    let request = authed_request()
        .uri(format!("/api/dreams/{dream_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let detail: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        detail["dream"]["spans"][0]["workspace_dir"],
        json!("/workspace/test")
    );
}

#[tokio::test]
async fn dream_scan_route_preserves_historical_incremental_topics() {
    let state = AppStateBuilder::new(test_config()).build();
    let thread_id = "thread::dream-history";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Dream History",
                "updated_at": "2026-05-21T10:10:00Z",
                "workspace_dir": "/workspace/test"
            }),
        )
        .await;
    state
        .ops
        .garyx_db
        .replace_dreams_in_window(
            "2026-05-20T00:00:00.000Z",
            "2026-05-21T23:59:59.999Z",
            "claude",
            &[DreamTopicDraft {
                dream_id: "dream::historical".to_owned(),
                title: "Historical Dream".to_owned(),
                summary: "A topic that started before the manual scan window.".to_owned(),
                first_message_at: "2026-05-20T10:00:00.000Z".to_owned(),
                last_message_at: "2026-05-21T09:00:00.000Z".to_owned(),
                source: "claude".to_owned(),
                confidence: 0.8,
                message_count: 1,
                spans: vec![DreamSpanDraft {
                    span_id: "span::historical".to_owned(),
                    thread_id: thread_id.to_owned(),
                    workspace_dir: Some("/workspace/test".to_owned()),
                    start_seq: 1,
                    end_seq: 1,
                    start_at: "2026-05-20T10:00:00.000Z".to_owned(),
                    end_at: "2026-05-20T10:10:00.000Z".to_owned(),
                    excerpt: "historical".to_owned(),
                    message_count: 1,
                }],
            }],
            None,
        )
        .expect("seed historical topic");
    let mut assistant = ProviderMessage::assistant_text("prior assistant message");
    assistant.timestamp = Some("2026-05-21T09:50:00Z".to_owned());
    let mut user = ProviderMessage::user_text("Continue dreams from the manual scan window");
    user.timestamp = Some("2026-05-21T10:00:00Z".to_owned());
    state
        .threads
        .history
        .transcript_store()
        .append_committed_messages(
            thread_id,
            Some("run::dream-history"),
            &[
                serde_json::to_value(assistant).expect("assistant message serializes"),
                serde_json::to_value(user).expect("user message serializes"),
            ],
        )
        .await
        .expect("append transcript");
    let router = build_router(state.clone());

    let request = authed_request()
        .method("POST")
        .uri("/api/dreams/scan")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "from": "2026-05-21T00:00:00Z",
                "to": "2026-05-21T23:59:59Z",
                "mode": "heuristic"
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    assert!(
        state
            .ops
            .garyx_db
            .get_dream_topic("dream::historical")
            .expect("get historical dream")
            .is_some()
    );
}

#[tokio::test]
async fn update_thread_persists_and_clears_model_overrides() {
    let (state, _logger, _dir) = test_state().await;
    let thread_id = "thread::model-update";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Model update",
                "metadata": {},
            }),
        )
        .await;

    let router = build_router(state.clone());
    let request = authed_request()
        .method("PATCH")
        .uri("/api/threads/thread%3A%3Amodel-update")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "model": " claude-opus-4-7 ",
                "modelReasoningEffort": " max ",
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let stored = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("stored thread after update");
    assert_eq!(stored["metadata"]["model_override"], "claude-opus-4-7");
    assert_eq!(stored["metadata"]["model_reasoning_effort_override"], "max");

    let request = authed_request()
        .method("PATCH")
        .uri("/api/threads/thread%3A%3Amodel-update")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "model": "",
                "modelReasoningEffort": "",
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let stored = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("stored thread after clear");
    assert!(stored["metadata"].get("model_override").is_none());
    assert!(
        stored["metadata"]
            .get("model_reasoning_effort_override")
            .is_none()
    );
}

#[tokio::test]
async fn thread_history_runtime_reports_effective_model_overrides() {
    let (state, _logger, _dir) = test_state().await;
    let thread_id = "thread::runtime-model";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Runtime model",
                "agent_id": "claude",
                "provider_type": "claude_code",
                "metadata": {
                    "model_override": "claude-opus-4-7",
                    "model_reasoning_effort_override": "max",
                },
            }),
        )
        .await;

    let router = build_router(state);
    let request = authed_request()
        .uri("/api/threads/history?thread_id=thread%3A%3Aruntime-model&limit=1")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["thread_runtime"]["agent_id"], "claude");
    assert_eq!(payload["thread_runtime"]["provider_type"], "claude_code");
    assert_eq!(payload["thread_runtime"]["model"], "claude-opus-4-7");
    assert_eq!(payload["thread_runtime"]["model_reasoning_effort"], "max");
    assert_eq!(
        payload["thread_runtime"]["model_override"],
        "claude-opus-4-7"
    );
    assert_eq!(
        payload["thread_runtime"]["model_reasoning_effort_override"],
        "max"
    );
}

#[tokio::test]
async fn thread_summary_routes_include_runtime_summary() {
    let (state, _logger, _dir) = test_state().await;
    let thread_id = "thread::runtime-summary-routes";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Runtime summary routes",
                "agent_id": "codex",
                "provider_type": "codex_app_server",
                "metadata": {
                    "model_override": "gpt-5.5",
                    "model_reasoning_effort_override": "xhigh",
                },
                "sdk_session_id": "sdk-codex-123",
                "created_at": "2026-06-13T10:00:00.000Z",
                "updated_at": "2026-06-13T10:01:00.000Z",
            }),
        )
        .await;
    state
        .ops
        .garyx_db
        .replace_thread_meta_projection(ThreadMetaProjectionDraft {
            thread_id: thread_id.to_owned(),
            thread_meta: ThreadMetaDraft {
                thread_id: thread_id.to_owned(),
                workspace_dir: None,
                thread_type: "chat".to_owned(),
                thread_label: Some("Runtime summary routes".to_owned()),
                agent_id: Some("codex".to_owned()),
                provider_type: Some("codex_app_server".to_owned()),
                created_at: Some("2026-06-13T10:00:00.000Z".to_owned()),
                updated_at: Some("2026-06-13T10:01:00.000Z".to_owned()),
                message_count: 1,
                last_user_message: Some("hello".to_owned()),
                last_assistant_message: Some("hi".to_owned()),
                last_message_preview: Some("hi".to_owned()),
                recent_run_id: None,
                active_run_id: None,
                worktree_json: None,
                last_delivery_context_json: None,
                last_delivery_updated_at: None,
                default_list_hidden: false,
            },
            channel_endpoints: vec![],
            message_routes: vec![],
        })
        .expect("seed thread meta projection");
    state
        .ops
        .garyx_db
        .upsert_recent_thread(RecentThreadDraft {
            thread_id: thread_id.to_owned(),
            title: "Runtime summary routes".to_owned(),
            workspace_dir: None,
            thread_type: "chat".to_owned(),
            provider_type: Some("codex_app_server".to_owned()),
            agent_id: Some("codex".to_owned()),
            message_count: 1,
            last_message_preview: "hi".to_owned(),
            recent_run_id: None,
            active_run_id: None,
            run_state: "idle".to_owned(),
            updated_at: Some("2026-06-13T10:01:00.000Z".to_owned()),
            last_active_at: "2026-06-13T10:01:00.000Z".to_owned(),
        })
        .expect("seed recent thread projection");

    let router = build_router(state);
    for (uri, nested_in_threads) in [
        ("/api/threads/thread::runtime-summary-routes", false),
        (
            "/api/threads?limit=10&prefix=thread%3A%3Aruntime-summary-routes",
            true,
        ),
        ("/api/recent-threads?limit=10", true),
    ] {
        let request = authed_request().uri(uri).body(Body::empty()).unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "route {uri}");
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        let runtime = if nested_in_threads {
            &payload["threads"][0]["thread_runtime"]
        } else {
            &payload["thread_runtime"]
        };
        assert_eq!(runtime["agent_id"], "codex", "route {uri}");
        assert_eq!(runtime["provider_type"], "codex_app_server", "route {uri}");
        assert_eq!(runtime["model"], "gpt-5.5", "route {uri}");
        assert_eq!(runtime["model_reasoning_effort"], "xhigh", "route {uri}");
        assert_eq!(runtime["model_override"], "gpt-5.5", "route {uri}");
        assert_eq!(
            runtime["model_reasoning_effort_override"], "xhigh",
            "route {uri}"
        );
        assert_eq!(runtime["sdk_session_id"], "sdk-codex-123", "route {uri}");
    }
}

#[tokio::test]
async fn thread_history_runtime_reports_provider_default_alias() {
    let mut config = test_config();
    config.agents.insert(
        "openai".to_owned(),
        json!({
            "provider_type": "gpt",
            "default_model": "gpt-5.4",
            "model_reasoning_effort": "high",
        }),
    );
    let state = AppStateBuilder::new(config)
        .with_custom_agent_store(Arc::new(crate::custom_agents::CustomAgentStore::new()))
        .with_agent_team_store(Arc::new(crate::agent_teams::AgentTeamStore::new()))
        .build();
    let thread_id = "thread::runtime-provider-default";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Runtime provider default",
                "agent_id": "gpt",
                "provider_type": "gpt",
                "metadata": {},
            }),
        )
        .await;

    let router = build_router(state);
    let request = authed_request()
        .uri("/api/threads/history?thread_id=thread%3A%3Aruntime-provider-default&limit=1")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["thread_runtime"]["agent_id"], "gpt");
    assert_eq!(payload["thread_runtime"]["provider_type"], "gpt");
    assert_eq!(payload["thread_runtime"]["model"], "gpt-5.4");
    assert_eq!(payload["thread_runtime"]["model_reasoning_effort"], "high");
    assert!(payload["thread_runtime"]["model_override"].is_null());
    assert!(payload["thread_runtime"]["model_reasoning_effort_override"].is_null());
}

#[tokio::test]
async fn thread_history_runtime_leaves_cli_provider_defaults_empty() {
    let (state, _logger, _dir) = test_state().await;
    for (thread_id, provider_type) in [
        ("thread::runtime-codex-cli-default", "codex_app_server"),
        ("thread::runtime-claude-cli-default", "claude_code"),
        ("thread::runtime-gemini-cli-default", "gemini_cli"),
    ] {
        state
            .threads
            .thread_store
            .set(
                thread_id,
                json!({
                    "thread_id": thread_id,
                    "label": "Runtime CLI default",
                    "provider_type": provider_type,
                    "metadata": {},
                }),
            )
            .await;
    }

    let router = build_router(state);
    for (encoded_thread_id, provider_type) in [
        ("thread%3A%3Aruntime-codex-cli-default", "codex_app_server"),
        ("thread%3A%3Aruntime-claude-cli-default", "claude_code"),
        ("thread%3A%3Aruntime-gemini-cli-default", "gemini_cli"),
    ] {
        let request = authed_request()
            .uri(format!(
                "/api/threads/history?thread_id={encoded_thread_id}&limit=1"
            ))
            .body(Body::empty())
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert!(payload["thread_runtime"]["agent_id"].is_null());
        assert_eq!(payload["thread_runtime"]["provider_type"], provider_type);
        assert!(payload["thread_runtime"]["model"].is_null());
        assert!(payload["thread_runtime"]["model_reasoning_effort"].is_null());
    }
}

#[tokio::test]
async fn thread_history_runtime_reports_native_builtin_provider_default() {
    let (state, _logger, _dir) = test_state().await;
    let thread_id = "thread::runtime-native-builtin-default";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Runtime native builtin default",
                "agent_id": "gpt",
                "provider_type": "gpt",
                "metadata": {},
            }),
        )
        .await;

    let router = build_router(state);
    let request = authed_request()
        .uri("/api/threads/history?thread_id=thread%3A%3Aruntime-native-builtin-default&limit=1")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["thread_runtime"]["agent_id"], "gpt");
    assert_eq!(payload["thread_runtime"]["provider_type"], "gpt");
    assert_eq!(payload["thread_runtime"]["model"], "gpt-5.5");
    assert_eq!(
        payload["thread_runtime"]["model_reasoning_effort"],
        "medium"
    );
}

#[tokio::test]
async fn create_thread_rejects_unknown_sdk_session_id_for_requested_provider() {
    let (state, _logger, _dir) = test_state().await;
    let router = build_router(state);
    let request = authed_request()
        .method("POST")
        .uri("/api/threads")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "sdkSessionId": "missing-local-provider-session-for-gateway-test",
                "sdkSessionProviderHint": "codex"
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert!(
        payload["error"]
            .as_str()
            .unwrap_or_default()
            .contains("No local Codex session was found")
    );
}

#[tokio::test]
async fn seed_imported_thread_history_persists_transcript_and_thread_state() {
    let (state, _logger, _dir) = test_state().await;
    let workspace = tempdir().unwrap();
    let workspace_dir = workspace.path().to_string_lossy().to_string();
    let (thread_id, mut data, _resolved) = create_thread_for_agent_reference(
        state.threads.thread_store.clone(),
        state.integration.bridge.clone(),
        state.ops.custom_agents.clone(),
        state.ops.agent_teams.clone(),
        ThreadEnsureOptions {
            label: Some("Recovered Session".to_owned()),
            workspace_dir: Some(workspace_dir),
            workspace_mode: Default::default(),
            worktree_base_dir: None,
            agent_id: Some("claude".to_owned()),
            metadata: HashMap::new(),
            provider_type: None,
            sdk_session_id: Some("recovered-session".to_owned()),
            thread_kind: None,
            origin_channel: None,
            origin_account_id: None,
            origin_from_id: None,
            is_group: None,
        },
    )
    .await
    .expect("thread created");

    let imported_messages = vec![
        json!({
            "role": "user",
            "content": "hello",
            "timestamp": "2026-04-14T00:00:00Z"
        }),
        json!({
            "role": "assistant",
            "content": "world",
            "timestamp": "2026-04-14T00:00:01Z"
        }),
    ];

    seed_imported_thread_history(&state, &thread_id, &mut data, &imported_messages)
        .await
        .expect("seed imported history");

    let stored = state
        .threads
        .thread_store
        .get(&thread_id)
        .await
        .expect("stored thread");
    assert_eq!(stored["history"]["message_count"], 2);
    assert_eq!(stored["message_count"], 2);
    assert_eq!(
        stored["messages"].as_array().expect("messages array").len(),
        2
    );

    let snapshot = state
        .threads
        .history
        .thread_snapshot(&thread_id, 10)
        .await
        .expect("snapshot");
    let combined = snapshot.combined_messages();
    assert_eq!(combined.len(), 2);
    assert_eq!(combined[0]["content"], "hello");
    assert_eq!(combined[1]["content"], "world");
}

#[tokio::test]
async fn create_thread_rejects_unknown_agent_id() {
    let (state, _logger, _dir) = test_state().await;
    let router = build_router(state);
    for agent_id in ["definitely-not-real", "gpt"] {
        let request = authed_request()
            .method("POST")
            .uri("/api/threads")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "label": "Bad thread",
                    "agentId": agent_id
                })
                .to_string(),
            ))
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{agent_id} should not resolve as an agent id"
        );
    }
}

#[tokio::test]
async fn create_thread_without_workspace_uses_private_thread_workspace() {
    let data_dir = tempdir().unwrap();
    let mut config = test_config();
    config.sessions.data_dir = Some(data_dir.path().join("data").to_string_lossy().to_string());
    let state = AppStateBuilder::new(config)
        .with_custom_agent_store(Arc::new(crate::custom_agents::CustomAgentStore::new()))
        .with_agent_team_store(Arc::new(crate::agent_teams::AgentTeamStore::new()))
        .build();
    let router = build_router(state.clone());

    let request = authed_request()
        .method("POST")
        .uri("/api/threads")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "label": "No workspace thread"
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let thread_id = payload["thread_id"].as_str().expect("thread id");
    let workspace_dir = payload["workspace_dir"].as_str().expect("workspace dir");

    assert!(
        Path::new(workspace_dir).starts_with(data_dir.path().join("thread-workspaces")),
        "workspace_dir should be inside private thread workspace root: {workspace_dir}"
    );
    assert!(Path::new(workspace_dir).exists());
    let stored = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("stored thread");
    assert_eq!(
        workspace_dir_from_value(&stored).as_deref(),
        Some(workspace_dir)
    );
    assert!(
        state.ops.garyx_db.list_workspaces().unwrap().is_empty(),
        "private thread workspace must not be registered as a user workspace"
    );
}

#[tokio::test]
async fn git_status_marks_only_git_root_as_worktree_capable() {
    let (state, _logger, _dir) = test_state().await;
    let repo = tempdir().unwrap();
    init_test_git_repo(repo.path());
    let nested = repo.path().join("nested");
    std::fs::create_dir(&nested).expect("nested dir");
    let router = build_router(state);

    let request = authed_request()
        .uri(format!(
            "/api/workspaces/git-status?workspace_dir={}",
            urlencoding::encode(&repo.path().to_string_lossy())
        ))
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["is_git_repo"], true);
    assert_eq!(
        payload["repo_root"].as_str(),
        Some(
            repo.path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .as_ref()
        )
    );
    assert_eq!(
        payload["current_branch"].as_str(),
        Some(git_output(repo.path(), &["branch", "--show-current"]).as_str())
    );

    let request = authed_request()
        .uri(format!(
            "/api/workspaces/git-status?workspace_dir={}",
            urlencoding::encode(&nested.to_string_lossy())
        ))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["is_git_repo"], false);
    assert_eq!(
        payload["repo_root"].as_str(),
        Some(
            repo.path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .as_ref()
        )
    );
}

#[tokio::test]
async fn workspaces_route_seeds_from_config_only_when_workspace_table_is_empty() {
    let mut config = test_config();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            PluginAccountEntry {
                name: Some("Bot Repo".to_owned()),
                workspace_dir: Some("/workspace/bot-repo".to_owned()),
                ..PluginAccountEntry::default()
            },
        );
    config.cron.jobs.push(CronJobConfig {
        id: "cron::nightly".to_owned(),
        kind: CronJobKind::AutomationPrompt,
        label: Some("Nightly Repo".to_owned()),
        schedule: CronSchedule::default(),
        ui_schedule: None,
        action: CronAction::AgentTurn,
        target: None,
        message: Some("Run nightly check".to_owned()),
        workspace_dir: Some("/workspace/cron-repo".to_owned()),
        agent_id: None,
        thread_id: None,
        delete_after_run: false,
        enabled: true,
        system: false,
    });
    config.cron.jobs.push(CronJobConfig {
        id: "cron::relative".to_owned(),
        kind: CronJobKind::AutomationPrompt,
        label: Some("Relative Repo".to_owned()),
        schedule: CronSchedule::default(),
        ui_schedule: None,
        action: CronAction::AgentTurn,
        target: None,
        message: Some("Run relative check".to_owned()),
        workspace_dir: Some("relative/repo".to_owned()),
        agent_id: None,
        thread_id: None,
        delete_after_run: false,
        enabled: true,
        system: false,
    });
    let state = AppStateBuilder::new(config).build();
    create_thread_record(
        &state.threads.thread_store,
        ThreadEnsureOptions {
            label: Some("Inferred only".to_owned()),
            workspace_dir: Some("/workspace/inferred-only".to_owned()),
            workspace_mode: Default::default(),
            worktree_base_dir: None,
            agent_id: None,
            metadata: HashMap::new(),
            provider_type: None,
            sdk_session_id: None,
            thread_kind: None,
            origin_channel: None,
            origin_account_id: None,
            origin_from_id: None,
            is_group: None,
        },
    )
    .await
    .unwrap();
    let router = build_router(state);

    let request = authed_request()
        .uri("/api/workspaces")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let workspaces = payload["workspaces"].as_array().unwrap();

    assert_eq!(workspaces.len(), 2);
    assert!(
        workspaces
            .iter()
            .any(|workspace| workspace["path"] == "/workspace/bot-repo"
                && workspace["name"] == "Bot Repo")
    );
    assert!(
        workspaces
            .iter()
            .any(|workspace| workspace["path"] == "/workspace/cron-repo"
                && workspace["name"] == "Nightly Repo")
    );
    assert!(
        !workspaces
            .iter()
            .any(|workspace| workspace["path"] == "/workspace/inferred-only")
    );
    assert!(
        !workspaces
            .iter()
            .any(|workspace| workspace["path"] == "relative/repo")
    );
}

#[tokio::test]
async fn workspaces_route_persists_add_and_delete() {
    let mut config = test_config();
    config.gateway.auth_token = crate::test_support::TEST_GATEWAY_TOKEN.to_owned();
    let state = AppStateBuilder::new(config).build();
    let router = build_router(state.clone());

    let request = authed_request()
        .method("POST")
        .uri("/api/workspaces")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "path": "/workspace/saved",
                "name": "Saved"
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        state.ops.garyx_db.list_workspaces().unwrap()[0].path,
        "/workspace/saved"
    );

    let request = authed_request()
        .method("DELETE")
        .uri(format!(
            "/api/workspaces?path={}",
            urlencoding::encode("/workspace/saved")
        ))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(state.ops.garyx_db.list_workspaces().unwrap().is_empty());
    assert_eq!(state.ops.garyx_db.count_workspace_rows().unwrap(), 1);
}

#[tokio::test]
async fn workspaces_route_rejects_relative_path() {
    let mut config = test_config();
    config.gateway.auth_token = crate::test_support::TEST_GATEWAY_TOKEN.to_owned();
    let state = AppStateBuilder::new(config).build();
    let router = build_router(state);

    let request = authed_request()
        .method("POST")
        .uri("/api/workspaces")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "path": "relative/project",
                "name": "Relative"
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn workspaces_route_does_not_seed_when_only_deleted_rows_exist() {
    let mut config = test_config();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            PluginAccountEntry {
                name: Some("Deleted Bot Workspace".to_owned()),
                workspace_dir: Some("/workspace/from-config".to_owned()),
                ..PluginAccountEntry::default()
            },
        );
    config.channels.api.accounts.insert(
        "not-a-bot".to_owned(),
        ApiAccount {
            workspace_dir: Some("/workspace/from-config".to_owned()),
            ..ApiAccount::default()
        },
    );
    let state = AppStateBuilder::new(config).build();
    state
        .ops
        .garyx_db
        .upsert_workspace(WorkspaceDraft {
            name: None,
            path: "/workspace/from-config".to_owned(),
        })
        .unwrap();
    state
        .ops
        .garyx_db
        .delete_workspace("/workspace/from-config")
        .unwrap();
    let router = build_router(state);

    let request = authed_request()
        .uri("/api/workspaces")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert!(payload["workspaces"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn create_thread_with_worktree_creates_managed_git_worktree() {
    let repo = tempdir().unwrap();
    init_test_git_repo(repo.path());
    let data_dir = tempdir().unwrap();
    let mut config = test_config();
    config.sessions.data_dir = Some(data_dir.path().join("data").to_string_lossy().to_string());
    let state = AppStateBuilder::new(config).build();
    let router = build_router(state.clone());

    let request = authed_request()
        .method("POST")
        .uri("/api/threads")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "label": "Worktree thread",
                "workspaceDir": repo.path(),
                "workspaceMode": "worktree"
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let thread_id = payload["thread_id"].as_str().expect("thread id");
    let workspace_dir = payload["workspace_dir"].as_str().expect("workspace dir");
    assert_ne!(workspace_dir, repo.path().to_string_lossy().as_ref());
    assert!(Path::new(workspace_dir).exists());
    assert!(
        Path::new(workspace_dir).starts_with(data_dir.path().join("worktrees")),
        "workspace_dir should be inside managed worktree root: {workspace_dir}"
    );
    assert_eq!(
        git_output(Path::new(workspace_dir), &["rev-parse", "--show-toplevel"]),
        Path::new(workspace_dir)
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );

    let stored = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("stored thread");
    assert_eq!(stored["workspace_dir"], workspace_dir);
    assert_eq!(stored["worktree"]["mode"], "worktree");
    assert_eq!(stored["worktree"]["enabled"], true);
    assert_eq!(
        stored["worktree"]["source_workspace_dir"].as_str(),
        Some(
            repo.path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .as_ref()
        )
    );
    assert_eq!(
        stored["worktree"]["source_repo_root"].as_str(),
        Some(
            repo.path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .as_ref()
        )
    );
    assert_eq!(stored["worktree"]["path"], workspace_dir);
    assert_eq!(stored["worktree"]["worktree_dir"], workspace_dir);
    assert_eq!(stored["worktree"]["thread_id"], thread_id);
    assert!(
        stored["worktree"]["created_at"]
            .as_str()
            .is_some_and(|value| !value.trim().is_empty())
    );
    assert!(
        stored["worktree"]["branch"]
            .as_str()
            .unwrap()
            .starts_with("garyx/")
    );
    assert_eq!(
        stored["worktree"]["base_commit"],
        git_output(repo.path(), &["rev-parse", "HEAD"])
    );
    assert_eq!(
        stored["worktree"]["base_head"],
        git_output(repo.path(), &["rev-parse", "HEAD"])
    );
}

#[tokio::test]
async fn create_thread_worktree_rejects_non_git_root_workspace() {
    let repo = tempdir().unwrap();
    init_test_git_repo(repo.path());
    let nested = repo.path().join("nested");
    std::fs::create_dir(&nested).expect("nested dir");
    let data_dir = tempdir().unwrap();
    let mut config = test_config();
    config.sessions.data_dir = Some(data_dir.path().join("data").to_string_lossy().to_string());
    let state = AppStateBuilder::new(config).build();
    let router = build_router(state);

    let request = authed_request()
        .method("POST")
        .uri("/api/threads")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "label": "Bad worktree",
                "workspaceDir": nested,
                "workspaceMode": "worktree"
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert!(
        payload["error"]
            .as_str()
            .unwrap_or_default()
            .contains("git repository root")
    );
}

#[tokio::test]
async fn create_thread_worktree_rejects_git_repo_without_head_as_bad_request() {
    let repo = tempdir().unwrap();
    run_git(repo.path(), &["init"]);
    let data_dir = tempdir().unwrap();
    let mut config = test_config();
    config.sessions.data_dir = Some(data_dir.path().join("data").to_string_lossy().to_string());
    let state = AppStateBuilder::new(config).build();
    let router = build_router(state.clone());

    let request = authed_request()
        .method("POST")
        .uri("/api/threads")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "label": "Empty repo worktree",
                "workspaceDir": repo.path(),
                "workspaceMode": "worktree"
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let error = payload["error"].as_str().unwrap_or_default();
    assert!(error.starts_with("workspace_mode=worktree failed:"));
    assert!(error.contains("rev-parse HEAD"));
    assert!(
        state
            .threads
            .thread_store
            .list_keys(Some("thread::"))
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn update_thread_accepts_encoded_thread_path_segment() {
    let (state, _logger, _dir) = test_state().await;
    let thread_id = "thread::with/slash";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Before",
            }),
        )
        .await;

    let router = build_router(state);
    let request = authed_request()
        .method("PATCH")
        .uri("/api/threads/thread%3A%3Awith%2Fslash")
        .header("content-type", "application/json")
        .body(Body::from(json!({ "label": "After" }).to_string()))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["thread_id"], thread_id);
    assert_eq!(payload["label"], "After");
}

#[tokio::test]
async fn delete_thread_removes_thread_log_file() {
    let (state, logger, _dir) = test_state().await;
    let (thread_id, _) = create_thread_record(
        &state.threads.thread_store,
        ThreadEnsureOptions {
            label: Some("Delete".to_owned()),
            workspace_dir: None,
            workspace_mode: Default::default(),
            worktree_base_dir: None,
            agent_id: None,
            metadata: HashMap::new(),
            provider_type: None,
            sdk_session_id: None,
            thread_kind: None,
            origin_channel: None,
            origin_account_id: None,
            origin_from_id: None,
            is_group: None,
        },
    )
    .await
    .unwrap();
    logger
        .record_event(ThreadLogEvent::info(&thread_id, "run", "to-delete"))
        .await;
    let log_path = logger.thread_log_path(&thread_id);
    assert!(log_path.exists());

    let router = build_router(state);
    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/threads/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!log_path.exists());
}

#[tokio::test]
async fn delete_thread_rejects_enabled_channel_binding() {
    let mut config = test_config();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(&TelegramAccount {
                token: "token-main".to_owned(),
                enabled: true,
                name: None,
                agent_id: "claude".to_owned(),
                workspace_dir: None,
                owner_target: None,
                groups: std::collections::HashMap::new(),
            }),
        );

    let state = AppStateBuilder::new(config).build();
    let thread_id = "thread::delete-bound-enabled";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Bound Enabled",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "binding_key": "u1",
                    "chat_id": "u1",
                    "delivery_target_type": DELIVERY_TARGET_TYPE_CHAT_ID,
                    "delivery_target_id": "u1",
                    "display_label": "u1"
                }]
            }),
        )
        .await;

    let router = build_router(state.clone());
    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/threads/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        payload["error"],
        "cannot delete thread with active channel bindings"
    );
    assert!(state.threads.thread_store.get(thread_id).await.is_some());
}

#[test]
fn archive_thread_body_accepts_snake_case_endpoint_keys_alias() {
    let body: ArchiveThreadBody = serde_json::from_value(json!({
        "endpoint_keys": ["api::main::loop"]
    }))
    .unwrap();

    assert_eq!(body.endpoint_keys, vec!["api::main::loop"]);
}

#[tokio::test]
async fn archive_thread_detaches_live_channel_binding_and_prevents_recent_revival() {
    let mut config = test_config();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(&TelegramAccount {
                token: "${TOKEN}".to_owned(),
                enabled: true,
                name: Some("Test Telegram".to_owned()),
                agent_id: "claude".to_owned(),
                workspace_dir: Some("/Users/test/project".to_owned()),
                owner_target: None,
                groups: std::collections::HashMap::new(),
            }),
        );

    let state = AppStateBuilder::new(config).build();
    let thread_id = "thread::archive-bound-telegram";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "New Thread",
                "workspace_dir": "/Users/test/project",
                "created_at": "2026-06-21T08:00:00.000Z",
                "updated_at": "2026-06-21T08:01:00.000Z",
                "messages": [
                    {"role": "user", "content": "reconnect proof"}
                ],
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "binding_key": "1000000001",
                    "chat_id": "1000000001",
                    "delivery_target_type": DELIVERY_TARGET_TYPE_CHAT_ID,
                    "delivery_target_id": "1000000001",
                    "display_label": "Test User",
                    "last_inbound_at": "2026-06-21T08:01:00.000Z"
                }]
            }),
        )
        .await;
    state
        .ops
        .garyx_db
        .pin_thread(thread_id)
        .expect("pin archived candidate");
    assert_eq!(
        state
            .ops
            .garyx_db
            .list_recent_threads(10, 0)
            .expect("seed recent projection")
            .len(),
        1
    );
    assert_eq!(
        state
            .ops
            .garyx_db
            .list_thread_meta()
            .expect("seed thread meta projection")
            .len(),
        1
    );

    let router = build_router(state.clone());
    let request = authed_request()
        .method("POST")
        .uri(format!("/api/threads/{thread_id}/archive"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "endpointKeys": ["api::main::loop"]
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        payload["detached_endpoint_keys"],
        json!(["api::main::loop", "telegram::main::1000000001"])
    );

    assert!(state.threads.thread_store.get(thread_id).await.is_none());
    assert!(
        state
            .ops
            .garyx_db
            .list_recent_threads(10, 0)
            .expect("recent projection after archive")
            .is_empty()
    );
    assert!(
        state
            .ops
            .garyx_db
            .list_thread_meta()
            .expect("thread meta after archive")
            .is_empty()
    );
    assert!(
        state
            .ops
            .garyx_db
            .list_pinned_threads()
            .expect("pins after archive")
            .is_empty()
    );

    let reconnected_thread_id = {
        let mut router = state.threads.router.lock().await;
        router
            .resolve_or_create_inbound_thread("telegram", "main", "1000000001", &HashMap::new())
            .await
    };
    assert_ne!(reconnected_thread_id, thread_id);
    assert!(state.threads.thread_store.get(thread_id).await.is_none());
    assert!(
        state
            .ops
            .garyx_db
            .list_recent_threads(10, 0)
            .expect("recent projection after reconnect")
            .iter()
            .all(|record| record.thread_id != thread_id)
    );
}

#[tokio::test]
async fn archive_thread_rejects_active_run_without_deleting() {
    let state = AppStateBuilder::new(test_config()).build();
    let thread_id = "thread::archive-active-run";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Active Archive",
                "created_at": "2026-06-21T08:00:00.000Z",
                "updated_at": "2026-06-21T08:01:00.000Z",
                "messages": []
            }),
        )
        .await;
    append_dangling_run_start(&state, thread_id, "run::archive-active").await;

    let router = build_router(state.clone());
    let request = authed_request()
        .method("POST")
        .uri(format!("/api/threads/{thread_id}/archive"))
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(state.threads.thread_store.get(thread_id).await.is_some());
    assert!(
        !state
            .ops
            .garyx_db
            .is_thread_archived(thread_id)
            .expect("archive tombstone check")
    );
}

#[tokio::test]
async fn archive_thread_rejects_automation_thread_id_without_deleting() {
    let temp = tempdir().unwrap();
    let cron = Arc::new(CronService::new(temp.path().to_path_buf()));
    let thread_id = "thread::archive-automation-target";
    cron.add(CronJobConfig {
        id: "automation::archive-target".to_owned(),
        kind: CronJobKind::AutomationPrompt,
        label: Some("Archive Target".to_owned()),
        schedule: CronSchedule::Interval {
            interval_secs: 3600,
        },
        ui_schedule: None,
        action: CronAction::AgentTurn,
        target: None,
        message: Some("Summarize the thread.".to_owned()),
        workspace_dir: None,
        agent_id: Some("claude".to_owned()),
        thread_id: Some(thread_id.to_owned()),
        delete_after_run: false,
        enabled: true,
        system: false,
    })
    .await
    .expect("add automation target");
    let state = AppStateBuilder::new(test_config())
        .with_cron_service(cron)
        .build();
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Automation Target",
                "created_at": "2026-06-21T08:00:00.000Z",
                "updated_at": "2026-06-21T08:01:00.000Z",
                "messages": []
            }),
        )
        .await;

    let router = build_router(state.clone());
    let request = authed_request()
        .method("POST")
        .uri(format!("/api/threads/{thread_id}/archive"))
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(state.threads.thread_store.get(thread_id).await.is_some());
    assert!(
        !state
            .ops
            .garyx_db
            .is_thread_archived(thread_id)
            .expect("archive tombstone check")
    );
}

#[tokio::test]
async fn archive_thread_rejects_automation_target_reference_without_deleting() {
    let temp = tempdir().unwrap();
    let cron = Arc::new(CronService::new(temp.path().to_path_buf()));
    let thread_id = "thread::archive-automation-target-ref";
    cron.add(CronJobConfig {
        id: "automation::archive-target-ref".to_owned(),
        kind: CronJobKind::AutomationPrompt,
        label: Some("Archive Target Ref".to_owned()),
        schedule: CronSchedule::Interval {
            interval_secs: 3600,
        },
        ui_schedule: None,
        action: CronAction::AgentTurn,
        target: Some(format!("thread:{thread_id}")),
        message: Some("Summarize the thread.".to_owned()),
        workspace_dir: None,
        agent_id: Some("claude".to_owned()),
        thread_id: None,
        delete_after_run: false,
        enabled: true,
        system: false,
    })
    .await
    .expect("add automation target reference");
    let state = AppStateBuilder::new(test_config())
        .with_cron_service(cron)
        .build();
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Automation Target Ref",
                "created_at": "2026-06-21T08:00:00.000Z",
                "updated_at": "2026-06-21T08:01:00.000Z",
                "messages": []
            }),
        )
        .await;

    let router = build_router(state.clone());
    let request = authed_request()
        .method("POST")
        .uri(format!("/api/threads/{thread_id}/archive"))
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(state.threads.thread_store.get(thread_id).await.is_some());
    assert!(
        !state
            .ops
            .garyx_db
            .is_thread_archived(thread_id)
            .expect("archive tombstone check")
    );
}

#[tokio::test]
async fn archived_thread_tombstone_blocks_projection_rewrite() {
    let state = AppStateBuilder::new(test_config()).build();
    let thread_id = "thread::archived-projection-rewrite";
    state
        .ops
        .garyx_db
        .mark_thread_archived(thread_id)
        .expect("mark thread archived");

    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Should Not Return",
                "created_at": "2026-06-21T08:00:00.000Z",
                "updated_at": "2026-06-21T08:01:00.000Z",
                "messages": [{"role": "user", "content": "hello ws"}]
            }),
        )
        .await;

    assert!(state.threads.thread_store.get(thread_id).await.is_none());
    assert!(
        state
            .ops
            .garyx_db
            .list_recent_threads(10, 0)
            .expect("recent projection")
            .is_empty()
    );
}

#[tokio::test]
async fn chat_start_rejects_archived_thread_id() {
    let state = AppStateBuilder::new(test_config()).build();
    let thread_id = "thread::archived-chat-start";
    state
        .ops
        .garyx_db
        .mark_thread_archived(thread_id)
        .expect("mark thread archived");

    let router = build_router(state);
    let request = authed_request()
        .method("POST")
        .uri("/api/chat/start")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "threadId": thread_id,
                "message": "reconnect proof",
                "waitForResponse": false
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::GONE);
}

#[tokio::test]
async fn delete_thread_allows_disabled_channel_binding() {
    let mut config = test_config();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(&TelegramAccount {
                token: "token-main".to_owned(),
                enabled: false,
                name: None,
                agent_id: "claude".to_owned(),
                workspace_dir: None,
                owner_target: None,
                groups: std::collections::HashMap::new(),
            }),
        );

    let state = AppStateBuilder::new(config).build();
    let thread_id = "thread::delete-bound-disabled";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Bound Disabled",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "binding_key": "u1",
                    "chat_id": "u1",
                    "delivery_target_type": DELIVERY_TARGET_TYPE_CHAT_ID,
                    "delivery_target_id": "u1",
                    "display_label": "u1"
                }]
            }),
        )
        .await;

    let router = build_router(state.clone());
    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/threads/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(state.threads.thread_store.get(thread_id).await.is_none());
}

#[tokio::test]
async fn delete_thread_allows_orphan_channel_binding() {
    let state = AppStateBuilder::new(test_config()).build();
    let thread_id = "thread::delete-bound-orphan";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Bound Orphan",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "binding_key": "u1",
                    "chat_id": "u1",
                    "delivery_target_type": DELIVERY_TARGET_TYPE_CHAT_ID,
                    "delivery_target_id": "u1",
                    "display_label": "u1"
                }]
            }),
        )
        .await;

    let router = build_router(state.clone());
    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/threads/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(state.threads.thread_store.get(thread_id).await.is_none());
}

#[tokio::test]
async fn delete_thread_aborts_active_run_and_prevents_recreation() {
    let mut config = test_config();
    config.channels.api.accounts.insert(
        "main".to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            workspace_mode: None,
        },
    );

    let provider = Arc::new(SlowDeleteProvider::new(250));
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("api-test-provider", provider.clone())
        .await;
    bridge.set_route("api", "main", "api-test-provider").await;
    bridge.set_default_provider_key("api-test-provider").await;

    let state = AppStateBuilder::new(config)
        .with_bridge(bridge.clone())
        .build();
    bridge.set_event_tx(state.ops.events.sender()).await;
    bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;

    let (thread_id, _) = create_thread_record(
        &state.threads.thread_store,
        ThreadEnsureOptions {
            label: Some("Delete Active".to_owned()),
            workspace_dir: None,
            workspace_mode: Default::default(),
            worktree_base_dir: None,
            agent_id: None,
            metadata: HashMap::new(),
            provider_type: None,
            sdk_session_id: None,
            thread_kind: None,
            origin_channel: None,
            origin_account_id: None,
            origin_from_id: None,
            is_group: None,
        },
    )
    .await
    .unwrap();

    bridge
        .start_agent_run(
            garyx_models::provider::AgentRunRequest::new(
                &thread_id,
                "delete me",
                "run-delete-session",
                "api",
                "main",
                HashMap::new(),
            ),
            None,
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(bridge.is_run_active("run-delete-session").await);

    let router = build_router(state.clone());
    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/threads/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
    assert!(!bridge.is_run_active("run-delete-session").await);
    assert!(state.threads.thread_store.get(&thread_id).await.is_none());
    assert_eq!(provider.cleared_sessions(), vec![thread_id]);
}

#[tokio::test]
async fn delete_thread_drops_local_state_even_when_provider_clear_fails() {
    let mut config = test_config();
    config.channels.api.accounts.insert(
        "main".to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            workspace_mode: None,
        },
    );

    let failing_provider = Arc::new(SlowDeleteProvider::with_clear_result(0, false));
    let default_provider = Arc::new(SlowDeleteProvider::with_clear_result(0, true));
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("api-test-provider", failing_provider.clone())
        .await;
    bridge
        .register_provider("api-default-provider", default_provider)
        .await;
    bridge
        .set_route("api", "main", "api-default-provider")
        .await;
    bridge
        .set_default_provider_key("api-default-provider")
        .await;

    let state = AppStateBuilder::new(config)
        .with_bridge(bridge.clone())
        .build();
    bridge.set_event_tx(state.ops.events.sender()).await;
    bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;

    let (thread_id, _) = create_thread_record(
        &state.threads.thread_store,
        ThreadEnsureOptions {
            label: Some("Delete Local State".to_owned()),
            workspace_dir: None,
            workspace_mode: Default::default(),
            worktree_base_dir: None,
            agent_id: None,
            metadata: HashMap::new(),
            provider_type: None,
            sdk_session_id: None,
            thread_kind: None,
            origin_channel: None,
            origin_account_id: None,
            origin_from_id: None,
            is_group: None,
        },
    )
    .await
    .unwrap();

    bridge
        .set_thread_affinity(&thread_id, "api-test-provider")
        .await;
    bridge
        .set_thread_workspace_binding(&thread_id, Some("/tmp/delete-thread".to_owned()))
        .await;

    let router = build_router(state.clone());
    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/threads/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    assert!(state.threads.thread_store.get(&thread_id).await.is_none());
    assert_eq!(failing_provider.cleared_sessions(), vec![thread_id.clone()]);
    assert_eq!(
        bridge
            .resolve_provider_for_thread(&thread_id, "api", "main")
            .await,
        Some("api-default-provider".to_owned())
    );
    assert!(
        !bridge
            .thread_workspace_bindings_snapshot()
            .await
            .contains_key(&thread_id)
    );
}

#[tokio::test]
async fn delete_thread_clears_in_memory_reply_routing() {
    let (state, _logger, _dir) = test_state().await;
    let thread_id = "thread::reply-delete";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            serde_json::json!({
                "thread_id": thread_id,
                "thread_id": thread_id,
                "label": "Reply Delete",
                "outbound_message_ids": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "chat_id": "42",
                    "message_id": "msg-delete-1"
                }]
            }),
        )
        .await;
    {
        let mut router = state.threads.router.lock().await;
        router
            .message_routing_index_mut()
            .rebuild_from_store(state.threads.thread_store.as_ref(), "telegram")
            .await;
        assert_eq!(
            router.resolve_reply_thread_for_chat(
                "telegram",
                "main",
                Some("42"),
                None,
                "msg-delete-1",
            ),
            Some(thread_id)
        );
    }

    let router = build_router(state.clone());
    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/threads/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let router = state.threads.router.lock().await;
    assert_eq!(
        router.resolve_reply_thread_for_chat("telegram", "main", Some("42"), None, "msg-delete-1",),
        None
    );
}

#[tokio::test]
async fn delete_thread_clears_in_memory_last_delivery() {
    let (state, _logger, _dir) = test_state().await;
    let thread_id = "thread::delivery-delete";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            serde_json::json!({
                "thread_id": thread_id,
                "thread_id": thread_id,
                "label": "Delivery Delete"
            }),
        )
        .await;
    {
        let mut router = state.threads.router.lock().await;
        router.set_last_delivery(
            thread_id,
            garyx_models::routing::DeliveryContext {
                channel: "telegram".to_owned(),
                account_id: "main".to_owned(),
                chat_id: "42".to_owned(),
                user_id: "42".to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: "42".to_owned(),
                thread_id: None,
                metadata: Default::default(),
            },
        );
        assert!(router.get_last_delivery(thread_id).is_some());
    }

    let router = build_router(state.clone());
    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/threads/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let router = state.threads.router.lock().await;
    assert!(router.get_last_delivery(thread_id).is_none());
    assert!(
        router
            .resolve_delivery_target(&format!("thread:{thread_id}"))
            .is_none()
    );
}

#[tokio::test]
async fn delete_thread_clears_switched_thread_references() {
    let (state, _logger, _dir) = test_state().await;
    let thread_id = "thread::switch-delete";
    state
        .threads
        .thread_store
        .set(
            "thread::older",
            serde_json::json!({
                "thread_id": "thread::older",
                "thread_id": "thread::older",
                "label": "Older"
            }),
        )
        .await;
    state
        .threads
        .thread_store
        .set(
            thread_id,
            serde_json::json!({
                "thread_id": thread_id,
                "thread_id": thread_id,
                "label": "Switch Delete"
            }),
        )
        .await;
    {
        let mut router = state.threads.router.lock().await;
        let user_key = MessageRouter::build_account_user_key("telegram", "main", "u1", false, None);
        router.switch_to_thread(&user_key, "thread::older");
        router.switch_to_thread(&user_key, thread_id);
        assert_eq!(
            router.get_current_thread_id_for_account("telegram", "main", "u1", false, None),
            Some(thread_id)
        );
    }

    let router = build_router(state.clone());
    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/threads/{thread_id}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let router = state.threads.router.lock().await;
    assert_eq!(
        router.get_current_thread_id_for_account("telegram", "main", "u1", false, None),
        Some("thread::older")
    );
}

#[tokio::test]
async fn configured_bots_route_returns_only_account_workspace_bindings() {
    let mut config = test_config();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "bound".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(
                &garyx_models::config::TelegramAccount {
                    token: "token-a".to_owned(),
                    enabled: true,
                    name: None,
                    agent_id: "claude".to_owned(),
                    workspace_dir: Some("/tmp/bound-workspace".to_owned()),
                    owner_target: None,
                    groups: std::collections::HashMap::new(),
                },
            ),
        );
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "unbound".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(
                &garyx_models::config::TelegramAccount {
                    token: "token-b".to_owned(),
                    enabled: true,
                    name: None,
                    agent_id: "claude".to_owned(),
                    workspace_dir: None,
                    owner_target: None,
                    groups: std::collections::HashMap::new(),
                },
            ),
        );
    // Generic plugin-owned subprocess channel — same `bots` route
    // must surface entries from `channels.plugins[id].accounts`.
    let mut plugin_cfg = garyx_models::config::PluginChannelConfig::default();
    plugin_cfg.accounts.insert(
        "main".to_owned(),
        garyx_models::config::PluginAccountEntry {
            enabled: true,
            name: None,
            agent_id: Some("claude".to_owned()),
            workspace_dir: Some("/tmp/plugin-workspace".to_owned()),
            workspace_mode: None,
            config: serde_json::json!({
                "token": "plugin_agent_test",
                "base_url": "https://example.com",
            }),
        },
    );
    config
        .channels
        .plugins
        .insert("sample_plugin".to_owned(), plugin_cfg);

    let log_dir = tempdir().unwrap();
    let logger = Arc::new(ThreadFileLogger::new(log_dir.path()));
    let state = AppStateBuilder::new(config)
        .with_thread_log_sink(logger)
        .build();
    let router = build_router(state);

    let request = authed_request()
        .uri("/api/configured-bots")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let bots = payload["bots"].as_array().unwrap();

    let bound = bots
        .iter()
        .find(|entry| entry["account_id"] == "bound")
        .unwrap();
    let unbound = bots
        .iter()
        .find(|entry| entry["account_id"] == "unbound")
        .unwrap();
    let plugin_bot = bots
        .iter()
        .find(|entry| entry["channel"] == "sample_plugin" && entry["account_id"] == "main")
        .unwrap();

    assert_eq!(bound["workspace_dir"], "/tmp/bound-workspace");
    assert!(unbound["workspace_dir"].is_null());
    assert_eq!(plugin_bot["workspace_dir"], "/tmp/plugin-workspace");
    assert_eq!(bound["main_endpoint_status"], "unresolved");
    assert_eq!(unbound["main_endpoint_status"], "unresolved");
    assert_eq!(plugin_bot["main_endpoint_status"], "unresolved");
    assert!(bound["default_open_endpoint"].is_null());
    assert!(plugin_bot["default_open_endpoint"].is_null());
}

#[tokio::test]
async fn cached_channel_endpoints_reuses_snapshot_until_invalidated() {
    let log_dir = tempdir().unwrap();
    let logger = Arc::new(ThreadFileLogger::new(log_dir.path()));
    let state = AppStateBuilder::new(test_config())
        .with_thread_log_sink(logger)
        .build();

    state
        .threads
        .thread_store
        .set(
            "thread::cached-endpoint",
            serde_json::json!({
                "thread_id": "thread::cached-endpoint",
                "label": "Cached Endpoint",
                "updated_at": "2026-03-16T01:00:00Z",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "binding_key": "chat-1",
                    "chat_id": "chat-1",
                    "display_label": "Initial Chat",
                    "last_inbound_at": "2026-03-16T01:00:00Z"
                }]
            }),
        )
        .await;

    let initial = state.cached_channel_endpoints().await;
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0].display_label, "Initial Chat");

    state
        .threads
        .thread_store
        .set(
            "thread::cached-endpoint",
            serde_json::json!({
                "thread_id": "thread::cached-endpoint",
                "label": "Cached Endpoint",
                "updated_at": "2026-03-16T01:00:01Z",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "binding_key": "chat-1",
                    "chat_id": "chat-1",
                    "display_label": "Updated Chat",
                    "last_inbound_at": "2026-03-16T01:00:01Z"
                }]
            }),
        )
        .await;

    let cached = state.cached_channel_endpoints().await;
    assert_eq!(cached[0].display_label, "Initial Chat");

    state.invalidate_channel_endpoint_cache().await;
    let refreshed = state.cached_channel_endpoints().await;
    assert_eq!(refreshed[0].display_label, "Updated Chat");
}

#[tokio::test]
async fn cached_thread_list_entries_reuses_snapshot_until_invalidated() {
    let log_dir = tempdir().unwrap();
    let logger = Arc::new(ThreadFileLogger::new(log_dir.path()));
    let state = AppStateBuilder::new(test_config())
        .with_thread_log_sink(logger)
        .build();

    state
        .threads
        .thread_store
        .set(
            "thread::cached-list",
            serde_json::json!({
                "thread_id": "thread::cached-list",
                "label": "Initial Thread",
                "updated_at": "2026-03-16T01:00:00Z"
            }),
        )
        .await;

    let initial = state.cached_thread_list_entries().await;
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0].data["label"], "Initial Thread");

    state
        .threads
        .thread_store
        .set(
            "thread::cached-list",
            serde_json::json!({
                "thread_id": "thread::cached-list",
                "label": "Updated Thread",
                "updated_at": "2026-03-16T01:00:01Z"
            }),
        )
        .await;

    let cached = state.cached_thread_list_entries().await;
    assert_eq!(cached[0].data["label"], "Initial Thread");

    state.invalidate_thread_list_cache().await;
    let refreshed = state.cached_thread_list_entries().await;
    assert_eq!(refreshed[0].data["label"], "Updated Thread");
}

#[tokio::test]
async fn configured_bots_route_exposes_resolved_main_endpoints() {
    let mut config = test_config();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "telegram_owner".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(
                &garyx_models::config::TelegramAccount {
                    token: "token-telegram".to_owned(),
                    enabled: true,
                    name: Some("Telegram Owner".to_owned()),
                    agent_id: "claude".to_owned(),
                    workspace_dir: Some("/tmp/telegram-owner".to_owned()),
                    owner_target: Some(garyx_models::config::OwnerTargetConfig {
                        target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_owned(),
                        target_id: "1000000001".to_owned(),
                    }),
                    groups: std::collections::HashMap::new(),
                },
            ),
        );
    config
        .channels
        .plugin_channel_mut("feishu")
        .accounts
        .insert(
            "feishu_owner".to_owned(),
            garyx_models::config::feishu_account_to_plugin_entry(
                &garyx_models::config::FeishuAccount {
                    app_id: "cli_test_app".to_owned(),
                    app_secret: "cli_test_secret".to_owned(),
                    enabled: true,
                    domain: garyx_models::config::FeishuDomain::Feishu,
                    name: Some("Feishu Owner".to_owned()),
                    agent_id: "claude".to_owned(),
                    workspace_dir: Some("/tmp/feishu-owner".to_owned()),
                    owner_target: Some(garyx_models::config::OwnerTargetConfig {
                        target_type: DELIVERY_TARGET_TYPE_OPEN_ID.to_owned(),
                        target_id: "ou_owner_123".to_owned(),
                    }),
                    require_mention: true,
                    topic_session_mode: garyx_models::config::TopicSessionMode::Disabled,
                },
            ),
        );
    config
        .channels
        .plugin_channel_mut("weixin")
        .accounts
        .insert(
            "wechat_owner".to_owned(),
            garyx_models::config::weixin_account_to_plugin_entry(
                &garyx_models::config::WeixinAccount {
                    token: "token-wechat".to_owned(),
                    uin: String::new(),
                    enabled: true,
                    base_url: "https://ilinkai.weixin.qq.com".to_owned(),
                    name: Some("Wechat".to_owned()),
                    agent_id: "claude".to_owned(),
                    workspace_dir: Some("/tmp/wechat-owner".to_owned()),
                    streaming_update: true,
                },
            ),
        );
    let mut sample_plugin = garyx_models::config::PluginChannelConfig::default();
    sample_plugin.accounts.insert(
        "plugin_owner".to_owned(),
        garyx_models::config::PluginAccountEntry {
            enabled: true,
            name: None,
            agent_id: Some("claude".to_owned()),
            workspace_dir: Some("/tmp/plugin-owner".to_owned()),
            workspace_mode: None,
            config: serde_json::json!({
                "token": "plugin_agent_owner",
                "base_url": "https://plugin.example.com",
            }),
        },
    );
    config
        .channels
        .plugins
        .insert("sample_plugin".to_owned(), sample_plugin);

    let log_dir = tempdir().unwrap();
    let logger = Arc::new(ThreadFileLogger::new(log_dir.path()));
    let state = AppStateBuilder::new(config)
        .with_thread_log_sink(logger)
        .build();
    let router = build_router(state);

    let request = authed_request()
        .uri("/api/configured-bots?include_endpoints=true")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let bots = payload["bots"].as_array().unwrap();

    let telegram_bot = bots
        .iter()
        .find(|entry| entry["channel"] == "telegram" && entry["account_id"] == "telegram_owner")
        .unwrap();
    assert_eq!(telegram_bot["main_endpoint_status"], "resolved");
    assert_eq!(telegram_bot["display_name"], "Telegram Owner");
    assert_eq!(telegram_bot["main_endpoint"]["source"], "owner_target");
    assert_eq!(
        telegram_bot["default_open_endpoint"]["delivery_target_id"],
        "1000000001"
    );
    assert_eq!(
        telegram_bot["main_endpoint"]["delivery_target_type"],
        DELIVERY_TARGET_TYPE_CHAT_ID
    );
    assert_eq!(
        telegram_bot["main_endpoint"]["delivery_target_id"],
        "1000000001"
    );
    assert_eq!(
        telegram_bot["main_endpoint"]["workspace_dir"],
        "/tmp/telegram-owner"
    );

    let feishu_bot = bots
        .iter()
        .find(|entry| entry["channel"] == "feishu" && entry["account_id"] == "feishu_owner")
        .unwrap();
    assert_eq!(feishu_bot["main_endpoint_status"], "resolved");
    assert_eq!(feishu_bot["display_name"], "Feishu Owner");
    assert_eq!(feishu_bot["main_endpoint"]["source"], "owner_target");
    assert_eq!(
        feishu_bot["main_endpoint"]["delivery_target_type"],
        DELIVERY_TARGET_TYPE_OPEN_ID
    );
    assert_eq!(
        feishu_bot["main_endpoint"]["delivery_target_id"],
        "ou_owner_123"
    );
    assert_eq!(
        feishu_bot["main_endpoint"]["workspace_dir"],
        "/tmp/feishu-owner"
    );
    assert_eq!(
        feishu_bot["default_open_endpoint"]["delivery_target_id"],
        "ou_owner_123"
    );

    let weixin_bot = bots
        .iter()
        .find(|entry| entry["channel"] == "weixin" && entry["account_id"] == "wechat_owner")
        .unwrap();
    assert_eq!(weixin_bot["display_name"], "Wechat");
    assert_eq!(weixin_bot["workspace_dir"], "/tmp/wechat-owner");
    assert_eq!(weixin_bot["main_endpoint_status"], "unresolved");
    assert!(weixin_bot["default_open_endpoint"].is_null());

    let plugin_bot = bots
        .iter()
        .find(|entry| entry["channel"] == "sample_plugin" && entry["account_id"] == "plugin_owner")
        .unwrap();
    assert_eq!(plugin_bot["display_name"], "plugin_owner");
    assert_eq!(plugin_bot["workspace_dir"], "/tmp/plugin-owner");
    assert_eq!(plugin_bot["main_endpoint_status"], "unresolved");
    assert!(plugin_bot["default_open_endpoint"].is_null());
}

#[tokio::test]
async fn configured_bots_route_resolves_legacy_telegram_private_endpoint_without_valid_agent() {
    let mut config = test_config();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "legacy".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(
                &garyx_models::config::TelegramAccount {
                    token: "token-telegram-legacy".to_owned(),
                    enabled: true,
                    name: Some("Legacy Telegram".to_owned()),
                    agent_id: "missing-agent".to_owned(),
                    workspace_dir: Some("/tmp/telegram-legacy".to_owned()),
                    owner_target: None,
                    groups: std::collections::HashMap::new(),
                },
            ),
        );

    let log_dir = tempdir().unwrap();
    let logger = Arc::new(ThreadFileLogger::new(log_dir.path()));
    let state = AppStateBuilder::new(config)
        .with_thread_log_sink(logger)
        .build();
    state
        .threads
        .thread_store
        .set(
            "thread::telegram-legacy",
            serde_json::json!({
                "thread_id": "thread::telegram-legacy",
                "label": "Legacy Telegram",
                "workspace_dir": "/tmp/telegram-legacy",
                "updated_at": "2026-03-16T01:00:00Z",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "legacy",
                    "binding_key": "1000000001",
                    "chat_id": "",
                    "delivery_target_type": "chat_id",
                    "delivery_target_id": "",
                    "display_label": "Test User",
                    "last_inbound_at": "2026-03-16T01:00:00Z"
                }]
            }),
        )
        .await;

    let router = build_router(state);
    let request = authed_request()
        .uri("/api/configured-bots?include_endpoints=true")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let bots = payload["bots"].as_array().unwrap();
    let bot = bots
        .iter()
        .find(|entry| entry["channel"] == "telegram" && entry["account_id"] == "legacy")
        .unwrap();

    assert_eq!(bot["agent_id"], "missing-agent");
    assert_eq!(bot["main_endpoint_status"], "resolved");
    assert_eq!(bot["main_endpoint"]["thread_id"], "thread::telegram-legacy");
    assert_eq!(bot["main_endpoint"]["chat_id"], "1000000001");
    assert_eq!(bot["main_endpoint"]["delivery_target_type"], "chat_id");
    assert_eq!(bot["main_endpoint"]["delivery_target_id"], "1000000001");
    assert!(bot["main_endpoint"]["delivery_thread_id"].is_null());
}

#[tokio::test]
async fn bot_consoles_route_aggregates_configured_bots_and_endpoints() {
    let mut config = test_config();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "main".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(
                &garyx_models::config::TelegramAccount {
                    token: "token-main".to_owned(),
                    enabled: true,
                    name: Some("Main Bot".to_owned()),
                    agent_id: "claude".to_owned(),
                    workspace_dir: Some("/tmp/main-workspace".to_owned()),
                    owner_target: None,
                    groups: std::collections::HashMap::new(),
                },
            ),
        );

    let log_dir = tempdir().unwrap();
    let logger = Arc::new(ThreadFileLogger::new(log_dir.path()));
    let state = AppStateBuilder::new(config)
        .with_thread_log_sink(logger)
        .build();

    state
        .threads
        .thread_store
        .set(
            "thread::support",
            serde_json::json!({
                "thread_id": "thread::support",
                "label": "Support",
                "workspace_dir": "/tmp/main-workspace",
                "updated_at": "2026-03-16T01:00:00Z",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "main",
                    "peer_id": "alice",
                    "chat_id": "alice",
                    "display_label": "Alice",
                    "last_inbound_at": "2026-03-16T01:00:00Z"
                }]
            }),
        )
        .await;

    let router = build_router(state);
    let request = authed_request()
        .uri("/api/bot-consoles")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let bots = payload["bots"].as_array().unwrap();
    let main = bots
        .iter()
        .find(|entry| entry["id"] == "telegram::main")
        .unwrap();

    assert_eq!(main["display_name"], "Main Bot");
    assert_eq!(main["workspace_dir"], "/tmp/main-workspace");
    assert_eq!(main["status"], "connected");
    assert_eq!(main["main_endpoint_status"], "resolved");
    assert_eq!(main["main_endpoint"]["thread_id"], "thread::support");
    assert_eq!(main["main_endpoint"]["delivery_target_type"], "chat_id");
    assert_eq!(main["main_endpoint"]["delivery_target_id"], "alice");
    assert_eq!(main["default_open_thread_id"], "thread::support");
    assert_eq!(main["endpoint_count"], 1);
    assert_eq!(main["bound_endpoint_count"], 1);
    assert_eq!(main["endpoints"][0]["thread_id"], "thread::support");
    assert_eq!(main["conversation_nodes"].as_array().unwrap().len(), 1);
    assert_eq!(
        main["conversation_nodes"][0]["endpoint"]["thread_id"],
        "thread::support"
    );
}

#[tokio::test]
async fn bot_consoles_route_preserves_plugin_main_endpoint_resolution() {
    let mut config = test_config();
    config
        .channels
        .plugin_channel_mut("telegram")
        .accounts
        .insert(
            "owner".to_owned(),
            garyx_models::config::telegram_account_to_plugin_entry(
                &garyx_models::config::TelegramAccount {
                    token: "${TOKEN}".to_owned(),
                    enabled: true,
                    name: Some("Owner Bot".to_owned()),
                    agent_id: "claude".to_owned(),
                    workspace_dir: Some("/tmp/owner-workspace".to_owned()),
                    owner_target: Some(garyx_models::config::OwnerTargetConfig {
                        target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_owned(),
                        target_id: "1000000001".to_owned(),
                    }),
                    groups: std::collections::HashMap::new(),
                },
            ),
        );

    let log_dir = tempdir().unwrap();
    let logger = Arc::new(ThreadFileLogger::new(log_dir.path()));
    let state = AppStateBuilder::new(config)
        .with_thread_log_sink(logger)
        .build();

    state
        .threads
        .thread_store
        .set(
            "thread::owner-group",
            serde_json::json!({
                "thread_id": "thread::owner-group",
                "label": "Owner Group",
                "workspace_dir": "/tmp/owner-workspace",
                "updated_at": "2026-03-16T03:00:00Z",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "owner",
                    "binding_key": "group-1",
                    "chat_id": "group-1",
                    "delivery_target_type": "chat_id",
                    "delivery_target_id": "group-1",
                    "display_label": "Owner Group",
                    "last_inbound_at": "2026-03-16T03:00:00Z"
                }]
            }),
        )
        .await;

    let router = build_router(state);
    let request = authed_request()
        .uri("/api/bot-consoles")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let owner = payload["bots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "telegram::owner")
        .unwrap();

    assert_eq!(owner["main_endpoint_status"], "resolved");
    assert_eq!(owner["main_endpoint"]["source"], "owner_target");
    assert_eq!(owner["main_endpoint"]["delivery_target_id"], "1000000001");
    assert_eq!(
        owner["default_open_endpoint"]["delivery_target_id"],
        "1000000001"
    );
    assert_eq!(owner["conversation_nodes"].as_array().unwrap().len(), 1);
    assert_eq!(
        owner["conversation_nodes"][0]["endpoint"]["thread_id"],
        "thread::owner-group"
    );
}

#[tokio::test]
async fn bot_consoles_route_uses_configured_bot_order_not_activity_order() {
    let mut config = test_config();
    for (account_id, name) in [("alpha", "Alpha Bot"), ("beta", "Beta Bot")] {
        config
            .channels
            .plugin_channel_mut("telegram")
            .accounts
            .insert(
                account_id.to_owned(),
                garyx_models::config::telegram_account_to_plugin_entry(
                    &garyx_models::config::TelegramAccount {
                        token: format!("token-{account_id}"),
                        enabled: true,
                        name: Some(name.to_owned()),
                        agent_id: "claude".to_owned(),
                        workspace_dir: None,
                        owner_target: None,
                        groups: std::collections::HashMap::new(),
                    },
                ),
            );
    }

    let log_dir = tempdir().unwrap();
    let logger = Arc::new(ThreadFileLogger::new(log_dir.path()));
    let state = AppStateBuilder::new(config)
        .with_thread_log_sink(logger)
        .build();

    state
        .threads
        .thread_store
        .set(
            "thread::alpha-z-room",
            serde_json::json!({
                "thread_id": "thread::alpha-z-room",
                "label": "Alpha Z Room",
                "updated_at": "2026-03-16T02:00:00Z",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "alpha",
                    "binding_key": "alpha-z",
                    "chat_id": "alpha-z",
                    "display_label": "Z Room",
                    "last_inbound_at": "2026-03-16T02:00:00Z"
                }]
            }),
        )
        .await;
    state
        .threads
        .thread_store
        .set(
            "thread::alpha-a-room",
            serde_json::json!({
                "thread_id": "thread::alpha-a-room",
                "label": "Alpha A Room",
                "updated_at": "2026-03-16T01:00:00Z",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "alpha",
                    "binding_key": "alpha-a",
                    "chat_id": "alpha-a",
                    "display_label": "A Room",
                    "last_inbound_at": "2026-03-16T01:00:00Z"
                }]
            }),
        )
        .await;
    state
        .threads
        .thread_store
        .set(
            "thread::beta-latest",
            serde_json::json!({
                "thread_id": "thread::beta-latest",
                "label": "Beta Latest",
                "updated_at": "2026-03-16T03:00:00Z",
                "channel_bindings": [{
                    "channel": "telegram",
                    "account_id": "beta",
                    "binding_key": "beta-latest",
                    "chat_id": "beta-latest",
                    "display_label": "Beta Room",
                    "last_inbound_at": "2026-03-16T03:00:00Z"
                }]
            }),
        )
        .await;

    let router = build_router(state);
    let request = authed_request()
        .uri("/api/bot-consoles?include_endpoints=true")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let bots = payload["bots"].as_array().unwrap();
    let account_ids: Vec<_> = bots
        .iter()
        .filter(|entry| entry["channel"] == "telegram")
        .map(|entry| entry["account_id"].as_str().unwrap())
        .collect();

    assert_eq!(account_ids, vec!["alpha", "beta"]);
    let alpha = bots
        .iter()
        .find(|entry| entry["channel"] == "telegram" && entry["account_id"] == "alpha")
        .unwrap();
    let alpha_endpoint_labels: Vec<_> = alpha["endpoints"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["display_label"].as_str().unwrap())
        .collect();
    assert_eq!(alpha_endpoint_labels, vec!["A Room", "Z Room"]);
}

#[tokio::test]
async fn bot_consoles_route_ignores_unconfigured_endpoint_accounts() {
    let config = test_config();
    let log_dir = tempdir().unwrap();
    let logger = Arc::new(ThreadFileLogger::new(log_dir.path()));
    let state = AppStateBuilder::new(config)
        .with_thread_log_sink(logger)
        .build();

    state
        .threads
        .thread_store
        .set(
            "thread::api-smoke",
            serde_json::json!({
                "thread_id": "thread::api-smoke",
                "label": "api/main/e2e-image-smoke",
                "workspace_dir": "/tmp/api-smoke",
                "updated_at": "2026-03-16T01:00:00Z",
                "channel_bindings": [{
                    "channel": "api",
                    "account_id": "main",
                    "peer_id": "e2e-image-smoke",
                    "chat_id": "e2e-image-smoke",
                    "display_label": "api/main/e2e-image-smoke",
                    "last_inbound_at": "2026-03-16T01:00:00Z"
                }]
            }),
        )
        .await;

    let router = build_router(state);
    let request = authed_request()
        .uri("/api/bot-consoles?include_endpoints=true")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let bots = payload["bots"].as_array().unwrap();

    assert!(bots.iter().all(|entry| entry["id"] != "api::main"));
}

// ---------------------------------------------------------------------
// Team block in thread metadata response.
// ---------------------------------------------------------------------

async fn seed_product_ship_team(state: &Arc<AppState>) {
    use crate::agent_teams::UpsertAgentTeamRequest;
    state
        .ops
        .agent_teams
        .upsert_team(UpsertAgentTeamRequest {
            team_id: "product-ship".to_owned(),
            display_name: "Product Ship".to_owned(),
            leader_agent_id: "planner".to_owned(),
            member_agent_ids: vec![
                "planner".to_owned(),
                "coder".to_owned(),
                "reviewer".to_owned(),
            ],
            workflow_text: "Ship the product.".to_owned(),
            avatar_data_url: None,
        })
        .await
        .expect("team upsert");
}

#[tokio::test]
async fn thread_metadata_omits_team_block_for_standalone_agent_thread() {
    let (state, _logger, _dir) = test_state().await;
    let thread_id = "thread::standalone-claude";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "agent_id": "claude",
                "provider_type": "claude_code",
            }),
        )
        .await;

    let data = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("thread data");
    let response = thread_metadata_response(&state, thread_id, &data).await;
    assert!(
        response.get("team").is_none(),
        "standalone-agent thread must not emit `team`, got: {response}"
    );
}

#[tokio::test]
async fn thread_metadata_preserves_workflow_thread_type() {
    let (state, _logger, _dir) = test_state().await;
    let thread_id = "thread::workflow-metadata";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "thread_kind": "workflow_run",
                "workflow_run_id": thread_id,
                "workflow_definition_id": "test-workflow",
            }),
        )
        .await;

    let data = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("thread data");
    let response = thread_metadata_response(&state, thread_id, &data).await;
    assert_eq!(response["thread_type"], "workflow_run");
}

#[tokio::test]
async fn thread_metadata_defaults_missing_thread_kind_to_chat() {
    let (state, _logger, _dir) = test_state().await;
    let thread_id = "cron::legacy-metadata";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "label": "Legacy cron-shaped metadata",
            }),
        )
        .await;

    let data = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("thread data");
    let response = thread_metadata_response(&state, thread_id, &data).await;
    assert_eq!(response["thread_type"], "chat");
}

#[tokio::test]
async fn thread_metadata_emits_empty_child_map_when_group_never_persisted() {
    let (state, _logger, _dir) = test_state().await;
    seed_product_ship_team(&state).await;

    let thread_id = "thread::team-fresh";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "agent_id": "product-ship",
                "provider_type": "agent_team",
            }),
        )
        .await;

    let data = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("thread data");
    let response = thread_metadata_response(&state, thread_id, &data).await;
    let team = response
        .get("team")
        .expect("team-bound thread emits `team`");
    assert_eq!(team["team_id"], "product-ship");
    assert_eq!(team["display_name"], "Product Ship");
    assert_eq!(team["leader_agent_id"], "planner");
    let members = team["member_agent_ids"].as_array().expect("members");
    assert_eq!(members.len(), 3);
    let child_map = team["child_thread_ids"]
        .as_object()
        .expect("child_thread_ids must be an object, not null");
    assert!(
        child_map.is_empty(),
        "fresh team thread has no Group yet, expected {{}} got {:?}",
        child_map
    );
}

#[tokio::test]
async fn thread_metadata_projects_known_child_thread_ids_from_group_store() {
    use garyx_bridge::providers::agent_team::Group;
    let (state, _logger, _dir) = test_state().await;
    seed_product_ship_team(&state).await;

    let thread_id = "thread::team-partial";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "agent_id": "product-ship",
                "provider_type": "agent_team",
            }),
        )
        .await;

    // Seed a Group that has seen `coder` but not `reviewer`.
    let mut group = Group::new(thread_id, "product-ship");
    group.record_child_thread("coder", "th::child-coder-0001");
    group.record_child_thread("ghost", "th::child-ghost-0001");
    state.ops.agent_team_group_store.save(&group).await;

    let data = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("thread data");
    let response = thread_metadata_response(&state, thread_id, &data).await;
    let team = response.get("team").expect("team block present");
    let child_map = team["child_thread_ids"]
        .as_object()
        .expect("child_thread_ids object");
    assert_eq!(
        child_map.get("coder").and_then(Value::as_str),
        Some("th::child-coder-0001")
    );
    assert!(
        !child_map.contains_key("reviewer"),
        "reviewer has no child thread yet, should be absent from the map"
    );
    assert!(
        !child_map.contains_key("ghost"),
        "stale child thread from a removed team member should be filtered out"
    );
}

#[tokio::test]
async fn thread_summary_omits_team_block_for_team_bound_thread() {
    // `/api/threads` summaries stay lightweight. Team metadata is available
    // from the thread detail/history endpoints when a thread is opened.
    use garyx_bridge::providers::agent_team::Group;
    let (state, _logger, _dir) = test_state().await;
    seed_product_ship_team(&state).await;

    let thread_id = "thread::list-team-summary";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "agent_id": "product-ship",
                "provider_type": "agent_team",
            }),
        )
        .await;

    let mut group = Group::new(thread_id, "product-ship");
    group.record_child_thread("coder", "th::child-coder-42");
    state.ops.agent_team_group_store.save(&group).await;

    let data = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("thread data");
    let summary = thread_summary(thread_id, &data);
    assert!(
        summary.get("team").is_none(),
        "list summary must not emit `team`, got: {summary}"
    );
}

#[tokio::test]
async fn thread_summary_omits_team_block_for_standalone_agent_thread() {
    // Inverse of the test above: standalone-agent threads must not be
    // decorated with a phantom `team` block just because the summary
    // pipeline runs in the same function.
    let (state, _logger, _dir) = test_state().await;
    let thread_id = "thread::list-standalone-summary";
    state
        .threads
        .thread_store
        .set(
            thread_id,
            json!({
                "thread_id": thread_id,
                "agent_id": "claude",
                "provider_type": "claude_code",
            }),
        )
        .await;

    let data = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("thread data");
    let summary = thread_summary(thread_id, &data);
    assert!(
        summary.get("team").is_none(),
        "standalone-agent summary must not emit `team`, got: {summary}"
    );
}

#[tokio::test]
async fn task_routes_resolve_percent_encoded_ids() {
    let dir = tempdir().unwrap();
    let mut config = test_config();
    config.tasks.enabled = true;
    config.sessions.data_dir = Some(dir.path().to_string_lossy().to_string());
    let state = AppStateBuilder::new(config).build();
    let router = build_router(state);

    let request = authed_request()
        .method("POST")
        .uri("/api/tasks")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "title": "Check task routing",
                "notification_target": {"kind": "none"}
            }))
            .unwrap(),
        ))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let task_id = payload["task_id"].as_str().unwrap();
    assert!(task_id.starts_with("#TASK-"));

    let request = authed_request()
        .method("GET")
        .uri(format!("/api/tasks/{}", urlencoding::encode(task_id)))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["task_id"], task_id);
    assert_eq!(payload["task"]["title"], "Check task routing");
}

#[tokio::test]
async fn task_title_routes_update_backing_thread_label_and_projection() {
    let dir = tempdir().unwrap();
    let mut config = test_config();
    config.tasks.enabled = true;
    config.sessions.data_dir = Some(dir.path().to_string_lossy().to_string());
    let state = AppStateBuilder::new(config).build();
    let router = build_router(state.clone());

    let request = authed_request()
        .method("GET")
        .uri("/api/threads")
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["count"], 0);

    let request = authed_request()
        .method("POST")
        .uri("/api/tasks")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "title": "Gateway task title",
                "notification_target": {"kind": "none"}
            }))
            .unwrap(),
        ))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let task_id = payload["task_id"].as_str().unwrap().to_owned();
    let thread_id = payload["thread_id"].as_str().unwrap().to_owned();
    let created_title = format!("{task_id} Gateway task title");
    assert_eq!(payload["task"]["title"], "Gateway task title");

    let stored = state
        .threads
        .thread_store
        .get(&thread_id)
        .await
        .expect("task backing thread");
    assert_eq!(stored["label"], created_title);
    assert_eq!(stored["thread_title_source"], "task");
    let recent = state
        .ops
        .garyx_db
        .list_recent_threads(10, 0)
        .expect("recent threads");
    let projected = recent
        .iter()
        .find(|record| record.thread_id == thread_id)
        .expect("projected task thread");
    assert_eq!(projected.title, created_title);
    let request = authed_request()
        .method("GET")
        .uri("/api/threads")
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["count"], 1);
    assert_eq!(payload["threads"][0]["label"], created_title);

    let request = authed_request()
        .method("PATCH")
        .uri(format!(
            "/api/tasks/{}/title",
            urlencoding::encode(&task_id)
        ))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({ "title": "Gateway retitled" })).unwrap(),
        ))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let updated_title = format!("{task_id} Gateway retitled");
    assert_eq!(payload["task"]["title"], "Gateway retitled");

    let stored = state
        .threads
        .thread_store
        .get(&thread_id)
        .await
        .expect("task backing thread");
    assert_eq!(stored["label"], updated_title);
    let recent = state
        .ops
        .garyx_db
        .list_recent_threads(10, 0)
        .expect("recent threads");
    let projected = recent
        .iter()
        .find(|record| record.thread_id == thread_id)
        .expect("projected task thread");
    assert_eq!(projected.title, updated_title);
    let request = authed_request()
        .method("GET")
        .uri("/api/threads")
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["count"], 1);
    assert_eq!(payload["threads"][0]["label"], updated_title);

    let request = authed_request()
        .method("PATCH")
        .uri(format!("/api/threads/{}", urlencoding::encode(&thread_id)))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({ "label": "Manual thread title" })).unwrap(),
        ))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let request = authed_request()
        .method("PATCH")
        .uri(format!(
            "/api/tasks/{}/title",
            urlencoding::encode(&task_id)
        ))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({ "title": "Task-only title" })).unwrap(),
        ))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["task"]["title"], "Task-only title");

    let stored = state
        .threads
        .thread_store
        .get(&thread_id)
        .await
        .expect("task backing thread");
    assert_eq!(stored["label"], "Manual thread title");
    assert_eq!(stored["thread_title_source"], "explicit");
    let recent = state
        .ops
        .garyx_db
        .list_recent_threads(10, 0)
        .expect("recent threads");
    let projected = recent
        .iter()
        .find(|record| record.thread_id == thread_id)
        .expect("projected task thread");
    assert_eq!(projected.title, "Manual thread title");
    let request = authed_request()
        .method("GET")
        .uri("/api/threads")
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["count"], 1);
    assert_eq!(payload["threads"][0]["label"], "Manual thread title");
}

#[tokio::test]
async fn task_create_with_worktree_runtime_creates_thread_in_managed_worktree() {
    let repo = tempdir().unwrap();
    init_test_git_repo(repo.path());
    let data_dir = tempdir().unwrap();
    let mut config = test_config();
    config.tasks.enabled = true;
    config.sessions.data_dir = Some(data_dir.path().join("data").to_string_lossy().to_string());
    let state = AppStateBuilder::new(config).build();
    let router = build_router(state.clone());

    let request = authed_request()
        .method("POST")
        .uri("/api/tasks")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "title": "Task worktree",
                "body": "Use an isolated git worktree.",
                "notification_target": {"kind": "none"},
                "runtime": {
                    "workspace_dir": repo.path(),
                    "workspace_mode": "worktree"
                }
            }))
            .unwrap(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let thread_id = payload["thread_id"].as_str().expect("thread id");
    let stored = state
        .threads
        .thread_store
        .get(thread_id)
        .await
        .expect("stored thread");
    let workspace_dir = stored["workspace_dir"].as_str().expect("workspace dir");
    assert_ne!(workspace_dir, repo.path().to_string_lossy().as_ref());
    assert!(
        Path::new(workspace_dir).starts_with(data_dir.path().join("worktrees")),
        "workspace_dir should be inside managed worktree root: {workspace_dir}"
    );
    assert_eq!(stored["worktree"]["enabled"], true);
    assert_eq!(stored["worktree"]["mode"], "worktree");
    assert_eq!(
        stored["worktree"]["source_repo_root"].as_str(),
        Some(
            repo.path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .as_ref()
        )
    );
    assert_eq!(stored["worktree"]["path"], workspace_dir);
    assert_eq!(stored["worktree"]["worktree_dir"], workspace_dir);
    assert_eq!(stored["worktree"]["thread_id"], thread_id);
}

#[tokio::test]
async fn task_create_with_agent_assignee_queues_agent_dispatch() {
    let dir = tempdir().unwrap();
    let mut config = test_config();
    config.tasks.enabled = true;
    config.sessions.data_dir = Some(dir.path().to_string_lossy().to_string());
    let custom_agents = Arc::new(crate::custom_agents::CustomAgentStore::new());
    custom_agents
        .upsert_agent(crate::custom_agents::UpsertCustomAgentRequest {
            agent_id: "workspace-reviewer".to_owned(),
            display_name: "Workspace Reviewer".to_owned(),
            provider_type: ProviderType::CodexAppServer,
            model: Some("gpt-5".to_owned()),
            model_reasoning_effort: Some(String::new()),
            model_service_tier: Some(String::new()),
            provider_env: None,
            auth_source: None,
            base_url: None,
            codex_home: None,
            max_tool_iterations: None,
            request_timeout_seconds: None,
            default_workspace_dir: Some("/tmp/agent-route-default".to_owned()),
            avatar_data_url: None,
            system_prompt: "Review the assigned task.".to_owned(),
        })
        .await
        .expect("custom agent");

    let provider = Arc::new(RecordingTaskProvider::new());
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("task-recording-provider", provider.clone())
        .await;
    bridge
        .set_route("garyx", "tasks", "task-recording-provider")
        .await;
    bridge
        .set_route("api", "main", "task-recording-provider")
        .await;
    bridge
        .set_default_provider_key("task-recording-provider")
        .await;

    let state = AppStateBuilder::new(config)
        .with_custom_agent_store(custom_agents.clone())
        .with_bridge(bridge.clone())
        .build();
    bridge
        .replace_agent_profiles(custom_agents.list_agents().await)
        .await;
    bridge.set_event_tx(state.ops.events.sender()).await;
    bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;
    let router = build_router(state);

    let request = authed_request()
        .method("POST")
        .uri("/api/tasks")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "title": "Auto dispatch task",
                "body": "Move this task to review and then done.",
                "assignee": {"kind": "agent", "agent_id": "workspace-reviewer"},
                "notification_target": {"kind": "none"}
            }))
            .unwrap(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let task_id = payload["task_id"].as_str().unwrap();
    assert!(task_id.starts_with("#TASK-"));
    assert_eq!(payload["status"], "in_progress");
    assert_eq!(payload["dispatch"]["queued"], true);

    for _ in 0..250 {
        if !provider.runs().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let runs = provider.runs();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].thread_id, payload["thread_id"].as_str().unwrap());
    assert!(runs[0].message.contains(task_id));
    assert!(runs[0].message.contains("Move this task to review"));
    assert!(!runs[0].message.contains("Garyx will move this task"));
    assert!(!runs[0].message.contains("mark it done"));
    assert_eq!(runs[0].metadata["task_auto_start"], true);
    assert_eq!(
        runs[0].workspace_dir.as_deref(),
        Some("/tmp/agent-route-default")
    );
}

#[tokio::test]
async fn task_stop_aborts_active_backing_thread_run_and_releases_task() {
    let dir = tempdir().unwrap();
    let mut config = test_config();
    config.tasks.enabled = true;
    config.sessions.data_dir = Some(dir.path().to_string_lossy().to_string());
    config.channels.api.accounts.insert(
        "main".to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            workspace_mode: None,
        },
    );

    let provider = Arc::new(SlowDeleteProvider::new(2_000));
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("task-stop-provider", provider)
        .await;
    bridge.set_route("api", "main", "task-stop-provider").await;
    bridge.set_default_provider_key("task-stop-provider").await;

    let state = AppStateBuilder::new(config)
        .with_bridge(bridge.clone())
        .build();
    bridge.set_event_tx(state.ops.events.sender()).await;
    bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;
    let router = build_router(state.clone());

    let request = authed_request()
        .method("POST")
        .uri("/api/tasks")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "title": "Stop active task",
                "start": true,
                "notification_target": {"kind": "none"}
            }))
            .unwrap(),
        ))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let task_id = payload["task_id"].as_str().unwrap().to_owned();
    let thread_id = payload["thread_id"].as_str().unwrap().to_owned();
    assert_eq!(payload["status"], "in_progress");

    bridge
        .start_agent_run(
            garyx_models::provider::AgentRunRequest::new(
                &thread_id,
                "run until stopped",
                "run-task-stop",
                "api",
                "main",
                HashMap::new(),
            ),
            None,
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(bridge.is_run_active("run-task-stop").await);

    let request = authed_request()
        .method("POST")
        .uri(format!("/api/tasks/{}/stop", urlencoding::encode(&task_id)))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["task"]["status"], "todo");
    assert!(payload["task"]["assignee"].is_null());
    assert_eq!(payload["interrupted"], true);
    assert_eq!(payload["aborted_runs"], json!(["run-task-stop"]));
    assert!(!bridge.is_run_active("run-task-stop").await);
}

#[tokio::test]
async fn task_delete_aborts_run_and_removes_task_overlay_but_retains_thread() {
    let dir = tempdir().unwrap();
    let mut config = test_config();
    config.tasks.enabled = true;
    config.sessions.data_dir = Some(dir.path().to_string_lossy().to_string());
    config.channels.api.accounts.insert(
        "main".to_owned(),
        ApiAccount {
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            workspace_mode: None,
        },
    );

    let provider = Arc::new(SlowDeleteProvider::new(2_000));
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("task-delete-provider", provider)
        .await;
    bridge
        .set_route("api", "main", "task-delete-provider")
        .await;
    bridge
        .set_default_provider_key("task-delete-provider")
        .await;

    let state = AppStateBuilder::new(config)
        .with_bridge(bridge.clone())
        .build();
    bridge.set_event_tx(state.ops.events.sender()).await;
    bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;
    let router = build_router(state.clone());

    let request = authed_request()
        .method("POST")
        .uri("/api/tasks")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "title": "Delete task metadata",
                "body": "The backing thread remains after deletion.",
                "start": true,
                "notification_target": {"kind": "none"}
            }))
            .unwrap(),
        ))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let task_id = payload["task_id"].as_str().unwrap().to_owned();
    let thread_id = payload["thread_id"].as_str().unwrap().to_owned();

    bridge
        .start_agent_run(
            garyx_models::provider::AgentRunRequest::new(
                &thread_id,
                "delete while running",
                "run-task-delete",
                "api",
                "main",
                HashMap::new(),
            ),
            None,
        )
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(bridge.is_run_active("run-task-delete").await);

    let request = authed_request()
        .method("DELETE")
        .uri(format!("/api/tasks/{}", urlencoding::encode(&task_id)))
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["deleted"], true);
    assert_eq!(payload["task_id"], task_id);
    assert_eq!(payload["thread_id"], thread_id);
    assert_eq!(payload["thread_retained"], true);
    assert_eq!(payload["transcripts_retained"], true);
    assert_eq!(payload["aborted_runs"], json!(["run-task-delete"]));
    assert!(!bridge.is_run_active("run-task-delete").await);

    let retained = state
        .threads
        .thread_store
        .get(&thread_id)
        .await
        .expect("backing thread should remain");
    assert!(retained.get("task").is_none());

    let request = authed_request()
        .method("GET")
        .uri("/api/tasks?include_done=true")
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["total"], 0);
    assert_eq!(payload["tasks"], json!([]));

    let request = authed_request()
        .method("GET")
        .uri(format!("/api/tasks/{}", urlencoding::encode(&task_id)))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn task_assign_queues_dispatch_with_original_body() {
    let dir = tempdir().unwrap();
    let mut config = test_config();
    config.tasks.enabled = true;
    config.sessions.data_dir = Some(dir.path().to_string_lossy().to_string());
    let custom_agents = Arc::new(crate::custom_agents::CustomAgentStore::new());
    custom_agents
        .upsert_agent(crate::custom_agents::UpsertCustomAgentRequest {
            agent_id: "workspace-reviewer".to_owned(),
            display_name: "Workspace Reviewer".to_owned(),
            provider_type: ProviderType::CodexAppServer,
            model: Some("gpt-5".to_owned()),
            model_reasoning_effort: Some(String::new()),
            model_service_tier: Some(String::new()),
            provider_env: None,
            auth_source: None,
            base_url: None,
            codex_home: None,
            max_tool_iterations: None,
            request_timeout_seconds: None,
            default_workspace_dir: Some("/tmp/agent-route-default".to_owned()),
            avatar_data_url: None,
            system_prompt: "Review the assigned task.".to_owned(),
        })
        .await
        .expect("custom agent");

    let provider = Arc::new(RecordingTaskProvider::new());
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("task-recording-provider", provider.clone())
        .await;
    bridge
        .set_route("garyx", "tasks", "task-recording-provider")
        .await;
    bridge
        .set_route("api", "main", "task-recording-provider")
        .await;
    bridge
        .set_default_provider_key("task-recording-provider")
        .await;

    let state = AppStateBuilder::new(config)
        .with_custom_agent_store(custom_agents.clone())
        .with_bridge(bridge.clone())
        .build();
    bridge
        .replace_agent_profiles(custom_agents.list_agents().await)
        .await;
    bridge.set_event_tx(state.ops.events.sender()).await;
    bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;
    let router = build_router(state);

    let request = authed_request()
        .method("POST")
        .uri("/api/tasks")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "title": "Assignable task",
                "body": "Use this original body when assigning later.",
                "runtime": {"agent_id": "workspace-reviewer"},
                "notification_target": {"kind": "none"},
                "start": false
            }))
            .unwrap(),
        ))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let task_id = payload["task_id"].as_str().unwrap();
    assert_eq!(payload["status"], "todo");
    assert!(payload.get("dispatch").is_none());

    let request = authed_request()
        .method("PATCH")
        .uri(format!(
            "/api/tasks/{}/assign",
            urlencoding::encode(task_id)
        ))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "to": {"kind": "agent", "agent_id": "workspace-reviewer"}
            }))
            .unwrap(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["task"]["status"], "in_progress");
    assert_eq!(payload["dispatch"]["queued"], true);

    for _ in 0..250 {
        if !provider.runs().is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let runs = provider.runs();
    assert_eq!(runs.len(), 1);
    assert!(runs[0].message.contains(task_id));
    assert!(
        runs[0]
            .message
            .contains("Use this original body when assigning later.")
    );
    assert!(!runs[0].message.contains("Title: Assignable task"));
    assert!(!runs[0].message.contains("Garyx will move this task"));
    assert!(!runs[0].message.contains("mark it done"));
}

#[tokio::test]
async fn task_assign_rejects_assignee_that_differs_from_bound_thread_agent() {
    let dir = tempdir().unwrap();
    let mut config = test_config();
    config.tasks.enabled = true;
    config.sessions.data_dir = Some(dir.path().to_string_lossy().to_string());
    let custom_agents = Arc::new(crate::custom_agents::CustomAgentStore::new());

    let claude_provider = Arc::new(RecordingTaskProvider::with_provider_type(
        ProviderType::ClaudeCode,
    ));
    let gemini_provider = Arc::new(RecordingTaskProvider::with_provider_type(
        ProviderType::GeminiCli,
    ));
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("task-claude-provider", claude_provider.clone())
        .await;
    bridge
        .register_provider("task-gemini-provider", gemini_provider.clone())
        .await;
    bridge
        .set_route("api", "main", "task-claude-provider")
        .await;
    bridge
        .set_default_provider_key("task-claude-provider")
        .await;

    let state = AppStateBuilder::new(config)
        .with_custom_agent_store(custom_agents.clone())
        .with_bridge(bridge.clone())
        .build();
    bridge
        .replace_agent_profiles(custom_agents.list_agents().await)
        .await;
    bridge.set_event_tx(state.ops.events.sender()).await;
    bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;
    let router = build_router(state.clone());

    let request = authed_request()
        .method("POST")
        .uri("/api/tasks")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "title": "Provider switch task",
                "body": "Run this with the latest assignee provider.",
                "runtime": {"agent_id": "claude", "workspace_dir": "/tmp/provider-switch"},
                "notification_target": {"kind": "none"},
                "start": false
            }))
            .unwrap(),
        ))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let task_id = payload["task_id"].as_str().unwrap();
    let thread_id = payload["thread_id"].as_str().unwrap().to_owned();
    assert_eq!(payload["status"], "todo");
    assert!(payload.get("dispatch").is_none());

    let before = state
        .threads
        .thread_store
        .get(&thread_id)
        .await
        .expect("thread before assign");
    assert_eq!(before["agent_id"], "claude");
    assert_eq!(before["provider_type"], "claude_code");

    let request = authed_request()
        .method("PATCH")
        .uri(format!(
            "/api/tasks/{}/assign",
            urlencoding::encode(task_id)
        ))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "to": {"kind": "agent", "agent_id": "gemini"}
            }))
            .unwrap(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["code"], "BadRequest");
    assert!(
        payload["error"]
            .as_str()
            .unwrap_or_default()
            .contains("is bound to agent claude; cannot assign it to agent gemini")
    );
    assert_eq!(claude_provider.runs().len(), 0);
    assert_eq!(gemini_provider.runs().len(), 0);

    let after = state
        .threads
        .thread_store
        .get(&thread_id)
        .await
        .expect("thread after assign");
    assert_eq!(after["agent_id"], "claude");
    assert_eq!(after["provider_type"], "claude_code");
}

#[tokio::test]
async fn task_create_unassigned_todo_can_be_assigned_to_first_agent() {
    let dir = tempdir().unwrap();
    let mut config = test_config();
    config.tasks.enabled = true;
    config.sessions.data_dir = Some(dir.path().to_string_lossy().to_string());
    let custom_agents = Arc::new(crate::custom_agents::CustomAgentStore::new());
    custom_agents
        .upsert_agent(crate::custom_agents::UpsertCustomAgentRequest {
            agent_id: "late-gemini".to_owned(),
            display_name: "Late Gemini".to_owned(),
            provider_type: ProviderType::GeminiCli,
            model: Some("gemini-test".to_owned()),
            model_reasoning_effort: Some(String::new()),
            model_service_tier: Some(String::new()),
            provider_env: None,
            auth_source: None,
            base_url: None,
            codex_home: None,
            max_tool_iterations: None,
            request_timeout_seconds: None,
            default_workspace_dir: Some("/tmp/late-gemini-default".to_owned()),
            avatar_data_url: None,
            system_prompt: "Work normally.".to_owned(),
        })
        .await
        .expect("custom agent");

    let gemini_provider = Arc::new(RecordingTaskProvider::with_provider_type(
        ProviderType::GeminiCli,
    ));
    let bridge = Arc::new(MultiProviderBridge::new());
    bridge
        .register_provider("task-gemini-provider", gemini_provider.clone())
        .await;
    bridge
        .set_default_provider_key("task-gemini-provider")
        .await;

    let state = AppStateBuilder::new(config)
        .with_custom_agent_store(custom_agents.clone())
        .with_bridge(bridge.clone())
        .build();
    bridge
        .replace_agent_profiles(custom_agents.list_agents().await)
        .await;
    bridge.set_event_tx(state.ops.events.sender()).await;
    bridge
        .set_thread_store(state.threads.thread_store.clone())
        .await;
    let router = build_router(state.clone());

    let request = authed_request()
        .method("POST")
        .uri("/api/tasks")
        .header("X-Garyx-Actor", "agent:codex")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "title": "Assignable later",
                "body": "Created by an agent, assigned later.",
                "notification_target": {"kind": "none"}
            }))
            .unwrap(),
        ))
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    let task_id = payload["task_id"].as_str().unwrap();
    let thread_id = payload["thread_id"].as_str().unwrap().to_owned();
    assert_eq!(payload["status"], "todo");
    assert_eq!(payload["runtime_agent_id"], "");
    assert!(payload.get("dispatch").is_none());

    let before = state
        .threads
        .thread_store
        .get(&thread_id)
        .await
        .expect("thread before assign");
    assert!(before.get("agent_id").is_none());
    assert!(before.get("provider_type").is_none());

    let request = authed_request()
        .method("PATCH")
        .uri(format!(
            "/api/tasks/{}/assign",
            urlencoding::encode(task_id)
        ))
        .header("X-Garyx-Actor", "agent:codex")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "to": {"kind": "agent", "agent_id": "late-gemini"}
            }))
            .unwrap(),
        ))
        .unwrap();
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let payload: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["task"]["status"], "in_progress");
    assert_eq!(payload["dispatch"]["queued"], true);

    let after = state
        .threads
        .thread_store
        .get(&thread_id)
        .await
        .expect("thread after assign");
    assert_eq!(after["agent_id"], "late-gemini");
    assert_eq!(after["provider_type"], json!(ProviderType::GeminiCli));
    assert_eq!(after["workspace_dir"], "/tmp/late-gemini-default");
}
