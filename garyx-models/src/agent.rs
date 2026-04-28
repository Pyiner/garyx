use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// State of an agent run.
///
/// All variants are part of the public serde contract and may arrive via JSON
/// deserialization even if Rust code does not construct them directly.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Pending,
    Running,
    Streaming,
    Completed,
    /// Run was aborted externally (e.g. user cancellation). Currently only
    /// reached via deserialized state; no Rust code path sets this today.
    Aborted,
    Error,
}

#[cfg(test)]
mod tests;
