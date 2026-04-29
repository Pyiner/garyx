pub mod agent;
pub mod agent_node;
pub mod agent_reference;
pub mod agent_team;
pub mod auto_research;
pub mod command_catalog;
pub mod config;
pub mod config_loader;
pub mod custom_agent;
pub mod debug_runtime;
pub mod execution;
pub mod local_paths;
pub mod messages;
pub mod provider;
pub mod routing;
pub mod session;
pub mod thread_logs;
pub mod thread_record;
pub mod threading;
pub mod wiki;

// Re-export commonly used types at the crate root.
pub use agent::RunState;
pub use agent_node::{GatewayToNode, NodeLoadInfo, NodeToGateway, ProviderInfo};
pub use agent_reference::{
    AgentReference, resolve_agent_reference, validate_agent_team_registry_uniqueness,
};
pub use agent_team::AgentTeamProfile;
pub use auto_research::{
    AutoResearchIteration, AutoResearchIterationState, AutoResearchRun, AutoResearchRunState,
    Candidate, Verdict,
};
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
pub use custom_agent::{CustomAgentProfile, builtin_provider_agent_profiles};
pub use debug_runtime::{
    BotThreadDebugSummary, MessageLedgerEvent, MessageLedgerRecord, MessageLifecycleStatus,
    MessageTerminalReason,
};
pub use execution::{
    ElevatedLevel, ExecAsk, ExecHost, ExecSecurity, ReasoningLevel, ResponseUsage,
};
pub use messages::MessageMetadata;
pub use provider::{
    ATTACHMENTS_METADATA_KEY, FilePayload, GeminiCliConfig, ImagePayload, PromptAttachment,
    PromptAttachmentKind, ProviderMessage, ProviderMessageRole, ProviderRunOptions,
    ProviderRunResult, ProviderType, StreamBoundaryKind, StreamEvent, attachments_from_metadata,
    attachments_to_metadata_value, build_prompt_message_with_attachments,
    build_user_content_from_parts, file_attachments_from_paths, stage_file_payloads_for_prompt,
    stage_image_payloads_for_prompt,
};
pub use routing::DeliveryContext;
pub use session::{ChatType, SessionEntry, SessionOrigin, SessionTokenUsage};
pub use thread_logs::{
    CANONICAL_THREAD_PREFIX, NoopThreadLogSink, ThreadLogChunk, ThreadLogEvent, ThreadLogLevel,
    ThreadLogSink, is_canonical_thread_id, resolve_thread_log_thread_id,
};
pub use thread_record::{
    ActiveRunSnapshot, ProviderRuntimeState, THREAD_HISTORY_SOURCE_TRANSCRIPT_V1,
    ThreadHistoryState, ThreadQueueState, ThreadRecord, ThreadRecordView, ThreadRoutingState,
    ThreadUsageState,
};
pub use threading::{
    GroupActivation, QueueDrop, QueueMode, SendPolicy, ThreadOrigin, ThreadTokenUsage,
};
pub use wiki::WikiEntry;
