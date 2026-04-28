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
            BridgeError::SessionError("gone".into()),
            "session error: gone",
        ),
    ];
    for (err, expected) in cases {
        assert_eq!(err.to_string(), expected);
    }
}

// -- ProviderHealth tests --

#[test]
fn test_health_new_is_healthy() {
    let h = ProviderHealth::new("test-provider");
    assert_eq!(h.status, HealthStatus::Healthy);
    assert_eq!(h.total_runs, 0);
    assert_eq!(h.consecutive_failures, 0);
    assert!((h.success_rate() - 1.0).abs() < f64::EPSILON);
}

#[test]
fn test_health_record_success() {
    let mut h = ProviderHealth::new("test");
    h.record_success(100.0);
    assert_eq!(h.total_runs, 1);
    assert_eq!(h.successful_runs, 1);
    assert_eq!(h.status, HealthStatus::Healthy);
    assert!((h.avg_latency_ms - 100.0).abs() < f64::EPSILON);
}

#[test]
fn test_health_degraded_after_2_failures() {
    let mut h = ProviderHealth::new("test");
    h.record_success(50.0);
    h.record_failure("err1");
    assert_eq!(h.status, HealthStatus::Healthy);
    h.record_failure("err2");
    assert_eq!(h.status, HealthStatus::Degraded);
    assert_eq!(h.consecutive_failures, 2);
}

#[test]
fn test_health_unavailable_after_5_failures() {
    let mut h = ProviderHealth::new("test");
    for i in 0..5 {
        h.record_failure(&format!("err{i}"));
    }
    assert_eq!(h.status, HealthStatus::Unavailable);
    assert_eq!(h.consecutive_failures, 5);
}

#[test]
fn test_health_recovery_after_success() {
    let mut h = ProviderHealth::new("test");
    h.record_failure("err1");
    h.record_failure("err2");
    assert_eq!(h.status, HealthStatus::Degraded);

    h.record_success(80.0);
    assert_eq!(h.consecutive_failures, 0);
    // Still degraded due to low success rate (1/3 = 0.33)
    assert_eq!(h.status, HealthStatus::Degraded);

    // More successes bring it back
    h.record_success(80.0);
    h.record_success(80.0);
    assert_eq!(h.status, HealthStatus::Healthy);
}

#[test]
fn test_health_success_rate() {
    let mut h = ProviderHealth::new("test");
    h.record_success(100.0);
    h.record_success(100.0);
    h.record_failure("err");
    assert!((h.success_rate() - 2.0 / 3.0).abs() < 0.01);
}

#[test]
fn test_health_latency_ema() {
    let mut h = ProviderHealth::new("test");
    h.record_success(100.0);
    assert!((h.avg_latency_ms - 100.0).abs() < f64::EPSILON);
    h.record_success(200.0);
    // EMA: 0.7 * 100 + 0.3 * 200 = 130
    assert!((h.avg_latency_ms - 130.0).abs() < f64::EPSILON);
}
