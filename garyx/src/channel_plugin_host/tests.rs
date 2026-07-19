use super::*;
use garyx_channels::channel_trait::ChannelError;
use garyx_channels::dispatcher::{
    ChannelDispatcherImpl, ChannelInfo, SendMessageResult, StreamDispatchCallback,
    StreamDispatchEnvelope, StreamingDispatchTarget,
};
use garyx_channels::plugin_host::{
    CapabilitiesResponse, DispatchOutbound, DispatchOutboundResult, PluginRpcClient,
    PluginSenderHandle, Transport, TransportConfig,
};
use garyx_router::recent_threads::{
    RecentThreadFilter, RecentThreadListEntry, RecentThreadPage, RecentThreadPageReader,
};
use garyx_router::{
    InMemoryThreadStore, NATIVE_COMMAND_TEXT_METADATA_KEY, ThreadHistoryRepository, ThreadStore,
    ThreadTranscriptStore,
};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default)]
struct RecordingOriginNativeDispatcher {
    envelopes: Arc<StdMutex<Vec<StreamDispatchEnvelope>>>,
}

impl RecordingOriginNativeDispatcher {
    fn envelopes(&self) -> Vec<StreamDispatchEnvelope> {
        self.envelopes
            .lock()
            .expect("recording native origin stream dispatcher lock poisoned")
            .clone()
    }
}

#[async_trait::async_trait]
impl ChannelDispatcher for RecordingOriginNativeDispatcher {
    async fn send_message(
        &self,
        _request: OutboundMessage,
    ) -> Result<SendMessageResult, ChannelError> {
        unreachable!("native origin stream test should not call send_message")
    }

    fn available_channels(&self) -> Vec<ChannelInfo> {
        Vec::new()
    }

    fn build_stream_event_callback(
        &self,
        _target: StreamingDispatchTarget,
    ) -> Option<StreamDispatchCallback> {
        let envelopes = self.envelopes.clone();
        Some(Arc::new(move |envelope| {
            envelopes
                .lock()
                .expect("recording native origin stream dispatcher lock poisoned")
                .push(envelope);
        }))
    }
}

struct RejectInbound;

#[async_trait::async_trait]
impl InboundHandler for RejectInbound {
    async fn on_request(&self, method: String, _params: Value) -> Result<Value, (i32, String)> {
        Err((-32601, format!("unexpected request: {method}")))
    }

    async fn on_notification(&self, _method: String, _params: Value) {}
}

struct RecordingOutboundPlugin {
    requests: Arc<StdMutex<Vec<DispatchOutbound>>>,
}

#[async_trait::async_trait]
impl InboundHandler for RecordingOutboundPlugin {
    async fn on_request(&self, method: String, params: Value) -> Result<Value, (i32, String)> {
        if method != "dispatch_outbound" {
            return Err((-32601, format!("unexpected request: {method}")));
        }
        let request: DispatchOutbound = serde_json::from_value(params)
            .map_err(|error| (-32602, format!("invalid outbound request: {error}")))?;
        self.requests
            .lock()
            .expect("recording outbound plugin lock poisoned")
            .push(request);
        serde_json::to_value(DispatchOutboundResult {
            message_ids: vec!["outbound-message-1".to_owned()],
        })
        .map_err(|error| (-32603, error.to_string()))
    }

    async fn on_notification(&self, _method: String, _params: Value) {}
}

fn recording_plugin_sender() -> (
    PluginSenderHandle,
    Arc<StdMutex<Vec<DispatchOutbound>>>,
    PluginRpcClient,
) {
    let (host_io, plugin_io) = tokio::io::duplex(64 * 1024);
    let (host_reader, host_writer) = tokio::io::split(host_io);
    let (plugin_reader, plugin_writer) = tokio::io::split(plugin_io);
    let (host_rpc, _host_handles) = Transport::spawn(
        host_reader,
        host_writer,
        TransportConfig {
            plugin_id: "test-plugin".to_owned(),
            ..Default::default()
        },
        Arc::new(RejectInbound),
    );
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let (plugin_keep_alive, _plugin_handles) = Transport::spawn(
        plugin_reader,
        plugin_writer,
        TransportConfig {
            plugin_id: "test-plugin-peer".to_owned(),
            ..Default::default()
        },
        Arc::new(RecordingOutboundPlugin {
            requests: requests.clone(),
        }),
    );
    let sender = PluginSenderHandle::new(
        "test-plugin".to_owned(),
        host_rpc,
        CapabilitiesResponse {
            outbound: true,
            inbound: true,
            streaming: false,
            dispatch_stream_event: false,
            images: false,
            files: false,
            survives_respawn: false,
        },
    );
    (sender, requests, plugin_keep_alive)
}

struct RecordingRecentThreadPageReader {
    entries: Vec<RecentThreadListEntry>,
    calls: StdMutex<Vec<(RecentThreadFilter, usize, usize)>>,
}

#[async_trait::async_trait]
impl RecentThreadPageReader for RecordingRecentThreadPageReader {
    async fn page(
        &self,
        filter: RecentThreadFilter,
        limit: usize,
        offset: usize,
    ) -> Result<RecentThreadPage, String> {
        self.calls
            .lock()
            .expect("recent page calls lock poisoned")
            .push((filter, limit, offset));
        let entries = self
            .entries
            .iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect::<Vec<_>>();
        Ok(RecentThreadPage {
            has_more: offset.saturating_add(entries.len()) < self.entries.len(),
            entries,
            total: self.entries.len(),
            offset,
        })
    }

    async fn contains_selectable_thread(&self, thread_id: &str) -> Result<bool, String> {
        Ok(self
            .entries
            .iter()
            .any(|entry| entry.thread_id == thread_id))
    }
}

fn build_handler() -> HostInboundHandler {
    build_handler_with_config(GaryxConfig::default())
}

fn build_handler_with_config(config: GaryxConfig) -> HostInboundHandler {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let router = Arc::new(Mutex::new(MessageRouter::new(store, config)));
    let bridge = Arc::new(MultiProviderBridge::new());
    let swap = Arc::new(SwappableDispatcher::new(ChannelDispatcherImpl::new()));
    // Both C-architecture deps are inert in tests that don't drive
    // the request_self_replace path: a never-upgradable Weak +
    // master switch defaulted to false so any stray RPC short-
    // circuits via "refused: master_disabled" instead of trying to
    // hit the real GitHub API.
    let plugin_manager: std::sync::Weak<Mutex<garyx_channels::plugin::ChannelPluginManager>> =
        std::sync::Weak::new();
    let plugin_auto_update_enabled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    HostInboundHandler::new(
        "test-plugin".into(),
        router,
        bridge,
        swap,
        plugin_manager,
        plugin_auto_update_enabled,
    )
}

#[tokio::test]
async fn abandon_inbound_tombstones_stream() {
    let handler = build_handler();
    // Stream id that's never been issued — abandon must still
    // tombstone it cleanly and return {ok:true} (idempotent by
    // §7.3: plugin may race the host and abandon before the
    // host's deliver_inbound reply even lands).
    let params = json!({ "stream_id": "str_abandon_1", "reason": "user cancelled" });
    let result = handler
        .handle_abandon_inbound(params)
        .expect("abandon_inbound should succeed");
    assert_eq!(result["ok"], true);
    let id = StreamId::from("str_abandon_1");
    assert!(
        handler.streams.is_tombstoned(&id),
        "stream must be tombstoned after abandon_inbound"
    );
}

#[tokio::test]
async fn abandon_inbound_rejects_malformed_params() {
    let handler = build_handler();
    let result = handler.handle_abandon_inbound(json!({ "reason": "no id" }));
    match result {
        Err((code, _)) => assert_eq!(code, PluginErrorCode::InvalidParams.as_i32()),
        Ok(v) => panic!("expected InvalidParams, got {v:?}"),
    }
}

#[tokio::test]
async fn abandon_inbound_is_idempotent() {
    let handler = build_handler();
    let params = json!({ "stream_id": "str_idempotent", "reason": "first" });
    handler.handle_abandon_inbound(params.clone()).unwrap();
    // Second abandon on the same id must still return {ok:true};
    // the tombstone registry dedupes internally.
    let second = handler.handle_abandon_inbound(params).unwrap();
    assert_eq!(second["ok"], true);
}

#[test]
fn active_stream_count_is_zero_when_no_live_streams() {
    // Fresh handler has never seen deliver_inbound — the auto-update
    // stream-idle gate must observe a clean 0 so it can proceed.
    let handler = build_handler();
    assert_eq!(handler.active_stream_count(), 0);
}

#[test]
fn active_stream_count_reflects_live_streams_set_cardinality() {
    // `live_streams` is the source of truth — `handle_deliver_inbound`
    // inserts at stream-id allocation and removes after
    // `route_and_dispatch` returns, so its cardinality at any instant
    // is the count of in-flight inbound dispatches the host is
    // driving. Simulate two concurrent inbound runs directly to keep
    // this test independent of the full deliver_inbound async path.
    let handler = build_handler();
    {
        let mut guard = handler.live_streams.lock().expect("live_streams lock");
        guard.insert("str_active_a".to_owned());
        guard.insert("str_active_b".to_owned());
    }
    assert_eq!(
        handler.active_stream_count(),
        2,
        "two inserted ids must surface as count=2"
    );

    // After one finishes (mirrors the post-route_and_dispatch remove),
    // count drops to 1.
    {
        let mut guard = handler.live_streams.lock().expect("live_streams lock");
        guard.remove("str_active_a");
    }
    assert_eq!(handler.active_stream_count(), 1);

    // Drained → 0, idle-gate-ready.
    {
        let mut guard = handler.live_streams.lock().expect("live_streams lock");
        guard.remove("str_active_b");
    }
    assert_eq!(handler.active_stream_count(), 0);
}

#[tokio::test]
async fn deferred_origin_native_stream_buffers_until_thread_attached() {
    let handler = build_handler();
    let dispatcher = Arc::new(RecordingOriginNativeDispatcher::default());
    let stream_id = "str_origin_native_1".to_owned();
    handler
        .live_streams
        .lock()
        .expect("live stream lock")
        .insert(stream_id.clone());

    let origin = DeferredOriginNativeStream::new(DeferredOriginNativeStreamCtx {
        plugin_id: "minolab".to_owned(),
        account_id: "main".to_owned(),
        chat_id: "chat-1".to_owned(),
        stream_id: stream_id.clone(),
        run_id: "run-origin-native".to_owned(),
        endpoint_identity: "minolab::main::chat-1".to_owned(),
        dispatcher: dispatcher.clone(),
        streams: handler.streams.clone(),
        live_streams: handler.live_streams.clone(),
    });
    let consumer = origin.consumer();

    consumer(StreamEvent::Delta {
        text: "first".to_owned(),
    });
    assert!(
        dispatcher.envelopes().is_empty(),
        "origin stream events must wait for canonical thread id"
    );

    origin.attach_thread("thread::origin-native").await;
    let envelopes = dispatcher.envelopes();
    assert_eq!(envelopes.len(), 1);
    assert_eq!(envelopes[0].thread_id, "thread::origin-native");
    assert_eq!(envelopes[0].run_id, "run-origin-native");
    assert_eq!(envelopes[0].endpoint_identity, "minolab::main::chat-1");
    assert_eq!(envelopes[0].chat_id, "chat-1");
    assert!(matches!(
        envelopes[0].event,
        StreamEvent::Delta { ref text } if text == "first"
    ));

    consumer(StreamEvent::Done);
    let envelopes = dispatcher.envelopes();
    assert_eq!(envelopes.len(), 2);
    assert!(matches!(envelopes[1].event, StreamEvent::Done));
    assert!(
        !handler
            .live_streams
            .lock()
            .expect("live stream lock")
            .contains(&stream_id),
        "native origin stream must clear the live stream gate on Done"
    );
}

#[test]
fn merge_inbound_image_refs_promotes_path_images_into_attachment_metadata() {
    let mut metadata = HashMap::from([(
        "attachments".to_owned(),
        json!([
            {
                "kind": "image",
                "path": "/tmp/existing.png",
                "name": "existing.png",
                "media_type": "image/png"
            }
        ]),
    )]);

    let inline_images = HostInboundHandler::merge_inbound_image_refs(
        &[
            AttachmentRef::Inline {
                data: "YWJj".to_owned(),
                media_type: "image/png".to_owned(),
            },
            AttachmentRef::Path {
                path: "/tmp/path-image.webp".to_owned(),
                media_type: "image/webp".to_owned(),
            },
        ],
        &mut metadata,
    );

    assert_eq!(inline_images.len(), 1);
    assert_eq!(inline_images[0].data, "YWJj");
    let attachments = attachments_from_metadata(&metadata);
    assert_eq!(attachments.len(), 2);
    assert_eq!(attachments[0].path, "/tmp/existing.png");
    assert_eq!(attachments[1].kind, PromptAttachmentKind::Image);
    assert_eq!(attachments[1].path, "/tmp/path-image.webp");
    assert_eq!(attachments[1].name, "path-image.webp");
}

#[tokio::test]
async fn commands_list_returns_plugin_filtered_command_list() {
    let mut config = GaryxConfig::default();
    config.commands.push(garyx_models::config::SlashCommand {
        name: "summary".to_owned(),
        description: "Summarize the active thread".to_owned(),
        prompt: Some("Please summarize the active thread.".to_owned()),
        skill_id: None,
    });
    let handler = build_handler_with_config(config);

    let result = handler
        .on_request(
            "commands/list".to_owned(),
            json!({
                "account_id": "main",
                "surface": "telegram",
                "include_hidden": false
            }),
        )
        .await
        .expect("commands/list should return command list");

    assert_eq!(result["version"], 1);
    assert!(result["revision"].as_str().unwrap().starts_with("v1:"));
    let names = result["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(names.contains(&"newthread"));
    assert!(names.contains(&"summary"));
    assert!(
        result["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["name"] == "newthread" && entry["kind"] == "channel_native" })
    );
    assert!(
        result["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["name"] == "summary" && entry["kind"] == "shortcut" })
    );
}

#[tokio::test]
async fn deliver_inbound_ignores_deprecated_reply_id_and_uses_current_binding() {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let current_thread_id = "thread::current";
    store
        .set(
            current_thread_id,
            json!({
                "thread_id": current_thread_id,
                "label": "Current thread",
            }),
        )
        .await
        .unwrap();
    let reader = Arc::new(RecordingRecentThreadPageReader {
        entries: (1..=11)
            .map(|index| RecentThreadListEntry {
                thread_id: format!("thread::recent-{index:02}"),
                title: format!("Recent thread {index}"),
                last_message_preview: format!("Preview {index}"),
                last_active_at: format!("2026-07-11T12:{index:02}:00Z"),
            })
            .collect(),
        calls: StdMutex::new(Vec::new()),
    });
    let mut router = MessageRouter::new(store.clone(), GaryxConfig::default());
    router.set_recent_thread_page_reader(reader.clone());
    router.switch_to_thread(
        &MessageRouter::build_binding_context_key("test-plugin", "main", "chat-1"),
        current_thread_id,
    );

    let bridge = Arc::new(MultiProviderBridge::new());
    bridge.set_thread_store(store.clone()).await;
    bridge.set_thread_history(Arc::new(ThreadHistoryRepository::new(
        store,
        Arc::new(ThreadTranscriptStore::memory()),
    )));
    let (event_tx, _event_rx) = tokio::sync::broadcast::channel(16);
    bridge.set_event_tx(event_tx).await;

    let (plugin_sender, outbound_requests, _plugin_keep_alive) = recording_plugin_sender();
    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher
        .register_plugin(plugin_sender)
        .expect("test plugin sender should register");
    let handler = HostInboundHandler::new(
        "test-plugin".to_owned(),
        Arc::new(Mutex::new(router)),
        bridge,
        Arc::new(SwappableDispatcher::new(dispatcher)),
        std::sync::Weak::new(),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    );

    let result = handler
        .on_request(
            "deliver_inbound".to_owned(),
            json!({
                "account_id": "main",
                "from_id": "user-1",
                "thread_binding_key": "chat-1",
                "message": "/threads@sample_bot 2",
                "reply_to_message_id": "message-from-another-thread",
                "extra_metadata": {
                    NATIVE_COMMAND_TEXT_METADATA_KEY: "/threads@sample_bot 2",
                    "chat_id": "chat-1"
                }
            }),
        )
        .await
        .expect("native command should be handled by the host router");

    assert_eq!(result["thread_id"], current_thread_id);
    assert!(result["local_reply"].is_null());
    assert_eq!(
        *reader
            .calls
            .lock()
            .expect("recent page calls lock poisoned"),
        vec![(RecentThreadFilter::Exclude, 10, 10)],
        "the addressed command and its page argument must reach the router intact"
    );

    let outbound_requests = outbound_requests
        .lock()
        .expect("recording outbound plugin lock poisoned");
    assert_eq!(outbound_requests.len(), 1);
    let outbound = &outbound_requests[0];
    assert_eq!(outbound.account_id, "main");
    assert_eq!(outbound.chat_id, "chat-1");
    assert_eq!(outbound.thread_id.as_deref(), Some(current_thread_id));
    let reply = outbound
        .content
        .as_text()
        .expect("local command reply should be outbound text");
    assert!(reply.contains("Recent threads · page 2/2 (11 total)"));
    assert!(reply.contains("11. Recent thread 11"));
}

/// Source guard: the subprocess plugin host must route inbound
/// dispatches through the shared `InboundPipeline`, never hand-roll
/// the fanout → committed-replay → route_and_dispatch → attach →
/// settle sequence again. Every marker below names one piece of the
/// formerly duplicated orchestration; if any of them reappears in
/// this file, the fifth inbound copy is being reintroduced.
#[test]
fn host_inbound_goes_through_shared_pipeline_only() {
    let source = include_str!("../channel_plugin_host.rs");
    assert!(
        source.contains("inbound::InboundPipeline"),
        "the host must dispatch through garyx_channels::inbound::InboundPipeline"
    );
    for forbidden in [
        "route_and_dispatch(",
        "committed_replay::committed_callback",
        "DeferredBoundStreamFanout",
        "DeferredFanoutAgentDispatcher",
        "replay_subscription",
    ] {
        let occurrences = source.matches(forbidden).count();
        assert_eq!(
            occurrences, 0,
            "channel_plugin_host.rs must not hand-roll the inbound sequence; found `{forbidden}` {occurrences} time(s)"
        );
    }
}
