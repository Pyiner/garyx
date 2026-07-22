use super::*;

pub(crate) const QUOTA_RECOVERY_MIGRATION_NAME: &str = "quota_recovery_jobs_v1";
pub(crate) const QUOTA_RECOVERY_MIGRATION_VERSION: i64 = 1;

const QUOTA_RECOVERY_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS quota_recovery_jobs (
    job_id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    blocked_run_id TEXT NOT NULL,
    blocked_seq INTEGER NOT NULL CHECK (blocked_seq > 0),
    quota_window TEXT,
    reset_at TEXT,
    due_at TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN (
        'waiting', 'claimed', 'delivered', 'superseded', 'cancelled'
    )),
    wake_reason TEXT NOT NULL CHECK (wake_reason IN (
        'quota_reset', 'account_switch', 'manual'
    )),
    claim_token TEXT,
    claim_expires_at TEXT,
    dispatch_intent_id TEXT NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    settled_at TEXT,
    UNIQUE(thread_id, blocked_run_id),
    CHECK (
        (state = 'claimed' AND claim_token IS NOT NULL AND claim_expires_at IS NOT NULL)
        OR
        (state <> 'claimed' AND claim_token IS NULL AND claim_expires_at IS NULL)
    ),
    CHECK (
        (state IN ('delivered', 'superseded', 'cancelled') AND settled_at IS NOT NULL)
        OR
        (state IN ('waiting', 'claimed') AND settled_at IS NULL)
    ),
    FOREIGN KEY (thread_id) REFERENCES thread_records(key) ON DELETE CASCADE
) STRICT;
"#;

const QUOTA_RECOVERY_ACTIVE_INDEX_SQL: &str = r#"
CREATE UNIQUE INDEX IF NOT EXISTS idx_quota_recovery_active_thread
    ON quota_recovery_jobs(thread_id)
    WHERE state IN ('waiting', 'claimed');
"#;

const QUOTA_RECOVERY_DUE_INDEX_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_quota_recovery_due
    ON quota_recovery_jobs(state, due_at);
"#;

const QUOTA_RECOVERY_PROVIDER_INDEX_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_quota_recovery_provider_waiting
    ON quota_recovery_jobs(provider, state, due_at);
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuotaRecoveryState {
    Waiting,
    Claimed,
    Delivered,
    Superseded,
    Cancelled,
}

impl QuotaRecoveryState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Claimed => "claimed",
            Self::Delivered => "delivered",
            Self::Superseded => "superseded",
            Self::Cancelled => "cancelled",
        }
    }

    fn parse(value: &str) -> GaryxDbResult<Self> {
        match value {
            "waiting" => Ok(Self::Waiting),
            "claimed" => Ok(Self::Claimed),
            "delivered" => Ok(Self::Delivered),
            "superseded" => Ok(Self::Superseded),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(GaryxDbError::Configuration(format!(
                "invalid quota recovery state '{value}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuotaRecoveryWakeReason {
    QuotaReset,
    AccountSwitch,
    Manual,
}

impl QuotaRecoveryWakeReason {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::QuotaReset => "quota_reset",
            Self::AccountSwitch => "account_switch",
            Self::Manual => "manual",
        }
    }

    fn parse(value: &str) -> GaryxDbResult<Self> {
        match value {
            "quota_reset" => Ok(Self::QuotaReset),
            "account_switch" => Ok(Self::AccountSwitch),
            "manual" => Ok(Self::Manual),
            _ => Err(GaryxDbError::Configuration(format!(
                "invalid quota recovery wake reason '{value}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QuotaRecoveryJob {
    pub job_id: String,
    pub thread_id: String,
    pub provider: String,
    pub blocked_run_id: String,
    pub blocked_seq: i64,
    pub quota_window: Option<String>,
    pub reset_at: Option<String>,
    pub due_at: String,
    pub state: QuotaRecoveryState,
    pub wake_reason: QuotaRecoveryWakeReason,
    pub claim_token: Option<String>,
    pub claim_expires_at: Option<String>,
    pub dispatch_intent_id: String,
    pub attempt_count: i64,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub settled_at: Option<String>,
}

pub(crate) struct NewQuotaRecoveryJob<'a> {
    pub thread_id: &'a str,
    pub provider: &'a str,
    pub blocked_run_id: &'a str,
    pub blocked_seq: u64,
    pub quota_window: Option<&'a str>,
    pub reset_at: Option<&'a str>,
    pub due_at: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QuotaRecoveryClaimWitness {
    pub job_id: String,
    pub claim_token: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub(crate) struct QuotaRecoveryExpediteSummary {
    pub matched_threads: usize,
    pub expedited_threads: usize,
    pub already_claimed_threads: usize,
}

const QUOTA_RECOVERY_SELECT: &str =
    "SELECT job_id, thread_id, provider, blocked_run_id, blocked_seq, quota_window,
            reset_at, due_at, state, wake_reason, claim_token, claim_expires_at,
            dispatch_intent_id, attempt_count, last_error, created_at, updated_at,
            settled_at
       FROM quota_recovery_jobs";

fn read_quota_recovery_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<QuotaRecoveryJob> {
    let state = row.get::<_, String>(8)?;
    let wake_reason = row.get::<_, String>(9)?;
    Ok(QuotaRecoveryJob {
        job_id: row.get(0)?,
        thread_id: row.get(1)?,
        provider: row.get(2)?,
        blocked_run_id: row.get(3)?,
        blocked_seq: row.get(4)?,
        quota_window: row.get(5)?,
        reset_at: row.get(6)?,
        due_at: row.get(7)?,
        state: QuotaRecoveryState::parse(&state).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                8,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        wake_reason: QuotaRecoveryWakeReason::parse(&wake_reason).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                9,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        claim_token: row.get(10)?,
        claim_expires_at: row.get(11)?,
        dispatch_intent_id: row.get(12)?,
        attempt_count: row.get(13)?,
        last_error: row.get(14)?,
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
        settled_at: row.get(17)?,
    })
}

fn canonical_schema_sql(sql: &str) -> String {
    sql.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace("CREATE TABLE IF NOT EXISTS", "CREATE TABLE")
        .replace("CREATE UNIQUE INDEX IF NOT EXISTS", "CREATE UNIQUE INDEX")
        .replace("CREATE INDEX IF NOT EXISTS", "CREATE INDEX")
        .trim_end_matches(';')
        .to_owned()
}

fn validate_quota_recovery_schema(tx: &Transaction<'_>) -> GaryxDbResult<()> {
    for (kind, name, expected) in [
        ("table", "quota_recovery_jobs", QUOTA_RECOVERY_TABLE_SQL),
        (
            "index",
            "idx_quota_recovery_active_thread",
            QUOTA_RECOVERY_ACTIVE_INDEX_SQL,
        ),
        (
            "index",
            "idx_quota_recovery_due",
            QUOTA_RECOVERY_DUE_INDEX_SQL,
        ),
        (
            "index",
            "idx_quota_recovery_provider_waiting",
            QUOTA_RECOVERY_PROVIDER_INDEX_SQL,
        ),
    ] {
        let actual = tx
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = ?1 AND name = ?2",
                params![kind, name],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| {
                GaryxDbError::Configuration(format!(
                    "quota recovery schema is missing {kind} '{name}'"
                ))
            })?;
        if canonical_schema_sql(&actual) != canonical_schema_sql(expected) {
            return Err(GaryxDbError::Configuration(format!(
                "quota recovery {kind} '{name}' does not match the committed v1 schema"
            )));
        }
    }
    Ok(())
}

fn quota_recovery_job_id(blocked_run_id: &str) -> String {
    format!("quota-recovery:{blocked_run_id}")
}

impl GaryxDbService {
    pub(crate) fn migrate_quota_recovery_jobs_v1(&self) -> GaryxDbResult<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let marker = tx
            .query_row(
                "SELECT projection_version FROM projection_states WHERE projection_name = ?1",
                params![QUOTA_RECOVERY_MIGRATION_NAME],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if marker.is_some_and(|version| version != QUOTA_RECOVERY_MIGRATION_VERSION) {
            return Err(GaryxDbError::Configuration(format!(
                "quota recovery marker version mismatch: expected {}, found {}",
                QUOTA_RECOVERY_MIGRATION_VERSION,
                marker.unwrap_or_default()
            )));
        }
        if marker.is_none() {
            tx.execute_batch(QUOTA_RECOVERY_TABLE_SQL)?;
            tx.execute_batch(QUOTA_RECOVERY_ACTIVE_INDEX_SQL)?;
            tx.execute_batch(QUOTA_RECOVERY_DUE_INDEX_SQL)?;
            tx.execute_batch(QUOTA_RECOVERY_PROVIDER_INDEX_SQL)?;
        }
        validate_quota_recovery_schema(&tx)?;
        if marker.is_none() {
            record_projection_state_tx(
                &tx,
                QUOTA_RECOVERY_MIGRATION_NAME,
                QUOTA_RECOVERY_MIGRATION_VERSION,
                0,
                None,
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn register_quota_recovery_job(
        &self,
        input: NewQuotaRecoveryJob<'_>,
    ) -> GaryxDbResult<QuotaRecoveryJob> {
        let thread_id = normalize_thread_id(input.thread_id)?;
        let provider = normalize_required("provider", input.provider)?;
        let blocked_run_id = normalize_required("blocked_run_id", input.blocked_run_id)?;
        let blocked_seq = i64::try_from(input.blocked_seq).map_err(|_| {
            GaryxDbError::BadRequest("blocked_seq exceeds SQLite INTEGER range".to_owned())
        })?;
        if blocked_seq <= 0 {
            return Err(GaryxDbError::BadRequest(
                "blocked_seq must be positive".to_owned(),
            ));
        }
        let due_at = normalize_required("due_at", input.due_at)?;
        let quota_window = normalize_optional(input.quota_window);
        let reset_at = normalize_optional(input.reset_at);
        let job_id = quota_recovery_job_id(&blocked_run_id);
        let dispatch_intent_id = job_id.clone();
        let now = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

        let existing = tx
            .query_row(
                &format!("{QUOTA_RECOVERY_SELECT} WHERE thread_id = ?1 AND blocked_run_id = ?2"),
                params![thread_id, blocked_run_id],
                read_quota_recovery_row,
            )
            .optional()?;
        if let Some(existing) = existing {
            tx.commit()?;
            return Ok(existing);
        }

        let newest = tx
            .query_row(
                &format!(
                    "{QUOTA_RECOVERY_SELECT} WHERE thread_id = ?1
                      ORDER BY blocked_seq DESC, rowid DESC LIMIT 1"
                ),
                params![thread_id],
                read_quota_recovery_row,
            )
            .optional()?;
        if let Some(newest) = newest
            && newest.blocked_seq >= blocked_seq
        {
            tx.commit()?;
            return Ok(newest);
        }

        let thread_exists = tx
            .query_row(
                "SELECT 1 FROM thread_records WHERE key = ?1",
                params![thread_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !thread_exists {
            return Err(GaryxDbError::NotFound(format!(
                "thread not found: {thread_id}"
            )));
        }

        tx.execute(
            "UPDATE quota_recovery_jobs
                SET state = 'superseded', claim_token = NULL,
                    claim_expires_at = NULL, settled_at = ?2, updated_at = ?2
              WHERE thread_id = ?1 AND state IN ('waiting', 'claimed')",
            params![thread_id, now],
        )?;
        tx.execute(
            "INSERT INTO quota_recovery_jobs (
                job_id, thread_id, provider, blocked_run_id, blocked_seq,
                quota_window, reset_at, due_at, state, wake_reason,
                dispatch_intent_id, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'waiting',
                       'quota_reset', ?9, ?10, ?10)",
            params![
                job_id,
                thread_id,
                provider,
                blocked_run_id,
                blocked_seq,
                quota_window,
                reset_at,
                due_at,
                dispatch_intent_id,
                now,
            ],
        )?;
        let record = tx.query_row(
            &format!("{QUOTA_RECOVERY_SELECT} WHERE job_id = ?1"),
            params![job_id],
            read_quota_recovery_row,
        )?;
        tx.commit()?;
        Ok(record)
    }

    #[cfg(test)]
    pub(crate) fn active_quota_recovery_job(
        &self,
        thread_id: &str,
    ) -> GaryxDbResult<Option<QuotaRecoveryJob>> {
        let thread_id = normalize_thread_id(thread_id)?;
        let conn = self.read_conn()?;
        conn.query_row(
            &format!(
                "{QUOTA_RECOVERY_SELECT} WHERE thread_id = ?1
                  AND state IN ('waiting', 'claimed')"
            ),
            params![thread_id],
            read_quota_recovery_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub(crate) fn latest_quota_recovery_job_for_thread(
        &self,
        thread_id: &str,
    ) -> GaryxDbResult<Option<QuotaRecoveryJob>> {
        let thread_id = normalize_thread_id(thread_id)?;
        let conn = self.read_conn()?;
        conn.query_row(
            &format!(
                "{QUOTA_RECOVERY_SELECT} WHERE thread_id = ?1
                  ORDER BY blocked_seq DESC, rowid DESC LIMIT 1"
            ),
            params![thread_id],
            read_quota_recovery_row,
        )
        .optional()
        .map_err(Into::into)
    }

    #[cfg(test)]
    pub(crate) fn quota_recovery_job(
        &self,
        job_id: &str,
    ) -> GaryxDbResult<Option<QuotaRecoveryJob>> {
        let job_id = normalize_required("job_id", job_id)?;
        let conn = self.read_conn()?;
        conn.query_row(
            &format!("{QUOTA_RECOVERY_SELECT} WHERE job_id = ?1"),
            params![job_id],
            read_quota_recovery_row,
        )
        .optional()
        .map_err(Into::into)
    }

    pub(crate) fn expedite_quota_recovery_provider(
        &self,
        provider: &str,
        due_at: &str,
    ) -> GaryxDbResult<QuotaRecoveryExpediteSummary> {
        let provider = normalize_required("provider", provider)?;
        let due_at = normalize_required("due_at", due_at)?;
        let now = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let waiting = tx.query_row(
            "SELECT COUNT(*) FROM quota_recovery_jobs
              WHERE provider = ?1 AND state = 'waiting'",
            params![provider],
            |row| row.get::<_, i64>(0),
        )?;
        let claimed = tx.query_row(
            "SELECT COUNT(*) FROM quota_recovery_jobs
              WHERE provider = ?1 AND state = 'claimed'",
            params![provider],
            |row| row.get::<_, i64>(0),
        )?;
        let changed = tx.execute(
            "UPDATE quota_recovery_jobs
                SET due_at = ?2, wake_reason = 'account_switch',
                    last_error = NULL, updated_at = ?3
              WHERE provider = ?1 AND state = 'waiting'",
            params![provider, due_at, now],
        )?;
        tx.commit()?;
        Ok(QuotaRecoveryExpediteSummary {
            matched_threads: usize::try_from(waiting.saturating_add(claimed)).unwrap_or(usize::MAX),
            expedited_threads: changed,
            already_claimed_threads: usize::try_from(claimed).unwrap_or(usize::MAX),
        })
    }

    pub(crate) fn expedite_quota_recovery_thread(
        &self,
        thread_id: &str,
        due_at: &str,
    ) -> GaryxDbResult<bool> {
        let thread_id = normalize_thread_id(thread_id)?;
        let due_at = normalize_required("due_at", due_at)?;
        let now = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE quota_recovery_jobs
                SET due_at = ?2, wake_reason = 'manual', last_error = NULL,
                    updated_at = ?3
              WHERE thread_id = ?1 AND state = 'waiting'",
            params![thread_id, due_at, now],
        )? > 0;
        let already_claimed = if changed {
            false
        } else {
            tx.query_row(
                "SELECT 1 FROM quota_recovery_jobs
                  WHERE thread_id = ?1 AND state = 'claimed'",
                params![thread_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        };
        tx.commit()?;
        Ok(changed || already_claimed)
    }

    pub(crate) fn next_quota_recovery_due_at(&self) -> GaryxDbResult<Option<String>> {
        let conn = self.read_conn()?;
        conn.query_row(
            "SELECT MIN(due_at) FROM quota_recovery_jobs
              WHERE state = 'waiting'
                AND (reset_at IS NOT NULL OR wake_reason <> 'quota_reset')",
            [],
            |row| row.get(0),
        )
        .map_err(Into::into)
    }

    pub(crate) fn claim_next_due_quota_recovery(
        &self,
        now: &str,
        claim_token: &str,
        claim_expires_at: &str,
    ) -> GaryxDbResult<Option<QuotaRecoveryJob>> {
        let now = normalize_required("now", now)?;
        let claim_token = normalize_required("claim_token", claim_token)?;
        let claim_expires_at = normalize_required("claim_expires_at", claim_expires_at)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let candidate = tx
            .query_row(
                "SELECT job_id FROM quota_recovery_jobs
                  WHERE state = 'waiting' AND due_at <= ?1
                    AND (reset_at IS NOT NULL OR wake_reason <> 'quota_reset')
                  ORDER BY due_at ASC, created_at ASC, job_id ASC
                  LIMIT 1",
                params![now],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(job_id) = candidate else {
            tx.commit()?;
            return Ok(None);
        };
        let changed = tx.execute(
            "UPDATE quota_recovery_jobs
                SET state = 'claimed', claim_token = ?2, claim_expires_at = ?3,
                    attempt_count = attempt_count + 1, updated_at = ?1
              WHERE job_id = ?4 AND state = 'waiting' AND due_at <= ?1
                AND (reset_at IS NOT NULL OR wake_reason <> 'quota_reset')",
            params![now, claim_token, claim_expires_at, job_id],
        )?;
        if changed == 0 {
            tx.commit()?;
            return Ok(None);
        }
        let record = tx.query_row(
            &format!("{QUOTA_RECOVERY_SELECT} WHERE job_id = ?1"),
            params![job_id],
            read_quota_recovery_row,
        )?;
        tx.commit()?;
        Ok(Some(record))
    }

    pub(crate) fn retry_claimed_quota_recovery(
        &self,
        job_id: &str,
        claim_token: &str,
        due_at: &str,
        error: &str,
    ) -> GaryxDbResult<bool> {
        let now = now_string();
        let conn = self.conn()?;
        Ok(conn.execute(
            "UPDATE quota_recovery_jobs
                SET state = 'waiting', due_at = ?3, claim_token = NULL,
                    claim_expires_at = NULL, last_error = ?4, updated_at = ?5
              WHERE job_id = ?1 AND state = 'claimed' AND claim_token = ?2",
            params![job_id, claim_token, due_at, error, now],
        )? > 0)
    }

    pub(crate) fn deliver_claimed_quota_recovery(
        &self,
        job_id: &str,
        claim_token: &str,
    ) -> GaryxDbResult<bool> {
        self.settle_claimed_quota_recovery(job_id, claim_token, QuotaRecoveryState::Delivered)
    }

    pub(crate) fn supersede_claimed_quota_recovery(
        &self,
        job_id: &str,
        claim_token: &str,
    ) -> GaryxDbResult<bool> {
        self.settle_claimed_quota_recovery(job_id, claim_token, QuotaRecoveryState::Superseded)
    }

    pub(crate) fn cancel_claimed_quota_recovery(
        &self,
        job_id: &str,
        claim_token: &str,
    ) -> GaryxDbResult<bool> {
        self.settle_claimed_quota_recovery(job_id, claim_token, QuotaRecoveryState::Cancelled)
    }

    fn settle_claimed_quota_recovery(
        &self,
        job_id: &str,
        claim_token: &str,
        state: QuotaRecoveryState,
    ) -> GaryxDbResult<bool> {
        if !matches!(
            state,
            QuotaRecoveryState::Delivered
                | QuotaRecoveryState::Superseded
                | QuotaRecoveryState::Cancelled
        ) {
            return Err(GaryxDbError::BadRequest(format!(
                "invalid claimed quota recovery settlement state '{}'",
                state.as_str()
            )));
        }
        let now = now_string();
        let conn = self.conn()?;
        Ok(conn.execute(
            "UPDATE quota_recovery_jobs
                SET state = ?3, claim_token = NULL,
                    claim_expires_at = NULL, last_error = NULL,
                    updated_at = ?4, settled_at = ?4
              WHERE job_id = ?1 AND state = 'claimed' AND claim_token = ?2",
            params![job_id, claim_token, state.as_str(), now],
        )? > 0)
    }

    pub(crate) fn recover_stale_quota_recovery_claims(&self) -> GaryxDbResult<usize> {
        let now = now_string();
        let conn = self.conn()?;
        conn.execute(
            "UPDATE quota_recovery_jobs
                SET state = 'waiting', due_at = ?1, claim_token = NULL,
                    claim_expires_at = NULL,
                    last_error = 'gateway restarted during quota recovery',
                    updated_at = ?1
              WHERE state = 'claimed'",
            params![now],
        )
        .map_err(Into::into)
    }

    pub fn prune_settled_quota_recovery_jobs(&self, settled_before: &str) -> GaryxDbResult<usize> {
        let settled_before = normalize_required("settled_before", settled_before)?;
        let conn = self.conn()?;
        conn.execute(
            "DELETE FROM quota_recovery_jobs
              WHERE state IN ('delivered', 'superseded', 'cancelled')
                AND settled_at < ?1",
            params![settled_before],
        )
        .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> GaryxDbService {
        let db = GaryxDbService::memory().unwrap();
        db.run_thread_data_startup_migrations().unwrap();
        db.write_thread_record_with_projections("thread::quota", "{}", None, None)
            .unwrap();
        db
    }

    fn insert(db: &GaryxDbService, run_id: &str, due_at: &str) -> QuotaRecoveryJob {
        db.register_quota_recovery_job(NewQuotaRecoveryJob {
            thread_id: "thread::quota",
            provider: "claude",
            blocked_run_id: run_id,
            blocked_seq: match run_id {
                "run::one" => 1,
                "run::two" => 2,
                _ => 3,
            },
            quota_window: Some("primary"),
            reset_at: Some("2026-07-23T00:00:00Z"),
            due_at,
        })
        .unwrap()
    }

    #[test]
    fn registration_replay_is_idempotent_and_new_generation_supersedes() {
        let db = db();
        let first = insert(&db, "run::one", "2026-07-23T00:01:00Z");
        assert_eq!(insert(&db, "run::one", "2099-01-01T00:00:00Z"), first);

        let second = insert(&db, "run::two", "2026-07-23T00:02:00Z");
        assert_eq!(second.state, QuotaRecoveryState::Waiting);
        assert_eq!(
            db.quota_recovery_job(&first.job_id).unwrap().unwrap().state,
            QuotaRecoveryState::Superseded
        );
        assert_eq!(
            db.active_quota_recovery_job("thread::quota")
                .unwrap()
                .unwrap()
                .blocked_run_id,
            "run::two"
        );
    }

    #[test]
    fn delayed_older_projection_cannot_replace_a_newer_generation() {
        let db = db();
        let newer = insert(&db, "run::two", "2026-07-23T00:02:00Z");

        let observed = db
            .register_quota_recovery_job(NewQuotaRecoveryJob {
                thread_id: "thread::quota",
                provider: "claude",
                blocked_run_id: "run::late-old",
                blocked_seq: 1,
                quota_window: Some("primary"),
                reset_at: Some("2026-07-23T00:00:00Z"),
                due_at: "2026-07-23T00:01:00Z",
            })
            .unwrap();

        assert_eq!(observed.job_id, newer.job_id);
        assert!(
            db.quota_recovery_job("quota-recovery:run::late-old")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            db.active_quota_recovery_job("thread::quota")
                .unwrap()
                .unwrap()
                .blocked_run_id,
            "run::two"
        );
    }

    #[test]
    fn account_switch_and_timer_share_one_claim() {
        let db = db();
        let job = insert(&db, "run::one", "2099-01-01T00:00:00Z");
        let summary = db
            .expedite_quota_recovery_provider("claude", "2026-07-23T00:00:00Z")
            .unwrap();
        assert_eq!(summary.expedited_threads, 1);
        let claimed = db
            .claim_next_due_quota_recovery(
                "2026-07-23T00:00:01Z",
                "claim::one",
                "2026-07-23T00:01:01Z",
            )
            .unwrap()
            .unwrap();
        assert_eq!(claimed.job_id, job.job_id);
        assert!(
            db.expedite_quota_recovery_thread("thread::quota", "2026-07-23T00:00:02Z")
                .unwrap(),
            "a repeated manual retry should accept an already claimed generation"
        );
        assert!(
            db.claim_next_due_quota_recovery(
                "2099-01-01T00:00:01Z",
                "claim::two",
                "2099-01-01T00:01:01Z",
            )
            .unwrap()
            .is_none()
        );
        assert!(
            db.deliver_claimed_quota_recovery(&job.job_id, "claim::one")
                .unwrap()
        );
    }

    #[test]
    fn expired_claim_is_recovered_without_losing_generation() {
        let db = db();
        let job = insert(&db, "run::one", "2020-01-01T00:00:00Z");
        db.claim_next_due_quota_recovery(
            "2020-01-01T00:00:01Z",
            "claim::one",
            "2020-01-01T00:00:02Z",
        )
        .unwrap()
        .unwrap();
        assert_eq!(db.recover_stale_quota_recovery_claims().unwrap(), 1);
        let recovered = db.quota_recovery_job(&job.job_id).unwrap().unwrap();
        assert_eq!(recovered.state, QuotaRecoveryState::Waiting);
        assert_eq!(recovered.blocked_run_id, "run::one");
    }

    #[test]
    fn no_reset_job_stays_parked_until_an_explicit_wake() {
        let db = db();
        db.register_quota_recovery_job(NewQuotaRecoveryJob {
            thread_id: "thread::quota",
            provider: "claude_code",
            blocked_run_id: "run::no-reset",
            blocked_seq: 3,
            quota_window: None,
            reset_at: None,
            due_at: "9999-12-31T23:59:59.999Z",
        })
        .unwrap();
        assert!(db.next_quota_recovery_due_at().unwrap().is_none());
        assert!(
            db.claim_next_due_quota_recovery(
                "9999-12-31T23:59:59.999Z",
                "claim::timer",
                "9999-12-31T23:59:59.999Z",
            )
            .unwrap()
            .is_none(),
            "a parked generation must not become timer-claimable even at its sentinel deadline"
        );

        assert!(
            db.expedite_quota_recovery_thread("thread::quota", "2026-07-23T00:00:00.000Z")
                .unwrap()
        );
        assert_eq!(
            db.next_quota_recovery_due_at().unwrap().as_deref(),
            Some("2026-07-23T00:00:00.000Z")
        );
    }

    #[test]
    fn ordinary_dispatch_atomically_supersedes_claimed_recovery() {
        let db = db();
        let job = insert(&db, "run::blocked", "2020-01-01T00:00:00Z");
        db.claim_next_due_quota_recovery(
            "2020-01-01T00:00:01Z",
            "claim::recovery",
            "2020-01-01T00:01:01Z",
        )
        .unwrap()
        .unwrap();
        let key = DispatchAdmissionKey {
            scope_identity: "desktop".to_owned(),
            scope_epoch: 1,
            thread_id: "thread::quota".to_owned(),
            kind: DispatchAdmissionKind::ChatStart,
            client_intent_id: "user::new-turn".to_owned(),
        };
        db.insert_dispatch_admission_with_records_for_existing_thread(
            NewDispatchAdmission {
                key: &key,
                request_fingerprint: "fingerprint",
                requested_run_id: Some("run::user"),
                effective_run_id: Some("run::user"),
                pending_input_id: None,
                outcome: Some(DispatchOutcome::Started),
            },
            Vec::new(),
            &[],
            true,
            None,
        )
        .unwrap();

        assert_eq!(
            db.quota_recovery_job(&job.job_id).unwrap().unwrap().state,
            QuotaRecoveryState::Superseded
        );
        let stale = db.insert_dispatch_admission_with_records_for_existing_thread(
            NewDispatchAdmission {
                key: &DispatchAdmissionKey {
                    scope_identity: "__quota_recovery__".to_owned(),
                    scope_epoch: 1,
                    thread_id: "thread::quota".to_owned(),
                    kind: DispatchAdmissionKind::ChatStart,
                    client_intent_id: job.dispatch_intent_id.clone(),
                },
                request_fingerprint: "quota-fingerprint",
                requested_run_id: Some("run::recovery"),
                effective_run_id: Some("run::recovery"),
                pending_input_id: None,
                outcome: Some(DispatchOutcome::Started),
            },
            Vec::new(),
            &[],
            false,
            Some(&QuotaRecoveryClaimWitness {
                job_id: job.job_id,
                claim_token: "claim::recovery".to_owned(),
            }),
        );
        assert!(stale.unwrap_err().to_string().contains("no longer active"));
    }
}
