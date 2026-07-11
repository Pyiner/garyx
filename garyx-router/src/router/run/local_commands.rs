use std::collections::HashMap;

use chrono::Local;

use super::super::inbound::NativeThreadCommand;
use super::super::*;
use crate::threads::{
    ThreadEnsureOptions, default_agent_for_channel_account, default_workspace_for_channel_account,
    default_workspace_mode_for_channel_account, worktree_base_dir_for_config,
};

impl MessageRouter {
    pub(super) async fn execute_native_thread_command(
        &mut self,
        channel: &str,
        account_id: &str,
        from_id: &str,
        is_group: bool,
        thread_binding_key: &str,
        extra_metadata: &HashMap<String, Value>,
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
                let workspace_mode =
                    default_workspace_mode_for_channel_account(&self.config, channel, account_id);
                let options = ThreadEnsureOptions {
                    label: Some(thread_name.clone()),
                    workspace_dir: default_workspace_for_channel_account(
                        &self.config,
                        channel,
                        account_id,
                    ),
                    workspace_mode,
                    worktree_base_dir: workspace_mode
                        .is_worktree()
                        .then(|| worktree_base_dir_for_config(&self.config)),
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
                    .endpoint_binding_from_inbound(
                        channel,
                        account_id,
                        thread_binding_key,
                        extra_metadata,
                        Some(&thread_name),
                    )
                    .await;
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
                    let binding = self
                        .endpoint_binding_from_inbound(
                            channel,
                            account_id,
                            thread_binding_key,
                            extra_metadata,
                            None,
                        )
                        .await;
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
}
