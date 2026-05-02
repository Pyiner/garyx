pub mod auth_flow;
pub mod builtin_catalog;
pub mod channel_trait;
pub mod dispatcher;
pub mod feishu;
pub(crate) mod generated_images;
pub mod plugin;
pub mod plugin_host;
pub mod streaming_core;
pub mod telegram;
pub mod weixin;
pub mod weixin_auth;
pub mod weixin_auth_executor;

/// Sanitize a filename for safe local storage.
///
/// Replaces any character that is not alphanumeric, `.`, `-`, or `_` with `_`.
/// Returns `"file.bin"` for empty, `"."`, or `".."` inputs.
pub(crate) fn sanitize_filename(name: &str) -> String {
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
pub use dispatcher::{
    ChannelDispatcher, ChannelDispatcherImpl, ChannelInfo, FeishuChatSummary, FeishuSender,
    OutboundMessage, SendMessageResult, StreamingDispatchTarget, SwappableDispatcher,
    TelegramSender,
};
pub use feishu::FeishuChannel;
pub use garyx_models::ChannelOutboundContent;
pub use plugin::{
    BuiltInPluginDiscoverer, ChannelPluginManager, LocalDescriptorDiscoverer, PluginMetadata,
    PluginState, PluginStatus, SubprocessPluginCatalogEntry, SubprocessPluginError,
    builtin_plugin_metadata, builtin_plugin_metadata_list,
};
pub use telegram::TelegramChannel;
pub use weixin::WeixinChannel;
pub use weixin_auth_executor::WeixinAuthExecutor;
