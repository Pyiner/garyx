use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
struct BotSelection {
    bot: String,
    channel: String,
    account_id: String,
}

#[derive(Debug, Clone)]
struct ThreadBotBinding {
    selection: BotSelection,
    chat_id: String,
    delivery_target_type: String,
    delivery_target_id: String,
    delivery_thread_id: Option<String>,
}

impl BotSelection {
    fn new(channel: &str, account_id: &str) -> Self {
        Self {
            bot: format!("{channel}:{account_id}"),
            channel: channel.to_owned(),
            account_id: account_id.to_owned(),
        }
    }
}

impl GaryMcpServer {
    pub(super) async fn execute_message(
        &self,
        run_ctx: RunContext,
        params: MessageParams,
    ) -> Result<Value, String> {
        let state = &self.app_state;
        Self::require_auth(state, &run_ctx, params.token.as_deref())?;

        let action = params.action.as_deref().unwrap_or("send");
        if action != "send" {
            return Err(format!(
                "unsupported action: {action}. only 'send' is supported"
            ));
        }

        let mut text = params.text.clone().unwrap_or_default();
        if params.image.is_some() && params.file.is_some() {
            return Err("message supports at most one attachment: choose image or file".to_owned());
        }
        if text.trim().is_empty() && params.image.is_none() && params.file.is_none() {
            return Err("either text, image, or file is required".to_owned());
        }

        let target = self.resolve_message_target(&run_ctx, &params).await?;
        let run_id = params
            .run_id
            .clone()
            .or(run_ctx.run_id.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        if Self::is_scheduled_thread_key(target.thread_id.as_deref()) {
            text = Self::format_scheduled_message(&text, target.thread_id.as_deref());
        }

        if let Some(image) = params.image.as_deref() {
            let message_id = self
                .send_image_message(
                    &target,
                    image,
                    if text.trim().is_empty() {
                        None
                    } else {
                        Some(text.as_str())
                    },
                    params.reply_to.as_deref(),
                )
                .await?;
            if let Some(thread_id) = target.thread_id.as_deref() {
                let mut router = state.threads.router.lock().await;
                router
                    .record_outbound_message_with_thread_log(
                        thread_id,
                        &target.channel,
                        &target.account_id,
                        &target.chat_id,
                        target.delivery_thread_id.as_deref(),
                        &message_id,
                        None,
                    )
                    .await;
            }
            return Ok(json!({
                "tool": "message",
                "action": "send",
                "status": "ok",
                "run_id": run_id,
                "target": params.target,
                "bot": BotSelection::new(&target.channel, &target.account_id).bot,
                "channel": target.channel,
                "account_id": target.account_id,
                "chat_id": target.chat_id,
                "thread_id": target.thread_id,
                "text": text,
                "message_ids": [message_id],
                "image": params.image,
                "file": params.file,
            }));
        }

        if let Some(file_path) = params.file.as_deref() {
            let message_id = self
                .send_file_message(
                    &target,
                    file_path,
                    if text.trim().is_empty() {
                        None
                    } else {
                        Some(text.as_str())
                    },
                    params.reply_to.as_deref(),
                )
                .await?;
            if let Some(thread_id) = target.thread_id.as_deref() {
                let mut router = state.threads.router.lock().await;
                router
                    .record_outbound_message_with_thread_log(
                        thread_id,
                        &target.channel,
                        &target.account_id,
                        &target.chat_id,
                        target.delivery_thread_id.as_deref(),
                        &message_id,
                        None,
                    )
                    .await;
            }
            return Ok(json!({
                "tool": "message",
                "action": "send",
                "status": "ok",
                "run_id": run_id,
                "target": params.target,
                "bot": BotSelection::new(&target.channel, &target.account_id).bot,
                "channel": target.channel,
                "account_id": target.account_id,
                "chat_id": target.chat_id,
                "thread_id": target.thread_id,
                "text": text,
                "message_ids": [message_id],
                "image": params.image,
                "file": params.file,
            }));
        }

        let send_result = state
            .channel_dispatcher()
            .send_message(OutboundMessage {
                channel: target.channel.clone(),
                account_id: target.account_id.clone(),
                chat_id: target.chat_id.clone(),
                delivery_target_type: target.delivery_target_type.clone(),
                delivery_target_id: target.delivery_target_id.clone(),
                text: text.clone(),
                reply_to: params.reply_to.clone(),
                thread_id: target.delivery_thread_id.clone(),
            })
            .await
            .map_err(|e| format!("message delivery failed: {e}"))?;

        if let Some(thread_id) = target.thread_id.as_deref() {
            let mut router = state.threads.router.lock().await;
            for message_id in &send_result.message_ids {
                router
                    .record_outbound_message_with_thread_log(
                        thread_id,
                        &target.channel,
                        &target.account_id,
                        &target.chat_id,
                        target.delivery_thread_id.as_deref(),
                        message_id,
                        None,
                    )
                    .await;
            }
        }
        Ok(json!({
            "tool": "message",
            "action": "send",
            "status": "ok",
            "run_id": run_id,
            "target": params.target,
            "bot": BotSelection::new(&target.channel, &target.account_id).bot,
            "channel": target.channel,
            "account_id": target.account_id,
            "chat_id": target.chat_id,
            "thread_id": target.thread_id,
            "text": text,
            "message_ids": send_result.message_ids,
            "image": params.image,
            "file": params.file,
        }))
    }

    pub(super) fn record_tool_metric(&self, tool_name: &str, status: &str, elapsed: Duration) {
        let duration_ms = elapsed.as_millis().min(u128::from(u64::MAX)) as u64;
        self.app_state
            .ops
            .mcp_tool_metrics
            .record_call(tool_name, status, duration_ms);
        tracing::info!(
            metric = "mcp_tool_calls_total",
            tool = tool_name,
            status = status,
            value = 1u8,
            "mcp tool call recorded"
        );
        tracing::info!(
            metric = "mcp_tool_duration_ms",
            tool = tool_name,
            duration_ms = duration_ms,
            "mcp tool duration recorded"
        );
    }

    pub(super) async fn resolve_message_target(
        &self,
        run_ctx: &RunContext,
        params: &MessageParams,
    ) -> Result<ResolvedMessageTarget, String> {
        let state = &self.app_state;
        let requested_bot = self.resolve_requested_bot(params)?;
        let explicit_target = params
            .target
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());

        if let Some(bot) = requested_bot.as_ref() {
            if !explicit_target {
                let Some(endpoint) = crate::routes::resolve_main_endpoint_by_bot(
                    state,
                    &bot.channel,
                    &bot.account_id,
                )
                .await
                else {
                    return Err(format!("bot '{}' has no resolved main endpoint", bot.bot));
                };

                return Ok(ResolvedMessageTarget {
                    channel: endpoint.channel,
                    account_id: endpoint.account_id,
                    chat_id: endpoint.chat_id,
                    delivery_target_type: endpoint.delivery_target_type,
                    delivery_target_id: endpoint.delivery_target_id,
                    delivery_thread_id: endpoint.delivery_thread_id,
                    thread_id: endpoint.thread_id,
                });
            }
        }

        let target_input = if let Some(target) = params
            .target
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            target.to_owned()
        } else if requested_bot.is_none() {
            let Some(thread_id) = run_ctx.thread_id.clone() else {
                return Err(
                    "message tool requires current thread context or explicit target".to_owned(),
                );
            };
            thread_id
        } else {
            "last".to_owned()
        };
        let resolved_target = if target_input.starts_with("thread::") {
            target_input.as_str()
        } else {
            target_input
                .strip_prefix("thread:")
                .unwrap_or(target_input.as_str())
        };
        let thread_like_target = target_input == "last"
            || target_input.starts_with("thread:")
            || target_input.contains("::");

        if let Some(bot) = requested_bot.as_ref() {
            if !thread_like_target {
                return Ok(ResolvedMessageTarget {
                    channel: bot.channel.clone(),
                    account_id: bot.account_id.clone(),
                    chat_id: resolved_target.to_owned(),
                    delivery_target_type: "chat_id".to_owned(),
                    delivery_target_id: resolved_target.to_owned(),
                    delivery_thread_id: None,
                    thread_id: None,
                });
            }
        }

        // 1) Prefer in-memory delivery target resolution first.
        let resolved = if thread_like_target {
            resolve_delivery_target_with_recovery(&state.threads.router, resolved_target).await
        } else {
            let router = state.threads.router.lock().await;
            router.resolve_delivery_target(resolved_target)
        };
        if let Some((thread_id, delivery)) = resolved {
            if delivery.channel.is_empty()
                || delivery.account_id.is_empty()
                || delivery.chat_id.is_empty()
            {
                return Err(format!(
                    "delivery target resolved but incomplete for thread '{thread_id}'"
                ));
            }
            if let Some(bot) = requested_bot.as_ref() {
                if bot.channel == delivery.channel && bot.account_id == delivery.account_id {
                    return Ok(ResolvedMessageTarget {
                        channel: delivery.channel,
                        account_id: delivery.account_id,
                        chat_id: delivery.chat_id,
                        delivery_target_type: delivery.delivery_target_type,
                        delivery_target_id: delivery.delivery_target_id,
                        delivery_thread_id: delivery.thread_id,
                        thread_id: Some(thread_id),
                    });
                }

                if let Some(binding) = self.find_thread_binding_for_bot(&thread_id, bot).await {
                    return Ok(ResolvedMessageTarget {
                        channel: binding.selection.channel,
                        account_id: binding.selection.account_id,
                        chat_id: binding.chat_id,
                        delivery_target_type: binding.delivery_target_type,
                        delivery_target_id: binding.delivery_target_id,
                        delivery_thread_id: binding.delivery_thread_id,
                        thread_id: Some(thread_id),
                    });
                }

                let available_thread_bots = self.thread_bot_ids(&thread_id).await;
                let available_suffix = if available_thread_bots.is_empty() {
                    "no bot bindings were found on that thread".to_owned()
                } else {
                    format!(
                        "available thread bots: {}",
                        available_thread_bots.join(", ")
                    )
                };
                return Err(format!(
                    "thread '{thread_id}' is not bound to bot '{}'; {available_suffix}",
                    bot.bot
                ));
            }
            return Ok(ResolvedMessageTarget {
                channel: delivery.channel,
                account_id: delivery.account_id,
                chat_id: delivery.chat_id,
                delivery_target_type: delivery.delivery_target_type,
                delivery_target_id: delivery.delivery_target_id,
                delivery_thread_id: delivery.thread_id,
                thread_id: Some(thread_id),
            });
        }

        if thread_like_target {
            return Err(format!("unable to resolve delivery target: {target_input}"));
        }

        // 2) Fallback to explicit channel/account with target as chat id.
        if let (Some(channel), Some(account_id)) = (
            params.channel.as_deref().or(run_ctx.channel.as_deref()),
            params
                .account_id
                .as_deref()
                .or(run_ctx.account_id.as_deref()),
        ) {
            return Ok(ResolvedMessageTarget {
                channel: channel.to_owned(),
                account_id: account_id.to_owned(),
                chat_id: resolved_target.to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: resolved_target.to_owned(),
                delivery_thread_id: run_ctx.delivery_thread_id.clone(),
                thread_id: run_ctx.thread_id.clone(),
            });
        }

        // 3) Final fallback: current run context chat/user.
        if let (Some(channel), Some(account_id), Some(chat_id)) = (
            run_ctx.channel.as_deref(),
            run_ctx.account_id.as_deref(),
            run_ctx.from_id.as_deref(),
        ) {
            return Ok(ResolvedMessageTarget {
                channel: channel.to_owned(),
                account_id: account_id.to_owned(),
                chat_id: chat_id.to_owned(),
                delivery_target_type: "chat_id".to_owned(),
                delivery_target_id: chat_id.to_owned(),
                delivery_thread_id: run_ctx.delivery_thread_id.clone(),
                thread_id: run_ctx.thread_id.clone(),
            });
        }

        Err("message tool not available: no resolvable delivery target".to_owned())
    }

    pub(super) async fn status_payload(&self, run_ctx: RunContext) -> Result<Value, String> {
        let state = &self.app_state;
        let cfg = state.config_snapshot();
        let uptime_secs = state.runtime.start_time.elapsed().as_secs();
        let current_workspace_dir = if let Some(thread_id) = run_ctx.thread_id.as_deref() {
            state
                .threads
                .thread_store
                .get(thread_id)
                .await
                .and_then(|value| garyx_router::workspace_dir_from_value(&value))
        } else {
            None
        };

        let thread_keys = state.threads.thread_store.list_keys(None).await;
        let thread_count = thread_keys.len();

        let bridge = &state.integration.bridge;
        let keys = bridge.provider_keys().await;
        let default_key = bridge.default_provider_key().await;
        let mut list = Vec::new();
        for key in &keys {
            if let Some(p) = bridge.get_provider(key).await {
                list.push(json!({
                    "key": key,
                    "type": format!("{:?}", p.provider_type()),
                    "ready": p.is_ready(),
                    "default": default_key.as_deref() == Some(key.as_str()),
                }));
            }
        }

        let mut channels = Vec::new();
        for (plugin_id, plugin_cfg) in &cfg.channels.plugins {
            for (name, acct) in &plugin_cfg.accounts {
                channels.push(json!({
                    "name": name, "channel_type": plugin_id, "enabled": acct.enabled,
                }));
            }
        }
        for (name, acct) in &cfg.channels.api.accounts {
            channels.push(json!({
                "name": name, "channel_type": "api", "enabled": acct.enabled,
            }));
        }

        let (current_bot, thread_bound_bots, available_bots, other_bots) =
            self.status_bot_inventory(&cfg, &run_ctx).await;

        let cron_available = state.ops.cron_service.is_some();
        let cron_job_count = if let Some(svc) = &state.ops.cron_service {
            svc.list().await.len()
        } else {
            0
        };

        Ok(json!({
            "tool": "status",
            "status": "ok",
            "uptime_secs": uptime_secs,
            "threads": { "count": thread_count },
            "providers": list,
            "channels": channels,
            "bots": {
                "current": current_bot,
                "thread_bound": thread_bound_bots,
                "others": other_bots,
                "available": available_bots,
            },
            "cron": { "available": cron_available, "job_count": cron_job_count },
            "heartbeat": { "available": state.ops.heartbeat_service.is_some() },
            "gateway": { "port": cfg.gateway.port },
            "current_context": {
                "channel": run_ctx.channel,
                "thread_id": run_ctx.thread_id,
                "workspace_dir": current_workspace_dir,
            }
        }))
    }

    fn enabled_bots_from_config(
        &self,
        cfg: &garyx_models::config::GaryxConfig,
    ) -> Vec<BotSelection> {
        let mut bots = Vec::new();
        for (plugin_id, plugin_cfg) in &cfg.channels.plugins {
            for (account_id, account) in &plugin_cfg.accounts {
                if account.enabled {
                    bots.push(BotSelection::new(plugin_id, account_id));
                }
            }
        }
        for (account_id, account) in &cfg.channels.api.accounts {
            if account.enabled {
                bots.push(BotSelection::new("api", account_id));
            }
        }
        bots.sort_by(|a, b| a.bot.cmp(&b.bot));
        bots
    }

    fn resolve_requested_bot(
        &self,
        params: &MessageParams,
    ) -> Result<Option<BotSelection>, String> {
        let Some(raw_bot) = params
            .bot
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(None);
        };

        let cfg = self.app_state.config_snapshot();
        let available = self.enabled_bots_from_config(&cfg);
        let selected = Self::parse_bot_selection(raw_bot, &available)?;

        if let Some(channel) = params
            .channel
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if channel != selected.channel {
                return Err(format!(
                    "conflicting bot/channel parameters: bot '{}' uses channel '{}', but channel='{}' was requested",
                    selected.bot, selected.channel, channel
                ));
            }
        }
        if let Some(account_id) = params
            .account_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if account_id != selected.account_id {
                return Err(format!(
                    "conflicting bot/account parameters: bot '{}' uses account '{}', but account_id='{}' was requested",
                    selected.bot, selected.account_id, account_id
                ));
            }
        }

        Ok(Some(selected))
    }

    fn parse_bot_selection(
        raw_bot: &str,
        available: &[BotSelection],
    ) -> Result<BotSelection, String> {
        let raw_bot = raw_bot.trim();
        if let Some((channel, account_id)) =
            raw_bot.split_once(':').or_else(|| raw_bot.split_once('/'))
        {
            let channel = channel.trim();
            let account_id = account_id.trim();
            if channel.is_empty() || account_id.is_empty() {
                return Err(format!("invalid bot selector: '{raw_bot}'"));
            }
            if let Some(found) = available.iter().find(|candidate| {
                candidate.channel == channel && candidate.account_id == account_id
            }) {
                return Ok(found.clone());
            }
            return Err(format!(
                "unknown bot '{}'. Available bots: {}",
                raw_bot,
                Self::format_available_bots(available)
            ));
        }

        let matches = available
            .iter()
            .filter(|candidate| candidate.account_id == raw_bot)
            .cloned()
            .collect::<Vec<_>>();
        match matches.len() {
            1 => Ok(matches[0].clone()),
            0 => Err(format!(
                "unknown bot '{}'. Available bots: {}",
                raw_bot,
                Self::format_available_bots(available)
            )),
            _ => Err(format!(
                "ambiguous bot '{}'. Use channel:account_id. Matches: {}",
                raw_bot,
                matches
                    .iter()
                    .map(|candidate| candidate.bot.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    fn format_available_bots(available: &[BotSelection]) -> String {
        if available.is_empty() {
            "none".to_owned()
        } else {
            available
                .iter()
                .map(|candidate| candidate.bot.clone())
                .collect::<Vec<_>>()
                .join(", ")
        }
    }

    async fn load_thread_bot_bindings(&self, thread_id: &str) -> Vec<ThreadBotBinding> {
        let Some(thread_data) = self.app_state.threads.thread_store.get(thread_id).await else {
            return Vec::new();
        };
        let mut bindings = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for binding in garyx_router::bindings_from_value(&thread_data) {
            let selection = BotSelection::new(&binding.channel, &binding.account_id);
            if !seen.insert(selection.bot.clone()) {
                continue;
            }
            bindings.push(ThreadBotBinding {
                selection,
                chat_id: binding.chat_id.clone(),
                delivery_target_type: binding.resolved_delivery_target_type(),
                delivery_target_id: binding.resolved_delivery_target_id(),
                delivery_thread_id: crate::routes::binding_delivery_thread_id(
                    &binding.binding_key,
                    &binding.chat_id,
                ),
            });
        }
        bindings.sort_by(|a, b| a.selection.bot.cmp(&b.selection.bot));
        bindings
    }

    async fn find_thread_binding_for_bot(
        &self,
        thread_id: &str,
        bot: &BotSelection,
    ) -> Option<ThreadBotBinding> {
        self.load_thread_bot_bindings(thread_id)
            .await
            .into_iter()
            .find(|binding| {
                binding.selection.channel == bot.channel
                    && binding.selection.account_id == bot.account_id
            })
    }

    async fn thread_bot_ids(&self, thread_id: &str) -> Vec<String> {
        self.load_thread_bot_bindings(thread_id)
            .await
            .into_iter()
            .map(|binding| binding.selection.bot)
            .collect()
    }

    async fn status_bot_inventory(
        &self,
        cfg: &garyx_models::config::GaryxConfig,
        run_ctx: &RunContext,
    ) -> (Value, Vec<Value>, Vec<Value>, Vec<Value>) {
        let available = self.enabled_bots_from_config(cfg);
        let mut thread_bound = Vec::<BotSelection>::new();
        let mut current_bot: Option<(BotSelection, &'static str)> = None;

        if let Some(thread_id) = run_ctx.thread_id.as_deref() {
            let bindings = self.load_thread_bot_bindings(thread_id).await;
            thread_bound = bindings
                .iter()
                .map(|binding| binding.selection.clone())
                .collect();

            if let (Some(channel), Some(account_id)) =
                (run_ctx.channel.as_deref(), run_ctx.account_id.as_deref())
            {
                current_bot = Some((BotSelection::new(channel, account_id), "run_context"));
            }

            if current_bot.is_none() {
                if let Some(thread_data) = self.app_state.threads.thread_store.get(thread_id).await
                {
                    if let (Some(channel), Some(account_id)) = (
                        thread_data
                            .get("channel")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty()),
                        thread_data
                            .get("account_id")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|value| !value.is_empty()),
                    ) {
                        current_bot =
                            Some((BotSelection::new(channel, account_id), "thread_origin"));
                    } else if thread_bound.len() == 1 {
                        current_bot = Some((thread_bound[0].clone(), "single_thread_binding"));
                    }
                }
            }
        } else if let (Some(channel), Some(account_id)) =
            (run_ctx.channel.as_deref(), run_ctx.account_id.as_deref())
        {
            current_bot = Some((BotSelection::new(channel, account_id), "run_context"));
        }

        let current_bot_value = current_bot.as_ref().map_or(Value::Null, |(bot, source)| {
            json!({
                "bot": bot.bot,
                "channel": bot.channel,
                "account_id": bot.account_id,
                "enabled": available.iter().any(|candidate| candidate == bot),
                "thread_bound": thread_bound.iter().any(|candidate| candidate == bot),
                "source": source,
            })
        });

        let thread_bound_values = thread_bound
            .iter()
            .map(|bot| {
                json!({
                    "bot": bot.bot,
                    "channel": bot.channel,
                    "account_id": bot.account_id,
                    "enabled": available.iter().any(|candidate| candidate == bot),
                    "thread_bound": true,
                })
            })
            .collect::<Vec<_>>();

        let available_values = available
            .iter()
            .map(|bot| {
                json!({
                    "bot": bot.bot,
                    "channel": bot.channel,
                    "account_id": bot.account_id,
                    "enabled": true,
                    "thread_bound": thread_bound.iter().any(|candidate| candidate == bot),
                })
            })
            .collect::<Vec<_>>();

        let other_values = available
            .iter()
            .filter(|candidate| {
                current_bot
                    .as_ref()
                    .map(|(bot, _)| bot != *candidate)
                    .unwrap_or(true)
            })
            .map(|bot| {
                json!({
                    "bot": bot.bot,
                    "channel": bot.channel,
                    "account_id": bot.account_id,
                    "enabled": true,
                    "thread_bound": thread_bound.iter().any(|candidate| candidate == bot),
                })
            })
            .collect::<Vec<_>>();

        (
            current_bot_value,
            thread_bound_values,
            available_values,
            other_values,
        )
    }

    pub(super) async fn send_image_message(
        &self,
        target: &ResolvedMessageTarget,
        image_path: &str,
        caption: Option<&str>,
        reply_to: Option<&str>,
    ) -> Result<String, String> {
        let api_base = if target.channel == "telegram" {
            "https://api.telegram.org"
        } else {
            ""
        };
        self.send_image_message_via_api_base(target, image_path, caption, reply_to, api_base)
            .await
    }

    pub(super) async fn send_file_message(
        &self,
        target: &ResolvedMessageTarget,
        file_path: &str,
        caption: Option<&str>,
        reply_to: Option<&str>,
    ) -> Result<String, String> {
        let api_base = if target.channel == "telegram" {
            "https://api.telegram.org"
        } else {
            ""
        };
        self.send_file_message_via_api_base(target, file_path, caption, reply_to, api_base)
            .await
    }

    pub(super) async fn send_image_message_via_api_base(
        &self,
        target: &ResolvedMessageTarget,
        image_path: &str,
        caption: Option<&str>,
        reply_to: Option<&str>,
        api_base: &str,
    ) -> Result<String, String> {
        let path = Path::new(image_path);
        if !path.is_absolute() {
            return Err("image path must be absolute".to_owned());
        }
        if !path.is_file() {
            return Err(format!("image file not found: {}", path.display()));
        }

        let cfg = self.app_state.config_snapshot();
        if target.channel == "telegram" {
            let account = cfg
                .channels
                .resolved_telegram_config()
                .map_err(|error| format!("invalid telegram plugin config: {error}"))?
                .accounts
                .get(&target.account_id)
                .ok_or_else(|| {
                    format!(
                        "telegram account '{}' not found in configuration",
                        target.account_id
                    )
                })?
                .clone();

            let chat_id = target
                .chat_id
                .parse::<i64>()
                .map_err(|e| format!("invalid telegram chat_id '{}': {e}", target.chat_id))?;

            let reply_to_message_id = match reply_to {
                Some(v) if !v.trim().is_empty() => Some(
                    v.parse::<i64>()
                        .map_err(|e| format!("invalid reply_to '{}': {e}", v))?,
                ),
                _ => None,
            };

            let thread_id = match target.delivery_thread_id.as_deref() {
                Some(v) if !v.trim().is_empty() => {
                    // delivery_thread_id may contain a Garyx internal thread key
                    // (e.g. "thread::uuid") rather than a numeric Telegram topic ID.
                    // Gracefully treat non-numeric values as None.
                    v.parse::<i64>().ok()
                }
                _ => None,
            };

            let http = reqwest::Client::new();
            let message_id = garyx_channels::telegram::send_photo(
                garyx_channels::telegram::TelegramSendTarget::new(
                    &http,
                    &account.token,
                    chat_id,
                    thread_id,
                    api_base,
                ),
                path,
                caption,
                reply_to_message_id,
            )
            .await
            .map_err(|e| format!("image delivery failed: {e}"))?;

            return Ok(message_id.to_string());
        }

        if target.channel == "weixin" {
            let mut account = cfg
                .channels
                .resolved_weixin_config()
                .map_err(|error| format!("invalid weixin plugin config: {error}"))?
                .accounts
                .get(&target.account_id)
                .ok_or_else(|| {
                    format!(
                        "weixin account '{}' not found in configuration",
                        target.account_id
                    )
                })?
                .clone();
            let api_base_override = api_base.trim();
            if !api_base_override.is_empty() {
                account.base_url = api_base_override.to_owned();
            }

            let mut context_token = garyx_channels::weixin::get_context_token_for_thread(
                &target.account_id,
                &target.chat_id,
                target.thread_id.as_deref(),
            )
            .await;
            if context_token
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
            {
                context_token = self.recover_weixin_context_token_from_thread(target).await;
            }
            if context_token
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
            {
                return Err(format!(
                    "weixin context_token missing for account '{}' and user '{}'",
                    target.account_id, target.chat_id
                ));
            }

            let http = reqwest::Client::new();
            let message_id = if api_base_override.is_empty() {
                garyx_channels::weixin::send_image_message_from_path(
                    &http,
                    &account,
                    &target.chat_id,
                    path,
                    caption,
                    context_token.as_deref(),
                )
                .await
                .map_err(|e| format!("image delivery failed: {e}"))?
            } else {
                garyx_channels::weixin::send_image_message_from_path_with_cdn_base(
                    &http,
                    &account,
                    &target.chat_id,
                    path,
                    caption,
                    context_token.as_deref(),
                    api_base_override,
                )
                .await
                .map_err(|e| format!("image delivery failed: {e}"))?
            };

            return Ok(message_id);
        }

        if target.channel == "feishu" {
            let account = cfg
                .channels
                .resolved_feishu_config()
                .map_err(|error| format!("invalid feishu plugin config: {error}"))?
                .accounts
                .get(&target.account_id)
                .ok_or_else(|| {
                    format!(
                        "feishu account '{}' not found in configuration",
                        target.account_id
                    )
                })?
                .clone();
            let api_base = if api_base.trim().is_empty() {
                match account.domain {
                    garyx_models::config::FeishuDomain::Lark => {
                        "https://open.larksuite.com/open-apis".to_owned()
                    }
                    garyx_models::config::FeishuDomain::Feishu => {
                        "https://open.feishu.cn/open-apis".to_owned()
                    }
                }
            } else {
                api_base.to_owned()
            };
            let sender = garyx_channels::FeishuSender::new(
                target.account_id.clone(),
                account.app_id,
                account.app_secret,
                api_base,
                true,
            );
            let message_ids = sender
                .send_image(
                    &target.delivery_target_type,
                    &target.delivery_target_id,
                    path,
                    reply_to,
                )
                .await
                .map_err(|e| format!("image delivery failed: {e}"))?;
            let message_id = message_ids
                .last()
                .cloned()
                .ok_or_else(|| "image delivery returned no message id".to_owned())?;
            return Ok(message_id);
        }

        Err(format!(
            "image sending is currently supported only for telegram/weixin/feishu, got '{}'",
            target.channel
        ))
    }

    pub(super) async fn send_file_message_via_api_base(
        &self,
        target: &ResolvedMessageTarget,
        file_path: &str,
        caption: Option<&str>,
        reply_to: Option<&str>,
        api_base: &str,
    ) -> Result<String, String> {
        let path = Path::new(file_path);
        if !path.is_absolute() {
            return Err("file path must be absolute".to_owned());
        }
        if !path.is_file() {
            return Err(format!("file not found: {}", path.display()));
        }

        let cfg = self.app_state.config_snapshot();
        if target.channel == "telegram" {
            let account = cfg
                .channels
                .resolved_telegram_config()
                .map_err(|error| format!("invalid telegram plugin config: {error}"))?
                .accounts
                .get(&target.account_id)
                .ok_or_else(|| {
                    format!(
                        "telegram account '{}' not found in configuration",
                        target.account_id
                    )
                })?
                .clone();

            let chat_id = target
                .chat_id
                .parse::<i64>()
                .map_err(|e| format!("invalid telegram chat_id '{}': {e}", target.chat_id))?;

            let reply_to_message_id = match reply_to {
                Some(v) if !v.trim().is_empty() => Some(
                    v.parse::<i64>()
                        .map_err(|e| format!("invalid reply_to '{}': {e}", v))?,
                ),
                _ => None,
            };

            let thread_id = match target.delivery_thread_id.as_deref() {
                Some(v) if !v.trim().is_empty() => v.parse::<i64>().ok(),
                _ => None,
            };

            let http = reqwest::Client::new();
            let message_id = garyx_channels::telegram::send_document(
                garyx_channels::telegram::TelegramSendTarget::new(
                    &http,
                    &account.token,
                    chat_id,
                    thread_id,
                    api_base,
                ),
                path,
                caption,
                reply_to_message_id,
            )
            .await
            .map_err(|e| format!("file delivery failed: {e}"))?;

            return Ok(message_id.to_string());
        }

        if target.channel == "weixin" {
            let mut account = cfg
                .channels
                .resolved_weixin_config()
                .map_err(|error| format!("invalid weixin plugin config: {error}"))?
                .accounts
                .get(&target.account_id)
                .ok_or_else(|| {
                    format!(
                        "weixin account '{}' not found in configuration",
                        target.account_id
                    )
                })?
                .clone();
            let api_base_override = api_base.trim();
            if !api_base_override.is_empty() {
                account.base_url = api_base_override.to_owned();
            }

            let mut context_token = garyx_channels::weixin::get_context_token_for_thread(
                &target.account_id,
                &target.chat_id,
                target.thread_id.as_deref(),
            )
            .await;
            if context_token
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
            {
                context_token = self.recover_weixin_context_token_from_thread(target).await;
            }
            if context_token
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
            {
                return Err(format!(
                    "weixin context_token missing for account '{}' and user '{}'",
                    target.account_id, target.chat_id
                ));
            }

            let http = reqwest::Client::new();
            let message_id = if api_base_override.is_empty() {
                garyx_channels::weixin::send_file_message_from_path(
                    &http,
                    &account,
                    &target.chat_id,
                    path,
                    caption,
                    context_token.as_deref(),
                )
                .await
                .map_err(|e| format!("file delivery failed: {e}"))?
            } else {
                garyx_channels::weixin::send_file_message_from_path_with_cdn_base(
                    &http,
                    &account,
                    &target.chat_id,
                    path,
                    caption,
                    context_token.as_deref(),
                    api_base_override,
                )
                .await
                .map_err(|e| format!("file delivery failed: {e}"))?
            };

            return Ok(message_id);
        }

        if target.channel == "feishu" {
            let account = cfg
                .channels
                .resolved_feishu_config()
                .map_err(|error| format!("invalid feishu plugin config: {error}"))?
                .accounts
                .get(&target.account_id)
                .ok_or_else(|| {
                    format!(
                        "feishu account '{}' not found in configuration",
                        target.account_id
                    )
                })?
                .clone();
            let api_base = if api_base.trim().is_empty() {
                match account.domain {
                    garyx_models::config::FeishuDomain::Lark => {
                        "https://open.larksuite.com/open-apis".to_owned()
                    }
                    garyx_models::config::FeishuDomain::Feishu => {
                        "https://open.feishu.cn/open-apis".to_owned()
                    }
                }
            } else {
                api_base.to_owned()
            };
            let sender = garyx_channels::FeishuSender::new(
                target.account_id.clone(),
                account.app_id,
                account.app_secret,
                api_base,
                true,
            );
            let message_ids = sender
                .send_file(
                    &target.delivery_target_type,
                    &target.delivery_target_id,
                    path,
                    reply_to,
                )
                .await
                .map_err(|e| format!("file delivery failed: {e}"))?;
            let message_id = message_ids
                .last()
                .cloned()
                .ok_or_else(|| "file delivery returned no message id".to_owned())?;
            return Ok(message_id);
        }

        Err(format!(
            "file sending is currently supported only for telegram/weixin/feishu, got '{}'",
            target.channel
        ))
    }

    async fn recover_weixin_context_token_from_thread(
        &self,
        target: &ResolvedMessageTarget,
    ) -> Option<String> {
        let thread_id = target.thread_id.as_deref()?;
        let thread_data = self.app_state.threads.thread_store.get(thread_id).await?;
        let token = Self::extract_weixin_context_token_from_thread_data(&thread_data)?;
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return None;
        }
        let token = trimmed.to_owned();
        garyx_channels::weixin::set_context_token_for_thread(
            &target.account_id,
            &target.chat_id,
            target.thread_id.as_deref(),
            &token,
        )
        .await;
        Some(token)
    }

    pub(super) fn extract_weixin_context_token_from_thread_data(
        thread_data: &Value,
    ) -> Option<String> {
        let messages = thread_data.get("messages")?.as_array()?;
        for message in messages.iter().rev() {
            if let Some(token) = message
                .get("context_token")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Some(token.to_owned());
            }
            if let Some(token) = message
                .get("metadata")
                .and_then(Value::as_object)
                .and_then(|metadata| metadata.get("context_token"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Some(token.to_owned());
            }
        }
        None
    }

    pub(super) fn is_scheduled_thread_key(thread_id: Option<&str>) -> bool {
        thread_id
            .map(garyx_router::MessageRouter::is_scheduled_thread)
            .unwrap_or(false)
    }

    pub(super) fn format_scheduled_message(text: &str, thread_id: Option<&str>) -> String {
        let Some(thread_id) = thread_id else {
            return text.to_owned();
        };
        if text.trim().is_empty() {
            return text.to_owned();
        }
        let header = format!("#{thread_id}");
        if text.trim_start().starts_with(&header) {
            text.to_owned()
        } else {
            format!("{header}\n{text}")
        }
    }

    pub(super) async fn run_image_gen(
        prompt: &str,
        aspect_ratio: &str,
        image_size: &str,
        reference_images: &[String],
        configured_api_key: &str,
        configured_model: &str,
    ) -> Result<Value, String> {
        let prompt_trimmed = prompt.trim();
        if prompt_trimmed.is_empty() {
            return Err("prompt is required".to_owned());
        }
        let api_key = Self::resolve_image_gen_api_key(configured_api_key).ok_or_else(|| {
            "image generation backend not configured: set gateway.image_gen.api_key in garyx.json"
                .to_owned()
        })?;
        let model = if configured_model.is_empty() {
            "gemini-3.1-flash-image-preview".to_owned()
        } else {
            configured_model.to_owned()
        };

        let mut content_parts = vec![json!({ "text": prompt_trimmed })];
        for image_path in reference_images {
            let path = Path::new(image_path);
            if !path.is_file() {
                tracing::warn!(path = %path.display(), "skipping missing reference image");
                continue;
            }

            let raw = tokio::fs::read(path)
                .await
                .map_err(|e| format!("failed to read reference image '{}': {e}", path.display()))?;
            let mime = Self::mime_type_for_path(path);
            let encoded = base64::engine::general_purpose::STANDARD.encode(raw);
            content_parts.push(json!({
                "inlineData": {
                    "mimeType": mime,
                    "data": encoded,
                }
            }));
        }

        let endpoint = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key={api_key}"
        );
        let request_body = json!({
            "contents": [{
                "role": "user",
                "parts": content_parts,
            }],
            "generationConfig": {
                "responseModalities": ["IMAGE"],
                "imageConfig": {
                    "aspectRatio": aspect_ratio,
                    "imageSize": image_size,
                }
            }
        });

        let response = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| format!("failed to build http client: {e}"))?
            .post(endpoint)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("image generation request failed: {e}"))?;
        let status = response.status();
        let response_json: Value = response
            .json()
            .await
            .map_err(|e| format!("failed to parse image generation response: {e}"))?;

        if !status.is_success() {
            let detail = response_json
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            return Err(format!("image generation api error ({status}): {detail}"));
        }

        let Some((bytes, mime_type)) = Self::extract_generated_image(&response_json) else {
            let detail = response_json
                .pointer("/candidates/0/finishReason")
                .and_then(Value::as_str)
                .unwrap_or("no inline image data in response");
            return Err(format!("image generation returned no image: {detail}"));
        };

        let extension = Self::extension_for_mime_type(&mime_type);
        let output_name = format!(
            "gemini_image_{}_{}.{}",
            chrono::Utc::now().format("%Y%m%d_%H%M%S"),
            Uuid::new_v4(),
            extension
        );
        let output_path = std::env::temp_dir().join(output_name);

        tokio::fs::write(&output_path, bytes)
            .await
            .map_err(|e| format!("failed to write generated image: {e}"))?;

        Ok(json!({
            "success": true,
            "image_path": output_path.to_string_lossy(),
            "mime_type": mime_type,
            "model": model,
        }))
    }

    pub(super) async fn run_search(
        query: &str,
        configured_api_key: &str,
        configured_model: &str,
    ) -> Result<Value, String> {
        let api_key = Self::resolve_search_api_key(configured_api_key).ok_or_else(|| {
            "search backend not configured: set gateway.search.api_key in garyx.json".to_owned()
        })?;
        let model = if configured_model.is_empty() {
            "gemini-3-flash-preview".to_owned()
        } else {
            configured_model.to_owned()
        };

        let endpoint = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key={api_key}"
        );
        let request_body = json!({
            "contents": [{
                "role": "user",
                "parts": [{ "text": query }]
            }],
            "tools": [{
                "google_search": {}
            }]
        });

        let response = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| format!("failed to build http client: {e}"))?
            .post(endpoint)
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("search request failed: {e}"))?;
        let status = response.status();
        let response_json: Value = response
            .json()
            .await
            .map_err(|e| format!("failed to parse search response: {e}"))?;

        if !status.is_success() {
            let detail = response_json
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            return Err(format!("search api error ({status}): {detail}"));
        }

        let answer = response_json
            .pointer("/candidates/0/content/parts/0/text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();

        let sources = Self::extract_grounding_sources(&response_json);

        Ok(json!({
            "success": true,
            "answer": answer,
            "sources": sources,
            "model": model,
        }))
    }

    pub(super) fn resolve_search_api_key(configured_api_key: &str) -> Option<String> {
        if !configured_api_key.trim().is_empty() {
            return Some(configured_api_key.trim().to_owned());
        }
        std::env::var("GEMINI_API_KEY")
            .ok()
            .or_else(|| std::env::var("GOOGLE_API_KEY").ok())
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    }

    pub(super) fn extract_grounding_sources(response: &Value) -> Vec<Value> {
        let Some(chunks) = response
            .pointer("/candidates/0/groundingMetadata/groundingChunks")
            .and_then(Value::as_array)
        else {
            return Vec::new();
        };

        chunks
            .iter()
            .filter_map(|chunk| {
                let web = chunk.get("web")?;
                let uri = web.get("uri").and_then(Value::as_str).unwrap_or("");
                let title = web.get("title").and_then(Value::as_str).unwrap_or("");
                Some(json!({
                    "url": uri,
                    "title": title,
                }))
            })
            .collect()
    }

    pub(super) fn resolve_image_gen_api_key(configured_api_key: &str) -> Option<String> {
        if !configured_api_key.trim().is_empty() {
            return Some(configured_api_key.trim().to_owned());
        }
        std::env::var("GEMINI_API_KEY")
            .ok()
            .or_else(|| std::env::var("GOOGLE_API_KEY").ok())
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    }

    pub(super) fn extract_generated_image(response: &Value) -> Option<(Vec<u8>, String)> {
        let candidates = response.get("candidates")?.as_array()?;
        for candidate in candidates {
            let parts = candidate.get("content")?.get("parts")?.as_array()?;
            if let Some(part) = parts.iter().next() {
                let inline = part.get("inlineData").or_else(|| part.get("inline_data"))?;
                let data = inline.get("data")?.as_str()?;
                let mime = inline
                    .get("mimeType")
                    .or_else(|| inline.get("mime_type"))
                    .and_then(Value::as_str)
                    .unwrap_or("image/png")
                    .to_owned();
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .ok()?;
                return Some((decoded, mime));
            }
        }
        None
    }

    pub(super) fn extension_for_mime_type(mime_type: &str) -> &'static str {
        match mime_type.to_ascii_lowercase().as_str() {
            "image/jpeg" | "image/jpg" => "jpg",
            "image/webp" => "webp",
            "image/gif" => "gif",
            _ => "png",
        }
    }

    pub(super) fn mime_type_for_path(path: &Path) -> &'static str {
        let ext = path
            .extension()
            .and_then(|v| v.to_str())
            .map(|v| v.to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "gif" => "image/gif",
            "png" => "image/png",
            _ => "application/octet-stream",
        }
    }

    pub(super) fn require_auth(
        state: &AppState,
        ctx: &RunContext,
        param_token: Option<&str>,
    ) -> Result<(), String> {
        if state.ops.restart_tokens.is_empty() {
            return Ok(());
        }
        let token = param_token
            .map(|s| s.to_owned())
            .or_else(|| ctx.auth_token.clone())
            .unwrap_or_default();
        if state.ops.restart_tokens.iter().any(|t| t == &token) {
            Ok(())
        } else {
            Err("authorization token required".to_owned())
        }
    }

    pub(super) fn parse_schedule(params: &CronParams) -> Result<CronSchedule, String> {
        if let Some(v) = &params.schedule {
            if let Ok(schedule) = serde_json::from_value::<CronSchedule>(v.clone()) {
                return Ok(schedule);
            }
            if let Some(s) = v.as_str() {
                if let Some(rest) = s.strip_prefix("interval:") {
                    let interval_secs = rest
                        .parse::<u64>()
                        .map_err(|_| "invalid schedule interval".to_owned())?;
                    return Ok(CronSchedule::Interval { interval_secs });
                }
                if let Some(rest) = s.strip_prefix("cron:") {
                    return Ok(CronSchedule::Cron {
                        expr: rest.trim().to_owned(),
                        timezone: None,
                    });
                }
                let timestamp = crate::cron::parse_once_timestamp(s).ok_or_else(|| {
                    "invalid one-time schedule (expected RFC3339, YYYY-MM-DDTHH:MM, or ONCE:YYYY-MM-DD HH:MM)"
                        .to_owned()
                })?;
                return Ok(CronSchedule::Once {
                    at: timestamp.to_rfc3339(),
                });
            }
        }
        if let Some(interval) = params.interval_secs {
            return Ok(CronSchedule::Interval {
                interval_secs: interval,
            });
        }
        if let Some(at) = &params.at {
            let timestamp = crate::cron::parse_once_timestamp(at).ok_or_else(|| {
                "invalid one-time schedule (expected RFC3339, YYYY-MM-DDTHH:MM, or ONCE:YYYY-MM-DD HH:MM)"
                    .to_owned()
            })?;
            return Ok(CronSchedule::Once {
                at: timestamp.to_rfc3339(),
            });
        }
        Err("missing schedule (expected `schedule`, `interval_secs`, or `at`)".to_owned())
    }

    pub(super) fn parse_cron_action(params: &CronParams) -> Result<CronAction, String> {
        let raw = params
            .job_action
            .as_deref()
            .or(params.cron_action.as_deref())
            .unwrap_or("log");
        let normalized = raw.trim().to_ascii_lowercase();

        match normalized.as_str() {
            "log" => Ok(CronAction::Log),
            "heartbeat" => Ok(CronAction::Heartbeat),
            "system_event" | "systemevent" => Ok(CronAction::SystemEvent),
            "agent_turn" | "agentturn" => Ok(CronAction::AgentTurn),
            _ => Err(format!("invalid job_action: {raw}")),
        }
    }
}
