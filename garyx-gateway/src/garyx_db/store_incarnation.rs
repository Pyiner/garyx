//! Store incarnation identity (favorites CAS fencing).

use super::*;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StoreIncarnation {
    pub store_incarnation_id: String,
}

pub(super) fn ensure_store_incarnation_row(conn: &Connection) -> GaryxDbResult<()> {
    conn.execute(
        "INSERT INTO garyx_store_meta (id, store_incarnation_id)
         VALUES (1, ?1)
         ON CONFLICT(id) DO NOTHING",
        params![Uuid::new_v4().to_string()],
    )?;
    // Treat corruption as a startup failure rather than silently rotating the
    // CAS domain during an ordinary reopen.
    read_store_incarnation_id(conn).map(|_| ())
}

pub(super) fn read_store_incarnation_id(conn: &Connection) -> GaryxDbResult<String> {
    let raw: String = conn
        .query_row(
            "SELECT store_incarnation_id FROM garyx_store_meta WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| {
            GaryxDbError::Configuration("garyx_store_meta singleton is missing".to_owned())
        })?;
    Uuid::parse_str(&raw)
        .map(|uuid| uuid.to_string())
        .map_err(|_| {
            GaryxDbError::Configuration("store_incarnation_id is not a valid UUID".to_owned())
        })
}

pub(super) fn rotate_store_incarnation_tx(conn: &Connection) -> GaryxDbResult<String> {
    let next = Uuid::new_v4().to_string();
    let updated = conn.execute(
        "UPDATE garyx_store_meta SET store_incarnation_id = ?1 WHERE id = 1",
        params![next],
    )?;
    if updated != 1 {
        return Err(GaryxDbError::Configuration(
            "garyx_store_meta singleton is missing".to_owned(),
        ));
    }
    Ok(next)
}

impl GaryxDbService {
    pub fn store_incarnation_id(&self) -> GaryxDbResult<String> {
        let conn = self.read_conn()?;
        read_store_incarnation_id(&conn)
    }

    /// Rotate the persistent CAS identity for an offline full-data-dir
    /// restore/clone. Normal opens and process restarts never call this.
    pub fn rotate_store_incarnation(&self) -> GaryxDbResult<StoreIncarnation> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let store_incarnation_id = rotate_store_incarnation_tx(&tx)?;
        tx.commit()?;
        Ok(StoreIncarnation {
            store_incarnation_id,
        })
    }
}
