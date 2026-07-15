use super::*;

pub(crate) fn canonical_channel_id(channel: &str) -> String {
    let normalized = channel.trim().to_ascii_lowercase();
    available_channel_options()
        .into_iter()
        .find_map(|option| {
            (option.id == normalized || option.aliases.iter().any(|alias| alias == &normalized))
                .then_some(option.id)
        })
        .unwrap_or(normalized)
}

#[derive(Debug, Clone)]
struct ChannelOption {
    id: String,
    display_name: String,
    aliases: Vec<String>,
}

impl ChannelOption {
    fn from_metadata(metadata: PluginMetadata) -> Self {
        Self {
            id: metadata.id,
            display_name: metadata.display_name,
            aliases: metadata.aliases,
        }
    }
}

fn available_channel_options() -> Vec<ChannelOption> {
    let mut options: Vec<ChannelOption> = builtin_plugin_metadata_list()
        .into_iter()
        .map(ChannelOption::from_metadata)
        .collect();

    if let Ok(outcome) = ManifestDiscoverer::new(crate::channel_plugin_host::plugin_root_paths(
        &GaryxConfig::default(),
    ))
    .discover()
    {
        for manifest in outcome.plugins {
            if options.iter().any(|option| option.id == manifest.plugin.id) {
                continue;
            }
            options.push(ChannelOption {
                id: manifest.plugin.id.clone(),
                display_name: if manifest.plugin.display_name.trim().is_empty() {
                    manifest.plugin.id.clone()
                } else {
                    manifest.plugin.display_name.clone()
                },
                aliases: manifest.plugin.aliases.clone(),
            });
        }
    }

    options.push(ChannelOption {
        id: "api".to_owned(),
        display_name: "API".to_owned(),
        aliases: Vec::new(),
    });
    options
}

fn channel_display_name(channel: &str) -> Option<String> {
    let resolved = canonical_channel_id(channel);
    available_channel_options()
        .into_iter()
        .find(|option| option.id == resolved)
        .map(|option| option.display_name)
}

fn plugin_account_mut<'a>(
    cfg: &'a mut GaryxConfig,
    channel: &str,
    account: &str,
) -> Option<&'a mut PluginAccountEntry> {
    cfg.channels
        .plugins
        .get_mut(channel)
        .and_then(|plugin_cfg| plugin_cfg.accounts.get_mut(account))
}

pub(crate) fn cmd_channels_list(
    config_path: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    let cfg = loaded.config;
    if json {
        println!("{}", serde_json::to_string_pretty(&cfg.channels)?);
    } else {
        println!("api:");
        for (id, account) in &cfg.channels.api.accounts {
            println!("  - {} (enabled={})", id, account.enabled);
        }
        for (plugin_id, plugin_cfg) in &cfg.channels.plugins {
            println!("{plugin_id} (plugin):");
            for (id, entry) in &plugin_cfg.accounts {
                println!("  - {} (enabled={})", id, entry.enabled);
            }
        }
    }
    Ok(())
}

pub(crate) async fn cmd_channels_enable(
    config_path: &str,
    channel: &str,
    account: &str,
    enabled: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    let config_path = loaded.path;
    let mut cfg = loaded.config;
    let channel = canonical_channel_id(channel);
    match channel.as_str() {
        "api" => {
            let Some(acc) = cfg.channels.api.accounts.get_mut(account) else {
                eprintln!("API account not found: {account}");
                std::process::exit(1);
            };
            acc.enabled = enabled;
        }
        _ => {
            let Some(entry) = plugin_account_mut(&mut cfg, &channel, account) else {
                eprintln!("{channel} account not found: {account}");
                std::process::exit(1);
            };
            entry.enabled = enabled;
        }
    }
    save_config_struct(&config_path, &cfg)?;
    println!("Updated {channel}.{account}.enabled={enabled}");
    notify_gateway_reload(&config_path).await;
    Ok(())
}

pub(crate) async fn cmd_channels_remove(
    config_path: &str,
    channel: &str,
    account: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    let config_path = loaded.path;
    let mut cfg = loaded.config;
    let channel = canonical_channel_id(channel);
    let removed = match channel.as_str() {
        "api" => cfg.channels.api.accounts.remove(account).is_some(),
        _ => cfg
            .channels
            .plugins
            .get_mut(&channel)
            .and_then(|plugin_cfg| plugin_cfg.accounts.remove(account))
            .is_some(),
    };
    if !removed {
        eprintln!("Account not found: {channel}.{account}");
        std::process::exit(1);
    }
    save_config_struct(&config_path, &cfg)?;
    println!("Removed {channel}.{account}");
    notify_gateway_reload(&config_path).await;
    Ok(())
}

/// Collected channel-account fields gathered from either CLI flags or
/// interactive prompts. Shared by `garyx channels add` and the onboarding
/// flow so they stay perfectly in sync.
#[derive(Debug, Default, Clone)]
pub(crate) struct ChannelOverrides {
    pub account: Option<String>,
    pub name: Option<String>,
    pub workspace_dir: Option<String>,
    pub workspace_mode: Option<String>,
    pub agent_id: Option<String>,
    pub token: Option<String>,
    pub uin: Option<String>,
    pub base_url: Option<String>,
    pub app_id: Option<String>,
    pub app_secret: Option<String>,
    /// Feishu brand: `feishu` (国内 / default) or `lark` (海外).
    /// Parsed via [`parse_feishu_domain`] when calling `upsert_channel_account`.
    pub domain: Option<String>,
    /// Extra channel-owned config fields not modelled by the built-in channel
    /// flag surface. These get merged into `channels.<channel_id>.accounts[*].config`.
    pub plugin_extras: Map<String, Value>,
}

/// Parse a user-provided domain string into a [`FeishuDomain`], accepting
/// the common synonyms users actually type.
pub(crate) fn parse_feishu_domain(raw: &str) -> Option<FeishuDomain> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "feishu" | "lark-cn" | "cn" | "飞书" | "国内" | "domestic" => {
            Some(FeishuDomain::Feishu)
        }
        "lark" | "larksuite" | "intl" | "international" | "海外" => Some(FeishuDomain::Lark),
        _ => None,
    }
}

fn trim_opt(value: Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn normalize_channel_workspace_mode(
    value: Option<String>,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let Some(value) = trim_opt(value) else {
        return Ok(None);
    };
    match value.to_ascii_lowercase().as_str() {
        "local" => Ok(Some("local".to_owned())),
        "worktree" => Ok(Some("worktree".to_owned())),
        _ => Err(format!("invalid workspace mode `{value}`; use `local` or `worktree`").into()),
    }
}

fn discover_plugin_manifest(
    channel: &str,
) -> Result<Option<PluginManifest>, Box<dyn std::error::Error>> {
    let resolved = canonical_channel_id(channel);
    if matches!(
        resolved.as_str(),
        BUILTIN_CHANNEL_PLUGIN_TELEGRAM
            | BUILTIN_CHANNEL_PLUGIN_DISCORD
            | BUILTIN_CHANNEL_PLUGIN_FEISHU
            | BUILTIN_CHANNEL_PLUGIN_WEIXIN
            | "api"
    ) {
        return Ok(None);
    }

    let outcome = ManifestDiscoverer::new(crate::channel_plugin_host::plugin_root_paths(
        &GaryxConfig::default(),
    ))
    .discover()?;
    Ok(outcome
        .plugins
        .into_iter()
        .find(|manifest| manifest.plugin.id == resolved))
}

fn plugin_form_state_from_overrides(overrides: &ChannelOverrides) -> Map<String, Value> {
    let mut map = overrides.plugin_extras.clone();
    if let Some(value) = overrides
        .token
        .as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        map.insert("token".to_owned(), Value::String(value.to_owned()));
    }
    if let Some(value) = overrides
        .uin
        .as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        map.insert("uin".to_owned(), Value::String(value.to_owned()));
    }
    if let Some(value) = overrides
        .base_url
        .as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        map.insert("base_url".to_owned(), Value::String(value.to_owned()));
    }
    if let Some(value) = overrides
        .app_id
        .as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        map.insert("app_id".to_owned(), Value::String(value.to_owned()));
    }
    if let Some(value) = overrides
        .app_secret
        .as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        map.insert("app_secret".to_owned(), Value::String(value.to_owned()));
    }
    if let Some(value) = overrides
        .domain
        .as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        map.insert("domain".to_owned(), Value::String(value.to_owned()));
    }
    map
}

fn set_plugin_form_value(overrides: &mut ChannelOverrides, key: &str, value: Value) {
    match (key, value) {
        ("token", Value::String(value)) => overrides.token = Some(value),
        ("uin", Value::String(value)) => overrides.uin = Some(value),
        ("base_url", Value::String(value)) => overrides.base_url = Some(value),
        ("app_id", Value::String(value)) => overrides.app_id = Some(value),
        ("app_secret", Value::String(value)) => overrides.app_secret = Some(value),
        ("domain", Value::String(value)) => overrides.domain = Some(value),
        (other_key, other_value) => {
            overrides
                .plugin_extras
                .insert(other_key.to_owned(), other_value);
        }
    }
}

fn plugin_suggested_account_id(
    values: &std::collections::BTreeMap<String, Value>,
) -> Option<String> {
    for key in ["account_id", "agent_id"] {
        match values.get(key) {
            Some(Value::String(value)) if !value.trim().is_empty() => return Some(value.clone()),
            Some(_) | None => {}
        }
    }
    None
}

fn plugin_schema_declares_field(schema: &Value, key: &str) -> bool {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|properties| properties.contains_key(key))
}

fn strip_plugin_identity_hints(
    values: &mut std::collections::BTreeMap<String, Value>,
    schema: &Value,
) {
    if !plugin_schema_declares_field(schema, "account_id") {
        values.remove("account_id");
    }
    if !plugin_schema_declares_field(schema, "agent_id") {
        values.remove("agent_id");
    }
}

pub(super) fn value_is_missing(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(value)) => value.trim().is_empty(),
        Some(Value::Array(values)) => values.is_empty(),
        Some(Value::Object(values)) => values.is_empty(),
        _ => false,
    }
}

pub(super) fn plugin_required_fields(schema: &Value) -> Vec<String> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn validate_plugin_required_fields(
    channel: &str,
    schema: &Value,
    form_state: &Map<String, Value>,
) -> Result<(), Box<dyn std::error::Error>> {
    let missing: Vec<String> = plugin_required_fields(schema)
        .into_iter()
        .filter(|key| value_is_missing(form_state.get(key)))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "missing required fields for plugin `{channel}`: {}",
            missing.join(", ")
        )
        .into())
    }
}

fn schema_property_secret(name: &str, schema: &Value) -> bool {
    let name = name.to_ascii_lowercase();
    if name.contains("token")
        || name.contains("secret")
        || name.contains("password")
        || name.contains("api_key")
        || name.contains("apikey")
    {
        return true;
    }
    if schema.get("format").and_then(Value::as_str) == Some("password") {
        return true;
    }
    schema
        .get("x-garyx")
        .and_then(Value::as_object)
        .and_then(|obj| obj.get("secret"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

struct CliPluginAuthFlowHandler;

#[async_trait]
impl InboundHandler for CliPluginAuthFlowHandler {
    async fn on_request(&self, method: String, _params: Value) -> Result<Value, (i32, String)> {
        Err((
            PluginErrorCode::MethodNotFound.as_i32(),
            format!("cli auth-flow host does not handle `{method}`"),
        ))
    }

    async fn on_notification(&self, _method: String, _params: Value) {}
}

async fn run_plugin_auth_flow(
    manifest: &PluginManifest,
    form_state: Value,
) -> Result<std::collections::BTreeMap<String, Value>, Box<dyn std::error::Error>> {
    run_plugin_auth_flow_with_options(manifest, form_state, false).await
}

async fn run_plugin_auth_flow_with_options(
    manifest: &PluginManifest,
    form_state: Value,
    json_events: bool,
) -> Result<std::collections::BTreeMap<String, Value>, Box<dyn std::error::Error>> {
    let plugin = SubprocessPlugin::spawn(
        manifest,
        SpawnOptions::default(),
        Arc::new(CliPluginAuthFlowHandler),
    )?;
    let executor = SubprocessAuthFlowExecutor::new(manifest.plugin.id.clone(), plugin.client());
    let result = run_auth_flow_with_options(&executor, form_state, json_events).await;
    let _ = plugin.shutdown_gracefully().await;
    result
}

fn prompt_plugin_auth_mode(manifest: &PluginManifest) -> Result<bool, Box<dyn std::error::Error>> {
    use dialoguer::{Select, theme::ColorfulTheme};
    if manifest.auth_flows.len() == 1 {
        let prompt = manifest.auth_flows[0].prompt.trim();
        if !prompt.is_empty() {
            println!("{prompt}");
        }
    }
    let auto_label = if manifest.auth_flows.len() == 1 {
        let label = manifest.auth_flows[0].label.trim();
        if label.is_empty() {
            format!("使用 {} 的登录流程", manifest.plugin.display_name)
        } else {
            label.to_owned()
        }
    } else {
        format!("使用 {} 的登录流程", manifest.plugin.display_name)
    };
    let items = [auto_label.as_str(), "手动填写插件配置"];
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("选择绑定方式")
        .default(0)
        .items(&items)
        .interact()?;
    Ok(selection == 0)
}

fn prompt_channel_identity_if_missing(
    channel: &str,
    overrides: &mut ChannelOverrides,
) -> Result<(), Box<dyn std::error::Error>> {
    if overrides.account.is_some() && overrides.name.is_some() {
        return Ok(());
    }
    let default_label = overrides
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            overrides
                .account
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
        .or_else(|| channel_display_name(channel));
    let value = prompt_value("名称", default_label.as_deref(), false)?;
    let value = value.trim().to_owned();
    if overrides.account.is_none() {
        overrides.account = Some(value.clone());
    }
    if overrides.name.is_none() {
        overrides.name = Some(value);
    }
    Ok(())
}

fn prompt_plugin_schema_value(
    field_name: &str,
    field_schema: &Value,
    required: bool,
) -> Result<Option<Value>, Box<dyn std::error::Error>> {
    let label = field_schema
        .get("title")
        .and_then(Value::as_str)
        .or_else(|| field_schema.get("description").and_then(Value::as_str))
        .unwrap_or(field_name);
    let default_string = match field_schema.get("default") {
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Bool(value)) => Some(if *value { "true" } else { "false" }.to_owned()),
        Some(Value::Number(value)) => Some(value.to_string()),
        _ => None,
    };
    let field_type = field_schema
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("string");

    match field_type {
        "string" => {
            let value = if schema_property_secret(field_name, field_schema) {
                prompt_secret(label)?
            } else {
                prompt_value(label, default_string.as_deref(), !required)?
            };
            if value.trim().is_empty() && !required {
                Ok(None)
            } else {
                Ok(Some(Value::String(value)))
            }
        }
        "integer" => {
            let value = prompt_value(label, default_string.as_deref(), !required)?;
            if value.trim().is_empty() && !required {
                return Ok(None);
            }
            let parsed = value
                .trim()
                .parse::<i64>()
                .map_err(|err| format!("field `{field_name}` expects integer: {err}"))?;
            Ok(Some(Value::Number(parsed.into())))
        }
        "number" => {
            let value = prompt_value(label, default_string.as_deref(), !required)?;
            if value.trim().is_empty() && !required {
                return Ok(None);
            }
            let parsed = value
                .trim()
                .parse::<f64>()
                .map_err(|err| format!("field `{field_name}` expects number: {err}"))?;
            Ok(Some(json!(parsed)))
        }
        "boolean" => {
            let value = prompt_value(label, default_string.as_deref(), !required)?;
            if value.trim().is_empty() && !required {
                return Ok(None);
            }
            let parsed = match value.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "y" | "on" => true,
                "false" | "0" | "no" | "n" | "off" => false,
                other => {
                    return Err(
                        format!("field `{field_name}` expects boolean, got `{other}`").into(),
                    );
                }
            };
            Ok(Some(Value::Bool(parsed)))
        }
        other => {
            if required {
                Err(format!(
                    "plugin field `{field_name}` uses unsupported schema type `{other}`; use the desktop UI for this plugin"
                )
                .into())
            } else {
                Ok(None)
            }
        }
    }
}

async fn interactive_fill_plugin_channel_overrides(
    manifest: &PluginManifest,
    mut overrides: ChannelOverrides,
) -> Result<ChannelOverrides, Box<dyn std::error::Error>> {
    let mut form_state = plugin_form_state_from_overrides(&overrides);
    if !manifest.auth_flows.is_empty()
        && plugin_required_fields(&manifest.schema)
            .iter()
            .any(|field| value_is_missing(form_state.get(field)))
        && prompt_plugin_auth_mode(manifest)?
    {
        let mut values = run_plugin_auth_flow(manifest, Value::Object(form_state.clone())).await?;
        if overrides.account.is_none()
            && let Some(account_id) = plugin_suggested_account_id(&values)
        {
            overrides.account = Some(account_id);
        }
        strip_plugin_identity_hints(&mut values, &manifest.schema);
        for (key, value) in values {
            set_plugin_form_value(&mut overrides, &key, value);
        }
        form_state = plugin_form_state_from_overrides(&overrides);
    }

    let properties = manifest
        .schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let required = plugin_required_fields(&manifest.schema);

    for (field_name, field_schema) in &properties {
        if value_is_missing(form_state.get(field_name))
            && required.iter().any(|name| name == field_name)
            && let Some(value) = prompt_plugin_schema_value(field_name, field_schema, true)?
        {
            set_plugin_form_value(&mut overrides, field_name, value);
            form_state = plugin_form_state_from_overrides(&overrides);
        }
    }
    for (field_name, field_schema) in &properties {
        if required.iter().any(|name| name == field_name)
            || !value_is_missing(form_state.get(field_name))
        {
            continue;
        }
        if let Some(value) = prompt_plugin_schema_value(field_name, field_schema, false)? {
            set_plugin_form_value(&mut overrides, field_name, value);
            form_state = plugin_form_state_from_overrides(&overrides);
        }
    }

    Ok(overrides)
}

/// Fill any still-empty fields of `overrides` by prompting the user.
///
/// Assumes the caller already verified that stdin/stdout are interactive
/// (via `can_prompt_interactively`). For weixin, this triggers the full QR
/// login flow and reads `token` + `account` out of the scan result.
async fn interactive_fill_channel_overrides(
    channel: &str,
    mut overrides: ChannelOverrides,
) -> Result<ChannelOverrides, Box<dyn std::error::Error>> {
    if let Some(manifest) = discover_plugin_manifest(channel)? {
        if overrides.workspace_dir.is_none() {
            let value = prompt_value("运行目录（回车跳过，默认用户目录）", None, true)?;
            if !value.trim().is_empty() {
                overrides.workspace_dir = Some(value);
            }
        }
        if overrides.agent_id.is_none() {
            overrides.agent_id = prompt_agent_reference_choice(None)?;
        }
        overrides = interactive_fill_plugin_channel_overrides(&manifest, overrides).await?;
        prompt_channel_identity_if_missing(channel, &mut overrides)?;
        return Ok(overrides);
    }

    // One "名称" prompt fills both the internal account id (HashMap key in
    // config) and the display name — users almost always want them to match,
    // and asking twice is just noise. Callers who care about distinguishing
    // the two can still pass `--account` and `--name` explicitly.
    if channel != "weixin" && (overrides.account.is_none() || overrides.name.is_none()) {
        prompt_channel_identity_if_missing(channel, &mut overrides)?;
    }
    if overrides.workspace_dir.is_none() {
        let value = prompt_value("运行目录（回车跳过，默认用户目录）", None, true)?;
        if !value.trim().is_empty() {
            overrides.workspace_dir = Some(value);
        }
    }
    if overrides.agent_id.is_none() {
        overrides.agent_id = prompt_agent_reference_choice(None)?;
    }
    match channel {
        "telegram" if overrides.token.is_none() => {
            overrides.token = Some(prompt_secret("Telegram Bot Token")?);
        }
        "discord" if overrides.token.is_none() => {
            overrides.token = Some(prompt_secret("Discord Bot Token")?);
        }
        "feishu" => {
            // If the caller already passed app_id + app_secret via flags,
            // skip the mode selector — they clearly want manual.
            let has_credentials = overrides.app_id.is_some() && overrides.app_secret.is_some();
            if !has_credentials {
                let use_device_flow = prompt_feishu_auth_mode()?;
                if use_device_flow {
                    let domain = match overrides.domain.as_deref().and_then(parse_feishu_domain) {
                        Some(value) => value,
                        None => prompt_feishu_domain_choice()?,
                    };
                    let domain_str = match domain {
                        FeishuDomain::Feishu => "feishu",
                        FeishuDomain::Lark => "lark",
                    };
                    let executor = FeishuAuthExecutor::new(reqwest::Client::new());
                    let mut values =
                        run_auth_flow(&executor, json!({ "domain": domain_str })).await?;
                    overrides.app_id = Some(take_string(&mut values, "app_id")?);
                    overrides.app_secret = Some(take_string(&mut values, "app_secret")?);
                    overrides.domain = Some(take_string(&mut values, "domain")?);
                } else {
                    if overrides.app_id.is_none() {
                        overrides.app_id = Some(prompt_value("Feishu App ID", None, false)?);
                    }
                    if overrides.app_secret.is_none() {
                        overrides.app_secret = Some(prompt_secret("Feishu App Secret")?);
                    }
                    if overrides.domain.is_none() {
                        let domain = prompt_feishu_domain_choice()?;
                        overrides.domain = Some(match domain {
                            FeishuDomain::Feishu => "feishu".to_owned(),
                            FeishuDomain::Lark => "lark".to_owned(),
                        });
                    }
                }
            }
        }
        "weixin" => {
            if overrides.account.is_none() {
                let value = prompt_value("账号名（回车则使用扫码返回的 bot id）", None, true)?;
                if !value.trim().is_empty() {
                    overrides.account = Some(value);
                }
            }
            // Weixin base_url intentionally not prompted — default works
            // for everyone; advanced users can still pass --base-url.
            if overrides.token.is_none() {
                let executor = WeixinAuthExecutor::new(reqwest::Client::new());
                let mut values = run_auth_flow(
                    &executor,
                    json!({
                        "base_url": normalize_weixin_base_url(overrides.base_url.clone()),
                        "timeout_secs": 480,
                    }),
                )
                .await?;
                let token = take_string(&mut values, "token")?;
                let base_url = take_string(&mut values, "base_url")?;
                let account_id = take_string(&mut values, "account_id")?;
                if overrides.account.is_none() {
                    overrides.account = Some(account_id);
                }
                overrides.token = Some(token);
                overrides.base_url = Some(base_url);
            }
        }
        "api" => {}
        _ => {}
    }
    Ok(overrides)
}

/// Fully interactive bind of one channel into the in-memory config.
/// Returns the account id that was inserted (or updated). Caller is
/// responsible for persisting the config afterwards.
pub(super) async fn interactive_bind_channel(
    cfg: &mut GaryxConfig,
    channel: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let overrides =
        interactive_fill_channel_overrides(channel, ChannelOverrides::default()).await?;
    let account = overrides
        .account
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("missing account id")?
        .to_owned();
    upsert_channel_account(
        cfg,
        channel,
        &account,
        overrides.name,
        overrides.workspace_dir,
        overrides.workspace_mode,
        overrides.agent_id,
        overrides.token,
        overrides.uin,
        overrides.base_url,
        overrides.app_id,
        overrides.app_secret,
        overrides.domain,
        overrides.plugin_extras,
    )?;
    Ok(account)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn cmd_channels_add(
    config_path: &str,
    channel: Option<String>,
    account: Option<String>,
    name: Option<String>,
    workspace_dir: Option<String>,
    workspace_mode: Option<String>,
    agent_id: Option<String>,
    token: Option<String>,
    uin: Option<String>,
    base_url: Option<String>,
    app_id: Option<String>,
    app_secret: Option<String>,
    domain: Option<String>,
    auto_register: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let interactive = can_prompt_interactively();
    // `--domain` only makes sense for feishu/lark, so if the caller passes
    // it without an explicit channel we infer feishu. `--auto-register`
    // works for feishu now, so it no longer pins a channel by itself —
    // the user still picks interactively.
    let inferred_feishu = domain.is_some();
    let channel = match channel.as_deref() {
        Some(value) => canonical_channel_id(value),
        None if inferred_feishu => "feishu".to_owned(),
        None if interactive => prompt_channel_choice()?,
        None => return Err("missing channel; use e.g. `garyx channels add weixin ...`".into()),
    };

    let mut overrides = ChannelOverrides {
        account: trim_opt(account),
        name: trim_opt(name),
        workspace_dir: trim_opt(workspace_dir),
        workspace_mode: trim_opt(workspace_mode),
        agent_id: trim_opt(agent_id),
        token,
        uin,
        base_url,
        app_id,
        app_secret,
        domain: trim_opt(domain),
        plugin_extras: Map::new(),
    };

    // Non-interactive `--auto-register`: trigger device flow directly
    // without the mode-selection prompt. This still requires a controlling
    // TTY so the user can scan the printed QR / click the URL — we don't
    // attempt to run device flow completely headless.
    if auto_register {
        if let Some(manifest) = discover_plugin_manifest(&channel)? {
            let mut values = run_plugin_auth_flow(
                &manifest,
                Value::Object(plugin_form_state_from_overrides(&overrides)),
            )
            .await?;
            if overrides.account.is_none()
                && let Some(account_id) = plugin_suggested_account_id(&values)
            {
                overrides.account = Some(account_id);
            }
            strip_plugin_identity_hints(&mut values, &manifest.schema);
            for (key, value) in values {
                set_plugin_form_value(&mut overrides, &key, value);
            }
        } else {
            match channel.as_str() {
                "feishu" => {
                    if overrides.app_id.is_some() && overrides.app_secret.is_some() {
                        return Err(
                        "`--auto-register` cannot be combined with `--app-id` / `--app-secret`; \
                         drop the credentials flags or drop `--auto-register`"
                            .into(),
                    );
                    }
                    let domain = overrides
                        .domain
                        .as_deref()
                        .and_then(parse_feishu_domain)
                        .unwrap_or_default();
                    let domain_str = match domain {
                        FeishuDomain::Feishu => "feishu",
                        FeishuDomain::Lark => "lark",
                    };
                    let executor = FeishuAuthExecutor::new(reqwest::Client::new());
                    let mut values =
                        run_auth_flow(&executor, json!({ "domain": domain_str })).await?;
                    overrides.app_id = Some(take_string(&mut values, "app_id")?);
                    overrides.app_secret = Some(take_string(&mut values, "app_secret")?);
                    overrides.domain = Some(take_string(&mut values, "domain")?);
                }
                _ => {
                    return Err(format!(
                        "`--auto-register` is only supported for feishu or plugins with auth flows, not `{channel}`"
                    )
                    .into());
                }
            }
        }
    }

    if interactive {
        overrides = interactive_fill_channel_overrides(&channel, overrides).await?;
    }

    let account = overrides
        .account
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("missing account id")?
        .to_owned();

    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    let config_path = loaded.path;
    let mut cfg = loaded.config;
    upsert_channel_account(
        &mut cfg,
        &channel,
        &account,
        overrides.name,
        overrides.workspace_dir,
        overrides.workspace_mode,
        overrides.agent_id,
        overrides.token,
        overrides.uin,
        overrides.base_url,
        overrides.app_id,
        overrides.app_secret,
        overrides.domain,
        overrides.plugin_extras,
    )?;
    save_config_struct(&config_path, &cfg)?;
    println!("Added {channel}.{account}");
    notify_gateway_reload(&config_path).await;
    Ok(())
}

fn normalize_weixin_base_url(input: Option<String>) -> String {
    input
        .unwrap_or_else(|| "https://ilinkai.weixin.qq.com".to_owned())
        .trim()
        .trim_end_matches('/')
        .to_owned()
}

fn can_prompt_interactively() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn prompt_value(
    label: &str,
    default: Option<&str>,
    allow_empty: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    loop {
        match default {
            Some(value) if !value.is_empty() => {
                print!("{label} [{value}]: ");
            }
            _ => {
                print!("{label}: ");
            }
        }
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let trimmed = input.trim();
        if trimmed.is_empty() {
            if let Some(value) = default
                && (!value.is_empty() || allow_empty)
            {
                return Ok(value.to_owned());
            }
            if allow_empty {
                return Ok(String::new());
            }
            println!("该字段不能为空。");
            continue;
        }
        return Ok(trimmed.to_owned());
    }
}

fn prompt_secret(label: &str) -> Result<String, Box<dyn std::error::Error>> {
    loop {
        let value = rpassword::prompt_password(format!("{label}: "))?;
        let trimmed = value.trim();
        if trimmed.is_empty() {
            println!("该字段不能为空。");
            continue;
        }
        return Ok(trimmed.to_owned());
    }
}

pub(super) fn prompt_channel_choice() -> Result<String, Box<dyn std::error::Error>> {
    use dialoguer::{Select, theme::ColorfulTheme};
    let items = available_channel_options();
    let labels: Vec<&str> = items
        .iter()
        .map(|option| option.display_name.as_str())
        .collect();
    let default_index = items
        .iter()
        .position(|option| option.id == "feishu")
        .unwrap_or(0);
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("选择渠道")
        .default(default_index)
        .items(&labels)
        .interact()?;
    Ok(items[selection].id.clone())
}

#[derive(Debug, Clone)]
struct AgentReferenceOption {
    id: Option<String>,
    label: String,
}

fn provider_type_label(provider_type: &ProviderType) -> &'static str {
    match provider_type {
        ProviderType::CodexAppServer => "Codex",
        ProviderType::Traex => "Traex",
        ProviderType::AntigravityCli => "Antigravity",
        ProviderType::ClaudeCode => "Claude",
    }
}

fn selectable_cli_agent_profiles(
    profiles: impl IntoIterator<Item = CustomAgentProfile>,
) -> Vec<CustomAgentProfile> {
    let mut profiles = profiles
        .into_iter()
        .filter(|profile| profile.standalone && profile.enabled)
        .collect::<Vec<_>>();
    profiles.sort_by(|left, right| {
        left.built_in
            .cmp(&right.built_in)
            .reverse()
            .then_with(|| left.display_name.cmp(&right.display_name))
            .then_with(|| left.agent_id.cmp(&right.agent_id))
    });
    profiles
}

fn parse_cli_agent_profiles(
    content: &str,
) -> Result<Vec<CustomAgentProfile>, garyx_models::AgentStoreDocumentError> {
    parse_agent_store_document(content)
        .map(|document| selectable_cli_agent_profiles(document.agents))
}

fn load_cli_agent_profiles() -> Vec<CustomAgentProfile> {
    let path = default_custom_agents_state_path();
    fs::read_to_string(&path)
        .ok()
        .filter(|content| !content.trim().is_empty())
        .and_then(|content| parse_cli_agent_profiles(&content).ok())
        .unwrap_or_else(|| selectable_cli_agent_profiles(builtin_provider_agent_profiles()))
}

fn format_cli_agent_label(agent: &CustomAgentProfile) -> String {
    let display = if agent.display_name.trim() == agent.agent_id.trim() {
        agent.display_name.clone()
    } else {
        format!("{} ({})", agent.display_name, agent.agent_id)
    };
    let kind = if agent.built_in {
        "内置 Agent"
    } else {
        "自定义 Agent"
    };
    format!(
        "{kind} · {display} · {}",
        provider_type_label(&agent.provider_type)
    )
}

fn available_agent_reference_options() -> Vec<AgentReferenceOption> {
    std::iter::once(AgentReferenceOption {
        id: None,
        label: "跟随全局默认 Agent".to_owned(),
    })
    .chain(load_cli_agent_profiles().into_iter().map(|agent| {
        let label = format_cli_agent_label(&agent);
        AgentReferenceOption {
            id: Some(agent.agent_id),
            label,
        }
    }))
    .collect()
}

fn prompt_agent_reference_choice(
    default_id: Option<&str>,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    use dialoguer::{Select, theme::ColorfulTheme};
    let items = available_agent_reference_options();
    let labels = items
        .iter()
        .map(|item| item.label.as_str())
        .collect::<Vec<_>>();
    let default_index = default_id
        .and_then(|id| items.iter().position(|item| item.id.as_deref() == Some(id)))
        .or_else(|| items.iter().position(|item| item.id.is_none()))
        .unwrap_or(0);
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("选择 Agent")
        .default(default_index)
        .items(&labels)
        .interact()?;
    Ok(items[selection].id.clone())
}

#[allow(clippy::too_many_arguments)]
fn upsert_channel_account(
    cfg: &mut GaryxConfig,
    channel: &str,
    account: &str,
    name: Option<String>,
    workspace_dir: Option<String>,
    workspace_mode: Option<String>,
    agent_id: Option<String>,
    token: Option<String>,
    uin: Option<String>,
    base_url: Option<String>,
    app_id: Option<String>,
    app_secret: Option<String>,
    domain: Option<String>,
    plugin_extras: Map<String, Value>,
) -> Result<(), Box<dyn std::error::Error>> {
    let agent_id = trim_opt(agent_id);
    let workspace_mode = normalize_channel_workspace_mode(workspace_mode)?;
    match channel {
        BUILTIN_CHANNEL_PLUGIN_TELEGRAM => {
            let Some(token) = token else {
                return Err("missing telegram token".into());
            };
            let mut entry = telegram_account_to_plugin_entry(&TelegramAccount {
                token,
                enabled: true,
                name,
                agent_id: agent_id.clone(),
                workspace_dir,
                owner_target: None,
                groups: Default::default(),
            });
            entry.workspace_mode = workspace_mode;
            cfg.channels
                .plugin_channel_mut(BUILTIN_CHANNEL_PLUGIN_TELEGRAM)
                .accounts
                .insert(account.to_owned(), entry);
        }
        BUILTIN_CHANNEL_PLUGIN_DISCORD => {
            let Some(token) = token else {
                return Err("missing discord token".into());
            };
            let mut entry = discord_account_to_plugin_entry(&DiscordAccount {
                token,
                enabled: true,
                name,
                agent_id: agent_id.clone(),
                workspace_dir,
                owner_target: None,
                require_mention: true,
                api_base: "https://discord.com/api/v10".to_owned(),
                gateway_url: "wss://gateway.discord.gg/?v=10&encoding=json".to_owned(),
            });
            entry.workspace_mode = workspace_mode;
            cfg.channels
                .plugin_channel_mut(BUILTIN_CHANNEL_PLUGIN_DISCORD)
                .accounts
                .insert(account.to_owned(), entry);
        }
        BUILTIN_CHANNEL_PLUGIN_FEISHU => {
            let Some(app_id) = app_id else {
                return Err("missing feishu app_id".into());
            };
            let Some(app_secret) = app_secret else {
                return Err("missing feishu app_secret".into());
            };
            let resolved_domain = domain
                .as_deref()
                .and_then(parse_feishu_domain)
                .unwrap_or_default();
            let mut entry = feishu_account_to_plugin_entry(&FeishuAccount {
                app_id,
                app_secret,
                enabled: true,
                domain: resolved_domain,
                name,
                agent_id: agent_id.clone(),
                workspace_dir,
                owner_target: None,
                require_mention: true,
                topic_session_mode: Default::default(),
            });
            entry.workspace_mode = workspace_mode;
            cfg.channels
                .plugin_channel_mut(BUILTIN_CHANNEL_PLUGIN_FEISHU)
                .accounts
                .insert(account.to_owned(), entry);
        }
        BUILTIN_CHANNEL_PLUGIN_WEIXIN => {
            let Some(token) = token else {
                return Err("missing weixin token".into());
            };
            let mut entry = weixin_account_to_plugin_entry(&WeixinAccount {
                token,
                uin: uin.unwrap_or_default(),
                enabled: true,
                base_url: base_url.unwrap_or_else(|| "https://ilinkai.weixin.qq.com".to_owned()),
                name,
                agent_id: agent_id.clone(),
                workspace_dir,
                streaming_update: true,
            });
            entry.workspace_mode = workspace_mode;
            cfg.channels
                .plugin_channel_mut(BUILTIN_CHANNEL_PLUGIN_WEIXIN)
                .accounts
                .insert(account.to_owned(), entry);
        }
        "api" => {
            cfg.channels.api.accounts.insert(
                account.to_owned(),
                ApiAccount {
                    enabled: true,
                    name,
                    agent_id: agent_id.clone(),
                    workspace_dir,
                    workspace_mode,
                },
            );
        }
        _ => {
            let Some(manifest) = discover_plugin_manifest(channel)? else {
                return Err(format!("unknown channel: {channel}").into());
            };
            let overrides = ChannelOverrides {
                account: Some(account.to_owned()),
                name: name.clone(),
                workspace_dir: workspace_dir.clone(),
                workspace_mode: workspace_mode.clone(),
                agent_id: agent_id.clone(),
                token,
                uin,
                base_url,
                app_id,
                app_secret,
                domain,
                plugin_extras,
            };
            let form_state = plugin_form_state_from_overrides(&overrides);
            validate_plugin_required_fields(channel, &manifest.schema, &form_state)?;
            cfg.channels
                .plugin_channel_mut(&manifest.plugin.id)
                .accounts
                .insert(
                    account.to_owned(),
                    PluginAccountEntry {
                        enabled: true,
                        name,
                        agent_id,
                        workspace_dir,
                        workspace_mode,
                        config: Value::Object(form_state),
                    },
                );
        }
    }
    Ok(())
}

/// Prompt: "one-click authorize" (default) vs "manual input App ID / Secret".
///
/// Returns `true` for one-click device flow, `false` for manual input.
fn prompt_feishu_auth_mode() -> Result<bool, Box<dyn std::error::Error>> {
    use dialoguer::{Select, theme::ColorfulTheme};
    let items = [
        "一键授权（推荐）：扫码 / 点击链接，自动获取 App ID 和 Secret",
        "手动输入：在飞书开放平台创建应用后，粘贴 App ID 和 Secret",
    ];
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("选择绑定方式")
        .default(0)
        .items(&items)
        .interact()?;
    Ok(selection == 0)
}

fn prompt_feishu_domain_choice() -> Result<FeishuDomain, Box<dyn std::error::Error>> {
    use dialoguer::{Select, theme::ColorfulTheme};
    let items = ["飞书（国内 feishu.cn）", "Lark（海外 larksuite.com）"];
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("选择租户")
        .default(0)
        .items(&items)
        .interact()?;
    Ok(if selection == 0 {
        FeishuDomain::Feishu
    } else {
        FeishuDomain::Lark
    })
}

/// Render a URL as a terminal QR using Unicode half-block characters.
///
/// Returns `None` if the underlying encoder refuses the payload (e.g. it is
/// too long for QR version 40). Callers are expected to fall back to
/// printing the URL verbatim so the flow never becomes un-scannable.
fn render_terminal_qr(payload: &str) -> Option<String> {
    use qrcode::render::unicode::Dense1x2;
    use qrcode::{EcLevel, QrCode};

    let code = QrCode::with_error_correction_level(payload.as_bytes(), EcLevel::M).ok()?;
    let rendered = code
        .render::<Dense1x2>()
        // Give the QR a one-module quiet zone on every side so terminal
        // scanners lock on reliably.
        .quiet_zone(true)
        // Dark = foreground module (drawn on the terminal), Light = bg.
        .dark_color(Dense1x2::Dark)
        .light_color(Dense1x2::Light)
        .build();
    Some(rendered)
}

/// Walk a batch of [`AuthDisplayItem`]s and print them to stdout. Text
/// items render as-is; Qr items are encoded via [`render_terminal_qr`]
/// with a plain-text fallback so a scanner-hostile terminal never hides
/// the underlying payload from the user. Unknown items (forward-compat)
/// are silently skipped.
fn render_display_with_options(items: &[AuthDisplayItem], json_events: bool) {
    for item in items {
        match item {
            AuthDisplayItem::Text { value } => {
                if json_events {
                    println!(
                        "{}",
                        json!({
                            "event": "auth_display",
                            "kind": "text",
                            "value": value,
                        })
                    );
                    continue;
                }
                println!("{value}");
                // Convenience: if a Text item is a standalone URL on
                // its own line, try to open it in the user's default
                // browser (preserves the pre-auth-flow UX where the
                // per-channel device-auth helpers auto-opened the
                // verification URL). We keep this in the CLI's
                // render layer — not in the executor — so the
                // abstraction stays channel-blind: the executor
                // just emits Text; the terminal "UI" decides what
                // that means.
                if looks_like_standalone_url(value) {
                    let _ = open_url_in_browser(value);
                }
            }
            AuthDisplayItem::Qr { value } => {
                if json_events {
                    println!(
                        "{}",
                        json!({
                            "event": "auth_display",
                            "kind": "qr",
                            "value": value,
                        })
                    );
                    continue;
                }
                match render_terminal_qr(value) {
                    Some(art) => println!("{art}"),
                    None => {
                        eprintln!("[warn] 无法在终端渲染二维码，请使用下列原始内容：");
                        println!("{value}");
                    }
                }
            }
            AuthDisplayItem::Unknown => {}
        }
    }
}

/// True when `s` is a http(s) URL with no surrounding whitespace /
/// punctuation — i.e. a line the user could copy verbatim. Narrow
/// heuristic on purpose so hint prose like `"打开链接: https://…"`
/// doesn't get auto-opened (the channel-authored prose and the URL
/// ride on separate Text items by convention).
fn looks_like_standalone_url(s: &str) -> bool {
    let t = s.trim();
    if t.len() != s.len() {
        return false; // surrounding whitespace
    }
    (t.starts_with("http://") || t.starts_with("https://")) && !t.contains(char::is_whitespace)
}

/// True when the environment looks like a graphical session a real
/// browser could open into. Treats an empty value the same as unset —
/// that's how X11 and xdg-open's fallback chain read it.
#[cfg(any(target_os = "linux", test))]
fn gui_session_available(
    display: Option<&std::ffi::OsStr>,
    wayland: Option<&std::ffi::OsStr>,
) -> bool {
    let set = |v: Option<&std::ffi::OsStr>| v.is_some_and(|s| !s.is_empty());
    set(display) || set(wayland)
}

/// Best-effort: shell out to the platform's default-browser opener.
/// Silent on failure — the URL is already printed above so the user
/// can always copy-paste. All stdio is detached so an opener that
/// resolves to a terminal browser (xdg-open → w3m via `$BROWSER` or a
/// misconfigured desktop) can never hijack the live auth-flow TTY.
fn open_url_in_browser(url: &str) -> io::Result<()> {
    use std::process::{Command, Stdio};
    #[cfg(target_os = "macos")]
    let cmd = Command::new("open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    #[cfg(target_os = "linux")]
    let cmd = if !gui_session_available(
        std::env::var_os("DISPLAY").as_deref(),
        std::env::var_os("WAYLAND_DISPLAY").as_deref(),
    ) {
        // Headless box: xdg-open degrades to terminal browsers (w3m),
        // which take over the terminal mid-auth-flow. The URL is
        // already printed above, so skipping auto-open loses nothing.
        return Ok(());
    } else {
        Command::new("xdg-open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    };
    #[cfg(target_os = "windows")]
    // NB: do NOT shell out through `cmd /C start` — that reinterprets
    // the URL on cmd.exe's command line, giving shell metacharacters
    // like `&`/`|`/`^` (legal inside a URL) attack surface. Use
    // `rundll32 url.dll,FileProtocolHandler` which takes the URL as
    // a direct argv[2] with no shell involvement.
    let cmd = Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let cmd: io::Result<std::process::Child> = Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "no known opener for this target",
    ));
    cmd.map(|_| ())
}

/// Channel-blind driver for any [`AuthFlowExecutor`]. Invokes `start`,
/// renders the initial display, then polls at the executor's cadence
/// until the session confirms, fails, or the deadline elapses. The
/// returned map is the executor's `Confirmed { values }` patch — the
/// caller is responsible for pulling the keys it needs via
/// [`take_string`] (e.g. `app_id` / `token` / `base_url`).
async fn run_auth_flow<E: AuthFlowExecutor + ?Sized>(
    executor: &E,
    form_state: Value,
) -> Result<std::collections::BTreeMap<String, Value>, Box<dyn std::error::Error>> {
    run_auth_flow_with_options(executor, form_state, false).await
}

async fn run_auth_flow_with_options<E: AuthFlowExecutor + ?Sized>(
    executor: &E,
    form_state: Value,
    json_events: bool,
) -> Result<std::collections::BTreeMap<String, Value>, Box<dyn std::error::Error>> {
    let session = executor
        .start(form_state)
        .await
        .map_err(|e| format!("auth flow start failed: {e}"))?;
    render_display_with_options(&session.display, json_events);

    let mut interval = session.poll_interval_secs.max(1);
    // Honour the executor's exact TTL — callers MUST NOT poll past
    // `expires_in_secs` per the trait contract. Only guard against
    // a zero value (pathological executor) by treating it as "no
    // cap beyond the server's own 401s".
    let deadline = std::time::Instant::now()
        + Duration::from_secs(if session.expires_in_secs == 0 {
            u64::MAX / 2
        } else {
            session.expires_in_secs
        });

    loop {
        if std::time::Instant::now() >= deadline {
            return Err(format!("auth flow timed out after {}s", session.expires_in_secs).into());
        }
        tokio::time::sleep(Duration::from_secs(interval)).await;

        match executor
            .poll(&session.session_id)
            .await
            .map_err(|e| format!("auth flow poll failed: {e}"))?
        {
            AuthPollResult::Pending {
                display,
                next_interval_secs,
            } => {
                if let Some(d) = display {
                    render_display_with_options(&d, json_events);
                }
                if let Some(i) = next_interval_secs {
                    interval = i.max(1);
                }
            }
            AuthPollResult::Confirmed { values } => return Ok(values),
            AuthPollResult::Failed { reason } => {
                return Err(format!("auth flow failed: {reason}").into());
            }
        }
    }
}

/// Pop a required string field out of a `Confirmed { values }` map.
/// Returns a descriptive error if the key is absent or not a JSON
/// string — that only happens when the executor's documented contract
/// changes, so the message points at the contract rather than the end
/// user.
fn take_string(
    values: &mut std::collections::BTreeMap<String, Value>,
    key: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    match values.remove(key) {
        Some(Value::String(s)) => Ok(s),
        Some(other) => Err(format!("auth flow returned non-string `{key}`: {other}").into()),
        None => Err(format!("auth flow did not return `{key}`").into()),
    }
}

fn reauthorize_account_entry(
    cfg: &GaryxConfig,
    channel: &str,
    reauthorize: Option<&str>,
) -> Result<Option<PluginAccountEntry>, Box<dyn std::error::Error>> {
    let Some(account_id) = reauthorize else {
        return Ok(None);
    };
    let entry = cfg
        .channels
        .plugin_channel(channel)
        .and_then(|plugin| plugin.accounts.get(account_id))
        .cloned()
        .ok_or_else(|| format!("--reauthorize `{account_id}` was not found in {channel}"))?;
    Ok(Some(entry))
}

fn config_string(entry: &PluginAccountEntry, key: &str) -> Option<String> {
    entry
        .config
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn finish_reauthorization(
    cfg: &mut GaryxConfig,
    channel: &str,
    reauthorize: Option<&str>,
    target_account: &str,
    forget_previous: bool,
) -> Result<Option<&'static str>, Box<dyn std::error::Error>> {
    let Some(old_account) = reauthorize else {
        return Ok(None);
    };
    if old_account == target_account {
        return Ok(None);
    }
    let accounts = &mut cfg.channels.plugin_channel_mut(channel).accounts;
    if forget_previous {
        accounts.remove(old_account);
        return Ok(Some("deleted"));
    }
    let entry = accounts
        .get_mut(old_account)
        .ok_or_else(|| format!("--reauthorize `{old_account}` disappeared from {channel}"))?;
    entry.enabled = false;
    Ok(Some("disabled"))
}

fn print_login_json_summary(
    channel: &str,
    account_id: &str,
    action: &str,
    reauthorize: Option<&str>,
    previous_account_action: Option<&str>,
    config_path: &Path,
) {
    println!(
        "{}",
        json!({
            "ok": true,
            "event": "channel_login_saved",
            "channel": channel,
            "account_id": account_id,
            "action": action,
            "reauthorize": reauthorize,
            "previous_account_action": previous_account_action,
            "config_path": config_path.to_string_lossy().to_string(),
        })
    );
}

fn previous_account_action_label(action: &str) -> &str {
    match action {
        "deleted" => "删除",
        "disabled" => "禁用",
        _ => action,
    }
}

pub(crate) async fn cmd_channels_login(
    config_path: &str,
    channel: &str,
    account: Option<String>,
    reauthorize: Option<String>,
    forget_previous: bool,
    name: Option<String>,
    workspace_dir: Option<String>,
    workspace_mode: Option<String>,
    agent_id: Option<String>,
    uin: Option<String>,
    base_url: Option<String>,
    domain: Option<String>,
    timeout_seconds: u64,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let channel = canonical_channel_id(channel);
    if forget_previous && trim_opt(reauthorize.clone()).is_none() {
        return Err("--forget-previous requires --reauthorize".into());
    }
    let loaded = load_config_or_default(config_path, ConfigRuntimeOverrides::default())?;
    let config_path = loaded.path;
    let mut cfg = loaded.config;
    let reauthorize = trim_opt(reauthorize);
    let reauthorize_entry = reauthorize_account_entry(&cfg, &channel, reauthorize.as_deref())?;
    let inherited_auth_form_state = reauthorize_entry
        .as_ref()
        .and_then(|entry| entry.config.as_object().cloned())
        .unwrap_or_default();
    let selected_name =
        trim_opt(name).or_else(|| reauthorize_entry.as_ref().and_then(|e| e.name.clone()));
    let selected_workspace_dir = trim_opt(workspace_dir).or_else(|| {
        reauthorize_entry
            .as_ref()
            .and_then(|e| e.workspace_dir.clone())
    });
    let selected_workspace_mode = trim_opt(workspace_mode).or_else(|| {
        reauthorize_entry
            .as_ref()
            .and_then(|entry| entry.workspace_mode.clone())
    });
    let selected_uin = trim_opt(uin).or_else(|| {
        reauthorize_entry
            .as_ref()
            .and_then(|entry| config_string(entry, "uin"))
    });
    let mut selected_agent_id =
        trim_opt(agent_id).or_else(|| reauthorize_entry.as_ref().and_then(|e| e.agent_id.clone()));
    if selected_agent_id.is_none() && can_prompt_interactively() && !json_output {
        selected_agent_id = prompt_agent_reference_choice(None)?;
    }

    match channel.as_str() {
        BUILTIN_CHANNEL_PLUGIN_WEIXIN => {
            let executor = WeixinAuthExecutor::new(reqwest::Client::new());
            let login_base_url = trim_opt(base_url.clone()).or_else(|| {
                reauthorize_entry
                    .as_ref()
                    .and_then(|entry| config_string(entry, "base_url"))
            });
            let mut values = run_auth_flow_with_options(
                &executor,
                json!({
                    "base_url": normalize_weixin_base_url(login_base_url),
                    "timeout_secs": timeout_seconds,
                }),
                json_output,
            )
            .await?;
            let token = take_string(&mut values, "token")?;
            let login_base_url = take_string(&mut values, "base_url")?;
            let scanned_account_id = take_string(&mut values, "account_id")?;
            let target_account_id = account
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| scanned_account_id.clone());
            let existed = cfg
                .channels
                .plugin_channel(BUILTIN_CHANNEL_PLUGIN_WEIXIN)
                .and_then(|plugin| plugin.accounts.get(&target_account_id))
                .is_some();

            upsert_channel_account(
                &mut cfg,
                BUILTIN_CHANNEL_PLUGIN_WEIXIN,
                &target_account_id,
                selected_name.clone(),
                selected_workspace_dir.clone(),
                selected_workspace_mode.clone(),
                selected_agent_id.clone(),
                Some(token),
                selected_uin.clone(),
                Some(login_base_url),
                None,
                None,
                None,
                Map::new(),
            )?;
            let previous_account_action = finish_reauthorization(
                &mut cfg,
                BUILTIN_CHANNEL_PLUGIN_WEIXIN,
                reauthorize.as_deref(),
                &target_account_id,
                forget_previous,
            )?;
            save_config_struct(&config_path, &cfg)?;
            if json_output {
                notify_gateway_reload_quiet(&config_path).await;
                print_login_json_summary(
                    BUILTIN_CHANNEL_PLUGIN_WEIXIN,
                    &target_account_id,
                    if existed { "updated" } else { "added" },
                    reauthorize.as_deref(),
                    previous_account_action,
                    &config_path,
                );
            } else {
                println!("已添加 weixin 账号: {target_account_id}");
                if let Some(action) = previous_account_action {
                    println!(
                        "已{}旧 weixin 账号: {}",
                        previous_account_action_label(action),
                        reauthorize.as_deref().unwrap()
                    );
                }
                notify_gateway_reload(&config_path).await;
            }
            Ok(())
        }
        BUILTIN_CHANNEL_PLUGIN_FEISHU => {
            let login_domain = trim_opt(domain.clone()).or_else(|| {
                reauthorize_entry
                    .as_ref()
                    .and_then(|entry| config_string(entry, "domain"))
            });
            let resolved_domain = login_domain
                .as_deref()
                .and_then(parse_feishu_domain)
                .unwrap_or_default();
            let domain_str = match resolved_domain {
                FeishuDomain::Feishu => "feishu",
                FeishuDomain::Lark => "lark",
            };
            let executor = FeishuAuthExecutor::new(reqwest::Client::new());
            let mut values =
                run_auth_flow_with_options(&executor, json!({ "domain": domain_str }), json_output)
                    .await?;
            let app_id = take_string(&mut values, "app_id")?;
            let app_secret = take_string(&mut values, "app_secret")?;
            let confirmed_domain = take_string(&mut values, "domain")?;
            let target_account_id = account
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| app_id.clone());
            let existed = cfg
                .channels
                .plugin_channel(BUILTIN_CHANNEL_PLUGIN_FEISHU)
                .and_then(|plugin| plugin.accounts.get(&target_account_id))
                .is_some();

            upsert_channel_account(
                &mut cfg,
                BUILTIN_CHANNEL_PLUGIN_FEISHU,
                &target_account_id,
                selected_name.clone(),
                selected_workspace_dir.clone(),
                selected_workspace_mode.clone(),
                selected_agent_id.clone(),
                None,
                None,
                None,
                Some(app_id),
                Some(app_secret),
                Some(confirmed_domain),
                Map::new(),
            )?;
            let previous_account_action = finish_reauthorization(
                &mut cfg,
                BUILTIN_CHANNEL_PLUGIN_FEISHU,
                reauthorize.as_deref(),
                &target_account_id,
                forget_previous,
            )?;
            save_config_struct(&config_path, &cfg)?;
            if json_output {
                notify_gateway_reload_quiet(&config_path).await;
                print_login_json_summary(
                    BUILTIN_CHANNEL_PLUGIN_FEISHU,
                    &target_account_id,
                    if existed { "updated" } else { "added" },
                    reauthorize.as_deref(),
                    previous_account_action,
                    &config_path,
                );
            } else {
                println!("已添加 feishu 账号: {target_account_id}");
                if let Some(action) = previous_account_action {
                    println!(
                        "已{}旧 feishu 账号: {}",
                        previous_account_action_label(action),
                        reauthorize.as_deref().unwrap()
                    );
                }
                notify_gateway_reload(&config_path).await;
            }
            Ok(())
        }
        _ => {
            let Some(manifest) = discover_plugin_manifest(&channel)? else {
                return Err(format!(
                    "channel login is currently supported only for feishu/weixin or plugins with auth flows, got: {channel}"
                )
                .into());
            };
            if manifest.auth_flows.is_empty() {
                return Err(format!("channel `{channel}` does not advertise any auth flow").into());
            }
            let mut values = run_plugin_auth_flow_with_options(
                &manifest,
                Value::Object({
                    let mut form_state = inherited_auth_form_state.clone();
                    if let Some(value) = trim_opt(base_url) {
                        form_state.insert("base_url".to_owned(), Value::String(value));
                    }
                    if let Some(value) = trim_opt(domain) {
                        form_state.insert("domain".to_owned(), Value::String(value));
                    }
                    form_state
                }),
                json_output,
            )
            .await?;
            let target_account_id = account
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .or_else(|| plugin_suggested_account_id(&values))
                .ok_or_else(|| {
                    format!("plugin `{channel}` auth flow did not return an account id hint")
                })?;
            let existed = cfg
                .channels
                .plugin_channel(&channel)
                .and_then(|plugin| plugin.accounts.get(&target_account_id))
                .is_some();
            strip_plugin_identity_hints(&mut values, &manifest.schema);
            let mut overrides = ChannelOverrides {
                account: Some(target_account_id.clone()),
                name: selected_name.clone(),
                workspace_dir: selected_workspace_dir.clone(),
                workspace_mode: selected_workspace_mode.clone(),
                agent_id: selected_agent_id.clone(),
                plugin_extras: Map::new(),
                ..ChannelOverrides::default()
            };
            for (key, value) in values {
                set_plugin_form_value(&mut overrides, &key, value);
            }
            upsert_channel_account(
                &mut cfg,
                &channel,
                &target_account_id,
                overrides.name,
                overrides.workspace_dir,
                overrides.workspace_mode,
                overrides.agent_id,
                overrides.token,
                overrides.uin,
                overrides.base_url,
                overrides.app_id,
                overrides.app_secret,
                overrides.domain,
                overrides.plugin_extras,
            )?;
            let previous_account_action = finish_reauthorization(
                &mut cfg,
                &channel,
                reauthorize.as_deref(),
                &target_account_id,
                forget_previous,
            )?;
            save_config_struct(&config_path, &cfg)?;
            if json_output {
                notify_gateway_reload_quiet(&config_path).await;
                print_login_json_summary(
                    &channel,
                    &target_account_id,
                    if existed { "updated" } else { "added" },
                    reauthorize.as_deref(),
                    previous_account_action,
                    &config_path,
                );
            } else {
                println!("已添加 {channel} 账号: {target_account_id}");
                if let Some(action) = previous_account_action {
                    println!(
                        "已{}旧 {channel} 账号: {}",
                        previous_account_action_label(action),
                        reauthorize.as_deref().unwrap()
                    );
                }
                notify_gateway_reload(&config_path).await;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::await_holding_lock)]

    use super::*;
    use crate::commands::test_support::*;
    use std::ffi::OsStr;
    use tempfile::tempdir;

    fn write_test_plugin_bundle(root: &Path, plugin_id: &str, required_fields: &[&str]) -> PathBuf {
        let plugin_dir = root.join(plugin_id);
        std::fs::create_dir_all(&plugin_dir).expect("create plugin dir");
        let binary_name = if cfg!(windows) {
            "fake-plugin.cmd"
        } else {
            "fake-plugin.sh"
        };
        let binary_path = plugin_dir.join(binary_name);
        if cfg!(windows) {
            std::fs::write(&binary_path, "@echo off\r\nexit /b 0\r\n").expect("write fake plugin");
        } else {
            std::fs::write(&binary_path, "#!/bin/sh\nexit 0\n").expect("write fake plugin");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut permissions = std::fs::metadata(&binary_path)
                    .expect("fake plugin metadata")
                    .permissions();
                permissions.set_mode(0o755);
                std::fs::set_permissions(&binary_path, permissions).expect("chmod fake plugin");
            }
        }

        let required = serde_json::to_string(required_fields).expect("required fields json");
        let manifest = format!(
            r#"[plugin]
id = "{plugin_id}"
version = "0.1.0"
display_name = "Test {plugin_id}"

[entry]
binary = "./{binary_name}"

[capabilities]
delivery_model = "pull_explicit_ack"
outbound = true
inbound = true

[schema]
type = "object"
required = {required}

[schema.properties.token]
type = "string"

[schema.properties.base_url]
type = "string"
"#
        );
        std::fs::write(plugin_dir.join("plugin.toml"), manifest).expect("write plugin manifest");
        plugin_dir
    }

    fn write_empty_config_file(dir: &tempfile::TempDir) -> PathBuf {
        let config_path = dir.path().join("gary.json");
        std::fs::write(
            &config_path,
            serde_json::to_vec_pretty(&GaryxConfig::default()).expect("config json"),
        )
        .expect("write config");
        config_path
    }

    // Regression guard for the Weixin onboarding: `qrcode_img_content` from
    // the iLink endpoint is a short URL, not ASCII art, so we must render a
    // real QR locally. These tests pin that contract — a URL goes in, a
    // multi-line block of Unicode half-block characters comes out.
    #[test]
    fn render_terminal_qr_produces_scannable_block_art() {
        let payload = "https://liteapp.weixin.qq.com/q/7GiQu1?qrcode=abc123&bot_type=3";
        let rendered = render_terminal_qr(payload).expect("QR should encode short URL");
        // For local debugging: `GARYX_TEST_SHOW_QR=1 cargo test -p garyx \
        //   render_terminal_qr_produces_scannable_block_art -- --nocapture`
        if std::env::var_os("GARYX_TEST_SHOW_QR").is_some() {
            eprintln!("\n--- Weixin QR sample ---\n{rendered}\n({payload})\n");
        }
        // Unicode half-blocks used by qrcode::render::unicode::Dense1x2.
        assert!(rendered.contains('\u{2580}') || rendered.contains('\u{2584}'));
        // Dense1x2 packs 2 module rows per line of text, so a ~29×29 QR
        // (version 3) becomes ~17 lines plus a 1-line quiet zone either
        // side. Accept anything ≥ 10 non-trivial rows — enough to prove
        // we produced a real block, not a single-line stub.
        let non_empty_rows = rendered
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count();
        assert!(
            non_empty_rows >= 10,
            "expected at least ~10 rows, got {non_empty_rows}: \n{rendered}"
        );
    }

    #[test]
    fn render_terminal_qr_returns_none_for_unencodable_input() {
        // QR version 40 maxes out at ~2953 bytes with ECL M. Feed it more.
        let huge = "x".repeat(8_000);
        assert!(render_terminal_qr(&huge).is_none());
    }

    #[test]
    fn reauthorize_weixin_can_inherit_metadata_and_disable_previous_account() {
        let mut cfg = GaryxConfig::default();
        upsert_channel_account(
            &mut cfg,
            BUILTIN_CHANNEL_PLUGIN_WEIXIN,
            "old-wx",
            Some("Wiki".to_owned()),
            Some("/Users/test".to_owned()),
            Some("worktree".to_owned()),
            Some("wiki-curator".to_owned()),
            Some("old-token".to_owned()),
            Some("old-uin".to_owned()),
            Some("https://ilinkai.weixin.qq.com".to_owned()),
            None,
            None,
            None,
            Map::new(),
        )
        .unwrap();

        let inherited =
            reauthorize_account_entry(&cfg, BUILTIN_CHANNEL_PLUGIN_WEIXIN, Some("old-wx"))
                .unwrap()
                .expect("previous account should exist");
        assert_eq!(inherited.name.as_deref(), Some("Wiki"));
        assert_eq!(inherited.workspace_dir.as_deref(), Some("/Users/test"));
        assert_eq!(inherited.workspace_mode.as_deref(), Some("worktree"));
        assert_eq!(inherited.agent_id.as_deref(), Some("wiki-curator"));
        assert_eq!(config_string(&inherited, "uin").as_deref(), Some("old-uin"));

        upsert_channel_account(
            &mut cfg,
            BUILTIN_CHANNEL_PLUGIN_WEIXIN,
            "new-wx",
            inherited.name.clone(),
            inherited.workspace_dir.clone(),
            inherited.workspace_mode.clone(),
            inherited.agent_id.clone(),
            Some("new-token".to_owned()),
            config_string(&inherited, "uin"),
            Some("https://ilinkai.weixin.qq.com".to_owned()),
            None,
            None,
            None,
            Map::new(),
        )
        .unwrap();

        let action = finish_reauthorization(
            &mut cfg,
            BUILTIN_CHANNEL_PLUGIN_WEIXIN,
            Some("old-wx"),
            "new-wx",
            false,
        )
        .unwrap();
        assert_eq!(action, Some("disabled"));

        let accounts = &cfg
            .channels
            .plugin_channel(BUILTIN_CHANNEL_PLUGIN_WEIXIN)
            .unwrap()
            .accounts;
        assert!(!accounts["old-wx"].enabled);
        assert!(accounts["new-wx"].enabled);
        assert_eq!(accounts["new-wx"].name.as_deref(), Some("Wiki"));
        assert_eq!(
            accounts["new-wx"].workspace_dir.as_deref(),
            Some("/Users/test")
        );
        assert_eq!(accounts["new-wx"].agent_id.as_deref(), Some("wiki-curator"));
        assert_eq!(accounts["new-wx"].config["uin"], "old-uin");
        assert_eq!(accounts["new-wx"].config["token"], "new-token");
    }

    #[tokio::test]
    async fn channels_add_persists_generic_plugin_accounts() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let plugin_root = tempdir().expect("plugin root");
        let _env = ScopedEnvVar::set_path("GARYX_PLUGIN_DIR", plugin_root.path());
        write_test_plugin_bundle(plugin_root.path(), "test-acmechat-cli", &["token"]);

        let config_dir = tempdir().expect("config dir");
        let config_path = write_empty_config_file(&config_dir);

        cmd_channels_add(
            config_path.to_str().expect("config path"),
            Some("test-acmechat-cli".to_owned()),
            Some("agent-1".to_owned()),
            Some("AcmeChat Main".to_owned()),
            None,
            Some("worktree".to_owned()),
            None,
            Some("tok-1".to_owned()),
            None,
            Some("https://chat.example.com".to_owned()),
            None,
            None,
            None,
            false,
        )
        .await
        .expect("plugin add should succeed");

        let loaded = load_config_or_default(
            config_path.to_str().expect("config path"),
            ConfigRuntimeOverrides::default(),
        )
        .expect("load config");
        let entry = loaded
            .config
            .channels
            .plugins
            .get("test-acmechat-cli")
            .and_then(|plugin| plugin.accounts.get("agent-1"))
            .expect("plugin account should exist");
        assert_eq!(entry.name.as_deref(), Some("AcmeChat Main"));
        assert_eq!(entry.agent_id, None);
        assert_eq!(entry.workspace_mode.as_deref(), Some("worktree"));
        assert_eq!(entry.config["token"], "tok-1");
        assert_eq!(entry.config["base_url"], "https://chat.example.com");
    }

    #[test]
    fn upsert_all_channel_account_shapes_preserve_follow_global_none() {
        let mut cfg = GaryxConfig::default();
        let cases = [
            (
                BUILTIN_CHANNEL_PLUGIN_TELEGRAM,
                Some("telegram-token".to_owned()),
                None,
                None,
                None,
                None,
            ),
            (
                BUILTIN_CHANNEL_PLUGIN_DISCORD,
                Some("discord-token".to_owned()),
                None,
                None,
                None,
                None,
            ),
            (
                BUILTIN_CHANNEL_PLUGIN_FEISHU,
                None,
                None,
                None,
                Some("app-id".to_owned()),
                Some("app-secret".to_owned()),
            ),
            (
                BUILTIN_CHANNEL_PLUGIN_WEIXIN,
                Some("weixin-token".to_owned()),
                Some("1000000001".to_owned()),
                Some("https://ilinkai.weixin.qq.com".to_owned()),
                None,
                None,
            ),
        ];
        for (channel, token, uin, base_url, app_id, app_secret) in cases {
            upsert_channel_account(
                &mut cfg,
                channel,
                "main",
                None,
                None,
                None,
                None,
                token,
                uin,
                base_url,
                app_id,
                app_secret,
                None,
                Map::new(),
            )
            .unwrap();
            assert_eq!(
                cfg.channels.plugin_channel(channel).unwrap().accounts["main"].agent_id,
                None,
                "{channel} must preserve follow-global state"
            );
        }

        upsert_channel_account(
            &mut cfg,
            "api",
            "main",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Map::new(),
        )
        .unwrap();
        assert_eq!(cfg.channels.api.accounts["main"].agent_id, None);
    }

    #[test]
    fn cli_picker_decodes_v2_envelope_and_filters_disabled_agents() {
        let mut agents = builtin_provider_agent_profiles();
        agents
            .iter_mut()
            .find(|agent| agent.agent_id == "claude")
            .unwrap()
            .enabled = false;
        let mut custom = agents[1].clone();
        custom.agent_id = "reviewer".to_owned();
        custom.display_name = "Reviewer".to_owned();
        custom.built_in = false;
        agents.push(custom);
        let encoded =
            garyx_models::serialize_agent_store_document(&agents, Some("reviewer")).unwrap();

        let parsed = parse_cli_agent_profiles(&encoded).unwrap();
        assert!(!parsed.iter().any(|agent| agent.agent_id == "claude"));
        assert!(parsed.iter().any(|agent| agent.agent_id == "reviewer"));
        assert!(parsed.iter().any(|agent| agent.agent_id == "codex"));
    }

    #[test]
    fn upsert_plugin_account_rejects_missing_required_fields() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let plugin_root = tempdir().expect("plugin root");
        let _env = ScopedEnvVar::set_path("GARYX_PLUGIN_DIR", plugin_root.path());
        write_test_plugin_bundle(plugin_root.path(), "test-acmechat-cli", &["token"]);

        let mut cfg = GaryxConfig::default();
        let err = upsert_channel_account(
            &mut cfg,
            "test-acmechat-cli",
            "agent-1",
            None,
            None,
            None,
            None,
            None,
            None,
            Some("https://chat.example.com".to_owned()),
            None,
            None,
            None,
            Map::new(),
        )
        .expect_err("missing token should fail");
        assert!(
            err.to_string().contains("missing required fields"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn upsert_channel_account_rejects_direct_workspace_mode() {
        let mut cfg = GaryxConfig::default();
        let err = upsert_channel_account(
            &mut cfg,
            "api",
            "scripted",
            None,
            None,
            Some("direct".to_owned()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Map::new(),
        )
        .expect_err("direct should not be accepted as a workspace mode");

        assert!(
            err.to_string().contains("use `local` or `worktree`"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn gui_session_available_false_when_both_unset() {
        assert!(!gui_session_available(None, None));
    }

    #[test]
    fn gui_session_available_false_when_both_empty() {
        // X11 convention: an empty DISPLAY behaves like unset, and
        // xdg-open's fallback chain treats it the same way.
        assert!(!gui_session_available(
            Some(OsStr::new("")),
            Some(OsStr::new(""))
        ));
    }

    #[test]
    fn gui_session_available_true_with_x11_display() {
        assert!(gui_session_available(Some(OsStr::new(":0")), None));
    }

    #[test]
    fn gui_session_available_true_with_wayland_only() {
        assert!(gui_session_available(None, Some(OsStr::new("wayland-0"))));
    }
}
