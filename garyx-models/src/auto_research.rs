use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutoResearchRunState {
    Queued,
    Researching,
    Judging,
    BudgetExhausted,
    Blocked,
    UserStopped,
}

impl AutoResearchRunState {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::BudgetExhausted | Self::Blocked | Self::UserStopped
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutoResearchIterationState {
    Researching,
    Judging,
    Completed,
}

fn default_max_iterations() -> u32 {
    10
}

fn default_time_budget() -> u64 {
    1800
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AutoResearchRun {
    pub run_id: String,
    pub state: AutoResearchRunState,
    #[serde(default)]
    pub state_started_at: Option<String>,
    pub goal: String,
    pub workspace_dir: Option<String>,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    #[serde(default = "default_time_budget")]
    pub time_budget_secs: u64,
    #[serde(default)]
    pub iterations_used: u32,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub terminal_reason: Option<String>,
    #[serde(default)]
    pub candidates: Vec<Candidate>,
    #[serde(default)]
    pub selected_candidate: Option<String>,
    /// The thread currently being executed (work, verify, or reverify).
    /// Set explicitly by the loop; cleared when the run reaches a terminal state.
    #[serde(default)]
    pub active_thread_id: Option<String>,
}

/// Tracks the lifecycle of a single research iteration (thread IDs + timing).
/// Content and verdict live on `Candidate` — this struct only tracks execution metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AutoResearchIteration {
    pub run_id: String,
    pub iteration_index: u32,
    pub state: AutoResearchIterationState,
    #[serde(default)]
    pub work_thread_id: Option<String>,
    #[serde(default)]
    pub verify_thread_id: Option<String>,
    pub started_at: String,
    #[serde(default)]
    pub completed_at: Option<String>,
}

/// A single candidate solution produced by the worker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Candidate {
    pub candidate_id: String,
    pub iteration: u32,
    /// Worker output preview (truncated to ~2000 chars for storage efficiency).
    pub output: String,
    #[serde(default)]
    pub verdict: Option<Verdict>,
    #[serde(default)]
    pub duration_secs: u64,
}

/// Verifier evaluation: a score for ranking and free-text feedback for the worker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Verdict {
    /// 0.0–10.0 score used to rank candidates.
    pub score: f32,
    pub feedback: String,
}
