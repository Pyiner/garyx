//! AgentTeam provider module.
//!
//! Implements a meta-provider that orchestrates a group chat across
//! per-sub-agent threads.

pub mod dispatcher;
pub mod planner;
pub mod provider;
pub mod store;

#[cfg(test)]
mod tests;

pub use dispatcher::{SubAgentDispatcher, TeamProfileResolver};
pub use planner::{TurnPlan, plan_turn};
pub use provider::AgentTeamProvider;
pub use store::{FileGroupStore, Group, GroupStore};
