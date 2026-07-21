//! Thread favorites: pages, snapshots, and revision fencing.

use super::*;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FavoriteThreadRecord {
    pub thread_id: String,
    pub favorited_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ThreadFavoritesPage {
    pub favorites: Vec<FavoriteThreadRecord>,
    pub revision: i64,
    pub store_incarnation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FavoriteThreadResult {
    Updated {
        changed: bool,
        page: ThreadFavoritesPage,
    },
    Conflict(ThreadFavoritesPage),
    WrongIncarnation(ThreadFavoritesPage),
    NotFound(ThreadFavoritesPage),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadFavoritesSnapshot {
    pub page: ThreadFavoritesPage,
    pub recent_threads: Vec<RecentThreadRecord>,
    pub recent_total: usize,
    pub recent_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadFavoritesSummarySnapshot {
    pub snapshot: ThreadFavoritesSnapshot,
    pub summaries: Vec<ThreadSummaryRow>,
    pub summaries_truncated: bool,
}

pub const THREAD_FAVORITES_SNAPSHOT_CAP: usize = 500;

pub(super) fn read_thread_favorites_tx(
    conn: &Connection,
) -> GaryxDbResult<Vec<FavoriteThreadRecord>> {
    let mut stmt = conn.prepare(
        "SELECT thread_id, favorited_at
           FROM thread_favorites
          ORDER BY favorited_at DESC, thread_id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(FavoriteThreadRecord {
            thread_id: row.get(0)?,
            favorited_at: row.get(1)?,
        })
    })?;
    let mut favorites = Vec::new();
    for row in rows {
        favorites.push(row?);
    }
    Ok(favorites)
}

pub(super) fn read_thread_favorites_revision_tx(conn: &Connection) -> GaryxDbResult<i64> {
    Ok(conn.query_row(
        "SELECT favorites_revision FROM thread_favorites_meta WHERE id = 1",
        [],
        |row| row.get(0),
    )?)
}

pub(super) fn read_thread_favorites_page_with_rows_tx(
    conn: &Connection,
    favorites: Vec<FavoriteThreadRecord>,
) -> GaryxDbResult<ThreadFavoritesPage> {
    Ok(ThreadFavoritesPage {
        favorites,
        revision: read_thread_favorites_revision_tx(conn)?,
        store_incarnation_id: read_store_incarnation_id(conn)?,
    })
}

pub(super) fn read_thread_favorites_page_tx(
    conn: &Connection,
) -> GaryxDbResult<ThreadFavoritesPage> {
    read_thread_favorites_page_with_rows_tx(conn, read_thread_favorites_tx(conn)?)
}

pub(super) fn bump_thread_favorites_revision_tx(conn: &Connection) -> GaryxDbResult<()> {
    let updated = conn.execute(
        "UPDATE thread_favorites_meta
            SET favorites_revision = favorites_revision + 1
          WHERE id = 1",
        [],
    )?;
    if updated != 1 {
        return Err(GaryxDbError::Configuration(
            "thread_favorites_meta singleton is missing".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn bump_thread_favorites_revision_if_changed_tx(
    conn: &Connection,
    changed: bool,
) -> GaryxDbResult<()> {
    if changed {
        bump_thread_favorites_revision_tx(conn)?;
    }
    Ok(())
}

impl GaryxDbService {
    pub fn list_thread_favorites(&self) -> GaryxDbResult<ThreadFavoritesPage> {
        self.list_thread_favorites_inner(|| Ok(()))
    }

    pub(super) fn list_thread_favorites_inner<F>(
        &self,
        after_favorites: F,
    ) -> GaryxDbResult<ThreadFavoritesPage>
    where
        F: FnOnce() -> GaryxDbResult<()>,
    {
        let mut conn = self.read_conn()?;
        let tx = conn.transaction()?;
        let favorites = read_thread_favorites_tx(&tx)?;

        // Deterministic WAL seam: the identity and revision below must stay
        // on the same snapshot even if another writer commits here.
        after_favorites()?;

        let page = read_thread_favorites_page_with_rows_tx(&tx, favorites)?;
        tx.commit()?;
        Ok(page)
    }

    pub fn set_thread_favorite(
        &self,
        thread_id: &str,
        favorited: bool,
        expected_revision: i64,
        expected_store_incarnation: &str,
    ) -> GaryxDbResult<FavoriteThreadResult> {
        let thread_id = normalize_thread_id(thread_id)?;
        if expected_revision < 0 {
            return Err(GaryxDbError::BadRequest(
                "expected_revision must be a non-negative integer".to_owned(),
            ));
        }
        let expected_store_incarnation = Uuid::parse_str(expected_store_incarnation)
            .map(|uuid| uuid.to_string())
            .map_err(|_| {
                GaryxDbError::BadRequest("expected_store_incarnation must be a UUID".to_owned())
            })?;
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;

        // Identity is the outer CAS fence: an old revision must never become
        // usable merely because a restored store happens to reuse its value.
        let current_incarnation = read_store_incarnation_id(&tx)?;
        if current_incarnation != expected_store_incarnation {
            let page = read_thread_favorites_page_tx(&tx)?;
            tx.commit()?;
            return Ok(FavoriteThreadResult::WrongIncarnation(page));
        }
        let current_revision = read_thread_favorites_revision_tx(&tx)?;
        if current_revision != expected_revision {
            let page = read_thread_favorites_page_tx(&tx)?;
            tx.commit()?;
            return Ok(FavoriteThreadResult::Conflict(page));
        }

        let changed = if favorited {
            let favorited_at = now_string();
            let inserted = tx.execute(
                "INSERT INTO thread_favorites (thread_id, favorited_at)
                 SELECT ?1, ?2
                  WHERE EXISTS (SELECT 1 FROM thread_records WHERE key = ?1)
                 ON CONFLICT(thread_id) DO NOTHING",
                params![thread_id, favorited_at],
            )? > 0;
            if !inserted && !thread_record_exists_tx(&tx, &thread_id)? {
                let page = read_thread_favorites_page_tx(&tx)?;
                tx.commit()?;
                return Ok(FavoriteThreadResult::NotFound(page));
            }
            inserted
        } else {
            if !thread_record_exists_tx(&tx, &thread_id)? {
                let page = read_thread_favorites_page_tx(&tx)?;
                tx.commit()?;
                return Ok(FavoriteThreadResult::NotFound(page));
            }
            tx.execute(
                "DELETE FROM thread_favorites WHERE thread_id = ?1",
                params![thread_id],
            )? > 0
        };

        // Every accepted conditional write advances the fence, including an
        // idempotent repeated PUT or no-op DELETE.
        bump_thread_favorites_revision_tx(&tx)?;
        let page = read_thread_favorites_page_tx(&tx)?;
        tx.commit()?;
        Ok(FavoriteThreadResult::Updated { changed, page })
    }

    pub fn thread_favorites_snapshot(&self) -> GaryxDbResult<ThreadFavoritesSnapshot> {
        self.thread_favorites_snapshot_inner(|| Ok(()))
    }

    pub fn thread_favorites_snapshot_with_summaries(
        &self,
    ) -> GaryxDbResult<ThreadFavoritesSummarySnapshot> {
        let (snapshot, summaries) = self.thread_favorites_snapshot_with_options(true, || Ok(()))?;
        let (summaries, summaries_truncated) =
            summaries.expect("enhanced favorites snapshot always computes its summary window");
        Ok(ThreadFavoritesSummarySnapshot {
            snapshot,
            summaries,
            summaries_truncated,
        })
    }

    pub(super) fn thread_favorites_snapshot_inner<F>(
        &self,
        after_favorites: F,
    ) -> GaryxDbResult<ThreadFavoritesSnapshot>
    where
        F: FnOnce() -> GaryxDbResult<()>,
    {
        self.thread_favorites_snapshot_with_options(false, after_favorites)
            .map(|(snapshot, _)| snapshot)
    }

    pub(super) fn thread_favorites_snapshot_with_options<F>(
        &self,
        include_summaries: bool,
        after_favorites: F,
    ) -> GaryxDbResult<(
        ThreadFavoritesSnapshot,
        Option<(Vec<ThreadSummaryRow>, bool)>,
    )>
    where
        F: FnOnce() -> GaryxDbResult<()>,
    {
        let mut conn = self.read_conn()?;
        let tx = conn.transaction()?;
        let favorites = read_thread_favorites_tx(&tx)?;
        let page = read_thread_favorites_page_with_rows_tx(&tx, favorites)?;

        // The joined recent rows and membership page are one atomic read
        // unit. A commit here must be invisible until the next snapshot.
        after_favorites()?;

        let recent_total: i64 = tx.query_row(
            "SELECT COUNT(*)
               FROM recent_threads AS recent
               JOIN thread_favorites AS favorite
                 ON favorite.thread_id = recent.thread_id",
            [],
            |row| row.get(0),
        )?;
        let recent_total = usize::try_from(recent_total).unwrap_or(usize::MAX);
        let mut stmt = tx.prepare(
            "SELECT recent.thread_id, recent.title, recent.workspace_dir,
                    recent.thread_type, recent.provider_type, recent.agent_id,
                    recent.message_count, recent.last_message_preview,
                    recent.recent_run_id, recent.active_run_id, recent.run_state,
                    recent.updated_at, recent.last_active_at, recent.activity_seq,
                    recent.recorded_at
               FROM recent_threads AS recent
               JOIN thread_favorites AS favorite
                 ON favorite.thread_id = recent.thread_id
              ORDER BY recent.activity_seq DESC
              LIMIT ?1",
        )?;
        let rows = stmt.query_map(
            params![i64::try_from(THREAD_FAVORITES_SNAPSHOT_CAP).unwrap_or(i64::MAX)],
            recent_thread_record_from_row,
        )?;
        let mut recent_threads = Vec::new();
        for row in rows {
            recent_threads.push(row?);
        }
        drop(stmt);
        let summaries = if include_summaries {
            let summaries_truncated = page.favorites.len() > THREAD_FAVORITES_SNAPSHOT_CAP;
            let mut stmt = tx.prepare(
                "WITH summary_window AS (
                    SELECT favorite.thread_id,
                           recent.activity_seq,
                           favorite.favorited_at,
                           CASE WHEN recent.thread_id IS NULL THEN 1 ELSE 0 END AS raw_segment
                      FROM thread_favorites AS favorite
                      LEFT JOIN recent_threads AS recent
                        ON recent.thread_id = favorite.thread_id
                     ORDER BY raw_segment ASC,
                              recent.activity_seq DESC,
                              favorite.favorited_at DESC,
                              favorite.thread_id ASC
                     LIMIT ?1
                 )
                 SELECT meta.thread_id, meta.thread_label, meta.workspace_dir,
                        meta.thread_type, meta.provider_type, meta.agent_id,
                        meta.created_at, meta.updated_at, meta.message_count,
                        meta.last_user_message, meta.last_assistant_message,
                        meta.last_message_preview, meta.recent_run_id,
                        meta.active_run_id, meta.worktree_json,
                        meta.root_workspace_path, meta.workspace_origin,
                        meta.sort_updated_at_us
                   FROM summary_window AS member
                   JOIN thread_meta AS meta ON meta.thread_id = member.thread_id
                  WHERE meta.default_list_hidden = 0
                  ORDER BY member.raw_segment ASC,
                           member.activity_seq DESC,
                           member.favorited_at DESC,
                           member.thread_id ASC",
            )?;
            let rows = stmt.query_map(
                params![i64::try_from(THREAD_FAVORITES_SNAPSHOT_CAP).unwrap_or(i64::MAX)],
                thread_summary_row_from_row,
            )?;
            let mut summaries = Vec::new();
            for row in rows {
                summaries.push(row?);
            }
            drop(stmt);
            Some((summaries, summaries_truncated))
        } else {
            None
        };
        tx.commit()?;
        Ok((
            ThreadFavoritesSnapshot {
                page,
                recent_truncated: recent_total > recent_threads.len(),
                recent_total,
                recent_threads,
            },
            summaries,
        ))
    }
}
