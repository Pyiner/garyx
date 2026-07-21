//! Engine logging wrappers: the only place in the engine allowed to touch
//! `tracing` directly.
//!
//! Each wrapper pins the stable `garyx_gateway::cron` target structurally, so
//! a callsite cannot omit or drift it — module moves never change what
//! operator `RUST_LOG` filters and alert rules see. The runtime
//! capturing-subscriber test in `tests.rs` asserts engine events land on this
//! stable target and none drift to module-path targets; engine code outside
//! this module must keep routing through the `cron_*` wrappers (review-owned
//! contract, see docs/agents/repository-contracts.md).

macro_rules! cron_debug {
    ($($arg:tt)*) => {
        tracing::debug!(target: "garyx_gateway::cron", $($arg)*)
    };
}

macro_rules! cron_info {
    ($($arg:tt)*) => {
        tracing::info!(target: "garyx_gateway::cron", $($arg)*)
    };
}

macro_rules! cron_warn {
    ($($arg:tt)*) => {
        tracing::warn!(target: "garyx_gateway::cron", $($arg)*)
    };
}

pub(crate) use {cron_debug, cron_info, cron_warn};
