use super::*;

#[test]
fn test_bridge_error_display() {
    let err = BridgeError::ProviderNotFound("foo".into());
    assert_eq!(err.to_string(), "provider not found: foo");
}

#[test]
fn test_bridge_error_variants() {
    let cases = vec![
        (BridgeError::ProviderNotReady, "provider not ready"),
        (BridgeError::Timeout, "timeout"),
        (BridgeError::Internal("oops".into()), "internal error: oops"),
        (
            BridgeError::Overloaded("busy".into()),
            "bridge overloaded: busy",
        ),
        (BridgeError::RunFailed("bad".into()), "run failed: bad"),
        (
            BridgeError::SessionParseUnsupportedBlock("document".into()),
            "session parse unsupported block: document",
        ),
        (
            BridgeError::SessionStore("unreadable".into()),
            "session store error: unreadable",
        ),
        (
            BridgeError::SessionError("gone".into()),
            "session error: gone",
        ),
    ];
    for (err, expected) in cases {
        assert_eq!(err.to_string(), expected);
    }
}
