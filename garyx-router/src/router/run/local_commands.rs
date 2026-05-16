use std::collections::HashMap;

use chrono::{Local, Utc};
use garyx_models::routing::DELIVERY_TARGET_TYPE_CHAT_ID;
use serde_json::{Map, Value, json};

use super::super::inbound::NativeThreadCommand;
use super::super::*;
use crate::threads::{
    ChannelBinding, ThreadEnsureOptions, default_agent_for_channel_account,
    default_workspace_for_channel_account, loop_enabled_from_value,
};

impl MessageRouter {
    pub(super) async fn execute_native_thread_command(
        &mut self,
        channel: &str,
        account_id: &str,
        from_id: &str,
        is_group: bool,
        thread_binding_key: &str,
        command: NativeThreadCommand,
    ) -> Result<NativeThreadResult, String> {
        let binding_context_key =
            Self::build_binding_context_key(channel, account_id, thread_binding_key);
        let current_thread = self
            .current_canonical_thread_for_binding(channel, account_id, thread_binding_key)
            .await;

        let result = match command {
            NativeThreadCommand::Threads => {
                let threads = self
                    .list_user_threads_for_account(channel, account_id, thread_binding_key)
                    .await;
                let mut display: Vec<(String, Option<String>)> = threads
                    .into_iter()
                    .map(|entry| (entry.thread_id, entry.label))
                    .collect();
                if let Some(current) = current_thread.as_ref()
                    && !display.iter().any(|(key, _)| key == current)
                {
                    display.insert(0, (current.clone(), None));
                }

                let mut lines = vec!["Your Threads:".to_owned()];
                if display.is_empty() {
                    lines.push("No named threads yet.".to_owned());
                } else {
                    for (idx, (thread_id, label)) in display.iter().enumerate() {
                        let marker = if current_thread
                            .as_deref()
                            .is_some_and(|current| current == thread_id)
                        {
                            " ⬅️"
                        } else {
                            ""
                        };
                        let shown = label.as_deref().unwrap_or(thread_id);
                        lines.push(format!("{}. {shown}{marker}", idx + 1));
                    }
                }
                lines.push(String::new());
                lines.push("Use /newthread to create a thread.".to_owned());

                NativeThreadResult {
                    reply_text: lines.join("\n"),
                    switched_thread: current_thread,
                }
            }
            NativeThreadCommand::New => {
                let thread_name = Local::now().format("thread-%Y%m%d-%H%M%S").to_string();
                let options = ThreadEnsureOptions {
                    label: Some(thread_name.clone()),
                    workspace_dir: default_workspace_for_channel_account(
                        &self.config,
                        channel,
                        account_id,
                    ),
                    workspace_mode: Default::default(),
                    worktree_base_dir: None,
                    agent_id: default_agent_for_channel_account(&self.config, channel, account_id),
                    metadata: HashMap::new(),
                    provider_type: None,
                    sdk_session_id: None,
                    thread_kind: None,
                    origin_channel: Some(channel.to_owned()),
                    origin_account_id: Some(account_id.to_owned()),
                    origin_from_id: Some(from_id.to_owned()),
                    is_group: Some(is_group),
                };
                let Ok((new_thread_key, _thread_data)) =
                    self.create_thread_with_options(options).await
                else {
                    return Err("failed to create thread".to_owned());
                };

                let binding = self
                    .endpoint_binding_for_thread(
                        channel,
                        account_id,
                        thread_binding_key,
                        Some(&new_thread_key),
                    )
                    .await
                    .unwrap_or(ChannelBinding {
                        channel: channel.to_owned(),
                        account_id: account_id.to_owned(),
                        binding_key: thread_binding_key.to_owned(),
                        chat_id: thread_binding_key.to_owned(),
                        delivery_target_type: DELIVERY_TARGET_TYPE_CHAT_ID.to_owned(),
                        delivery_target_id: thread_binding_key.to_owned(),
                        display_label: thread_name.clone(),
                        last_inbound_at: Some(Utc::now().to_rfc3339()),
                        last_delivery_at: None,
                    });
                self.bind_endpoint_runtime(&new_thread_key, binding.clone())
                    .await?;

                self.switch_to_thread(&binding_context_key, &new_thread_key);

                NativeThreadResult {
                    reply_text: format!("Created and switched to new thread: {thread_name}"),
                    switched_thread: Some(new_thread_key),
                }
            }
            NativeThreadCommand::ThreadPrev | NativeThreadCommand::ThreadNext => {
                let direction = if command == NativeThreadCommand::ThreadPrev {
                    -1
                } else {
                    1
                };
                let switched = self
                    .navigate_thread_with_rebuild(
                        &binding_context_key,
                        NavigationContext {
                            channel,
                            account_id,
                            thread_binding_key,
                        },
                        direction,
                    )
                    .await;
                if let Some(target_thread) = switched.as_deref()
                    && let Some(binding) = self
                        .endpoint_binding_for_thread(
                            channel,
                            account_id,
                            thread_binding_key,
                            Some(target_thread),
                        )
                        .await
                {
                    self.bind_endpoint_runtime(target_thread, binding.clone())
                        .await?;
                }
                let reply_text = match (command, switched.as_ref()) {
                    (NativeThreadCommand::ThreadPrev, Some(key)) => {
                        format!("Switched to previous thread: {key}")
                    }
                    (NativeThreadCommand::ThreadNext, Some(key)) => {
                        format!("Switched to next thread: {key}")
                    }
                    (NativeThreadCommand::ThreadPrev, None) => {
                        "Already at the oldest thread.".to_owned()
                    }
                    (NativeThreadCommand::ThreadNext, None) => {
                        "Already at the newest thread.".to_owned()
                    }
                    _ => String::new(),
                };

                NativeThreadResult {
                    reply_text,
                    switched_thread: switched,
                }
            }
        };

        Ok(result)
    }

    pub(super) async fn toggle_loop_mode(&self, thread_id: &str) -> (String, String) {
        let current = self
            .threads
            .get(thread_id)
            .await
            .map(|v| loop_enabled_from_value(&v))
            .unwrap_or(false);

        let new_state = !current;

        if let Some(mut data) = self.threads.get(thread_id).await
            && let Some(obj) = data.as_object_mut()
        {
            obj.insert("loop_enabled".to_owned(), Value::Bool(new_state));
            if !new_state {
                obj.insert("loop_iteration_count".to_owned(), json!(0));
            }
            self.threads.set(thread_id, data).await;
        }

        let reply = if new_state {
            "\u{1f504} Loop mode enabled. The agent will auto-continue after each run until you send /loop again or the agent calls stop_loop.".to_owned()
        } else {
            "\u{23f9}\u{fe0f} Loop mode disabled.".to_owned()
        };

        (reply, thread_id.to_owned())
    }

    pub(super) async fn update_goal_mode(
        &self,
        thread_id: &str,
        command_text: &str,
    ) -> (String, String) {
        let argument = goal_command_argument(command_text);
        let now = Utc::now().to_rfc3339();
        let mut data = self
            .threads
            .get(thread_id)
            .await
            .unwrap_or_else(|| json!({}));
        let Some(obj) = data.as_object_mut() else {
            return (
                "Goal command failed: thread data is not an object.".to_owned(),
                thread_id.to_owned(),
            );
        };

        if argument.is_empty() {
            let reply = current_goal_reply(obj);
            self.threads.set(thread_id, data).await;
            return (reply, thread_id.to_owned());
        }

        let normalized_argument = argument.to_ascii_lowercase();
        match normalized_argument.as_str() {
            "off" | "clear" | "stop" | "done" | "complete" | "completed" => {
                obj.remove("goal");
                if let Some(metadata) = ensure_metadata_object(obj) {
                    metadata.remove("goal");
                }
                obj.insert("loop_enabled".to_owned(), Value::Bool(false));
                obj.insert("loop_iteration_count".to_owned(), json!(0));
                obj.insert("updated_at".to_owned(), Value::String(now));
                self.threads.set(thread_id, data).await;
                (
                    "Goal cleared. Loop mode disabled.".to_owned(),
                    thread_id.to_owned(),
                )
            }
            "pause" | "paused" => {
                if let Some(goal) = current_goal_object_mut(obj) {
                    goal.insert("status".to_owned(), Value::String("paused".to_owned()));
                    goal.insert("updated_at".to_owned(), Value::String(now.clone()));
                    sync_goal_metadata(obj);
                    obj.insert("loop_enabled".to_owned(), Value::Bool(false));
                    obj.insert("loop_iteration_count".to_owned(), json!(0));
                    obj.insert("updated_at".to_owned(), Value::String(now));
                    self.threads.set(thread_id, data).await;
                    (
                        "Goal paused. Loop mode disabled.".to_owned(),
                        thread_id.to_owned(),
                    )
                } else {
                    (
                        "No goal is set for this thread.".to_owned(),
                        thread_id.to_owned(),
                    )
                }
            }
            "resume" | "active" => {
                if let Some(goal) = current_goal_object_mut(obj) {
                    goal.insert("status".to_owned(), Value::String("active".to_owned()));
                    goal.insert("updated_at".to_owned(), Value::String(now.clone()));
                    sync_goal_metadata(obj);
                    obj.insert("loop_enabled".to_owned(), Value::Bool(true));
                    obj.insert("loop_iteration_count".to_owned(), json!(0));
                    obj.insert("updated_at".to_owned(), Value::String(now));
                    self.threads.set(thread_id, data).await;
                    (
                        "Goal resumed. Loop mode enabled.".to_owned(),
                        thread_id.to_owned(),
                    )
                } else {
                    (
                        "No goal is set for this thread.".to_owned(),
                        thread_id.to_owned(),
                    )
                }
            }
            _ => {
                let mut goal = current_goal_object(obj).unwrap_or_default();
                goal.entry("created_at".to_owned())
                    .or_insert_with(|| Value::String(now.clone()));
                goal.insert("objective".to_owned(), Value::String(argument));
                goal.insert("status".to_owned(), Value::String("active".to_owned()));
                goal.insert("updated_at".to_owned(), Value::String(now.clone()));
                let goal_value = Value::Object(goal);
                obj.insert("goal".to_owned(), goal_value.clone());
                if let Some(metadata) = ensure_metadata_object(obj) {
                    metadata.insert("goal".to_owned(), goal_value);
                }
                obj.insert("loop_enabled".to_owned(), Value::Bool(true));
                obj.insert("loop_iteration_count".to_owned(), json!(0));
                obj.insert("updated_at".to_owned(), Value::String(now));
                self.threads.set(thread_id, data).await;
                (
                    "Goal set. Loop mode enabled.".to_owned(),
                    thread_id.to_owned(),
                )
            }
        }
    }
}

fn goal_command_argument(command_text: &str) -> String {
    let trimmed = command_text.trim();
    let Some((_, rest)) = trimmed.split_once(char::is_whitespace) else {
        return String::new();
    };
    rest.trim().to_owned()
}

fn ensure_metadata_object(object: &mut Map<String, Value>) -> Option<&mut Map<String, Value>> {
    if !object.get("metadata").is_some_and(Value::is_object) {
        object.insert("metadata".to_owned(), Value::Object(Map::new()));
    }
    object.get_mut("metadata").and_then(Value::as_object_mut)
}

fn current_goal_object(object: &Map<String, Value>) -> Option<Map<String, Value>> {
    object
        .get("goal")
        .or_else(|| object.get("metadata").and_then(|value| value.get("goal")))
        .and_then(Value::as_object)
        .cloned()
}

fn current_goal_object_mut(object: &mut Map<String, Value>) -> Option<&mut Map<String, Value>> {
    if !object.get("goal").is_some_and(Value::is_object)
        && let Some(goal) = object
            .get("metadata")
            .and_then(|value| value.get("goal"))
            .cloned()
    {
        object.insert("goal".to_owned(), goal);
    }
    object.get_mut("goal").and_then(Value::as_object_mut)
}

fn sync_goal_metadata(object: &mut Map<String, Value>) {
    let Some(goal) = object.get("goal").cloned() else {
        return;
    };
    if let Some(metadata) = ensure_metadata_object(object) {
        metadata.insert("goal".to_owned(), goal);
    }
}

fn current_goal_reply(object: &Map<String, Value>) -> String {
    let Some(goal) = current_goal_object(object) else {
        return "No goal is set for this thread.".to_owned();
    };
    let objective = goal
        .get("objective")
        .and_then(Value::as_str)
        .unwrap_or("(missing objective)");
    let status = goal
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("active");
    format!("Current goal [{status}]: {objective}")
}
