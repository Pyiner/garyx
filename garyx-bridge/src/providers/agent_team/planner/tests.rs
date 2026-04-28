use super::*;

fn team(leader: &str, members: &[&str]) -> AgentTeamProfile {
    AgentTeamProfile {
        team_id: "team::demo".to_string(),
        display_name: "Demo Team".to_string(),
        leader_agent_id: leader.to_string(),
        member_agent_ids: members.iter().map(|s| s.to_string()).collect(),
        workflow_text: String::new(),
        created_at: "2026-04-19T00:00:00Z".to_string(),
        updated_at: "2026-04-19T00:00:00Z".to_string(),
    }
}

// ---------- parse_mentions ----------

#[test]
fn parse_mentions_extracts_single_mention() {
    let m = parse_mentions("@[Coder](coder) please implement it");
    assert_eq!(m, vec!["coder".to_string()]);
}

#[test]
fn parse_mentions_extracts_multiple_in_document_order() {
    let m = parse_mentions("@[Planner](planner) and @[Coder](coder) collab");
    assert_eq!(m, vec!["planner".to_string(), "coder".to_string()]);
}

#[test]
fn parse_mentions_extracts_adjacent_mentions() {
    let m = parse_mentions("@[A](a)@[B](b)");
    assert_eq!(m, vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn parse_mentions_ignores_malformed_no_paren() {
    // `@[NoId]` with no `(id)` is malformed and should be skipped.
    let m = parse_mentions("@[NoId] hi");
    assert!(m.is_empty(), "expected no mentions, got {m:?}");
}

#[test]
fn parse_mentions_ignores_unterminated_paren() {
    let m = parse_mentions("@[A](");
    assert!(m.is_empty(), "expected no mentions, got {m:?}");
}

#[test]
fn parse_mentions_trims_whitespace_in_id() {
    let m = parse_mentions("@[A]( coder )");
    assert_eq!(m, vec!["coder".to_string()]);
}

#[test]
fn parse_mentions_accepts_empty_display_name() {
    let m = parse_mentions("@[](coder)");
    assert_eq!(m, vec!["coder".to_string()]);
}

#[test]
fn parse_mentions_preserves_duplicates() {
    // plan_turn dedupes; parse_mentions must not.
    let m = parse_mentions("@[Coder](coder) @[Coder](coder) again");
    assert_eq!(m, vec!["coder".to_string(), "coder".to_string()]);
}

// ---------- plan_turn ----------

#[test]
fn plan_turn_empty_message_defaults_to_leader() {
    let t = team("leader", &["coder", "planner"]);
    let plan = plan_turn("", &t);
    assert_eq!(plan.targets, vec!["leader".to_string()]);
    assert!(!plan.from_explicit_mentions);
    assert!(plan.unknown_mentions.is_empty());
}

#[test]
fn plan_turn_plain_text_defaults_to_leader() {
    let t = team("leader", &["coder", "planner"]);
    let plan = plan_turn("hi team", &t);
    assert_eq!(plan.targets, vec!["leader".to_string()]);
    assert!(!plan.from_explicit_mentions);
    assert!(plan.unknown_mentions.is_empty());
}

#[test]
fn plan_turn_single_mention_routes_to_that_agent() {
    let t = team("leader", &["coder", "planner"]);
    let plan = plan_turn("@[Coder](coder) please implement it", &t);
    assert_eq!(plan.targets, vec!["coder".to_string()]);
    assert!(plan.from_explicit_mentions);
    assert!(plan.unknown_mentions.is_empty());
}

#[test]
fn plan_turn_multi_mention_fans_out_in_order() {
    let t = team("leader", &["coder", "planner"]);
    let plan = plan_turn("@[Planner](planner) and @[Coder](coder) collab", &t);
    assert_eq!(
        plan.targets,
        vec!["planner".to_string(), "coder".to_string()]
    );
    assert!(plan.from_explicit_mentions);
    assert!(plan.unknown_mentions.is_empty());
}

#[test]
fn plan_turn_dedupes_repeated_valid_mentions() {
    let t = team("leader", &["coder", "planner"]);
    let plan = plan_turn("@[Coder](coder) @[Coder](coder) again", &t);
    assert_eq!(plan.targets, vec!["coder".to_string()]);
    assert!(plan.from_explicit_mentions);
    assert!(plan.unknown_mentions.is_empty());
}

#[test]
fn plan_turn_unknown_only_falls_back_to_leader_and_records_unknown() {
    let t = team("leader", &["coder", "planner"]);
    let plan = plan_turn("@[Who](who)", &t);
    assert_eq!(plan.targets, vec!["leader".to_string()]);
    assert!(!plan.from_explicit_mentions);
    assert_eq!(plan.unknown_mentions, vec!["who".to_string()]);
}

#[test]
fn plan_turn_mixed_valid_and_unknown_mentions() {
    let t = team("leader", &["coder", "planner"]);
    let plan = plan_turn("@[Coder](coder) @[Who](who)", &t);
    assert_eq!(plan.targets, vec!["coder".to_string()]);
    assert!(plan.from_explicit_mentions);
    assert_eq!(plan.unknown_mentions, vec!["who".to_string()]);
}

#[test]
fn plan_turn_leader_can_be_mentioned_explicitly() {
    let t = team("leader", &["coder", "planner"]);
    let plan = plan_turn("@[Leader](leader) please coordinate", &t);
    assert_eq!(plan.targets, vec!["leader".to_string()]);
    assert!(plan.from_explicit_mentions);
    assert!(plan.unknown_mentions.is_empty());
}

#[test]
fn plan_turn_adjacent_mentions_are_both_targeted() {
    let t = team("leader", &["a", "b"]);
    let plan = plan_turn("@[A](a)@[B](b)", &t);
    assert_eq!(plan.targets, vec!["a".to_string(), "b".to_string()]);
    assert!(plan.from_explicit_mentions);
    assert!(plan.unknown_mentions.is_empty());
}

#[test]
fn plan_turn_malformed_mention_is_plain_text() {
    let t = team("leader", &["coder"]);
    let plan = plan_turn("@[NoId] hi", &t);
    assert_eq!(plan.targets, vec!["leader".to_string()]);
    assert!(!plan.from_explicit_mentions);
    assert!(plan.unknown_mentions.is_empty());
}

#[test]
fn plan_turn_whitespace_in_agent_id_is_trimmed() {
    let t = team("leader", &["coder"]);
    let plan = plan_turn("@[A]( coder )", &t);
    assert_eq!(plan.targets, vec!["coder".to_string()]);
    assert!(plan.from_explicit_mentions);
    assert!(plan.unknown_mentions.is_empty());
}

#[test]
fn plan_turn_empty_display_name_still_routes() {
    let t = team("leader", &["coder"]);
    let plan = plan_turn("@[](coder)", &t);
    assert_eq!(plan.targets, vec!["coder".to_string()]);
    assert!(plan.from_explicit_mentions);
    assert!(plan.unknown_mentions.is_empty());
}

#[test]
fn plan_turn_leader_not_implicitly_in_members_still_valid_when_mentioned() {
    // Leader is NOT listed in member_agent_ids, but the design says the
    // valid set is `union(member_agent_ids, {leader_agent_id})`.
    let t = team("solo-leader", &["coder"]);
    let plan = plan_turn("@[Boss](solo-leader)", &t);
    assert_eq!(plan.targets, vec!["solo-leader".to_string()]);
    assert!(plan.from_explicit_mentions);
    assert!(plan.unknown_mentions.is_empty());
}
