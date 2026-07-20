use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use reqwest::header::CONTENT_TYPE;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc, watch};
use tokio::task::JoinHandle;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};

use garyx_bridge::MultiProviderBridge;
use garyx_models::config::{DiscordAccount, DiscordConfig};
use garyx_models::provider::{
    ATTACHMENTS_METADATA_KEY, PromptAttachment, PromptAttachmentKind, ProviderMessage,
    StreamBoundaryKind, StreamEvent, attachments_to_metadata_value,
};
use garyx_router::{InboundRequest, MessageRouter, NATIVE_COMMAND_TEXT_METADATA_KEY};

use crate::channel_trait::{Channel, ChannelError};
use crate::dispatcher::{ChannelDispatcher, DiscordSender};
use crate::generated_images::{extract_image_generation_result, write_generated_image_temp};

pub mod outbound;
use crate::plugin_tools::{
    PluginStreamSendDecision, PluginStreamSendPolicy, PluginStreamSendState,
    should_hide_tool_call_display,
};
use outbound::{DISCORD_MAX_MESSAGE_LENGTH, split_discord_message};

const DISCORD_RECONNECT_DELAY: Duration = Duration::from_secs(5);

pub struct DiscordChannel {
    config: DiscordConfig,
    http: Client,
    running: Arc<AtomicBool>,
    tasks: Vec<JoinHandle<()>>,
    router: Arc<Mutex<MessageRouter>>,
    bridge: Arc<MultiProviderBridge>,
    dispatcher: Arc<dyn ChannelDispatcher>,
}

impl DiscordChannel {
    pub fn new(
        config: DiscordConfig,
        router: Arc<Mutex<MessageRouter>>,
        bridge: Arc<MultiProviderBridge>,
        dispatcher: Arc<dyn ChannelDispatcher>,
    ) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|err| {
                warn!(
                    error = %err,
                    "failed to build Discord reqwest client; falling back to default client"
                );
                Client::new()
            });
        Self {
            config,
            http,
            running: Arc::new(AtomicBool::new(false)),
            tasks: Vec::new(),
            router,
            bridge,
            dispatcher,
        }
    }

    async fn verify_bot(
        http: &Client,
        account_id: &str,
        account: &DiscordAccount,
    ) -> Result<DiscordCurrentUser, ChannelError> {
        let token = account.token.trim();
        if token.is_empty() {
            return Err(ChannelError::Config(format!(
                "Discord account '{account_id}' token is required"
            )));
        }
        let response = http
            .get(format!(
                "{}/users/@me",
                account.api_base.trim_end_matches('/')
            ))
            .header("Authorization", format!("Bot {token}"))
            .send()
            .await
            .map_err(|error| {
                ChannelError::Connection(format!("Discord users/@me request failed: {error}"))
            })?;
        let status = response.status();
        if !status.is_success() {
            return Err(ChannelError::Connection(format!(
                "Discord users/@me HTTP {status}"
            )));
        }
        response
            .json::<DiscordCurrentUser>()
            .await
            .map_err(|error| {
                ChannelError::Connection(format!("Discord users/@me parse failed: {error}"))
            })
    }

    fn inbound_runtime(&self, account_id: &str, account: &DiscordAccount) -> DiscordInboundRuntime {
        DiscordInboundRuntime {
            http: self.http.clone(),
            account_id: account_id.to_owned(),
            account: account.clone(),
            router: self.router.clone(),
            bridge: self.bridge.clone(),
            dispatcher: self.dispatcher.clone(),
        }
    }

    async fn gateway_loop(
        runtime: DiscordInboundRuntime,
        bot: DiscordCurrentUser,
        running: Arc<AtomicBool>,
    ) {
        let mut last_sequence: Option<u64> = None;
        let mut session_id: Option<String> = None;
        let mut resume_gateway_url: Option<String> = None;

        while running.load(Ordering::Relaxed) {
            let gateway_url = resume_gateway_url
                .as_deref()
                .map(discord_gateway_url_with_query)
                .unwrap_or_else(|| runtime.account.gateway_url.clone());
            let connection = connect_async(&gateway_url).await;
            let (socket, _) = match connection {
                Ok(connection) => connection,
                Err(error) => {
                    warn!(
                        account_id = %runtime.account_id,
                        error = %error,
                        "Discord Gateway connect failed; retrying"
                    );
                    tokio::time::sleep(DISCORD_RECONNECT_DELAY).await;
                    continue;
                }
            };

            info!(
                account_id = %runtime.account_id,
                bot_id = %bot.id,
                "Discord Gateway connected"
            );
            let (mut write, mut read) = socket.split();
            let mut heartbeat = tokio::time::interval(Duration::from_secs(45));
            heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut identified = false;

            loop {
                if !running.load(Ordering::Relaxed) {
                    break;
                }
                tokio::select! {
                    _ = heartbeat.tick(), if identified => {
                        let heartbeat_payload = json!({
                            "op": 1,
                            "d": last_sequence
                        });
                        if let Err(error) = write.send(Message::Text(heartbeat_payload.to_string().into())).await {
                            warn!(account_id = %runtime.account_id, error = %error, "Discord heartbeat failed");
                            break;
                        }
                    }
                    maybe_message = read.next() => {
                        let Some(message) = maybe_message else {
                            break;
                        };
                        let message = match message {
                            Ok(message) => message,
                            Err(error) => {
                                warn!(account_id = %runtime.account_id, error = %error, "Discord Gateway read failed");
                                break;
                            }
                        };
                        let text = match message {
                            Message::Text(text) => text.to_string(),
                            Message::Binary(bytes) => String::from_utf8_lossy(&bytes).to_string(),
                            Message::Close(_) => break,
                            _ => continue,
                        };
                        let envelope = match serde_json::from_str::<DiscordGatewayEnvelope>(&text) {
                            Ok(envelope) => envelope,
                            Err(error) => {
                                warn!(account_id = %runtime.account_id, error = %error, "Discord Gateway payload parse failed");
                                continue;
                            }
                        };
                        if let Some(sequence) = envelope.s {
                            last_sequence = Some(sequence);
                        }
                        match envelope.op {
                            10 => {
                                let hello = serde_json::from_value::<DiscordHello>(envelope.d)
                                    .unwrap_or(DiscordHello { heartbeat_interval: 45_000 });
                                heartbeat = tokio::time::interval(Duration::from_millis(
                                    hello.heartbeat_interval.max(1),
                                ));
                                heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                                let (handshake, handshake_name) = if let (Some(session_id), Some(sequence)) =
                                    (session_id.as_deref(), last_sequence)
                                {
                                    (
                                        discord_resume_payload(&runtime.account.token, session_id, sequence),
                                        "resume",
                                    )
                                } else {
                                    (discord_identify_payload(&runtime.account.token), "identify")
                                };
                                if let Err(error) = write.send(Message::Text(handshake.to_string().into())).await {
                                    warn!(account_id = %runtime.account_id, error = %error, handshake = handshake_name, "Discord Gateway handshake failed");
                                    break;
                                }
                                if handshake_name == "resume" {
                                    info!(
                                        account_id = %runtime.account_id,
                                        sequence = last_sequence.unwrap_or_default(),
                                        "Discord Gateway resume requested"
                                    );
                                }
                                identified = true;
                            }
                            0 if envelope.t.as_deref() == Some("READY") => {
                                match serde_json::from_value::<DiscordReady>(envelope.d) {
                                    Ok(ready) => {
                                        session_id = Some(ready.session_id);
                                        resume_gateway_url = ready
                                            .resume_gateway_url
                                            .map(|url| discord_gateway_url_with_query(&url));
                                    }
                                    Err(error) => {
                                        warn!(account_id = %runtime.account_id, error = %error, "Discord READY parse failed");
                                    }
                                }
                            }
                            0 if envelope.t.as_deref() == Some("MESSAGE_CREATE") => {
                                match serde_json::from_value::<DiscordMessageCreateEvent>(envelope.d) {
                                    Ok(event) => {
                                        Self::handle_message_create(&runtime, &bot, event).await;
                                    }
                                    Err(error) => {
                                        warn!(account_id = %runtime.account_id, error = %error, "Discord MESSAGE_CREATE parse failed");
                                    }
                                }
                            }
                            1 => {
                                let heartbeat_payload = json!({
                                    "op": 1,
                                    "d": last_sequence
                                });
                                if let Err(error) = write.send(Message::Text(heartbeat_payload.to_string().into())).await {
                                    warn!(account_id = %runtime.account_id, error = %error, "Discord heartbeat request response failed");
                                    break;
                                }
                            }
                            7 => break,
                            9 => {
                                let resumable = envelope.d.as_bool().unwrap_or(false);
                                if !resumable {
                                    session_id = None;
                                    last_sequence = None;
                                    resume_gateway_url = None;
                                }
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }

            if running.load(Ordering::Relaxed) {
                tokio::time::sleep(DISCORD_RECONNECT_DELAY).await;
            }
        }
        info!(account_id = %runtime.account_id, "Discord Gateway loop stopped");
    }

    async fn handle_message_create(
        runtime: &DiscordInboundRuntime,
        bot: &DiscordCurrentUser,
        event: DiscordMessageCreateEvent,
    ) {
        let Some(mut request) = build_inbound_request(
            &runtime.account_id,
            &runtime.account,
            &bot.id,
            event.clone(),
        ) else {
            return;
        };
        enrich_inbound_request_with_discord_attachments(runtime, &event, &mut request).await;
        let reply_target = event.channel_id.clone();
        let reply_to = event.id.clone();
        let sender = DiscordSender {
            account_id: runtime.account_id.clone(),
            token: runtime.account.token.clone(),
            http: runtime.http.clone(),
            api_base: runtime.account.api_base.clone(),
            is_running: true,
        };
        let (response_callback, thread_id_tx) =
            build_discord_response_callback(DiscordStreamingCallbackConfig {
                sender: sender.clone(),
                chat_id: reply_target.clone(),
                reply_to_message_id: Some(reply_to.clone()),
            });

        let run_id = request.run_id.clone();
        let pipeline = crate::inbound::InboundPipeline {
            router: &runtime.router,
            bridge: &runtime.bridge,
            dispatcher: &runtime.dispatcher,
        };
        let dispatch_result = pipeline
            .dispatch(request, response_callback, None, |thread_id| async move {
                let _ = thread_id_tx.send(thread_id);
            })
            .await;
        match dispatch_result {
            Ok(result) => {
                if let Some(local_reply) = result.local_reply {
                    match sender
                        .send_text(&reply_target, &local_reply, Some(&reply_to))
                        .await
                    {
                        Ok(_) => {}
                        Err(error) => {
                            warn!(
                                account_id = %runtime.account_id,
                                error = %error,
                                "failed to send local Discord reply"
                            );
                        }
                    }
                }
            }
            Err(crate::inbound::InboundDispatchFailure::CommittedReplay(error)) => {
                tracing::error!(run_id = %run_id, error = %error, "committed replay bus missing for Discord dispatch");
            }
            Err(crate::inbound::InboundDispatchFailure::Dispatch(error)) => {
                warn!(
                    account_id = %runtime.account_id,
                    error = %error,
                    "failed to route Discord inbound message"
                );
                if let Err(send_error) = sender
                    .send_text(&reply_target, &format!("Error: {error}"), Some(&reply_to))
                    .await
                {
                    warn!(
                        account_id = %runtime.account_id,
                        error = %send_error,
                        "failed to send Discord routing error reply"
                    );
                }
            }
        }
    }
}

#[async_trait]
impl Channel for DiscordChannel {
    fn name(&self) -> &str {
        "discord"
    }

    async fn start(&mut self) -> Result<(), ChannelError> {
        if self.running.load(Ordering::Relaxed) {
            return Err(ChannelError::Internal("already running".into()));
        }
        self.running.store(true, Ordering::Relaxed);
        for (account_id, account) in &self.config.accounts {
            if !account.enabled {
                info!(account_id, "Discord account disabled, skipping");
                continue;
            }
            let bot = Self::verify_bot(&self.http, account_id, account).await?;
            info!(
                account_id,
                bot_id = %bot.id,
                bot_username = bot.username.as_deref().unwrap_or(""),
                "verified Discord bot"
            );
            let runtime = self.inbound_runtime(account_id, account);
            let running = self.running.clone();
            self.tasks
                .push(tokio::spawn(Self::gateway_loop(runtime, bot, running)));
        }
        Ok(())
    }

    async fn stop(&mut self) -> Result<(), ChannelError> {
        self.running.store(false, Ordering::Relaxed);
        for task in self.tasks.drain(..) {
            task.abort();
            let _ = task.await;
        }
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

mod inbound;
mod media;
mod protocol;
mod streaming;

pub(crate) use inbound::build_inbound_request;
pub(crate) use protocol::{DiscordAttachment, DiscordMessageCreateEvent};
#[allow(unused_imports)]
pub(crate) use protocol::{DiscordMessageReference, DiscordUser};
pub(crate) use streaming::{DiscordStreamingCallbackConfig, build_discord_response_callback};

use inbound::*;
use media::*;
use protocol::*;
#[cfg(test)]
use streaming::DISCORD_TOOL_PLACEHOLDER_UPDATE_INTERVAL;

#[cfg(test)]
mod tests;
