use serde::Serialize;
use std::time::Instant;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheckResult {
    pub name: String,
    pub status: HealthStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthReport {
    pub status: HealthStatus,
    pub timestamp: String,
    pub checks: Vec<HealthCheckResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<serde_json::Value>,
}

pub struct HealthChecker {
    start_time: Instant,
}

impl HealthChecker {
    pub fn new(start_time: Instant) -> Self {
        Self { start_time }
    }

    pub async fn run_checks(&self) -> HealthReport {
        let mut checks = Vec::new();
        let mut overall = HealthStatus::Healthy;

        // Tokio runtime check
        let start = Instant::now();
        tokio::task::yield_now().await;
        let latency = start.elapsed().as_secs_f64() * 1000.0;

        let runtime_status = if latency < 100.0 {
            HealthStatus::Healthy
        } else {
            overall = HealthStatus::Degraded;
            HealthStatus::Degraded
        };

        checks.push(HealthCheckResult {
            name: "tokio_runtime".to_string(),
            status: runtime_status,
            message: None,
            latency_ms: Some(latency),
        });

        // Uptime check
        let uptime_secs = self.start_time.elapsed().as_secs();
        checks.push(HealthCheckResult {
            name: "uptime".to_string(),
            status: HealthStatus::Healthy,
            message: Some(format!("{}s", uptime_secs)),
            latency_ms: None,
        });

        let env = serde_json::json!({
            "platform": {
                "os": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
            },
            "process": {
                "pid": std::process::id(),
            },
        });

        HealthReport {
            status: overall,
            timestamp: chrono::Utc::now().to_rfc3339(),
            checks,
            environment: Some(env),
        }
    }
}
