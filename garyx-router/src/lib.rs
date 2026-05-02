pub mod conversation_index;
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
    pub use crate::conversation_index::{
        ConversationIndexManager, ConversationIndexResult, ConversationIndexSearchHit,
        ConversationIndexSearchRequest,
    };
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
        DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT, RECENT_COMMITTED_RUN_IDS_LIMIT, ThreadHistoryError,
        ThreadHistoryRepository, ThreadHistorySnapshot, ThreadTranscriptRecord,
        ThreadTranscriptStore, active_run_snapshot_messages, active_run_snapshot_run_id,
        extract_run_id, history_message_count, message_text,
    };
}

pub mod threading {
    pub use crate::threads::{
        ChannelBinding, KnownChannelEndpoint, ThreadEnsureOptions, ThreadIndexStats,
        agent_id_from_value, bind_endpoint_to_thread, bindings_from_value, create_thread_record,
        default_agent_for_channel_account, default_workspace_for_channel_account,
        delete_thread_record, detach_endpoint_from_thread, endpoint_key, is_hidden_thread_value,
        is_thread_key, label_from_value, list_known_channel_endpoints, loop_enabled_from_value,
        loop_iteration_count_from_value, new_thread_key, normalize_workspace_dir,
        thread_kind_from_value, thread_metadata_from_value, update_thread_record, upsert_binding,
        upsert_thread_fields, workspace_dir_from_value,
    };
}

pub use conversation_index::{
    ConversationIndexManager, ConversationIndexResult, ConversationIndexSearchHit,
    ConversationIndexSearchRequest,
};
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
    CreateTaskInput, PromoteTaskInput, TaskHistoryPage, TaskListFilter, TaskRef, TaskRuntimeInput,
    TaskService, TaskServiceError, TaskSummary, UpdateTaskStatusInput,
};
pub use thread_history::{
    DEFAULT_THREAD_HISTORY_SNAPSHOT_LIMIT, RECENT_COMMITTED_RUN_IDS_LIMIT, ThreadHistoryError,
    ThreadHistoryRepository, ThreadHistorySnapshot, ThreadTranscriptRecord, ThreadTranscriptStore,
    active_run_snapshot_messages, active_run_snapshot_run_id, extract_run_id,
    history_message_count, message_text,
};
pub use threads::{
    ChannelBinding, KnownChannelEndpoint, ThreadEnsureOptions, ThreadIndexStats,
    agent_id_from_value, bind_endpoint_to_thread, bindings_from_value, create_thread_record,
    default_agent_for_channel_account, default_workspace_for_channel_account, delete_thread_record,
    detach_endpoint_from_thread, endpoint_key, is_hidden_thread_value, is_thread_key,
    label_from_value, list_known_channel_endpoints, loop_enabled_from_value,
    loop_iteration_count_from_value, new_thread_key, normalize_workspace_dir,
    thread_kind_from_value, thread_metadata_from_value, update_thread_record, upsert_binding,
    upsert_thread_fields, workspace_dir_from_value,
};
