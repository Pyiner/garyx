use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use reqwest::Client as HttpClient;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::RwLock;
use tokio::time::Instant;
use tracing::debug;

use garyx_models::config::{FeishuAccount, FeishuDomain};

use super::message::build_streaming_card_body;
use super::{
    DEFAULT_TOKEN_LIFETIME, FEISHU_API_BASE, FeishuError, LARK_API_BASE, TOKEN_REFRESH_MARGIN,
    WS_RECONNECT_DELAY, extract_message_text,
};

#[derive(Debug, Deserialize)]
struct TokenResponse {
    code: i64,
    #[serde(default)]
    tenant_access_token: String,
    #[serde(default)]
    expire: u64,
    #[serde(default)]
    msg: String,
}

#[derive(Debug, Deserialize)]
struct SendMessageResponse {
    code: i64,
    #[serde(default)]
    msg: String,
    #[serde(default)]
    data: Option<SendMessageData>,
}

#[derive(Debug, Deserialize)]
struct SendMessageData {
    #[serde(default)]
    message_id: String,
}

#[derive(Debug, Deserialize)]
struct FeishuReactionResponse {
    code: i64,
    #[serde(default)]
    msg: String,
    #[serde(default)]
    data: Option<FeishuReactionData>,
}

#[derive(Debug, Deserialize)]
struct FeishuReactionData {
    #[serde(default)]
    reaction_id: String,
}

#[derive(Debug, Deserialize)]
struct WsEndpointResponse {
    code: i64,
    #[serde(default)]
    msg: String,
    #[serde(default)]
    data: Option<WsEndpointData>,
}

#[derive(Debug, Deserialize)]
struct WsEndpointData {
    /// The SDK uses PascalCase field `URL`.
    #[serde(default, alias = "url")]
    #[serde(rename = "URL")]
    ws_url: String,
    #[serde(default, alias = "clientConfig")]
    #[serde(rename = "ClientConfig")]
    client_config: Option<WsClientConfig>,
}

#[derive(Debug, Deserialize)]
struct WsClientConfig {
    #[serde(default, alias = "pingInterval")]
    #[serde(rename = "PingInterval")]
    ping_interval: u64,
    #[serde(default, alias = "reconnectInterval")]
    #[serde(rename = "ReconnectInterval")]
    reconnect_interval: u64,
}

/// Information returned from the WS endpoint, used for connecting.
#[derive(Debug)]
pub(super) struct WsConnectInfo {
    pub ws_url: String,
    pub service_id: i32,
    pub ping_interval_secs: u64,
    pub reconnect_interval_secs: u64,
}

#[derive(Debug, Deserialize)]
struct FeishuUserResponse {
    code: i64,
    #[serde(default)]
    msg: String,
    #[serde(default)]
    data: Option<FeishuUserData>,
}

#[derive(Debug, Deserialize)]
struct FeishuUserData {
    #[serde(default)]
    user: Option<FeishuUser>,
}

#[derive(Debug, Deserialize)]
struct FeishuUser {
    #[serde(default)]
    name: String,
    #[serde(default)]
    en_name: String,
    #[serde(default)]
    nickname: String,
}

/// Per-account Feishu API client with token management.
#[derive(Clone)]
pub(super) struct FeishuClient {
    pub(super) app_id: String,
    pub(super) app_secret: String,
    pub(super) domain: FeishuDomain,
    pub(super) http: HttpClient,
    /// Token and its expiry stored atomically to prevent inconsistent reads.
    pub(super) token_state: Arc<RwLock<Option<(String, Instant)>>>,
    /// Serialises refresh attempts so only one HTTP call runs at a time.
    pub(super) refresh_lock: Arc<tokio::sync::Mutex<()>>,
    pub(super) api_base_override: Option<String>,
}

impl FeishuClient {
    pub(super) fn new(account: &FeishuAccount) -> Self {
        Self {
            app_id: account.app_id.clone(),
            app_secret: account.app_secret.clone(),
            domain: account.domain.clone(),
            http: HttpClient::new(),
            token_state: Arc::new(RwLock::new(None)),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
            api_base_override: None,
        }
    }

    pub(super) fn api_base(&self) -> &str {
        if let Some(ref base) = self.api_base_override {
            return base;
        }
        match self.domain {
            FeishuDomain::Lark => LARK_API_BASE,
            FeishuDomain::Feishu => FEISHU_API_BASE,
        }
    }

    /// Obtain a tenant access token from Feishu Open API.
    async fn fetch_access_token(&self) -> Result<(String, Duration), FeishuError> {
        let url = format!("{}/auth/v3/tenant_access_token/internal", self.api_base());
        let body = serde_json::json!({
            "app_id": self.app_id,
            "app_secret": self.app_secret,
        });

        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;

        let token_resp: TokenResponse = resp
            .json()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;

        if token_resp.code != 0 {
            return Err(FeishuError::Api {
                code: token_resp.code,
                msg: token_resp.msg,
            });
        }

        if token_resp.tenant_access_token.is_empty() {
            return Err(FeishuError::Api {
                code: -1,
                msg: "empty access token".into(),
            });
        }

        let lifetime = if token_resp.expire > 0 {
            Duration::from_secs(token_resp.expire)
        } else {
            DEFAULT_TOKEN_LIFETIME
        };

        Ok((token_resp.tenant_access_token, lifetime))
    }

    /// Get a valid access token, refreshing if needed.
    pub(super) async fn get_access_token(&self) -> Result<String, FeishuError> {
        self.refresh_token_if_needed().await?;
        let guard = self.token_state.read().await;
        guard
            .as_ref()
            .map(|(token, _)| token.clone())
            .ok_or_else(|| FeishuError::Api {
                code: -1,
                msg: "no access token available".into(),
            })
    }

    /// Refresh the token if it is absent or about to expire.
    ///
    /// Uses a dedicated mutex to ensure only one HTTP refresh runs at a time.
    /// After acquiring the mutex, we re-check staleness to avoid redundant calls
    /// (the loser of the race will find a fresh token and return immediately).
    pub(super) async fn refresh_token_if_needed(&self) -> Result<(), FeishuError> {
        // Fast path: read-lock check without holding the refresh mutex.
        {
            let state = self.token_state.read().await;
            if let Some((_, expires_at)) = state.as_ref()
                && Instant::now() + TOKEN_REFRESH_MARGIN < *expires_at
            {
                return Ok(());
            }
        }

        // Serialise concurrent refresh attempts.
        let _refresh_guard = self.refresh_lock.lock().await;

        // Re-check after acquiring the mutex — another task may have refreshed already.
        {
            let state = self.token_state.read().await;
            if let Some((_, expires_at)) = state.as_ref()
                && Instant::now() + TOKEN_REFRESH_MARGIN < *expires_at
            {
                return Ok(());
            }
        }

        let (new_token, lifetime) = self.fetch_access_token().await?;
        let new_expires_at = Instant::now() + lifetime;

        {
            let mut state = self.token_state.write().await;
            *state = Some((new_token, new_expires_at));
        }

        debug!("Feishu access token refreshed (app_id={})", self.app_id);
        Ok(())
    }

    /// Send a message to a chat.
    pub(super) async fn send_message(
        &self,
        chat_id: &str,
        content: &str,
        msg_type: &str,
    ) -> Result<String, FeishuError> {
        self.send_message_to_target("chat_id", chat_id, content, msg_type)
            .await
    }

    /// Send a message to a typed target.
    pub(super) async fn send_message_to_target(
        &self,
        receive_id_type: &str,
        receive_id: &str,
        content: &str,
        msg_type: &str,
    ) -> Result<String, FeishuError> {
        let token = self.get_access_token().await?;
        let normalized_receive_id_type = match receive_id_type.trim() {
            "open_id" => "open_id",
            _ => "chat_id",
        };
        let url = format!(
            "{}/im/v1/messages?receive_id_type={normalized_receive_id_type}",
            self.api_base()
        );

        let body = serde_json::json!({
            "receive_id": receive_id,
            "msg_type": msg_type,
            "content": content,
        });

        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&body)
            .send()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;

        let api_resp: SendMessageResponse = resp
            .json()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;

        if api_resp.code != 0 {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        Ok(api_resp.data.map(|d| d.message_id).unwrap_or_default())
    }

    /// Reply to a specific message.
    pub(super) async fn reply_message(
        &self,
        message_id: &str,
        content: &str,
        msg_type: &str,
    ) -> Result<String, FeishuError> {
        self.reply_message_ext(message_id, content, msg_type, false)
            .await
    }

    /// Reply to a message, optionally keeping the reply inside a topic thread.
    ///
    /// When `reply_in_thread` is true the Feishu API creates/continues a topic
    /// thread anchored to the replied message (matching the reference SDK behaviour).
    pub(super) async fn reply_message_ext(
        &self,
        message_id: &str,
        content: &str,
        msg_type: &str,
        reply_in_thread: bool,
    ) -> Result<String, FeishuError> {
        let token = self.get_access_token().await?;
        let url = format!("{}/im/v1/messages/{}/reply", self.api_base(), message_id);

        let mut body = serde_json::json!({
            "msg_type": msg_type,
            "content": content,
        });
        if reply_in_thread {
            body["reply_in_thread"] = serde_json::Value::Bool(true);
        }

        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&body)
            .send()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;

        let api_resp: SendMessageResponse = resp
            .json()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;

        if api_resp.code != 0 {
            return Err(FeishuError::Api {
                code: api_resp.code,
                msg: api_resp.msg,
            });
        }

        Ok(api_resp.data.map(|d| d.message_id).unwrap_or_default())
    }

    async fn upload_message_image(&self, image_path: &Path) -> Result<String, FeishuError> {
        let token = self.get_access_token().await?;
        let image_bytes = tokio::fs::read(image_path)
            .await
            .map_err(|e| FeishuError::Http(format!("image read failed: {e}")))?;
        let filename = image_path
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("image.png")
            .to_owned();
        let image_part = reqwest::multipart::Part::bytes(image_bytes).file_name(filename);
        let upload_form = reqwest::multipart::Form::new()
            .text("image_type", "message")
            .part("image", image_part);
        let url = format!("{}/im/v1/images", self.api_base());

        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .multipart(upload_form)
            .send()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;

        let value: Value = resp
            .json()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;
        let code = value.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
        if code != 0 {
            let msg = value
                .get("msg")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(FeishuError::Api {
                code,
                msg: msg.to_owned(),
            });
        }

        let image_key = value
            .get("data")
            .and_then(|v| v.get("image_key"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| FeishuError::Api {
                code: -1,
                msg: "image upload returned empty image_key".to_owned(),
            })?;
        Ok(image_key.to_owned())
    }

    pub(super) async fn send_image(
        &self,
        chat_id: &str,
        image_path: &Path,
    ) -> Result<String, FeishuError> {
        let image_key = self.upload_message_image(image_path).await?;
        let content = serde_json::json!({ "image_key": image_key }).to_string();
        self.send_message(chat_id, &content, "image").await
    }

    pub(super) async fn reply_image_ext(
        &self,
        message_id: &str,
        image_path: &Path,
        reply_in_thread: bool,
    ) -> Result<String, FeishuError> {
        let image_key = self.upload_message_image(image_path).await?;
        let content = serde_json::json!({ "image_key": image_key }).to_string();
        self.reply_message_ext(message_id, &content, "image", reply_in_thread)
            .await
    }

    /// Domain base without the `/open-apis` path suffix.
    fn domain_base(&self) -> &str {
        if let Some(ref base) = self.api_base_override {
            return base.trim_end_matches("/open-apis");
        }
        match self.domain {
            FeishuDomain::Lark => "https://open.larksuite.com",
            FeishuDomain::Feishu => "https://open.feishu.cn",
        }
    }

    /// Request a WebSocket endpoint URL from the Feishu gateway.
    ///
    /// Uses AppID/AppSecret authentication (matching the official SDK behavior)
    /// rather than tenant_access_token Bearer auth.
    /// Note: this endpoint lives at `{domain}/callback/ws/endpoint` (no `/open-apis` prefix).
    pub(super) async fn get_ws_endpoint(&self) -> Result<WsConnectInfo, FeishuError> {
        let url = format!("{}/callback/ws/endpoint", self.domain_base());

        let resp = self
            .http
            .post(&url)
            .header("locale", "zh")
            .json(&serde_json::json!({
                "AppID": self.app_id,
                "AppSecret": self.app_secret,
            }))
            .send()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;

        let ws_resp: WsEndpointResponse = resp
            .json()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;

        if ws_resp.code != 0 {
            return Err(FeishuError::Api {
                code: ws_resp.code,
                msg: ws_resp.msg,
            });
        }

        let data = ws_resp.data.ok_or_else(|| FeishuError::Api {
            code: -1,
            msg: "missing data in WS endpoint response".into(),
        })?;

        if data.ws_url.is_empty() {
            return Err(FeishuError::Api {
                code: -1,
                msg: "empty WebSocket URL".into(),
            });
        }

        // Parse service_id from URL query string (e.g. ?...&service_id=123&...)
        let service_id = data
            .ws_url
            .split('?')
            .nth(1)
            .and_then(|qs| {
                qs.split('&').find_map(|pair| {
                    let (k, v) = pair.split_once('=')?;
                    if k == "service_id" {
                        v.parse::<i32>().ok()
                    } else {
                        None
                    }
                })
            })
            .unwrap_or(0);

        let ping_interval_secs = data
            .client_config
            .as_ref()
            .map(|c| c.ping_interval)
            .unwrap_or(120);
        let reconnect_interval_secs = data
            .client_config
            .as_ref()
            .map(|c| c.reconnect_interval)
            .filter(|secs| *secs > 0)
            .unwrap_or(WS_RECONNECT_DELAY.as_secs());

        Ok(WsConnectInfo {
            ws_url: data.ws_url,
            service_id,
            ping_interval_secs,
            reconnect_interval_secs,
        })
    }

    pub(super) async fn fetch_user_display_name(
        &self,
        sender_open_id: &str,
    ) -> Result<Option<String>, FeishuError> {
        if sender_open_id.is_empty() {
            return Ok(None);
        }

        let token = self.get_access_token().await?;
        let url = format!(
            "{}/contact/v3/users/{}?user_id_type=open_id",
            self.api_base(),
            sender_open_id
        );

        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;

        let user_resp: FeishuUserResponse = resp
            .json()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;

        if user_resp.code != 0 {
            return Err(FeishuError::Api {
                code: user_resp.code,
                msg: user_resp.msg,
            });
        }

        let Some(user) = user_resp.data.and_then(|d| d.user) else {
            return Ok(None);
        };
        for candidate in [user.name, user.en_name, user.nickname] {
            let trimmed = candidate.trim();
            if !trimmed.is_empty() {
                return Ok(Some(trimmed.to_owned()));
            }
        }
        Ok(None)
    }

    pub(super) async fn fetch_message_text(
        &self,
        message_id: &str,
    ) -> Result<Option<String>, FeishuError> {
        if message_id.is_empty() {
            return Ok(None);
        }

        let token = self.get_access_token().await?;
        let url = format!("{}/im/v1/messages/{}", self.api_base(), message_id);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;

        let value: Value = resp
            .json()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;
        let code = value.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
        if code != 0 {
            let msg = value
                .get("msg")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(FeishuError::Api {
                code,
                msg: msg.to_owned(),
            });
        }

        let items = value
            .get("data")
            .and_then(|v| v.get("items"))
            .and_then(|v| v.as_array());
        let Some(first) = items.and_then(|arr| arr.first()) else {
            return Ok(None);
        };

        let msg_type = first.get("msg_type").and_then(|v| v.as_str());
        let content = first
            .get("body")
            .and_then(|v| v.get("content"))
            .and_then(|v| v.as_str());
        let (Some(msg_type), Some(content)) = (msg_type, content) else {
            return Ok(None);
        };
        let text = extract_message_text(msg_type, content).trim().to_owned();
        if text.is_empty() {
            Ok(None)
        } else {
            Ok(Some(text))
        }
    }

    pub(super) async fn fetch_bot_open_id(&self) -> Result<Option<String>, FeishuError> {
        let token = self.get_access_token().await?;
        let url = format!("{}/bot/v3/info", self.api_base());
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;

        let value: Value = resp
            .json()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;
        let code = value.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
        if code != 0 {
            let msg = value
                .get("msg")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(FeishuError::Api {
                code,
                msg: msg.to_owned(),
            });
        }

        let bot_open_id = value
            .get("bot")
            .and_then(|v| v.get("open_id"))
            .and_then(|v| v.as_str())
            .or_else(|| {
                value
                    .get("data")
                    .and_then(|v| v.get("bot"))
                    .and_then(|v| v.get("open_id"))
                    .and_then(|v| v.as_str())
            });

        Ok(bot_open_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned))
    }

    #[cfg(test)]
    pub(super) async fn patch_message_text(
        &self,
        message_id: &str,
        text: &str,
    ) -> Result<(), FeishuError> {
        if message_id.is_empty() {
            return Ok(());
        }

        let token = self.get_access_token().await?;
        let url = format!("{}/im/v1/messages/{}", self.api_base(), message_id);
        let content = serde_json::json!({
            "text": text
        })
        .to_string();

        let patch_resp = self
            .http
            .patch(&url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({ "content": content }))
            .send()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;
        let patch_value: Value = patch_resp
            .json()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;
        let patch_code = patch_value
            .get("code")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        if patch_code == 0 {
            return Ok(());
        }

        // Fallback parity with Python implementation: some tenants require explicit msg_type.
        let token = self.get_access_token().await?;
        let update_resp = self
            .http
            .put(&url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({
                "msg_type": "text",
                "content": content,
            }))
            .send()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;
        let update_value: Value = update_resp
            .json()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;
        let update_code = update_value
            .get("code")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        if update_code == 0 {
            return Ok(());
        }

        let msg = update_value
            .get("msg")
            .and_then(|v| v.as_str())
            .or_else(|| patch_value.get("msg").and_then(|v| v.as_str()))
            .unwrap_or("unknown error");
        Err(FeishuError::Api {
            code: update_code,
            msg: msg.to_owned(),
        })
    }

    /// Create a Card Kit streaming card entity. Returns the card_id.
    pub(super) async fn create_cardkit_card(
        &self,
        initial_text: &str,
    ) -> Result<String, FeishuError> {
        let token = self.get_access_token().await?;
        let url = format!("{}/cardkit/v1/cards", self.api_base());
        let card_body = build_streaming_card_body(initial_text);

        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({
                "type": "card_json",
                "data": card_body,
            }))
            .send()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;

        let value: Value = resp
            .json()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;
        let code = value.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
        if code != 0 {
            let msg = value
                .get("msg")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            return Err(FeishuError::Api {
                code,
                msg: msg.to_owned(),
            });
        }
        let card_id = value
            .pointer("/data/card_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        if card_id.is_empty() {
            return Err(FeishuError::Api {
                code: -1,
                msg: "card_id missing in response".to_owned(),
            });
        }
        Ok(card_id)
    }

    /// Build the content string for referencing a Card Kit card in a message.
    pub(super) fn card_reference_content(card_id: &str) -> String {
        serde_json::json!({ "type": "card", "data": { "card_id": card_id } }).to_string()
    }

    /// Update a single markdown element of a Card Kit streaming card.
    pub(super) async fn update_cardkit_element(
        &self,
        card_id: &str,
        element_id: &str,
        text: &str,
        seq: u32,
    ) -> Result<(), FeishuError> {
        let token = self.get_access_token().await?;
        let url = format!(
            "{}/cardkit/v1/cards/{}/elements/{}/content",
            self.api_base(),
            card_id,
            element_id,
        );
        let uuid = format!("s_{card_id}_{seq}");

        let resp = self
            .http
            .put(&url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({
                "content": text,
                "sequence": seq,
                "uuid": uuid,
            }))
            .send()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;

        let value: Value = resp
            .json()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;
        let code = value.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
        if code != 0 {
            let msg = value
                .get("msg")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            return Err(FeishuError::Api {
                code,
                msg: msg.to_owned(),
            });
        }
        Ok(())
    }

    /// Close streaming mode on a Card Kit card and set the final text.
    pub(super) async fn close_cardkit_streaming(
        &self,
        card_id: &str,
        final_text: &str,
        seq: u32,
    ) -> Result<(), FeishuError> {
        // First do a final element update with the complete text
        self.update_cardkit_element(card_id, "content", final_text, seq)
            .await?;

        let token = self.get_access_token().await?;
        let url = format!("{}/cardkit/v1/cards/{}/settings", self.api_base(), card_id);
        let close_seq = seq + 1;
        let uuid = format!("c_{card_id}_{close_seq}");
        let summary = {
            let clean: String = final_text.chars().filter(|c| *c != '\n').take(50).collect();
            if clean.len() < final_text.chars().filter(|c| *c != '\n').count().min(50) {
                format!("{clean}...")
            } else {
                clean
            }
        };

        let settings = serde_json::json!({
            "config": {
                "streaming_mode": false,
                "summary": { "content": summary }
            }
        });

        let resp = self
            .http
            .patch(&url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({
                "settings": settings.to_string(),
                "sequence": close_seq,
                "uuid": uuid,
            }))
            .send()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;

        let value: Value = resp
            .json()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;
        let code = value.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
        if code != 0 {
            let msg = value
                .get("msg")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            return Err(FeishuError::Api {
                code,
                msg: msg.to_owned(),
            });
        }
        Ok(())
    }

    pub(super) async fn add_reaction(
        &self,
        message_id: &str,
        emoji_type: &str,
    ) -> Result<Option<String>, FeishuError> {
        if message_id.is_empty() || emoji_type.is_empty() {
            return Ok(None);
        }

        let token = self.get_access_token().await?;
        let url = format!(
            "{}/im/v1/messages/{}/reactions",
            self.api_base(),
            message_id
        );
        let body = serde_json::json!({
            "reaction_type": {
                "emoji_type": emoji_type
            }
        });
        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&body)
            .send()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;

        let reaction_resp: FeishuReactionResponse = resp
            .json()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;

        if reaction_resp.code != 0 {
            return Err(FeishuError::Api {
                code: reaction_resp.code,
                msg: reaction_resp.msg,
            });
        }

        Ok(reaction_resp.data.and_then(|d| {
            let id = d.reaction_id.trim();
            if id.is_empty() {
                None
            } else {
                Some(id.to_owned())
            }
        }))
    }

    /// Download a file/image/media resource attached to a Feishu message.
    ///
    /// Uses the Feishu IM API:
    /// `GET /im/v1/messages/:message_id/resources/:file_key?type=<type>`
    ///
    /// Returns the raw bytes on success.
    pub(super) async fn download_message_resource(
        &self,
        message_id: &str,
        file_key: &str,
        resource_type: &str,
    ) -> Result<Vec<u8>, FeishuError> {
        let token = self.get_access_token().await?;
        let url = format!(
            "{}/im/v1/messages/{}/resources/{}?type={}",
            self.api_base(),
            urlencoding::encode(message_id),
            urlencoding::encode(file_key),
            urlencoding::encode(resource_type),
        );
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(FeishuError::Http(format!(
                "resource download failed with status {}",
                resp.status()
            )));
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;
        Ok(bytes.to_vec())
    }

    pub(super) async fn remove_reaction(
        &self,
        message_id: &str,
        reaction_id: &str,
    ) -> Result<(), FeishuError> {
        if message_id.is_empty() || reaction_id.is_empty() {
            return Ok(());
        }

        let token = self.get_access_token().await?;
        let url = format!(
            "{}/im/v1/messages/{}/reactions/{}",
            self.api_base(),
            message_id,
            reaction_id
        );
        let resp = self
            .http
            .delete(&url)
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;

        let reaction_resp: FeishuReactionResponse = resp
            .json()
            .await
            .map_err(|e| FeishuError::Http(e.to_string()))?;
        if reaction_resp.code != 0 {
            return Err(FeishuError::Api {
                code: reaction_resp.code,
                msg: reaction_resp.msg,
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests;
