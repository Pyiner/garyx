use std::collections::HashMap;

use chrono::{Local, Utc};
use garyx_models::routing::DELIVERY_TARGET_TYPE_CHAT_ID;
use serde_json::{Value, json};

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
                if let Some(current) = current_thread.as_ref() {
                    if !display.iter().any(|(key, _)| key == current) {
                        display.insert(0, (current.clone(), None));
                    }
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
                if let Some(target_thread) = switched.as_deref() {
                    if let Some(binding) = self
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

        if let Some(mut data) = self.threads.get(thread_id).await {
            if let Some(obj) = data.as_object_mut() {
                obj.insert("loop_enabled".to_owned(), Value::Bool(new_state));
                if !new_state {
                    obj.insert("loop_iteration_count".to_owned(), json!(0));
                }
                self.threads.set(thread_id, data).await;
            }
        }

        let reply = if new_state {
            "\u{1f504} Loop mode enabled. The agent will auto-continue after each run until you send /loop again or the agent calls stop_loop.".to_owned()
        } else {
            "\u{23f9}\u{fe0f} Loop mode disabled.".to_owned()
        };

        (reply, thread_id.to_owned())
    }
}
