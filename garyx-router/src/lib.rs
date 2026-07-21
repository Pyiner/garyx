pub mod endpoint_binding;
pub mod endpoint_projection;
pub mod file_store;
pub mod memory_store;
pub mod message_ledger;
pub mod recent_threads;
pub mod router;
pub mod run_admission;
pub mod runtime_context;
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
        AgentDispatcher, InboundRequest, InboundResult, MessageRouter, ThreadCreationError,
        ThreadCreator, ThreadMessageRequest,
    };
}

pub mod storage {
    pub use crate::file_store::FileThreadStore;
    pub use crate::memory_store::InMemoryThreadStore;
    pub use crate::message_ledger::{
        MessageLedgerError, MessageLedgerStore, SharedMessageLedgerStore,
    };
    pub use crate::store::{
        ThreadPatchResult, ThreadRecordPatch, ThreadStore, ThreadStoreDomains, ThreadStoreError,
        ThreadTerminalState,
    };
    pub use crate::thread_history::{
        BackfillOutcome, DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT, RECENT_COMMITTED_RUN_IDS_LIMIT,
        THREAD_TRANSCRIPT_REPLAY_CAP, ThreadHistoryError, ThreadHistoryRepository,
        ThreadHistorySnapshot, ThreadTranscriptRecord, ThreadTranscriptStore,
        ThreadTranscriptWindow, TranscriptReplaceStage, count_user_query_messages, extract_run_id,
        history_message_count, is_user_query_message, message_text,
    };
}

pub mod threading {
    pub use crate::threads::{
        ChannelBinding, KNOWN_CHANNEL_ENDPOINTS_KEY, KnownChannelEndpoint, ThreadEnsureOptions,
        agent_id_from_value, bindings_from_value, create_thread_record,
        default_agent_for_channel_account, default_workspace_for_channel_account,
        default_workspace_mode_for_channel_account, delete_thread_record, endpoint_key,
        is_default_thread_list_hidden, is_hidden_thread_value, is_thread_key, label_from_value,
        list_known_channel_endpoints, list_registry_channel_endpoints, new_thread_key,
        normalize_workspace_dir, prepare_thread_record, remove_binding, thread_kind_from_value,
        thread_metadata_from_value, update_thread_record, upsert_binding, upsert_thread_fields,
        validate_thread_accepts_bot_binding, workspace_dir_from_value,
        worktree_base_dir_for_config,
    };
}

pub use endpoint_binding::{
    EndpointBindResult, EndpointBindingMutationError, EndpointBindingMutator, EndpointBindingOwner,
    EndpointDeliveryTimestampResult, EndpointDetachResult,
};
pub use endpoint_projection::{
    ChannelEndpointProjection, DeliveryContextRow, ScanChannelEndpointProjection,
    channel_endpoint_projection_for,
};
pub use file_store::FileThreadStore;
pub use memory_store::InMemoryThreadStore;
pub use message_ledger::{MessageLedgerError, MessageLedgerStore, SharedMessageLedgerStore};
pub use router::default_agent_from_config;
pub use router::{
    AgentDispatcher, InboundRequest, InboundResult, MessageRouter,
    NATIVE_COMMAND_TEXT_METADATA_KEY, ThreadCreationError, ThreadCreator, ThreadMessageRequest,
    command_catalog_for_config, is_native_command_text, reserved_command_names,
};
pub use run_admission::{
    AdmittedRun, ArchiveBarrier, CoordinationError, DrainedDeleteReservation,
    LifecycleCommitWitness, LifecycleReservation, RunAdmissionError, ThreadRunAborter,
    ThreadRunCoordinator, ThreadRunLease,
};
pub use runtime_context::build_runtime_context_metadata;
pub use store::contract as store_contract;
pub use store::{
    AtomicRecordMerge, ChannelBindingsMergeAuthority, ThreadPatchResult, ThreadRecordPatch,
    ThreadStore, ThreadStoreDomains, ThreadStoreError, ThreadStoreExt, ThreadTerminalState,
    ensure_channel_bindings_unchanged, validate_channel_bindings,
};
pub use task_counter::{InMemoryTaskCounterStore, TaskCounterError, TaskCounterStore};
pub use tasks::{
    CreateTaskInput, EnterReview, NewTaskAgentGate, ScanTaskProjectionReader, TaskHistoryPage,
    TaskId, TaskListFilter, TaskProjectionReader, TaskRuntimeInput, TaskService, TaskServiceError,
    TaskSummary, UpdateTaskStatusInput, mark_thread_task_in_progress_on_wake,
    mark_thread_task_in_review_if_in_progress, task_projection_reader_for,
};
pub use thread_history::{
    BackfillOutcome, DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT, RECENT_COMMITTED_RUN_IDS_LIMIT,
    RunTranscriptRecordDraft, THREAD_TRANSCRIPT_REPLAY_CAP, ThreadHistoryError,
    ThreadHistoryRepository, ThreadHistorySnapshot, ThreadTranscriptRecord, ThreadTranscriptStore,
    ThreadTranscriptWindow, TranscriptAppendRecordsResult, TranscriptReplaceStage,
    count_user_query_messages, extract_run_id, history_message_count, is_user_query_message,
    message_text,
};
pub use threads::{
    ChannelBinding, KNOWN_CHANNEL_ENDPOINTS_KEY, KnownChannelEndpoint, ThreadEnsureOptions,
    agent_id_from_value, bindings_from_value, create_thread_record,
    default_agent_for_channel_account, default_workspace_for_channel_account,
    default_workspace_mode_for_channel_account, delete_thread_record, endpoint_key,
    is_default_thread_list_hidden, is_hidden_thread_value, is_thread_key, label_from_value,
    list_known_channel_endpoints, list_registry_channel_endpoints, new_thread_key,
    normalize_workspace_dir, prepare_thread_record, remove_binding, thread_kind_from_value,
    thread_metadata_from_value, update_thread_record, upsert_binding, upsert_thread_fields,
    validate_thread_accepts_bot_binding, workspace_dir_from_value, worktree_base_dir_for_config,
};
pub use worktree::{
    PreparedWorktree, WorkspaceGitStatus, WorkspaceMode, planned_thread_worktree_path,
    prepare_thread_worktree, workspace_git_status,
};
