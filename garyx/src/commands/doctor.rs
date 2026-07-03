use super::*;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(super) struct DoctorIssue {
    code: String,
    message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<String>,
}

impl DoctorIssue {
    fn new(code: impl Into<String>, message: impl Into<String>, path: Option<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            path,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct DoctorCheck {
    ok: bool,
    detail: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    issues: Vec<DoctorIssue>,
}

impl DoctorCheck {
    fn new(ok: bool, detail: impl Into<String>) -> Self {
        Self {
            ok,
            detail: detail.into(),
            issues: Vec::new(),
        }
    }

    fn with_issues(ok: bool, detail: impl Into<String>, issues: Vec<DoctorIssue>) -> Self {
        Self {
            ok,
            detail: detail.into(),
            issues,
        }
    }
}

fn json_path_key(key: &str) -> String {
    if !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        format!(".{key}")
    } else {
        format!(
            "[{}]",
            serde_json::to_string(key).unwrap_or_else(|_| "\"<invalid>\"".to_owned())
        )
    }
}

fn channel_account_config_path(channel: &str, account: &str) -> String {
    format!(
        "$.channels{}.accounts{}.config",
        json_path_key(channel),
        json_path_key(account)
    )
}

fn push_channel_account_issue(
    issues: &mut Vec<DoctorIssue>,
    code: &'static str,
    channel: &str,
    account: &str,
    message: impl Into<String>,
) {
    issues.push(DoctorIssue::new(
        code,
        message,
        Some(channel_account_config_path(channel, account)),
    ));
}

fn validate_builtin_channel_account(
    channel: &str,
    account_id: &str,
    entry: &PluginAccountEntry,
    issues: &mut Vec<DoctorIssue>,
) {
    match channel {
        BUILTIN_CHANNEL_PLUGIN_TELEGRAM => match telegram_account_from_plugin_entry(entry) {
            Ok(account) => {
                if account.token.trim().is_empty() {
                    push_channel_account_issue(
                        issues,
                        "CONFIG_CHANNEL_ACCOUNT_REQUIRED",
                        channel,
                        account_id,
                        format!("channel `{channel}` account `{account_id}` is missing token"),
                    );
                }
            }
            Err(err) => push_channel_account_issue(
                issues,
                "CONFIG_CHANNEL_ACCOUNT_INVALID",
                channel,
                account_id,
                format!("channel `{channel}` account `{account_id}` config is invalid: {err}"),
            ),
        },
        BUILTIN_CHANNEL_PLUGIN_DISCORD => match discord_account_from_plugin_entry(entry) {
            Ok(account) => {
                if account.token.trim().is_empty() {
                    push_channel_account_issue(
                        issues,
                        "CONFIG_CHANNEL_ACCOUNT_REQUIRED",
                        channel,
                        account_id,
                        format!("channel `{channel}` account `{account_id}` is missing token"),
                    );
                }
            }
            Err(err) => push_channel_account_issue(
                issues,
                "CONFIG_CHANNEL_ACCOUNT_INVALID",
                channel,
                account_id,
                format!("channel `{channel}` account `{account_id}` config is invalid: {err}"),
            ),
        },
        BUILTIN_CHANNEL_PLUGIN_FEISHU => match feishu_account_from_plugin_entry(entry) {
            Ok(account) => {
                let mut missing = Vec::new();
                if account.app_id.trim().is_empty() {
                    missing.push("app_id");
                }
                if account.app_secret.trim().is_empty() {
                    missing.push("app_secret");
                }
                if !missing.is_empty() {
                    push_channel_account_issue(
                        issues,
                        "CONFIG_CHANNEL_ACCOUNT_REQUIRED",
                        channel,
                        account_id,
                        format!(
                            "channel `{channel}` account `{account_id}` is missing {}",
                            missing.join(", ")
                        ),
                    );
                }
            }
            Err(err) => push_channel_account_issue(
                issues,
                "CONFIG_CHANNEL_ACCOUNT_INVALID",
                channel,
                account_id,
                format!("channel `{channel}` account `{account_id}` config is invalid: {err}"),
            ),
        },
        BUILTIN_CHANNEL_PLUGIN_WEIXIN => match weixin_account_from_plugin_entry(entry) {
            Ok(account) => {
                if account.token.trim().is_empty() {
                    push_channel_account_issue(
                        issues,
                        "CONFIG_CHANNEL_ACCOUNT_REQUIRED",
                        channel,
                        account_id,
                        format!("channel `{channel}` account `{account_id}` is missing token"),
                    );
                }
            }
            Err(err) => push_channel_account_issue(
                issues,
                "CONFIG_CHANNEL_ACCOUNT_INVALID",
                channel,
                account_id,
                format!("channel `{channel}` account `{account_id}` config is invalid: {err}"),
            ),
        },
        _ => {}
    }
}

fn validate_plugin_schema_account(
    channel: &str,
    account_id: &str,
    entry: &PluginAccountEntry,
    schema: &Value,
    issues: &mut Vec<DoctorIssue>,
) {
    let Some(form_state) = entry.config.as_object() else {
        push_channel_account_issue(
            issues,
            "CONFIG_CHANNEL_ACCOUNT_CONFIG_TYPE",
            channel,
            account_id,
            format!("channel `{channel}` account `{account_id}` config must be an object"),
        );
        return;
    };

    let missing: Vec<String> = plugin_required_fields(schema)
        .into_iter()
        .filter(|key| value_is_missing(form_state.get(key)))
        .collect();
    if !missing.is_empty() {
        push_channel_account_issue(
            issues,
            "CONFIG_CHANNEL_ACCOUNT_REQUIRED",
            channel,
            account_id,
            format!(
                "channel `{channel}` account `{account_id}` is missing {}",
                missing.join(", ")
            ),
        );
    }
}

pub(super) fn validate_channel_account_configs(
    cfg: &GaryxConfig,
    plugin_schemas: &HashMap<String, Value>,
) -> Vec<DoctorIssue> {
    let mut issues = Vec::new();
    for (channel_id, plugin_cfg) in &cfg.channels.plugins {
        for (account_id, entry) in &plugin_cfg.accounts {
            if entry.config.is_null() {
                push_channel_account_issue(
                    &mut issues,
                    "CONFIG_CHANNEL_ACCOUNT_CONFIG_NULL",
                    channel_id,
                    account_id,
                    format!(
                        "channel `{channel_id}` account `{account_id}` has null config; re-run channel setup for this account or remove the stale account"
                    ),
                );
                continue;
            }

            validate_builtin_channel_account(channel_id, account_id, entry, &mut issues);

            if !matches!(
                channel_id.as_str(),
                BUILTIN_CHANNEL_PLUGIN_TELEGRAM
                    | BUILTIN_CHANNEL_PLUGIN_DISCORD
                    | BUILTIN_CHANNEL_PLUGIN_FEISHU
                    | BUILTIN_CHANNEL_PLUGIN_WEIXIN
            ) && let Some(schema) = plugin_schemas.get(channel_id)
            {
                validate_plugin_schema_account(channel_id, account_id, entry, schema, &mut issues);
            }
        }
    }
    issues
}

pub(super) fn discover_installed_plugin_schemas()
-> Result<(HashMap<String, Value>, Vec<DoctorIssue>), String> {
    let outcome = ManifestDiscoverer::new(crate::channel_plugin_host::plugin_root_paths(
        &GaryxConfig::default(),
    ))
    .discover()
    .map_err(|err| err.to_string())?;

    let schemas = outcome
        .plugins
        .into_iter()
        .map(|manifest| (manifest.plugin.id, manifest.schema))
        .collect();
    let issues = outcome
        .errors
        .into_iter()
        .map(|err| DoctorIssue::new("PLUGIN_MANIFEST_INVALID", err.to_string(), None))
        .collect();
    Ok((schemas, issues))
}

fn print_doctor_issues(issues: &[DoctorIssue]) {
    for issue in issues {
        if let Some(path) = &issue.path {
            println!("  - [{}] {} ({path})", issue.code, issue.message);
        } else {
            println!("  - [{}] {}", issue.code, issue.message);
        }
    }
}

pub(super) fn print_config_validation_issues(issues: &[DoctorIssue]) {
    for issue in issues {
        if let Some(path) = &issue.path {
            eprintln!("[error][{}] {} ({path})", issue.code, issue.message);
        } else {
            eprintln!("[error][{}] {}", issue.code, issue.message);
        }
    }
}

pub(crate) async fn cmd_doctor(
    config_path: &str,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let prepared = prepare_config_path_for_io_buf(config_path);
    print_diagnostics(&prepared.diagnostics);
    let config_path = prepared.active_path;
    let config_exists = config_path.exists();
    let config_path_display = config_path.to_string_lossy().to_string();
    let claude_available = which("claude");
    let codex_available = which("codex");
    let loaded = load_config_or_default(&config_path_display, ConfigRuntimeOverrides::default());
    let config_load_check = match &loaded {
        Ok(loaded) => {
            print_diagnostics(&loaded.diagnostics);
            DoctorCheck::new(true, "config loads successfully")
        }
        Err(err) => {
            print_errors(&err.diagnostics);
            DoctorCheck::new(false, err.to_string())
        }
    };

    let (plugin_schemas, plugin_manifest_check) = match discover_installed_plugin_schemas() {
        Ok((schemas, issues)) => {
            let detail = if issues.is_empty() {
                format!("{} installed plugin manifest(s) loaded", schemas.len())
            } else {
                format!("{} plugin manifest issue(s)", issues.len())
            };
            (
                schemas,
                DoctorCheck::with_issues(issues.is_empty(), detail, issues),
            )
        }
        Err(err) => (
            HashMap::new(),
            DoctorCheck::with_issues(
                false,
                "failed to discover installed plugin manifests",
                vec![DoctorIssue::new("PLUGIN_MANIFEST_DISCOVERY", err, None)],
            ),
        ),
    };

    let account_issues = loaded
        .as_ref()
        .map(|loaded| validate_channel_account_configs(&loaded.config, &plugin_schemas))
        .unwrap_or_default();
    let config_accounts_check = if loaded.is_ok() {
        let detail = if account_issues.is_empty() {
            "channel account configs look valid".to_owned()
        } else {
            format!("{} invalid channel account config(s)", account_issues.len())
        };
        DoctorCheck::with_issues(account_issues.is_empty(), detail, account_issues)
    } else {
        DoctorCheck::new(false, "skipped because config did not load")
    };

    let checks = vec![
        (
            "config_file",
            DoctorCheck::new(config_exists, config_path_display.clone()),
        ),
        ("config_load", config_load_check),
        ("config_accounts", config_accounts_check),
        ("plugin_manifests", plugin_manifest_check),
        (
            "claude_binary",
            DoctorCheck::new(claude_available, "claude"),
        ),
        ("codex_binary", DoctorCheck::new(codex_available, "codex")),
    ];

    if json {
        let obj: serde_json::Value = checks
            .iter()
            .map(|(name, check)| (name.to_string(), serde_json::to_value(check).unwrap()))
            .collect::<serde_json::Map<String, serde_json::Value>>()
            .into();
        println!("{}", serde_json::to_string_pretty(&obj)?);
    } else {
        for (name, check) in &checks {
            let mark = if check.ok {
                "ok"
            } else if *name == "config_file" || name.ends_with("_binary") {
                "MISSING"
            } else {
                "FAILED"
            };
            println!("[{}] {} ({})", mark, name, check.detail);
            print_doctor_issues(&check.issues);
        }
    }
    Ok(())
}

/// Check whether a binary exists on PATH.
pub(crate) fn which(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let full = dir.join(name);
                full.is_file() || full.with_extension("exe").is_file()
            })
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_channel_account_configs_flags_null_plugin_config() {
        let mut cfg = GaryxConfig::default();
        cfg.channels
            .plugin_channel_mut("test-plugin")
            .accounts
            .insert(
                "test-account".to_owned(),
                PluginAccountEntry {
                    enabled: true,
                    name: Some("Test Account".to_owned()),
                    agent_id: Some("claude".to_owned()),
                    workspace_dir: None,
                    workspace_mode: None,
                    config: Value::Null,
                },
            );

        let issues = validate_channel_account_configs(&cfg, &std::collections::HashMap::new());

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, "CONFIG_CHANNEL_ACCOUNT_CONFIG_NULL");
        assert_eq!(
            issues[0].path.as_deref(),
            Some("$.channels.test-plugin.accounts.test-account.config")
        );
    }

    #[test]
    fn validate_channel_account_configs_decodes_builtin_accounts() {
        let mut cfg = GaryxConfig::default();
        cfg.channels
            .plugin_channel_mut(BUILTIN_CHANNEL_PLUGIN_FEISHU)
            .accounts
            .insert(
                "work".to_owned(),
                PluginAccountEntry {
                    enabled: true,
                    name: None,
                    agent_id: Some("claude".to_owned()),
                    workspace_dir: None,
                    workspace_mode: None,
                    config: json!({}),
                },
            );

        let issues = validate_channel_account_configs(&cfg, &std::collections::HashMap::new());

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, "CONFIG_CHANNEL_ACCOUNT_INVALID");
        assert!(issues[0].message.contains("app_id"));
    }

    #[test]
    fn validate_channel_account_configs_uses_installed_plugin_required_fields() {
        let mut cfg = GaryxConfig::default();
        cfg.channels
            .plugin_channel_mut("test-acmechat-cli")
            .accounts
            .insert(
                "agent-1".to_owned(),
                PluginAccountEntry {
                    enabled: true,
                    name: None,
                    agent_id: Some("claude".to_owned()),
                    workspace_dir: None,
                    workspace_mode: None,
                    config: json!({
                        "base_url": "https://chat.example.invalid"
                    }),
                },
            );
        let schemas = std::collections::HashMap::from([(
            "test-acmechat-cli".to_owned(),
            json!({
                "type": "object",
                "required": ["token", "base_url"],
                "properties": {
                    "token": { "type": "string" },
                    "base_url": { "type": "string" }
                }
            }),
        )]);

        let issues = validate_channel_account_configs(&cfg, &schemas);

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, "CONFIG_CHANNEL_ACCOUNT_REQUIRED");
        assert!(issues[0].message.contains("token"));
    }
}
