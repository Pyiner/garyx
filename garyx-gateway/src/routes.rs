use crate::agent_identity::create_thread_for_agent_reference;
use crate::endpoint_binding_mutator::DeleteBindingPreflight;
use crate::garyx_db::{
    FavoriteThreadResult, GaryxDbError, GaryxDbResult, LifecycleDecisionInput,
    LifecycleMutationInput, LifecycleOperationKind, LifecycleOperationLookup,
    LifecycleOperationOutcome, LifecycleOperationRecord, LifecycleTransactionResult,
    MAX_RECENT_THREAD_ACTIVITY_SEQ_EXCLUSIVE, RecentThreadRecord, RecentThreadTaskFilter,
    ReorderThreadPinsResult, ThreadFavoritesPage, ThreadMetaRecord, ThreadPinsPage,
    ThreadSummaryTaskFilter,
};
use crate::provider_session_locator::{
    list_recent_local_provider_sessions, recover_local_provider_session,
};
use crate::server::AppState;
use crate::skills::SkillStoreError;
use crate::thread_lifecycle::{
    LIFECYCLE_JOIN_WINDOW, MutationSupervisor, OperationCellResult, OperationJoinHandle,
    OperationKey, OperationOwnerGuard, OperationRegistration, OperationRegistrationError,
    OperationWaitError, canonical_lifecycle_fingerprint,
};
use crate::thread_meta_projection::normalize_for_search;
use crate::thread_runtime::{
    AgentCatalogSnapshot, build_thread_runtime_summary, build_thread_runtime_summary_from_meta,
    build_thread_runtime_summary_with_catalog,
};
use crate::thread_type::thread_summary_type_from_record;
use crate::workspace_mode::{
    ensure_implicit_thread_workspace_for_config, worktree_base_dir_for_config,
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use futures_util::StreamExt;
use garyx_channels::plugin::{PluginAccountUi, PluginConversationEndpoint, PluginMainEndpoint};
use garyx_models::RenderSnapshot;
use garyx_models::config::ChannelsConfig;
#[cfg(test)]
use garyx_models::config::TelegramAccount;
use garyx_models::provider::{
    FORK_FROM_PROVIDER_TYPE_METADATA_KEY, FORK_FROM_SDK_SESSION_ID_METADATA_KEY,
    FORK_FROM_THREAD_ID_METADATA_KEY, MODEL_METADATA_KEY, MODEL_OVERRIDE_METADATA_KEY,
    MODEL_REASONING_EFFORT_METADATA_KEY, MODEL_REASONING_EFFORT_OVERRIDE_METADATA_KEY,
    MODEL_SERVICE_TIER_METADATA_KEY, MODEL_SERVICE_TIER_OVERRIDE_METADATA_KEY, ProviderType,
    SDK_SESSION_FORK_METADATA_KEY,
};
use garyx_models::routing::{DELIVERY_TARGET_TYPE_CHAT_ID, DELIVERY_TARGET_TYPE_OPEN_ID};
use garyx_router::ThreadStoreExt;
#[cfg(test)]
use garyx_router::create_thread_record;
use garyx_router::{
    ArchiveBarrier, ChannelBinding, CoordinationError, KnownChannelEndpoint,
    THREAD_TRANSCRIPT_REPLAY_CAP, ThreadCreationError, ThreadEnsureOptions, ThreadRecordPatch,
    ThreadTranscriptRecord, WorkspaceMode, bindings_from_value, history_message_count,
    is_thread_key, update_thread_record, workspace_dir_from_value,
    workspace_git_status as router_workspace_git_status,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::io;
use std::path::Path as FsPath;
use std::sync::Arc;
use std::time::Duration;
use tokio_stream;
use tokio_stream::wrappers::BroadcastStream;

mod bot_bindings;
mod bot_consoles;
mod channel_endpoints;
mod custom_agents;
mod diagnostics;
mod history;
mod lifecycle;
mod pins_favorites;
mod restart;
mod runtime;
mod send;
mod settings;
mod skills;
mod stream;
mod thread_summaries;
mod threads;
mod workspace_git;

pub use crate::automation::debug_api::*;
pub use bot_bindings::*;
pub use bot_consoles::*;
pub use channel_endpoints::*;
pub use custom_agents::*;
pub use diagnostics::*;
pub use history::*;
pub use lifecycle::*;
pub use pins_favorites::*;
pub use restart::*;
pub use runtime::*;
pub use send::*;
pub use settings::*;
pub use skills::*;
pub use stream::*;
pub use thread_summaries::*;
pub use threads::*;
pub use workspace_git::*;

fn trimmed_nonempty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .map(ToOwned::to_owned)
}

fn extend_json_object(payload: &mut Value, fields: Value) {
    let Some(payload) = payload.as_object_mut() else {
        return;
    };
    let Some(fields) = fields.as_object() else {
        return;
    };
    payload.extend(fields.clone());
}

fn garyx_db_error_response(error: GaryxDbError) -> (StatusCode, Json<Value>) {
    let (status, code) = match &error {
        GaryxDbError::BadRequest(_) => (StatusCode::BAD_REQUEST, "BadRequest"),
        GaryxDbError::NotFound(_) => (StatusCode::NOT_FOUND, "NotFound"),
        GaryxDbError::ThreadArchived(_) => (StatusCode::GONE, "ThreadArchived"),
        GaryxDbError::LockPoisoned
        | GaryxDbError::Join(_)
        | GaryxDbError::Configuration(_)
        | GaryxDbError::DataDirLocked { .. }
        | GaryxDbError::ParentHandoffTimedOut { .. }
        | GaryxDbError::Io(_)
        | GaryxDbError::Sqlite(_) => (StatusCode::INTERNAL_SERVER_ERROR, "InternalError"),
    };
    (
        status,
        Json(json!({
            "error": code,
            "message": error.to_string(),
        })),
    )
}

/// Uniform 500 body for store/projection failures at request boundaries.
fn thread_store_error_response(
    error: &garyx_router::ThreadStoreError,
) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "ok": false,
            "reason": "thread-store-error",
            "error": error.to_string(),
        })),
    )
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod api_tests;
