//! Pinned-thread projection reads and mutations.

use super::*;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PinnedThreadRecord {
    pub thread_id: String,
    pub pinned_at: String,
    pub sort_order: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ThreadPinsPage {
    pub pins: Vec<PinnedThreadRecord>,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReorderThreadPinsResult {
    Updated(ThreadPinsPage),
    Conflict(ThreadPinsPage),
}

pub(super) fn read_thread_pins_tx(conn: &Connection) -> GaryxDbResult<Vec<PinnedThreadRecord>> {
    let mut stmt = conn.prepare(
        "SELECT thread_id, pinned_at, sort_order
           FROM thread_pins
          ORDER BY sort_order ASC, pinned_at DESC, thread_id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(PinnedThreadRecord {
            thread_id: row.get(0)?,
            pinned_at: row.get(1)?,
            sort_order: row.get(2)?,
        })
    })?;
    let mut pins = Vec::new();
    for row in rows {
        pins.push(row?);
    }
    Ok(pins)
}

pub(super) fn read_thread_pins_revision_tx(conn: &Connection) -> GaryxDbResult<i64> {
    Ok(conn.query_row(
        "SELECT pins_revision FROM thread_pins_meta WHERE id = 1",
        [],
        |row| row.get(0),
    )?)
}

pub(super) fn read_thread_pins_page_tx(conn: &Connection) -> GaryxDbResult<ThreadPinsPage> {
    Ok(ThreadPinsPage {
        pins: read_thread_pins_tx(conn)?,
        revision: read_thread_pins_revision_tx(conn)?,
    })
}

/// Shared revision boundary for every runtime mutation of `thread_pins`.
/// Callers pass the mutation's affected-row result while still inside the
/// same transaction; no-op idempotent operations deliberately do not bump.
pub(super) fn bump_thread_pins_revision_if_changed_tx(
    conn: &Connection,
    changed: bool,
) -> GaryxDbResult<()> {
    if !changed {
        return Ok(());
    }
    let updated = conn.execute(
        "UPDATE thread_pins_meta
            SET pins_revision = pins_revision + 1
          WHERE id = 1",
        [],
    )?;
    if updated != 1 {
        return Err(GaryxDbError::Configuration(
            "thread_pins_meta singleton is missing".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn normalize_thread_pin_order(ordered_ids: Vec<String>) -> GaryxDbResult<Vec<String>> {
    if ordered_ids.is_empty() {
        return Err(GaryxDbError::BadRequest(
            "thread_ids must be a non-empty array".to_owned(),
        ));
    }
    let mut normalized = Vec::with_capacity(ordered_ids.len());
    let mut seen = BTreeSet::new();
    for thread_id in ordered_ids {
        let thread_id = normalize_thread_id(&thread_id)?;
        if !seen.insert(thread_id.clone()) {
            return Err(GaryxDbError::BadRequest(format!(
                "duplicate thread_id: {thread_id}"
            )));
        }
        normalized.push(thread_id);
    }
    Ok(normalized)
}

impl GaryxDbService {
    pub fn list_pinned_threads(&self) -> GaryxDbResult<ThreadPinsPage> {
        self.list_pinned_threads_inner(|| Ok(()))
    }

    pub(super) fn list_pinned_threads_inner<F>(
        &self,
        after_pins: F,
    ) -> GaryxDbResult<ThreadPinsPage>
    where
        F: FnOnce() -> GaryxDbResult<()>,
    {
        let mut conn = self.read_conn()?;
        let tx = conn.transaction()?;
        let pins = read_thread_pins_tx(&tx)?;

        // Deterministic test seam: a concurrent writer may commit here, but
        // the revision read below remains on this WAL snapshot.
        after_pins()?;

        let revision = read_thread_pins_revision_tx(&tx)?;
        tx.commit()?;
        Ok(ThreadPinsPage { pins, revision })
    }

    pub fn pin_thread(&self, thread_id: &str) -> GaryxDbResult<ThreadPinsPage> {
        let thread_id = normalize_thread_id(thread_id)?;
        let pinned_at = now_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "INSERT INTO thread_pins (thread_id, pinned_at, sort_order)
             VALUES (
                 ?1,
                 ?2,
                 COALESCE((SELECT MIN(sort_order) FROM thread_pins), 0) - 1
             )
             ON CONFLICT(thread_id) DO NOTHING",
            params![thread_id, pinned_at],
        )? > 0;
        bump_thread_pins_revision_if_changed_tx(&tx, changed)?;
        let page = read_thread_pins_page_tx(&tx)?;
        tx.commit()?;
        Ok(page)
    }

    pub fn unpin_thread(&self, thread_id: &str) -> GaryxDbResult<(bool, ThreadPinsPage)> {
        let thread_id = normalize_thread_id(thread_id)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let removed = tx.execute(
            "DELETE FROM thread_pins WHERE thread_id = ?1",
            params![thread_id],
        )? > 0;
        bump_thread_pins_revision_if_changed_tx(&tx, removed)?;
        let page = read_thread_pins_page_tx(&tx)?;
        tx.commit()?;
        Ok((removed, page))
    }

    pub fn reorder_thread_pins(
        &self,
        ordered_ids: Vec<String>,
        expected_revision: i64,
    ) -> GaryxDbResult<ReorderThreadPinsResult> {
        let ordered_ids = normalize_thread_pin_order(ordered_ids)?;
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let current = read_thread_pins_page_tx(&tx)?;
        if current.revision != expected_revision {
            tx.commit()?;
            return Ok(ReorderThreadPinsResult::Conflict(current));
        }

        let current_ids = current
            .pins
            .iter()
            .map(|pin| pin.thread_id.as_str())
            .collect::<BTreeSet<_>>();
        let requested_ids = ordered_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let mut next_order = Vec::with_capacity(current.pins.len());
        for thread_id in &ordered_ids {
            if current_ids.contains(thread_id.as_str()) {
                next_order.push(thread_id.clone());
            }
        }
        for pin in &current.pins {
            if !requested_ids.contains(pin.thread_id.as_str()) {
                next_order.push(pin.thread_id.clone());
            }
        }

        {
            let mut stmt = tx.prepare(
                "UPDATE thread_pins
                    SET sort_order = ?1
                  WHERE thread_id = ?2",
            )?;
            for (index, thread_id) in next_order.iter().enumerate() {
                let sort_order = i64::try_from(index).map_err(|_| {
                    GaryxDbError::BadRequest("too many thread_ids to reorder".to_owned())
                })?;
                stmt.execute(params![sort_order, thread_id])?;
            }
        }
        bump_thread_pins_revision_if_changed_tx(&tx, true)?;
        let page = read_thread_pins_page_tx(&tx)?;
        tx.commit()?;
        Ok(ReorderThreadPinsResult::Updated(page))
    }
}
