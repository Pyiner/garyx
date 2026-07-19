//! B2a characterization tests: pin the CURRENT dispatcher behavior
//! before the outbound-sender-registry inversion (Phase-6 B2b).
//!
//! Conservation oracle contract: every test in this module must pass
//! unchanged on the pre-inversion dispatcher AND on the post-inversion
//! registry. None of these tests may be edited in the B2b change; if
//! the inversion needs a test edited, the inversion changed behavior.
//!
//! Coverage maps 1:1 onto the Phase-6 risk matrix rows:
//! T1 alias routing, T2 error-text stability, T3 weixin_running
//! sharing across fork/hot-swap, T4 per-channel stream-callback
//! construction differences, T5 §9.4 register/unregister/fork,
//! T6 member-set/presence-marker semantics, T7 capability matrix.

use super::*;
use garyx_models::ChannelOutboundContent;
use reqwest::Client;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::duplex;

use crate::plugin_host::{
    CapabilitiesResponse, InboundHandler, PluginSenderHandle, Transport, TransportConfig,
};

fn telegram_sender(account_id: &str) -> TelegramSender {
    TelegramSender {
        account_id: account_id.to_owned(),
        token: "test-token".to_owned(),
        http: Client::new(),
        api_base: "https://api.telegram.org".to_owned(),
        is_running: true,
    }
}

fn discord_sender(account_id: &str) -> DiscordSender {
    DiscordSender {
        account_id: account_id.to_owned(),
        token: "test-token".to_owned(),
        http: Client::new(),
        api_base: "https://discord.com/api/v10".to_owned(),
        is_running: true,
    }
}

fn feishu_sender(account_id: &str) -> FeishuSender {
    FeishuSender::new(
        account_id.to_owned(),
        "app123".to_owned(),
        "secret".to_owned(),
        "https://open.feishu.cn/open-apis".to_owned(),
        true,
    )
}

fn weixin_sender(account_id: &str, running: Arc<AtomicBool>) -> WeixinSender {
    WeixinSender {
        account_id: account_id.to_owned(),
        account: garyx_models::config::WeixinAccount {
            token: "token".to_owned(),
            uin: "MTIz".to_owned(),
            enabled: true,
            base_url: "https://ilinkai.weixin.qq.com".to_owned(),
            name: None,
            agent_id: Some("claude".to_owned()),
            workspace_dir: None,
            streaming_update: true,
        },
        http: Client::new(),
        is_running: true,
        running,
    }
}

fn outbound_for(channel: &str, account_id: &str) -> OutboundMessage {
    OutboundMessage {
        channel: channel.to_owned(),
        account_id: account_id.to_owned(),
        chat_id: "chat-1".to_owned(),
        delivery_target_type: "chat_id".to_owned(),
        delivery_target_id: "chat-1".to_owned(),
        content: ChannelOutboundContent::text("hi"),
        reply_to: None,
        thread_id: None,
    }
}

fn stream_target(channel: &str, account_id: &str, chat_id: &str) -> StreamingDispatchTarget {
    StreamingDispatchTarget {
        target_thread_id: format!("thread::{channel}"),
        endpoint_identity: format!("{channel}::{account_id}::{chat_id}"),
        run_id: "run-char".to_owned(),
        channel: channel.to_owned(),
        account_id: account_id.to_owned(),
        chat_id: chat_id.to_owned(),
        delivery_target_type: "chat_id".to_owned(),
        delivery_target_id: chat_id.to_owned(),
        thread_id: None,
    }
}

fn plugin_handle_with_caps(plugin_id: &str, caps: CapabilitiesResponse) -> PluginSenderHandle {
    struct HostDrop;
    #[async_trait::async_trait]
    impl InboundHandler for HostDrop {
        async fn on_request(&self, _m: String, _p: Value) -> Result<Value, (i32, String)> {
            Err((-32601, "none".into()))
        }
        async fn on_notification(&self, _: String, _: Value) {}
    }
    // The peer side is deliberately left undriven: capability-matrix
    // and membership tests never complete an RPC round trip. Tests
    // that need a live peer use the tests.rs stub fixtures instead.
    let (host_rw, _plugin_rw) = duplex(64 * 1024);
    let (host_r, host_w) = tokio::io::split(host_rw);
    let (host_rpc, _handles) = Transport::spawn(
        host_r,
        host_w,
        TransportConfig {
            plugin_id: plugin_id.to_owned(),
            default_rpc_timeout: Duration::from_secs(2),
            ..Default::default()
        },
        Arc::new(HostDrop),
    );
    PluginSenderHandle::new(plugin_id.to_owned(), host_rpc, caps)
}

fn legacy_caps() -> CapabilitiesResponse {
    CapabilitiesResponse {
        outbound: true,
        inbound: true,
        streaming: false,
        dispatch_stream_event: false,
        images: false,
        files: false,
        survives_respawn: false,
    }
}

fn native_caps() -> CapabilitiesResponse {
    CapabilitiesResponse {
        dispatch_stream_event: true,
        ..legacy_caps()
    }
}

// ---------------------------------------------------------------------------
// T1 — alias routing
// ---------------------------------------------------------------------------

/// `lark` must hit the feishu resolution path and `wechat` the weixin
/// path. Proven network-free: with no sender registered, the error
/// text names the canonical channel family, not "Unknown channel".
#[tokio::test]
async fn aliases_resolve_to_canonical_channel_families() {
    let dispatcher = ChannelDispatcherImpl::new();
    for (alias, expected) in [
        ("lark", "Feishu account 'acct' not registered in dispatcher"),
        (
            "wechat",
            "Weixin account 'acct' not registered in dispatcher",
        ),
    ] {
        let err = dispatcher
            .send_message(outbound_for(alias, "acct"))
            .await
            .expect_err("unregistered account must fail");
        match err {
            ChannelError::Config(msg) => assert_eq!(
                msg, expected,
                "alias `{alias}` must resolve to its canonical family"
            ),
            other => panic!("expected Config for alias {alias}, got {other:?}"),
        }
    }
}

/// `channel_running_handle` must answer for both weixin spellings with
/// the same shared handle.
#[test]
fn wechat_alias_shares_the_weixin_running_handle() {
    let dispatcher = ChannelDispatcherImpl::new();
    let weixin = dispatcher
        .channel_running_handle("weixin")
        .expect("weixin running handle");
    let wechat = dispatcher
        .channel_running_handle("wechat")
        .expect("wechat running handle");
    assert!(
        Arc::ptr_eq(&weixin, &wechat),
        "weixin and wechat must share one AtomicBool"
    );
    assert!(
        dispatcher.channel_running_handle("telegram").is_none(),
        "only weixin exposes a running handle today"
    );
}

// ---------------------------------------------------------------------------
// T2 — error-text stability
// ---------------------------------------------------------------------------

/// Byte-exact unregistered-account error per channel family plus the
/// unknown-channel error. Upstream retry policies classify on
/// Config-vs-Connection, and users see these strings verbatim.
#[tokio::test]
async fn unregistered_account_error_texts_are_stable() {
    let dispatcher = ChannelDispatcherImpl::new();
    for (channel, expected) in [
        (
            "telegram",
            "Telegram account 'acct' not registered in dispatcher",
        ),
        (
            "discord",
            "Discord account 'acct' not registered in dispatcher",
        ),
        (
            "feishu",
            "Feishu account 'acct' not registered in dispatcher",
        ),
        (
            "weixin",
            "Weixin account 'acct' not registered in dispatcher",
        ),
    ] {
        let err = dispatcher
            .send_message(outbound_for(channel, "acct"))
            .await
            .expect_err("unregistered account must fail");
        match err {
            ChannelError::Config(msg) => assert_eq!(msg, expected),
            other => panic!("expected Config for {channel}, got {other:?}"),
        }
    }

    let err = dispatcher
        .send_message(outbound_for("no-such-channel", "acct"))
        .await
        .expect_err("unknown channel must fail");
    match err {
        ChannelError::Config(msg) => {
            assert_eq!(msg, "Unknown channel type: 'no-such-channel'")
        }
        other => panic!("expected Config for unknown channel, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// T3 — weixin_running sharing across construction and fork
// ---------------------------------------------------------------------------

/// The externally supplied weixin running flag must be THE handle the
/// dispatcher exposes, survive a §9.4 fork, and stay write-visible
/// across both sides.
#[tokio::test]
async fn weixin_running_handle_is_shared_across_fork() {
    let external = Arc::new(AtomicBool::new(false));
    let dispatcher = ChannelDispatcherImpl::from_config_with_weixin_running(
        &garyx_models::config::ChannelsConfig::default(),
        external.clone(),
    );
    let exposed = dispatcher
        .channel_running_handle("weixin")
        .expect("running handle");
    assert!(Arc::ptr_eq(&external, &exposed));

    let forked = dispatcher
        .fork_with_plugin_sender(plugin_handle_with_caps("mino-fork", legacy_caps()))
        .expect("fork with plugin");
    let forked_handle = forked
        .channel_running_handle("weixin")
        .expect("forked running handle");
    assert!(
        Arc::ptr_eq(&external, &forked_handle),
        "fork must not clone the weixin running flag into a new allocation"
    );

    external.store(true, Ordering::SeqCst);
    assert!(
        forked_handle.load(Ordering::SeqCst),
        "a flip on the original must be visible through the forked dispatcher"
    );
}

// ---------------------------------------------------------------------------
// T4 — per-channel stream-callback construction differences
// ---------------------------------------------------------------------------

/// Telegram parses the numeric chat id at callback-construction time:
/// non-numeric ids yield None, numeric ids yield Some.
#[tokio::test]
async fn telegram_stream_callback_requires_numeric_chat_id() {
    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_telegram(telegram_sender("main"));
    assert!(
        dispatcher
            .build_stream_event_callback(stream_target("telegram", "main", "12345"))
            .is_some(),
        "numeric chat id must construct"
    );
    assert!(
        dispatcher
            .build_stream_event_callback(stream_target("telegram", "main", "not-a-number"))
            .is_none(),
        "non-numeric chat id must refuse construction"
    );
}

/// Every built-in family: a registered account constructs a callback,
/// a missing account yields None (account resolution happens at
/// construction, not first send).
#[tokio::test]
async fn builtin_stream_callbacks_resolve_accounts_at_construction() {
    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_telegram(telegram_sender("main"));
    dispatcher.register_discord(discord_sender("main"));
    dispatcher.register_feishu(feishu_sender("main"));
    dispatcher.register_weixin(weixin_sender("main", Arc::new(AtomicBool::new(true))));

    for channel in ["telegram", "discord", "feishu", "weixin"] {
        let chat = if channel == "telegram" {
            "777"
        } else {
            "chat-1"
        };
        assert!(
            dispatcher
                .build_stream_event_callback(stream_target(channel, "main", chat))
                .is_some(),
            "{channel}: registered account must construct"
        );
        assert!(
            dispatcher
                .build_stream_event_callback(stream_target(channel, "ghost", chat))
                .is_none(),
            "{channel}: missing account must yield None"
        );
    }
}

// ---------------------------------------------------------------------------
// T5 — §9.4 register / unregister / fork membership semantics
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fork_adds_plugin_without_disturbing_existing_members() {
    let mut base = ChannelDispatcherImpl::new();
    base.register_telegram(telegram_sender("tg-main"));
    base.register_plugin(plugin_handle_with_caps("mino", legacy_caps()))
        .expect("register mino");

    let forked = base
        .fork_with_plugin_sender(plugin_handle_with_caps("second", legacy_caps()))
        .expect("fork with second plugin");

    let names = |d: &ChannelDispatcherImpl| {
        d.available_channels()
            .into_iter()
            .map(|c| c.channel)
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&base), vec!["mino", "telegram"]);
    assert_eq!(names(&forked), vec!["mino", "second", "telegram"]);
}

#[tokio::test]
async fn fork_replaces_same_id_plugin_handle() {
    let mut base = ChannelDispatcherImpl::new();
    base.register_plugin(plugin_handle_with_caps("mino", legacy_caps()))
        .expect("register mino");
    // §9.4 respawn: forking with the SAME plugin id swaps the handle
    // (spawn NEW + drain OLD), it must not error or duplicate.
    let forked = base
        .fork_with_plugin_sender(plugin_handle_with_caps("mino", native_caps()))
        .expect("fork with respawned handle");
    let members = forked
        .available_channels()
        .into_iter()
        .filter(|c| c.channel == "mino")
        .count();
    assert_eq!(members, 1, "respawn fork must replace, not duplicate");
    // The replacement handle's capabilities are live on the fork.
    assert!(
        forked
            .plugin_sender("mino")
            .expect("mino sender")
            .capabilities()
            .dispatch_stream_event,
        "fork must carry the NEW handle's capabilities"
    );
}

#[tokio::test]
async fn unregister_plugin_returns_the_handle_once() {
    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher
        .register_plugin(plugin_handle_with_caps("mino", legacy_caps()))
        .expect("register");
    let first = dispatcher.unregister_plugin("mino");
    assert_eq!(
        first.map(|handle| handle.plugin_id().to_owned()),
        Some("mino".to_owned())
    );
    assert!(
        dispatcher.unregister_plugin("mino").is_none(),
        "second unregister must be None"
    );
    assert!(dispatcher.available_channels().is_empty());
}

// ---------------------------------------------------------------------------
// T6 — member-set and plugin presence marker
// ---------------------------------------------------------------------------

/// The full-member snapshot: per-account rows for built-ins, one
/// presence-marker row per plugin (empty account_id, is_running=true),
/// sorted by (channel, account_id).
#[tokio::test]
async fn available_channels_member_set_and_presence_markers() {
    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_telegram(telegram_sender("tg-b"));
    dispatcher.register_telegram(telegram_sender("tg-a"));
    dispatcher.register_feishu(feishu_sender("fs-main"));
    dispatcher
        .register_plugin(plugin_handle_with_caps("mino", legacy_caps()))
        .expect("register plugin");

    let members = dispatcher
        .available_channels()
        .into_iter()
        .map(|c| (c.channel, c.account_id, c.is_running))
        .collect::<Vec<_>>();
    assert_eq!(
        members,
        vec![
            ("feishu".to_owned(), "fs-main".to_owned(), true),
            ("mino".to_owned(), String::new(), true),
            ("telegram".to_owned(), "tg-a".to_owned(), true),
            ("telegram".to_owned(), "tg-b".to_owned(), true),
        ],
        "member set, presence marker shape, and sort order must hold"
    );
}

// ---------------------------------------------------------------------------
// T7 — capability matrix
// ---------------------------------------------------------------------------

/// The full capability matrix across plugin generations and built-ins:
///
/// | sender                        | stream callback | legacy adapter |
/// |-------------------------------|-----------------|----------------|
/// | plugin outbound-only (legacy) | None            | true           |
/// | plugin dispatch_stream_event  | Some            | false          |
/// | built-in, account present     | Some            | false          |
/// | built-in, account missing     | None            | false          |
/// | unknown channel               | None            | false          |
#[tokio::test]
async fn capability_matrix_selects_exactly_one_streaming_path() {
    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_telegram(telegram_sender("main"));
    dispatcher
        .register_plugin(plugin_handle_with_caps("legacy-plug", legacy_caps()))
        .expect("register legacy");
    dispatcher
        .register_plugin(plugin_handle_with_caps("native-plug", native_caps()))
        .expect("register native");

    let cases: [(&str, &str, &str, bool, bool); 5] = [
        ("legacy-plug", "acct", "chat-1", false, true),
        ("native-plug", "acct", "chat-1", true, false),
        ("telegram", "main", "777", true, false),
        ("telegram", "ghost", "777", false, false),
        ("no-such", "acct", "chat-1", false, false),
    ];
    for (channel, account, chat, expect_callback, expect_legacy) in cases {
        let target = stream_target(channel, account, chat);
        assert_eq!(
            dispatcher
                .build_stream_event_callback(target.clone())
                .is_some(),
            expect_callback,
            "{channel}: native callback presence"
        );
        assert_eq!(
            dispatcher.supports_legacy_stream_adapter(&target),
            expect_legacy,
            "{channel}: legacy adapter selection"
        );
    }
}

/// The composed entry point must follow the same matrix: native
/// callback wins, legacy adapter is the fallback, and a channel with
/// neither yields no streaming at all.
#[tokio::test]
async fn build_stream_dispatch_callback_composes_the_matrix() {
    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_telegram(telegram_sender("main"));
    dispatcher
        .register_plugin(plugin_handle_with_caps("legacy-plug", legacy_caps()))
        .expect("register legacy");
    let dispatcher: Arc<dyn ChannelDispatcher> = Arc::new(dispatcher);

    for (channel, account, chat, expect_some) in [
        ("legacy-plug", "acct", "chat-1", true), // legacy adapter path
        ("telegram", "main", "777", true),       // native built-in path
        ("telegram", "ghost", "777", false),     // account miss: nothing
        ("no-such", "acct", "chat-1", false),    // unknown: nothing
    ] {
        let callback = build_stream_dispatch_callback(
            dispatcher.clone(),
            stream_target(channel, account, chat),
            StreamDispatchRole::Origin,
        );
        assert_eq!(
            callback.is_some(),
            expect_some,
            "{channel}: composed streaming selection"
        );
    }
}

/// Phase-6 B2 deliberate unification (a DECLARED behavior change, not
/// conservation): alias spellings now resolve streaming callbacks
/// exactly like their canonical ids. Pre-B2, `send_message` accepted
/// `lark`/`wechat` but `build_stream_event_callback` silently returned
/// `None` for them, so a persisted binding carrying an alias channel
/// value got no native streaming. The registry routes every surface
/// through the same alias-aware lookup; `supports_legacy` stays false
/// for built-ins under both spellings.
#[tokio::test]
async fn aliases_resolve_streaming_callbacks_like_canonical_ids() {
    let mut dispatcher = ChannelDispatcherImpl::new();
    dispatcher.register_feishu(feishu_sender("main"));
    dispatcher.register_weixin(weixin_sender("main", Arc::new(AtomicBool::new(true))));
    for (alias, canonical) in [("lark", "feishu"), ("wechat", "weixin")] {
        assert!(
            dispatcher
                .build_stream_event_callback(stream_target(canonical, "main", "chat-1"))
                .is_some(),
            "{canonical}: canonical id must stream"
        );
        assert!(
            dispatcher
                .build_stream_event_callback(stream_target(alias, "main", "chat-1"))
                .is_some(),
            "{alias} must stream like {canonical} (B2 alias unification)"
        );
        assert!(
            !dispatcher.supports_legacy_stream_adapter(&stream_target(alias, "main", "chat-1")),
            "{alias}: built-ins never use the legacy adapter"
        );
    }
}

// ---------------------------------------------------------------------------
// B2b source guard
// ---------------------------------------------------------------------------

/// Strip line and (nested) block comments while KEEPING string
/// literals, so the guard below can detect channel-name string
/// literals in real code without tripping on doc comments.
fn strip_comments_keeping_strings(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            let mut depth = 1;
            i += 2;
            while i < bytes.len() && depth > 0 {
                if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if bytes[i] == b'"' {
            out.push('"');
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    out.push(bytes[i] as char);
                    out.push(bytes[i + 1] as char);
                    i += 2;
                    continue;
                }
                out.push(bytes[i] as char);
                if bytes[i] == b'"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// B2b acceptance guard: the production dispatcher is channel-blind —
/// no built-in channel name may appear as a STRING LITERAL in
/// dispatcher.rs code (match arms, error text, running-handle keys).
/// Channel-name strings live only in
/// `builtin_catalog::RESERVED_CHANNEL_NAMES` and each channel's own
/// module (`channel_id()` / `aliases()` in `<channel>/outbound.rs`).
/// Typed composition (`TelegramChannelSender` fields, re-export
/// paths) is legitimate and deliberately not restricted.
#[test]
fn dispatcher_contains_no_builtin_channel_name_literals() {
    let production = include_str!("../dispatcher.rs");
    let code_with_strings = strip_comments_keeping_strings(production);
    for name in crate::builtin_catalog::RESERVED_CHANNEL_NAMES {
        let quoted = format!("\"{name}\"");
        assert!(
            !code_with_strings.contains(&quoted),
            "dispatcher.rs must be channel-blind; found channel-name string literal {quoted} in code"
        );
    }
}

/// The stripper must remove comments but keep string literals, or the
/// guard above silently degrades in either direction.
#[test]
fn b2b_guard_stripper_strips_comments_and_keeps_strings() {
    let source = "// \"telegram\" in comment\n/* \"weixin\" */\nlet a = \"feishu\";";
    let stripped = strip_comments_keeping_strings(source);
    assert!(!stripped.contains("telegram"), "comments must be stripped");
    assert!(
        !stripped.contains("weixin"),
        "block comments must be stripped"
    );
    assert!(
        stripped.contains("\"feishu\""),
        "string literals must be kept"
    );
}

/// Structural seal (round-2 review): the downcast capability lives
/// only in `outbound_registry` (private trait + private field), and
/// `OutboundChannelSender` carries no `Any`/clone-box surface — so a
/// `&dyn OutboundChannelSender` obtained from `route()` cannot be
/// turned back into a concrete channel type, and typed access
/// (`with_mut`) requires `&mut self`, inexpressible in `&self`
/// dispatch paths. This lexical check is only the tripwire on top of
/// that privacy boundary: dispatcher.rs must contain no downcast
/// vocabulary at all.
#[test]
fn dispatcher_has_no_downcast_vocabulary() {
    let production = include_str!("../dispatcher.rs");
    let stripped = strip_comments_keeping_strings(production);
    for forbidden in [
        "as_any",
        "downcast",
        "builtin_ref",
        "builtin_mut",
        "with_mut",
        "register::<",
    ] {
        assert!(
            !stripped.contains(forbidden),
            "dispatcher.rs must not carry downcast vocabulary; found `{forbidden}`"
        );
    }
}
