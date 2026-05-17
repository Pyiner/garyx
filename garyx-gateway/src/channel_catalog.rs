//! Synthesize catalog entries for the *built-in* channels
//! (telegram / feishu / weixin) so the desktop UI can render them
//! through the same schema-driven path as subprocess plugins.
//!
//! When a built-in channel eventually migrates to a Scheme B
//! subprocess plugin, the entry disappears from this module and the
//! plugin's own `describe` response takes over — UI code never
//! changes. That's the whole point.
//!
//! The schemas here deliberately cover only the channel-specific
//! config fields (token, app_id, base_url, etc.). Host-side
//! metadata (`enabled`, `name`, `agent_id`, `workspace_dir`) is
//! shared across every channel and rendered by a generic wrapper
//! form, not part of the per-channel schema.

use garyx_channels::PluginState;
use garyx_channels::SubprocessPluginCatalogEntry;
use garyx_channels::builtin_catalog::{
    BuiltinChannelDescriptor, BuiltinChannelKind, builtin_channel_descriptor,
};
use garyx_channels::plugin_host::AccountDescriptor;
use garyx_models::config::ChannelsConfig;
use serde_json::json;

fn bundled_image_data_url(bytes: &[u8], media_type: &str) -> String {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{media_type};base64,{encoded}")
}

fn telegram_icon_data_url() -> String {
    bundled_image_data_url(
        include_bytes!("../assets/channel-icons/telegram.png"),
        "image/png",
    )
}

fn feishu_icon_data_url() -> String {
    bundled_image_data_url(
        include_bytes!("../assets/channel-icons/feishu.png"),
        "image/png",
    )
}

fn weixin_icon_data_url() -> String {
    bundled_image_data_url(
        include_bytes!("../assets/channel-icons/weixin.png"),
        "image/png",
    )
}

fn discord_icon_data_url() -> String {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="14" fill="#5865F2"/><path fill="#fff" d="M44.2 21.3a29.4 29.4 0 0 0-7.2-2.2l-.4.8c2.7.6 4 1.6 4 1.6a25.4 25.4 0 0 0-17.2 0s1.4-1 4-1.6l-.4-.8a29.4 29.4 0 0 0-7.2 2.2c-4.5 6.7-5.7 13.2-5.1 19.6a29.7 29.7 0 0 0 8.8 4.5l1.8-3a18.8 18.8 0 0 1-2.9-1.4l.7-.5a21.1 21.1 0 0 0 17.8 0l.7.5c-.9.5-1.9 1-2.9 1.4l1.8 3a29.7 29.7 0 0 0 8.8-4.5c.8-7.4-1.3-13.8-5.1-19.6ZM25.8 36.7c-1.7 0-3.1-1.6-3.1-3.5s1.4-3.5 3.1-3.5c1.8 0 3.2 1.6 3.1 3.5 0 1.9-1.4 3.5-3.1 3.5Zm12.4 0c-1.7 0-3.1-1.6-3.1-3.5s1.4-3.5 3.1-3.5c1.8 0 3.2 1.6 3.1 3.5 0 1.9-1.3 3.5-3.1 3.5Z"/></svg>"##;
    bundled_image_data_url(svg.as_bytes(), "image/svg+xml")
}

/// Build one catalog entry per enabled built-in channel from the live
/// config. Channels with no accounts still appear so the UI can show
/// them as "Add your first <channel>".
pub fn builtin_channel_catalog(channels: &ChannelsConfig) -> Vec<SubprocessPluginCatalogEntry> {
    vec![
        telegram_catalog(channels),
        discord_catalog(channels),
        feishu_catalog(channels),
        weixin_catalog(channels),
    ]
}

fn builtin_icon_data_url(descriptor: BuiltinChannelDescriptor) -> String {
    match descriptor.kind {
        BuiltinChannelKind::Telegram => telegram_icon_data_url(),
        BuiltinChannelKind::Discord => discord_icon_data_url(),
        BuiltinChannelKind::Feishu => feishu_icon_data_url(),
        BuiltinChannelKind::Weixin => weixin_icon_data_url(),
    }
}

fn builtin_catalog_entry(
    descriptor: BuiltinChannelDescriptor,
    accounts: Vec<AccountDescriptor>,
) -> SubprocessPluginCatalogEntry {
    let mut entry = SubprocessPluginCatalogEntry {
        id: descriptor.id.into(),
        display_name: descriptor.display_name.into(),
        version: env!("CARGO_PKG_VERSION").into(),
        description: descriptor.description.into(),
        state: PluginState::Ready,
        last_error: None,
        capabilities: descriptor.capabilities(),
        schema: descriptor.schema(),
        auth_flows: descriptor.auth_flows(),
        config_methods: descriptor.config_methods(),
        accounts,
        // Built-ins ride the same `icon_data_url` contract as
        // subprocess plugins so the desktop never needs a separate
        // hardcoded logo map.
        icon_data_url: Some(builtin_icon_data_url(descriptor)),
        account_root_behavior: descriptor.account_root_behavior,
    };
    entry.project_account_configs_through_schema();
    entry
}

fn telegram_catalog(channels: &ChannelsConfig) -> SubprocessPluginCatalogEntry {
    let descriptor = builtin_channel_descriptor("telegram").expect("builtin telegram descriptor");
    let accounts = channels
        .resolved_telegram_config()
        .unwrap_or_default()
        .accounts
        .iter()
        .map(|(id, account)| AccountDescriptor {
            id: id.clone(),
            enabled: account.enabled,
            config: json!({
                "token": account.token,
            }),
        })
        .collect();
    builtin_catalog_entry(descriptor, accounts)
}

fn discord_catalog(channels: &ChannelsConfig) -> SubprocessPluginCatalogEntry {
    let descriptor = builtin_channel_descriptor("discord").expect("builtin discord descriptor");
    let accounts = channels
        .resolved_discord_config()
        .unwrap_or_default()
        .accounts
        .iter()
        .map(|(id, account)| AccountDescriptor {
            id: id.clone(),
            enabled: account.enabled,
            config: json!({
                "token": account.token,
                "require_mention": account.require_mention,
            }),
        })
        .collect();
    builtin_catalog_entry(descriptor, accounts)
}

fn feishu_catalog(channels: &ChannelsConfig) -> SubprocessPluginCatalogEntry {
    let descriptor = builtin_channel_descriptor("feishu").expect("builtin feishu descriptor");
    let accounts = channels
        .resolved_feishu_config()
        .unwrap_or_default()
        .accounts
        .iter()
        .map(|(id, account)| AccountDescriptor {
            id: id.clone(),
            enabled: account.enabled,
            config: json!({
                "app_id": account.app_id,
                "app_secret": account.app_secret,
                "domain": account.domain,
                "require_mention": account.require_mention,
                "topic_session_mode": account.topic_session_mode,
            }),
        })
        .collect();
    builtin_catalog_entry(descriptor, accounts)
}

fn weixin_catalog(channels: &ChannelsConfig) -> SubprocessPluginCatalogEntry {
    let descriptor = builtin_channel_descriptor("weixin").expect("builtin weixin descriptor");
    let accounts = channels
        .resolved_weixin_config()
        .unwrap_or_default()
        .accounts
        .iter()
        .map(|(id, account)| AccountDescriptor {
            id: id.clone(),
            enabled: account.enabled,
            config: json!({
                "token": account.token,
                "uin": account.uin,
                "base_url": account.base_url,
                "streaming_update": account.streaming_update,
            }),
        })
        .collect();
    builtin_catalog_entry(descriptor, accounts)
}

// Schema + capabilities helpers moved to
// `garyx_channels::builtin_catalog` so both this legacy path and the
// new `ChannelPlugin::schema()` / `ChannelPlugin::capabilities()`
// trait methods read from a single source of truth. This file only
// assembles `SubprocessPluginCatalogEntry`s from the shared helpers.

#[cfg(test)]
mod tests;
