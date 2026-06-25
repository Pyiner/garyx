use super::*;
use garyx_channels::channel_trait::ChannelError;
use garyx_channels::dispatcher::{
    ChannelDispatcherImpl, ChannelInfo, SendMessageResult, StreamDispatchCallback,
    StreamDispatchEnvelope, StreamingDispatchTarget,
};
use garyx_router::{InMemoryThreadStore, ThreadStore};
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
        _router: Arc<Mutex<MessageRouter>>,
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
        router: handler.router.clone(),
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
