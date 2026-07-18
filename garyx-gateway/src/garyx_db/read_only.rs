//! Read-only DB handle that bypasses the data-dir lock.

use super::*;

/// Narrow read-only handle for offline control-plane reads while the gateway
/// process may still own the writable database connection. Unlike
/// `GaryxDbService::open`, this never creates a database, changes WAL mode,
/// initializes schema, or exposes mutation methods.
pub(crate) struct ReadOnlyGaryxDb {
    pub(super) conn: Connection,
}

impl ReadOnlyGaryxDb {
    pub(crate) fn open(path: impl AsRef<Path>) -> GaryxDbResult<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        conn.pragma_update(None, "query_only", "ON")?;
        Ok(Self { conn })
    }

    pub(crate) fn list_active_recent_thread_ids(
        &mut self,
        limit: usize,
    ) -> GaryxDbResult<ActiveRecentThreadPage> {
        list_active_recent_thread_ids(&mut self.conn, limit)
    }
}
