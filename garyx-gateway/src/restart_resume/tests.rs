use super::*;

#[test]
fn build_replay_dispatch_uses_fresh_run_id_and_preserves_origin() {
    let (run_id, metadata) = build_replay_dispatch(Some("run-123".to_owned()));
    assert!(run_id.starts_with("restart-resume-"));
    assert_eq!(metadata.get("restart_resume"), Some(&Value::Bool(true)));
    assert_eq!(
        metadata.get("restart_origin_run_id"),
        Some(&Value::String("run-123".to_owned()))
    );
}

#[test]
fn build_replay_dispatch_omits_empty_origin_run_id() {
    let (_, metadata) = build_replay_dispatch(Some("   ".to_owned()));
    assert!(metadata.get("restart_origin_run_id").is_none());
}
