use std::collections::HashMap;

use chrono::Local;

use super::super::inbound::NativeThreadCommand;
use super::super::*;
use crate::recent_threads::{
    CurrentThreadDisplay, RECENT_THREADS_PAGE_SIZE, RecentThreadFilter, format_recent_thread_page,
    parse_recent_bind_request, parse_recent_page_request, recent_thread_display_title,
    requested_recent_page, resolve_recent_page,
};
use crate::store::ThreadStoreExt;
use crate::threads::{
    ThreadEnsureOptions, default_agent_for_channel_account, default_workspace_for_channel_account,
    default_workspace_mode_for_channel_account, label_from_value,
    validate_thread_accepts_bot_binding, worktree_base_dir_for_config,
};

const RECENT_THREADS_UNAVAILABLE: &str = "Recent threads are temporarily unavailable. Try again.";

impl MessageRouter {
    pub(super) async fn execute_native_thread_command(
        &mut self,
        channel: &str,
        account_id: &str,
        from_id: &str,
        is_group: bool,
        thread_binding_key: &str,
        extra_metadata: &HashMap<String, Value>,
        command_text: &str,
        command: NativeThreadCommand,
    ) -> Result<NativeThreadResult, String> {
        let binding_context_key =
            Self::build_binding_context_key(channel, account_id, thread_binding_key);

        let result = match command {
            NativeThreadCommand::ListRecent => {
                let request = match parse_recent_page_request(command_text) {
                    Ok(request) => request,
                    Err(error) => {
                        return Ok(self.reply_with_cached_thread(
                            error.to_string(),
                            channel,
                            account_id,
                            thread_binding_key,
                        ));
                    }
                };
                let Some(reader) = self.recent_thread_page_reader() else {
                    return Ok(self.reply_with_cached_thread(
                        RECENT_THREADS_UNAVAILABLE,
                        channel,
                        account_id,
                        thread_binding_key,
                    ));
                };
                let last_page = self
                    .recent_thread_browser
                    .last_successful_page(&binding_context_key);
                let requested_page = requested_recent_page(request, last_page);
                let requested_offset = requested_page
                    .saturating_sub(1)
                    .saturating_mul(RECENT_THREADS_PAGE_SIZE);
                let mut page = match reader
                    .page(
                        RecentThreadFilter::Exclude,
                        RECENT_THREADS_PAGE_SIZE,
                        requested_offset,
                    )
                    .await
                {
                    Ok(page) => page,
                    Err(_) => {
                        return Ok(self.reply_with_cached_thread(
                            RECENT_THREADS_UNAVAILABLE,
                            channel,
                            account_id,
                            thread_binding_key,
                        ));
                    }
                };
                let resolution = match resolve_recent_page(
                    request,
                    last_page,
                    page.total,
                    RECENT_THREADS_PAGE_SIZE,
                ) {
                    Ok(resolution) => resolution,
                    Err(error) => {
                        return Ok(self.reply_with_cached_thread(
                            error.to_string(),
                            channel,
                            account_id,
                            thread_binding_key,
                        ));
                    }
                };
                let resolved_offset = resolution
                    .page
                    .saturating_sub(1)
                    .saturating_mul(RECENT_THREADS_PAGE_SIZE);
                if page.offset != resolved_offset {
                    page = match reader
                        .page(
                            RecentThreadFilter::Exclude,
                            RECENT_THREADS_PAGE_SIZE,
                            resolved_offset,
                        )
                        .await
                    {
                        Ok(page) => page,
                        Err(_) => {
                            return Ok(self.reply_with_cached_thread(
                                RECENT_THREADS_UNAVAILABLE,
                                channel,
                                account_id,
                                thread_binding_key,
                            ));
                        }
                    };
                }

                let current_thread = self
                    .current_canonical_thread_for_binding(channel, account_id, thread_binding_key)
                    .await;
                let current_display = if let Some(thread_id) = current_thread.as_deref() {
                    // Display decoration only: a store failure falls back to
                    // the placeholder title instead of failing the listing.
                    let title = self
                        .threads
                        .get_logged(thread_id)
                        .await
                        .and_then(|record| label_from_value(&record))
                        .unwrap_or_else(|| "New Thread".to_owned());
                    Some(CurrentThreadDisplay {
                        thread_id: thread_id.to_owned(),
                        title,
                    })
                } else {
                    None
                };
                self.recent_thread_browser.record_successful_page(
                    &binding_context_key,
                    resolution,
                    &page,
                );
                NativeThreadResult {
                    reply_text: format_recent_thread_page(
                        &page,
                        resolution,
                        current_display.as_ref(),
                    ),
                    switched_thread: current_thread,
                }
            }
            NativeThreadCommand::BindRecent => {
                let request = match parse_recent_bind_request(command_text) {
                    Ok(request) => request,
                    Err(error) => {
                        return Ok(self.reply_with_cached_thread(
                            error.to_string(),
                            channel,
                            account_id,
                            thread_binding_key,
                        ));
                    }
                };
                let Some(reader) = self.recent_thread_page_reader() else {
                    return Ok(self.reply_with_cached_thread(
                        RECENT_THREADS_UNAVAILABLE,
                        channel,
                        account_id,
                        thread_binding_key,
                    ));
                };
                let selection = match self
                    .recent_thread_browser
                    .resolve_bind_request(&binding_context_key, &request)
                {
                    Ok(selection) => selection,
                    Err(error) => {
                        return Ok(self.reply_with_cached_thread(
                            error.to_string(),
                            channel,
                            account_id,
                            thread_binding_key,
                        ));
                    }
                };
                let target_thread = selection.thread_id;
                match reader.contains_selectable_thread(&target_thread).await {
                    Ok(true) => {}
                    Ok(false) => {
                        return Ok(self.reply_with_cached_thread(
                            "That thread no longer exists. Run /threads again.",
                            channel,
                            account_id,
                            thread_binding_key,
                        ));
                    }
                    Err(_) => {
                        return Ok(self.reply_with_cached_thread(
                            RECENT_THREADS_UNAVAILABLE,
                            channel,
                            account_id,
                            thread_binding_key,
                        ));
                    }
                }
                // A storage failure becomes a failed command, never a
                // "thread no longer exists" claim (#TASK-2130 class).
                let target_data = match self.threads.get(&target_thread).await {
                    Ok(Some(target_data)) => target_data,
                    Ok(None) => {
                        return Ok(self.reply_with_cached_thread(
                            "That thread no longer exists. Run /threads again.",
                            channel,
                            account_id,
                            thread_binding_key,
                        ));
                    }
                    Err(error) => {
                        return Err(format!("thread storage is unavailable: {error}"));
                    }
                };
                if let Err(error) = validate_thread_accepts_bot_binding(
                    &target_thread,
                    &target_data,
                    channel,
                    account_id,
                ) {
                    return Ok(NativeThreadResult {
                        reply_text: error,
                        switched_thread: self
                            .current_canonical_thread_for_binding(
                                channel,
                                account_id,
                                thread_binding_key,
                            )
                            .await,
                    });
                }
                let current_thread = self
                    .current_canonical_thread_for_binding(channel, account_id, thread_binding_key)
                    .await;
                let title = selection
                    .snapshot_title
                    .or_else(|| label_from_value(&target_data))
                    .unwrap_or_else(|| "New Thread".to_owned());
                let title = recent_thread_display_title(&title, "");
                if current_thread.as_deref() == Some(target_thread.as_str()) {
                    NativeThreadResult {
                        reply_text: format!("Already on thread: {title}"),
                        switched_thread: Some(target_thread),
                    }
                } else {
                    let binding = self
                        .endpoint_binding_from_inbound(
                            channel,
                            account_id,
                            thread_binding_key,
                            extra_metadata,
                            None,
                        )
                        .await;
                    self.bind_endpoint_runtime(&target_thread, binding)
                        .await
                        .map_err(|error| error.to_string())?;
                    self.switch_to_thread(&binding_context_key, &target_thread);
                    NativeThreadResult {
                        reply_text: format!("Switched to thread: {title}"),
                        switched_thread: Some(target_thread),
                    }
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
                let (new_thread_key, _thread_data) = self
                    .create_thread_with_options(options)
                    .await
                    .map_err(|error| format!("failed to create thread: {error}"))?;

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
                    .await
                    .map_err(|error| error.to_string())?;

                self.switch_to_thread(&binding_context_key, &new_thread_key);
                self.recent_thread_browser
                    .clear_context(&binding_context_key);

                NativeThreadResult {
                    reply_text: format!("Created and switched to new thread: {thread_name}"),
                    switched_thread: Some(new_thread_key),
                }
            }
            NativeThreadCommand::DeprecatedThreadPrev
            | NativeThreadCommand::DeprecatedThreadNext => {
                let current_thread = self
                    .current_canonical_thread_for_binding(channel, account_id, thread_binding_key)
                    .await;
                let (name, direction) = if command == NativeThreadCommand::DeprecatedThreadPrev {
                    ("/threadprev", "prev")
                } else {
                    ("/threadnext", "next")
                };
                NativeThreadResult {
                    reply_text: format!(
                        "{name} no longer switches threads. Use /threads {direction}, then /bindthread <n>."
                    ),
                    switched_thread: current_thread,
                }
            }
        };

        Ok(result)
    }

    fn reply_with_cached_thread(
        &self,
        reply_text: impl Into<String>,
        channel: &str,
        account_id: &str,
        thread_binding_key: &str,
    ) -> NativeThreadResult {
        NativeThreadResult {
            reply_text: reply_text.into(),
            switched_thread: self
                .get_current_thread_id_for_binding(channel, account_id, thread_binding_key)
                .map(ToOwned::to_owned),
        }
    }
}
