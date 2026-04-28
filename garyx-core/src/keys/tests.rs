use super::*;
// --- build_agent_session_key ---

#[test]
fn test_build_agent_session_key_no_thread() {
    assert_eq!(
        build_agent_session_key("main", "telegram", "dm", "user123", None),
        "agent:main:telegram:dm:user123"
    );
}

#[test]
fn test_build_agent_session_key_with_thread() {
    assert_eq!(
        build_agent_session_key("main", "slack", "channel", "C123", Some("t456")),
        "agent:main:slack:channel:C123:thread:t456"
    );
}

// --- build_subagent_session_key ---

#[test]
fn test_build_subagent_session_key() {
    assert_eq!(
        build_subagent_session_key("main", "openai-gpt4"),
        "agent:main:subagent:openai-gpt4"
    );
}
// --- parse_hierarchical_key ---

#[test]
fn test_parse_hierarchical_key_full() {
    let parsed = parse_hierarchical_key("agent:main:telegram:dm:user123").unwrap();
    assert_eq!(parsed.agent_id, "main");
    assert_eq!(parsed.channel.as_deref(), Some("telegram"));
    assert_eq!(parsed.surface.as_deref(), Some("dm"));
    assert_eq!(parsed.peer_id, "user123");
    assert_eq!(parsed.session_type, "dm");
    assert!(!parsed.is_subagent);
    assert!(parsed.thread_id.is_none());
}

#[test]
fn test_parse_hierarchical_key_with_thread() {
    let parsed = parse_hierarchical_key("agent:main:telegram:dm:user123:thread:t1").unwrap();
    assert_eq!(parsed.thread_id.as_deref(), Some("t1"));
}

#[test]
fn test_parse_hierarchical_key_subagent() {
    let parsed = parse_hierarchical_key("agent:main:subagent:gpt4").unwrap();
    assert!(parsed.is_subagent);
    assert_eq!(parsed.subagent_name.as_deref(), Some("gpt4"));
    assert_eq!(parsed.session_type, "subagent");
    assert_eq!(parsed.peer_id, "gpt4");
}

#[test]
fn test_parse_hierarchical_not_hierarchical() {
    assert!(parse_hierarchical_key("main::main::user").is_err());
}

#[test]
fn test_parse_hierarchical_too_few_parts() {
    assert!(parse_hierarchical_key("agent:main:telegram").is_err());
}
// --- predicates ---

#[test]
fn test_is_subagent_session_key() {
    assert!(is_subagent_session_key("agent:main:subagent:gpt4"));
    assert!(!is_subagent_session_key("main::main::user123"));
}

#[test]
fn test_is_thread_session_key() {
    assert!(is_thread_session_key(
        "agent:main:telegram:dm:user123:thread:t1"
    ));
    assert!(!is_thread_session_key("main::main::user123"));
}

#[test]
fn test_is_global_key() {
    assert!(is_global_key("global"));
    assert!(!is_global_key("main::main::global"));
}

#[test]
fn test_is_unknown_key() {
    assert!(is_unknown_key("unknown"));
    assert!(!is_unknown_key("main::main::unknown"));
}

// --- resolve_thread_parent ---

#[test]
fn test_resolve_thread_parent() {
    assert_eq!(
        resolve_thread_parent_session_key("agent:main:telegram:dm:user123:thread:t1"),
        Some("agent:main:telegram:dm:user123".to_owned())
    );
}

#[test]
fn test_resolve_thread_parent_not_thread() {
    assert_eq!(
        resolve_thread_parent_session_key("main::main::user123"),
        None
    );
}

// --- normalize_session_key ---

#[test]
fn test_normalize_already_hierarchical() {
    let key = "agent:main:telegram:dm:user123";
    assert_eq!(normalize_session_key(key, None), key);
}

#[test]
fn test_normalize_special_keys() {
    assert_eq!(normalize_session_key("global", None), "global");
    assert_eq!(normalize_session_key("unknown", None), "unknown");
}

#[test]
fn test_normalize_simple_key() {
    let key = "main::main::user123";
    assert_eq!(normalize_session_key(key, None), key);
}

#[test]
fn test_normalize_channel_prefixed() {
    assert_eq!(
        normalize_session_key("telegram:dm:user123", None),
        "agent:main:telegram:dm:user123"
    );
}

#[test]
fn test_normalize_channel_prefixed_custom_agent() {
    assert_eq!(
        normalize_session_key("telegram:dm:user123", Some("assistant")),
        "agent:assistant:telegram:dm:user123"
    );
}

// --- classify_session_key ---

#[test]
fn test_classify_global() {
    assert_eq!(classify_session_key("global"), SessionKeyClass::Global);
}

#[test]
fn test_classify_unknown() {
    assert_eq!(classify_session_key("unknown"), SessionKeyClass::Unknown);
}

#[test]
fn test_classify_subagent() {
    assert_eq!(
        classify_session_key("agent:main:subagent:gpt4"),
        SessionKeyClass::Subagent
    );
}

#[test]
fn test_classify_thread() {
    assert_eq!(
        classify_session_key("agent:main:telegram:dm:user:thread:t1"),
        SessionKeyClass::Thread
    );
}

#[test]
fn test_classify_group() {
    assert_eq!(
        classify_session_key("main::group::-987654321"),
        SessionKeyClass::Group
    );
}

#[test]
fn test_classify_channel() {
    assert_eq!(
        classify_session_key("main::channel::C123"),
        SessionKeyClass::Group
    );
}

#[test]
fn test_classify_direct() {
    assert_eq!(
        classify_session_key("main::main::user123"),
        SessionKeyClass::Direct
    );
}

// --- extract_channel_from_key ---

#[test]
fn test_extract_channel_hierarchical() {
    assert_eq!(
        extract_channel_from_key("agent:main:telegram:dm:user123"),
        Some("telegram".to_owned())
    );
}

#[test]
fn test_extract_channel_simple() {
    assert_eq!(extract_channel_from_key("main::main::user123"), None);
}
// --- roundtrip tests ---
#[test]
fn test_build_then_parse_hierarchical() {
    let key = build_agent_session_key("main", "telegram", "group", "G123", Some("t42"));
    let parsed = parse_hierarchical_key(&key).unwrap();
    assert_eq!(parsed.agent_id, "main");
    assert_eq!(parsed.channel.as_deref(), Some("telegram"));
    assert_eq!(parsed.surface.as_deref(), Some("group"));
    assert_eq!(parsed.peer_id, "G123");
    assert_eq!(parsed.thread_id.as_deref(), Some("t42"));
}

#[test]
fn test_build_then_parse_subagent() {
    let key = build_subagent_session_key("main", "openai");
    let parsed = parse_hierarchical_key(&key).unwrap();
    assert!(parsed.is_subagent);
    assert_eq!(parsed.subagent_name.as_deref(), Some("openai"));
}
