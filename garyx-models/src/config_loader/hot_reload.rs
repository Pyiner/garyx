use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::config::GaryxConfig;

use super::diagnostics::ConfigDiagnostics;
use super::load::{ConfigLoadOptions, load_config};

type ReloadCallback = Arc<dyn Fn(GaryxConfig, ConfigDiagnostics) + Send + Sync + 'static>;

#[derive(Debug, Clone)]
pub struct ConfigHotReloadOptions {
    pub poll_interval: Duration,
    pub debounce: Duration,
    pub load_options: ConfigLoadOptions,
}

impl Default for ConfigHotReloadOptions {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_millis(500),
            debounce: Duration::from_millis(400),
            load_options: ConfigLoadOptions::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigReloadMetricsSnapshot {
    pub attempts: u64,
    pub successes: u64,
    pub failures: u64,
    pub callback_notifications: u64,
}

#[derive(Debug)]
struct ConfigReloadMetrics {
    attempts: AtomicU64,
    successes: AtomicU64,
    failures: AtomicU64,
    callback_notifications: AtomicU64,
}

impl Default for ConfigReloadMetrics {
    fn default() -> Self {
        Self {
            attempts: AtomicU64::new(0),
            successes: AtomicU64::new(0),
            failures: AtomicU64::new(0),
            callback_notifications: AtomicU64::new(0),
        }
    }
}

impl ConfigReloadMetrics {
    fn snapshot(&self) -> ConfigReloadMetricsSnapshot {
        ConfigReloadMetricsSnapshot {
            attempts: self.attempts.load(Ordering::Relaxed),
            successes: self.successes.load(Ordering::Relaxed),
            failures: self.failures.load(Ordering::Relaxed),
            callback_notifications: self.callback_notifications.load(Ordering::Relaxed),
        }
    }
}

pub struct ConfigHotReloader {
    callbacks: Arc<RwLock<Vec<ReloadCallback>>>,
    metrics: Arc<ConfigReloadMetrics>,
    stop: Arc<AtomicBool>,
    join_handle: Mutex<Option<thread::JoinHandle<()>>>,
}

impl ConfigHotReloader {
    pub fn start(
        path: PathBuf,
        initial_config: GaryxConfig,
        options: ConfigHotReloadOptions,
    ) -> Self {
        let callbacks: Arc<RwLock<Vec<ReloadCallback>>> = Arc::new(RwLock::new(Vec::new()));
        let metrics = Arc::new(ConfigReloadMetrics::default());
        let stop = Arc::new(AtomicBool::new(false));

        let callbacks_bg = callbacks.clone();
        let metrics_bg = metrics.clone();
        let stop_bg = stop.clone();
        let load_options = options.load_options.clone();

        drop(initial_config); // no longer retained for fallback
        let join_handle = thread::spawn(move || {
            let mut last_seen_mtime = file_mtime(&path);
            let mut pending_until: Option<Instant> = None;

            while !stop_bg.load(Ordering::Relaxed) {
                let now_mtime = file_mtime(&path);
                if now_mtime != last_seen_mtime {
                    last_seen_mtime = now_mtime;
                    pending_until = Some(Instant::now() + options.debounce);
                }

                if let Some(deadline) = pending_until {
                    if Instant::now() >= deadline {
                        metrics_bg.attempts.fetch_add(1, Ordering::Relaxed);
                        match load_config(&path, &load_options) {
                            Ok(loaded) => {
                                metrics_bg.successes.fetch_add(1, Ordering::Relaxed);
                                let hooks =
                                    callbacks_bg.read().map(|g| g.clone()).unwrap_or_default();
                                for cb in hooks {
                                    cb(loaded.config.clone(), loaded.diagnostics.clone());
                                    metrics_bg
                                        .callback_notifications
                                        .fetch_add(1, Ordering::Relaxed);
                                }
                            }
                            Err(_err) => {
                                metrics_bg.failures.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        pending_until = None;
                    }
                }

                thread::sleep(options.poll_interval);
            }
        });

        Self {
            callbacks,
            metrics,
            stop,
            join_handle: Mutex::new(Some(join_handle)),
        }
    }

    pub fn register_callback<F>(&self, callback: F)
    where
        F: Fn(GaryxConfig, ConfigDiagnostics) + Send + Sync + 'static,
    {
        if let Ok(mut hooks) = self.callbacks.write() {
            hooks.push(Arc::new(callback));
        }
    }

    pub fn metrics(&self) -> ConfigReloadMetricsSnapshot {
        self.metrics.snapshot()
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut handle_guard) = self.join_handle.lock() {
            if let Some(handle) = handle_guard.take() {
                let _ = handle.join();
            }
        }
    }
}

impl Drop for ConfigHotReloader {
    fn drop(&mut self) {
        self.stop();
    }
}

fn file_mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}
