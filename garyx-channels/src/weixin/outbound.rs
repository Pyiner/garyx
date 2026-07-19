//! Weixin outbound senders: per-account [`WeixinSender`] and the
//! registry-facing [`WeixinChannelSender`] owning the shared running
//! flag. Moved verbatim from dispatcher.rs (Phase-6 B2b-2 pure code
//! motion).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use async_trait::async_trait;
use reqwest::Client;

use crate::channel_trait::ChannelError;
use crate::dispatcher::{
    ChannelInfo, OutboundChannelSender, OutboundMessage, OutboundSender, SendMessageResult,
    StreamDispatchCallback, StreamingDispatchTarget,
};
use crate::weixin;

#[derive(Clone)]
pub struct WeixinSender {
    pub account_id: String,
    pub account: garyx_models::config::WeixinAccount,
    pub http: Client,
    pub is_running: bool,
    pub running: Arc<std::sync::atomic::AtomicBool>,
}

impl WeixinSender {
    pub async fn send_text(
        &self,
        to_user_id: &str,
        text: &str,
        context_token: Option<&str>,
    ) -> Result<Vec<String>, ChannelError> {
        let message_id = crate::weixin::send_text_message(
            &self.http,
            &self.account,
            to_user_id,
            text,
            context_token,
        )
        .await?;
        Ok(vec![message_id])
    }

    pub async fn send_image(
        &self,
        to_user_id: &str,
        image_path: &Path,
        caption: Option<&str>,
        context_token: Option<&str>,
    ) -> Result<Vec<String>, ChannelError> {
        let message_id = crate::weixin::send_image_message_from_path(
            &self.http,
            &self.account,
            to_user_id,
            image_path,
            caption,
            context_token,
        )
        .await?;
        Ok(vec![message_id])
    }
}

#[async_trait]
impl OutboundSender for WeixinSender {
    async fn send_outbound(
        &self,
        request: OutboundMessage,
    ) -> Result<SendMessageResult, ChannelError> {
        let delivery_target_id = request.resolved_delivery_target_id();
        let context_token = weixin::get_context_token_for_thread(
            &request.account_id,
            &delivery_target_id,
            request.thread_id.as_deref(),
        )
        .await;
        if let Some(text) = request.text_content() {
            match self
                .send_text(&delivery_target_id, text, context_token.as_deref())
                .await
            {
                Ok(message_ids) => Ok(SendMessageResult { message_ids }),
                Err(error) => {
                    // Queue the failed message for later delivery when a
                    // fresh context_token arrives via an inbound message.
                    // Heuristic: only retry-queue on token-shaped errors;
                    // other failures propagate so the caller's retry
                    // policy can make its own decision.
                    let error_str = error.to_string();
                    let is_token_error = error_str.contains("ret=")
                        || error_str.contains("ret!=0")
                        || error_str.contains("context_token")
                        || error_str.contains("send limit");
                    if is_token_error {
                        weixin::queue_pending_outbound(
                            &request.account_id,
                            &delivery_target_id,
                            text,
                        )
                        .await;
                    }
                    Err(error)
                }
            }
        } else if let Some((image_path, _alt)) = request.image_content() {
            self.send_image(
                &delivery_target_id,
                Path::new(image_path),
                None,
                context_token.as_deref(),
            )
            .await
            .map(|message_ids| SendMessageResult { message_ids })
        } else if request.file_content().is_some() {
            Err(ChannelError::SendFailed(
                "file sending is currently supported only for telegram".to_owned(),
            ))
        } else {
            Ok(SendMessageResult::default())
        }
    }
}

#[derive(Clone)]
pub struct WeixinChannelSender {
    accounts: HashMap<String, WeixinSender>,
    running: Arc<AtomicBool>,
}

impl WeixinChannelSender {
    pub(crate) fn with_running(running: Arc<AtomicBool>) -> Self {
        Self {
            accounts: HashMap::new(),
            running,
        }
    }

    pub(crate) fn register(&mut self, mut sender: WeixinSender) {
        if self.accounts.is_empty() && !Arc::ptr_eq(&sender.running, &self.running) {
            self.running = sender.running.clone();
        } else {
            sender.running = self.running.clone();
        }
        self.accounts.insert(sender.account_id.clone(), sender);
    }
}

impl Default for WeixinChannelSender {
    fn default() -> Self {
        Self::with_running(Arc::new(AtomicBool::new(false)))
    }
}

#[async_trait]
impl OutboundChannelSender for WeixinChannelSender {
    fn channel_id(&self) -> &str {
        "weixin"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["wechat"]
    }

    fn accounts(&self) -> Vec<ChannelInfo> {
        self.accounts
            .values()
            .map(|sender| ChannelInfo {
                channel: self.channel_id().to_owned(),
                account_id: sender.account_id.clone(),
                is_running: sender.is_running,
            })
            .collect()
    }

    fn running_handle(&self) -> Option<Arc<AtomicBool>> {
        Some(self.running.clone())
    }

    async fn dispatch(&self, request: OutboundMessage) -> Result<SendMessageResult, ChannelError> {
        let sender = self.accounts.get(&request.account_id).ok_or_else(|| {
            ChannelError::Config(format!(
                "Weixin account '{}' not registered in dispatcher",
                request.account_id
            ))
        })?;
        sender.send_outbound(request).await
    }

    fn build_stream_event_callback(
        &self,
        target: StreamingDispatchTarget,
    ) -> Option<StreamDispatchCallback> {
        let sender = self.accounts.get(&target.account_id)?;
        let stream_callback = crate::weixin::build_weixin_response_callback(
            crate::weixin::WeixinStreamingCallbackConfig {
                http: sender.http.clone(),
                account: sender.account.clone(),
                account_id: sender.account_id.clone(),
                user_id: target.delivery_target_id.clone(),
                context_token: String::new(),
                thread_id: target.target_thread_id.clone(),
                typing_ticket: None,
                running: sender.running.clone(),
            },
        );
        Some(Arc::new(move |envelope| {
            stream_callback(envelope.event);
        }))
    }
}

impl crate::outbound_registry::AccountRegistration for WeixinSender {
    type Host = WeixinChannelSender;

    fn register_into(self, host: &mut WeixinChannelSender) {
        host.register(self);
    }
}
