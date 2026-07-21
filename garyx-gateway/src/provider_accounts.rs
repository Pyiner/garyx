use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use futures_util::future::join_all;
use garyx_models::config::{ClaudeCodeManagedAccount, GaryxConfig};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::coding_usage::{self, ProviderUsage};
use crate::mcp_config::{ConfigMutateError, mutate_config};
use crate::server::AppState;

const MANAGED_ROOT_COMPONENTS: &[&str] = &["provider-accounts", "claude-code"];
const OWNERSHIP_MARKER: &str = ".garyx-claude-account";

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeCodeAccountsResponse {
    pub active_account_id: Option<String>,
    pub accounts: Vec<ClaudeCodeAccountView>,
    pub refreshed_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaudeCodeAccountView {
    pub id: Option<String>,
    pub name: String,
    pub system_default: bool,
    pub selected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<String>,
    pub usage: ProviderUsage,
}

#[derive(Debug, Deserialize)]
pub struct SelectClaudeCodeAccountRequest {
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RenameClaudeCodeAccountRequest {
    name: String,
}

#[derive(Debug, Clone)]
struct AccountUsageSpec {
    id: Option<String>,
    name: String,
    system_default: bool,
    selected: bool,
    email: Option<String>,
    organization: Option<String>,
    plan: Option<String>,
    auth_method: Option<String>,
}

pub(crate) fn managed_accounts_root(config_path: Option<&Path>) -> PathBuf {
    let mut root = config_path
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(garyx_models::local_paths::gary_home_dir);
    for component in MANAGED_ROOT_COMPONENTS {
        root.push(component);
    }
    root
}

#[cfg(test)]
pub(crate) fn managed_account_dir(config_path: Option<&Path>, account_id: &str) -> PathBuf {
    managed_accounts_root(config_path).join(account_id)
}

pub(crate) async fn validated_active_claude_config_dir(
    state: &AppState,
    config: &GaryxConfig,
) -> Option<PathBuf> {
    let account_id = config
        .provider_accounts
        .claude_code
        .active_account_id
        .clone()?;
    let validation = if config
        .provider_accounts
        .claude_code
        .account(&account_id)
        .is_some()
    {
        validate_owned_account_dir(state, &account_id).await
    } else {
        Err(AccountsApiError::not_found(&account_id))
    };
    match validation {
        Ok(path) => Some(path),
        Err(error) => isolate_invalid_selection(state, &account_id, &error),
    }
}

fn isolate_invalid_selection(
    state: &AppState,
    account_id: &str,
    error: &AccountsApiError,
) -> Option<PathBuf> {
    // Never answer an invalid managed selection with the system profile: that
    // would silently run work under the wrong account. This fixed, nonexistent
    // quarantine path carries no credentials and does not interpolate a
    // potentially malformed account ID into the filesystem.
    tracing::warn!(account_id, error = %error.message, "invalid active Claude account; isolating future Claude runs");
    Some(
        state
            .ops
            .config_path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(garyx_models::local_paths::gary_home_dir)
            .join(".invalid-claude-account-selection"),
    )
}

pub async fn list_claude_code_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ClaudeCodeAccountsResponse>, AccountsApiError> {
    let config = state.config_snapshot();
    let active_id = config
        .provider_accounts
        .claude_code
        .active_account_id
        .clone();
    let mut specs = Vec::with_capacity(config.provider_accounts.claude_code.accounts.len() + 1);
    specs.push(AccountUsageSpec {
        id: None,
        name: "System default".to_owned(),
        system_default: true,
        selected: active_id.is_none(),
        email: None,
        organization: None,
        plan: None,
        auth_method: None,
    });
    specs.extend(
        config
            .provider_accounts
            .claude_code
            .accounts
            .iter()
            .map(|account| AccountUsageSpec {
                id: Some(account.id.clone()),
                name: account.name.clone(),
                system_default: false,
                selected: active_id.as_deref() == Some(account.id.as_str()),
                email: account.email.clone(),
                organization: account.organization.clone(),
                plan: account.plan.clone(),
                auth_method: account.auth_method.clone(),
            }),
    );
    drop(config);

    let accounts = join_all(specs.into_iter().map(|spec| {
        let state = state.clone();
        async move {
            let usage = if let Some(account_id) = spec.id.as_deref() {
                match validate_owned_account_dir(&state, account_id).await {
                    Ok(config_dir) => {
                        coding_usage::resolve_claude_usage_for_config_dir(
                            Some(&config_dir),
                            account_id,
                        )
                        .await
                    }
                    Err(error) => coding_usage::unavailable_claude_usage(error.message),
                }
            } else {
                coding_usage::resolve_claude_usage_for_config_dir(None, "system").await
            };
            ClaudeCodeAccountView {
                id: spec.id,
                name: spec.name,
                system_default: spec.system_default,
                selected: spec.selected,
                email: spec.email,
                organization: spec.organization,
                plan: spec.plan.or_else(|| usage.plan.clone()),
                auth_method: spec.auth_method,
                usage,
            }
        }
    }))
    .await;

    Ok(Json(ClaudeCodeAccountsResponse {
        active_account_id: active_id,
        accounts,
        refreshed_at: Utc::now().to_rfc3339(),
    }))
}

pub async fn select_claude_code_account(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SelectClaudeCodeAccountRequest>,
) -> Result<Json<Value>, AccountsApiError> {
    let account_id = normalize_optional_account_id(request.account_id)?;
    if let Some(account_id) = account_id.as_deref() {
        validate_owned_account_dir(&state, account_id).await?;
    }
    let selected = account_id.clone();
    mutate_config(&state, move |config| {
        if let Some(account_id) = selected.as_deref()
            && config
                .provider_accounts
                .claude_code
                .account(account_id)
                .is_none()
        {
            return Err(AccountsApiError::not_found(account_id));
        }
        config.provider_accounts.claude_code.active_account_id = selected.clone();
        Ok(())
    })
    .await
    .map_err(map_mutate_error)?;
    Ok(Json(json!({ "active_account_id": account_id })))
}

pub async fn rename_claude_code_account(
    State(state): State<Arc<AppState>>,
    AxumPath(account_id): AxumPath<String>,
    Json(request): Json<RenameClaudeCodeAccountRequest>,
) -> Result<Json<Value>, AccountsApiError> {
    let account_id = require_account_id(&account_id)?;
    let name = normalize_account_name(&request.name)?;
    let response_name = name.clone();
    let response_id = account_id.clone();
    mutate_config(&state, move |config| {
        let account = config
            .provider_accounts
            .claude_code
            .account_mut(&account_id)
            .ok_or_else(|| AccountsApiError::not_found(&account_id))?;
        account.name = name;
        account.updated_at = Utc::now().to_rfc3339();
        Ok(())
    })
    .await
    .map_err(map_mutate_error)?;
    Ok(Json(json!({ "id": response_id, "name": response_name })))
}

pub async fn delete_claude_code_account(
    State(state): State<Arc<AppState>>,
    AxumPath(account_id): AxumPath<String>,
) -> Result<Json<Value>, AccountsApiError> {
    let account_id = require_account_id(&account_id)?;
    let account_dir = validate_owned_account_dir(&state, &account_id).await?;
    let removed_id = account_id.clone();
    mutate_config(&state, move |config| {
        let accounts = &mut config.provider_accounts.claude_code;
        let Some(index) = accounts
            .accounts
            .iter()
            .position(|account| account.id == removed_id)
        else {
            return Err(AccountsApiError::not_found(&removed_id));
        };
        accounts.accounts.remove(index);
        if accounts.active_account_id.as_deref() == Some(removed_id.as_str()) {
            accounts.active_account_id = None;
        }
        Ok(())
    })
    .await
    .map_err(map_mutate_error)?;

    if let Err(error) = crate::claude_oauth::delete_scoped_oauth_keychain(&account_dir).await {
        tracing::warn!(
            account_id,
            error,
            "managed Claude account removed but scoped Keychain cleanup failed"
        );
    }
    coding_usage::invalidate_claude_usage_cache(&account_id);
    tokio::fs::remove_dir_all(&account_dir).await.map_err(|error| {
        AccountsApiError::internal(format!(
            "Account was removed from Garyx, but its managed directory {} could not be deleted: {error}",
            account_dir.display()
        ))
    })?;
    Ok(Json(json!({ "deleted_account_id": account_id })))
}

#[derive(Debug, Clone)]
pub(crate) struct ClaudeAuthTarget {
    pub account_id: Option<String>,
    pub config_dir: Option<PathBuf>,
    pub account_name: Option<String>,
    pub is_new: bool,
}

impl ClaudeAuthTarget {
    pub(crate) fn environment(&self) -> HashMap<String, String> {
        self.config_dir
            .as_ref()
            .map(|config_dir| {
                HashMap::from([(
                    "CLAUDE_CONFIG_DIR".to_owned(),
                    config_dir.to_string_lossy().into_owned(),
                )])
            })
            .unwrap_or_default()
    }
}

pub(crate) async fn prepare_auth_target(
    state: &Arc<AppState>,
    managed_account_name: Option<&str>,
    account_id: Option<&str>,
) -> Result<ClaudeAuthTarget, AccountsApiError> {
    if managed_account_name.is_some() && account_id.is_some() {
        return Err(AccountsApiError::bad_request(
            "ambiguous_auth_target",
            "Choose either a new managed account or an existing account to reauthenticate.",
        ));
    }
    if let Some(account_id) = account_id {
        let account_id = require_account_id(account_id)?;
        let config = state.config_snapshot();
        let account = config
            .provider_accounts
            .claude_code
            .account(&account_id)
            .ok_or_else(|| AccountsApiError::not_found(&account_id))?;
        let account_name = account.name.clone();
        drop(config);
        let config_dir = validate_owned_account_dir(state, &account_id).await?;
        return Ok(ClaudeAuthTarget {
            account_id: Some(account_id),
            config_dir: Some(config_dir),
            account_name: Some(account_name),
            is_new: false,
        });
    }
    if let Some(name) = managed_account_name {
        let name = normalize_account_name(name)?;
        let account_id = Uuid::new_v4().to_string();
        let config_dir = create_owned_account_dir(state, &account_id).await?;
        return Ok(ClaudeAuthTarget {
            account_id: Some(account_id),
            config_dir: Some(config_dir),
            account_name: Some(name),
            is_new: true,
        });
    }
    Ok(ClaudeAuthTarget {
        account_id: None,
        config_dir: None,
        account_name: None,
        is_new: false,
    })
}

pub(crate) async fn complete_auth_target(
    state: &Arc<AppState>,
    target: &ClaudeAuthTarget,
    auth_status: &Value,
) -> Result<(), AccountsApiError> {
    let Some(account_id) = target.account_id.clone() else {
        coding_usage::invalidate_claude_usage_cache("system");
        return Ok(());
    };
    let now = Utc::now().to_rfc3339();
    let email = status_string(auth_status, &["email"]);
    let organization = status_string(auth_status, &["orgName", "organization"]);
    let plan = status_string(auth_status, &["subscriptionType", "plan"]);
    let auth_method = status_string(auth_status, &["authMethod"]);
    let name = target
        .account_name
        .clone()
        .unwrap_or_else(|| "Claude account".to_owned());
    let is_new = target.is_new;
    let cache_identity = account_id.clone();
    mutate_config(state, move |config| {
        let accounts = &mut config.provider_accounts.claude_code;
        if is_new {
            if accounts.account(&account_id).is_some() {
                return Err(AccountsApiError::conflict(
                    "claude_account_exists",
                    "The managed Claude account already exists.",
                ));
            }
            // Adding an account never changes the active selection; switching
            // is an explicit user action through the select endpoint.
            accounts.accounts.push(ClaudeCodeManagedAccount {
                id: account_id.clone(),
                name,
                email,
                organization,
                plan,
                auth_method,
                created_at: now.clone(),
                updated_at: now,
            });
        } else {
            let account = accounts
                .account_mut(&account_id)
                .ok_or_else(|| AccountsApiError::not_found(&account_id))?;
            account.email = email;
            account.organization = organization;
            account.plan = plan;
            account.auth_method = auth_method;
            account.updated_at = now;
        }
        Ok(())
    })
    .await
    .map_err(map_mutate_error)?;
    coding_usage::invalidate_claude_usage_cache(&cache_identity);
    Ok(())
}

pub(crate) async fn cleanup_failed_auth_target(state: &Arc<AppState>, target: &ClaudeAuthTarget) {
    if !target.is_new {
        return;
    }
    let Some(account_id) = target.account_id.as_deref() else {
        return;
    };
    match validate_owned_account_dir(state, account_id).await {
        Ok(path) => {
            if let Err(error) = tokio::fs::remove_dir_all(&path).await {
                tracing::warn!(path = %path.display(), error = %error, "failed to clean up Claude auth profile");
            }
        }
        Err(error) => {
            tracing::warn!(account_id, error = %error.message, "refused unsafe Claude auth profile cleanup");
        }
    }
}

async fn create_owned_account_dir(
    state: &Arc<AppState>,
    account_id: &str,
) -> Result<PathBuf, AccountsApiError> {
    let account_id = require_account_id(account_id)?;
    let root = managed_accounts_root(state.ops.config_path.as_deref());
    ensure_managed_root(&root).await?;
    let account_dir = root.join(&account_id);
    tokio::fs::create_dir(&account_dir).await.map_err(|error| {
        AccountsApiError::internal(format!(
            "Could not create managed Claude account directory {}: {error}",
            account_dir.display()
        ))
    })?;
    let marker = account_dir.join(OWNERSHIP_MARKER);
    if let Err(error) = tokio::fs::write(&marker, format!("{account_id}\n")).await {
        let _ = tokio::fs::remove_dir(&account_dir).await;
        return Err(AccountsApiError::internal(format!(
            "Could not create Claude account ownership marker {}: {error}",
            marker.display()
        )));
    }
    validate_owned_account_dir(state, &account_id).await
}

async fn ensure_managed_root(root: &Path) -> Result<(), AccountsApiError> {
    let container = managed_root_container(root).ok_or_else(|| {
        AccountsApiError::internal(format!(
            "Managed Claude account root {} has an invalid layout.",
            root.display()
        ))
    })?;
    ensure_directory_component(container).await?;
    ensure_directory_component(root).await?;
    canonical_safe_managed_root(root).await.map_err(|error| {
        AccountsApiError::internal(format!(
            "Managed Claude account root {} is not safe: {error}",
            root.display()
        ))
    })?;
    Ok(())
}

async fn ensure_directory_component(path: &Path) -> Result<(), AccountsApiError> {
    match tokio::fs::create_dir(path).await {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(AccountsApiError::internal(format!(
                "Could not create managed Claude account directory {}: {error}",
                path.display()
            )));
        }
    }
    let metadata = tokio::fs::symlink_metadata(path).await.map_err(|error| {
        AccountsApiError::internal(format!(
            "Could not inspect managed Claude account directory {}: {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AccountsApiError::internal(format!(
            "Managed Claude account directory {} is not a safe directory.",
            path.display()
        )));
    }
    Ok(())
}

fn managed_root_container(root: &Path) -> Option<&Path> {
    root.parent()
}

fn managed_root_anchor(root: &Path) -> Option<&Path> {
    managed_root_container(root)?.parent()
}

async fn canonical_safe_managed_root(root: &Path) -> Result<PathBuf, String> {
    let container = managed_root_container(root)
        .ok_or_else(|| "missing provider-accounts container".to_owned())?;
    let anchor = managed_root_anchor(root).ok_or_else(|| "missing config parent".to_owned())?;
    for path in [container, root] {
        let metadata = tokio::fs::symlink_metadata(path)
            .await
            .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!("{} is not a real directory", path.display()));
        }
    }
    let canonical_anchor = tokio::fs::canonicalize(anchor)
        .await
        .map_err(|error| format!("could not resolve {}: {error}", anchor.display()))?;
    let canonical_container = tokio::fs::canonicalize(container)
        .await
        .map_err(|error| format!("could not resolve {}: {error}", container.display()))?;
    let canonical_root = tokio::fs::canonicalize(root)
        .await
        .map_err(|error| format!("could not resolve {}: {error}", root.display()))?;
    if canonical_container.parent() != Some(canonical_anchor.as_path())
        || canonical_root.parent() != Some(canonical_container.as_path())
    {
        return Err("managed root escaped the Garyx config directory".to_owned());
    }
    Ok(canonical_root)
}

async fn validate_owned_account_dir(
    state: &AppState,
    account_id: &str,
) -> Result<PathBuf, AccountsApiError> {
    let account_id = require_account_id(account_id)?;
    let root = managed_accounts_root(state.ops.config_path.as_deref());
    let account_dir = root.join(&account_id);
    let canonical_root = canonical_safe_managed_root(&root)
        .await
        .map_err(|_| AccountsApiError::unsafe_directory(&account_dir))?;
    let account_metadata = tokio::fs::symlink_metadata(&account_dir)
        .await
        .map_err(|_| AccountsApiError::unsafe_directory(&account_dir))?;
    if account_metadata.file_type().is_symlink() || !account_metadata.is_dir() {
        return Err(AccountsApiError::unsafe_directory(&account_dir));
    }
    let canonical_account = tokio::fs::canonicalize(&account_dir)
        .await
        .map_err(|_| AccountsApiError::unsafe_directory(&account_dir))?;
    if canonical_account.parent() != Some(canonical_root.as_path()) {
        return Err(AccountsApiError::unsafe_directory(&account_dir));
    }
    let marker = account_dir.join(OWNERSHIP_MARKER);
    let marker_metadata = tokio::fs::symlink_metadata(&marker)
        .await
        .map_err(|_| AccountsApiError::unsafe_directory(&account_dir))?;
    if marker_metadata.file_type().is_symlink() || !marker_metadata.is_file() {
        return Err(AccountsApiError::unsafe_directory(&account_dir));
    }
    let marker_value = tokio::fs::read_to_string(&marker)
        .await
        .map_err(|_| AccountsApiError::unsafe_directory(&account_dir))?;
    if marker_value.trim() != account_id {
        return Err(AccountsApiError::unsafe_directory(&account_dir));
    }
    Ok(account_dir)
}

fn status_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn normalize_optional_account_id(
    account_id: Option<String>,
) -> Result<Option<String>, AccountsApiError> {
    account_id
        .map(|account_id| require_account_id(&account_id))
        .transpose()
}

fn require_account_id(account_id: &str) -> Result<String, AccountsApiError> {
    let trimmed = account_id.trim();
    if !valid_account_id(trimmed) {
        return Err(AccountsApiError::bad_request(
            "invalid_claude_account_id",
            "Claude account ID is invalid.",
        ));
    }
    Ok(trimmed.to_owned())
}

fn valid_account_id(account_id: &str) -> bool {
    Uuid::parse_str(account_id).is_ok()
}

fn normalize_account_name(name: &str) -> Result<String, AccountsApiError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(AccountsApiError::bad_request(
            "invalid_claude_account_name",
            "Account name must contain 1 to 60 characters.",
        ));
    }
    Ok(name.to_owned())
}

fn map_mutate_error(error: ConfigMutateError<AccountsApiError>) -> AccountsApiError {
    match error {
        ConfigMutateError::Rejected(error) => error,
        ConfigMutateError::Apply(error) => AccountsApiError::internal(error),
    }
}

#[derive(Debug)]
pub(crate) struct AccountsApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl AccountsApiError {
    pub(crate) fn into_parts(self) -> (StatusCode, &'static str, String) {
        (self.status, self.code, self.message)
    }

    fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message: message.into(),
        }
    }

    fn not_found(account_id: &str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "claude_account_not_found",
            message: format!("Claude account '{account_id}' was not found."),
        }
    }

    fn conflict(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code,
            message: message.into(),
        }
    }

    fn unsafe_directory(path: &Path) -> Self {
        Self::conflict(
            "unsafe_claude_account_directory",
            format!(
                "Managed Claude account directory {} failed ownership checks.",
                path.display()
            ),
        )
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "claude_account_operation_failed",
            message: message.into(),
        }
    }
}

impl IntoResponse for AccountsApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "error": {
                    "code": self.code,
                    "message": self.message,
                }
            })),
        )
            .into_response()
    }
}

impl std::fmt::Display for AccountsApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn managed_account_directory_is_derived_below_owned_root() {
        let config_path = Path::new("/tmp/garyx/config.yaml");
        let id = Uuid::new_v4().to_string();
        assert_eq!(
            managed_account_dir(Some(config_path), &id),
            PathBuf::from("/tmp/garyx/provider-accounts/claude-code").join(id)
        );
    }

    #[tokio::test]
    async fn invalid_active_account_is_quarantined_instead_of_using_system_profile() {
        let temp = tempdir().unwrap();
        let id = Uuid::new_v4().to_string();
        let mut config = GaryxConfig::default();
        config
            .provider_accounts
            .claude_code
            .accounts
            .push(ClaudeCodeManagedAccount {
                id: id.clone(),
                name: "Missing".to_owned(),
                email: None,
                organization: None,
                plan: None,
                auth_method: None,
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            });
        config.provider_accounts.claude_code.active_account_id = Some(id.clone());
        let state = crate::server::AppStateBuilder::new(config.clone())
            .with_config_path(temp.path().join("config.yaml"))
            .build();

        let selected = validated_active_claude_config_dir(&state, &config)
            .await
            .expect("invalid managed selection must still isolate Claude");
        assert_eq!(
            selected,
            temp.path().join(".invalid-claude-account-selection")
        );
        assert_ne!(selected, temp.path().join(".claude"));

        config.provider_accounts.claude_code.accounts.clear();
        let unknown_selected = validated_active_claude_config_dir(&state, &config)
            .await
            .expect("unknown managed selection must still isolate Claude");
        assert_eq!(unknown_selected, selected);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ownership_validation_rejects_symlinked_marker() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let state = crate::server::AppStateBuilder::new(GaryxConfig::default())
            .with_config_path(temp.path().join("config.yaml"))
            .build();
        let target = prepare_auth_target(&state, Some("Work"), None)
            .await
            .expect("managed target");
        let account_id = target.account_id.as_deref().unwrap();
        let config_dir = target.config_dir.as_deref().unwrap();
        let marker = config_dir.join(OWNERSHIP_MARKER);
        std::fs::remove_file(&marker).unwrap();
        let outside = temp.path().join("outside-marker");
        std::fs::write(&outside, account_id).unwrap();
        symlink(&outside, &marker).unwrap();

        let error = validate_owned_account_dir(&state, account_id)
            .await
            .expect_err("symlink marker must be rejected");
        assert_eq!(error.code, "unsafe_claude_account_directory");
        assert!(
            config_dir.is_dir(),
            "unsafe cleanup must not remove directory"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn managed_root_creation_rejects_symlinked_container_without_writing_outside() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let outside = tempdir().unwrap();
        symlink(outside.path(), temp.path().join("provider-accounts")).unwrap();
        let state = crate::server::AppStateBuilder::new(GaryxConfig::default())
            .with_config_path(temp.path().join("config.yaml"))
            .build();

        let error = prepare_auth_target(&state, Some("Work"), None)
            .await
            .expect_err("a symlinked managed-root component must be rejected");
        assert_eq!(error.code, "claude_account_operation_failed");
        assert!(
            !outside.path().join("claude-code").exists(),
            "validation must happen before creating anything through the symlink"
        );
    }
}
