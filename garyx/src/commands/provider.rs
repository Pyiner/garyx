use super::*;

pub(crate) async fn cmd_config_claude_cli(
    config_path: &str,
    mode: Option<String>,
    path: Option<String>,
    clear_path: bool,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    let config_path = loaded.path;
    let mut value = serde_json::to_value(loaded.config)?;
    let root = value
        .as_object_mut()
        .ok_or("config root is not an object")?;
    let agents = root
        .entry("agents".to_owned())
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("config.agents is not an object")?;
    let claude = agents
        .entry("claude".to_owned())
        .or_insert_with(|| json!({ "provider_type": "claude_code" }));
    if !claude.is_object() {
        *claude = json!({ "provider_type": "claude_code" });
    }
    let claude = claude.as_object_mut().expect("claude config object");
    claude.insert("provider_type".to_owned(), json!("claude_code"));

    if let Some(mode) = mode
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        claude.insert("claude_cli_mode".to_owned(), json!(mode));
    }
    if clear_path {
        claude.remove("claude_cli_path");
    } else if let Some(path) = path.as_deref().map(str::trim) {
        if path.is_empty() {
            claude.remove("claude_cli_path");
        } else {
            claude.insert("claude_cli_path".to_owned(), json!(path));
        }
    }

    let validated: GaryxConfig = serde_json::from_value(value.clone())?;
    save_config_struct(&config_path, &validated)?;
    notify_gateway_reload_quiet(&config_path).await;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(
                value
                    .get("agents")
                    .and_then(|agents| agents.get("claude"))
                    .unwrap_or(&Value::Null)
            )?
        );
    } else {
        let configured = value
            .get("agents")
            .and_then(|agents| agents.get("claude"))
            .and_then(Value::as_object);
        let mode = configured
            .and_then(|object| object.get("claude_cli_mode"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(default_claude_cli_mode);
        let path = configured
            .and_then(|object| object.get("claude_cli_path"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if path.is_empty() {
            println!("Claude Agent SDK CLI: mode={mode}, path=<auto>");
        } else {
            println!("Claude Agent SDK CLI: mode={mode}, path={path}");
        }
    }
    Ok(())
}

fn provider_model_config_key(provider_type: &ProviderType) -> Result<&'static str, String> {
    match provider_type {
        ProviderType::ClaudeCode => Ok("claude"),
        ProviderType::CodexAppServer => Ok("codex"),
        ProviderType::Traex => Ok("traex"),
        ProviderType::AntigravityCli => Ok("antigravity"),
        ProviderType::Gpt => Ok("gpt"),
        ProviderType::ClaudeLlm => Ok("anthropic"),
        ProviderType::GeminiLlm => Ok("google"),
    }
}

#[derive(Debug, Clone)]
struct ProviderDescriptor {
    label: &'static str,
    provider_type: ProviderType,
    key: &'static str,
}

fn provider_descriptors() -> Vec<ProviderDescriptor> {
    vec![
        ProviderDescriptor {
            label: "Claude Code",
            provider_type: ProviderType::ClaudeCode,
            key: "claude",
        },
        ProviderDescriptor {
            label: "Codex",
            provider_type: ProviderType::CodexAppServer,
            key: "codex",
        },
        ProviderDescriptor {
            label: "Traex",
            provider_type: ProviderType::Traex,
            key: "traex",
        },
        ProviderDescriptor {
            label: "Antigravity",
            provider_type: ProviderType::AntigravityCli,
            key: "antigravity",
        },
        ProviderDescriptor {
            label: "GPT",
            provider_type: ProviderType::Gpt,
            key: "gpt",
        },
        ProviderDescriptor {
            label: "Anthropic",
            provider_type: ProviderType::ClaudeLlm,
            key: "anthropic",
        },
        ProviderDescriptor {
            label: "Google",
            provider_type: ProviderType::GeminiLlm,
            key: "google",
        },
    ]
}

fn provider_descriptor_for_slug(provider: &str) -> Result<ProviderDescriptor, String> {
    let provider_type = ProviderType::from_slug(provider)
        .ok_or_else(|| format!("unsupported provider type: {provider}"))?;
    provider_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.provider_type == provider_type)
        .ok_or_else(|| format!("unsupported provider type: {provider}"))
}

fn provider_default_model_fallback(provider_type: &ProviderType) -> Option<String> {
    match provider_type {
        ProviderType::AntigravityCli => Some(garyx_models::provider::default_antigravity_model()),
        ProviderType::Gpt => Some("gpt-5.5".to_owned()),
        ProviderType::ClaudeLlm => Some("claude-sonnet-4-6".to_owned()),
        ProviderType::GeminiLlm => Some("gemini-3-flash-preview".to_owned()),
        _ => None,
    }
}

fn default_provider_config_value(descriptor: &ProviderDescriptor) -> Value {
    let mut config = AgentProviderConfig {
        provider_id: descriptor.key.to_owned(),
        provider_type: descriptor.provider_type.as_slug().to_owned(),
        ..AgentProviderConfig::default()
    };
    if let Some(default_model) = provider_default_model_fallback(&descriptor.provider_type) {
        config.default_model = default_model;
    }
    serde_json::to_value(config).unwrap_or_else(|_| {
        json!({
            "provider_id": descriptor.key,
            "provider_type": descriptor.provider_type.as_slug(),
        })
    })
}

fn provider_config_from_settings(settings: &Value, descriptor: &ProviderDescriptor) -> Value {
    let configured = settings
        .get("agents")
        .and_then(|agents| agents.get(descriptor.key))
        .cloned();
    let Some(value) = configured else {
        return default_provider_config_value(descriptor);
    };
    let Ok(mut config) = serde_json::from_value::<AgentProviderConfig>(value.clone()) else {
        return value;
    };
    config.provider_id = if config.provider_id.trim().is_empty() {
        descriptor.key.to_owned()
    } else {
        config.provider_id.trim().to_owned()
    };
    config.provider_type = descriptor.provider_type.as_slug().to_owned();
    serde_json::to_value(config).unwrap_or(value)
}

fn provider_config_string<'a>(config: &'a Value, key: &str) -> &'a str {
    config
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
}

fn provider_config_env(config: &Value) -> Map<String, Value> {
    config
        .get("env")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn provider_config_default_model_label(descriptor: &ProviderDescriptor, config: &Value) -> String {
    for key in ["default_model", "model"] {
        let value = provider_config_string(config, key);
        if !value.is_empty() {
            return value.to_owned();
        }
    }
    provider_default_model_fallback(&descriptor.provider_type)
        .unwrap_or_else(|| "(provider default)".to_owned())
}

fn provider_config_auth_label(
    descriptor: &ProviderDescriptor,
    config: &Value,
    usage: Option<&Value>,
) -> String {
    let env = provider_config_env(config);
    let has_api_key = api_key_env_name(Some(descriptor.provider_type.clone()))
        .and_then(|key| env.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if has_api_key {
        return "api key".to_owned();
    }

    match descriptor.provider_type {
        ProviderType::ClaudeCode | ProviderType::CodexAppServer | ProviderType::AntigravityCli => {
            if usage
                .and_then(|value| value.get("available"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                "signed in".to_owned()
            } else {
                "not signed in".to_owned()
            }
        }
        ProviderType::Traex => "local CLI".to_owned(),
        ProviderType::Gpt | ProviderType::ClaudeLlm | ProviderType::GeminiLlm => {
            let auth_source = provider_config_string(config, "auth_source");
            if auth_source.is_empty() {
                "not set".to_owned()
            } else {
                auth_source.to_owned()
            }
        }
    }
}

fn usage_provider_id_for_type(provider_type: &ProviderType) -> Option<&'static str> {
    match provider_type {
        ProviderType::ClaudeCode => Some("claude_code"),
        ProviderType::CodexAppServer => Some("codex"),
        ProviderType::AntigravityCli => Some("antigravity"),
        _ => None,
    }
}

fn usage_provider<'a>(usage: &'a Value, provider_type: &ProviderType) -> Option<&'a Value> {
    let id = usage_provider_id_for_type(provider_type)?;
    usage
        .get("providers")
        .and_then(Value::as_array)
        .and_then(|providers| {
            providers
                .iter()
                .find(|provider| provider.get("id").and_then(Value::as_str) == Some(id))
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UsageSeverity {
    Healthy,
    Warning,
    Critical,
    Unavailable,
}

fn usage_severity(available: bool, remaining_percent: f64) -> UsageSeverity {
    if !available {
        return UsageSeverity::Unavailable;
    }
    if remaining_percent < 20.0 {
        UsageSeverity::Critical
    } else if remaining_percent < 50.0 {
        UsageSeverity::Warning
    } else {
        UsageSeverity::Healthy
    }
}

fn percent_label(value: f64) -> String {
    format!("{:.0}%", value.clamp(0.0, 100.0))
}

fn format_reset_countdown(
    reset_after_seconds: Option<i64>,
    resets_at: Option<&str>,
    now: DateTime<FixedOffset>,
) -> Option<String> {
    let seconds = reset_after_seconds.or_else(|| {
        let resets_at = resets_at?;
        let parsed = DateTime::parse_from_rfc3339(resets_at).ok()?;
        Some(parsed.signed_duration_since(now).num_seconds())
    })?;
    Some(format_reset_seconds(seconds))
}

fn format_reset_seconds(seconds: i64) -> String {
    let seconds = seconds.max(0);
    if seconds < 60 {
        return "resets in <1m".to_owned();
    }
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    if days > 0 {
        let rem_hours = hours % 24;
        if rem_hours > 0 {
            format!("resets in {days}d {rem_hours}h")
        } else {
            format!("resets in {days}d")
        }
    } else if hours > 0 {
        let rem_minutes = minutes % 60;
        if rem_minutes > 0 {
            format!("resets in {hours}h {rem_minutes}m")
        } else {
            format!("resets in {hours}h")
        }
    } else {
        format!("resets in {minutes}m")
    }
}

fn format_usage_window_cell(window: Option<&Value>, now: DateTime<FixedOffset>) -> String {
    let Some(window) = window else {
        return "-".to_owned();
    };
    let remaining = window
        .get("remaining_percent")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let reset = format_reset_countdown(
        window.get("reset_after_seconds").and_then(Value::as_i64),
        window.get("resets_at").and_then(Value::as_str),
        now,
    )
    .unwrap_or_else(|| "reset unknown".to_owned());
    format!("{} | {reset}", percent_label(remaining))
}

fn refreshed_age_label(refreshed_at: Option<&str>, now: DateTime<FixedOffset>) -> Option<String> {
    let refreshed_at = refreshed_at?;
    let parsed = DateTime::parse_from_rfc3339(refreshed_at).ok()?;
    let seconds = now.signed_duration_since(parsed).num_seconds().max(0);
    if seconds < 60 {
        return Some("updated <1m ago".to_owned());
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return Some(format!("updated {minutes}m ago"));
    }
    let hours = minutes / 60;
    if hours < 24 {
        return Some(format!("updated {hours}h ago"));
    }
    let days = hours / 24;
    Some(format!("updated {days}d ago"))
}

fn provider_min_remaining_percent(provider: &Value) -> Option<f64> {
    let mut values = Vec::new();
    for key in ["session", "weekly"] {
        if let Some(value) = provider
            .get(key)
            .and_then(|window| window.get("remaining_percent"))
            .and_then(Value::as_f64)
        {
            values.push(value);
        }
    }
    if let Some(models) = provider.get("models").and_then(Value::as_array) {
        for model in models {
            if let Some(value) = model.get("remaining_percent").and_then(Value::as_f64) {
                values.push(value);
            }
        }
    }
    values
        .into_iter()
        .min_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal))
}

fn usage_status_label(
    provider: &Value,
    refreshed_at: Option<&str>,
    now: DateTime<FixedOffset>,
) -> String {
    if !provider
        .get("available")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let error = provider
            .get("error")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("No quota data");
        return format!("unavailable: {error}");
    }
    if provider
        .get("stale")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return refreshed_age_label(refreshed_at, now)
            .map(|age| format!("stale ({age})"))
            .unwrap_or_else(|| "stale".to_owned());
    }
    match usage_severity(
        true,
        provider_min_remaining_percent(provider).unwrap_or(100.0),
    ) {
        UsageSeverity::Critical => "critical".to_owned(),
        UsageSeverity::Warning => "warning".to_owned(),
        UsageSeverity::Healthy => "ok".to_owned(),
        UsageSeverity::Unavailable => "unavailable".to_owned(),
    }
}

fn provider_usage_summary(
    descriptor: &ProviderDescriptor,
    usage: Option<&Value>,
    now: DateTime<FixedOffset>,
) -> String {
    let Some(usage) = usage else {
        return "No quota data".to_owned();
    };
    if !usage
        .get("available")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return "Unavailable".to_owned();
    }
    if let Some(weekly) = usage.get("weekly") {
        let remaining = weekly
            .get("remaining_percent")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let suffix = if usage.get("stale").and_then(Value::as_bool).unwrap_or(false) {
            " stale"
        } else {
            ""
        };
        return format!("{} wk{suffix}", percent_label(remaining));
    }
    if descriptor.provider_type == ProviderType::AntigravityCli {
        let mut models = usage
            .get("models")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        models.sort_by(|left, right| {
            let left_pct = left
                .get("remaining_percent")
                .and_then(Value::as_f64)
                .unwrap_or(101.0);
            let right_pct = right
                .get("remaining_percent")
                .and_then(Value::as_f64)
                .unwrap_or(101.0);
            left_pct
                .partial_cmp(&right_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if let Some(model) = models.first() {
            let remaining = model
                .get("remaining_percent")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            return format!("{} tightest", percent_label(remaining));
        }
    }
    let _ = now;
    "No quota data".to_owned()
}

fn provider_status_summary(usage: Option<&Value>) -> String {
    let Some(usage) = usage else {
        return "unknown".to_owned();
    };
    if !usage
        .get("available")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return "error".to_owned();
    }
    if usage.get("stale").and_then(Value::as_bool).unwrap_or(false) {
        return "stale".to_owned();
    }
    "ready".to_owned()
}

fn provider_list_rows(settings: &Value, usage: Option<&Value>) -> Vec<Value> {
    let now = Local::now().fixed_offset();
    provider_descriptors()
        .iter()
        .map(|descriptor| {
            let config = provider_config_from_settings(settings, descriptor);
            let usage =
                usage.and_then(|payload| usage_provider(payload, &descriptor.provider_type));
            json!({
                "provider": descriptor.label,
                "type": descriptor.provider_type.as_slug(),
                "key": descriptor.key,
                "auth": provider_config_auth_label(descriptor, &config, usage),
                "default_model": provider_config_default_model_label(descriptor, &config),
                "usage": provider_usage_summary(descriptor, usage, now),
                "status": provider_status_summary(usage),
            })
        })
        .collect()
}

fn format_provider_list_table(settings: &Value, usage: Option<&Value>) -> String {
    let rows = provider_list_rows(settings, usage)
        .iter()
        .map(|row| {
            [
                "provider",
                "type",
                "key",
                "auth",
                "default_model",
                "usage",
                "status",
            ]
            .iter()
            .map(|field| row[*field].as_str().unwrap_or("-").to_owned())
            .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    render_text_table(
        &[
            "PROVIDER",
            "TYPE",
            "KEY",
            "AUTH",
            "DEFAULT MODEL",
            "USAGE",
            "STATUS",
        ],
        &rows,
    )
}

fn format_provider_show_table(descriptor: &ProviderDescriptor, config: &Value) -> String {
    let env = provider_config_env(config);
    let env_keys = if env.is_empty() {
        "(none)".to_owned()
    } else {
        let mut keys = env.keys().cloned().collect::<Vec<_>>();
        keys.sort();
        keys.join(", ")
    };
    let mut output = String::new();
    writeln!(output, "Provider: {}", descriptor.label).expect("write string");
    writeln!(output).expect("write string");
    writeln!(output, "Identity").expect("write string");
    writeln!(output, "  Type: {}", descriptor.provider_type.as_slug()).expect("write string");
    writeln!(output, "  Key: {}", descriptor.key).expect("write string");
    writeln!(
        output,
        "  Provider ID: {}",
        provider_config_string(config, "provider_id")
    )
    .expect("write string");
    writeln!(output).expect("write string");
    writeln!(output, "Auth").expect("write string");
    writeln!(
        output,
        "  Auth source: {}",
        display_or_default(provider_config_string(config, "auth_source"), "(default)")
    )
    .expect("write string");
    writeln!(
        output,
        "  Base URL: {}",
        display_or_default(provider_config_string(config, "base_url"), "(default)")
    )
    .expect("write string");
    writeln!(output, "  Env keys: {env_keys}").expect("write string");
    writeln!(output).expect("write string");
    writeln!(output, "Defaults").expect("write string");
    writeln!(
        output,
        "  Model: {}",
        display_or_default(
            provider_config_string(config, "default_model"),
            "(provider default)"
        )
    )
    .expect("write string");
    writeln!(
        output,
        "  Reasoning: {}",
        display_or_default(
            provider_config_string(config, "model_reasoning_effort"),
            "(provider default)"
        )
    )
    .expect("write string");
    writeln!(
        output,
        "  Service tier: {}",
        display_or_default(
            provider_config_string(config, "model_service_tier"),
            "(provider default)"
        )
    )
    .expect("write string");
    writeln!(output).expect("write string");
    writeln!(output, "Advanced").expect("write string");
    writeln!(
        output,
        "  Claude CLI mode: {}",
        display_or_default(
            provider_config_string(config, "claude_cli_mode"),
            "(default)"
        )
    )
    .expect("write string");
    writeln!(
        output,
        "  Claude CLI path: {}",
        display_or_default(provider_config_string(config, "claude_cli_path"), "(auto)")
    )
    .expect("write string");
    writeln!(
        output,
        "  Workspace dir: {}",
        config
            .get("workspace_dir")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("(default)")
    )
    .expect("write string");
    output
}

fn display_or_default<'a>(value: &'a str, default: &'a str) -> &'a str {
    if value.is_empty() { default } else { value }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProviderSetOptions {
    pub provider: String,
    pub model: Option<String>,
    pub clear_model: bool,
    pub reasoning: Option<String>,
    pub clear_reasoning: bool,
    pub service_tier: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub auth_source: Option<String>,
    pub claude_cli_mode: Option<String>,
    pub claude_cli_path: Option<String>,
    pub env: Vec<String>,
    pub clear_env: Vec<String>,
    pub json_output: bool,
}

#[derive(Debug, Clone)]
struct ProviderSetPatch {
    provider_type: ProviderType,
    provider_key: &'static str,
    provider_config: Map<String, Value>,
    patch: Value,
}

fn build_provider_set_patch(
    options: &ProviderSetOptions,
) -> Result<ProviderSetPatch, Box<dyn std::error::Error>> {
    let provider_type = ProviderType::from_slug(&options.provider)
        .ok_or_else(|| format!("unsupported provider type: {}", options.provider))?;
    let provider_key = provider_model_config_key(&provider_type)?;
    let has_claude_cli_update =
        options.claude_cli_mode.is_some() || options.claude_cli_path.is_some();
    if has_claude_cli_update && provider_type != ProviderType::ClaudeCode {
        return Err("--claude-cli-* options are only supported for provider claude_code".into());
    }

    let touched = options.model.is_some()
        || options.clear_model
        || options.reasoning.is_some()
        || options.clear_reasoning
        || options.service_tier.is_some()
        || options.base_url.is_some()
        || options.api_key.is_some()
        || options.auth_source.is_some()
        || has_claude_cli_update
        || !options.env.is_empty()
        || !options.clear_env.is_empty();
    if !touched {
        return Err(
            "set --model, --clear-model, --reasoning, --clear-reasoning, --service-tier, --base-url, --api-key, --auth-source, --claude-cli-mode, --claude-cli-path, --env, or --clear-env"
                .into(),
        );
    }

    let mut provider_config = Map::new();
    provider_config.insert(
        "provider_type".to_owned(),
        Value::String(provider_type.as_slug().to_owned()),
    );
    if options.clear_model {
        provider_config.insert("default_model".to_owned(), Value::String(String::new()));
    } else if let Some(model) = options.model.as_deref().map(str::trim) {
        provider_config.insert("default_model".to_owned(), Value::String(model.to_owned()));
    }
    if options.clear_reasoning {
        provider_config.insert(
            "model_reasoning_effort".to_owned(),
            Value::String(String::new()),
        );
    } else if let Some(reasoning) = options.reasoning.as_deref().map(str::trim) {
        provider_config.insert(
            "model_reasoning_effort".to_owned(),
            Value::String(reasoning.to_owned()),
        );
    }
    if let Some(service_tier) = options.service_tier.as_deref().map(str::trim) {
        provider_config.insert(
            "model_service_tier".to_owned(),
            Value::String(service_tier.to_owned()),
        );
    }
    if let Some(base_url) = options.base_url.as_deref().map(str::trim) {
        provider_config.insert("base_url".to_owned(), Value::String(base_url.to_owned()));
    }
    if let Some(auth_source) = options.auth_source.as_deref().map(str::trim) {
        provider_config.insert(
            "auth_source".to_owned(),
            Value::String(auth_source.to_owned()),
        );
    }
    if let Some(mode) = options.claude_cli_mode.as_deref().map(str::trim) {
        provider_config.insert("claude_cli_mode".to_owned(), Value::String(mode.to_owned()));
    }
    if let Some(path) = options.claude_cli_path.as_deref().map(str::trim) {
        provider_config.insert("claude_cli_path".to_owned(), Value::String(path.to_owned()));
    }

    let mut env = Map::new();
    for key in &options.clear_env {
        let key = key.trim();
        if !garyx_models::custom_agent::is_valid_env_key(key) {
            return Err(
                format!("invalid env key '{key}': must match [A-Za-z_][A-Za-z0-9_]*").into(),
            );
        }
        env.insert(key.to_owned(), Value::String(String::new()));
    }
    for pair in &options.env {
        let (key, value) = parse_env_pair(pair)?;
        env.insert(key, Value::String(value));
    }
    if let Some(api_key) = options.api_key.as_deref() {
        let env_name = api_key_env_name(Some(provider_type.clone()))
            .ok_or("--api-key is only supported for gpt, anthropic, or google providers")?;
        env.insert(env_name.to_owned(), Value::String(api_key.to_owned()));
        if provider_type == ProviderType::Gpt && !provider_config.contains_key("auth_source") {
            provider_config.insert(
                "auth_source".to_owned(),
                Value::String("api_key".to_owned()),
            );
        }
    }
    if !env.is_empty() {
        provider_config.insert("env".to_owned(), Value::Object(env));
    }

    let mut agents_patch = Map::new();
    agents_patch.insert(
        provider_key.to_owned(),
        Value::Object(provider_config.clone()),
    );
    let patch = json!({
        "agents": Value::Object(agents_patch)
    });

    Ok(ProviderSetPatch {
        provider_type,
        provider_key,
        provider_config,
        patch,
    })
}

pub(crate) async fn cmd_provider_list(
    config_path: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway = gateway_endpoint(config_path)?;
    let settings = fetch_gateway_json(&gateway, "/api/settings").await?;
    let usage = fetch_gateway_json(&gateway, "/api/usage/coding").await.ok();
    if json_output {
        return print_pretty_json(&json!({
            "providers": provider_list_rows(&settings, usage.as_ref()),
            "usage": usage,
        }));
    }
    print!("{}", format_provider_list_table(&settings, usage.as_ref()));
    Ok(())
}

pub(crate) async fn cmd_provider_show(
    config_path: &str,
    provider: &str,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let descriptor = provider_descriptor_for_slug(provider)?;
    let gateway = gateway_endpoint(config_path)?;
    let settings = fetch_gateway_json(&gateway, "/api/settings").await?;
    let config = provider_config_from_settings(&settings, &descriptor);
    if json_output {
        return print_pretty_json(&json!({
            "provider": descriptor.provider_type.as_slug(),
            "key": descriptor.key,
            "config": config,
        }));
    }
    print!("{}", format_provider_show_table(&descriptor, &config));
    Ok(())
}

pub(crate) async fn cmd_provider_set(
    config_path: &str,
    options: ProviderSetOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let patch = build_provider_set_patch(&options)?;
    let gateway = gateway_endpoint(config_path)?;
    put_gateway_json(&gateway, "/api/settings?merge=true", &patch.patch).await?;

    if options.json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "provider": patch.provider_type.as_slug(),
                "config_key": patch.provider_key,
                "config": Value::Object(patch.provider_config),
            }))?
        );
    } else {
        let default_model = patch
            .provider_config
            .get("default_model")
            .and_then(Value::as_str)
            .unwrap_or("<unchanged>");
        let effort = patch
            .provider_config
            .get("model_reasoning_effort")
            .and_then(Value::as_str)
            .unwrap_or("<unchanged>");
        let service_tier = patch
            .provider_config
            .get("model_service_tier")
            .and_then(Value::as_str)
            .unwrap_or("<unchanged>");
        let auth_source = patch
            .provider_config
            .get("auth_source")
            .and_then(Value::as_str)
            .unwrap_or("<unchanged>");
        println!(
            "Updated provider defaults: {} (key={}, model={default_model}, reasoning={effort}, service_tier={service_tier}, auth_source={auth_source})",
            patch.provider_type.as_slug(),
            patch.provider_key
        );
    }
    Ok(())
}

fn usage_table_providers<'a>(
    payload: &'a Value,
    provider_filter: Option<&str>,
) -> Result<Vec<&'a Value>, Box<dyn std::error::Error>> {
    let providers = payload
        .get("providers")
        .and_then(Value::as_array)
        .ok_or("usage response missing providers array")?;
    let Some(filter) = provider_filter else {
        return Ok(providers.iter().collect());
    };
    let descriptor = provider_descriptor_for_slug(filter)?;
    let Some(usage_id) = usage_provider_id_for_type(&descriptor.provider_type) else {
        return Ok(Vec::new());
    };
    Ok(providers
        .iter()
        .filter(|provider| provider.get("id").and_then(Value::as_str) == Some(usage_id))
        .collect())
}

fn format_usage_model_cell(model: &Value, now: DateTime<FixedOffset>) -> String {
    let remaining = model
        .get("remaining_percent")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let reset = format_reset_countdown(
        model.get("reset_after_seconds").and_then(Value::as_i64),
        model.get("resets_at").and_then(Value::as_str),
        now,
    )
    .or_else(|| {
        model
            .get("description")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    })
    .unwrap_or_else(|| "reset unknown".to_owned());
    format!("{} | {reset}", percent_label(remaining))
}

fn format_usage_table_with_now(
    payload: &Value,
    provider_filter: Option<&str>,
    now: DateTime<FixedOffset>,
) -> Result<String, Box<dyn std::error::Error>> {
    let providers = usage_table_providers(payload, provider_filter)?;
    if providers.is_empty() {
        if let Some(filter) = provider_filter {
            let descriptor = provider_descriptor_for_slug(filter)?;
            return Ok(format!("{}: No quota data\n", descriptor.label));
        }
        return Ok("No quota data\n".to_owned());
    }

    let refreshed_at = payload.get("refreshed_at").and_then(Value::as_str);
    let mut output = String::new();
    writeln!(
        output,
        "{:<14}  {:<8}  {:<24}  {:<24}  STATUS",
        "PROVIDER", "PLAN", "SESSION", "WEEKLY"
    )
    .expect("write string");
    writeln!(output, "{}", "-".repeat(86)).expect("write string");
    for provider in providers {
        let name = provider
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_else(|| provider.get("id").and_then(Value::as_str).unwrap_or("-"));
        let plan = provider
            .get("plan")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("-");
        let session = format_usage_window_cell(provider.get("session"), now);
        let weekly = format_usage_window_cell(provider.get("weekly"), now);
        let status = usage_status_label(provider, refreshed_at, now);
        writeln!(
            output,
            "{name:<14}  {plan:<8}  {session:<24}  {weekly:<24}  {status}"
        )
        .expect("write string");

        let mut models = provider
            .get("models")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        models.sort_by(|left, right| {
            let left_pct = left
                .get("remaining_percent")
                .and_then(Value::as_f64)
                .unwrap_or(101.0);
            let right_pct = right
                .get("remaining_percent")
                .and_then(Value::as_f64)
                .unwrap_or(101.0);
            left_pct
                .partial_cmp(&right_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for model in models {
            let model_name = model
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_else(|| model.get("id").and_then(Value::as_str).unwrap_or("-"));
            let cell = format_usage_model_cell(&model, now);
            writeln!(output, "  {model_name:<28}  {cell}").expect("write string");
        }
    }
    Ok(output)
}

fn format_usage_table(
    payload: &Value,
    provider_filter: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    format_usage_table_with_now(payload, provider_filter, Local::now().fixed_offset())
}

pub(crate) async fn cmd_usage(
    config_path: &str,
    provider_filter: Option<&str>,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(provider) = provider_filter {
        provider_descriptor_for_slug(provider)?;
    }
    let gateway = gateway_endpoint(config_path)?;
    // Failures bubble to the shared CLI failure reporter, which prints the
    // `{ok:false, error:{kind, message}}` envelope in --json mode and maps the
    // error kind onto the exit code.
    let payload = fetch_gateway_json(&gateway, "/api/usage/coding").await?;
    if json_output {
        return print_pretty_json(&payload);
    }
    print!("{}", format_usage_table(&payload, provider_filter)?);
    Ok(())
}

pub(crate) async fn cmd_config_provider_model(
    config_path: &str,
    provider: &str,
    model: Option<String>,
    clear_model: bool,
    model_reasoning_effort: Option<String>,
    clear_model_reasoning_effort: bool,
    claude_cli_mode: Option<String>,
    clear_claude_cli_mode: bool,
    claude_cli_path: Option<String>,
    clear_claude_cli_path: bool,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("deprecated - use `garyx provider set`");
    cmd_provider_set(
        config_path,
        ProviderSetOptions {
            provider: provider.to_owned(),
            model,
            clear_model,
            reasoning: model_reasoning_effort,
            clear_reasoning: clear_model_reasoning_effort,
            claude_cli_mode: if clear_claude_cli_mode {
                Some(String::new())
            } else {
                claude_cli_mode
            },
            claude_cli_path: if clear_claude_cli_path {
                Some(String::new())
            } else {
                claude_cli_path
            },
            json_output,
            ..ProviderSetOptions::default()
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]

    use super::*;
    use crate::commands::test_support::*;
    use axum::{Json, Router, http::StatusCode, routing::put};
    use std::sync::{Arc as StdArc, Mutex};
    use tempfile::tempdir;
    use tokio::{net::TcpListener, task::JoinHandle};

    async fn spawn_settings_update_http_test_server(
        requests: StdArc<Mutex<Vec<RecordedRequest>>>,
    ) -> (String, JoinHandle<()>) {
        let put_requests = requests.clone();
        let app = Router::new().route(
            "/api/settings",
            put(move |uri: axum::http::Uri, Json(payload): Json<Value>| {
                let requests = put_requests.clone();
                async move {
                    requests
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest {
                            method: "PUT".to_owned(),
                            path: uri
                                .path_and_query()
                                .map(|value| value.as_str().to_owned())
                                .unwrap_or_else(|| "/api/settings".to_owned()),
                            body: payload,
                        });
                    (StatusCode::OK, Json(json!({"ok": true})))
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test router");
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn cmd_config_provider_model_puts_settings_patch() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_settings_update_http_test_server(requests.clone()).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_config_provider_model(
            config_path.to_str().expect("config path"),
            "claude_code",
            Some("claude-opus-4-8".to_owned()),
            false,
            Some("max".to_owned()),
            false,
            None,
            false,
            None,
            false,
            true,
        )
        .await
        .expect("provider model update should succeed");

        handle.abort();

        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].method, "PUT");
        assert_eq!(records[0].path, "/api/settings?merge=true");
        assert_eq!(
            records[0].body["agents"]["claude"]["provider_type"],
            "claude_code"
        );
        assert_eq!(
            records[0].body["agents"]["claude"]["default_model"],
            "claude-opus-4-8"
        );
        assert_eq!(
            records[0].body["agents"]["claude"]["model_reasoning_effort"],
            "max"
        );
    }

    #[tokio::test]
    async fn cmd_config_provider_model_clears_native_provider_defaults() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_settings_update_http_test_server(requests.clone()).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_config_provider_model(
            config_path.to_str().expect("config path"),
            "anthropic",
            None,
            true,
            None,
            true,
            None,
            false,
            None,
            false,
            true,
        )
        .await
        .expect("provider model clear should succeed");

        handle.abort();

        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].method, "PUT");
        assert_eq!(records[0].path, "/api/settings?merge=true");
        assert_eq!(
            records[0].body["agents"]["anthropic"]["provider_type"],
            "anthropic"
        );
        assert_eq!(records[0].body["agents"]["anthropic"]["default_model"], "");
        assert_eq!(
            records[0].body["agents"]["anthropic"]["model_reasoning_effort"],
            ""
        );
    }

    #[tokio::test]
    async fn cmd_config_provider_model_rejects_unknown_provider_without_request() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_settings_update_http_test_server(requests.clone()).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        let error = cmd_config_provider_model(
            config_path.to_str().expect("config path"),
            "unknown_provider",
            Some("model-x".to_owned()),
            false,
            None,
            false,
            None,
            false,
            None,
            false,
            true,
        )
        .await
        .expect_err("unknown provider should fail");

        handle.abort();

        assert!(
            error
                .to_string()
                .contains("unsupported provider type: unknown_provider")
        );
        assert!(requests.lock().expect("request lock").is_empty());
    }

    #[tokio::test]
    async fn cmd_config_provider_model_puts_claude_cli_mode_patch() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_settings_update_http_test_server(requests.clone()).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_config_provider_model(
            config_path.to_str().expect("config path"),
            "claude_code",
            None,
            false,
            None,
            false,
            Some("cctty".to_owned()),
            false,
            Some("/opt/garyx/bin/custom-cctty".to_owned()),
            false,
            true,
        )
        .await
        .expect("claude cli mode update should succeed");

        handle.abort();

        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].method, "PUT");
        assert_eq!(records[0].path, "/api/settings?merge=true");
        assert_eq!(
            records[0].body["agents"]["claude"]["provider_type"],
            "claude_code"
        );
        assert_eq!(
            records[0].body["agents"]["claude"]["claude_cli_mode"],
            "cctty"
        );
        assert_eq!(
            records[0].body["agents"]["claude"]["claude_cli_path"],
            "/opt/garyx/bin/custom-cctty"
        );
    }

    #[tokio::test]
    async fn cmd_config_provider_model_rejects_claude_cli_mode_for_other_providers() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_settings_update_http_test_server(requests.clone()).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        let error = cmd_config_provider_model(
            config_path.to_str().expect("config path"),
            "codex_app_server",
            None,
            false,
            None,
            false,
            Some("cctty".to_owned()),
            false,
            None,
            false,
            true,
        )
        .await
        .expect_err("claude cli options should be claude-only");

        handle.abort();

        assert!(
            error
                .to_string()
                .contains("only supported for provider claude_code")
        );
        assert!(requests.lock().expect("request lock").is_empty());
    }

    #[test]
    fn provider_model_config_key_maps_configurable_provider_types() {
        assert_eq!(
            provider_model_config_key(&ProviderType::ClaudeCode).unwrap(),
            "claude"
        );
        assert_eq!(
            provider_model_config_key(&ProviderType::CodexAppServer).unwrap(),
            "codex"
        );
        assert_eq!(
            provider_model_config_key(&ProviderType::AntigravityCli).unwrap(),
            "antigravity"
        );
        assert_eq!(
            provider_model_config_key(&ProviderType::Gpt).unwrap(),
            "gpt"
        );
        assert_eq!(
            provider_model_config_key(&ProviderType::ClaudeLlm).unwrap(),
            "anthropic"
        );
        assert_eq!(
            provider_model_config_key(&ProviderType::GeminiLlm).unwrap(),
            "google"
        );
    }

    #[test]
    fn provider_set_patch_writes_native_api_key_env_shape() {
        let patch = build_provider_set_patch(&ProviderSetOptions {
            provider: "gpt".to_owned(),
            model: Some("gpt-5.5".to_owned()),
            reasoning: Some("high".to_owned()),
            service_tier: Some("priority".to_owned()),
            base_url: Some("https://example.invalid/v1".to_owned()),
            api_key: Some("sk-openai-EXAMPLE".to_owned()),
            auth_source: Some("api_key".to_owned()),
            env: vec!["OPENAI_ORG=org-test".to_owned()],
            clear_env: vec!["OLD_KEY".to_owned()],
            json_output: true,
            ..ProviderSetOptions::default()
        })
        .expect("provider set patch");

        assert_eq!(patch.provider_key, "gpt");
        assert_eq!(
            patch.patch,
            json!({
                "agents": {
                    "gpt": {
                        "provider_type": "gpt",
                        "default_model": "gpt-5.5",
                        "model_reasoning_effort": "high",
                        "model_service_tier": "priority",
                        "base_url": "https://example.invalid/v1",
                        "auth_source": "api_key",
                        "env": {
                            "OLD_KEY": "",
                            "OPENAI_ORG": "org-test",
                            "OPENAI_API_KEY": "sk-openai-EXAMPLE"
                        }
                    }
                }
            })
        );
    }

    #[tokio::test]
    async fn cmd_provider_set_puts_settings_patch() {
        let requests = StdArc::new(Mutex::new(Vec::new()));
        let (base_url, handle) = spawn_settings_update_http_test_server(requests.clone()).await;
        let dir = tempdir().expect("tempdir");
        let config_path = write_test_gateway_config(&dir, &base_url);

        cmd_provider_set(
            config_path.to_str().expect("config path"),
            ProviderSetOptions {
                provider: "anthropic".to_owned(),
                model: Some("claude-sonnet-4-6".to_owned()),
                clear_reasoning: true,
                api_key: Some("sk-ant-EXAMPLE".to_owned()),
                json_output: true,
                ..ProviderSetOptions::default()
            },
        )
        .await
        .expect("provider set should succeed");

        handle.abort();

        let records = requests.lock().expect("request lock");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].method, "PUT");
        assert_eq!(records[0].path, "/api/settings?merge=true");
        assert_eq!(
            records[0].body["agents"]["anthropic"]["provider_type"],
            "anthropic"
        );
        assert_eq!(
            records[0].body["agents"]["anthropic"]["default_model"],
            "claude-sonnet-4-6"
        );
        assert_eq!(
            records[0].body["agents"]["anthropic"]["model_reasoning_effort"],
            ""
        );
        assert_eq!(
            records[0].body["agents"]["anthropic"]["env"]["ANTHROPIC_API_KEY"],
            "sk-ant-EXAMPLE"
        );
    }

    #[test]
    fn usage_severity_and_reset_countdown_follow_provider_spec() {
        let now = DateTime::parse_from_rfc3339("2030-01-01T00:00:00Z").expect("now");

        assert_eq!(usage_severity(true, 50.0), UsageSeverity::Healthy);
        assert_eq!(usage_severity(true, 20.0), UsageSeverity::Warning);
        assert_eq!(usage_severity(true, 19.9), UsageSeverity::Critical);
        assert_eq!(usage_severity(false, 99.0), UsageSeverity::Unavailable);

        assert_eq!(
            format_reset_countdown(Some(187_200), None, now).as_deref(),
            Some("resets in 2d 4h")
        );
        assert_eq!(
            format_reset_countdown(Some(4_320), None, now).as_deref(),
            Some("resets in 1h 12m")
        );
        assert_eq!(
            format_reset_countdown(Some(30), None, now).as_deref(),
            Some("resets in <1m")
        );
        assert_eq!(
            format_reset_countdown(None, Some("2030-01-01T03:00:00Z"), now).as_deref(),
            Some("resets in 3h")
        );
    }

    #[test]
    fn usage_table_formats_windows_and_antigravity_models() {
        let payload = json!({
            "refreshed_at": "2030-01-01T00:00:00Z",
            "providers": [
                {
                    "id": "claude_code",
                    "name": "Claude Code",
                    "available": true,
                    "plan": "max",
                    "session": {
                        "used_percent": 2.0,
                        "remaining_percent": 98.0,
                        "reset_after_seconds": 7200
                    },
                    "weekly": {
                        "used_percent": 27.0,
                        "remaining_percent": 73.0,
                        "reset_after_seconds": 432000
                    }
                },
                {
                    "id": "codex",
                    "name": "Codex",
                    "available": true,
                    "stale": true,
                    "plan": "pro",
                    "session": {
                        "used_percent": 2.0,
                        "remaining_percent": 98.0,
                        "reset_after_seconds": 10800
                    },
                    "weekly": {
                        "used_percent": 89.0,
                        "remaining_percent": 11.0,
                        "reset_after_seconds": 172800
                    }
                },
                {
                    "id": "antigravity",
                    "name": "Antigravity",
                    "available": true,
                    "models": [
                        {
                            "id": "claude-opus-test",
                            "name": "claude-opus-test",
                            "remaining_percent": 99.0,
                            "reset_after_seconds": 3600
                        },
                        {
                            "id": "gemini-flash-test",
                            "name": "gemini-flash-test",
                            "remaining_percent": 84.0,
                            "reset_after_seconds": 18000
                        }
                    ]
                }
            ]
        });
        let now = DateTime::parse_from_rfc3339("2030-01-01T00:03:00Z").expect("now");

        let table =
            format_usage_table_with_now(&payload, None, now).expect("usage table should render");

        assert_eq!(
            table,
            concat!(
                "PROVIDER        PLAN      SESSION                   WEEKLY                    STATUS\n",
                "--------------------------------------------------------------------------------------\n",
                "Claude Code     max       98% | resets in 2h        73% | resets in 5d        ok\n",
                "Codex           pro       98% | resets in 3h        11% | resets in 2d        stale (updated 3m ago)\n",
                "Antigravity     -         -                         -                         ok\n",
                "  gemini-flash-test             84% | resets in 5h\n",
                "  claude-opus-test              99% | resets in 1h\n",
            )
        );
    }

    #[test]
    fn provider_list_rows_include_all_model_providers() {
        let settings = json!({
            "agents": {
                "gpt": {
                    "provider_type": "gpt",
                    "default_model": "gpt-5.5",
                    "auth_source": "api_key",
                    "env": {
                        "OPENAI_API_KEY": "sk-openai-EXAMPLE"
                    }
                }
            }
        });
        let usage = json!({
            "refreshed_at": "2030-01-01T00:00:00Z",
            "providers": [
                {
                    "id": "claude_code",
                    "name": "Claude Code",
                    "available": true,
                    "weekly": {
                        "used_percent": 27.0,
                        "remaining_percent": 73.0
                    }
                }
            ]
        });

        let rows = provider_list_rows(&settings, Some(&usage));

        assert_eq!(rows.len(), 7);
        assert_eq!(rows[0]["provider"], "Claude Code");
        assert_eq!(rows[0]["usage"], "73% wk");
        let gpt = rows
            .iter()
            .find(|row| row["type"] == "gpt")
            .expect("gpt row");
        assert_eq!(gpt["auth"], "api key");
        assert_eq!(gpt["default_model"], "gpt-5.5");
    }
}
