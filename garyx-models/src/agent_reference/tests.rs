use super::*;
use crate::{ProviderType, builtin_provider_agent_profiles};

#[test]
fn resolves_standalone_agent() {
    let agents = builtin_provider_agent_profiles();
    let reference = resolve_agent_reference("codex", &agents).expect("codex agent");

    assert_eq!(reference.requested_id(), "codex");
    assert_eq!(reference.bound_agent_id(), "codex");
    assert_eq!(reference.provider_type(), ProviderType::CodexAppServer);
}

#[test]
fn rejects_unknown_agent() {
    let error = resolve_agent_reference("missing", &builtin_provider_agent_profiles())
        .expect_err("unknown agent must fail");

    assert_eq!(error, "unknown agent_id: missing");
}

#[test]
fn runtime_metadata_contains_standalone_identity() {
    let agents = builtin_provider_agent_profiles();
    let reference = resolve_agent_reference("claude", &agents).expect("claude agent");
    let metadata = agent_runtime_metadata(&reference);

    assert_eq!(metadata["agent_id"], "claude");
    assert_eq!(metadata["requested_provider_type"], "claude_code");
}
