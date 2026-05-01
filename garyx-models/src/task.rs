use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const TASK_SCHEMA_VERSION_V1: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadTask {
    #[serde(default = "default_task_schema_version")]
    pub schema_version: u32,
    pub scope: TaskScope,
    pub number: u64,
    pub title: String,
    pub status: TaskStatus,
    pub creator: Principal,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<Principal>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub updated_by: Principal,
    #[serde(default)]
    pub events: Vec<TaskEvent>,
}

fn default_task_schema_version() -> u32 {
    TASK_SCHEMA_VERSION_V1
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskScope {
    pub channel: String,
    pub account_id: String,
}

impl TaskScope {
    pub fn new(channel: impl Into<String>, account_id: impl Into<String>) -> Self {
        Self {
            channel: normalize_scope_part(channel.into()),
            account_id: normalize_scope_part(account_id.into()),
        }
    }

    pub fn canonical(&self) -> String {
        format!("{}/{}", self.channel, self.account_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Todo,
    InProgress,
    InReview,
    Done,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::InProgress => "in_progress",
            Self::InReview => "in_review",
            Self::Done => "done",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Principal {
    Human { user_id: String },
    Agent { agent_id: String },
}

impl Principal {
    pub fn id(&self) -> &str {
        match self {
            Self::Human { user_id } => user_id,
            Self::Agent { agent_id } => agent_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskEvent {
    pub event_id: String,
    pub at: DateTime<Utc>,
    pub actor: Principal,
    pub kind: TaskEventKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum TaskEventKind {
    Created {
        initial_status: TaskStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        assignee: Option<Principal>,
    },
    Promoted {
        initial_status: TaskStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        assignee: Option<Principal>,
    },
    Claimed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Principal>,
    },
    Released {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous_assignee: Option<Principal>,
    },
    Assigned {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<Principal>,
        to: Principal,
    },
    Unassigned {
        from: Principal,
    },
    StatusChanged {
        from: TaskStatus,
        to: TaskStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    Reopened {
        from: TaskStatus,
    },
    TitleChanged {
        from: String,
        to: String,
    },
}

pub fn normalize_scope_part(value: String) -> String {
    value.trim().to_ascii_lowercase()
}
