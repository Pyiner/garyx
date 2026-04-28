use super::*;

#[test]
fn config_method_roundtrips() {
    let j = serde_json::to_value(ConfigMethod::Form).unwrap();
    assert_eq!(j, serde_json::json!({"kind": "form"}));
    let j = serde_json::to_value(ConfigMethod::AutoLogin).unwrap();
    assert_eq!(j, serde_json::json!({"kind": "auto_login"}));

    // Unknown variants on the wire deserialise as `Unknown`
    // instead of failing, so a newer plugin can list a method
    // an older host doesn't understand without breaking the
    // whole catalog fetch.
    let future: ConfigMethod =
        serde_json::from_value(serde_json::json!({"kind": "sso_callback"})).unwrap();
    assert_eq!(future, ConfigMethod::Unknown);
}

#[test]
fn display_item_roundtrips() {
    let text = AuthDisplayItem::text("hello");
    let j = serde_json::to_value(&text).unwrap();
    assert_eq!(j, serde_json::json!({"kind": "text", "value": "hello"}));

    let qr = AuthDisplayItem::qr("opaque-token");
    let j = serde_json::to_value(&qr).unwrap();
    assert_eq!(
        j,
        serde_json::json!({"kind": "qr", "value": "opaque-token"})
    );

    let future: AuthDisplayItem =
        serde_json::from_value(serde_json::json!({"kind": "image", "src": "..."})).unwrap();
    assert_eq!(future, AuthDisplayItem::Unknown);
}

#[test]
fn poll_result_roundtrips() {
    let pending_bare: AuthPollResult = AuthPollResult::Pending {
        display: None,
        next_interval_secs: None,
    };
    let j = serde_json::to_value(&pending_bare).unwrap();
    assert_eq!(j["status"], "pending");
    assert!(j.get("display").is_none());
    assert!(j.get("next_interval_secs").is_none());

    let pending_refresh = AuthPollResult::Pending {
        display: Some(vec![AuthDisplayItem::text("已扫码，请在微信内确认")]),
        next_interval_secs: Some(5),
    };
    let j = serde_json::to_value(&pending_refresh).unwrap();
    assert_eq!(j["status"], "pending");
    assert_eq!(j["display"][0]["kind"], "text");
    assert_eq!(j["next_interval_secs"], 5);

    let confirmed = AuthPollResult::Confirmed {
        values: BTreeMap::from_iter([("token".into(), Value::String("secret".into()))]),
    };
    let j = serde_json::to_value(&confirmed).unwrap();
    assert_eq!(j["status"], "confirmed");
    assert_eq!(j["values"]["token"], "secret");

    let failed = AuthPollResult::Failed {
        reason: "expired".into(),
    };
    let j = serde_json::to_value(&failed).unwrap();
    assert_eq!(j["status"], "failed");
    assert_eq!(j["reason"], "expired");
}
