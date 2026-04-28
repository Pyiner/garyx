use std::sync::Arc;

use garyx_models::{MessageLedgerEvent, MessageLifecycleStatus, MessageTerminalReason};
use serde_json::Value;
use uuid::Uuid;

use crate::server::AppState;

#[derive(Debug, Clone, Default)]
pub(crate) struct RuntimeDiagnosticContext {
    pub(crate) ledger_id: Option<String>,
    pub(crate) bot_id: Option<String>,
    pub(crate) thread_id: Option<String>,
    pub(crate) run_id: Option<String>,
    pub(crate) channel: Option<String>,
    pub(crate) account_id: Option<String>,
    pub(crate) chat_id: Option<String>,
    pub(crate) from_id: Option<String>,
    pub(crate) native_message_id: Option<String>,
    pub(crate) text_excerpt: Option<String>,
    pub(crate) terminal_reason: Option<MessageTerminalReason>,
    pub(crate) reply_message_id: Option<String>,
    pub(crate) metadata: Option<Value>,
}

impl RuntimeDiagnosticContext {
    pub(crate) fn bot_id(&self) -> Option<String> {
        self.bot_id.clone().or_else(|| {
            match (self.channel.as_deref(), self.account_id.as_deref()) {
                (Some(channel), Some(account_id))
                    if !channel.trim().is_empty() && !account_id.trim().is_empty() =>
                {
                    Some(format!("{channel}:{account_id}"))
                }
                _ => None,
            }
        })
    }

    pub(crate) fn ledger_id(&self) -> String {
        if let Some(ledger_id) = self
            .ledger_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return ledger_id.to_owned();
        }
        if let Some(native_message_id) = self
            .native_message_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return format!(
                "{}:{}:{}",
                self.bot_id()
                    .unwrap_or_else(|| "unknown:unknown".to_owned()),
                self.chat_id.as_deref().unwrap_or("unknown-chat"),
                native_message_id
            );
        }
        if let Some(run_id) = self
            .run_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return format!("run:{run_id}");
        }
        format!("generated:{}", Uuid::new_v4())
    }
}

pub(crate) async fn record_message_ledger_event(
    state: &Arc<AppState>,
    status: MessageLifecycleStatus,
    context: RuntimeDiagnosticContext,
) {
    let Some(bot_id) = context.bot_id() else {
        return;
    };

    let event = MessageLedgerEvent {
        ledger_id: context.ledger_id(),
        bot_id,
        status,
        created_at: chrono::Utc::now().to_rfc3339(),
        thread_id: normalize_field(context.thread_id),
        run_id: normalize_field(context.run_id),
        channel: normalize_field(context.channel),
        account_id: normalize_field(context.account_id),
        chat_id: normalize_field(context.chat_id),
        from_id: normalize_field(context.from_id),
        native_message_id: normalize_field(context.native_message_id),
        text_excerpt: normalize_field(context.text_excerpt),
        terminal_reason: context.terminal_reason,
        reply_message_id: normalize_field(context.reply_message_id),
        metadata: context
            .metadata
            .filter(Value::is_object)
            .unwrap_or_else(|| Value::Object(Default::default())),
    };

    if let Err(error) = state.threads.message_ledger.append_event(event).await {
        tracing::warn!(error = %error, "failed to record message ledger event");
    }
}

fn normalize_field(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests;
