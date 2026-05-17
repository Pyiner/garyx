use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};

use garyx_bridge::MultiProviderBridge;
use garyx_models::config::{DiscordAccount, DiscordConfig};
use garyx_router::{InboundRequest, MessageRouter, NATIVE_COMMAND_TEXT_METADATA_KEY};

use crate::channel_trait::{Channel, ChannelError};
use crate::dispatcher::DiscordSender;

const DISCORD_GATEWAY_INTENTS: u64 = (1 << 0) | (1 << 9) | (1 << 12) | (1 << 15);
const DISCORD_RECONNECT_DELAY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DiscordUser {
    pub id: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub bot: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DiscordMessageReference {
    #[serde(default)]
    pub message_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DiscordMessageCreateEvent {
    pub id: String,
    pub channel_id: String,
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub content: String,
    pub author: DiscordUser,
    #[serde(default)]
    pub mentions: Vec<DiscordUser>,
    #[serde(default)]
    pub message_reference: Option<DiscordMessageReference>,
}

#[derive(Debug, Clone, Deserialize)]
struct DiscordGatewayEnvelope {
    op: u64,
    #[serde(default)]
    t: Option<String>,
    #[serde(default)]
    s: Option<u64>,
    #[serde(default)]
    d: Value,
}

#[derive(Debug, Clone, Deserialize)]
struct DiscordHello {
    heartbeat_interval: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct DiscordCurrentUser {
    id: String,
    #[serde(default)]
    username: Option<String>,
}

#[derive(Clone)]
struct DiscordInboundRuntime {
    http: Client,
    account_id: String,
    account: DiscordAccount,
    router: Arc<Mutex<MessageRouter>>,
    bridge: Arc<MultiProviderBridge>,
}

fn discord_user_mentioned(event: &DiscordMessageCreateEvent, bot_id: &str) -> bool {
    let bot_id = bot_id.trim();
    if bot_id.is_empty() {
        return false;
    }
    event.mentions.iter().any(|mention| mention.id == bot_id)
        || event.content.contains(&format!("<@{bot_id}>"))
        || event.content.contains(&format!("<@!{bot_id}>"))
}

fn strip_discord_bot_mention(content: &str, bot_id: &str) -> String {
    content
        .replace(&format!("<@{bot_id}>"), "")
        .replace(&format!("<@!{bot_id}>"), "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn build_inbound_request(
    account_id: &str,
    account: &DiscordAccount,
    bot_id: &str,
    event: DiscordMessageCreateEvent,
) -> Option<InboundRequest> {
    if event.author.bot || event.author.id == bot_id {
        return None;
    }

    let is_group = event.guild_id.is_some();
    let mentioned = discord_user_mentioned(&event, bot_id);
    if is_group && account.require_mention && !mentioned {
        return None;
    }

    let mut message = if mentioned {
        strip_discord_bot_mention(&event.content, bot_id)
    } else {
        event.content.trim().to_owned()
    };
    if message.trim().is_empty() {
        message = "(The user sent a message with no text content)".to_owned();
    }

    let mut metadata: HashMap<String, Value> = HashMap::new();
    metadata.insert("channel".to_owned(), Value::String("discord".to_owned()));
    metadata.insert(
        "account_id".to_owned(),
        Value::String(account_id.to_owned()),
    );
    metadata.insert(
        "chat_id".to_owned(),
        Value::String(event.channel_id.clone()),
    );
    metadata.insert("from_id".to_owned(), Value::String(event.author.id.clone()));
    metadata.insert("message_id".to_owned(), Value::String(event.id.clone()));
    metadata.insert(
        NATIVE_COMMAND_TEXT_METADATA_KEY.to_owned(),
        Value::String(message.clone()),
    );
    if let Some(username) = event.author.username.as_deref() {
        metadata.insert("from_name".to_owned(), Value::String(username.to_owned()));
    }
    if let Some(guild_id) = event.guild_id.as_deref() {
        metadata.insert("guild_id".to_owned(), Value::String(guild_id.to_owned()));
        metadata.insert("is_group".to_owned(), Value::Bool(true));
        metadata.insert(
            "delivery_thread_id".to_owned(),
            Value::String(event.channel_id.clone()),
        );
    } else {
        metadata.insert("delivery_thread_id".to_owned(), Value::Null);
    }

    Some(InboundRequest {
        channel: "discord".to_owned(),
        account_id: account_id.to_owned(),
        from_id: event.author.id.clone(),
        is_group,
        thread_binding_key: if is_group {
            event.channel_id.clone()
        } else {
            event.author.id.clone()
        },
        message,
        run_id: uuid::Uuid::new_v4().to_string(),
        reply_to_message_id: event
            .message_reference
            .as_ref()
            .and_then(|reference| reference.message_id.clone()),
        images: Vec::new(),
        extra_metadata: metadata,
        file_paths: Vec::new(),
    })
}

pub struct DiscordChannel {
    config: DiscordConfig,
    http: Client,
    running: Arc<AtomicBool>,
    tasks: Vec<JoinHandle<()>>,
    router: Arc<Mutex<MessageRouter>>,
    bridge: Arc<MultiProviderBridge>,
}

impl DiscordChannel {
    pub fn new(
        config: DiscordConfig,
        router: Arc<Mutex<MessageRouter>>,
        bridge: Arc<MultiProviderBridge>,
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
        }
    }

    async fn gateway_loop(
        runtime: DiscordInboundRuntime,
        bot: DiscordCurrentUser,
        running: Arc<AtomicBool>,
    ) {
        while running.load(Ordering::Relaxed) {
            let gateway_url = runtime.account.gateway_url.clone();
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
            let mut last_sequence: Option<u64> = None;
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
                                let identify = json!({
                                    "op": 2,
                                    "d": {
                                        "token": runtime.account.token,
                                        "intents": DISCORD_GATEWAY_INTENTS,
                                        "properties": {
                                            "os": std::env::consts::OS,
                                            "browser": "garyx",
                                            "device": "garyx"
                                        }
                                    }
                                });
                                if let Err(error) = write.send(Message::Text(identify.to_string().into())).await {
                                    warn!(account_id = %runtime.account_id, error = %error, "Discord identify failed");
                                    break;
                                }
                                identified = true;
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
                            7 | 9 => break,
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
        let Some(request) = build_inbound_request(
            &runtime.account_id,
            &runtime.account,
            &bot.id,
            event.clone(),
        ) else {
            return;
        };
        let reply_target = event.channel_id.clone();
        let reply_to = event.id.clone();
        let dispatch_result = {
            let mut router = runtime.router.lock().await;
            router
                .route_and_dispatch(request, runtime.bridge.as_ref(), None)
                .await
        };
        match dispatch_result {
            Ok(result) => {
                if let Some(local_reply) = result.local_reply {
                    let sender = DiscordSender {
                        account_id: runtime.account_id.clone(),
                        token: runtime.account.token.clone(),
                        http: runtime.http.clone(),
                        api_base: runtime.account.api_base.clone(),
                        is_running: true,
                    };
                    if let Err(error) = sender
                        .send_text(&reply_target, &local_reply, Some(&reply_to))
                        .await
                    {
                        warn!(
                            account_id = %runtime.account_id,
                            error = %error,
                            "failed to send local Discord reply"
                        );
                    }
                }
            }
            Err(error) => {
                warn!(
                    account_id = %runtime.account_id,
                    error = %error,
                    "failed to route Discord inbound message"
                );
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

#[cfg(test)]
mod tests {
    use super::*;
    use garyx_models::config::DiscordAccount;

    fn account(require_mention: bool) -> DiscordAccount {
        DiscordAccount {
            token: "discord-token".to_owned(),
            enabled: true,
            name: None,
            agent_id: "claude".to_owned(),
            workspace_dir: None,
            owner_target: None,
            require_mention,
            api_base: "https://discord.com/api/v10".to_owned(),
            gateway_url: "wss://gateway.discord.gg/?v=10&encoding=json".to_owned(),
        }
    }

    #[test]
    fn dm_message_does_not_require_mention() {
        let event = DiscordMessageCreateEvent {
            id: "message-001".to_owned(),
            channel_id: "dm-channel-123".to_owned(),
            guild_id: None,
            content: "hello from dm".to_owned(),
            author: DiscordUser {
                id: "user-123".to_owned(),
                username: Some("Test User".to_owned()),
                bot: false,
            },
            mentions: Vec::new(),
            message_reference: None,
        };

        let request = build_inbound_request("main", &account(true), "bot-999", event)
            .expect("dm should route without mention");

        assert_eq!(request.channel, "discord");
        assert_eq!(request.account_id, "main");
        assert_eq!(request.from_id, "user-123");
        assert!(!request.is_group);
        assert_eq!(request.thread_binding_key, "user-123");
        assert_eq!(request.message, "hello from dm");
        assert_eq!(request.extra_metadata["chat_id"], "dm-channel-123");
    }

    #[test]
    fn guild_message_requires_mention_by_default() {
        let event = DiscordMessageCreateEvent {
            id: "message-002".to_owned(),
            channel_id: "guild-channel-123".to_owned(),
            guild_id: Some("guild-456".to_owned()),
            content: "not for the bot".to_owned(),
            author: DiscordUser {
                id: "user-123".to_owned(),
                username: Some("Test User".to_owned()),
                bot: false,
            },
            mentions: Vec::new(),
            message_reference: None,
        };

        assert!(build_inbound_request("main", &account(true), "bot-999", event).is_none());
    }

    #[test]
    fn guild_mention_is_stripped_and_reply_id_is_preserved() {
        let event = DiscordMessageCreateEvent {
            id: "message-003".to_owned(),
            channel_id: "guild-channel-123".to_owned(),
            guild_id: Some("guild-456".to_owned()),
            content: "<@bot-999> please help".to_owned(),
            author: DiscordUser {
                id: "user-123".to_owned(),
                username: Some("Test User".to_owned()),
                bot: false,
            },
            mentions: vec![DiscordUser {
                id: "bot-999".to_owned(),
                username: Some("Garyx".to_owned()),
                bot: true,
            }],
            message_reference: Some(DiscordMessageReference {
                message_id: Some("reply-001".to_owned()),
            }),
        };

        let request = build_inbound_request("main", &account(true), "bot-999", event)
            .expect("mentioned guild message should route");

        assert!(request.is_group);
        assert_eq!(request.thread_binding_key, "guild-channel-123");
        assert_eq!(request.message, "please help");
        assert_eq!(request.reply_to_message_id.as_deref(), Some("reply-001"));
        assert_eq!(request.extra_metadata["guild_id"], "guild-456");
        assert_eq!(
            request.extra_metadata["delivery_thread_id"],
            "guild-channel-123"
        );
    }

    #[test]
    fn bot_authored_messages_are_ignored() {
        let event = DiscordMessageCreateEvent {
            id: "message-004".to_owned(),
            channel_id: "dm-channel-123".to_owned(),
            guild_id: None,
            content: "ignore me".to_owned(),
            author: DiscordUser {
                id: "bot-999".to_owned(),
                username: Some("Garyx".to_owned()),
                bot: true,
            },
            mentions: Vec::new(),
            message_reference: None,
        };

        assert!(build_inbound_request("main", &account(true), "bot-999", event).is_none());
    }
}
