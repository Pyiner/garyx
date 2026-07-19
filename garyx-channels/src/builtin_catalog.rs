//! Built-in channel descriptor catalog. This is the single source of
//! truth for the protocol-visible contract of in-process channels:
//! metadata, schema, capabilities, auth-flow descriptors,
//! config-methods, and account-root behavior.
//!
//! The gateway's `channel_catalog.rs` and the in-process
//! [`crate::plugin::ChannelPlugin`] discoverer both derive from this
//! module so built-ins match subprocess plugins everywhere except the
//! transport (`describe` RPC vs direct function call).
//!
//! Kept as literal JSON so the contract with the desktop UI is
//! reviewable without running code — a picky regression test in the
//! gateway pins these against the wire shape.

use serde_json::{Value, json};

use crate::auth_flow::ConfigMethod;
use crate::plugin_host::ManifestCapabilities;
use crate::plugin_host::manifest::{AccountRootBehavior, AuthFlowDescriptor, DeliveryModel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinChannelKind {
    Telegram,
    Discord,
    Feishu,
    Weixin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinChannelDescriptor {
    pub kind: BuiltinChannelKind,
    pub id: &'static str,
    pub aliases: &'static [&'static str],
    pub display_name: &'static str,
    pub description: &'static str,
    pub account_root_behavior: AccountRootBehavior,
}

const BUILTIN_CHANNEL_DESCRIPTORS: [BuiltinChannelDescriptor; 4] = [
    BuiltinChannelDescriptor {
        kind: BuiltinChannelKind::Telegram,
        id: "telegram",
        aliases: &["tg"],
        display_name: "Telegram",
        description: "Built-in Telegram channel runtime",
        account_root_behavior: AccountRootBehavior::OpenDefault,
    },
    BuiltinChannelDescriptor {
        kind: BuiltinChannelKind::Discord,
        id: "discord",
        aliases: &[],
        display_name: "Discord",
        description: "Built-in Discord channel runtime",
        account_root_behavior: AccountRootBehavior::OpenDefault,
    },
    BuiltinChannelDescriptor {
        kind: BuiltinChannelKind::Feishu,
        id: "feishu",
        aliases: &["lark"],
        display_name: "Feishu / Lark",
        description: "Built-in Feishu/Lark channel runtime",
        account_root_behavior: AccountRootBehavior::OpenDefault,
    },
    BuiltinChannelDescriptor {
        kind: BuiltinChannelKind::Weixin,
        id: "weixin",
        aliases: &["wx", "wechat"],
        display_name: "Weixin (WeChat)",
        description: "Built-in Weixin channel runtime",
        account_root_behavior: AccountRootBehavior::OpenDefault,
    },
];

impl BuiltinChannelDescriptor {
    pub fn capabilities(self) -> ManifestCapabilities {
        match self.kind {
            BuiltinChannelKind::Telegram
            | BuiltinChannelKind::Discord
            | BuiltinChannelKind::Feishu
            | BuiltinChannelKind::Weixin => builtin_capabilities(true, true, false),
        }
    }

    pub fn schema(self) -> Value {
        match self.kind {
            BuiltinChannelKind::Telegram => telegram_schema(),
            BuiltinChannelKind::Discord => discord_schema(),
            BuiltinChannelKind::Feishu => feishu_schema(),
            BuiltinChannelKind::Weixin => weixin_schema(),
        }
    }

    pub fn auth_flows(self) -> Vec<AuthFlowDescriptor> {
        match self.kind {
            BuiltinChannelKind::Telegram | BuiltinChannelKind::Discord => Vec::new(),
            BuiltinChannelKind::Feishu => vec![AuthFlowDescriptor {
                id: "device_code".into(),
                label: "OAuth device code".into(),
                prompt: "Scan the QR with your phone to authorize the app".into(),
            }],
            BuiltinChannelKind::Weixin => vec![AuthFlowDescriptor {
                id: "qr_code".into(),
                label: "WeChat QR login".into(),
                prompt: "Scan the QR with your WeChat app to authorize".into(),
            }],
        }
    }

    pub fn config_methods(self) -> Vec<ConfigMethod> {
        match self.kind {
            BuiltinChannelKind::Telegram | BuiltinChannelKind::Discord => {
                vec![ConfigMethod::Form]
            }
            BuiltinChannelKind::Feishu | BuiltinChannelKind::Weixin => {
                vec![ConfigMethod::Form, ConfigMethod::AutoLogin]
            }
        }
    }
}

pub fn builtin_channel_descriptors() -> &'static [BuiltinChannelDescriptor] {
    &BUILTIN_CHANNEL_DESCRIPTORS
}

pub fn builtin_channel_descriptor(id: &str) -> Option<BuiltinChannelDescriptor> {
    builtin_channel_descriptors()
        .iter()
        .copied()
        .find(|descriptor| descriptor.id == id || descriptor.aliases.contains(&id))
}

/// Built-in channels all share the same "pull, explicit-ack"
/// delivery profile — no images, no files, no hot-reload yet. The
/// three bits that vary per channel (`outbound` / `inbound` /
/// `streaming`) are passed in.
pub fn builtin_capabilities(
    outbound: bool,
    inbound: bool,
    streaming: bool,
) -> ManifestCapabilities {
    ManifestCapabilities {
        outbound,
        inbound,
        streaming,
        dispatch_stream_event: true,
        images: false,
        files: false,
        hot_reload_accounts: false,
        requires_public_url: false,
        needs_host_ingress: false,
        // Built-in channels share the gateway's process — they never
        // get "respawned" in the subprocess sense, so the field is
        // moot here. Leaving false keeps the default behavior
        // consistent with how the auto-updater queries it (it only
        // ever asks subprocess plugins).
        survives_respawn: false,
        delivery_model: DeliveryModel::PullExplicitAck,
    }
}

pub fn telegram_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["token"],
        "properties": {
            "token": {
                "type": "string",
                "description": "Bot token from @BotFather (format 123:ABC...).",
                "x-garyx": { "secret": true }
            }
        }
    })
}

pub fn discord_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["token"],
        "properties": {
            "token": {
                "type": "string",
                "description": "Discord bot token from the Developer Portal.",
                "x-garyx": { "secret": true }
            },
            "require_mention": {
                "type": "boolean",
                "default": true,
                "description": "Only respond in server channels when the bot is @mentioned."
            }
        }
    })
}

pub fn feishu_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["app_id", "app_secret"],
        "properties": {
            "app_id": {
                "type": "string",
                "description": "Open Platform app id (cli_... / other)."
            },
            "app_secret": {
                "type": "string",
                "description": "Open Platform app secret.",
                "x-garyx": { "secret": true }
            },
            "domain": {
                "type": "string",
                "enum": ["feishu", "lark"],
                "default": "feishu",
                "description": "feishu (国内) or lark (海外)."
            },
            "require_mention": {
                "type": "boolean",
                "default": true,
                "description": "Only respond in groups when the bot is @mentioned."
            },
            "topic_session_mode": {
                "type": "string",
                "enum": ["disabled", "enabled"],
                "default": "disabled",
                "description": "When enabled, group replies are scoped by Feishu topic/thread instead of the whole group."
            },
            "meeting_entities": {
                "type": "boolean",
                "default": true,
                "description": "Capture invited Feishu meetings as durable meeting entities."
            }
        }
    })
}

pub fn weixin_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["token", "uin"],
        "properties": {
            "token": {
                "type": "string",
                "description": "Weixin bot token.",
                "x-garyx": { "secret": true }
            },
            "uin": {
                "type": "string",
                "description": "Weixin account UIN (base64-encoded)."
            },
            "base_url": {
                "type": "string",
                "default": "https://ilinkai.weixin.qq.com",
                "description": "Weixin API base URL."
            },
            "streaming_update": {
                "type": "boolean",
                "default": true,
                "description": "Enable in-place message updates for streamed assistant replies."
            }
        }
    })
}

/// Channel names the built-in outbound senders claim (canonical ids
/// plus aliases). A plugin registering under one of these would be
/// shadowed by built-in routing, so `register_plugin` /
/// `fork_with_plugin_sender` reject them up front. Single source of
/// truth: the dispatcher re-exports this — channel-name string
/// literals must not appear in the dispatcher itself.
pub const RESERVED_CHANNEL_NAMES: &[&str] =
    &["telegram", "discord", "feishu", "lark", "weixin", "wechat"];

/// Construction-layer injection point for the dispatcher's
/// type-erased built-in sender registry. The dispatcher core never
/// names a concrete channel type; adding a built-in channel means
/// registering its wrapper here and implementing
/// [`crate::dispatcher::OutboundChannelSender`] in the channel's own
/// module. The downcast capability stays sealed inside
/// [`crate::outbound_registry::BuiltinSenderRegistry`].
pub(crate) fn builtin_sender_registry() -> crate::outbound_registry::BuiltinSenderRegistry {
    let mut registry = crate::outbound_registry::BuiltinSenderRegistry::new();
    registry.push(crate::telegram::outbound::TelegramChannelSender::default());
    registry.push(crate::discord::outbound::DiscordChannelSender::default());
    registry.push(crate::feishu::outbound::FeishuChannelSender::default());
    registry.push(crate::weixin::outbound::WeixinChannelSender::default());
    registry
}
