pub mod agent;
pub mod agent_availability;
pub mod agent_reference;
pub mod channel_outbound;
pub mod codex_models;
pub mod command_catalog;
pub mod config;
pub mod config_loader;
pub mod custom_agent;
pub mod execution;
pub mod local_paths;
pub mod message_lifecycle;
pub mod message_preview;
pub mod messages;
pub mod provider;
pub mod quota;
pub mod routing;
pub mod task;
pub mod thread_logs;
pub mod thread_record;
pub mod threading;
pub mod tool_field_projection;
pub mod transcript_kind;
pub mod transcript_render_state;
pub mod transcript_run_state;

// Re-export commonly used types at the crate root.
pub use agent::RunState;
pub use agent_availability::{
    AgentAvailabilitySnapshot, AgentBindingError, ResolvedAgentBinding,
    SERVER_OWNED_AGENT_METADATA_KEYS, ensure_enabled_for_new_binding, resolve_agent_binding,
    resolve_effective_default, strip_server_owned_agent_metadata,
};
pub use agent_reference::{
    AgentReference, agent_runtime_metadata, agent_runtime_snapshot_metadata,
    merge_thread_agent_runtime_snapshot, resolve_agent_reference,
};
pub use channel_outbound::ChannelOutboundContent;
pub use command_catalog::{
    CommandCatalog, CommandCatalogEntry, CommandCatalogOptions, CommandDispatch, CommandKind,
    CommandSource, CommandSurface, CommandVisibility, CommandWarning,
};
pub use config::GaryxConfig;
pub use config_loader::{
    ConfigDiagnostic, ConfigDiagnostics, ConfigHotReloadOptions, ConfigHotReloader,
    ConfigLoadFailure, ConfigLoadOptions, ConfigReloadMetricsSnapshot, ConfigRuntimeOverrides,
    ConfigWriteOptions, LoadedConfig, backup_config, list_backups, load_config, process_includes,
    restore_config, write_config_atomic, write_config_value_atomic,
};
pub use custom_agent::{
    CustomAgentProfile, builtin_provider_agent_profiles, is_builtin_provider_agent_id,
};
pub use execution::{
    ElevatedLevel, ExecAsk, ExecHost, ExecSecurity, ReasoningLevel, ResponseUsage,
};
pub use message_lifecycle::{
    BotThreadProblemSummary, MessageLedgerEvent, MessageLedgerRecord, MessageLifecycleStatus,
    MessageTerminalReason,
};
pub use messages::MessageMetadata;
pub use provider::{
    ATTACHMENTS_METADATA_KEY, AntigravityCliConfig, FilePayload, ImagePayload, PromptAttachment,
    PromptAttachmentKind, ProviderMessage, ProviderMessageRole, ProviderRunOptions,
    ProviderRunResult, ProviderType, StreamBoundaryKind, StreamEvent, attachments_from_metadata,
    attachments_to_metadata_value, build_prompt_message_with_attachments,
    build_user_content_from_parts, file_attachments_from_paths, stage_file_payloads_for_prompt,
    stage_image_payloads_for_prompt,
};
pub use quota::{
    CodingModelUsage, CodingProviderUsage, CodingUsageSnapshot, CodingUsageWindow, QuotaCheckError,
    QuotaCredentialScope, QuotaScope, QuotaStatus, evaluate_quota,
};
pub use routing::DeliveryContext;
pub use task::{
    Principal, TASK_SCHEMA_VERSION_V1, TaskEvent, TaskEventKind, TaskExecutor,
    TaskNotificationTarget, TaskSource, TaskStatus, ThreadTask,
};
pub use thread_logs::{
    CANONICAL_THREAD_PREFIX, NoopThreadLogSink, ThreadLogChunk, ThreadLogEvent, ThreadLogLevel,
    ThreadLogSink, is_canonical_thread_id, resolve_thread_log_thread_id,
};
pub use thread_record::{
    ProviderRuntimeState, THREAD_HISTORY_SOURCE_TRANSCRIPT_V1, ThreadHistoryState,
    ThreadQueueState, ThreadRecord, ThreadRecordView, ThreadRoutingState, ThreadUsageState,
};
pub use threading::{
    GroupActivation, QueueDrop, QueueMode, SendPolicy, ThreadOrigin, ThreadTokenUsage,
};
pub use tool_field_projection::{
    RenderToolFieldFormat, RenderToolFieldLabel, RenderToolFieldProjection, RenderToolFieldRoot,
    RenderToolFieldSelector, RenderToolKind, RenderToolVisibility,
};
pub use transcript_kind::{
    is_control_message, is_tool_related_message, resolve_message_kind,
    resolve_message_kind_for_object,
};
pub use transcript_render_state::{
    RenderActivityRow, RenderAssistantReplyRow, RenderAssistantStep, RenderCapsuleAction,
    RenderCapsuleCard, RenderDelta, RenderDeltaError, RenderFilteredPlaceholder, RenderMessageRef,
    RenderPlaceholderFilterReason, RenderProgressLocus, RenderRow, RenderRowsDigest,
    RenderSnapshot, RenderStepItem, RenderStepRow, RenderTailActivity, RenderToolEntry,
    RenderToolEntryStatus, RenderToolGroup, RenderToolGroupStatus, RenderUserTurnRow, RenderWindow,
    TranscriptRenderPrefixState, apply_render_delta, apply_transcript_render_prefix_record,
    derive_render_delta, derive_render_delta_from_base, final_assistant_text_from_render_records,
    reduce_transcript_render_prefix_state, reduce_transcript_render_state,
    reduce_transcript_render_state_with_prefix_state,
    reduce_transcript_render_state_with_run_state, render_row_hash, render_row_id,
    render_rows_digest,
};
pub use transcript_run_state::{
    TranscriptRunActivity, TranscriptRunState, apply_transcript_record, reduce_transcript_run_state,
};
