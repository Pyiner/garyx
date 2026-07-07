//! Optimistic-concurrency primitives for the agent / team stores.
//!
//! Every profile carries an `updated_at` timestamp that changes on each
//! successful write, so it doubles as a concurrency token: a client that
//! edits based on a fetched profile sends that profile's `updated_at` back,
//! and the store refuses the write when the stored value moved on. This
//! turns the previous last-write-wins upsert into strict create / strict
//! conditional-update semantics (#TASK-1761).

/// Concurrency expectation for a store write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteExpectation {
    /// Strict create: fail with [`StoreWriteError::Conflict`] when the id
    /// already exists.
    Create,
    /// Strict update: fail with [`StoreWriteError::NotFound`] when the id is
    /// missing (a deleted profile must not be resurrected), and with
    /// [`StoreWriteError::Conflict`] unless the stored `updated_at` equals
    /// this token.
    UpdatedAt(String),
    /// Unconditional write for in-process system mutations that read and
    /// mutate under one store lock (no stale-read window). HTTP handlers must
    /// not use this: client edits are always based on a previously fetched
    /// snapshot and need the token check.
    Overwrite,
}

/// Store write failure, classified so HTTP handlers can map status codes
/// (400 / 404 / 409 / 500) instead of collapsing everything to 400.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreWriteError {
    /// The request itself is invalid (missing fields, built-in target, …).
    Invalid(String),
    /// Strict update addressed an id that does not exist.
    NotFound(String),
    /// The concurrency expectation failed: create hit an existing id, or the
    /// stored `updated_at` no longer matches the client's token.
    Conflict {
        message: String,
        /// The stored `updated_at` at rejection time, when one exists, so the
        /// client can re-read (or retry directly against it when it is sure).
        current_updated_at: Option<String>,
    },
    /// The mutation applied in memory but persisting it failed.
    Persist(String),
}

impl StoreWriteError {
    pub fn message(&self) -> &str {
        match self {
            StoreWriteError::Invalid(message)
            | StoreWriteError::NotFound(message)
            | StoreWriteError::Persist(message) => message,
            StoreWriteError::Conflict { message, .. } => message,
        }
    }
}

/// Check `expectation` against the currently stored `updated_at` (if any)
/// while the caller holds the store's write lock. `what` names the entity for
/// error messages ("custom agent" / "agent team").
pub fn check_write_expectation(
    expectation: &WriteExpectation,
    stored_updated_at: Option<&str>,
    what: &str,
) -> Result<(), StoreWriteError> {
    match expectation {
        WriteExpectation::Create => match stored_updated_at {
            Some(_) => Err(StoreWriteError::Conflict {
                message: format!("{what} already exists"),
                current_updated_at: stored_updated_at.map(ToOwned::to_owned),
            }),
            None => Ok(()),
        },
        WriteExpectation::UpdatedAt(token) => match stored_updated_at {
            None => Err(StoreWriteError::NotFound(format!("{what} not found"))),
            Some(current) if current != token => Err(StoreWriteError::Conflict {
                message: format!(
                    "{what} was modified concurrently — re-read it and retry with the latest updated_at"
                ),
                current_updated_at: Some(current.to_owned()),
            }),
            Some(_) => Ok(()),
        },
        WriteExpectation::Overwrite => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_requires_absence() {
        assert!(check_write_expectation(&WriteExpectation::Create, None, "custom agent").is_ok());
        let error = check_write_expectation(
            &WriteExpectation::Create,
            Some("2026-01-01T00:00:00Z"),
            "custom agent",
        )
        .expect_err("existing id must conflict");
        assert!(matches!(error, StoreWriteError::Conflict { .. }));
    }

    #[test]
    fn updated_at_requires_matching_token() {
        let expectation = WriteExpectation::UpdatedAt("2026-01-01T00:00:00Z".to_owned());
        assert!(
            check_write_expectation(&expectation, Some("2026-01-01T00:00:00Z"), "agent team")
                .is_ok()
        );
        let missing = check_write_expectation(&expectation, None, "agent team")
            .expect_err("missing id must be not-found");
        assert!(matches!(missing, StoreWriteError::NotFound(_)));
        let stale =
            check_write_expectation(&expectation, Some("2026-02-02T00:00:00Z"), "agent team")
                .expect_err("moved token must conflict");
        match stale {
            StoreWriteError::Conflict {
                current_updated_at, ..
            } => assert_eq!(current_updated_at.as_deref(), Some("2026-02-02T00:00:00Z")),
            other => panic!("expected conflict, got {other:?}"),
        }
    }

    #[test]
    fn overwrite_is_unconditional() {
        assert!(
            check_write_expectation(
                &WriteExpectation::Overwrite,
                Some("anything"),
                "custom agent"
            )
            .is_ok()
        );
        assert!(
            check_write_expectation(&WriteExpectation::Overwrite, None, "custom agent").is_ok()
        );
    }
}
