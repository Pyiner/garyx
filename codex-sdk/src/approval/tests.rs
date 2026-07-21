//! Contract pins for the SDK's default auto-approve policy.
//!
//! These tests freeze the wire shape of the default approval decisions.
//! Changing them (accept → decline, dropping `forSession`, renaming a
//! field) changes the SDK's default behavior contract and must show up
//! as a red test, not slip through as an incidental edit.

use super::*;

#[test]
fn command_execution_default_accepts_for_session() {
    let v = auto_approve_command_execution();
    assert_eq!(
        v,
        serde_json::json!({
            "decision": "accept",
            "acceptSettings": { "forSession": true },
        })
    );
}

#[test]
fn file_change_default_accepts() {
    let v = auto_approve_file_change();
    assert_eq!(v, serde_json::json!({ "decision": "accept" }));
}
