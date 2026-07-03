use super::*;

pub(super) const CLAUDE_MODELS_BASE_URL: &str = "https://api.anthropic.com";

pub(super) const CLAUDE_MODELS_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) async fn fetch_claude_code_models() -> Result<ProviderModelDiscovery, String> {
    #[cfg(test)]
    if std::env::var_os("GARYX_ALLOW_REAL_CLAUDE_MODEL_FETCH").is_none() {
        return Err("Claude model catalog fetch disabled in tests".to_owned());
    }

    let token = crate::claude_oauth::read_oauth_token().await?;
    fetch_claude_code_models_from_endpoint(CLAUDE_MODELS_BASE_URL, &token, CLAUDE_MODELS_TIMEOUT)
        .await
}

pub(super) async fn fetch_claude_code_models_from_endpoint(
    base_url: &str,
    token: &str,
    request_timeout: Duration,
) -> Result<ProviderModelDiscovery, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("Claude OAuth token was empty".to_owned());
    }
    let client = reqwest::Client::builder()
        .timeout(request_timeout)
        .build()
        .map_err(|error| format!("failed to build Claude model catalog HTTP client: {error}"))?;
    let endpoint = format!("{}/v1/models", base_url.trim_end_matches('/'));
    let response = timeout(
        request_timeout,
        client
            .get(endpoint)
            .bearer_auth(token)
            .header(
                "anthropic-version",
                crate::claude_oauth::CLAUDE_ANTHROPIC_VERSION,
            )
            .header("anthropic-beta", crate::claude_oauth::CLAUDE_OAUTH_BETA)
            .header(
                reqwest::header::USER_AGENT,
                crate::claude_oauth::CLAUDE_USER_AGENT,
            )
            .header(reqwest::header::ACCEPT, "application/json")
            .send(),
    )
    .await
    .map_err(|_| "Claude model catalog request timed out".to_owned())?
    .map_err(|error| {
        if error.is_timeout() {
            "Claude model catalog request timed out".to_owned()
        } else {
            format!("Claude model catalog request failed: {error}")
        }
    })?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Claude model catalog request returned HTTP {status}"
        ));
    }
    let value = response
        .json::<Value>()
        .await
        .map_err(|error| format!("Claude model catalog response was invalid: {error}"))?;
    Ok(parse_claude_code_models_response(&value))
}

#[derive(Debug)]
pub(super) struct ClaudeApiModelOption {
    index: usize,
    created_at: Option<String>,
    model: ProviderModelOption,
}

pub(super) fn parse_claude_code_models_response(value: &Value) -> ProviderModelDiscovery {
    let entries = value
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut seen = HashSet::new();
    let mut models = Vec::new();
    for (index, entry) in entries.into_iter().enumerate() {
        let Some(id) = entry
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if !seen.insert(id.to_owned()) {
            continue;
        }
        let label = entry
            .get("display_name")
            .or_else(|| entry.get("displayName"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| friendly_model_label(id));
        let description = entry
            .get("description")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let created_at = entry
            .get("created_at")
            .or_else(|| entry.get("createdAt"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        models.push(ClaudeApiModelOption {
            index,
            created_at,
            model: ProviderModelOption {
                id: id.to_owned(),
                label,
                description,
                recommended: false,
                default_reasoning_effort: None,
                supported_reasoning_efforts: parse_claude_code_model_reasoning_efforts(&entry),
                service_tiers: Vec::new(),
            },
        });
    }
    models.sort_by(compare_claude_api_models);
    let models = models
        .into_iter()
        .map(|entry| entry.model)
        .collect::<Vec<_>>();
    let reasoning_efforts = common_reasoning_efforts(&models);
    ProviderModelDiscovery {
        models,
        default_model: None,
        reasoning_efforts,
        service_tiers: Vec::new(),
        source: "claude_code_api",
        error: None,
    }
}

pub(super) fn compare_claude_api_models(
    left: &ClaudeApiModelOption,
    right: &ClaudeApiModelOption,
) -> Ordering {
    match (&left.created_at, &right.created_at) {
        (Some(left_created), Some(right_created)) => right_created
            .cmp(left_created)
            .then_with(|| left.model.id.cmp(&right.model.id)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => left
            .index
            .cmp(&right.index)
            .then_with(|| left.model.id.cmp(&right.model.id)),
    }
}

pub(super) fn parse_claude_code_model_reasoning_efforts(
    entry: &Value,
) -> Vec<ProviderReasoningEffortOption> {
    let Some(effort) = entry
        .get("capabilities")
        .and_then(|capabilities| capabilities.get("effort"))
    else {
        return Vec::new();
    };
    if effort.get("supported").and_then(Value::as_bool) != Some(true) {
        return Vec::new();
    }
    ["low", "medium", "high", "xhigh", "max"]
        .iter()
        .filter_map(|id| {
            let supported = effort
                .get(*id)
                .and_then(|value| value.get("supported"))
                .and_then(Value::as_bool)
                == Some(true);
            if !supported {
                return None;
            }
            native_reasoning_effort_metadata(id).map(|(id, label, description)| {
                ProviderReasoningEffortOption {
                    id: id.to_owned(),
                    label: label.to_owned(),
                    description: Some(description.to_owned()),
                    recommended: false,
                }
            })
        })
        .collect()
}

pub(super) fn friendly_model_label(id: &str) -> String {
    let without_prefix = id.strip_prefix("models/").unwrap_or(id);
    let mut parts = without_prefix
        .split(['-', '_'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts
        .last()
        .is_some_and(|part| part.len() >= 6 && part.chars().all(|ch| ch.is_ascii_digit()))
    {
        parts.pop();
    }
    let label = parts
        .into_iter()
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    format!(
                        "{}{}",
                        first.to_ascii_uppercase(),
                        chars.as_str().to_ascii_lowercase()
                    )
                }
                None => String::new(),
            }
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if label.is_empty() {
        id.to_owned()
    } else {
        label
    }
}

pub(super) fn claude_code_builtin_models(error: Option<String>) -> ProviderModelDiscovery {
    let models = claude_code_models();
    let reasoning_efforts = common_reasoning_efforts(&models);
    ProviderModelDiscovery {
        models,
        default_model: None,
        reasoning_efforts,
        service_tiers: Vec::new(),
        source: "claude_code_builtin",
        error,
    }
}
