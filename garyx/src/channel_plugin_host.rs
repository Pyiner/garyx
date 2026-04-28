//! Discovery and wiring for subprocess-backed channel plugins.
//!
//! Responsibilities:
//! - Scan the manifest directories (`GARYX_PLUGIN_DIR` + the default
//!   `$data_dir/plugins/`) via
//!   [`garyx_channels::plugin_host::ManifestDiscoverer`].
//! - Run per-manifest `preflight`; anything that fails is logged and
//!   skipped rather than aborting the whole boot.
//! - For each healthy manifest, build the `HostContext`, translate
//!   config into [`AccountDescriptor`]s, and call
//!   `ChannelPluginManager::register_subprocess_plugin`.
//!
//! The inbound handler routes `deliver_inbound` reverse-RPCs through
//! [`MessageRouter`]: assemble an [`InboundRequest`], await
//! `route_and_dispatch` with a response callback that accumulates
//! streamed text and, on `Done`, posts it back out through the
//! plugin's own `dispatch_outbound` via the [`ChannelDispatcher`].
//! `record_outbound` is a no-op for now — the stream-driven
//! callback already handles the write-path bookkeeping. `abandon_inbound`
//! and streaming notifications are still stubbed; they come in
//! follow-ups once a plugin actually needs them.

use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use garyx_bridge::MultiProviderBridge;
use garyx_channels::dispatcher::{ChannelDispatcher, OutboundMessage, SwappableDispatcher};
use garyx_channels::plugin::ChannelPluginManager;
use garyx_channels::plugin_host::{
    AccountDescriptor, AttachmentRef, HostContext, InboundHandler, ManifestDiscoverer,
    PluginErrorCode, PluginManifest, SpawnOptions, StreamId, StreamIdGenerator, StreamRegistry,
    TombstoneReason, preflight,
};
use garyx_models::command_catalog::{CommandCatalogOptions, CommandSurface};
use garyx_models::config::GaryxConfig;
use garyx_models::local_paths::{default_session_data_dir, gary_home_dir};
use garyx_models::provider::{
    ATTACHMENTS_METADATA_KEY, ImagePayload, PromptAttachment, PromptAttachmentKind,
    StreamBoundaryKind, StreamEvent, attachments_from_metadata, attachments_to_metadata_value,
};
use garyx_router::{InboundRequest, MessageRouter};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, info, warn};

/// Host-side inbound handler for subprocess channel plugins.
///
/// Each registered subprocess plugin gets its own handler instance
/// (created in [`register_one_manifest`]) so the plugin-id used for
/// reverse-dispatch stays correct.
pub struct HostInboundHandler {
    plugin_id: String,
    router: Arc<Mutex<MessageRouter>>,
    bridge: Arc<MultiProviderBridge>,
    /// Concrete dispatcher handle. Used both for the request-shaped
    /// `send_message` outbound path (consolidated Done-time reply)
    /// and for the notification-shaped `inbound/stream_frame` /
    /// `inbound/stream_end` frames (§7.1 streaming).
    swap: Arc<SwappableDispatcher>,
    /// Host-side stream id allocator. Plugin gets a fresh id per
    /// `deliver_inbound`.
    stream_ids: StreamIdGenerator,
    /// Streams tombstoned via `abandon_inbound`. The callback checks
    /// this set before emitting any frame / forwarding outbound;
    /// once tombstoned, the stream stays silent even if the agent
    /// run continues in the background.
    streams: Arc<StreamRegistry>,
    /// Tracks currently-active streams so `abandon_inbound` knows
    /// which ids correspond to live agent runs for this plugin.
    /// Agent runs the host can't cancel (no cancel token in
    /// `route_and_dispatch` today) still finish in the background;
    /// their output is silently dropped once the id tombstones.
    live_streams: Arc<StdMutex<HashSet<String>>>,
}

impl HostInboundHandler {
    pub fn new(
        plugin_id: String,
        router: Arc<Mutex<MessageRouter>>,
        bridge: Arc<MultiProviderBridge>,
        swap: Arc<SwappableDispatcher>,
    ) -> Self {
        Self {
            plugin_id,
            router,
            bridge,
            swap,
            stream_ids: StreamIdGenerator::new(),
            streams: Arc::new(StreamRegistry::new()),
            live_streams: Arc::new(StdMutex::new(HashSet::new())),
        }
    }
}

/// Mirror of `plugin_host::protocol::InboundRequestPayload` — kept as
/// a local shape so we only pull in what the handler needs. The wire
/// shape is `{ account_id, from_id, is_group, thread_binding_key,
/// message, run_id, reply_to_message_id?, images[], file_paths[],
/// extra_metadata{} }`.
#[derive(Debug, Deserialize)]
struct DeliverInboundParams {
    account_id: String,
    #[serde(default)]
    from_id: String,
    #[serde(default)]
    is_group: bool,
    thread_binding_key: String,
    message: String,
    #[serde(default)]
    run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reply_to_message_id: Option<String>,
    #[serde(default)]
    images: Vec<AttachmentRef>,
    #[serde(default)]
    file_paths: Vec<String>,
    #[serde(default)]
    extra_metadata: std::collections::HashMap<String, Value>,
}

#[derive(Debug, Deserialize)]
struct CommandsListParams {
    #[serde(default)]
    account_id: Option<String>,
    #[serde(default)]
    surface: Option<CommandSurface>,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    include_hidden: bool,
}

#[async_trait]
impl InboundHandler for HostInboundHandler {
    async fn on_request(&self, method: String, params: Value) -> Result<Value, (i32, String)> {
        match method.as_str() {
            "commands/list" => self.handle_commands_list(params).await,
            "deliver_inbound" => self.handle_deliver_inbound(params).await,
            "record_outbound" => {
                // The `route_and_dispatch` response callback already
                // persists outbound replies it emits, so an explicit
                // record from the plugin is redundant in the common
                // case. Accept as a no-op rather than rejecting —
                // plugins that track this bookkeeping externally
                // still round-trip cleanly.
                Ok(Value::Object(Default::default()))
            }
            "abandon_inbound" => self.handle_abandon_inbound(params),
            other => Err((
                PluginErrorCode::MethodNotFound.as_i32(),
                format!("unknown host method: {other}"),
            )),
        }
    }

    async fn on_notification(&self, method: String, _params: Value) {
        if method.starts_with("inbound/") {
            debug!(
                plugin_id = %self.plugin_id,
                method = %method,
                "plugin streaming notification dropped (host does not consume plugin-driven streams today)"
            );
        }
    }
}

/// Wire shape of `abandon_inbound` params (§7.3).
#[derive(Debug, Deserialize)]
struct AbandonInboundParams {
    stream_id: String,
    #[serde(default)]
    reason: String,
}

impl HostInboundHandler {
    async fn handle_commands_list(&self, params: Value) -> Result<Value, (i32, String)> {
        let parsed: CommandsListParams = serde_json::from_value(params).map_err(|err| {
            (
                PluginErrorCode::InvalidParams.as_i32(),
                format!("commands/list params: {err}"),
            )
        })?;
        let surface = parsed.surface.or(Some(CommandSurface::Plugin));
        let channel = parsed.channel.or_else(|| {
            if matches!(surface.as_ref(), Some(CommandSurface::Plugin)) {
                Some(self.plugin_id.clone())
            } else {
                None
            }
        });
        let options = CommandCatalogOptions {
            surface,
            channel,
            account_id: parsed.account_id,
            include_hidden: parsed.include_hidden,
        };
        let router = self.router.lock().await;
        serde_json::to_value(router.command_catalog(options)).map_err(|err| {
            (
                PluginErrorCode::InternalError.as_i32(),
                format!("commands/list response: {err}"),
            )
        })
    }

    fn merge_inbound_image_refs(
        images: &[AttachmentRef],
        extra_metadata: &mut std::collections::HashMap<String, Value>,
    ) -> Vec<ImagePayload> {
        let mut inline_images = Vec::new();
        let mut prompt_attachments = attachments_from_metadata(extra_metadata);

        for image in images {
            match image {
                AttachmentRef::Inline { data, media_type } => {
                    if data.trim().is_empty() {
                        continue;
                    }
                    inline_images.push(ImagePayload {
                        name: String::new(),
                        data: data.clone(),
                        media_type: media_type.clone(),
                    });
                }
                AttachmentRef::Path { path, media_type } => {
                    let trimmed = path.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    prompt_attachments.push(PromptAttachment {
                        kind: PromptAttachmentKind::Image,
                        path: trimmed.to_owned(),
                        name: Path::new(trimmed)
                            .file_name()
                            .and_then(|value| value.to_str())
                            .map(ToOwned::to_owned)
                            .unwrap_or_else(|| trimmed.to_owned()),
                        media_type: media_type.clone(),
                    });
                }
            }
        }

        if !prompt_attachments.is_empty() {
            extra_metadata.insert(
                ATTACHMENTS_METADATA_KEY.to_owned(),
                attachments_to_metadata_value(&prompt_attachments),
            );
        }

        inline_images
    }

    async fn handle_deliver_inbound(&self, params: Value) -> Result<Value, (i32, String)> {
        let parsed: DeliverInboundParams = serde_json::from_value(params).map_err(|err| {
            (
                PluginErrorCode::InvalidParams.as_i32(),
                format!("deliver_inbound params: {err}"),
            )
        })?;

        let mut extra_metadata: std::collections::HashMap<String, Value> =
            parsed.extra_metadata.into_iter().collect();
        let inline_images = Self::merge_inbound_image_refs(&parsed.images, &mut extra_metadata);
        let has_prompt_attachments = !attachments_from_metadata(&extra_metadata).is_empty();
        if parsed.message.trim().is_empty()
            && inline_images.is_empty()
            && parsed.file_paths.is_empty()
            && !has_prompt_attachments
        {
            return Err((
                PluginErrorCode::InvalidParams.as_i32(),
                "deliver_inbound: empty message and no attachments".into(),
            ));
        }

        let stream_id_typed = self.stream_ids.next();
        let stream_id = stream_id_typed.as_str().to_owned();

        // Register the stream as live so a concurrent
        // `abandon_inbound` call with this id tombstones it
        // correctly. We un-register (best effort) after
        // `route_and_dispatch` resolves; if the agent run was
        // abandoned mid-flight the tombstone remains to swallow any
        // late callback events.
        if let Ok(mut guard) = self.live_streams.lock() {
            guard.insert(stream_id.clone());
        }

        let thread_holder: Arc<StdMutex<Option<String>>> = Arc::new(StdMutex::new(None));

        let response_callback = build_response_callback(StreamCallbackCtx {
            plugin_id: self.plugin_id.clone(),
            account_id: parsed.account_id.clone(),
            chat_id: parsed.thread_binding_key.clone(),
            stream_id: stream_id.clone(),
            swap: self.swap.clone(),
            streams: self.streams.clone(),
            thread_holder: thread_holder.clone(),
        });

        let inbound_request = InboundRequest {
            channel: self.plugin_id.clone(),
            account_id: parsed.account_id.clone(),
            from_id: if parsed.from_id.is_empty() {
                parsed.thread_binding_key.clone()
            } else {
                parsed.from_id
            },
            is_group: parsed.is_group,
            thread_binding_key: parsed.thread_binding_key.clone(),
            message: parsed.message,
            run_id: parsed.run_id,
            reply_to_message_id: parsed.reply_to_message_id,
            images: inline_images,
            extra_metadata,
            file_paths: parsed.file_paths,
        };

        // `route_and_dispatch` resolves the thread and kicks off the
        // agent run. The callback streams events back in; we pin the
        // thread id so the Done-handler can tag its outbound back
        // through the dispatcher with the right chat.
        let result = {
            let mut router = self.router.lock().await;
            router
                .route_and_dispatch(
                    inbound_request,
                    self.bridge.as_ref(),
                    Some(response_callback),
                )
                .await
        };

        // Clear the live-stream entry regardless of outcome. The
        // tombstone (if any) stays and continues to gate the
        // callback, which is the correct behaviour for background
        // agent runs that outlive this call.
        if let Ok(mut guard) = self.live_streams.lock() {
            guard.remove(&stream_id);
        }

        let result = result.map_err(|err| (PluginErrorCode::InternalError.as_i32(), err))?;

        if let Ok(mut holder) = thread_holder.lock() {
            *holder = Some(result.thread_id.clone());
        }

        // If route_and_dispatch produced a synchronous `local_reply`,
        // send it directly through the plugin's outbound path.
        // Respect tombstones: if the plugin abandoned this stream
        // before we finished, drop the reply on the floor.
        if let Some(text) = result.local_reply.as_deref()
            && !text.trim().is_empty()
            && !self.streams.is_tombstoned(&stream_id_typed)
        {
            let outbound = OutboundMessage {
                channel: self.plugin_id.clone(),
                account_id: parsed.account_id.clone(),
                chat_id: parsed.thread_binding_key.clone(),
                delivery_target_type: "chat_id".into(),
                delivery_target_id: parsed.thread_binding_key.clone(),
                text: text.to_owned(),
                reply_to: None,
                thread_id: Some(result.thread_id.clone()),
            };
            if let Err(err) = self.swap.send_message(outbound).await {
                warn!(
                    plugin_id = %self.plugin_id,
                    error = %err,
                    "failed to dispatch local_reply through plugin"
                );
            }
        }

        Ok(json!({
            "stream_id": stream_id,
            "thread_id": result.thread_id,
            "local_reply": Value::Null,
        }))
    }

    fn handle_abandon_inbound(&self, params: Value) -> Result<Value, (i32, String)> {
        let parsed: AbandonInboundParams = serde_json::from_value(params).map_err(|err| {
            (
                PluginErrorCode::InvalidParams.as_i32(),
                format!("abandon_inbound params: {err}"),
            )
        })?;
        let id = StreamId::from(parsed.stream_id.clone());
        let fresh = self.streams.tombstone(&id, TombstoneReason::Abandoned);
        // Diagnostic: note whether the stream id was even known to us.
        // Unknown ids are tombstoned anyway (the plugin may have
        // observed the id before we finished registering; idempotent),
        // but a warn here helps catch id-echo bugs in plugins.
        let is_known = self
            .live_streams
            .lock()
            .map(|g| g.contains(&parsed.stream_id))
            .unwrap_or(false);
        if !is_known && !fresh {
            debug!(
                plugin_id = %self.plugin_id,
                stream_id = %parsed.stream_id,
                reason = %parsed.reason,
                "abandon_inbound for an id we already tombstoned or never issued"
            );
        } else {
            info!(
                plugin_id = %self.plugin_id,
                stream_id = %parsed.stream_id,
                reason = %parsed.reason,
                "stream abandoned by plugin; further frames will be dropped"
            );
        }
        Ok(json!({ "ok": true }))
    }
}

/// Context passed into [`build_response_callback`]. Grouped so the
/// constructor's signature doesn't balloon past clippy's arg cap and
/// so a new callback field doesn't ripple through every callsite.
struct StreamCallbackCtx {
    plugin_id: String,
    account_id: String,
    chat_id: String,
    stream_id: String,
    swap: Arc<SwappableDispatcher>,
    streams: Arc<StreamRegistry>,
    thread_holder: Arc<StdMutex<Option<String>>>,
}

/// Build the stream callback that does TWO things on every agent
/// event:
///
/// 1. **§7.1 streaming.** Emit `inbound/stream_frame` notifications
///    (one per `Delta`, one per `Boundary`) to the plugin's
///    transport, monotonically numbered with `seq`. On
///    `StreamEvent::Done` emit `inbound/stream_end`. This lets
///    streaming-aware plugins drive a real-time UI; batch-upstream
///    plugins just ignore these notifications.
/// 2. **Consolidated Done-time publish.** Accumulate text deltas
///    and, when the stream completes cleanly, dispatch the full
///    assembled reply back through the plugin's `dispatch_outbound`.
///    This is what publish-one-message upstreams expect.
///
/// Both paths respect the tombstone set: once the plugin has called
/// `abandon_inbound(stream_id, …)` the callback drops every
/// subsequent emit, streaming notification AND consolidated publish.
fn build_response_callback(ctx: StreamCallbackCtx) -> Arc<dyn Fn(StreamEvent) + Send + Sync> {
    let (tx, mut rx) = mpsc::unbounded_channel::<StreamEvent>();
    let StreamCallbackCtx {
        plugin_id,
        account_id,
        chat_id,
        stream_id,
        swap,
        streams,
        thread_holder,
    } = ctx;

    tokio::spawn(async move {
        let mut accumulated = String::new();
        let seq = AtomicU64::new(0);
        let typed_id = StreamId::from(stream_id.clone());
        let sender = swap.plugin_sender(&plugin_id);
        while let Some(event) = rx.recv().await {
            // Fast exit once the plugin abandons the stream.
            if streams.is_tombstoned(&typed_id) {
                continue;
            }
            match event {
                StreamEvent::Delta { ref text } => {
                    accumulated.push_str(text);
                    if let Some(sender) = sender.as_ref() {
                        let params = json!({
                            "stream_id": stream_id,
                            "seq": seq.fetch_add(1, Ordering::Relaxed),
                            "event": {
                                "type": "delta",
                                "text": text,
                            },
                        });
                        if let Err(err) = sender.notify("inbound/stream_frame", &params).await {
                            debug!(
                                plugin_id = %plugin_id,
                                stream_id = %stream_id,
                                error = %err,
                                "inbound/stream_frame notify failed; continuing"
                            );
                        }
                    }
                }
                StreamEvent::Boundary { kind, .. } => {
                    if matches!(kind, StreamBoundaryKind::UserAck) {
                        accumulated.clear();
                    }
                    if let Some(sender) = sender.as_ref() {
                        let params = json!({
                            "stream_id": stream_id,
                            "seq": seq.fetch_add(1, Ordering::Relaxed),
                            "event": {
                                "type": "boundary",
                                "kind": format!("{kind:?}").to_lowercase(),
                            },
                        });
                        let _ = sender.notify("inbound/stream_frame", &params).await;
                    }
                }
                StreamEvent::Done => {
                    let text = accumulated.trim().to_owned();
                    accumulated.clear();
                    let thread_id = thread_holder
                        .lock()
                        .ok()
                        .and_then(|g| g.clone())
                        .unwrap_or_default();

                    // Emit the spec-shaped stream terminal before any
                    // consolidated outbound so the plugin can close
                    // whatever streaming UI it was driving.
                    if let Some(sender) = sender.as_ref() {
                        let params = json!({
                            "stream_id": stream_id,
                            "seq": seq.fetch_add(1, Ordering::Relaxed),
                            "status": "ok",
                            "thread_id": thread_id,
                            "final_text": text,
                        });
                        let _ = sender.notify("inbound/stream_end", &params).await;
                    }

                    if text.is_empty() {
                        continue;
                    }
                    let outbound = OutboundMessage {
                        channel: plugin_id.clone(),
                        account_id: account_id.clone(),
                        chat_id: chat_id.clone(),
                        delivery_target_type: "chat_id".into(),
                        delivery_target_id: chat_id.clone(),
                        text,
                        reply_to: None,
                        thread_id: if thread_id.is_empty() {
                            None
                        } else {
                            Some(thread_id)
                        },
                    };
                    if let Err(err) = swap.send_message(outbound).await {
                        warn!(
                            plugin_id = %plugin_id,
                            error = %err,
                            "failed to forward agent reply to plugin"
                        );
                    }
                }
                _ => {}
            }
        }
    });
    Arc::new(move |event: StreamEvent| {
        let _ = tx.send(event);
    })
}

/// Roots scanned for `plugin.toml` files.
///
/// Two sources, union'd: the `GARYX_PLUGIN_DIR` PATH-style env var
/// and `~/.garyx/plugins/`. The default root sits at the top of the
/// garyx home dir (NOT nested under `data/`) so the install path
/// stays short: `garyx plugins install ./my-plugin` drops into
/// `~/.garyx/plugins/<id>/`, one level from the user's eye. Missing
/// roots are tolerated silently so a fresh host install without any
/// plugins is not a boot error.
pub fn plugin_root_paths(_config: &GaryxConfig) -> Vec<PathBuf> {
    let default_root = default_plugin_install_root();

    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(raw) = std::env::var_os("GARYX_PLUGIN_DIR") {
        for part in std::env::split_paths(&raw) {
            if !part.as_os_str().is_empty() {
                roots.push(part);
            }
        }
    }
    roots.push(default_root);
    roots
}

/// The canonical install destination for `garyx plugins install`.
/// One source of truth for both the discovery side (above) and the
/// install-CLI side.
pub fn default_plugin_install_root() -> PathBuf {
    gary_home_dir().join("plugins")
}

/// Per-plugin account set discovered from the channels config.
///
/// Every subprocess plugin's accounts live under
/// `channels.<plugin_id>.accounts`; the `config` field on each
/// entry is forwarded verbatim as the `AccountDescriptor.config` the
/// plugin's `initialize` / `accounts/reload` handlers consume.
/// Unknown plugin ids → empty list (plugin boots with nothing to
/// serve, which is the right fallback for a plugin whose config has
/// not been written yet).
fn accounts_for_plugin(config: &GaryxConfig, plugin_id: &str) -> Vec<AccountDescriptor> {
    config
        .channels
        .plugins
        .get(plugin_id)
        .map(|plugin_cfg| {
            plugin_cfg
                .accounts
                .iter()
                .map(|(id, entry)| AccountDescriptor {
                    id: id.clone(),
                    enabled: entry.enabled,
                    config: entry.config.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Discover all `plugin.toml`s, preflight each, and register the
/// healthy ones into `manager`. Registration failures are logged and
/// skipped — a misbehaving external plugin must not prevent the host
/// from starting. Returns the count of plugins successfully registered.
/// Bundle of host-side dependencies the inbound handler needs. Kept
/// as a single struct so `register_manifest_plugins` /
/// `register_one_manifest` don't balloon past clippy's argument-count
/// cap, and so the manifest loop can `.clone()` cheaply per plugin.
///
/// `swap` carries the concrete [`SwappableDispatcher`] rather than a
/// `Arc<dyn ChannelDispatcher>` because the inbound handler needs two
/// things only the concrete type exposes: (a) snapshot-then-lookup
/// of a specific plugin's `PluginSenderHandle` for §7.1 streaming
/// notifications, and (b) continued identity across hot-reloads so
/// respawn-staged forks still flow through.
#[derive(Clone)]
pub struct HostDeps {
    pub router: Arc<Mutex<MessageRouter>>,
    pub bridge: Arc<MultiProviderBridge>,
    pub swap: Arc<SwappableDispatcher>,
}

pub async fn register_manifest_plugins(
    manager: &Mutex<ChannelPluginManager>,
    config: &GaryxConfig,
    host_version: &str,
    deps: HostDeps,
) -> usize {
    let roots = plugin_root_paths(config);
    let discoverer = ManifestDiscoverer::new(roots);
    let outcome = match discoverer.discover() {
        Ok(outcome) => outcome,
        Err(err) => {
            warn!(error = %err, "plugin manifest discovery failed; skipping subprocess plugins");
            return 0;
        }
    };

    for err in &outcome.errors {
        warn!(error = %err, "plugin manifest failed to parse; skipping");
    }

    if outcome.plugins.is_empty() {
        return 0;
    }

    let data_dir_str = config
        .sessions
        .data_dir
        .clone()
        .unwrap_or_else(|| default_session_data_dir().to_string_lossy().into_owned());
    let host_ctx = HostContext {
        version: host_version.to_owned(),
        public_url: config.gateway.public_url.clone(),
        data_dir: data_dir_str,
        locale: None,
    };

    let mut registered = 0usize;
    for manifest in outcome.plugins {
        if register_one_manifest(
            manager,
            manifest,
            &host_ctx,
            host_version,
            config,
            deps.clone(),
        )
        .await
        .is_ok()
        {
            registered += 1;
        }
    }

    if registered > 0 {
        info!(count = registered, "subprocess channel plugins registered");
    }
    registered
}

async fn register_one_manifest(
    manager: &Mutex<ChannelPluginManager>,
    manifest: PluginManifest,
    host_ctx: &HostContext,
    host_version: &str,
    config: &GaryxConfig,
    deps: HostDeps,
) -> Result<(), ()> {
    let plugin_id = manifest.plugin.id.clone();

    // Preflight first so a misconfigured manifest never spawns a real
    // lifecycle child. `data_dir` and `public_url` mirror what the
    // live-lifecycle path passes via HostContext.
    match preflight(
        &manifest,
        host_version,
        &host_ctx.data_dir,
        &host_ctx.public_url,
    )
    .await
    {
        Ok(summary) => {
            info!(
                plugin_id = %summary.id,
                version = %summary.version,
                "plugin passed preflight"
            );
        }
        Err(err) => {
            warn!(
                plugin_id = %plugin_id,
                error = %err,
                "plugin failed preflight; skipping registration"
            );
            return Err(());
        }
    }

    let accounts = accounts_for_plugin(config, &plugin_id);
    let handler = Arc::new(HostInboundHandler::new(
        plugin_id.clone(),
        deps.router,
        deps.bridge,
        deps.swap,
    ));

    let mut guard = manager.lock().await;
    if let Err(err) = guard
        .register_subprocess_plugin(
            manifest,
            SpawnOptions::default(),
            host_ctx.clone(),
            accounts,
            handler,
        )
        .await
    {
        warn!(
            plugin_id = %plugin_id,
            error = %err,
            "plugin registration failed; child was torn down"
        );
        return Err(());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
