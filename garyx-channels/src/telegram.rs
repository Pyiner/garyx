use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use garyx_bridge::MultiProviderBridge;
use garyx_models::command_catalog::{CommandCatalog, CommandCatalogOptions, CommandSurface};
use garyx_models::config::{TelegramAccount, TelegramConfig};
use garyx_router::MessageRouter;

use crate::channel_trait::{Channel, ChannelError};
#[cfg(test)]
use media::resolve_document_image_media_type;
#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use text::split_message;
#[cfg(test)]
use text::{safe_log_preview, strip_mention};

mod api;
mod dedup;
mod handlers;
mod helpers;
mod media;
mod streaming;
mod text;
mod types;

pub(crate) use api::send_response;
pub use api::{TelegramSendTarget, send_document, send_photo};
pub use helpers::{
    build_group_thread_key, extract_message_content, is_mentioned, resolve_forum_thread_id,
    resolve_outbound_thread_id, resolve_reply_to, resolve_typing_thread_id,
};
pub(crate) use streaming::{StreamingCallbackConfig, build_bound_response_callback};
pub use types::{
    ResponseCallback, TgAnimation, TgAudio, TgChat, TgDocument, TgFile, TgMessage, TgMessageEntity,
    TgPhotoSize, TgResponse, TgSticker, TgUpdate, TgUser, TgVideo, TgVoice,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const TELEGRAM_API_BASE: &str = "https://api.telegram.org";
const LONG_POLL_TIMEOUT_SECS: u64 = 30;
const MAX_MESSAGE_LENGTH: usize = 4096;
const OUTBOUND_MAX_RETRIES: usize = 3;
const MAX_IMAGE_SIZE_BYTES: usize = 5 * 1024 * 1024; // 5MB (match Python)
const MEDIA_GROUP_TIMEOUT_MILLIS: u64 = 500; // match Python MEDIA_GROUP_TIMEOUT_SECONDS=0.5
const TELEGRAM_GENERAL_TOPIC_ID: i64 = 1;
const DEDUP_TTL_SECONDS: u64 = 300; // match Python DEDUP_TTL_SECONDS
const DEDUP_MAX_SIZE: usize = 2000; // match Python DEDUP_MAX_SIZE
const DEBOUNCE_WINDOW_MILLIS: u64 = 1000; // match Python DEBOUNCE_WINDOW_SECONDS=1.0
const DEBOUNCE_MAX_FRAGMENTS: usize = 12; // match Python DEBOUNCE_MAX_FRAGMENTS
const DEBOUNCE_MAX_CHARS: usize = 50_000; // match Python DEBOUNCE_MAX_CHARS
const TELEGRAM_COMMAND_MENU_SYNC_INTERVAL: Duration = Duration::from_secs(10 * 60);
const MAX_TELEGRAM_BOT_COMMANDS: usize = 100;
const MAX_TELEGRAM_COMMAND_NAME_LEN: usize = 32;
const MAX_TELEGRAM_COMMAND_DESCRIPTION_CHARS: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct TelegramBotCommand {
    pub(crate) command: String,
    pub(crate) description: String,
}

#[derive(Clone)]
pub struct TelegramInboundRuntime {
    http: Client,
    account_id: String,
    account: TelegramAccount,
    router: Arc<Mutex<MessageRouter>>,
    bridge: Arc<MultiProviderBridge>,
}

#[derive(Clone)]
struct TelegramBotIdentity {
    token: String,
    username: String,
    user_id: i64,
}

pub struct TelegramChannel {
    config: TelegramConfig,
    http: Client,
    running: Arc<AtomicBool>,
    poll_tasks: Vec<JoinHandle<()>>,
    router: Arc<Mutex<MessageRouter>>,
    bridge: Arc<MultiProviderBridge>,
}

impl TelegramChannel {
    pub fn inbound_runtime(
        http: &Client,
        account_id: &str,
        account: &TelegramAccount,
        router: &Arc<Mutex<MessageRouter>>,
        bridge: &Arc<MultiProviderBridge>,
    ) -> TelegramInboundRuntime {
        TelegramInboundRuntime {
            http: http.clone(),
            account_id: account_id.to_owned(),
            account: account.clone(),
            router: router.clone(),
            bridge: bridge.clone(),
        }
    }

    pub fn new(
        config: TelegramConfig,
        router: Arc<Mutex<MessageRouter>>,
        bridge: Arc<MultiProviderBridge>,
    ) -> Self {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(LONG_POLL_TIMEOUT_SECS + 10))
            .build()
            .unwrap_or_else(|err| {
                warn!(
                    error = %err,
                    "failed to build configured reqwest client; falling back to default client"
                );
                Client::new()
            });

        Self {
            config,
            http,
            running: Arc::new(AtomicBool::new(false)),
            poll_tasks: Vec::new(),
            router,
            bridge,
        }
    }

    fn telegram_bot_commands_from_catalog(catalog: &CommandCatalog) -> Vec<TelegramBotCommand> {
        let mut commands = Vec::new();
        for entry in &catalog.commands {
            if !entry.surfaces.contains(&CommandSurface::Telegram) {
                continue;
            }
            let Some(command) = sanitize_telegram_command_name(&entry.name) else {
                continue;
            };
            if command.len() > MAX_TELEGRAM_COMMAND_NAME_LEN {
                continue;
            }
            if commands
                .iter()
                .any(|existing: &TelegramBotCommand| existing.command == command)
            {
                continue;
            }
            commands.push(TelegramBotCommand {
                command,
                description: truncate_command_description(&entry.description),
            });
            if commands.len() >= MAX_TELEGRAM_BOT_COMMANDS {
                break;
            }
        }
        commands
    }

    async fn set_my_commands_with_http(
        http: &Client,
        token: &str,
        commands: &[TelegramBotCommand],
    ) -> bool {
        let url = format!("{TELEGRAM_API_BASE}/bot{token}/setMyCommands");
        let payload = serde_json::json!({ "commands": commands });

        match http.post(&url).json(&payload).send().await {
            Ok(resp) => {
                let status = resp.status();
                if !status.is_success() {
                    warn!(status = %status, "setMyCommands returned non-200");
                    return false;
                }
                true
            }
            Err(e) => {
                warn!(error = %e, "setMyCommands request failed (non-fatal)");
                false
            }
        }
    }

    async fn sync_command_menu_once(
        http: &Client,
        router: &Arc<Mutex<MessageRouter>>,
        token: &str,
        account_id: &str,
        last_successful_fingerprint: &mut Option<String>,
    ) {
        let catalog = {
            let router = router.lock().await;
            router.command_catalog(CommandCatalogOptions {
                surface: Some(CommandSurface::Telegram),
                channel: Some("telegram".to_owned()),
                account_id: Some(account_id.to_owned()),
                include_hidden: false,
            })
        };
        let commands = Self::telegram_bot_commands_from_catalog(&catalog);
        let fingerprint = command_menu_fingerprint(&catalog.revision, &commands);
        if last_successful_fingerprint.as_deref() == Some(fingerprint.as_str()) {
            return;
        }
        if Self::set_my_commands_with_http(http, token, &commands).await {
            *last_successful_fingerprint = Some(fingerprint);
            info!(
                account_id,
                command_count = commands.len(),
                revision = %catalog.revision,
                "Telegram command menu synced"
            );
        }
    }

    async fn command_menu_sync_loop(
        http: Client,
        router: Arc<Mutex<MessageRouter>>,
        token: String,
        account_id: String,
        running: Arc<AtomicBool>,
    ) {
        let mut last_successful_fingerprint = None;
        while running.load(Ordering::Relaxed) {
            Self::sync_command_menu_once(
                &http,
                &router,
                &token,
                &account_id,
                &mut last_successful_fingerprint,
            )
            .await;
            tokio::time::sleep(TELEGRAM_COMMAND_MENU_SYNC_INTERVAL).await;
        }
    }

    /// Verify a bot token by calling getMe.
    async fn verify_bot(&self, token: &str) -> Result<TgUser, ChannelError> {
        let url = format!("{TELEGRAM_API_BASE}/bot{token}/getMe");
        let resp: TgResponse<TgUser> = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| ChannelError::Connection(format!("getMe request failed: {e}")))?
            .json()
            .await
            .map_err(|e| ChannelError::Connection(format!("getMe parse failed: {e}")))?;

        match resp.result {
            Some(user) if resp.ok => Ok(user),
            _ => Err(ChannelError::Connection(format!(
                "getMe failed: {}",
                resp.description.unwrap_or_default()
            ))),
        }
    }

    /// Long-poll loop for a single account.
    async fn poll_loop(
        runtime: TelegramInboundRuntime,
        bot: TelegramBotIdentity,
        running: Arc<AtomicBool>,
    ) {
        let mut offset: i64 = 0;
        info!(account_id = runtime.account_id, "starting poll loop");

        while running.load(Ordering::Relaxed) {
            let url = format!(
                "{TELEGRAM_API_BASE}/bot{token}/getUpdates?offset={offset}&timeout={LONG_POLL_TIMEOUT_SECS}",
                token = bot.token,
            );

            let result = runtime.http.get(&url).send().await;
            let response = match result {
                Ok(r) => r,
                Err(e) => {
                    if !running.load(Ordering::Relaxed) {
                        break;
                    }
                    warn!(account_id = runtime.account_id, error = %e, "getUpdates request failed, retrying");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
            };

            let body: TgResponse<Vec<TgUpdate>> = match response.json().await {
                Ok(b) => b,
                Err(e) => {
                    warn!(account_id = runtime.account_id, error = %e, "getUpdates parse failed, retrying");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
            };

            if !body.ok {
                warn!(
                    account_id = runtime.account_id,
                    desc = body.description.as_deref().unwrap_or("unknown"),
                    "getUpdates returned ok=false"
                );
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }

            let updates = body.result.unwrap_or_default();
            for update in updates {
                offset = update.update_id + 1;
                let context = handlers::TelegramUpdateContext::new(
                    handlers::TelegramChannelResources {
                        http: &runtime.http,
                        router: &runtime.router,
                        bridge: &runtime.bridge,
                        api_base: TELEGRAM_API_BASE,
                    },
                    handlers::TelegramBotRuntime {
                        account_id: &runtime.account_id,
                        token: &bot.token,
                        bot_username: &bot.username,
                        bot_id: bot.user_id,
                        account: &runtime.account,
                    },
                );
                Self::handle_update(&context, &update).await;
            }
        }

        info!(account_id = runtime.account_id, "poll loop stopped");
    }
}

#[async_trait]
impl Channel for TelegramChannel {
    fn name(&self) -> &str {
        "telegram"
    }

    async fn start(&mut self) -> Result<(), ChannelError> {
        if self.running.load(Ordering::Relaxed) {
            return Err(ChannelError::Internal("already running".into()));
        }

        self.running.store(true, Ordering::Relaxed);

        for (account_id, account) in &self.config.accounts {
            if !account.enabled {
                info!(account_id, "account disabled, skipping");
                continue;
            }

            // Verify bot token
            let bot_user = self.verify_bot(&account.token).await?;
            let bot_username = bot_user.username.as_deref().unwrap_or("").to_lowercase();
            let bot_id = bot_user.id;
            info!(
                account_id,
                bot_username = bot_username,
                bot_id,
                "verified bot"
            );

            let runtime =
                Self::inbound_runtime(&self.http, account_id, account, &self.router, &self.bridge);
            let bot = TelegramBotIdentity {
                token: account.token.clone(),
                username: bot_username,
                user_id: bot_id,
            };
            let running = self.running.clone();
            let handle = tokio::spawn(Self::poll_loop(runtime, bot, running));
            self.poll_tasks.push(handle);
            let sync_handle = tokio::spawn(Self::command_menu_sync_loop(
                self.http.clone(),
                self.router.clone(),
                account.token.clone(),
                account_id.clone(),
                self.running.clone(),
            ));
            self.poll_tasks.push(sync_handle);
            info!(account_id, "Telegram account running in polling mode");
        }

        Ok(())
    }

    async fn stop(&mut self) -> Result<(), ChannelError> {
        info!("stopping telegram channel");
        self.running.store(false, Ordering::Relaxed);

        for handle in self.poll_tasks.drain(..) {
            handle.abort();
            let _ = handle.await;
        }

        Ok(())
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

fn sanitize_telegram_command_name(raw: &str) -> Option<String> {
    let mut result = String::new();
    let mut previous_underscore = false;
    for ch in raw.trim().trim_start_matches('/').chars() {
        let next = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else if ch == '-' || ch == '_' {
            Some('_')
        } else {
            None
        };
        let Some(next) = next else {
            continue;
        };
        if next == '_' {
            if previous_underscore {
                continue;
            }
            previous_underscore = true;
        } else {
            previous_underscore = false;
        }
        result.push(next);
    }
    let trimmed = result.trim_matches('_').to_owned();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn truncate_command_description(description: &str) -> String {
    let mut output = String::new();
    for ch in description
        .chars()
        .take(MAX_TELEGRAM_COMMAND_DESCRIPTION_CHARS)
    {
        output.push(ch);
    }
    output
}

fn command_menu_fingerprint(revision: &str, commands: &[TelegramBotCommand]) -> String {
    let commands = serde_json::to_string(commands).unwrap_or_default();
    format!("{revision}:{commands}")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
