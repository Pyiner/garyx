//! Turn planner for the AgentTeam provider.
//!
//! A pure function that inspects the incoming user message, parses out any
//! `@[DisplayName](agent_id)` mentions, and decides which sub-agent threads
//! should receive this turn. Invoked by the dispatch loop in `provider.rs`
//! before dispatch.
//!
//! This module performs NO I/O and has no async surface — it is meant to be
//! trivially unit-testable.

use garyx_models::AgentTeamProfile;

/// Outcome of planning a single turn against a team profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnPlan {
    /// Ordered list of target agent_ids for this turn. Never empty.
    pub targets: Vec<String>,
    /// True if targets came from explicit mentions; false if defaulted to leader.
    pub from_explicit_mentions: bool,
    /// Mentions that referenced agent_ids NOT in the team (logged; not targeted).
    pub unknown_mentions: Vec<String>,
}

/// Plan the dispatch targets for an incoming user turn.
///
/// Resolution rules:
/// - Mentions are parsed from `message` in document order.
/// - A mention is "valid" iff its agent_id is in
///   `team.member_agent_ids ∪ {team.leader_agent_id}`.
/// - Unknown mentions are collected in order (preserving input order, with
///   duplicates kept — they describe what the user typed).
/// - If there is at least one valid mention, `targets` is the deduped
///   first-appearance ordering of those valid agent_ids, and
///   `from_explicit_mentions = true`.
/// - Otherwise `targets = vec![team.leader_agent_id]` and
///   `from_explicit_mentions = false`.
pub fn plan_turn(message: &str, team: &AgentTeamProfile) -> TurnPlan {
    let mentions = parse_mentions(message);

    let mut valid_ordered: Vec<String> = Vec::new();
    let mut unknown_mentions: Vec<String> = Vec::new();

    for id in mentions {
        if is_valid_target(&id, team) {
            if !valid_ordered.iter().any(|existing| existing == &id) {
                valid_ordered.push(id);
            }
        } else {
            unknown_mentions.push(id);
        }
    }

    if valid_ordered.is_empty() {
        TurnPlan {
            targets: vec![team.leader_agent_id.clone()],
            from_explicit_mentions: false,
            unknown_mentions,
        }
    } else {
        TurnPlan {
            targets: valid_ordered,
            from_explicit_mentions: true,
            unknown_mentions,
        }
    }
}

fn is_valid_target(agent_id: &str, team: &AgentTeamProfile) -> bool {
    agent_id == team.leader_agent_id || team.member_agent_ids.iter().any(|m| m == agent_id)
}

/// Parse `@[DisplayName](agent_id)` mentions out of `text`.
///
/// - `DisplayName` is any run of non-`]` characters (possibly empty).
/// - `agent_id` is any run of non-`)` characters, surrounding whitespace trimmed.
/// - Mentions are returned in document order; duplicates are preserved — the
///   caller (see `plan_turn`) decides how to dedupe.
/// - Malformed fragments (no `]`, no `(`, no `)`, or `@[..](` followed by
///   literal text that is not the mention's closing paren) are skipped.
fn parse_mentions(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;

    while i < bytes.len() {
        // Look for the start of a mention: "@[".
        if bytes[i] != b'@' {
            i += 1;
            continue;
        }
        if i + 1 >= bytes.len() || bytes[i + 1] != b'[' {
            i += 1;
            continue;
        }

        // Find the matching ']' for the display-name segment.
        let display_start = i + 2;
        let Some(rel_close_bracket) = bytes[display_start..].iter().position(|b| *b == b']') else {
            // No closing ']' in the rest of the string; give up scanning forward.
            break;
        };
        let close_bracket = display_start + rel_close_bracket;

        // Require an immediate '(' after ']'.
        let paren_open = close_bracket + 1;
        if paren_open >= bytes.len() || bytes[paren_open] != b'(' {
            // Malformed (no "(id)" part). Skip the '@' and continue scanning.
            i += 1;
            continue;
        }

        // Find the matching ')' for the id segment.
        let id_start = paren_open + 1;
        let Some(rel_close_paren) = bytes[id_start..].iter().position(|b| *b == b')') else {
            // Unterminated paren: treat as malformed, skip '@' and continue.
            i += 1;
            continue;
        };
        let close_paren = id_start + rel_close_paren;

        // Safe to slice as &str because we only split on ASCII bracket/paren bytes,
        // which are guaranteed to be codepoint boundaries in UTF-8.
        let id_raw = &text[id_start..close_paren];
        let id_trimmed = id_raw.trim();
        if !id_trimmed.is_empty() {
            out.push(id_trimmed.to_string());
        }

        i = close_paren + 1;
    }

    out
}

#[cfg(test)]
mod tests;
