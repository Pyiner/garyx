pub mod file_store;
pub mod memory_store;
pub mod message_ledger;
pub mod message_routing;
pub mod router;
pub mod runtime_context;
pub mod scrub;
pub mod store;
pub mod task_counter;
pub mod tasks;
pub mod thread_history;
pub mod threads;
pub mod worktree;

pub mod inbound {
    pub use crate::router::NATIVE_COMMAND_TEXT_METADATA_KEY;
    pub use crate::router::is_native_command_text;
}

pub mod routing {
    pub use crate::router::{
        AgentDispatcher, InboundRequest, InboundResult, InboundSink, MessageRouter, ThreadCreator,
        ThreadListEntry, ThreadMessageRequest,
    };
}

pub mod storage {
    pub use crate::file_store::FileThreadStore;
    pub use crate::memory_store::InMemoryThreadStore;
    pub use crate::message_ledger::{
        MessageLedgerError, MessageLedgerStore, SharedMessageLedgerStore,
    };
    pub use crate::message_routing::{
        MessageRoutingIndex, MessageRoutingStats, OutboundMessageRecord,
    };
    pub use crate::scrub::{cleanup_legacy_team_runs_dir, scrub_legacy_team_fields};
    pub use crate::store::{ThreadStore, ThreadStoreError};
    pub use crate::thread_history::{
        DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT, RECENT_COMMITTED_RUN_IDS_LIMIT,
        THREAD_TRANSCRIPT_REPLAY_CAP, ThreadHistoryError, ThreadHistoryRepository,
        ThreadHistorySnapshot, ThreadTranscriptRecord, ThreadTranscriptStore,
        count_user_query_messages, extract_run_id, history_message_count, is_user_query_message,
        message_text,
    };
}

pub mod threading {
    pub use crate::threads::{
        ChannelBinding, KnownChannelEndpoint, ThreadEnsureOptions, ThreadIndexStats,
        agent_id_from_value, bind_endpoint_to_thread, bindings_from_value, create_thread_record,
        default_agent_for_channel_account, default_workspace_for_channel_account,
        default_workspace_mode_for_channel_account, delete_thread_record,
        detach_endpoint_from_thread, endpoint_key, is_default_thread_list_hidden,
        is_hidden_thread_value, is_thread_key, label_from_value, list_known_channel_endpoints,
        list_registry_channel_endpoints, new_thread_key, normalize_workspace_dir,
        thread_kind_from_value, thread_metadata_from_value, update_thread_record, upsert_binding,
        upsert_thread_fields, workspace_dir_from_value, worktree_base_dir_for_config,
    };
}

pub use file_store::FileThreadStore;
pub use memory_store::InMemoryThreadStore;
pub use message_ledger::{MessageLedgerError, MessageLedgerStore, SharedMessageLedgerStore};
pub use message_routing::{MessageRoutingIndex, MessageRoutingStats, OutboundMessageRecord};
pub use router::{
    AgentDispatcher, InboundRequest, InboundResult, InboundSink, MessageRouter,
    NATIVE_COMMAND_TEXT_METADATA_KEY, ThreadCreator, ThreadMessageRequest,
    command_catalog_for_config, is_native_command_text, reserved_command_names,
};
pub use runtime_context::build_runtime_context_metadata;
pub use scrub::{cleanup_legacy_team_runs_dir, scrub_legacy_team_fields};
pub use store::{ThreadStore, ThreadStoreError};
pub use task_counter::{
    FileTaskCounterStore, InMemoryTaskCounterStore, TaskCounterError, TaskCounterStore,
};
pub use tasks::{
    CreateTaskInput, EnterReview, TaskHistoryPage, TaskId, TaskListFilter, TaskRuntimeInput,
    TaskService, TaskServiceError, TaskSummary, UpdateTaskStatusInput,
    mark_thread_task_in_progress_on_wake, mark_thread_task_in_review_if_in_progress,
};
pub use thread_history::{
    DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT, RECENT_COMMITTED_RUN_IDS_LIMIT,
    RunTranscriptRecordDraft, THREAD_TRANSCRIPT_REPLAY_CAP, ThreadHistoryError,
    ThreadHistoryRepository, ThreadHistorySnapshot, ThreadTranscriptRecord, ThreadTranscriptStore,
    TranscriptAppendRecordsResult, count_user_query_messages, extract_run_id,
    history_message_count, is_user_query_message, message_text,
};
pub use threads::{
    ChannelBinding, KnownChannelEndpoint, ThreadEnsureOptions, ThreadIndexStats,
    agent_id_from_value, bind_endpoint_to_thread, bindings_from_value, create_thread_record,
    default_agent_for_channel_account, default_workspace_for_channel_account,
    default_workspace_mode_for_channel_account, delete_thread_record, detach_endpoint_from_thread,
    endpoint_key, is_default_thread_list_hidden, is_hidden_thread_value, is_thread_key,
    label_from_value, list_known_channel_endpoints, list_registry_channel_endpoints,
    new_thread_key, normalize_workspace_dir, thread_kind_from_value, thread_metadata_from_value,
    update_thread_record, upsert_binding, upsert_thread_fields, workspace_dir_from_value,
    worktree_base_dir_for_config,
};
pub use worktree::{
    PreparedWorktree, WorkspaceGitStatus, WorkspaceMode, prepare_thread_worktree,
    workspace_git_status,
};
