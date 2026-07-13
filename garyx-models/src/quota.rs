use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ProviderType;

const EXHAUSTED_EPSILON: f64 = f64::EPSILON;

/// Consumer-side mirror of `GET /api/usage/coding`.
///
/// The gateway remains the producer and source of truth. These types live in
/// `garyx-models` so quota decisions can be deterministic, headless, and free
/// of gateway or provider-runtime dependencies.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CodingUsageSnapshot {
    pub providers: Vec<CodingProviderUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refreshed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CodingProviderUsage {
    pub id: String,
    pub available: bool,
    /// The gateway intentionally omits `false` on the wire.
    #[serde(default)]
    pub stale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekly: Option<CodingUsageWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<CodingUsageWindow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<CodingModelUsage>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CodingUsageWindow {
    /// Numeric fields must stay optional: a missing wire value is unknown,
    /// never zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_after_seconds: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CodingModelUsage {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_fraction: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_after_seconds: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaCredentialScope {
    DefaultLocal,
    Customized,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotaScope {
    Window { name: String },
    Model { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotaStatus {
    Ok,
    Exhausted {
        provider: ProviderType,
        scope: QuotaScope,
        reset_at: Option<String>,
    },
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum QuotaCheckError {
    #[error("quota query timed out")]
    TimedOut,
    #[error("quota data source unavailable: {0}")]
    SourceUnavailable(String),
    #[error("quota data is indeterminate: {0}")]
    Indeterminate(String),
    #[error("quota credentials do not match the target agent")]
    CredentialScopeMismatch,
    #[error("quota data has no exact bucket for model '{0}'")]
    ModelNotFound(String),
}

/// Evaluate one provider against a normalized coding-usage snapshot.
///
/// Provider support is derived from the snapshot. Entries join through
/// [`ProviderType::from_slug`], so the usage id `codex` correctly matches the
/// canonical provider `codex_app_server` without a provider-specific table.
pub fn evaluate_quota(
    snapshot: &CodingUsageSnapshot,
    provider: &ProviderType,
    model: Option<&str>,
    credential_scope: QuotaCredentialScope,
    now: DateTime<Utc>,
) -> Result<QuotaStatus, QuotaCheckError> {
    if credential_scope == QuotaCredentialScope::Customized {
        return Err(QuotaCheckError::CredentialScopeMismatch);
    }

    let mut matches = snapshot
        .providers
        .iter()
        .filter(|entry| ProviderType::from_slug(&entry.id).as_ref() == Some(provider));
    let Some(entry) = matches.next() else {
        return Ok(QuotaStatus::Unsupported);
    };
    if matches.next().is_some() {
        return Err(QuotaCheckError::Indeterminate(format!(
            "multiple usage entries normalize to provider '{}'",
            provider.as_slug()
        )));
    }
    if !entry.available {
        return Err(QuotaCheckError::SourceUnavailable(format!(
            "provider '{}' has no available reading",
            provider.as_slug()
        )));
    }
    if entry.stale {
        return Err(QuotaCheckError::Indeterminate(format!(
            "provider '{}' reading is stale",
            provider.as_slug()
        )));
    }

    let has_windows = entry.weekly.is_some() || entry.session.is_some();
    let has_models = !entry.models.is_empty();
    match (has_windows, has_models) {
        (true, false) => evaluate_windows(entry, provider, now),
        (false, true) => evaluate_model_bucket(entry, provider, model, now),
        (true, true) => Err(QuotaCheckError::Indeterminate(
            "usage entry mixes window and model-bucket shapes".to_owned(),
        )),
        (false, false) => Err(QuotaCheckError::Indeterminate(
            "usage entry has no windows or model buckets".to_owned(),
        )),
    }
}

fn evaluate_windows(
    entry: &CodingProviderUsage,
    provider: &ProviderType,
    now: DateTime<Utc>,
) -> Result<QuotaStatus, QuotaCheckError> {
    let windows = [
        ("weekly", entry.weekly.as_ref()),
        ("session", entry.session.as_ref()),
    ];
    let mut first_current_zero = None;
    let mut saw_positive = false;
    let mut indeterminate_reason = None;

    for (name, window) in windows {
        let Some(window) = window else {
            continue;
        };
        match classify_remaining(
            window.remaining_percent,
            window.resets_at.as_deref(),
            now,
            100.0,
        ) {
            Ok(Remaining::CurrentZero { reset_at }) => {
                first_current_zero.get_or_insert((name, reset_at));
            }
            Ok(Remaining::Positive) => saw_positive = true,
            Err(reason) => {
                indeterminate_reason.get_or_insert(format!("{name} window {reason}"));
            }
        }
    }

    if let Some((name, reset_at)) = first_current_zero {
        return Ok(QuotaStatus::Exhausted {
            provider: provider.clone(),
            scope: QuotaScope::Window {
                name: name.to_owned(),
            },
            reset_at,
        });
    }
    if let Some(reason) = indeterminate_reason {
        return Err(QuotaCheckError::Indeterminate(reason));
    }
    if saw_positive {
        return Ok(QuotaStatus::Ok);
    }
    Err(QuotaCheckError::Indeterminate(
        "usage entry has no comparable windows".to_owned(),
    ))
}

fn evaluate_model_bucket(
    entry: &CodingProviderUsage,
    provider: &ProviderType,
    model: Option<&str>,
    now: DateTime<Utc>,
) -> Result<QuotaStatus, QuotaCheckError> {
    let model = model.map(str::trim).filter(|value| !value.is_empty());
    let Some(model) = model else {
        return Err(QuotaCheckError::ModelNotFound(String::new()));
    };
    let mut matches = entry
        .models
        .iter()
        .filter(|bucket| bucket.id.trim() == model || bucket.name.trim() == model);
    let Some(bucket) = matches.next() else {
        return Err(QuotaCheckError::ModelNotFound(model.to_owned()));
    };
    if matches.next().is_some() {
        return Err(QuotaCheckError::Indeterminate(format!(
            "multiple quota buckets exactly match model '{model}'"
        )));
    }

    match classify_remaining(
        bucket.remaining_percent,
        bucket.resets_at.as_deref(),
        now,
        100.0,
    ) {
        Ok(Remaining::CurrentZero { reset_at }) => Ok(QuotaStatus::Exhausted {
            provider: provider.clone(),
            scope: QuotaScope::Model {
                name: bucket.name.trim().to_owned(),
            },
            reset_at,
        }),
        Ok(Remaining::Positive) => Ok(QuotaStatus::Ok),
        Err(reason) => Err(QuotaCheckError::Indeterminate(format!(
            "model '{model}' bucket {reason}"
        ))),
    }
}

enum Remaining {
    CurrentZero { reset_at: Option<String> },
    Positive,
}

fn classify_remaining(
    remaining: Option<f64>,
    reset_at: Option<&str>,
    now: DateTime<Utc>,
    maximum: f64,
) -> Result<Remaining, String> {
    let Some(remaining) = remaining else {
        return Err("has no remaining value".to_owned());
    };
    if !remaining.is_finite() || !(0.0..=maximum).contains(&remaining) {
        return Err("has an invalid remaining value".to_owned());
    }
    if remaining > EXHAUSTED_EPSILON {
        return Ok(Remaining::Positive);
    }

    let reset_at = match reset_at {
        Some(value) => {
            let parsed = DateTime::parse_from_rfc3339(value)
                .map_err(|_| "has an invalid reset timestamp".to_owned())?
                .with_timezone(&Utc);
            if parsed <= now {
                return Err("has a zero reading whose reset has passed".to_owned());
            }
            Some(value.to_owned())
        }
        None => None,
    };
    Ok(Remaining::CurrentZero { reset_at })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2030, 1, 1, 12, 0, 0)
            .single()
            .expect("fixed time")
    }

    fn window(remaining: Option<f64>, resets_at: Option<&str>) -> CodingUsageWindow {
        CodingUsageWindow {
            used_percent: remaining.map(|value| 100.0 - value),
            remaining_percent: remaining,
            resets_at: resets_at.map(ToOwned::to_owned),
            reset_after_seconds: None,
        }
    }

    fn window_provider(id: &str, remaining: Option<f64>) -> CodingProviderUsage {
        CodingProviderUsage {
            id: id.to_owned(),
            available: true,
            stale: false,
            weekly: None,
            session: Some(window(remaining, Some("2030-01-02T12:00:00Z"))),
            models: Vec::new(),
        }
    }

    fn bucket(id: &str, name: &str, remaining: Option<f64>) -> CodingModelUsage {
        CodingModelUsage {
            id: id.to_owned(),
            name: name.to_owned(),
            remaining_fraction: remaining.map(|value| value / 100.0),
            remaining_percent: remaining,
            used_percent: remaining.map(|value| 100.0 - value),
            resets_at: Some("2030-01-02T12:00:00Z".to_owned()),
            reset_after_seconds: None,
        }
    }

    fn snapshot(provider: CodingProviderUsage) -> CodingUsageSnapshot {
        CodingUsageSnapshot {
            providers: vec![provider],
            refreshed_at: Some("2030-01-01T12:00:00Z".to_owned()),
        }
    }

    #[test]
    fn codex_usage_id_normalizes_and_explicit_zero_is_exhausted() {
        let status = evaluate_quota(
            &snapshot(window_provider("codex", Some(0.0))),
            &ProviderType::CodexAppServer,
            None,
            QuotaCredentialScope::DefaultLocal,
            now(),
        )
        .expect("quota status");

        assert_eq!(
            status,
            QuotaStatus::Exhausted {
                provider: ProviderType::CodexAppServer,
                scope: QuotaScope::Window {
                    name: "session".to_owned(),
                },
                reset_at: Some("2030-01-02T12:00:00Z".to_owned()),
            }
        );
    }

    #[test]
    fn positive_and_near_zero_positive_values_are_ok() {
        for remaining in [42.0, 0.000_001] {
            let status = evaluate_quota(
                &snapshot(window_provider("claude_code", Some(remaining))),
                &ProviderType::ClaudeCode,
                None,
                QuotaCredentialScope::DefaultLocal,
                now(),
            )
            .expect("quota status");
            assert_eq!(status, QuotaStatus::Ok);
        }
    }

    #[test]
    fn any_current_zero_window_exhausts_even_when_another_is_positive() {
        let mut provider = window_provider("claude_code", Some(25.0));
        provider.weekly = Some(window(Some(0.0), Some("2030-01-03T12:00:00Z")));
        let status = evaluate_quota(
            &snapshot(provider),
            &ProviderType::ClaudeCode,
            None,
            QuotaCredentialScope::DefaultLocal,
            now(),
        )
        .expect("quota status");
        assert!(matches!(
            status,
            QuotaStatus::Exhausted {
                scope: QuotaScope::Window { ref name },
                ..
            } if name == "weekly"
        ));
    }

    #[test]
    fn stale_unavailable_missing_and_invalid_values_are_errors() {
        let mut stale = window_provider("claude_code", Some(0.0));
        stale.stale = true;
        assert!(matches!(
            evaluate_quota(
                &snapshot(stale),
                &ProviderType::ClaudeCode,
                None,
                QuotaCredentialScope::DefaultLocal,
                now()
            ),
            Err(QuotaCheckError::Indeterminate(_))
        ));

        let mut unavailable = window_provider("claude_code", Some(0.0));
        unavailable.available = false;
        assert!(matches!(
            evaluate_quota(
                &snapshot(unavailable),
                &ProviderType::ClaudeCode,
                None,
                QuotaCredentialScope::DefaultLocal,
                now()
            ),
            Err(QuotaCheckError::SourceUnavailable(_))
        ));

        for remaining in [
            None,
            Some(f64::NAN),
            Some(f64::INFINITY),
            Some(-1.0),
            Some(101.0),
        ] {
            assert!(matches!(
                evaluate_quota(
                    &snapshot(window_provider("claude_code", remaining)),
                    &ProviderType::ClaudeCode,
                    None,
                    QuotaCredentialScope::DefaultLocal,
                    now()
                ),
                Err(QuotaCheckError::Indeterminate(_))
            ));
        }
    }

    #[test]
    fn expired_or_invalid_reset_zero_is_indeterminate() {
        for reset in [Some("2029-12-31T12:00:00Z"), Some("not-a-time")] {
            let mut provider = window_provider("claude_code", Some(0.0));
            provider.session = Some(window(Some(0.0), reset));
            assert!(matches!(
                evaluate_quota(
                    &snapshot(provider),
                    &ProviderType::ClaudeCode,
                    None,
                    QuotaCredentialScope::DefaultLocal,
                    now()
                ),
                Err(QuotaCheckError::Indeterminate(_))
            ));
        }
    }

    #[test]
    fn customized_scope_fails_before_snapshot_inspection() {
        let empty = CodingUsageSnapshot {
            providers: Vec::new(),
            refreshed_at: None,
        };
        assert_eq!(
            evaluate_quota(
                &empty,
                &ProviderType::ClaudeCode,
                None,
                QuotaCredentialScope::Customized,
                now()
            ),
            Err(QuotaCheckError::CredentialScopeMismatch)
        );
    }

    #[test]
    fn provider_absent_from_snapshot_is_unsupported() {
        let snapshot = snapshot(window_provider("codex", Some(50.0)));
        assert_eq!(
            evaluate_quota(
                &snapshot,
                &ProviderType::Traex,
                None,
                QuotaCredentialScope::DefaultLocal,
                now()
            ),
            Ok(QuotaStatus::Unsupported)
        );
    }

    #[test]
    fn model_bucket_checks_only_the_exact_target() {
        let provider = CodingProviderUsage {
            id: "antigravity".to_owned(),
            available: true,
            stale: false,
            weekly: None,
            session: None,
            models: vec![
                bucket("model-a", "Model A", Some(0.0)),
                bucket("model-b", "Model B", Some(60.0)),
            ],
        };
        let snapshot = snapshot(provider);

        assert!(matches!(
            evaluate_quota(
                &snapshot,
                &ProviderType::AntigravityCli,
                Some("model-a"),
                QuotaCredentialScope::DefaultLocal,
                now()
            ),
            Ok(QuotaStatus::Exhausted {
                scope: QuotaScope::Model { ref name },
                ..
            }) if name == "Model A"
        ));
        assert_eq!(
            evaluate_quota(
                &snapshot,
                &ProviderType::AntigravityCli,
                Some("Model B"),
                QuotaCredentialScope::DefaultLocal,
                now()
            ),
            Ok(QuotaStatus::Ok)
        );
        assert!(matches!(
            evaluate_quota(
                &snapshot,
                &ProviderType::AntigravityCli,
                Some("model"),
                QuotaCredentialScope::DefaultLocal,
                now()
            ),
            Err(QuotaCheckError::ModelNotFound(_))
        ));
    }

    #[test]
    fn empty_model_and_expired_model_zero_fail_open_as_errors() {
        let mut provider = CodingProviderUsage {
            id: "antigravity".to_owned(),
            available: true,
            stale: false,
            weekly: None,
            session: None,
            models: vec![bucket("model-a", "Model A", Some(0.0))],
        };
        let snapshot_with_future_reset = snapshot(provider.clone());
        assert!(matches!(
            evaluate_quota(
                &snapshot_with_future_reset,
                &ProviderType::AntigravityCli,
                Some(" "),
                QuotaCredentialScope::DefaultLocal,
                now()
            ),
            Err(QuotaCheckError::ModelNotFound(_))
        ));

        provider.models[0].resets_at = Some("2029-12-31T12:00:00Z".to_owned());
        assert!(matches!(
            evaluate_quota(
                &snapshot(provider),
                &ProviderType::AntigravityCli,
                Some("model-a"),
                QuotaCredentialScope::DefaultLocal,
                now()
            ),
            Err(QuotaCheckError::Indeterminate(_))
        ));
    }

    #[test]
    fn sanitized_gateway_fixture_deserializes_and_missing_numeric_is_unknown() {
        let fixture = include_str!("../tests/fixtures/coding_usage_snapshot.json");
        let snapshot: CodingUsageSnapshot = serde_json::from_str(fixture).expect("fixture shape");
        assert_eq!(snapshot.providers.len(), 3);

        let missing_numeric: CodingUsageSnapshot = serde_json::from_str(
            r#"{
                "providers": [{
                    "id": "codex",
                    "available": true,
                    "session": {"used_percent": 100.0}
                }]
            }"#,
        )
        .expect("missing numeric stays optional");
        assert!(matches!(
            evaluate_quota(
                &missing_numeric,
                &ProviderType::CodexAppServer,
                None,
                QuotaCredentialScope::DefaultLocal,
                now()
            ),
            Err(QuotaCheckError::Indeterminate(_))
        ));
    }
}
