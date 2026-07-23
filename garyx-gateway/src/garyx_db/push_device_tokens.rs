use super::*;

pub(crate) const PUSH_DEVICE_TOKENS_MIGRATION_NAME: &str = "push_device_tokens_v1";
pub(crate) const PUSH_DEVICE_TOKENS_MIGRATION_VERSION: i64 = 1;

const PUSH_DEVICE_TOKENS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS push_device_tokens (
    token TEXT PRIMARY KEY,
    platform TEXT NOT NULL CHECK (platform = 'ios'),
    environment TEXT NOT NULL CHECK (environment IN ('development', 'production')),
    bundle_id TEXT NOT NULL,
    device_name TEXT,
    registered_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL
) STRICT;
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PushDeviceToken {
    pub token: String,
    pub platform: String,
    pub environment: String,
    pub bundle_id: String,
    pub device_name: Option<String>,
    pub registered_at: String,
    pub last_seen_at: String,
}

pub(crate) struct PushDeviceTokenDraft<'a> {
    pub token: &'a str,
    pub platform: &'a str,
    pub environment: &'a str,
    pub bundle_id: &'a str,
    pub device_name: Option<&'a str>,
}

fn canonical_schema_sql(sql: &str) -> String {
    sql.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace("CREATE TABLE IF NOT EXISTS", "CREATE TABLE")
        .trim_end_matches(';')
        .to_owned()
}

impl GaryxDbService {
    pub(crate) fn migrate_push_device_tokens_v1(&self) -> GaryxDbResult<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let marker = tx
            .query_row(
                "SELECT projection_version FROM projection_states WHERE projection_name = ?1",
                params![PUSH_DEVICE_TOKENS_MIGRATION_NAME],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if marker.is_some_and(|version| version != PUSH_DEVICE_TOKENS_MIGRATION_VERSION) {
            return Err(GaryxDbError::Configuration(format!(
                "push device token marker version mismatch: expected {}, found {}",
                PUSH_DEVICE_TOKENS_MIGRATION_VERSION,
                marker.unwrap_or_default()
            )));
        }

        let existing = tx
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'push_device_tokens'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(existing) = existing.as_deref() {
            if canonical_schema_sql(existing) != canonical_schema_sql(PUSH_DEVICE_TOKENS_TABLE_SQL)
            {
                return Err(GaryxDbError::Configuration(
                    "push_device_tokens table has an unsupported shape".to_owned(),
                ));
            }
        } else {
            tx.execute_batch(PUSH_DEVICE_TOKENS_TABLE_SQL)?;
        }

        if marker.is_none() {
            tx.execute(
                "INSERT INTO projection_states (
                    projection_name, projection_version, source_row_count, projected_at,
                    based_on_import_generation
                 ) VALUES (?1, ?2, 0, ?3, NULL)",
                params![
                    PUSH_DEVICE_TOKENS_MIGRATION_NAME,
                    PUSH_DEVICE_TOKENS_MIGRATION_VERSION,
                    now_string(),
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn upsert_push_device_token(
        &self,
        draft: PushDeviceTokenDraft<'_>,
    ) -> GaryxDbResult<()> {
        let token = normalize_required("token", draft.token)?;
        let platform = normalize_required("platform", draft.platform)?;
        if platform != "ios" {
            return Err(GaryxDbError::BadRequest("platform must be ios".to_owned()));
        }
        let environment = normalize_required("environment", draft.environment)?;
        if !matches!(environment.as_str(), "development" | "production") {
            return Err(GaryxDbError::BadRequest(
                "environment must be development or production".to_owned(),
            ));
        }
        let bundle_id = normalize_required("bundle_id", draft.bundle_id)?;
        let device_name = normalize_optional(draft.device_name);
        let now = now_string();
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO push_device_tokens (
                token, platform, environment, bundle_id, device_name,
                registered_at, last_seen_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(token) DO UPDATE SET
                platform = excluded.platform,
                environment = excluded.environment,
                bundle_id = excluded.bundle_id,
                device_name = excluded.device_name,
                last_seen_at = excluded.last_seen_at",
            params![token, platform, environment, bundle_id, device_name, now,],
        )?;
        Ok(())
    }

    pub(crate) fn delete_push_device_token(&self, token: &str) -> GaryxDbResult<bool> {
        let token = normalize_required("token", token)?;
        let conn = self.conn()?;
        Ok(conn.execute(
            "DELETE FROM push_device_tokens WHERE token = ?1",
            params![token],
        )? > 0)
    }

    pub(crate) fn list_push_device_tokens(&self) -> GaryxDbResult<Vec<PushDeviceToken>> {
        let conn = self.read_conn()?;
        let mut statement = conn.prepare(
            "SELECT token, platform, environment, bundle_id, device_name,
                    registered_at, last_seen_at
               FROM push_device_tokens
              ORDER BY token",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(PushDeviceToken {
                token: row.get(0)?,
                platform: row.get(1)?,
                environment: row.get(2)?,
                bundle_id: row.get(3)?,
                device_name: row.get(4)?,
                registered_at: row.get(5)?,
                last_seen_at: row.get(6)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_is_idempotent_and_upsert_preserves_registration_time() {
        let db = GaryxDbService::memory().unwrap();
        db.run_thread_data_startup_migrations().unwrap();
        db.run_thread_data_startup_migrations().unwrap();
        db.upsert_push_device_token(PushDeviceTokenDraft {
            token: "synthetic-device-token",
            platform: "ios",
            environment: "development",
            bundle_id: "com.garyx.mobile",
            device_name: Some("Test iPhone"),
        })
        .unwrap();
        let first = db.list_push_device_tokens().unwrap().remove(0);

        db.upsert_push_device_token(PushDeviceTokenDraft {
            token: "synthetic-device-token",
            platform: "ios",
            environment: "production",
            bundle_id: "com.garyx.mobile",
            device_name: None,
        })
        .unwrap();
        let second = db.list_push_device_tokens().unwrap().remove(0);

        assert_eq!(second.registered_at, first.registered_at);
        assert_eq!(second.environment, "production");
        assert_eq!(second.device_name, None);
        assert!(
            db.delete_push_device_token("synthetic-device-token")
                .unwrap()
        );
        assert!(db.list_push_device_tokens().unwrap().is_empty());
    }
}
