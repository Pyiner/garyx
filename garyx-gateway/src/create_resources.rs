use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use chrono::{Duration as ChronoDuration, SecondsFormat, Utc};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::{Duration, MissedTickBehavior};

use crate::garyx_db::{
    CreateIntentKey, CreateResourceKind, CreateResourceRecord, CreateResourceState,
};
use crate::server::AppState;
use crate::workspace_mode::{
    implicit_thread_workspace_dir_for_data_dir, worktree_base_dir_for_data_dir,
};

const CREATE_LEASE_TTL: ChronoDuration = ChronoDuration::minutes(2);
const CREATE_LEASE_RENEW_INTERVAL: Duration = Duration::from_secs(30);
const RESOURCE_CLEANUP_INTERVAL: Duration = Duration::from_secs(30);
const RESOURCE_CLEANUP_BATCH: usize = 64;

#[derive(Debug, Clone)]
pub(crate) struct CreateResourceLease {
    pub kind: CreateResourceKind,
    pub path: PathBuf,
    pub marker: String,
}

pub(crate) struct CreatePreparationLease {
    heartbeat: tokio::task::JoinHandle<()>,
}

impl Drop for CreatePreparationLease {
    fn drop(&mut self) {
        self.heartbeat.abort();
    }
}

pub(crate) fn create_lease_expires_at() -> String {
    (Utc::now() + CREATE_LEASE_TTL).to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(crate) fn start_create_lease_heartbeat(
    state: &Arc<AppState>,
    key: CreateIntentKey,
) -> CreatePreparationLease {
    let db = Arc::clone(&state.ops.garyx_db);
    let owner_boot_id = state.server_boot_id().to_owned();
    let heartbeat = tokio::spawn(async move {
        let mut interval = tokio::time::interval(CREATE_LEASE_RENEW_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        // The initial lease was written before this task was spawned. Do not
        // issue an immediate duplicate write.
        interval.tick().await;
        loop {
            interval.tick().await;
            let lease_expires_at = create_lease_expires_at();
            let db = Arc::clone(&db);
            let key = key.clone();
            let owner_boot_id = owner_boot_id.clone();
            match db
                .run_blocking(move |db| {
                    db.renew_create_intent_lease(&key, &owner_boot_id, &lease_expires_at)
                })
                .await
            {
                Ok(true) => {}
                Ok(false) => break,
                Err(error) => {
                    tracing::warn!(%error, "failed to renew create-intent preparation lease");
                }
            }
        }
    });
    CreatePreparationLease { heartbeat }
}

pub(crate) fn spawn_create_resource_cleanup_worker(state: &Arc<AppState>) {
    let state = Arc::downgrade(state);
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        tracing::warn!("create-resource cleanup worker requires a tokio runtime");
        return;
    };
    runtime.spawn(async move {
        let mut interval = tokio::time::interval(RESOURCE_CLEANUP_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            let Some(state) = state.upgrade() else {
                break;
            };
            if let Err(error) = process_due_create_resource_cleanup(&state).await {
                tracing::warn!(%error, "failed create-resource cleanup pass");
            }
        }
    });
}

pub(crate) async fn process_due_create_resource_cleanup(
    state: &Arc<AppState>,
) -> Result<(), String> {
    let db = Arc::clone(&state.ops.garyx_db);
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let candidates = db
        .run_blocking(move |db| {
            db.due_create_resource_cleanup_intents(&now, RESOURCE_CLEANUP_BATCH)
        })
        .await
        .map_err(|error| error.to_string())?;
    for (key, thread_id) in candidates {
        if let Err(error) = cleanup_unadopted_create_resources(state, &key, &thread_id).await {
            tracing::warn!(
                create_intent_id = %key.create_intent_id,
                %thread_id,
                %error,
                "create-resource cleanup remains pending"
            );
        }
    }
    Ok(())
}

pub(crate) async fn cleanup_unadopted_create_resources(
    state: &Arc<AppState>,
    key: &CreateIntentKey,
    thread_id: &str,
) -> Result<(), String> {
    let db = Arc::clone(&state.ops.garyx_db);
    let query_key = key.clone();
    let resources = db
        .run_blocking(move |db| db.create_resources_for_intent(&query_key))
        .await
        .map_err(|error| error.to_string())?;
    for resource in resources {
        if !matches!(
            resource.state,
            CreateResourceState::DeletePending
                | CreateResourceState::Materializing
                | CreateResourceState::Materialized
        ) {
            continue;
        }
        if let Err(error) = cleanup_resource(state, thread_id, &resource).await {
            let db = Arc::clone(&state.ops.garyx_db);
            let failed = resource.clone();
            let message = error.clone();
            let _ = db
                .run_blocking(move |db| {
                    db.fail_create_resource_cleanup(
                        failed.create_intent_row_id,
                        failed.kind,
                        &failed.resource_path,
                        &failed.owner_marker,
                        &message,
                    )
                })
                .await;
            return Err(error);
        }
        let db = Arc::clone(&state.ops.garyx_db);
        db.run_blocking(move |db| {
            db.mark_create_resource_deleted(
                resource.create_intent_row_id,
                resource.kind,
                &resource.resource_path,
                &resource.owner_marker,
            )?;
            Ok(())
        })
        .await
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(crate) async fn begin_create_resource(
    state: &Arc<AppState>,
    key: &CreateIntentKey,
    thread_id: &str,
    kind: CreateResourceKind,
    path: PathBuf,
) -> Result<CreateResourceLease, String> {
    validate_approved_resource_path(state, thread_id, kind, &path)?;
    let marker = format!("create-resource:{}", uuid::Uuid::new_v4());
    let path_string = path.to_string_lossy().into_owned();
    let db = Arc::clone(&state.ops.garyx_db);
    let db_key = key.clone();
    let db_path = path_string.clone();
    let db_marker = marker.clone();
    db.run_blocking(move |db| {
        db.begin_create_resource_materialization(&db_key, kind, &db_path, &db_marker)
            .map(|_| ())
    })
    .await
    .map_err(|error| error.to_string())?;

    let marker_path = owner_marker_path(&path);
    if let Some(parent) = marker_path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("failed to create resource marker parent: {error}"))?;
    }
    match fs::symlink_metadata(&marker_path).await {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err("create-resource owner marker is not a regular file".to_owned());
        }
        Ok(_) => {
            let existing = fs::read_to_string(&marker_path)
                .await
                .map_err(|error| format!("failed to read resource owner marker: {error}"))?;
            if existing != marker {
                return Err("create-resource owner marker is owned by another attempt".to_owned());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&marker_path)
                .await
                .map_err(|error| format!("failed to create resource owner marker: {error}"))?;
            file.write_all(marker.as_bytes())
                .await
                .map_err(|error| format!("failed to write resource owner marker: {error}"))?;
            file.sync_all()
                .await
                .map_err(|error| format!("failed to sync resource owner marker: {error}"))?;
        }
        Err(error) => return Err(format!("failed to inspect resource owner marker: {error}")),
    }
    Ok(CreateResourceLease { kind, path, marker })
}

pub(crate) async fn mark_create_resource_materialized(
    state: &Arc<AppState>,
    key: &CreateIntentKey,
    lease: &CreateResourceLease,
) -> Result<(), String> {
    let db = Arc::clone(&state.ops.garyx_db);
    let key = key.clone();
    let kind = lease.kind;
    let path = lease.path.to_string_lossy().into_owned();
    let marker = lease.marker.clone();
    let changed = db
        .run_blocking(move |db| db.mark_create_resource_materialized(&key, kind, &path, &marker))
        .await
        .map_err(|error| error.to_string())?;
    if !changed {
        return Err("create-resource materialization lost its reservation".to_owned());
    }
    Ok(())
}

pub(crate) async fn materialize_managed_workspace(
    state: &Arc<AppState>,
    key: &CreateIntentKey,
    thread_id: &str,
) -> Result<String, String> {
    let path = implicit_thread_workspace_dir_for_data_dir(
        state.ops.prompt_attachments.data_dir(),
        thread_id,
    );
    let lease = begin_create_resource(
        state,
        key,
        thread_id,
        CreateResourceKind::ManagedWorkspace,
        path.clone(),
    )
    .await?;
    match fs::symlink_metadata(&path).await {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err("managed workspace path is not a regular directory".to_owned());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&path)
                .await
                .map_err(|error| format!("failed to create managed workspace: {error}"))?;
        }
        Err(error) => return Err(format!("failed to inspect managed workspace: {error}")),
    }
    mark_create_resource_materialized(state, key, &lease).await?;
    Ok(path.to_string_lossy().into_owned())
}

pub(crate) async fn remove_adopted_resource_markers(state: &Arc<AppState>, key: &CreateIntentKey) {
    let db = Arc::clone(&state.ops.garyx_db);
    let key = key.clone();
    let Ok(resources) = db
        .run_blocking(move |db| db.create_resources_for_intent(&key))
        .await
    else {
        return;
    };
    for resource in resources {
        if resource.state != CreateResourceState::Adopted {
            continue;
        }
        let marker_path = owner_marker_path(Path::new(&resource.resource_path));
        if marker_matches(&marker_path, &resource.owner_marker)
            .await
            .unwrap_or(false)
        {
            let _ = fs::remove_file(marker_path).await;
        }
    }
}

async fn cleanup_resource(
    state: &Arc<AppState>,
    thread_id: &str,
    resource: &CreateResourceRecord,
) -> Result<(), String> {
    let path = PathBuf::from(&resource.resource_path);
    validate_approved_resource_path(state, thread_id, resource.kind, &path)?;
    let metadata = match fs::symlink_metadata(&path).await {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("failed to inspect create resource: {error}")),
    };
    if metadata.is_some() {
        if metadata
            .as_ref()
            .is_some_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err("refusing to clean a symlinked create resource".to_owned());
        }
        let marker_path = owner_marker_path(&path);
        if !marker_matches(&marker_path, &resource.owner_marker).await? {
            return Err("create-resource owner marker mismatch".to_owned());
        }
        match resource.kind {
            CreateResourceKind::ManagedWorkspace => fs::remove_dir_all(&path)
                .await
                .map_err(|error| format!("failed to remove managed workspace: {error}"))?,
            CreateResourceKind::ImportedTranscript => fs::remove_file(&path)
                .await
                .map_err(|error| format!("failed to remove imported transcript: {error}"))?,
            CreateResourceKind::Worktree => remove_worktree(&path).await?,
        }
    }
    let marker_path = owner_marker_path(&path);
    match fs::remove_file(marker_path).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("failed to remove create-resource marker: {error}")),
    }
    Ok(())
}

async fn remove_worktree(path: &Path) -> Result<(), String> {
    if !path.join(".git").exists() {
        return fs::remove_dir_all(path)
            .await
            .map_err(|error| format!("failed to remove partial worktree: {error}"));
    }
    let common_dir = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .await
        .map_err(|error| format!("failed to resolve worktree repository: {error}"))?;
    if !common_dir.status.success() {
        return Err(format!(
            "failed to resolve worktree repository: {}",
            String::from_utf8_lossy(&common_dir.stderr).trim()
        ));
    }
    let common_dir = PathBuf::from(String::from_utf8_lossy(&common_dir.stdout).trim());
    let repository = common_dir
        .parent()
        .ok_or_else(|| "resolved worktree common directory has no repository parent".to_owned())?;
    let output = Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(["worktree", "remove", "--force"])
        .arg(path)
        .output()
        .await
        .map_err(|error| format!("failed to run git worktree remove: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git worktree remove failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

fn owner_marker_path(path: &Path) -> PathBuf {
    let mut marker = path.as_os_str().to_os_string();
    marker.push(".garyx-create-owner");
    PathBuf::from(marker)
}

async fn marker_matches(path: &Path, expected: &str) -> Result<bool, String> {
    match fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Ok(false),
        Ok(_) => fs::read_to_string(path)
            .await
            .map(|value| value == expected)
            .map_err(|error| format!("failed to read create-resource marker: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("failed to inspect create-resource marker: {error}")),
    }
}

fn validate_approved_resource_path(
    state: &Arc<AppState>,
    thread_id: &str,
    kind: CreateResourceKind,
    path: &Path,
) -> Result<(), String> {
    if !path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::Prefix(_) | Component::CurDir
            )
        })
    {
        return Err("create-resource path is not absolute and normalized".to_owned());
    }
    let approved = match kind {
        CreateResourceKind::ManagedWorkspace => {
            path == implicit_thread_workspace_dir_for_data_dir(
                state.ops.prompt_attachments.data_dir(),
                thread_id,
            )
        }
        CreateResourceKind::Worktree => path.starts_with(worktree_base_dir_for_data_dir(
            state.ops.prompt_attachments.data_dir(),
        )),
        CreateResourceKind::ImportedTranscript => {
            state
                .threads
                .history
                .transcript_store()
                .transcript_path(thread_id)
                .as_deref()
                == Some(path)
        }
    };
    approved
        .then_some(())
        .ok_or_else(|| "create-resource path is outside its approved managed root".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::garyx_db::{CreateCommandKind, NewCreateIntent};
    use garyx_models::config::GaryxConfig;
    use tempfile::tempdir;

    #[tokio::test]
    async fn old_boot_managed_workspace_is_deleted_before_intent_reuse() {
        let root = tempdir().unwrap();
        let mut config = GaryxConfig::default();
        config.sessions.data_dir =
            Some(root.path().join("sessions").to_string_lossy().into_owned());
        let state = crate::server::AppStateBuilder::new(config).build();
        let key = CreateIntentKey {
            scope_identity: "resource-cleanup-test".to_owned(),
            scope_epoch: 1,
            create_intent_id: "managed-workspace-crash".to_owned(),
        };
        let thread_id = garyx_router::new_thread_key();
        let db = Arc::clone(&state.ops.garyx_db);
        let reserve_key = key.clone();
        let reserve_thread_id = thread_id.clone();
        db.run_blocking(move |db| {
            db.reserve_create_intent(NewCreateIntent {
                key: &reserve_key,
                thread_id: &reserve_thread_id,
                request_fingerprint: "resource-cleanup-fingerprint",
                command_kind: CreateCommandKind::CreateOnly,
                dispatch_client_intent_id: None,
            })?;
            db.mark_create_intent_preparing(&reserve_key, "old-boot", "2099-01-01T00:00:00Z")?;
            Ok(())
        })
        .await
        .unwrap();

        let workspace = PathBuf::from(
            materialize_managed_workspace(&state, &key, &thread_id)
                .await
                .unwrap(),
        );
        assert!(workspace.is_dir());

        let db = Arc::clone(&state.ops.garyx_db);
        db.run_blocking(move |db| {
            assert_eq!(db.recover_stale_create_intents()?, 1);
            Ok(())
        })
        .await
        .unwrap();
        process_due_create_resource_cleanup(&state).await.unwrap();

        assert!(!workspace.exists());
        let db = Arc::clone(&state.ops.garyx_db);
        let query_key = key.clone();
        let (claim, resources) = db
            .run_blocking(move |db| {
                Ok((
                    db.create_intent(&query_key)?.unwrap(),
                    db.create_resources_for_intent(&query_key)?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(claim.state, crate::garyx_db::CreateIntentState::Reserved);
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].state, CreateResourceState::Deleted);
    }

    #[tokio::test]
    async fn registered_worktree_cleanup_removes_directory_and_git_registration() {
        async fn git(repo: &Path, args: &[&str]) -> String {
            let output = Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(args)
                .output()
                .await
                .unwrap();
            assert!(
                output.status.success(),
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).into_owned()
        }

        let temp = tempdir().unwrap();
        let repo = temp.path().join("source");
        fs::create_dir(&repo).await.unwrap();
        git(&repo, &["init"]).await;
        git(&repo, &["config", "user.name", "Test User"]).await;
        git(&repo, &["config", "user.email", "test@example.com"]).await;
        fs::write(repo.join("README.md"), b"test\n").await.unwrap();
        git(&repo, &["add", "README.md"]).await;
        git(&repo, &["commit", "-m", "initial"]).await;

        let base = temp.path().join("managed-worktrees");
        let prepared = garyx_router::prepare_thread_worktree(
            "thread::resource-cleanup",
            repo.to_str().unwrap(),
            Some(&base),
        )
        .await
        .unwrap();
        let worktree = PathBuf::from(prepared.worktree_dir);
        assert!(worktree.is_dir());

        remove_worktree(&worktree).await.unwrap();

        assert!(!worktree.exists());
        let registered = git(&repo, &["worktree", "list", "--porcelain"]).await;
        assert!(!registered.contains(worktree.to_str().unwrap()));
    }
}
