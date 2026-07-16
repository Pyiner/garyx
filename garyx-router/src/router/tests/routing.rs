use super::*;
use serde_json::json;

#[test]
fn test_switched_thread_is_account_scoped() {
    let mut router = make_router();

    let bot1_key = MessageRouter::build_binding_context_key("telegram", "bot1", "u1");
    router.switch_to_thread(&bot1_key, "custom_bot1");

    let bot1_thread = router.resolve_inbound_thread("telegram", "bot1", "u1", false, Some("u1"));
    let bot2_thread = router.resolve_inbound_thread("telegram", "bot2", "u1", false, Some("u1"));

    assert_eq!(bot1_thread, "custom_bot1");
    assert!(bot2_thread.starts_with("thread::"));
}

#[test]
fn test_is_scheduled_thread() {
    assert!(MessageRouter::is_scheduled_thread("cron::daily"));
    assert!(!MessageRouter::is_scheduled_thread("bot1::main::user1"));
}

#[test]
fn test_resolve_agent_default() {
    let router = make_router();
    assert_eq!(
        router.resolve_agent_for_channel("telegram", "bot1", Some("u1"), false),
        "main"
    );
}

#[test]
fn test_update_config() {
    let mut router = make_router();
    assert_eq!(router.default_agent, "main");

    let mut new_config = GaryxConfig::default();
    new_config
        .agents
        .insert("default".to_owned(), json!("assistant1"));
    router.update_config(new_config);
    assert_eq!(router.default_agent, "assistant1");
}
