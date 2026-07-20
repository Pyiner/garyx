//! The automation domain: user-facing scheduled automations, the cron engine
//! that drives them, and the debug/data HTTP surface.
//!
//! One domain, five layers:
//! - [`engine`]: job model (`CronJob`/`RunRecord`), persistence, schedule
//!   math, the scheduler loop, and job execution against an injected
//!   execution environment. Engine logging routes through the
//!   `engine/log.rs` wrapper macros, which pin the stable
//!   `garyx_gateway::cron` target structurally so operator `RUST_LOG`
//!   filters and alert rules survive module moves.
//! - [`dispatch`]: the execution-time contracts ([`dispatch::AutomationExecEnv`],
//!   [`dispatch::AutomationDispatchPort`]) implemented at the composition
//!   seam (`crate::composition::automation_wiring`) — the engine has no
//!   `AppState` knowledge.
//! - [`http`]: the `/api/automations` product surface mapping Automation
//!   request/summary shapes onto engine jobs.
//! - [`mapping`]: the schedule translation between the product
//!   `AutomationScheduleView` and the engine's persisted `CronSchedule`.
//! - [`debug_api`]: cron-data listing and system-job debug endpoints.

pub mod debug_api;
pub(crate) mod dispatch;
pub mod engine;
pub mod http;
pub(crate) mod mapping;

pub use engine::{CronJob, CronService, JobRunStatus, RunRecord};
pub use http::{
    automation_activity, automation_threads, create_automation, delete_automation, get_automation,
    list_automations, run_automation_now, update_automation,
};
