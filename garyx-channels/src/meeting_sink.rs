use std::sync::Arc;

use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinedMeeting {
    pub feishu_meeting_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MeetingApiError {
    #[error("bot is not in the meeting")]
    NotInMeeting,
    #[error("meeting API authentication failed (code={code}): {message}")]
    AuthFailed { code: i64, message: String },
    #[error("meeting API transport failed: {0}")]
    RetriableTransport(String),
    #[error("meeting API failed (code={code}): {message}")]
    Other {
        code: i64,
        message: String,
        meeting_id: Option<String>,
    },
}

impl MeetingApiError {
    pub fn meeting_id(&self) -> Option<&str> {
        match self {
            Self::Other { meeting_id, .. } => meeting_id.as_deref(),
            _ => None,
        }
    }

    pub fn failure_kind(&self) -> Option<&'static str> {
        match self {
            Self::AuthFailed { .. } => Some("auth"),
            Self::RetriableTransport(_) => Some("transport"),
            Self::NotInMeeting | Self::Other { .. } => None,
        }
    }
}

#[async_trait]
pub trait MeetingPlatformClient: Send + Sync {
    async fn join(
        &self,
        meeting_no: &str,
        password: Option<&str>,
    ) -> Result<JoinedMeeting, MeetingApiError>;

    async fn leave(&self, feishu_meeting_id: &str) -> Result<(), MeetingApiError>;

    /// The channel resolves this once per WS runtime. The default keeps test
    /// clients and non-Feishu assemblies source-compatible.
    fn bot_open_id(&self) -> Option<String> {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingInvite {
    pub account_id: String,
    pub event_id: String,
    pub meeting_reference_id: String,
    pub meeting_no: String,
    pub topic: String,
    pub bot_id: String,
    pub inviter_id: String,
}

pub trait MeetingEventSink: Send + Sync {
    fn register_client(&self, account_id: &str, client: Arc<dyn MeetingPlatformClient>);
    fn unregister_client(&self, account_id: &str);
    fn on_meeting_invited(&self, invite: MeetingInvite);
    fn on_meeting_activity(&self, account_id: &str, event_id: &str, payload: serde_json::Value);
    fn on_meeting_ended(&self, account_id: &str, feishu_meeting_id: &str);
}

#[derive(Debug, Default)]
pub struct NoopMeetingEventSink;

impl MeetingEventSink for NoopMeetingEventSink {
    fn register_client(&self, _account_id: &str, _client: Arc<dyn MeetingPlatformClient>) {}
    fn unregister_client(&self, _account_id: &str) {}
    fn on_meeting_invited(&self, _invite: MeetingInvite) {}
    fn on_meeting_activity(&self, _account_id: &str, _event_id: &str, _payload: serde_json::Value) {
    }
    fn on_meeting_ended(&self, _account_id: &str, _feishu_meeting_id: &str) {}
}

pub fn noop_meeting_event_sink() -> Arc<dyn MeetingEventSink> {
    Arc::new(NoopMeetingEventSink)
}
