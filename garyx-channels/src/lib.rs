pub mod auth_flow;
/// Deferred bound-endpoint fanout. `pub(crate)` on purpose: the only
/// production consumer is [`inbound::InboundPipeline`]; keeping the
/// module crate-private makes hand-rolling a second inbound
/// orchestration in downstream crates a compile error, not just a
/// source-guard failure.
pub(crate) mod bound_fanout;
pub mod builtin_catalog;
pub mod channel_trait;
pub mod committed_replay;
pub mod discord;
pub mod dispatcher;
pub mod feishu;
pub mod generated_images;
pub mod inbound;
pub mod meeting_sink;
pub(crate) mod outbound_registry;
pub mod plugin;
pub mod plugin_host;
pub mod plugin_tools;
pub mod streaming_core;
pub mod telegram;
pub mod weixin;
pub mod weixin_auth;
pub mod weixin_auth_executor;

/// Sanitize a filename for safe local storage.
///
/// Replaces any character that is not alphanumeric, `.`, `-`, or `_` with `_`.
/// Returns `"file.bin"` for empty, `"."`, or `".."` inputs.
pub fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "file.bin".to_owned()
    } else {
        cleaned
    }
}

#[cfg(test)]
pub(crate) mod test_helpers;

pub use channel_trait::{Channel, ChannelError};
pub use discord::DiscordChannel;
pub use dispatcher::{
    ChannelDispatcher, ChannelDispatcherImpl, ChannelInfo, DiscordSender, FeishuChatSummary,
    FeishuSender, OutboundMessage, SendMessageResult, StreamDispatchCallback,
    StreamDispatchEnvelope, StreamDispatchRole, StreamingDispatchTarget, SwappableDispatcher,
    TelegramSender, build_outbound_stream_callback, build_stream_dispatch_callback,
};
pub use feishu::FeishuChannel;
pub use garyx_models::ChannelOutboundContent;
pub use meeting_sink::{
    JoinedMeeting, MeetingApiError, MeetingEventSink, MeetingInvite, MeetingPlatformClient,
    NoopMeetingEventSink, noop_meeting_event_sink,
};
pub use plugin::{
    BuiltInPluginDiscoverer, ChannelPluginManager, PluginMetadata, PluginState, PluginStatus,
    SubprocessPluginCatalogEntry, SubprocessPluginError, builtin_plugin_metadata,
    builtin_plugin_metadata_list,
};
pub use telegram::TelegramChannel;
pub use weixin::WeixinChannel;
pub use weixin_auth_executor::WeixinAuthExecutor;
