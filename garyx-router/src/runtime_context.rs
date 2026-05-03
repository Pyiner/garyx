use std::collections::{BTreeSet, HashMap};

use garyx_models::{Principal, ThreadTask};
use serde_json::{Map, Value};

use crate::tasks::{canonical_task_ref, task_from_record};
use crate::threads::{
    agent_id_from_value, bindings_from_value, label_from_value, thread_kind_from_value,
    workspace_dir_from_value,
};

pub fn build_runtime_context_metadata(
    thread_id: &str,
    thread_record: Option<&Value>,
    metadata: &HashMap<String, Value>,
    channel: &str,
    account_id: &str,
    from_id: &str,
    workspace_dir: Option<&str>,
) -> Value {
    let mut context = metadata
        .get("runtime_context")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    insert_string(&mut context, "channel", channel);
    insert_string(&mut context, "account_id", account_id);
    insert_string(&mut context, "from_id", from_id);
    if let Some(is_group) = metadata.get("is_group").and_then(Value::as_bool) {
        context.insert("is_group".to_owned(), Value::Bool(is_group));
    }
    insert_string(&mut context, "thread_id", thread_id);

    let resolved_workspace = workspace_dir
        .and_then(non_empty_owned)
        .or_else(|| thread_record.and_then(workspace_dir_from_value));
    match resolved_workspace {
        Some(workspace_dir) => {
            context.insert("workspace_dir".to_owned(), Value::String(workspace_dir));
        }
        None => {
            context
                .entry("workspace_dir".to_owned())
                .or_insert(Value::Null);
        }
    }

    if let Some(bot_id) = bot_id(channel, account_id) {
        context.insert("bot_id".to_owned(), Value::String(bot_id));
    }
    if let Some(bot) = build_bot_context(metadata, channel, account_id) {
        context.insert("bot".to_owned(), bot);
    }

    if let Some(thread_record) = thread_record {
        context.insert(
            "thread".to_owned(),
            build_thread_context(thread_id, thread_record),
        );
        if let Ok(Some(task)) = task_from_record(thread_record) {
            context.insert("task".to_owned(), build_task_context(&task));
        }
    }

    Value::Object(context)
}

fn build_bot_context(
    metadata: &HashMap<String, Value>,
    channel: &str,
    account_id: &str,
) -> Option<Value> {
    let mut bot = Map::new();
    if let Some(bot_id) = bot_id(channel, account_id) {
        bot.insert("id".to_owned(), Value::String(bot_id));
    }
    insert_string(&mut bot, "channel", channel);
    insert_string(&mut bot, "account_id", account_id);
    insert_metadata_string(&mut bot, metadata, "thread_binding_key");
    insert_metadata_string(&mut bot, metadata, "chat_id");
    insert_metadata_string(&mut bot, metadata, "display_label");
    insert_metadata_string(&mut bot, metadata, "delivery_target_type");
    insert_metadata_string(&mut bot, metadata, "delivery_target_id");
    insert_metadata_string(&mut bot, metadata, "delivery_thread_id");
    if let Some(is_group) = metadata.get("is_group").and_then(Value::as_bool) {
        bot.insert("is_group".to_owned(), Value::Bool(is_group));
    }

    (!bot.is_empty()).then_some(Value::Object(bot))
}

fn build_thread_context(thread_id: &str, thread_record: &Value) -> Value {
    let mut thread = Map::new();
    insert_string(&mut thread, "id", thread_id);
    insert_optional_string(&mut thread, "label", label_from_value(thread_record));
    insert_optional_string(&mut thread, "kind", thread_kind_from_value(thread_record));
    insert_optional_string(&mut thread, "agent_id", agent_id_from_value(thread_record));
    insert_optional_string(
        &mut thread,
        "workspace_dir",
        workspace_dir_from_value(thread_record),
    );
    for key in ["channel", "account_id", "from_id"] {
        insert_value_string(&mut thread, key, thread_record.get(key));
    }
    if let Some(is_group) = thread_record.get("is_group").and_then(Value::as_bool) {
        thread.insert("is_group".to_owned(), Value::Bool(is_group));
    }
    if let Some(provider_type) = thread_record.get("provider_type") {
        thread.insert("provider_type".to_owned(), provider_type.clone());
    }

    let bindings = bindings_from_value(thread_record);
    if !bindings.is_empty() {
        let mut bound_bots = BTreeSet::new();
        let values = bindings
            .into_iter()
            .map(|binding| {
                let mut value = Map::new();
                insert_string(&mut value, "channel", &binding.channel);
                insert_string(&mut value, "account_id", &binding.account_id);
                if let Some(bot_id) = bot_id(&binding.channel, &binding.account_id) {
                    bound_bots.insert(bot_id.clone());
                    value.insert("bot_id".to_owned(), Value::String(bot_id));
                }
                insert_string(&mut value, "binding_key", &binding.binding_key);
                insert_string(&mut value, "chat_id", &binding.chat_id);
                insert_string(
                    &mut value,
                    "delivery_target_type",
                    &binding.delivery_target_type,
                );
                insert_string(
                    &mut value,
                    "delivery_target_id",
                    &binding.delivery_target_id,
                );
                insert_string(&mut value, "display_label", &binding.display_label);
                if let Some(last_inbound_at) = binding.last_inbound_at {
                    insert_string(&mut value, "last_inbound_at", &last_inbound_at);
                }
                if let Some(last_delivery_at) = binding.last_delivery_at {
                    insert_string(&mut value, "last_delivery_at", &last_delivery_at);
                }
                Value::Object(value)
            })
            .collect::<Vec<_>>();
        thread.insert("channel_bindings".to_owned(), Value::Array(values));
        thread.insert(
            "bound_bots".to_owned(),
            Value::Array(bound_bots.into_iter().map(Value::String).collect()),
        );
    }

    Value::Object(thread)
}

fn build_task_context(task: &ThreadTask) -> Value {
    let mut value = Map::new();
    value.insert(
        "task_ref".to_owned(),
        Value::String(canonical_task_ref(task)),
    );
    value.insert("number".to_owned(), Value::Number(task.number.into()));
    insert_string(&mut value, "title", &task.title);
    value.insert(
        "status".to_owned(),
        Value::String(task.status.as_str().to_owned()),
    );
    value.insert("creator".to_owned(), principal_value(&task.creator));
    if let Some(assignee) = task.assignee.as_ref() {
        value.insert("assignee".to_owned(), principal_value(assignee));
    }
    value.insert(
        "updated_at".to_owned(),
        Value::String(task.updated_at.to_rfc3339()),
    );
    value.insert("updated_by".to_owned(), principal_value(&task.updated_by));
    Value::Object(value)
}

fn principal_value(principal: &Principal) -> Value {
    serde_json::to_value(principal).unwrap_or(Value::Null)
}

fn bot_id(channel: &str, account_id: &str) -> Option<String> {
    let channel = channel.trim();
    let account_id = account_id.trim();
    (!channel.is_empty() && !account_id.is_empty()).then(|| format!("{channel}:{account_id}"))
}

fn insert_metadata_string(
    target: &mut Map<String, Value>,
    metadata: &HashMap<String, Value>,
    key: &str,
) {
    if let Some(value) = metadata.get(key).and_then(value_to_string) {
        target.insert(key.to_owned(), Value::String(value));
    }
}

fn insert_value_string(target: &mut Map<String, Value>, key: &str, value: Option<&Value>) {
    if let Some(value) = value.and_then(value_to_string) {
        target.insert(key.to_owned(), Value::String(value));
    }
}

fn insert_optional_string(target: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value.and_then(|value| non_empty_owned(&value)) {
        target.insert(key.to_owned(), Value::String(value));
    }
}

fn insert_string(target: &mut Map<String, Value>, key: &str, value: &str) {
    if let Some(value) = non_empty_owned(value) {
        target.insert(key.to_owned(), Value::String(value));
    }
}

fn non_empty_owned(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => non_empty_owned(value),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use garyx_models::{Principal, TaskStatus};
    use serde_json::json;

    use super::*;

    #[test]
    fn builds_compact_thread_task_and_bot_context() {
        let now = Utc::now();
        let task = ThreadTask {
            schema_version: 1,
            number: 7,
            title: "Fix prompt context".to_owned(),
            status: TaskStatus::InProgress,
            creator: Principal::Human {
                user_id: "user42".to_owned(),
            },
            assignee: Some(Principal::Agent {
                agent_id: "codex".to_owned(),
            }),
            created_at: now,
            updated_at: now,
            updated_by: Principal::Agent {
                agent_id: "codex".to_owned(),
            },
            events: Vec::new(),
        };
        let record = json!({
            "thread_id": "thread::abc",
            "label": "Prompt work",
            "thread_kind": "single_agent",
            "agent_id": "codex",
            "channel": "telegram",
            "account_id": "bot1",
            "from_id": "user42",
            "is_group": false,
            "workspace_dir": "/repo",
            "channel_bindings": [{
                "channel": "telegram",
                "account_id": "bot1",
                "binding_key": "user42",
                "chat_id": "chat42",
                "delivery_target_type": "chat_id",
                "delivery_target_id": "chat42",
                "display_label": "Pyiner"
            }],
            "task": task,
        });
        let metadata = HashMap::from([
            ("is_group".to_owned(), Value::Bool(false)),
            (
                "thread_binding_key".to_owned(),
                Value::String("user42".to_owned()),
            ),
        ]);

        let context = build_runtime_context_metadata(
            "thread::abc",
            Some(&record),
            &metadata,
            "telegram",
            "bot1",
            "user42",
            None,
        );

        assert_eq!(context["thread_id"], "thread::abc");
        assert_eq!(context["account_id"], "bot1");
        assert_eq!(context["bot_id"], "telegram:bot1");
        assert_eq!(context["bot"]["thread_binding_key"], "user42");
        assert_eq!(context["thread"]["label"], "Prompt work");
        assert_eq!(context["thread"]["bound_bots"][0], "telegram:bot1");
        assert_eq!(context["task"]["task_ref"], "#TASK-7");
        assert_eq!(context["task"]["status"], "in_progress");
    }
}
