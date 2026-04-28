use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use garyx_models::routing::DeliveryContext;
use garyx_router::MessageRouter;
use serde::Serialize;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Default)]
pub struct DeliveryTargetDurationStat {
    pub count: u64,
    pub min_ms: u64,
    pub max_ms: u64,
    pub avg_ms: f64,
    pub total_ms: u128,
    pub p50_ms: u64,
    pub p95_ms: u64,
    pub p99_ms: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct DeliveryTargetByTargetStat {
    pub last_target: u64,
    pub thread_target: u64,
    pub explicit_target: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct DeliveryTargetDimensionCount {
    pub key: String,
    pub value: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct DeliveryTargetMetricsSnapshot {
    pub cache_hits: u64,
    pub store_hits: u64,
    pub store_misses: u64,
    pub by_target: DeliveryTargetByTargetStat,
    pub by_channel: Vec<DeliveryTargetDimensionCount>,
    pub by_account: Vec<DeliveryTargetDimensionCount>,
    pub recovery_duration_ms: DeliveryTargetDurationStat,
}

#[derive(Default)]
struct DurationAccumulator {
    count: u64,
    min_ms: u64,
    max_ms: u64,
    total_ms: u128,
    samples: VecDeque<u64>,
}

impl DurationAccumulator {
    const MAX_SAMPLES: usize = 4096;

    const fn new() -> Self {
        Self {
            count: 0,
            min_ms: 0,
            max_ms: 0,
            total_ms: 0,
            samples: VecDeque::new(),
        }
    }

    fn record(&mut self, duration_ms: u64) {
        if self.count == 0 {
            self.min_ms = duration_ms;
            self.max_ms = duration_ms;
        } else {
            self.min_ms = self.min_ms.min(duration_ms);
            self.max_ms = self.max_ms.max(duration_ms);
        }
        self.count += 1;
        self.total_ms += duration_ms as u128;
        self.samples.push_back(duration_ms);
        if self.samples.len() > Self::MAX_SAMPLES {
            self.samples.pop_front();
        }
    }

    fn percentile(sorted: &[u64], p: f64) -> u64 {
        if sorted.is_empty() {
            return 0;
        }
        let clamped = p.clamp(0.0, 1.0);
        let idx = ((sorted.len() as f64 - 1.0) * clamped).round() as usize;
        sorted[idx]
    }

    fn snapshot(&self) -> DeliveryTargetDurationStat {
        let avg_ms = if self.count == 0 {
            0.0
        } else {
            self.total_ms as f64 / self.count as f64
        };
        let mut samples = self.samples.iter().copied().collect::<Vec<u64>>();
        samples.sort_unstable();
        let p50_ms = Self::percentile(&samples, 0.50);
        let p95_ms = Self::percentile(&samples, 0.95);
        let p99_ms = Self::percentile(&samples, 0.99);
        DeliveryTargetDurationStat {
            count: self.count,
            min_ms: self.min_ms,
            max_ms: self.max_ms,
            avg_ms,
            total_ms: self.total_ms,
            p50_ms,
            p95_ms,
            p99_ms,
        }
    }
}

struct DeliveryTargetMetrics {
    cache_hits: AtomicU64,
    store_hits: AtomicU64,
    store_misses: AtomicU64,
    target_last: AtomicU64,
    target_thread: AtomicU64,
    target_explicit: AtomicU64,
    channel_counts: std::sync::Mutex<Vec<(String, u64)>>,
    account_counts: std::sync::Mutex<Vec<(String, u64)>>,
    duration: std::sync::Mutex<DurationAccumulator>,
}

impl DeliveryTargetMetrics {
    const fn new() -> Self {
        Self {
            cache_hits: AtomicU64::new(0),
            store_hits: AtomicU64::new(0),
            store_misses: AtomicU64::new(0),
            target_last: AtomicU64::new(0),
            target_thread: AtomicU64::new(0),
            target_explicit: AtomicU64::new(0),
            channel_counts: std::sync::Mutex::new(Vec::new()),
            account_counts: std::sync::Mutex::new(Vec::new()),
            duration: std::sync::Mutex::new(DurationAccumulator::new()),
        }
    }

    fn bump_dimension(counter: &mut Vec<(String, u64)>, key: &str) {
        if let Some((_, value)) = counter.iter_mut().find(|(k, _)| k == key) {
            *value += 1;
            return;
        }
        counter.push((key.to_owned(), 1));
    }

    fn record_delivery_dimensions(&self, delivery: &DeliveryContext) {
        if let Ok(mut channels) = self.channel_counts.lock() {
            Self::bump_dimension(&mut channels, &delivery.channel);
        }
        if let Ok(mut accounts) = self.account_counts.lock() {
            Self::bump_dimension(&mut accounts, &delivery.account_id);
        }
    }

    fn snapshot_dimension(
        counter: &std::sync::Mutex<Vec<(String, u64)>>,
    ) -> Vec<DeliveryTargetDimensionCount> {
        let mut out = if let Ok(entries) = counter.lock() {
            entries
                .iter()
                .map(|(k, v)| DeliveryTargetDimensionCount {
                    key: k.clone(),
                    value: *v,
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        out.sort_by(|a, b| b.value.cmp(&a.value).then_with(|| a.key.cmp(&b.key)));
        out
    }

    fn record_duration(&self, duration_ms: u64) {
        if let Ok(mut guard) = self.duration.lock() {
            guard.record(duration_ms);
        }
    }

    fn snapshot(&self) -> DeliveryTargetMetricsSnapshot {
        let recovery_duration_ms = if let Ok(guard) = self.duration.lock() {
            guard.snapshot()
        } else {
            DeliveryTargetDurationStat::default()
        };
        DeliveryTargetMetricsSnapshot {
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            store_hits: self.store_hits.load(Ordering::Relaxed),
            store_misses: self.store_misses.load(Ordering::Relaxed),
            by_target: DeliveryTargetByTargetStat {
                last_target: self.target_last.load(Ordering::Relaxed),
                thread_target: self.target_thread.load(Ordering::Relaxed),
                explicit_target: self.target_explicit.load(Ordering::Relaxed),
            },
            by_channel: Self::snapshot_dimension(&self.channel_counts),
            by_account: Self::snapshot_dimension(&self.account_counts),
            recovery_duration_ms,
        }
    }
}

static DELIVERY_TARGET_METRICS: DeliveryTargetMetrics = DeliveryTargetMetrics::new();

pub fn metrics_snapshot() -> DeliveryTargetMetricsSnapshot {
    DELIVERY_TARGET_METRICS.snapshot()
}

/// Resolve delivery target with minimized router lock hold time.
///
/// Flow:
/// 1. Try in-memory cache under router lock.
/// 2. On miss, clone thread store handle under lock.
/// 3. Perform store-based recovery outside lock.
/// 4. Re-acquire lock only to backfill in-memory cache.
pub async fn resolve_delivery_target_with_recovery(
    router: &Arc<Mutex<MessageRouter>>,
    target: &str,
) -> Option<(String, DeliveryContext)> {
    let normalized_target = normalize_target(target);
    let started = Instant::now();
    match classify_target(normalized_target) {
        TargetKind::Last => {
            DELIVERY_TARGET_METRICS
                .target_last
                .fetch_add(1, Ordering::Relaxed);
        }
        TargetKind::ThreadLike => {
            DELIVERY_TARGET_METRICS
                .target_thread
                .fetch_add(1, Ordering::Relaxed);
        }
        TargetKind::Explicit => {
            DELIVERY_TARGET_METRICS
                .target_explicit
                .fetch_add(1, Ordering::Relaxed);
        }
    }
    let in_memory = {
        let router = router.lock().await;
        router.resolve_delivery_target(normalized_target)
    };
    if in_memory.is_some() {
        DELIVERY_TARGET_METRICS
            .cache_hits
            .fetch_add(1, Ordering::Relaxed);
        if let Some((_, delivery)) = &in_memory {
            DELIVERY_TARGET_METRICS.record_delivery_dimensions(delivery);
        }
        let duration_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        DELIVERY_TARGET_METRICS.record_duration(duration_ms);
        return in_memory;
    }

    let thread_store = {
        let router = router.lock().await;
        router.thread_store()
    };
    let recovered =
        MessageRouter::resolve_delivery_target_from_store(thread_store, normalized_target).await;
    if let Some((thread_id, delivery_ctx)) = &recovered {
        DELIVERY_TARGET_METRICS
            .store_hits
            .fetch_add(1, Ordering::Relaxed);
        DELIVERY_TARGET_METRICS.record_delivery_dimensions(delivery_ctx);
        let mut router = router.lock().await;
        router.set_last_delivery(thread_id, delivery_ctx.clone());
    } else {
        DELIVERY_TARGET_METRICS
            .store_misses
            .fetch_add(1, Ordering::Relaxed);
    }
    let duration_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    DELIVERY_TARGET_METRICS.record_duration(duration_ms);
    recovered
}

enum TargetKind {
    Last,
    ThreadLike,
    Explicit,
}

fn classify_target(target: &str) -> TargetKind {
    let t = target.trim();
    if t.is_empty() || t == "last" {
        return TargetKind::Last;
    }
    if t.starts_with("thread:") || t.contains("::") {
        return TargetKind::ThreadLike;
    }
    TargetKind::Explicit
}

fn normalize_target(target: &str) -> &str {
    let trimmed = target.trim();
    if trimmed.starts_with("thread::") {
        trimmed
    } else {
        trimmed.strip_prefix("thread:").unwrap_or(trimmed)
    }
}
