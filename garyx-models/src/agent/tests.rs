use super::*;

#[test]
fn test_run_state_serde() {
    let s = RunState::Completed;
    let json = serde_json::to_string(&s).unwrap();
    assert_eq!(json, "\"completed\"");
    let back: RunState = serde_json::from_str(&json).unwrap();
    assert_eq!(back, RunState::Completed);
}
