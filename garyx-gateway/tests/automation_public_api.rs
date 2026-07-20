//! External-crate compile probe for the automation domain's public surface.
//!
//! Internal unit tests cannot see the crate privacy boundary, so a pure-motion
//! refactor breaks the public API silently unless an external crate imports
//! it. This file pins the paths external consumers use today: the crate-root
//! `CronService` re-export, the automation hub re-exports, and the engine /
//! debug-api module paths.

#[allow(unused_imports)]
use garyx_gateway::CronService as RootCronService;

#[allow(unused_imports)]
use garyx_gateway::automation::{
    CronJob, CronService, JobRunStatus, RunRecord, automation_activity, automation_threads,
    create_automation, delete_automation, get_automation, list_automations, run_automation_now,
    update_automation,
};

#[allow(unused_imports)]
use garyx_gateway::automation::debug_api::{
    cron_jobs, cron_runs, debug_run_system_cron_job, debug_system_cron_jobs,
};

#[allow(unused_imports)]
use garyx_gateway::automation::engine::{
    CronJob as EngineCronJob, CronService as EngineCronService, JobRunStatus as EngineJobRunStatus,
    RunRecord as EngineRunRecord,
};

// The routes module re-export keeps the historical handler paths alive for
// the route graph.
#[allow(unused_imports)]
use garyx_gateway::routes::{cron_jobs as routes_cron_jobs, cron_runs as routes_cron_runs};

#[test]
fn automation_public_surface_compiles() {}
