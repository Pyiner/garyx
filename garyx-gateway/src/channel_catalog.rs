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

/// Build one catalog entry per enabled built-in channel from the live
/// config. Channels with no accounts still appear so the UI can show
/// them as "Add your first <channel>".
pub fn builtin_channel_catalog(channels: &ChannelsConfig) -> Vec<SubprocessPluginCatalogEntry> {
    vec![
        telegram_catalog(channels),
        feishu_catalog(channels),
        weixin_catalog(channels),
    ]
}

fn builtin_icon_data_url(descriptor: BuiltinChannelDescriptor) -> String {
    match descriptor.kind {
        BuiltinChannelKind::Telegram => telegram_icon_data_url(),
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
