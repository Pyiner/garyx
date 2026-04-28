use super::*;
use serde_json::json;

fn make_config(agents: serde_json::Value) -> GaryxConfig {
    let mut config = GaryxConfig::default();
    if let Some(obj) = agents.as_object() {
        for (k, v) in obj {
            config.agents.insert(k.clone(), v.clone());
        }
    }
    config
}

#[test]
fn test_default_agent_no_config() {
    let config = GaryxConfig::default();
    let mut resolver = RouteResolver::new(config);
    let route = resolver.resolve("telegram", None, None, None, None, None);
    assert_eq!(route.agent_id, "main");
    assert!(route.is_default);
}

#[test]
fn test_custom_default_agent() {
    let config = make_config(json!({
        "default": "assistant"
    }));
    let mut resolver = RouteResolver::new(config);
    let route = resolver.resolve("telegram", None, None, None, None, None);
    assert_eq!(route.agent_id, "assistant");
    assert!(route.is_default);
}

#[test]
fn test_channel_binding() {
    let config = make_config(json!({
        "bindings": [
            {
                "agentId": "telegram-agent",
                "match": { "channel": "telegram" }
            }
        ]
    }));
    let mut resolver = RouteResolver::new(config);

    let route = resolver.resolve("telegram", None, None, None, None, None);
    assert_eq!(route.agent_id, "telegram-agent");
    assert!(!route.is_default);

    let route = resolver.resolve("slack", None, None, None, None, None);
    assert_eq!(route.agent_id, "main");
    assert!(route.is_default);
}

#[test]
fn test_peer_kind_binding() {
    let config = make_config(json!({
        "bindings": [
            {
                "agentId": "group-agent",
                "match": {
                    "channel": "telegram",
                    "peer": { "kind": "group" }
                }
            }
        ]
    }));
    let mut resolver = RouteResolver::new(config);

    let route = resolver.resolve("telegram", None, Some("group"), None, None, None);
    assert_eq!(route.agent_id, "group-agent");

    let route = resolver.resolve("telegram", None, Some("dm"), None, None, None);
    assert_eq!(route.agent_id, "main");
}

#[test]
fn test_peer_id_binding() {
    let config = make_config(json!({
        "bindings": [
            {
                "agentId": "vip-agent",
                "match": {
                    "peer": { "id": "user42" }
                }
            }
        ]
    }));
    let mut resolver = RouteResolver::new(config);

    let route = resolver.resolve("telegram", None, None, Some("user42"), None, None);
    assert_eq!(route.agent_id, "vip-agent");

    let route = resolver.resolve("telegram", None, None, Some("user99"), None, None);
    assert_eq!(route.agent_id, "main");
}

#[test]
fn test_peer_pattern_binding() {
    let config = make_config(json!({
        "bindings": [
            {
                "agentId": "admin-agent",
                "match": {
                    "peer": { "pattern": "^admin_.*" }
                }
            }
        ]
    }));
    let mut resolver = RouteResolver::new(config);

    let route = resolver.resolve("telegram", None, None, Some("admin_bob"), None, None);
    assert_eq!(route.agent_id, "admin-agent");

    let route = resolver.resolve("telegram", None, None, Some("user_bob"), None, None);
    assert_eq!(route.agent_id, "main");
}

#[test]
fn test_peer_pattern_binding_without_peer_id_does_not_match() {
    let config = make_config(json!({
        "bindings": [
            {
                "agentId": "admin-agent",
                "match": {
                    "peer": { "pattern": "^admin_.*" }
                }
            }
        ]
    }));
    let mut resolver = RouteResolver::new(config);

    let route = resolver.resolve("telegram", None, None, None, None, None);
    assert_eq!(route.agent_id, "main");
    assert!(route.is_default);
}

#[test]
fn test_priority_ordering() {
    let config = make_config(json!({
        "bindings": [
            {
                "agentId": "low-priority",
                "priority": 10,
                "match": { "channel": "telegram" }
            },
            {
                "agentId": "high-priority",
                "priority": 100,
                "match": { "channel": "telegram" }
            }
        ]
    }));
    let mut resolver = RouteResolver::new(config);

    let route = resolver.resolve("telegram", None, None, None, None, None);
    assert_eq!(route.agent_id, "high-priority");
}

#[test]
fn test_account_wildcard() {
    let config = make_config(json!({
        "bindings": [
            {
                "agentId": "any-account-agent",
                "match": {
                    "channel": "telegram",
                    "accountId": "*"
                }
            }
        ]
    }));
    let mut resolver = RouteResolver::new(config);

    let route = resolver.resolve("telegram", Some("acct1"), None, None, None, None);
    assert_eq!(route.agent_id, "any-account-agent");
}

#[test]
fn test_account_specific() {
    let config = make_config(json!({
        "bindings": [
            {
                "agentId": "acct1-agent",
                "match": {
                    "accountId": "acct1"
                }
            }
        ]
    }));
    let mut resolver = RouteResolver::new(config);

    let route = resolver.resolve("telegram", Some("acct1"), None, None, None, None);
    assert_eq!(route.agent_id, "acct1-agent");

    let route = resolver.resolve("telegram", Some("acct2"), None, None, None, None);
    assert_eq!(route.agent_id, "main");
}

#[test]
fn test_guild_id_binding() {
    let config = make_config(json!({
        "bindings": [
            {
                "agentId": "guild-agent",
                "match": { "guildId": "guild123" }
            }
        ]
    }));
    let mut resolver = RouteResolver::new(config);

    let route = resolver.resolve("discord", None, None, None, Some("guild123"), None);
    assert_eq!(route.agent_id, "guild-agent");

    let route = resolver.resolve("discord", None, None, None, Some("other"), None);
    assert_eq!(route.agent_id, "main");
}

#[test]
fn test_team_id_binding() {
    let config = make_config(json!({
        "bindings": [
            {
                "agentId": "team-agent",
                "match": { "teamId": "T123" }
            }
        ]
    }));
    let mut resolver = RouteResolver::new(config);

    let route = resolver.resolve("slack", None, None, None, None, Some("T123"));
    assert_eq!(route.agent_id, "team-agent");

    let route = resolver.resolve("slack", None, None, None, None, Some("T999"));
    assert_eq!(route.agent_id, "main");
}

#[test]
fn test_update_config() {
    let config1 = make_config(json!({ "default": "agent1" }));
    let mut resolver = RouteResolver::new(config1);
    assert_eq!(resolver.get_default_agent(), "agent1");

    let config2 = make_config(json!({ "default": "agent2" }));
    resolver.update_config(config2);
    assert_eq!(resolver.get_default_agent(), "agent2");
}

#[test]
fn test_update_config_resets_default_when_missing() {
    let config1 = make_config(json!({ "default": "agent1" }));
    let mut resolver = RouteResolver::new(config1);
    assert_eq!(resolver.get_default_agent(), "agent1");

    let config2 = make_config(json!({
        "bindings": [
            { "agentId": "peer-only", "match": { "peer": { "id": "x" } } }
        ]
    }));
    resolver.update_config(config2);
    assert_eq!(resolver.get_default_agent(), "main");
}

#[test]
fn test_list_bindings() {
    let config = make_config(json!({
        "bindings": [
            { "agentId": "a1", "match": { "channel": "telegram" } },
            { "agentId": "a2", "match": { "channel": "slack" } }
        ]
    }));
    let resolver = RouteResolver::new(config);
    let bindings = resolver.list_bindings();
    assert_eq!(bindings.len(), 2);
}

#[test]
fn test_invalid_binding_skipped() {
    let config = make_config(json!({
        "bindings": [
            "not an object",
            { "no_agent_id": true },
            { "agentId": "valid", "match": { "channel": "ok" } }
        ]
    }));
    let resolver = RouteResolver::new(config);
    assert_eq!(resolver.list_bindings().len(), 1);
}

#[test]
fn test_invalid_regex_pattern() {
    let config = make_config(json!({
        "bindings": [
            {
                "agentId": "bad-regex-agent",
                "match": { "peer": { "pattern": "[invalid" } }
            }
        ]
    }));
    let mut resolver = RouteResolver::new(config);
    // Should not match because regex is invalid
    let route = resolver.resolve("telegram", None, None, Some("anything"), None, None);
    assert_eq!(route.agent_id, "main");
}

#[test]
fn test_invalid_regex_pattern_cached_once() {
    let config = make_config(json!({
        "bindings": [
            {
                "agentId": "bad-regex-agent",
                "match": { "peer": { "pattern": "[invalid" } }
            }
        ]
    }));
    let mut resolver = RouteResolver::new(config);

    let route1 = resolver.resolve("telegram", None, None, Some("a"), None, None);
    let route2 = resolver.resolve("telegram", None, None, Some("b"), None, None);

    assert_eq!(route1.agent_id, "main");
    assert_eq!(route2.agent_id, "main");
    assert_eq!(resolver.pattern_cache.len(), 1);
    assert!(
        resolver
            .pattern_cache
            .get("[invalid")
            .is_some_and(|re| re.is_none())
    );
}
