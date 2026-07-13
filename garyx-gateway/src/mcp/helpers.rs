use super::*;
use garyx_router::ThreadStoreExt;

#[derive(Debug, Clone, PartialEq, Eq)]
struct BotSelection {
    bot: String,
    channel: String,
    account_id: String,
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
    pub(super) async fn status_payload(&self, run_ctx: RunContext) -> Result<Value, String> {
        let state = &self.app_state;
        let cfg = state.config_snapshot();
        let uptime_secs = state.runtime.start_time.elapsed().as_secs();
        let current_workspace_dir = if let Some(thread_id) = run_ctx.thread_id.as_deref() {
            state
                .threads
                .thread_store
                .get_logged(thread_id)
                .await
                .and_then(|value| garyx_router::workspace_dir_from_value(&value))
        } else {
            None
        };

        let thread_count = state
            .threads
            .thread_store
            .count_keys(None)
            .await
            .map_err(|error| format!("thread store count failed: {error}"))?;

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

    async fn load_thread_bot_bindings(&self, thread_id: &str) -> Vec<BotSelection> {
        let Some(thread_data) = self
            .app_state
            .threads
            .thread_store
            .get_logged(thread_id)
            .await
        else {
            return Vec::new();
        };
        let mut bindings = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for binding in garyx_router::bindings_from_value(&thread_data) {
            let selection = BotSelection::new(&binding.channel, &binding.account_id);
            if seen.insert(selection.bot.clone()) {
                bindings.push(selection);
            }
        }
        bindings.sort_by(|a, b| a.bot.cmp(&b.bot));
        bindings
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
            thread_bound = self.load_thread_bot_bindings(thread_id).await;

            if let (Some(channel), Some(account_id)) =
                (run_ctx.channel.as_deref(), run_ctx.account_id.as_deref())
            {
                current_bot = Some((BotSelection::new(channel, account_id), "run_context"));
            }

            if current_bot.is_none()
                && let Some(thread_data) = self
                    .app_state
                    .threads
                    .thread_store
                    .get_logged(thread_id)
                    .await
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
                    current_bot = Some((BotSelection::new(channel, account_id), "thread_origin"));
                } else if thread_bound.len() == 1 {
                    current_bot = Some((thread_bound[0].clone(), "single_thread_binding"));
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
}
