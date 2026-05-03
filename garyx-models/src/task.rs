use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const TASK_SCHEMA_VERSION_V1: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadTask {
    #[serde(default = "default_task_schema_version")]
    pub schema_version: u32,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Principal {
    Human { user_id: String },
    Agent { agent_id: String },
}

impl<'de> Deserialize<'de> for Principal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case", tag = "kind")]
        enum PrincipalWire {
            Human { user_id: String },
            Agent { agent_id: String },
        }

        let mut value = Value::deserialize(deserializer)?;
        promote_legacy_type_tag(&mut value);
        match PrincipalWire::deserialize(value).map_err(serde::de::Error::custom)? {
            PrincipalWire::Human { user_id } => Ok(Self::Human { user_id }),
            PrincipalWire::Agent { agent_id } => Ok(Self::Agent { agent_id }),
        }
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
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

impl<'de> Deserialize<'de> for TaskEventKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case", tag = "kind")]
        enum TaskEventKindWire {
            Created {
                initial_status: TaskStatus,
                #[serde(default)]
                assignee: Option<Principal>,
            },
            Promoted {
                initial_status: TaskStatus,
                #[serde(default)]
                assignee: Option<Principal>,
            },
            Claimed {
                #[serde(default)]
                from: Option<Principal>,
            },
            Released {
                #[serde(default)]
                previous_assignee: Option<Principal>,
            },
            Assigned {
                #[serde(default)]
                from: Option<Principal>,
                to: Principal,
            },
            Unassigned {
                from: Principal,
            },
            StatusChanged {
                from: TaskStatus,
                to: TaskStatus,
                #[serde(default)]
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

        let mut value = Value::deserialize(deserializer)?;
        promote_legacy_type_tag(&mut value);
        match TaskEventKindWire::deserialize(value).map_err(serde::de::Error::custom)? {
            TaskEventKindWire::Created {
                initial_status,
                assignee,
            } => Ok(Self::Created {
                initial_status,
                assignee,
            }),
            TaskEventKindWire::Promoted {
                initial_status,
                assignee,
            } => Ok(Self::Promoted {
                initial_status,
                assignee,
            }),
            TaskEventKindWire::Claimed { from } => Ok(Self::Claimed { from }),
            TaskEventKindWire::Released { previous_assignee } => {
                Ok(Self::Released { previous_assignee })
            }
            TaskEventKindWire::Assigned { from, to } => Ok(Self::Assigned { from, to }),
            TaskEventKindWire::Unassigned { from } => Ok(Self::Unassigned { from }),
            TaskEventKindWire::StatusChanged { from, to, note } => {
                Ok(Self::StatusChanged { from, to, note })
            }
            TaskEventKindWire::Reopened { from } => Ok(Self::Reopened { from }),
            TaskEventKindWire::TitleChanged { from, to } => Ok(Self::TitleChanged { from, to }),
        }
    }
}

fn promote_legacy_type_tag(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if object.contains_key("kind") {
        return;
    }
    if let Some(tag) = object.remove("type") {
        object.insert("kind".to_owned(), tag);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn principal_serializes_kind_and_accepts_legacy_type() {
        let principal = Principal::Agent {
            agent_id: "codex".to_owned(),
        };
        assert_eq!(
            serde_json::to_value(&principal).unwrap(),
            json!({ "kind": "agent", "agent_id": "codex" })
        );
        let decoded: Principal =
            serde_json::from_value(json!({ "type": "human", "user_id": "owner" })).unwrap();
        assert_eq!(
            decoded,
            Principal::Human {
                user_id: "owner".to_owned()
            }
        );
    }

    #[test]
    fn task_event_kind_serializes_kind_and_accepts_legacy_type() {
        let kind = TaskEventKind::StatusChanged {
            from: TaskStatus::Todo,
            to: TaskStatus::InProgress,
            note: None,
        };
        assert_eq!(
            serde_json::to_value(&kind).unwrap(),
            json!({ "kind": "status_changed", "from": "todo", "to": "in_progress" })
        );
        let decoded: TaskEventKind = serde_json::from_value(json!({
            "type": "assigned",
            "from": null,
            "to": { "type": "agent", "agent_id": "cindy" }
        }))
        .unwrap();
        assert_eq!(
            decoded,
            TaskEventKind::Assigned {
                from: None,
                to: Principal::Agent {
                    agent_id: "cindy".to_owned()
                }
            }
        );
    }
}
