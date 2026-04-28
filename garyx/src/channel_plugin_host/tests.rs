use super::*;
use garyx_channels::dispatcher::ChannelDispatcherImpl;
use garyx_router::{InMemoryThreadStore, ThreadStore};
use std::collections::HashMap;
use std::sync::Arc;

fn build_handler() -> HostInboundHandler {
    build_handler_with_config(GaryxConfig::default())
}

fn build_handler_with_config(config: GaryxConfig) -> HostInboundHandler {
    let store: Arc<dyn ThreadStore> = Arc::new(InMemoryThreadStore::new());
    let router = Arc::new(Mutex::new(MessageRouter::new(store, config)));
    let bridge = Arc::new(MultiProviderBridge::new());
    let swap = Arc::new(SwappableDispatcher::new(ChannelDispatcherImpl::new()));
    HostInboundHandler::new("test-plugin".into(), router, bridge, swap)
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
