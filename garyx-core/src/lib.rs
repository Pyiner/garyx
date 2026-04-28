pub mod keys;
pub mod label;
pub mod route_resolver;
pub mod slash_commands;

// Re-export key types and functions at the crate root.
pub use keys::{
    // Constants
    AGENT_PREFIX,
    DEFAULT_AGENT_ID,
    GLOBAL_KEY,
    MAX_COMPONENT_LENGTH,
    MAX_KEY_LENGTH,
    MAX_SESSION_KEY_LENGTH,
    // Types
    ParsedSessionKey,
    SUBAGENT_SEGMENT,
    SessionKeyClass,
    SessionKeyError,
    THREAD_SEGMENT,
    UNKNOWN_KEY,
    VALID_SESSION_TYPES,
    // Key building
    build_agent_session_key,
    build_subagent_session_key,
    // Normalization & classification
    classify_session_key,
    extract_channel_from_key,
    // Predicates
    is_global_key,
    is_subagent_session_key,
    is_thread_session_key,
    is_unknown_key,
    normalize_session_key,
    // Key parsing
    parse_hierarchical_key,
    // Thread resolution
    resolve_thread_parent_session_key,
};

pub use label::{
    LabelValidationResult, MAX_LABEL_LENGTH, MIN_LABEL_LENGTH, labels_match,
    normalize_label_for_search, validate_session_label,
};

pub use route_resolver::{ResolvedRoute, RouteBinding, RouteMatch, RouteResolver};

pub use slash_commands::{
    SLASH_COMMAND_NAME_KEY, SLASH_COMMAND_PROMPT_APPLIED_KEY, SLASH_COMMAND_SKILL_ID_KEY,
    SLASH_COMMAND_TRIGGERED_KEY, annotate_slash_command_metadata, apply_custom_slash_command,
};
