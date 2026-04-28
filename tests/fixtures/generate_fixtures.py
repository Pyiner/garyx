#!/usr/bin/env python3
"""Generate JSON fixtures for Rust differential testing.

Imports from Garyx Python source and generates JSON fixture files that
capture the exact behavior of each function. The Rust test harness loads
these fixtures and verifies its implementation matches Python.
"""
import json
import os
import sys
from datetime import UTC, datetime

# ---------------------------------------------------------------------------
# Bootstrap: add Garyx src to path so we can import the Python modules
# ---------------------------------------------------------------------------
_REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", "src"))
sys.path.insert(0, _REPO_ROOT)

from garyx.sessions.keys import (
    build_agent_session_key,
    build_subagent_session_key,
    parse_hierarchical_key,
    classify_session_key,
    normalize_session_key,
    is_subagent_session_key,
    is_thread_session_key,
    is_global_key,
    extract_channel_from_key,
    SessionKeyValidationError,
)
from garyx.sessions.label import (
    validate_session_label,
    normalize_label_for_search,
    labels_match,
)
from garyx.sessions.route_resolver import RouteResolver

FIXTURES_DIR = os.path.dirname(os.path.abspath(__file__))
GENERATED_AT = datetime.now(UTC).isoformat()


def _safe_call(fn, *args, **kwargs):
    """Call fn and return (result, None) or (None, error_info)."""
    try:
        result = fn(*args, **kwargs)
        return result, None
    except Exception as e:
        return None, {"error_type": type(e).__name__, "error_message": str(e)}


def write_fixture(name: str, data: dict) -> None:
    path = os.path.join(FIXTURES_DIR, name)
    with open(path, "w") as f:
        json.dump(data, f, indent=2, ensure_ascii=False)
    print(f"  wrote {name} ({len(data['test_cases'])} test cases)")


# ============================================================================
# keys_fixtures.json
# ============================================================================
def generate_keys_fixtures() -> None:
    cases = []

    # --- build_agent_session_key ---
    hier_inputs = [
        ("build_hier_dm", "main", "telegram", "dm", "user123", None),
        ("build_hier_group", "main", "telegram", "group", "chat456", None),
        ("build_hier_thread", "main", "slack", "channel", "C123", "t456"),
        ("build_hier_no_thread", "bot2", "discord", "dm", "user789", None),
    ]
    for name, agent_id, channel, surface, peer_id, thread_id in hier_inputs:
        result = build_agent_session_key(agent_id, channel, surface, peer_id, thread_id)
        cases.append({
            "name": f"test_{name}",
            "function": "build_agent_session_key",
            "input": {
                "agent_id": agent_id, "channel": channel, "surface": surface,
                "peer_id": peer_id, "thread_id": thread_id,
            },
            "expected": result,
        })

    # --- build_subagent_session_key ---
    sub_inputs = [
        ("build_subagent_gpt4", "main", "openai-gpt4"),
        ("build_subagent_claude", "assistant1", "claude-3"),
    ]
    for name, agent_id, subagent_name in sub_inputs:
        result = build_subagent_session_key(agent_id, subagent_name)
        cases.append({
            "name": f"test_{name}",
            "function": "build_subagent_session_key",
            "input": {"agent_id": agent_id, "subagent_name": subagent_name},
            "expected": result,
        })

    # --- parse_hierarchical_key ---
    hier_parse_inputs = [
        ("hier_parse_dm", "agent:main:telegram:dm:user123"),
        ("hier_parse_group", "agent:main:telegram:group:chat456"),
        ("hier_parse_thread", "agent:main:slack:channel:C123:thread:t456"),
        ("hier_parse_subagent", "agent:main:subagent:openai-gpt4"),
    ]
    for name, key in hier_parse_inputs:
        result, err = _safe_call(parse_hierarchical_key, key)
        if err:
            cases.append({
                "name": f"test_{name}",
                "function": "parse_hierarchical_key",
                "input": {"session_key": key},
                "expected_error": True,
                **err,
            })
        else:
            cases.append({
                "name": f"test_{name}",
                "function": "parse_hierarchical_key",
                "input": {"session_key": key},
                "expected": {
                    "agent_id": result.agent_id,
                    "session_type": result.session_type,
                    "peer_id": result.peer_id,
                    "channel": result.channel,
                    "surface": result.surface,
                    "thread_id": result.thread_id,
                    "is_subagent": result.is_subagent,
                    "subagent_name": result.subagent_name,
                },
            })

    hier_parse_errors = [
        ("hier_parse_not_agent", "notagent:main:telegram:dm:user123"),
        ("hier_parse_too_short", "agent:main:telegram"),
    ]
    for name, key in hier_parse_errors:
        result, err = _safe_call(parse_hierarchical_key, key)
        if err:
            cases.append({
                "name": f"test_{name}",
                "function": "parse_hierarchical_key",
                "input": {"session_key": key},
                "expected_error": True,
                **err,
            })

    # --- classify_session_key ---
    classify_inputs = [
        ("classify_global", "global"),
        ("classify_unknown", "unknown"),
        ("classify_subagent", "agent:main:subagent:gpt4"),
        ("classify_thread", "agent:main:telegram:dm:user:thread:t1"),
        ("classify_group_simple", "main::group::chat123"),
        ("classify_channel_simple", "main::channel::C123"),
        ("classify_direct_simple", "main::dm::user123"),
        ("classify_direct_main", "main::main::user123"),
        ("classify_hier_group", "agent:main:telegram:group:chat456"),
        ("classify_hier_dm", "agent:main:telegram:dm:user456"),
    ]
    for name, key in classify_inputs:
        result = classify_session_key(key)
        cases.append({
            "name": f"test_{name}",
            "function": "classify_session_key",
            "input": {"session_key": key},
            "expected": result,
        })

    # --- normalize_session_key ---
    normalize_inputs = [
        ("normalize_already_hier", "agent:main:telegram:dm:user123", "main"),
        ("normalize_simple", "main::dm::user123", "main"),
        ("normalize_channel_prefixed", "telegram:dm:user123", "main"),
        ("normalize_channel_prefixed_custom_agent", "telegram:dm:user123", "bot2"),
        ("normalize_global", "global", "main"),
        ("normalize_unknown", "unknown", "main"),
        ("normalize_short", "ab", "main"),
    ]
    for name, key, default_agent in normalize_inputs:
        result = normalize_session_key(key, default_agent)
        cases.append({
            "name": f"test_{name}",
            "function": "normalize_session_key",
            "input": {"session_key": key, "default_agent_id": default_agent},
            "expected": result,
        })

    # --- boolean checks ---
    bool_checks = [
        ("is_subagent_true", "is_subagent_session_key", "agent:main:subagent:gpt4", True),
        ("is_subagent_false", "is_subagent_session_key", "main::dm::user123", False),
        ("is_thread_true", "is_thread_session_key", "agent:main:telegram:dm:u:thread:t1", True),
        ("is_thread_false", "is_thread_session_key", "main::dm::user123", False),
        ("is_global_true", "is_global_key", "global", True),
        ("is_global_false", "is_global_key", "main::dm::user123", False),
        ("is_global_not_prefix", "is_global_key", "global_extra", False),
    ]
    for name, fn_name, key, expected in bool_checks:
        fn_map = {
            "is_subagent_session_key": is_subagent_session_key,
            "is_thread_session_key": is_thread_session_key,
            "is_global_key": is_global_key,
        }
        result = fn_map[fn_name](key)
        cases.append({
            "name": f"test_{name}",
            "function": fn_name,
            "input": {"session_key": key},
            "expected": result,
        })

    # --- extract_channel_from_key ---
    channel_inputs = [
        ("extract_channel_hier", "agent:main:telegram:dm:user123", "telegram"),
        ("extract_channel_slack", "agent:main:slack:channel:C123:thread:t1", "slack"),
        ("extract_channel_simple", "main::main::user123", None),
        ("extract_channel_subagent", "agent:main:subagent:gpt4", None),
    ]
    for name, key, expected in channel_inputs:
        result = extract_channel_from_key(key)
        cases.append({
            "name": f"test_{name}",
            "function": "extract_channel_from_key",
            "input": {"session_key": key},
            "expected": result,
        })

    write_fixture("keys_fixtures.json", {
        "module": "keys",
        "generated_at": GENERATED_AT,
        "test_cases": cases,
    })


# ============================================================================
# label_fixtures.json
# ============================================================================
def generate_label_fixtures() -> None:
    cases = []

    # --- validate_session_label ---
    validate_inputs = [
        ("validate_normal", "My Session"),
        ("validate_single_char", "A"),
        ("validate_numbers", "session123"),
        ("validate_hyphens_underscores", "my-session_1"),
        ("validate_with_spaces", "My Cool Session"),
        ("validate_empty", ""),
        ("validate_whitespace_only", "   "),
        ("validate_too_long", "a" * 65),
        ("validate_max_length", "a" * 64),
        ("validate_special_chars", "session!@#"),
        ("validate_leading_space_trimmed", "  hello  "),
        ("validate_starts_with_special", "-invalid"),
        ("validate_ends_with_special", "invalid-"),
        ("validate_starts_with_space_after_trim", "ok"),
    ]
    for name, label in validate_inputs:
        result = validate_session_label(label)
        case = {
            "name": f"test_{name}",
            "function": "validate_session_label",
            "input": {"raw_label": label},
            "expected": {
                "ok": result.ok,
                "label": result.label,
                "error": result.error,
            },
        }
        cases.append(case)

    # --- normalize_label_for_search ---
    normalize_inputs = [
        ("normalize_mixed_case", "Hello World"),
        ("normalize_all_upper", "HELLO"),
        ("normalize_all_lower", "hello"),
        ("normalize_with_spaces", "  Hello  "),
        ("normalize_empty", ""),
    ]
    for name, label in normalize_inputs:
        result = normalize_label_for_search(label)
        cases.append({
            "name": f"test_{name}",
            "function": "normalize_label_for_search",
            "input": {"label": label},
            "expected": result,
        })

    # --- labels_match ---
    match_inputs = [
        ("match_same_case", "Hello", "Hello", True),
        ("match_diff_case", "Hello", "hello", True),
        ("match_with_spaces", "  Hello  ", "hello", True),
        ("match_different", "Hello", "World", False),
        ("match_empty", "", "", True),
        ("match_one_empty", "Hello", "", False),
    ]
    for name, label1, label2, expected in match_inputs:
        result = labels_match(label1, label2)
        cases.append({
            "name": f"test_{name}",
            "function": "labels_match",
            "input": {"label1": label1, "label2": label2},
            "expected": result,
        })

    write_fixture("label_fixtures.json", {
        "module": "label",
        "generated_at": GENERATED_AT,
        "test_cases": cases,
    })


# ============================================================================
# route_resolver_fixtures.json
# ============================================================================
def generate_route_resolver_fixtures() -> None:
    """Generate route resolver fixtures.

    Since RouteResolver requires a GaryxConfig, we test it by constructing
    minimal configs with known bindings and capturing resolve() results.
    """
    cases = []

    # We need a minimal GaryxConfig-like object. RouteResolver only reads
    # config.agents (a dict). We can use a simple namespace.
    class FakeConfig:
        def __init__(self, agents=None):
            self.agents = agents

    # --- Test 1: no bindings, default agent ---
    config = FakeConfig(agents=None)
    resolver = RouteResolver(config)
    result = resolver.resolve(channel="telegram")
    cases.append({
        "name": "test_no_bindings_default",
        "config": {"agents": None},
        "input": {"channel": "telegram"},
        "expected": {
            "agent_id": result.agent_id,
            "is_default": result.is_default,
        },
    })

    # --- Test 2: channel binding ---
    config = FakeConfig(agents={
        "default": "main",
        "bindings": [
            {"agentId": "telegram-bot", "match": {"channel": "telegram"}},
            {"agentId": "slack-bot", "match": {"channel": "slack"}},
        ],
    })
    resolver = RouteResolver(config)

    for channel, expected_agent, expected_default in [
        ("telegram", "telegram-bot", False),
        ("slack", "slack-bot", False),
        ("discord", "main", True),
    ]:
        result = resolver.resolve(channel=channel)
        cases.append({
            "name": f"test_channel_binding_{channel}",
            "config": {"agents": config.agents},
            "input": {"channel": channel},
            "expected": {
                "agent_id": result.agent_id,
                "is_default": result.is_default,
            },
        })

    # --- Test 3: peer_kind binding ---
    config = FakeConfig(agents={
        "default": "main",
        "bindings": [
            {"agentId": "group-handler", "match": {"channel": "telegram", "peer": {"kind": "group"}}},
        ],
    })
    resolver = RouteResolver(config)

    for peer_kind, expected_agent, expected_default in [
        ("group", "group-handler", False),
        ("dm", "main", True),
    ]:
        result = resolver.resolve(channel="telegram", peer_kind=peer_kind)
        cases.append({
            "name": f"test_peer_kind_{peer_kind}",
            "config": {"agents": config.agents},
            "input": {"channel": "telegram", "peer_kind": peer_kind},
            "expected": {
                "agent_id": result.agent_id,
                "is_default": result.is_default,
            },
        })

    # --- Test 4: peer_id exact match ---
    config = FakeConfig(agents={
        "default": "main",
        "bindings": [
            {"agentId": "vip-handler", "match": {"channel": "telegram", "peer": {"id": "user_vip"}}},
        ],
    })
    resolver = RouteResolver(config)

    for peer_id, expected_agent, expected_default in [
        ("user_vip", "vip-handler", False),
        ("user_regular", "main", True),
    ]:
        result = resolver.resolve(channel="telegram", peer_id=peer_id)
        cases.append({
            "name": f"test_peer_id_{peer_id}",
            "config": {"agents": config.agents},
            "input": {"channel": "telegram", "peer_id": peer_id},
            "expected": {
                "agent_id": result.agent_id,
                "is_default": result.is_default,
            },
        })

    # --- Test 5: peer_pattern match ---
    config = FakeConfig(agents={
        "default": "main",
        "bindings": [
            {"agentId": "admin-handler", "match": {"channel": "telegram", "peer": {"pattern": "admin_.*"}}},
        ],
    })
    resolver = RouteResolver(config)

    for peer_id, expected_agent, expected_default in [
        ("admin_alice", "admin-handler", False),
        ("admin_bob", "admin-handler", False),
        ("user_charlie", "main", True),
    ]:
        result = resolver.resolve(channel="telegram", peer_id=peer_id)
        cases.append({
            "name": f"test_peer_pattern_{peer_id}",
            "config": {"agents": config.agents},
            "input": {"channel": "telegram", "peer_id": peer_id},
            "expected": {
                "agent_id": result.agent_id,
                "is_default": result.is_default,
            },
        })

    # --- Test 6: priority ordering ---
    config = FakeConfig(agents={
        "default": "main",
        "bindings": [
            {"agentId": "low-prio", "match": {"channel": "telegram"}, "priority": 10},
            {"agentId": "high-prio", "match": {"channel": "telegram"}, "priority": 100},
        ],
    })
    resolver = RouteResolver(config)
    result = resolver.resolve(channel="telegram")
    cases.append({
        "name": "test_priority_ordering",
        "config": {"agents": config.agents},
        "input": {"channel": "telegram"},
        "expected": {
            "agent_id": result.agent_id,
            "is_default": result.is_default,
        },
    })

    # --- Test 7: account_id matching ---
    config = FakeConfig(agents={
        "default": "main",
        "bindings": [
            {"agentId": "account-bot", "match": {"channel": "telegram", "accountId": "bot_123"}},
        ],
    })
    resolver = RouteResolver(config)

    for account_id, expected_agent, expected_default in [
        ("bot_123", "account-bot", False),
        ("bot_456", "main", True),
    ]:
        result = resolver.resolve(channel="telegram", account_id=account_id)
        cases.append({
            "name": f"test_account_id_{account_id}",
            "config": {"agents": config.agents},
            "input": {"channel": "telegram", "account_id": account_id},
            "expected": {
                "agent_id": result.agent_id,
                "is_default": result.is_default,
            },
        })

    # --- Test 8: wildcard account ---
    config = FakeConfig(agents={
        "default": "main",
        "bindings": [
            {"agentId": "any-account-bot", "match": {"channel": "telegram", "accountId": "*"}},
        ],
    })
    resolver = RouteResolver(config)
    result = resolver.resolve(channel="telegram", account_id="any_account")
    cases.append({
        "name": "test_wildcard_account",
        "config": {"agents": config.agents},
        "input": {"channel": "telegram", "account_id": "any_account"},
        "expected": {
            "agent_id": result.agent_id,
            "is_default": result.is_default,
        },
    })

    write_fixture("route_resolver_fixtures.json", {
        "module": "route_resolver",
        "generated_at": GENERATED_AT,
        "test_cases": cases,
    })


# ============================================================================
# Main
# ============================================================================
def main():
    print(f"Generating fixtures at {GENERATED_AT}")
    print(f"Output directory: {FIXTURES_DIR}")
    print()

    generate_keys_fixtures()
    generate_label_fixtures()
    generate_route_resolver_fixtures()

    print()
    print("All fixtures generated successfully!")


if __name__ == "__main__":
    main()
