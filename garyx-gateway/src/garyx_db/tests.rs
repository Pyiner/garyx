use super::*;
use chrono::DateTime;
use serde_json::json;

/// A read query slow enough (tens of ms) to make lock serialization
/// visible to the wall clock.
fn run_slow_read(conn: &Connection) -> u128 {
    let started = std::time::Instant::now();
    let _: i64 = conn
            .query_row(
                "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt WHERE x < 3000000) SELECT count(*) FROM cnt",
                [],
                |row| row.get(0),
            )
            .expect("slow read");
    started.elapsed().as_millis()
}

#[test]
fn concurrent_reads_run_in_parallel_across_the_pool() {
    let dir = tempfile::tempdir().expect("temp dir");
    let service = std::sync::Arc::new(
        GaryxDbService::open(dir.path().join("garyx-db.sqlite3")).expect("db opens"),
    );

    // Hold every acquired connection until the main thread releases the
    // gate. This proves the pool exposes four independent connections
    // without depending on CPU throughput or wall-clock ratios while the
    // rest of the test binary is running in parallel.
    let readers = READ_POOL_SIZE;
    let start = std::sync::Arc::new(std::sync::Barrier::new(readers + 1));
    let release = std::sync::Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
    let handles: Vec<_> = (0..readers)
        .map(|_| {
            let service = std::sync::Arc::clone(&service);
            let start = std::sync::Arc::clone(&start);
            let release = std::sync::Arc::clone(&release);
            let acquired_tx = acquired_tx.clone();
            std::thread::spawn(move || {
                start.wait();
                let conn = service.read_conn().expect("read conn");
                let _: i64 = conn
                    .query_row("SELECT 1", [], |row| row.get(0))
                    .expect("read query");
                acquired_tx.send(()).expect("report acquired connection");
                let (released, wake) = &*release;
                let mut released = released.lock().expect("release lock");
                while !*released {
                    released = wake.wait(released).expect("release wait");
                }
            })
        })
        .collect();
    drop(acquired_tx);
    start.wait();

    let mut acquired = 0;
    while acquired < readers
        && acquired_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .is_ok()
    {
        acquired += 1;
    }
    {
        let (released, wake) = &*release;
        *released.lock().expect("release lock") = true;
        wake.notify_all();
    }
    for handle in handles {
        handle.join().expect("reader thread");
    }

    assert_eq!(
        acquired, readers,
        "readers did not acquire every pool connection concurrently"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn blocking_entry_keeps_the_runtime_responsive() {
    // One runtime worker: if database work runs ON the worker (the old
    // direct-call shape), the heartbeat below cannot tick until the DB
    // call finishes. Through `run_blocking` the worker stays free.
    let dir = tempfile::tempdir().expect("temp dir");
    let service = std::sync::Arc::new(
        GaryxDbService::open(dir.path().join("garyx-db.sqlite3")).expect("db opens"),
    );

    let ticks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let heartbeat = {
        let ticks = std::sync::Arc::clone(&ticks);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                ticks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        })
    };

    service
        .run_blocking(|db| {
            let conn = db.read_conn()?;
            run_slow_read(&conn);
            Ok(())
        })
        .await
        .expect("blocking read");

    heartbeat.abort();
    let observed = ticks.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        observed >= 3,
        "runtime worker was starved during database work: {observed} heartbeat ticks"
    );
}

#[test]
fn open_configures_wal_normal_synchronous_and_busy_timeout() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garyx-db.sqlite3");

    let service = GaryxDbService::open(&path).expect("db opens");
    {
        let conn = service.conn().expect("conn");
        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("journal_mode");
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        let synchronous: i64 = conn
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .expect("synchronous");
        assert_eq!(synchronous, 1, "synchronous should be NORMAL (1)");
        let busy_timeout: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .expect("busy_timeout");
        assert_eq!(busy_timeout, BUSY_TIMEOUT.as_millis() as i64);
    }
    drop(service);

    // WAL is a persistent database property: a reopen must still be WAL.
    let reopened = GaryxDbService::open(&path).expect("db reopens");
    let conn = reopened.conn().expect("conn");
    let journal_mode: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("journal_mode");
    assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
}

#[test]
fn file_store_incarnation_is_uuid_stable_on_reopen_and_rotates_only_explicitly() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garyx-db.sqlite3");
    let service = GaryxDbService::open(&path).expect("first open");
    let first = service.store_incarnation_id().expect("first identity");
    assert_eq!(Uuid::parse_str(&first).unwrap().to_string(), first);
    drop(service);

    let reopened = GaryxDbService::open(&path).expect("ordinary reopen");
    assert_eq!(reopened.store_incarnation_id().unwrap(), first);
    let rotated = reopened
        .rotate_store_incarnation()
        .expect("explicit offline rotation")
        .store_incarnation_id;
    assert_ne!(rotated, first);
    drop(reopened);

    let after_rotation = GaryxDbService::open(&path).expect("reopen after rotation");
    assert_eq!(after_rotation.store_incarnation_id().unwrap(), rotated);
}

#[test]
fn data_dir_lock_precedes_schema_initialization_is_cloexec_and_times_out_boundedly() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garyx-db.sqlite3");
    Connection::open(&path)
        .expect("seed raw database")
        .execute("CREATE TABLE untouched(value INTEGER)", [])
        .expect("seed sentinel schema");

    let owner = DataDirLock::acquire(&path, Duration::ZERO).expect("own data dir");
    assert!(owner.close_on_exec().expect("CLOEXEC query"));
    let started = Instant::now();
    let error = GaryxDbService::open_with_lock_wait(&path, Duration::from_millis(80))
        .err()
        .expect("second gateway must time out");
    assert!(matches!(error, GaryxDbError::DataDirLocked { .. }));
    assert!(
        started.elapsed() >= Duration::from_millis(70),
        "lock wait returned before its bounded deadline"
    );

    let raw = Connection::open(&path).expect("inspect untouched database");
    assert!(!sqlite_table_exists(&raw, "garyx_store_meta").unwrap());
    assert!(sqlite_table_exists(&raw, "untouched").unwrap());
    drop(raw);
    drop(owner);

    let service = GaryxDbService::open_with_lock_wait(&path, Duration::ZERO)
        .expect("lock release permits startup");
    assert!(service.store_incarnation_id().is_ok());
}

#[test]
fn data_dir_lock_waiter_continues_after_old_gateway_releases_for_restart_fallback() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garyx-db.sqlite3");
    let old_gateway = GaryxDbService::open(&path).expect("old gateway owns lock");
    let waiter_path = path.clone();
    let waiter = std::thread::spawn(move || {
        GaryxDbService::open_with_lock_wait(waiter_path, Duration::from_secs(2))
    });

    std::thread::sleep(Duration::from_millis(100));
    assert!(!waiter.is_finished(), "new gateway skipped the held lock");
    drop(old_gateway);
    let new_gateway = waiter
        .join()
        .expect("waiter thread")
        .expect("new gateway takes released lock");
    assert!(new_gateway.store_incarnation_id().is_ok());
}

#[test]
fn pre_r5_parent_handoff_has_continue_and_fail_closed_branches() {
    let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let release = alive.clone();
    let exiting_parent = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(60));
        release.store(false, std::sync::atomic::Ordering::Release);
    });
    wait_for_parent_exit(4242, Duration::from_secs(1), || {
        alive.load(std::sync::atomic::Ordering::Acquire)
    })
    .expect("startup continues after parent exits");
    exiting_parent.join().unwrap();

    let error = wait_for_parent_exit(4243, Duration::from_millis(70), || true)
        .expect_err("live parent at cap must fail closed");
    assert!(matches!(
        error,
        GaryxDbError::ParentHandoffTimedOut {
            parent_pid: 4243,
            ..
        }
    ));

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garyx-db.sqlite3");
    let raw = Connection::open(&path).expect("raw pre-R5 database");
    raw.execute("CREATE TABLE untouched(value INTEGER)", [])
        .unwrap();
    drop(raw);
    let lock = DataDirLock::acquire(&path, Duration::ZERO).expect("new binary lock");
    let barrier = wait_for_parent_exit(4244, Duration::from_millis(60), || true);
    assert!(barrier.is_err());
    drop(lock);
    let raw = Connection::open(&path).expect("inspect after failed handoff");
    assert!(sqlite_table_exists(&raw, "untouched").unwrap());
    assert!(
        !sqlite_table_exists(&raw, "garyx_store_meta").unwrap(),
        "fail-closed parent timeout must precede destructive/schema initialization"
    );
    drop(raw);
    DataDirLock::acquire(&path, Duration::ZERO).expect("failed child released the data-dir lock");
}

#[cfg(unix)]
#[test]
fn parent_executable_resolution_failure_is_fail_closed() {
    let error = parent_has_same_executable_name_with(4242, |_| {
        Err(GaryxDbError::Configuration(
            "synthetic ps failure".to_owned(),
        ))
    })
    .expect_err("an unknown parent executable must abort startup");
    assert!(matches!(error, GaryxDbError::Configuration(_)));
}

#[test]
fn read_only_handle_queries_during_a_writer_lock_and_rejects_writes() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garyx-db.sqlite3");
    let service = GaryxDbService::open(&path).expect("create database");
    service
        .upsert_recent_thread(RecentThreadDraft {
            thread_id: "thread::read-only-snapshot".to_owned(),
            title: "Read only snapshot".to_owned(),
            workspace_dir: None,
            thread_type: "chat".to_owned(),
            provider_type: None,
            agent_id: None,
            message_count: 0,
            last_message_preview: String::new(),
            recent_run_id: None,
            active_run_id: Some("run::active".to_owned()),
            run_state: "running".to_owned(),
            updated_at: None,
            last_active_at: "2026-07-14T00:00:00Z".to_owned(),
        })
        .expect("seed recent projection");

    let writer = Connection::open(&path).expect("writer connection");
    writer
        .execute_batch("BEGIN IMMEDIATE;")
        .expect("hold the database write lock");

    let mut read_only = ReadOnlyGaryxDb::open(&path).expect("open read-only handle");
    let query_only: i64 = read_only
        .conn
        .query_row("PRAGMA query_only", [], |row| row.get(0))
        .expect("query_only pragma");
    assert_eq!(query_only, 1);
    let page = read_only
        .list_active_recent_thread_ids(16)
        .expect("WAL reader remains available during a write transaction");
    assert_eq!(page.thread_ids, vec!["thread::read-only-snapshot"]);

    writer.execute_batch("COMMIT;").expect("release write lock");
    let error = read_only
        .conn
        .execute("DELETE FROM recent_threads", [])
        .expect_err("read-only handle must reject writes");
    assert_eq!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::ReadOnly),
        "unexpected write error: {error}"
    );
}

fn sample_recent_draft(thread_id: &str) -> RecentThreadDraft {
    RecentThreadDraft {
        thread_id: thread_id.to_owned(),
        title: "Sample".to_owned(),
        workspace_dir: None,
        thread_type: "chat".to_owned(),
        provider_type: None,
        agent_id: None,
        message_count: 1,
        last_message_preview: "hello".to_owned(),
        recent_run_id: None,
        active_run_id: None,
        run_state: "idle".to_owned(),
        updated_at: None,
        last_active_at: "2026-07-08T00:00:00Z".to_owned(),
    }
}

#[test]
fn recent_activity_schema_initializes_before_writes_and_reopens_stably() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garyx-db.sqlite3");
    {
        let conn = Connection::open(&path).expect("legacy db");
        conn.execute_batch(
            "CREATE TABLE recent_threads (
                     thread_id TEXT PRIMARY KEY,
                     title TEXT NOT NULL DEFAULT '',
                     workspace_dir TEXT,
                     thread_type TEXT NOT NULL DEFAULT 'chat',
                     provider_type TEXT,
                     agent_id TEXT,
                     message_count INTEGER NOT NULL DEFAULT 0,
                     last_message_preview TEXT NOT NULL DEFAULT '',
                     recent_run_id TEXT,
                     active_run_id TEXT,
                     run_state TEXT NOT NULL DEFAULT 'idle',
                     updated_at TEXT,
                     last_active_at TEXT NOT NULL,
                     recorded_at TEXT NOT NULL
                 ) STRICT;
                 INSERT INTO recent_threads (
                     thread_id, last_active_at, recorded_at
                 ) VALUES (
                     'thread::legacy-before-seq',
                     '2026-07-01T00:00:00Z',
                     '2026-07-01T00:00:00Z'
                 );",
        )
        .expect("seed legacy recent table");
    }

    let db = GaryxDbService::open(&path).expect("open upgraded db");
    let conn = db.conn().expect("writer");
    let meta: i64 = conn
        .query_row(
            "SELECT activity_seq FROM recent_threads_meta WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .expect("meta initialized during schema open");
    let legacy_seq: i64 = conn
        .query_row(
            "SELECT activity_seq FROM recent_threads
                  WHERE thread_id = 'thread::legacy-before-seq'",
            [],
            |row| row.get(0),
        )
        .expect("legacy column added during schema open");
    assert_eq!((meta, legacy_seq), (0, 0));
    drop(conn);
    drop(db);

    let reopened = GaryxDbService::open(&path).expect("reopen upgraded db");
    let conn = reopened.conn().expect("writer after reopen");
    assert_eq!(
        conn.query_row(
            "SELECT activity_seq FROM recent_threads_meta WHERE id = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        0,
        "ordinary reopen must not move the activity high-water mark"
    );
}

#[test]
fn recent_activity_backfill_preserves_old_order_and_is_truly_one_shot() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garyx-db.sqlite3");
    {
        let conn = Connection::open(&path).expect("legacy db");
        conn.execute_batch(
            "CREATE TABLE recent_threads (
                     thread_id TEXT PRIMARY KEY,
                     title TEXT NOT NULL DEFAULT '',
                     workspace_dir TEXT,
                     thread_type TEXT NOT NULL DEFAULT 'chat',
                     provider_type TEXT,
                     agent_id TEXT,
                     message_count INTEGER NOT NULL DEFAULT 0,
                     last_message_preview TEXT NOT NULL DEFAULT '',
                     recent_run_id TEXT,
                     active_run_id TEXT,
                     run_state TEXT NOT NULL DEFAULT 'idle',
                     updated_at TEXT,
                     last_active_at TEXT NOT NULL,
                     recorded_at TEXT NOT NULL
                 ) STRICT;
                 INSERT INTO recent_threads (
                     thread_id, last_active_at, recorded_at
                 ) VALUES
                     ('thread::z-old', '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z'),
                     ('thread::b-tie', '2026-07-02T00:00:00Z', '2026-07-02T00:00:00Z'),
                     ('thread::a-tie', '2026-07-02T00:00:00Z', '2026-07-02T00:00:00Z');",
        )
        .expect("seed legacy order");
    }

    let db = GaryxDbService::open(&path).expect("open upgraded db");
    db.conn()
        .unwrap()
        .execute(
            "UPDATE recent_threads_meta SET activity_seq = 50 WHERE id = 1",
            [],
        )
        .unwrap();
    let first = db
        .migrate_recent_thread_activity_seq_v1()
        .expect("backfill activity sequence");
    assert_eq!(first.source_row_count, 3);
    assert_eq!(first.updated_row_count, 3);
    assert!(!first.already_completed);

    let rows = db
        .list_recent_threads(10, 0)
        .expect("list migrated recent rows");
    assert_eq!(
        rows.iter()
            .map(|row| (row.thread_id.as_str(), row.activity_seq))
            .collect::<Vec<_>>(),
        vec![
            ("thread::a-tie", 53),
            ("thread::b-tie", 52),
            ("thread::z-old", 51),
        ],
        "descending seq must exactly preserve the former timestamp/id order"
    );
    let conn = db.conn().expect("writer");
    assert_eq!(
        conn.query_row(
            "SELECT activity_seq FROM recent_threads_meta WHERE id = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        53,
        "backfill must floor against and then advance the existing meta"
    );
    assert!(
        conn.execute(
            "INSERT INTO recent_threads (
                     thread_id, last_active_at, activity_seq, recorded_at
                 ) VALUES (
                     'thread::duplicate-seq', '2026-07-03T00:00:00Z', 53,
                     '2026-07-03T00:00:00Z'
                 )",
            [],
        )
        .is_err(),
        "the post-backfill activity sequence index must be unique"
    );
    drop(conn);

    let second = db
        .migrate_recent_thread_activity_seq_v1()
        .expect("one-shot rerun");
    assert!(second.already_completed);
    assert_eq!(second.source_row_count, 3);
    assert_eq!(second.updated_row_count, 0);
    assert_eq!(
        db.list_recent_threads(10, 0)
            .unwrap()
            .iter()
            .map(|row| row.activity_seq)
            .collect::<Vec<_>>(),
        vec![53, 52, 51]
    );
    drop(db);

    let reopened = GaryxDbService::open(&path).expect("reopen migrated db");
    assert!(
        reopened
            .migrate_recent_thread_activity_seq_v1()
            .unwrap()
            .already_completed
    );
    assert_eq!(
        reopened.list_recent_threads(10, 0).unwrap()[0].activity_seq,
        53
    );
}

#[test]
fn recent_activity_allocator_is_transactional_strict_and_safe_integer_bounded() {
    let db = std::sync::Arc::new(GaryxDbService::memory().expect("memory db"));

    {
        let mut conn = db.conn().expect("writer");
        let tx = conn.transaction().expect("transaction");
        let record = upsert_recent_thread_tx(
            &tx,
            sample_recent_draft("thread::rolled-back-seq"),
            "2026-07-16T00:00:00Z",
        )
        .expect("upsert inside uncommitted transaction");
        assert_eq!(record.activity_seq, 1);
        drop(tx);
    }
    assert!(db.list_recent_threads(10, 0).unwrap().is_empty());
    assert_eq!(
        db.conn()
            .unwrap()
            .query_row(
                "SELECT activity_seq FROM recent_threads_meta WHERE id = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0,
        "allocator and projection upsert must roll back together"
    );

    let handles = (0..24)
        .map(|index| {
            let db = std::sync::Arc::clone(&db);
            std::thread::spawn(move || {
                db.upsert_recent_thread(sample_recent_draft(&format!(
                    "thread::concurrent-seq-{index:02}"
                )))
                .expect("concurrent upsert")
                .activity_seq
            })
        })
        .collect::<Vec<_>>();
    let mut allocated = handles
        .into_iter()
        .map(|handle| handle.join().expect("allocator thread"))
        .collect::<Vec<_>>();
    allocated.sort_unstable();
    assert_eq!(allocated, (1..=24).collect::<Vec<_>>());

    let first = db
        .upsert_recent_thread(sample_recent_draft("thread::moves-to-head"))
        .unwrap();
    let second = db
        .upsert_recent_thread(sample_recent_draft("thread::other-head"))
        .unwrap();
    let moved = db
        .upsert_recent_thread(sample_recent_draft("thread::moves-to-head"))
        .unwrap();
    assert!(first.activity_seq < second.activity_seq);
    assert!(second.activity_seq < moved.activity_seq);
    assert_eq!(
        db.list_recent_threads(2, 0).unwrap()[0].thread_id,
        "thread::moves-to-head",
        "every read-modify-write upsert gets a fresh monotonic ordering key"
    );

    let conn = db.conn().expect("writer");
    assert!(
        conn.execute(
            "UPDATE recent_threads_meta SET activity_seq = 9007199254740991 WHERE id = 1",
            [],
        )
        .is_err(),
        "meta must reject values that are not exactly representable as desktop integers"
    );
    assert!(
        conn.execute(
            "UPDATE recent_threads SET activity_seq = 9007199254740991
                  WHERE thread_id = 'thread::moves-to-head'",
            [],
        )
        .is_err(),
        "rows must enforce the same exclusive safe-integer bound"
    );
}

#[test]
fn recovery_generation_never_resets_activity_meta_or_one_shot_marker() {
    let db = GaryxDbService::memory().expect("memory db");
    db.migrate_recent_thread_activity_seq_v1()
        .expect("mark empty backfill complete");
    let older = db
        .upsert_recent_thread(sample_recent_draft("thread::before-recovery-older"))
        .unwrap();
    let old_head = db
        .upsert_recent_thread(sample_recent_draft("thread::before-recovery-head"))
        .unwrap();
    assert_eq!((older.activity_seq, old_head.activity_seq), (1, 2));

    assert_eq!(db.commit_legacy_import(0, false).unwrap(), 1);
    db.record_legacy_archive_retirement().unwrap();
    db.clear_projection_state(crate::legacy_boot_import::THREAD_RECORDS_IMPORT_NAME)
        .unwrap();
    assert_eq!(db.commit_legacy_import(2, true).unwrap(), 2);
    assert!(
        db.projection_state_exists(
            RECENT_THREAD_ACTIVITY_SEQ_MIGRATION_NAME,
            RECENT_THREAD_ACTIVITY_SEQ_MIGRATION_VERSION,
        )
        .unwrap(),
        "recovery generation changes must not clear the independent seq marker"
    );

    let recovered = db
        .upsert_recent_thread(sample_recent_draft("thread::recovery-import"))
        .unwrap();
    assert_eq!(recovered.activity_seq, 3);
    assert!(
        db.migrate_recent_thread_activity_seq_v1()
            .unwrap()
            .already_completed
    );
    let old_cursor_page = db
        .list_recent_threads_keyset_page(
            RecentThreadTaskFilter::Include,
            10,
            Some(old_head.activity_seq),
        )
        .expect("old cursor remains valid");
    assert_eq!(
        old_cursor_page
            .records
            .iter()
            .map(|row| (row.thread_id.as_str(), row.activity_seq))
            .collect::<Vec<_>>(),
        vec![("thread::before-recovery-older", 1)]
    );
}

#[test]
fn recent_membership_cutover_rebuilds_exact_membership_and_preserves_retained_order() {
    let db = GaryxDbService::memory().expect("memory db");
    prepare_recent_membership_prerequisites(&db);

    for (thread_id, body) in [
        (
            "thread::retained-z",
            json!({
                "thread_id": "thread::retained-z",
                "label": "Retained Z",
                "exclude_from_recent": true,
                "excludeFromRecent": true,
                "metadata": {
                    "exclude_from_recent": true,
                    "excludeFromRecent": true
                }
            }),
        ),
        (
            "thread::retained-a",
            json!({"thread_id": "thread::retained-a", "label": "Retained A"}),
        ),
        (
            "thread::new-missing",
            json!({"thread_id": "thread::new-missing", "label": "No time"}),
        ),
        (
            "thread::new-between",
            json!({
                "thread_id": "thread::new-between",
                "label": "Between",
                "updated_at": "2026-07-02T00:00:00Z",
                "automation_thread_mode": "generated_thread"
            }),
        ),
        (
            "thread::new-same",
            json!({
                "thread_id": "thread::new-same",
                "label": "Same timestamp",
                "updated_at": "2026-07-03T00:00:00Z"
            }),
        ),
        (
            "thread::new-latest",
            json!({
                "thread_id": "thread::new-latest",
                "label": "Latest",
                "updated_at": "2026-07-04T00:00:00Z"
            }),
        ),
        (
            "thread::hidden",
            json!({"thread_id": "thread::hidden", "hidden": true}),
        ),
        (
            "thread::side-chat",
            json!({
                "thread_id": "thread::side-chat",
                "source": "side_chat",
                "side_chat_parent_thread_id": "thread::retained-z"
            }),
        ),
    ] {
        seed_recent_membership_canonical(&db, thread_id, body);
    }
    seed_recent_membership_row(&db, "thread::retained-z", "2026-07-03T00:00:00Z", 10);
    seed_recent_membership_row(&db, "thread::retained-a", "2026-07-01T00:00:00Z", 20);
    // If this orphan were included in the insertion count, new-between
    // would move from bucket 1 to bucket 2 (after retained-a).
    seed_recent_membership_row(&db, "thread::orphan-slot", "2026-07-01T12:00:00Z", 30);
    seed_recent_membership_row(&db, "thread::hidden", "2026-07-05T00:00:00Z", 40);
    {
        let conn = db.conn().unwrap();
        conn.execute(
            "UPDATE recent_threads_meta SET activity_seq = 100 WHERE id = 1",
            [],
        )
        .unwrap();
    }

    let incarnation = db.store_incarnation_id().unwrap();
    assert!(matches!(
        db.set_thread_favorite("thread::retained-z", true, 0, &incarnation)
            .unwrap(),
        FavoriteThreadResult::Updated { .. }
    ));
    assert!(matches!(
        db.set_thread_favorite("thread::retained-a", true, 1, &incarnation)
            .unwrap(),
        FavoriteThreadResult::Updated { .. }
    ));
    assert_eq!(
        db.thread_favorites_snapshot()
            .unwrap()
            .recent_threads
            .iter()
            .map(|row| row.thread_id.as_str())
            .collect::<Vec<_>>(),
        vec!["thread::retained-a", "thread::retained-z"]
    );

    let first = db
        .migrate_recent_membership_v2()
        .expect("membership cutover");
    assert_eq!(first.source_row_count, 8);
    assert!(!first.already_completed);
    assert_eq!(
        db.store_incarnation_id().unwrap(),
        incarnation,
        "sequence-only cutover must not rotate favorites CAS identity"
    );
    let ascending = recent_membership_rows_ascending(&db);
    assert_eq!(
        ascending,
        vec![
            ("thread::new-missing".to_owned(), 101),
            ("thread::retained-z".to_owned(), 102),
            ("thread::new-between".to_owned(), 103),
            ("thread::new-same".to_owned(), 104),
            ("thread::retained-a".to_owned(), 105),
            ("thread::new-latest".to_owned(), 106),
        ],
        "retained order, count insertion, missing-time bottom, and same-time id tie"
    );
    assert_eq!(
        db.thread_favorites_snapshot()
            .unwrap()
            .recent_threads
            .iter()
            .map(|row| row.thread_id.as_str())
            .collect::<Vec<_>>(),
        vec!["thread::retained-a", "thread::retained-z"],
        "favorites retain their pre-cutover relative display order"
    );

    let conn = db.conn().unwrap();
    let (meta_high_water, row_high_water): (i64, i64) = conn
        .query_row(
            "SELECT
                    (SELECT activity_seq FROM recent_threads_meta WHERE id = 1),
                    (SELECT MAX(activity_seq) FROM recent_threads)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((meta_high_water, row_high_water), (106, 106));
    let index_rows = {
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master
                      WHERE type = 'index'
                        AND name IN (
                            'idx_recent_threads_activity_seq',
                            'idx_recent_threads_task_activity_seq',
                            'idx_recent_threads_non_task_activity_seq'
                        )
                      ORDER BY name",
            )
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    assert_eq!(index_rows.len(), 3);
    let unique: i64 = conn
        .query_row(
            "SELECT [unique] FROM pragma_index_list('recent_threads')
                  WHERE name = 'idx_recent_threads_activity_seq'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(unique, 1);
    let side_chat_body: String = conn
        .query_row(
            "SELECT body FROM thread_records WHERE key = 'thread::side-chat'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&side_chat_body).unwrap()["hidden"],
        true
    );
    let normalized_body: String = conn
        .query_row(
            "SELECT body FROM thread_records WHERE key = 'thread::retained-z'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_retired_recent_exclusion_paths_absent(
        &serde_json::from_str::<Value>(&normalized_body).unwrap(),
    );
    drop(conn);
    let mut parity_conn = db.conn().unwrap();
    let parity_tx = parity_conn.transaction().unwrap();
    assert_recent_membership_parity_tx(&parity_tx).unwrap();
    parity_tx.commit().unwrap();
    drop(parity_conn);

    let second = db
        .migrate_recent_membership_v2()
        .expect("idempotent cutover");
    assert!(second.already_completed);
    assert_eq!(second.updated_row_count, 0);
    assert_eq!(recent_membership_rows_ascending(&db), ascending);
}

#[test]
fn recent_membership_cutover_reruns_once_per_import_generation() {
    let db = GaryxDbService::memory().expect("memory db");
    prepare_recent_membership_prerequisites(&db);
    seed_recent_membership_canonical(
        &db,
        "thread::generation-zero",
        json!({
            "thread_id": "thread::generation-zero",
            "automation_thread_mode": "generated_thread"
        }),
    );
    assert!(!db.migrate_recent_membership_v2().unwrap().already_completed);
    assert!(db.migrate_recent_membership_v2().unwrap().already_completed);

    seed_recent_membership_canonical(
        &db,
        "thread::generation-one",
        json!({
            "thread_id": "thread::generation-one",
            "automation_thread_mode": "generated_thread"
        }),
    );
    db.conn()
        .unwrap()
        .execute(
            "INSERT INTO thread_meta (
                    thread_id, thread_label, projected_at
                 ) VALUES ('thread::generation-one', 'stale', '2026-07-17T00:00:00Z')",
            [],
        )
        .unwrap();
    assert_eq!(db.commit_legacy_import(2, false).unwrap(), 1);
    assert!(
        !db.migrate_thread_meta_summary_v1()
            .unwrap()
            .already_completed
    );
    let generation_one = db.migrate_recent_membership_v2().unwrap();
    assert!(!generation_one.already_completed);
    assert!(db.migrate_recent_membership_v2().unwrap().already_completed);

    let conn = db.conn().unwrap();
    let based_on: i64 = conn
        .query_row(
            "SELECT based_on_import_generation
                   FROM projection_states
                  WHERE projection_name = ?1",
            params![RECENT_MEMBERSHIP_MIGRATION_NAME],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(based_on, 1);
    drop(conn);
    assert_eq!(
        recent_membership_rows_ascending(&db)
            .into_iter()
            .map(|(thread_id, _)| thread_id)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "thread::generation-zero".to_owned(),
            "thread::generation-one".to_owned(),
        ])
    );
}

#[test]
fn canonical_exclusion_strip_v3_repairs_completed_v2_without_disturbing_state() {
    let db = GaryxDbService::memory().expect("memory db");
    db.record_projection_state(
        crate::legacy_boot_import::THREAD_RECORDS_IMPORT_NAME,
        crate::legacy_boot_import::THREAD_RECORDS_IMPORT_VERSION,
        2,
    )
    .expect("seed completed import");
    for (thread_id, body) in [
        (
            "thread::v3-flagged",
            json!({
                "thread_id": "thread::v3-flagged",
                "label": "Flagged"
            }),
        ),
        (
            "thread::v3-plain",
            json!({
                "thread_id": "thread::v3-plain",
                "label": "Plain"
            }),
        ),
    ] {
        seed_recent_membership_canonical(&db, thread_id, body);
    }
    prepare_recent_membership_prerequisites(&db);
    db.migrate_recent_membership_v2()
        .expect("seed completed v2");
    db.migrate_thread_meta_schema_v2()
        .expect("seed completed schema migration");

    let historical_body = json!({
        "thread_id": "thread::v3-flagged",
        "label": "Flagged",
        "exclude_from_recent": true,
        "excludeFromRecent": true,
        "metadata": {
            "safe": "preserved",
            "exclude_from_recent": "yes",
            "excludeFromRecent": false
        }
    });
    db.conn()
        .unwrap()
        .execute(
            "UPDATE thread_records SET body = ?1 WHERE key = 'thread::v3-flagged'",
            params![historical_body.to_string()],
        )
        .expect("recreate body left by the buggy historical v2");

    let marker_rows_before =
        projection_state_rows_except(&db, CANONICAL_EXCLUSION_STRIP_MIGRATION_NAME);
    let recent_before = recent_membership_rows_ascending(&db);
    let meta_before = db.list_thread_meta().expect("meta before repair");
    {
        let mut conn = db.conn().unwrap();
        let tx = conn.transaction().unwrap();
        assert_recent_membership_parity_tx(&tx).unwrap();
        tx.commit().unwrap();
    }

    let first = db
        .migrate_canonical_exclusion_strip_v3()
        .expect("repair completed v2");
    assert_eq!(first.source_row_count, 2);
    assert_eq!(first.updated_row_count, 1);
    assert!(!first.already_completed);
    let repaired: Value = serde_json::from_str(
        &db.get_thread_record_body("thread::v3-flagged")
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_retired_recent_exclusion_paths_absent(&repaired);
    assert_eq!(repaired["metadata"]["safe"], "preserved");
    assert_eq!(recent_membership_rows_ascending(&db), recent_before);
    assert_eq!(db.list_thread_meta().unwrap(), meta_before);
    assert_eq!(
        projection_state_rows_except(&db, CANONICAL_EXCLUSION_STRIP_MIGRATION_NAME),
        marker_rows_before,
        "v3 must not rewrite any historical marker tuple"
    );
    {
        let mut conn = db.conn().unwrap();
        let tx = conn.transaction().unwrap();
        assert_recent_membership_parity_tx(&tx).unwrap();
        tx.commit().unwrap();
    }
    let marker: (i64, i64, i64) = db
        .conn()
        .unwrap()
        .query_row(
            "SELECT projection_version, source_row_count,
                        based_on_import_generation
                   FROM projection_states
                  WHERE projection_name = ?1",
            params![CANONICAL_EXCLUSION_STRIP_MIGRATION_NAME],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(marker, (CANONICAL_EXCLUSION_STRIP_MIGRATION_VERSION, 2, 1));
    assert!(
        db.migrate_canonical_exclusion_strip_v3()
            .unwrap()
            .already_completed
    );

    db.record_legacy_archive_retirement().unwrap();
    db.clear_projection_state(crate::legacy_boot_import::THREAD_RECORDS_IMPORT_NAME)
        .unwrap();
    assert_eq!(db.commit_legacy_import(2, true).unwrap(), 2);
    assert!(
        !db.migrate_thread_meta_summary_v1()
            .unwrap()
            .already_completed
    );
    assert!(!db.migrate_recent_membership_v2().unwrap().already_completed);
    let generation_two = db.migrate_canonical_exclusion_strip_v3().unwrap();
    assert!(!generation_two.already_completed);
    assert_eq!(generation_two.updated_row_count, 0);
    let based_on_generation: i64 = db
        .conn()
        .unwrap()
        .query_row(
            "SELECT based_on_import_generation
                   FROM projection_states
                  WHERE projection_name = ?1",
            params![CANONICAL_EXCLUSION_STRIP_MIGRATION_NAME],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(based_on_generation, 2);
}

#[test]
fn canonical_exclusion_strip_v3_rolls_back_body_updates_and_marker_on_decode_failure() {
    let db = GaryxDbService::memory().expect("memory db");
    prepare_recent_membership_prerequisites(&db);
    seed_recent_membership_canonical(
        &db,
        "thread::v3-valid",
        json!({"thread_id": "thread::v3-valid"}),
    );
    db.migrate_recent_membership_v2()
        .expect("seed completed v2");
    let flagged_body = json!({
        "thread_id": "thread::v3-valid",
        "exclude_from_recent": true,
        "metadata": {"excludeFromRecent": true}
    });
    let conn = db.conn().unwrap();
    conn.execute(
        "UPDATE thread_records SET body = ?1 WHERE key = 'thread::v3-valid'",
        params![flagged_body.to_string()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO thread_records (key, body, updated_at, recorded_at)
             VALUES ('thread::v3-z-malformed', '{', NULL, '2026-07-17T00:00:00Z')",
        [],
    )
    .unwrap();
    drop(conn);

    assert!(db.migrate_canonical_exclusion_strip_v3().is_err());
    let after_failure: Value = serde_json::from_str(
        &db.get_thread_record_body("thread::v3-valid")
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(after_failure["exclude_from_recent"], true);
    assert!(
        !db.projection_state_exists(
            CANONICAL_EXCLUSION_STRIP_MIGRATION_NAME,
            CANONICAL_EXCLUSION_STRIP_MIGRATION_VERSION,
        )
        .unwrap(),
        "failed repair must not leave a marker"
    );

    db.conn()
        .unwrap()
        .execute(
            "DELETE FROM thread_records WHERE key = 'thread::v3-z-malformed'",
            [],
        )
        .unwrap();
    let retry = db.migrate_canonical_exclusion_strip_v3().unwrap();
    assert_eq!(retry.updated_row_count, 1);
    let repaired: Value = serde_json::from_str(
        &db.get_thread_record_body("thread::v3-valid")
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_retired_recent_exclusion_paths_absent(&repaired);
}

#[test]
fn recent_membership_cutover_high_water_uses_larger_existing_row_sequence() {
    let db = GaryxDbService::memory().expect("memory db");
    prepare_recent_membership_prerequisites(&db);
    seed_recent_membership_canonical(
        &db,
        "thread::row-high-retained",
        json!({"thread_id": "thread::row-high-retained"}),
    );
    seed_recent_membership_canonical(
        &db,
        "thread::row-high-new",
        json!({
            "thread_id": "thread::row-high-new",
            "updated_at": "2026-07-18T00:00:00Z"
        }),
    );
    seed_recent_membership_row(&db, "thread::row-high-retained", "2026-07-17T00:00:00Z", 50);
    db.conn()
        .unwrap()
        .execute(
            "UPDATE recent_threads_meta SET activity_seq = 10 WHERE id = 1",
            [],
        )
        .unwrap();

    db.migrate_recent_membership_v2().unwrap();
    assert_eq!(
        recent_membership_rows_ascending(&db),
        vec![
            ("thread::row-high-retained".to_owned(), 51),
            ("thread::row-high-new".to_owned(), 52),
        ]
    );
    let meta: i64 = db
        .conn()
        .unwrap()
        .query_row(
            "SELECT activity_seq FROM recent_threads_meta WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(meta, 52);
}

#[test]
fn recent_membership_cutover_safe_integer_failure_rolls_back_and_retries() {
    let db = GaryxDbService::memory().expect("memory db");
    prepare_recent_membership_prerequisites(&db);
    seed_recent_membership_canonical(
        &db,
        "thread::at-sequence-limit",
        json!({"thread_id": "thread::at-sequence-limit"}),
    );
    seed_recent_membership_canonical(
        &db,
        "thread::rollback-side-chat",
        json!({
            "thread_id": "thread::rollback-side-chat",
            "source": "side_chat",
            "exclude_from_recent": true,
            "metadata": {"excludeFromRecent": true}
        }),
    );
    db.conn()
        .unwrap()
        .execute(
            "UPDATE recent_threads_meta SET activity_seq = ?1 WHERE id = 1",
            params![MAX_RECENT_THREAD_ACTIVITY_SEQ_EXCLUSIVE - 1],
        )
        .unwrap();

    assert!(db.migrate_recent_membership_v2().is_err());
    let conn = db.conn().unwrap();
    let side_chat_body: String = conn
        .query_row(
            "SELECT body FROM thread_records WHERE key = 'thread::rollback-side-chat'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        serde_json::from_str::<Value>(&side_chat_body).unwrap()["exclude_from_recent"] == true,
        "failed cutover must roll back canonical exclusion stripping"
    );
    assert!(
        serde_json::from_str::<Value>(&side_chat_body)
            .unwrap()
            .get("hidden")
            .is_none(),
        "failed cutover must roll back canonical side-chat normalization"
    );
    let marker_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM projection_states WHERE projection_name = ?1",
            params![RECENT_MEMBERSHIP_MIGRATION_NAME],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(marker_count, 0);
    let index_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
                  WHERE type = 'index'
                    AND name IN (
                        'idx_recent_threads_activity_seq',
                        'idx_recent_threads_task_activity_seq',
                        'idx_recent_threads_non_task_activity_seq'
                    )",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(index_count, 3, "failed cutover restores dropped indexes");
    conn.execute(
        "UPDATE recent_threads_meta SET activity_seq = 0 WHERE id = 1",
        [],
    )
    .unwrap();
    drop(conn);
    assert!(!db.migrate_recent_membership_v2().unwrap().already_completed);
    let normalized_body: String = db
        .conn()
        .unwrap()
        .query_row(
            "SELECT body FROM thread_records WHERE key = 'thread::rollback-side-chat'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&normalized_body).unwrap()["hidden"],
        true
    );
    assert_retired_recent_exclusion_paths_absent(
        &serde_json::from_str::<Value>(&normalized_body).unwrap(),
    );
}

#[test]
fn recent_membership_cutover_registration_requires_summary_then_activity() {
    let db = GaryxDbService::memory().expect("memory db");
    assert!(db.migrate_recent_membership_v2().is_err());
    assert!(db.migrate_canonical_exclusion_strip_v3().is_err());
    db.migrate_thread_meta_summary_v1().unwrap();
    assert!(db.migrate_recent_membership_v2().is_err());
    db.migrate_recent_thread_activity_seq_v1().unwrap();
    assert!(!db.migrate_recent_membership_v2().unwrap().already_completed);
    assert!(
        !db.migrate_canonical_exclusion_strip_v3()
            .unwrap()
            .already_completed
    );
    assert!(
        db.migrate_canonical_exclusion_strip_v3()
            .unwrap()
            .already_completed
    );

    // The registered order inside `run_thread_data_startup_migrations` is
    // enforced behaviorally: the precondition errors above plus the
    // full-runner tests (`run_thread_data_startup_migrations().unwrap()` on
    // fresh databases elsewhere in this file) fail if any registration is
    // reordered against its runtime preconditions.
}

#[test]
fn recent_membership_canonical_normalizer_strips_all_exclusion_paths() {
    let mut cases = [
        ("top source", json!({"source": "side_chat"}), true),
        (
            "metadata source",
            json!({"metadata": {"source": "side_chat"}}),
            true,
        ),
        (
            "both sources",
            json!({"source": "side_chat", "metadata": {"source": "side_chat"}}),
            true,
        ),
        (
            "parent only",
            json!({"side_chat_parent_thread_id": "thread::parent"}),
            false,
        ),
        (
            "automation mode only",
            json!({"automation_thread_mode": "generated_thread"}),
            false,
        ),
        (
            "exclusion only",
            json!({
                "exclude_from_recent": true,
                "excludeFromRecent": true,
                "metadata": {
                    "exclude_from_recent": true,
                    "excludeFromRecent": true
                }
            }),
            false,
        ),
        (
            "malformed payload",
            json!({"source": 42, "metadata": ["side_chat"]}),
            false,
        ),
    ];
    for (label, data, expected_hidden) in &mut cases {
        normalize_recent_membership_canonical_record(data);
        assert_eq!(
            data.get("hidden").and_then(Value::as_bool),
            (*expected_hidden).then_some(true),
            "{label}"
        );
        assert_retired_recent_exclusion_paths_absent(data);
    }
}

fn prepare_recent_membership_prerequisites(service: &GaryxDbService) {
    service
        .migrate_thread_meta_summary_v1()
        .expect("summary prerequisite");
    service
        .migrate_recent_thread_activity_seq_v1()
        .expect("activity prerequisite");
}

fn assert_retired_recent_exclusion_paths_absent(data: &Value) {
    let object = data.as_object().expect("canonical object");
    assert!(object.get("exclude_from_recent").is_none());
    assert!(object.get("excludeFromRecent").is_none());
    if let Some(metadata) = object.get("metadata").and_then(Value::as_object) {
        assert!(metadata.get("exclude_from_recent").is_none());
        assert!(metadata.get("excludeFromRecent").is_none());
    }
}

fn projection_state_rows_except(
    service: &GaryxDbService,
    excluded_name: &str,
) -> Vec<(String, i64, i64, String, Option<i64>)> {
    let conn = service.conn().expect("projection state connection");
    let mut stmt = conn
        .prepare(
            "SELECT projection_name, projection_version, source_row_count,
                        projected_at, based_on_import_generation
                   FROM projection_states
                  WHERE projection_name != ?1
                  ORDER BY projection_name",
        )
        .expect("projection state query");
    stmt.query_map(params![excluded_name], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
        ))
    })
    .expect("projection state rows")
    .collect::<Result<Vec<_>, _>>()
    .expect("collect projection states")
}

fn seed_recent_membership_canonical(service: &GaryxDbService, thread_id: &str, body: Value) {
    service
        .conn()
        .unwrap()
        .execute(
            "INSERT INTO thread_records (key, body, updated_at, recorded_at)
                 VALUES (?1, ?2, NULL, '2026-07-17T00:00:00Z')",
            params![thread_id, body.to_string()],
        )
        .unwrap();
}

fn seed_recent_membership_row(
    service: &GaryxDbService,
    thread_id: &str,
    last_active_at: &str,
    activity_seq: i64,
) {
    service
        .conn()
        .unwrap()
        .execute(
            "INSERT INTO recent_threads (
                    thread_id, title, thread_type, last_active_at, activity_seq, recorded_at
                 ) VALUES (?1, ?1, 'chat', ?2, ?3, '2026-07-17T00:00:00Z')",
            params![thread_id, last_active_at, activity_seq],
        )
        .unwrap();
}

fn recent_membership_rows_ascending(service: &GaryxDbService) -> Vec<(String, i64)> {
    let conn = service.conn().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT thread_id, activity_seq
                   FROM recent_threads
                  ORDER BY activity_seq ASC",
        )
        .unwrap();
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn seed_favorite_thread(service: &GaryxDbService, thread_id: &str, recent: bool) {
    service
        .write_thread_record_with_projections(
            thread_id,
            &json!({"thread_id": thread_id}).to_string(),
            Some("2026-07-16T00:00:00Z"),
            None,
        )
        .expect("seed favorite thread record");
    if recent {
        service
            .upsert_recent_thread(sample_recent_draft(thread_id))
            .expect("seed favorite recent projection");
    }
}

fn seed_summary_favorite_tx(
    tx: &Transaction<'_>,
    thread_id: &str,
    favorited_at: &str,
    hidden: bool,
) {
    tx.execute(
        "INSERT INTO thread_favorites (thread_id, favorited_at) VALUES (?1, ?2)",
        params![thread_id, favorited_at],
    )
    .expect("seed favorite membership");
    tx.execute(
        "INSERT INTO thread_meta (
                thread_id, thread_label, default_list_hidden,
                sort_updated_at_us, search_text, projected_at
             ) VALUES (?1, ?2, ?3, 0, '', '2026-07-17T00:00:00Z')",
        params![
            thread_id,
            format!("Title for {thread_id}"),
            if hidden { 1 } else { 0 },
        ],
    )
    .expect("seed favorite summary");
}

fn seed_summary_recent_tx(tx: &Transaction<'_>, thread_id: &str, activity_seq: i64) {
    tx.execute(
        "INSERT INTO recent_threads (
                thread_id, title, thread_type, message_count, last_message_preview,
                run_state, last_active_at, activity_seq, recorded_at
             ) VALUES (?1, ?2, 'chat', 0, '', 'idle',
                       '2026-07-17T00:00:00Z', ?3, '2026-07-17T00:00:00Z')",
        params![thread_id, format!("Title for {thread_id}"), activity_seq],
    )
    .expect("seed recent favorite");
}

fn seed_task_kind_migration_row(
    service: &GaryxDbService,
    thread_id: &str,
    body: &str,
    has_task_projection: bool,
) {
    let conn = service.conn().expect("conn");
    conn.execute(
        "INSERT INTO thread_records (key, body, updated_at, recorded_at)
             VALUES (?1, ?2, '2026-07-01T00:00:00Z', '2026-07-01T00:00:01Z')",
        params![thread_id, body],
    )
    .expect("seed thread record");
    conn.execute(
        "INSERT INTO recent_threads (
                thread_id, title, thread_type, last_active_at, recorded_at
             ) VALUES (?1, 'Legacy title', 'chat',
                       '2026-07-01T00:00:00Z', '2026-07-01T00:00:01Z')",
        params![thread_id],
    )
    .expect("seed recent row");
    conn.execute(
        "INSERT INTO thread_meta (
                thread_id, thread_type, thread_label, updated_at, projected_at
             ) VALUES (?1, 'chat', 'Legacy title',
                       '2026-07-01T00:00:00Z', '2026-07-01T00:00:01Z')",
        params![thread_id],
    )
    .expect("seed meta row");
    if has_task_projection {
        conn.execute(
            "INSERT INTO task_projection (
                    thread_id, number, status, title, creator_json, creator_id,
                    updated_by_json, created_at, updated_at, source_updated_at,
                    source_events_len, projected_at
                 ) VALUES (
                    ?1, 41, 'todo', 'Legacy task',
                    '{\"kind\":\"agent\",\"agent_id\":\"test-agent\"}',
                    'test-agent',
                    '{\"kind\":\"agent\",\"agent_id\":\"test-agent\"}',
                    '2026-07-01T00:00:00Z', '2026-07-01T00:00:00Z',
                    '2026-07-01T00:00:00Z', 1, '2026-07-01T00:00:01Z'
                 )",
            params![thread_id],
        )
        .expect("seed task projection");
    }
}

fn raw_legacy_import_generation(service: &GaryxDbService) -> Option<i64> {
    service
        .conn()
        .expect("conn")
        .query_row(
            "SELECT source_row_count FROM projection_states
                  WHERE projection_name = ?1",
            params![LEGACY_IMPORT_GENERATION_NAME],
            |row| row.get(0),
        )
        .optional()
        .expect("generation query")
}

fn seed_pre_generation_cutover_markers(service: &GaryxDbService) {
    let conn = service.conn().expect("conn");
    for (name, version) in [
        (
            RECENT_TASK_THREAD_KIND_MIGRATION_NAME,
            RECENT_TASK_THREAD_KIND_MIGRATION_VERSION,
        ),
        (
            ENDPOINT_HOLDER_DEDUP_MIGRATION_NAME,
            ENDPOINT_HOLDER_DEDUP_MIGRATION_VERSION,
        ),
    ] {
        conn.execute(
            "INSERT INTO projection_states (
                    projection_name, projection_version, source_row_count, projected_at
                 ) VALUES (?1, ?2, 0, '2026-07-15T00:00:00Z')",
            params![name, version],
        )
        .expect("seed pre-generation cutover marker");
    }
}

fn seed_retired_thread_message_routes_table(service: &GaryxDbService) {
    service
        .conn()
        .expect("conn")
        .execute_batch(
            "CREATE TABLE thread_message_routes (message_id TEXT NOT NULL);
                 INSERT INTO thread_message_routes (message_id) VALUES ('legacy-message');",
        )
        .expect("seed retired message routes table");
}

#[test]
fn drop_thread_message_routes_migration_is_atomic_and_one_shot() {
    let service = GaryxDbService::memory().expect("memory db");
    seed_retired_thread_message_routes_table(&service);

    let failed = service.drop_thread_message_routes_v1_inner(|_| {
        Err(GaryxDbError::Configuration(
            "injected post-drop failure".to_owned(),
        ))
    });
    assert!(failed.is_err());
    assert!(
        sqlite_table_exists(
            &service.conn().expect("conn after rollback"),
            "thread_message_routes"
        )
        .expect("table check after rollback"),
        "the table drop must roll back when marker recording cannot commit"
    );
    assert!(
        !service
            .projection_state_exists(
                DROP_THREAD_MESSAGE_ROUTES_MIGRATION_NAME,
                DROP_THREAD_MESSAGE_ROUTES_MIGRATION_VERSION,
            )
            .expect("marker after rollback")
    );

    let first = service
        .drop_thread_message_routes_v1()
        .expect("first migration");
    assert_eq!(first.source_row_count, 1);
    assert_eq!(first.updated_row_count, 1);
    assert!(!first.already_completed);
    assert!(
        !sqlite_table_exists(
            &service.conn().expect("conn after migration"),
            "thread_message_routes"
        )
        .expect("table check after migration")
    );

    seed_retired_thread_message_routes_table(&service);
    let second = service
        .drop_thread_message_routes_v1()
        .expect("completed migration skips");
    assert!(second.already_completed);
    assert_eq!(second.updated_row_count, 0);
    assert!(
        sqlite_table_exists(
            &service.conn().expect("conn after skipped rerun"),
            "thread_message_routes"
        )
        .expect("table check after skipped rerun"),
        "an existing marker must prevent the migration from running again"
    );
}

#[test]
fn drop_thread_message_routes_migration_tolerates_missing_table() {
    let service = GaryxDbService::memory().expect("memory db");
    let summary = service
        .drop_thread_message_routes_v1()
        .expect("missing table is a no-op");
    assert_eq!(summary.source_row_count, 0);
    assert_eq!(summary.updated_row_count, 0);
    assert!(!summary.already_completed);
    assert!(
        service
            .projection_state_exists(
                DROP_THREAD_MESSAGE_ROUTES_MIGRATION_NAME,
                DROP_THREAD_MESSAGE_ROUTES_MIGRATION_VERSION,
            )
            .expect("migration marker")
    );
}

#[test]
fn legacy_import_generation_commit_is_atomic_monotonic_and_recovery_clears_retirement() {
    let service = GaryxDbService::memory().expect("memory db");
    let fresh_incarnation = service.store_incarnation_id().unwrap();
    service.fail_test_db_call(TestDbFaultPoint::LegacyImportCommit, 1);
    assert!(service.commit_legacy_import(0, false).is_err());
    assert_eq!(service.legacy_import_marker_pair().unwrap(), (false, false));
    assert_eq!(raw_legacy_import_generation(&service), None);

    assert_eq!(service.commit_legacy_import(0, false).unwrap(), 1);
    assert_eq!(service.legacy_import_marker_pair().unwrap(), (true, false));
    assert_eq!(raw_legacy_import_generation(&service), Some(1));
    assert_eq!(service.store_incarnation_id().unwrap(), fresh_incarnation);
    service.record_legacy_archive_retirement().unwrap();
    assert_eq!(service.legacy_import_marker_pair().unwrap(), (true, true));
    let generation_one_incarnation = service.store_incarnation_id().unwrap();

    assert!(
        service
            .clear_projection_state(crate::legacy_boot_import::THREAD_RECORDS_IMPORT_NAME)
            .unwrap()
    );
    assert_eq!(service.legacy_import_marker_pair().unwrap(), (false, true));
    service.fail_test_db_call(TestDbFaultPoint::LegacyImportAfterIncarnationRotation, 1);
    assert!(service.commit_legacy_import(3, true).is_err());
    assert_eq!(
        service.store_incarnation_id().unwrap(),
        generation_one_incarnation,
        "a crash after rotation but before commit must roll the identity back"
    );
    assert_eq!(service.legacy_import_marker_pair().unwrap(), (false, true));
    assert_eq!(raw_legacy_import_generation(&service), Some(1));
    assert_eq!(service.commit_legacy_import(3, true).unwrap(), 2);
    assert_eq!(service.legacy_import_marker_pair().unwrap(), (true, false));
    assert_eq!(raw_legacy_import_generation(&service), Some(2));
    assert_ne!(
        service.store_incarnation_id().unwrap(),
        generation_one_incarnation,
        "a committed recovery must rotate exactly with its marker transaction"
    );
    seed_favorite_thread(&service, "thread::recovered-store", false);
    assert!(matches!(
        service
            .set_thread_favorite(
                "thread::recovered-store",
                true,
                0,
                &generation_one_incarnation,
            )
            .expect("old incarnation write is classified"),
        FavoriteThreadResult::WrongIncarnation(ref page)
            if page.revision == 0 && page.favorites.is_empty()
    ));
    assert!(
        service
            .clear_projection_state(LEGACY_IMPORT_GENERATION_NAME)
            .is_err(),
        "the generation owner can never be deleted"
    );
    assert_eq!(raw_legacy_import_generation(&service), Some(2));
}

#[test]
fn pre_generation_cutover_markers_seed_one_without_rerun_then_generation_two_reruns_once() {
    let service = GaryxDbService::memory().expect("memory db");
    service
        .record_projection_state(
            crate::legacy_boot_import::THREAD_RECORDS_IMPORT_NAME,
            crate::legacy_boot_import::THREAD_RECORDS_IMPORT_VERSION,
            1,
        )
        .unwrap();
    seed_pre_generation_cutover_markers(&service);
    seed_task_kind_migration_row(
        &service,
        "thread::pre-generation-task",
        r#"{"thread_id":"thread::pre-generation-task","thread_title_source":"task"}"#,
        false,
    );

    service.run_thread_data_startup_migrations().unwrap();
    assert_eq!(raw_legacy_import_generation(&service), Some(1));
    let before_recovery: Value = serde_json::from_str(
        &service
            .get_thread_record_body("thread::pre-generation-task")
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert!(
        before_recovery.get("thread_kind").is_none(),
        "pre-generation markers are pinned to generation 1 and must not rerun"
    );

    service.record_legacy_archive_retirement().unwrap();
    service
        .clear_projection_state(crate::legacy_boot_import::THREAD_RECORDS_IMPORT_NAME)
        .unwrap();
    assert_eq!(service.commit_legacy_import(1, true).unwrap(), 2);
    service.run_thread_data_startup_migrations().unwrap();
    let after_recovery: Value = serde_json::from_str(
        &service
            .get_thread_record_body("thread::pre-generation-task")
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(after_recovery["thread_kind"], "task");
    assert_eq!(raw_legacy_import_generation(&service), Some(2));
    assert!(
        service
            .migrate_recent_task_thread_kind_v1()
            .unwrap()
            .already_completed
    );
    assert!(
        service
            .migrate_endpoint_holder_dedup_v1()
            .unwrap()
            .already_completed
    );
}

#[test]
fn lazy_generation_seed_failure_aborts_without_marker_movement() {
    let service = GaryxDbService::memory().expect("memory db");
    service
        .record_projection_state(
            crate::legacy_boot_import::THREAD_RECORDS_IMPORT_NAME,
            crate::legacy_boot_import::THREAD_RECORDS_IMPORT_VERSION,
            0,
        )
        .unwrap();
    seed_pre_generation_cutover_markers(&service);
    service.fail_test_db_call(TestDbFaultPoint::LegacyGenerationSeedWrite, 1);

    assert!(service.run_thread_data_startup_migrations().is_err());
    assert_eq!(raw_legacy_import_generation(&service), None);
    assert_eq!(service.legacy_import_marker_pair().unwrap(), (true, false));
    let conn = service.conn().expect("conn");
    let unchanged: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM projection_states
                  WHERE projection_name IN (?1, ?2)
                    AND based_on_import_generation IS NULL",
            params![
                RECENT_TASK_THREAD_KIND_MIGRATION_NAME,
                ENDPOINT_HOLDER_DEDUP_MIGRATION_NAME,
            ],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(unchanged, 2);
}

#[test]
fn recent_task_thread_kind_migration_updates_canonical_and_type_projections() {
    let service = GaryxDbService::memory().expect("memory db");
    seed_task_kind_migration_row(
        &service,
        "thread::legacy-overlay",
        r#"{"thread_id":"thread::legacy-overlay","label":"Overlay title","updated_at":"2026-07-01T00:00:00Z","task":{"number":41}}"#,
        true,
    );
    seed_task_kind_migration_row(
        &service,
        "thread::legacy-title-source",
        r#"{"thread_id":"thread::legacy-title-source","label":"Retained title","thread_title_source":"task","updated_at":"2026-07-01T00:00:00Z"}"#,
        false,
    );
    seed_task_kind_migration_row(
        &service,
        "thread::already-durable",
        r#"{"thread_id":"thread::already-durable","label":"Durable title","thread_kind":"task","updated_at":"2026-07-01T00:00:00Z"}"#,
        false,
    );
    seed_task_kind_migration_row(
        &service,
        "thread::prefix-only",
        r##"{"thread_id":"thread::prefix-only","label":"#TASK-99 ordinary chat","updated_at":"2026-07-01T00:00:00Z"}"##,
        false,
    );

    let summary = service
        .migrate_recent_task_thread_kind_v1()
        .expect("migration succeeds");
    assert_eq!(summary.source_row_count, 3);
    assert_eq!(summary.updated_row_count, 2);
    assert!(!summary.already_completed);

    for thread_id in [
        "thread::legacy-overlay",
        "thread::legacy-title-source",
        "thread::already-durable",
    ] {
        let body = service
            .get_thread_record_body(thread_id)
            .expect("read body")
            .expect("body exists");
        let body: Value = serde_json::from_str(&body).expect("valid body");
        assert_eq!(body["thread_kind"], "task", "{thread_id}");
        assert_eq!(body["updated_at"], "2026-07-01T00:00:00Z");
    }
    let prefix_body: Value = serde_json::from_str(
        &service
            .get_thread_record_body("thread::prefix-only")
            .expect("read prefix body")
            .expect("prefix body exists"),
    )
    .expect("valid prefix body");
    assert!(prefix_body.get("thread_kind").is_none());

    let conn = service.conn().expect("conn");
    for thread_id in [
        "thread::legacy-overlay",
        "thread::legacy-title-source",
        "thread::already-durable",
    ] {
        let recent: (String, String, String) = conn
            .query_row(
                "SELECT thread_type, title, last_active_at
                       FROM recent_threads WHERE thread_id = ?1",
                params![thread_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("recent row");
        assert_eq!(recent.0, "task", "{thread_id}");
        assert_eq!(recent.1, "Legacy title");
        assert_eq!(recent.2, "2026-07-01T00:00:00Z");
        let meta_type: String = conn
            .query_row(
                "SELECT thread_type FROM thread_meta WHERE thread_id = ?1",
                params![thread_id],
                |row| row.get(0),
            )
            .expect("meta row");
        assert_eq!(meta_type, "task", "{thread_id}");
    }
    let prefix_type: String = conn
        .query_row(
            "SELECT thread_type FROM recent_threads WHERE thread_id = 'thread::prefix-only'",
            [],
            |row| row.get(0),
        )
        .expect("prefix recent row");
    assert_eq!(prefix_type, "chat");
    drop(conn);
    assert!(
        service
            .projection_state_matches(
                RECENT_TASK_THREAD_KIND_MIGRATION_NAME,
                RECENT_TASK_THREAD_KIND_MIGRATION_VERSION,
                3,
            )
            .expect("marker")
    );
}

#[test]
fn recent_task_thread_kind_migration_records_zero_and_never_reruns() {
    let service = GaryxDbService::memory().expect("memory db");
    let first = service
        .migrate_recent_task_thread_kind_v1()
        .expect("zero-row migration succeeds");
    assert_eq!(first.source_row_count, 0);
    assert_eq!(first.updated_row_count, 0);
    assert!(!first.already_completed);

    seed_task_kind_migration_row(
        &service,
        "thread::late-legacy-task",
        r#"{"thread_id":"thread::late-legacy-task","thread_title_source":"task"}"#,
        false,
    );
    let second = service
        .migrate_recent_task_thread_kind_v1()
        .expect("completed migration skips");
    assert_eq!(second.source_row_count, 0);
    assert_eq!(second.updated_row_count, 0);
    assert!(second.already_completed);
    let body: Value = serde_json::from_str(
        &service
            .get_thread_record_body("thread::late-legacy-task")
            .expect("read body")
            .expect("body exists"),
    )
    .expect("valid body");
    assert!(body.get("thread_kind").is_none());
}

#[test]
fn recent_task_thread_kind_migration_is_atomic_on_projection_failure() {
    let service = GaryxDbService::memory().expect("memory db");
    seed_task_kind_migration_row(
        &service,
        "thread::atomic-legacy-task",
        r#"{"thread_id":"thread::atomic-legacy-task","thread_title_source":"task"}"#,
        false,
    );
    service
        .conn()
        .expect("conn")
        .execute_batch(
            "CREATE TRIGGER fail_task_kind_projection
                 BEFORE UPDATE OF thread_type ON recent_threads
                 WHEN NEW.thread_type = 'task'
                 BEGIN
                     SELECT RAISE(ABORT, 'forced task-kind projection failure');
                 END;",
        )
        .expect("install failure trigger");

    assert!(
        service.migrate_recent_task_thread_kind_v1().is_err(),
        "projection failure must abort the migration"
    );
    let body: Value = serde_json::from_str(
        &service
            .get_thread_record_body("thread::atomic-legacy-task")
            .expect("read body")
            .expect("body exists"),
    )
    .expect("valid body");
    assert!(body.get("thread_kind").is_none());
    let conn = service.conn().expect("conn");
    let recent_type: String = conn
        .query_row(
            "SELECT thread_type FROM recent_threads
                  WHERE thread_id = 'thread::atomic-legacy-task'",
            [],
            |row| row.get(0),
        )
        .expect("recent type");
    assert_eq!(recent_type, "chat");
    drop(conn);
    assert!(
        !service
            .projection_state_exists(
                RECENT_TASK_THREAD_KIND_MIGRATION_NAME,
                RECENT_TASK_THREAD_KIND_MIGRATION_VERSION,
            )
            .expect("marker lookup")
    );
}

fn seed_endpoint_holder_record(
    service: &GaryxDbService,
    thread_id: &str,
    updated_at: &str,
    bindings: Value,
) {
    let body = json!({
        "thread_id": thread_id,
        "label": format!("Title for {thread_id}"),
        "workspace_dir": "/workspace/test",
        "updated_at": updated_at,
        "channel_bindings": bindings,
    });
    service
        .write_thread_record_with_projections(
            thread_id,
            &serde_json::to_string(&body).expect("record json"),
            Some(updated_at),
            None,
        )
        .expect("seed holder record");
}

fn test_binding(binding_key: &str, label: &str) -> Value {
    json!({
        "channel": "telegram",
        "account_id": "main",
        "binding_key": binding_key,
        "chat_id": binding_key,
        "delivery_target_type": "chat_id",
        "delivery_target_id": binding_key,
        "display_label": label,
        "last_inbound_at": "2026-07-01T00:00:00Z",
    })
}

#[test]
fn endpoint_holder_dedup_migration_keeps_preferred_holder_and_syncs_projection() {
    let service = GaryxDbService::memory().expect("memory db");
    seed_endpoint_holder_record(
        &service,
        "thread::holder-old",
        "2026-07-01T00:00:00Z",
        json!([
            test_binding("1000000001", "Old duplicate"),
            test_binding("1000000002", "Old unique"),
        ]),
    );
    seed_endpoint_holder_record(
        &service,
        "thread::holder-new",
        "2026-07-02T00:00:00Z",
        json!([test_binding("1000000001", "New duplicate")]),
    );
    service
        .conn()
        .expect("conn")
        .execute(
            "INSERT INTO thread_channel_endpoints (
                    endpoint_key, channel, account_id, binding_key, chat_id,
                    thread_id, projected_at
                 ) VALUES (
                    'telegram::main::1000000001', 'telegram', 'main',
                    '1000000001', '1000000001', 'thread::holder-old',
                    '2026-07-01T00:00:00Z'
                 )",
            [],
        )
        .expect("seed stale projection owner");

    let summary = service
        .migrate_endpoint_holder_dedup_v1()
        .expect("dedup migration");
    assert_eq!(summary.source_row_count, 3);
    assert_eq!(summary.updated_row_count, 1);
    assert!(!summary.already_completed);

    let old: Value = serde_json::from_str(
        &service
            .get_thread_record_body("thread::holder-old")
            .expect("old body")
            .expect("old record"),
    )
    .expect("old json");
    let new: Value = serde_json::from_str(
        &service
            .get_thread_record_body("thread::holder-new")
            .expect("new body")
            .expect("new record"),
    )
    .expect("new json");
    assert_eq!(old["updated_at"], "2026-07-01T00:00:00Z");
    assert_eq!(new["updated_at"], "2026-07-02T00:00:00Z");
    let old_bindings = garyx_router::bindings_from_value(&old);
    let new_bindings = garyx_router::bindings_from_value(&new);
    assert_eq!(old_bindings.len(), 1);
    assert_eq!(old_bindings[0].binding_key, "1000000002");
    assert_eq!(new_bindings.len(), 1);
    assert_eq!(new_bindings[0].binding_key, "1000000001");

    let projected = service
        .list_thread_channel_endpoints()
        .expect("endpoint projection");
    let duplicate = projected
        .iter()
        .find(|row| row.endpoint_key == "telegram::main::1000000001")
        .expect("deduplicated endpoint");
    assert_eq!(duplicate.thread_id.as_deref(), Some("thread::holder-new"));
    assert_eq!(duplicate.display_label, "New duplicate");
    let unique = projected
        .iter()
        .find(|row| row.endpoint_key == "telegram::main::1000000002")
        .expect("unique endpoint");
    assert_eq!(unique.thread_id.as_deref(), Some("thread::holder-old"));

    let second = service
        .migrate_endpoint_holder_dedup_v1()
        .expect("idempotent rerun");
    assert!(second.already_completed);
    assert_eq!(second.source_row_count, 3);
    assert_eq!(second.updated_row_count, 0);
}

#[test]
fn endpoint_holder_dedup_migration_records_zero_and_does_not_rerun() {
    let service = GaryxDbService::memory().expect("memory db");
    let first = service
        .migrate_endpoint_holder_dedup_v1()
        .expect("zero migration");
    assert_eq!(first.source_row_count, 0);
    assert!(!first.already_completed);

    seed_endpoint_holder_record(
        &service,
        "thread::late-holder-a",
        "2026-07-01T00:00:00Z",
        json!([test_binding("1000000003", "Late A")]),
    );
    seed_endpoint_holder_record(
        &service,
        "thread::late-holder-b",
        "2026-07-02T00:00:00Z",
        json!([test_binding("1000000003", "Late B")]),
    );
    let second = service
        .migrate_endpoint_holder_dedup_v1()
        .expect("completed migration skips");
    assert!(second.already_completed);
    assert_eq!(second.source_row_count, 0);
    for thread_id in ["thread::late-holder-a", "thread::late-holder-b"] {
        let body: Value = serde_json::from_str(
            &service
                .get_thread_record_body(thread_id)
                .expect("body read")
                .expect("body exists"),
        )
        .expect("body json");
        assert_eq!(garyx_router::bindings_from_value(&body).len(), 1);
    }
}

#[test]
fn endpoint_holder_dedup_migration_is_atomic_on_projection_failure() {
    let service = GaryxDbService::memory().expect("memory db");
    for (thread_id, updated_at) in [
        ("thread::atomic-holder-a", "2026-07-01T00:00:00Z"),
        ("thread::atomic-holder-b", "2026-07-02T00:00:00Z"),
    ] {
        seed_endpoint_holder_record(
            &service,
            thread_id,
            updated_at,
            json!([test_binding("1000000004", "Atomic")]),
        );
    }
    service
        .conn()
        .expect("conn")
        .execute_batch(
            "CREATE TRIGGER fail_endpoint_dedup_projection
                 BEFORE INSERT ON thread_channel_endpoints
                 BEGIN
                     SELECT RAISE(ABORT, 'forced endpoint projection failure');
                 END;",
        )
        .expect("failure trigger");

    assert!(service.migrate_endpoint_holder_dedup_v1().is_err());
    for thread_id in ["thread::atomic-holder-a", "thread::atomic-holder-b"] {
        let body: Value = serde_json::from_str(
            &service
                .get_thread_record_body(thread_id)
                .expect("body read")
                .expect("body exists"),
        )
        .expect("body json");
        assert_eq!(garyx_router::bindings_from_value(&body).len(), 1);
    }
    assert!(
        !service
            .projection_state_exists(
                ENDPOINT_HOLDER_DEDUP_MIGRATION_NAME,
                ENDPOINT_HOLDER_DEDUP_MIGRATION_VERSION,
            )
            .expect("marker lookup")
    );
}

#[test]
fn thread_record_write_read_list_delete_round_trip() {
    let dir = tempfile::tempdir().expect("temp dir");
    let service = GaryxDbService::open(dir.path().join("garyx-db.sqlite3")).expect("db opens");

    service
        .write_thread_record_with_projections(
            "thread::alpha",
            r#"{"thread_id":"thread::alpha"}"#,
            Some("2026-07-08T00:00:00Z"),
            None,
        )
        .expect("write record");
    service
        .write_thread_record_with_projections(
            "meta::known_channel_endpoints",
            r#"{"endpoints":[]}"#,
            None,
            None,
        )
        .expect("write meta record");

    // Reads go through the dedicated reader connection.
    assert_eq!(
        service
            .get_thread_record_body("thread::alpha")
            .expect("get"),
        Some(r#"{"thread_id":"thread::alpha"}"#.to_owned())
    );
    assert!(
        service
            .thread_record_exists("thread::alpha")
            .expect("exists")
    );
    assert!(
        !service
            .thread_record_exists("thread::missing")
            .expect("exists missing")
    );
    assert_eq!(
        service
            .list_thread_record_keys(Some("thread::"))
            .expect("list"),
        vec!["thread::alpha".to_owned()]
    );
    assert_eq!(
        service
            .list_thread_record_keys(None)
            .expect("list all")
            .len(),
        2
    );

    // Overwrite replaces the body.
    service
        .write_thread_record_with_projections(
            "thread::alpha",
            r#"{"thread_id":"thread::alpha","label":"v2"}"#,
            None,
            None,
        )
        .expect("overwrite");
    assert!(
        service
            .get_thread_record_body("thread::alpha")
            .expect("get v2")
            .expect("body")
            .contains("v2")
    );

    assert!(
        service
            .delete_thread_record_with_projections(
                "thread::alpha",
                &garyx_router::DrainedDeleteReservation::test_witness()
            )
            .expect("delete")
    );
    assert!(
        !service
            .delete_thread_record_with_projections(
                "thread::alpha",
                &garyx_router::DrainedDeleteReservation::test_witness()
            )
            .expect("delete again")
    );
    assert_eq!(
        service
            .get_thread_record_body("thread::alpha")
            .expect("get after delete"),
        None
    );
}

#[test]
fn thread_record_key_prefix_listing_is_case_sensitive() {
    // SQLite LIKE is ASCII case-insensitive; the store contract
    // (File/InMemory starts_with) is case-sensitive (#TASK-1896).
    let service = GaryxDbService::memory().expect("memory db");
    for key in ["thread::lower", "Thread::upper"] {
        service
            .write_thread_record_with_projections(key, "{}", None, None)
            .expect("write");
    }
    assert_eq!(
        service
            .list_thread_record_keys(Some("thread::"))
            .expect("list"),
        vec!["thread::lower".to_owned()]
    );
    assert_eq!(
        service
            .list_thread_record_keys(Some("Thread::"))
            .expect("list upper"),
        vec!["Thread::upper".to_owned()]
    );
}

#[test]
fn thread_record_write_derives_projections_in_the_same_transaction() {
    let service = GaryxDbService::memory().expect("memory db");
    let thread_id = "thread::projected";

    service
        .write_thread_record_with_projections(
            thread_id,
            r#"{"thread_id":"thread::projected"}"#,
            None,
            Some(ThreadRecordProjections {
                thread_meta: None,
                task: None,
                recent: Some(sample_recent_draft(thread_id)),
            }),
        )
        .expect("write with recent projection");
    let recent = service
        .list_recent_threads(10, 0)
        .expect("list recent")
        .into_iter()
        .find(|row| row.thread_id == thread_id);
    assert!(recent.is_some(), "recent projection row must exist");

    // A rewrite with `recent: None` removes the projection row in the
    // same transaction as the record update.
    service
        .write_thread_record_with_projections(
            thread_id,
            r#"{"thread_id":"thread::projected","hidden":true}"#,
            None,
            Some(ThreadRecordProjections {
                thread_meta: None,
                task: None,
                recent: None,
            }),
        )
        .expect("write removing recent projection");
    let recent = service
        .list_recent_threads(10, 0)
        .expect("list recent")
        .into_iter()
        .find(|row| row.thread_id == thread_id);
    assert!(recent.is_none(), "recent projection row must be removed");
    assert!(
        service.thread_record_exists(thread_id).expect("exists"),
        "record itself survives projection removal"
    );

    // Deleting the record clears every projection row and the pin
    // with it, in the same transaction.
    service
        .write_thread_record_with_projections(
            thread_id,
            r#"{"thread_id":"thread::projected"}"#,
            None,
            Some(ThreadRecordProjections {
                thread_meta: None,
                task: None,
                recent: Some(sample_recent_draft(thread_id)),
            }),
        )
        .expect("write again");
    service.pin_thread(thread_id).expect("pin");
    service
        .delete_thread_record_with_projections(
            thread_id,
            &garyx_router::DrainedDeleteReservation::test_witness(),
        )
        .expect("delete");
    assert!(
        !service
            .list_recent_threads(10, 0)
            .expect("list recent")
            .iter()
            .any(|row| row.thread_id == thread_id),
        "projection rows must not survive record deletion"
    );
    assert!(
        !service
            .list_pinned_threads()
            .expect("list pins")
            .pins
            .iter()
            .any(|pin| pin.thread_id == thread_id),
        "the pin must be removed in the delete transaction"
    );
}

#[test]
fn thread_record_write_rolls_back_atomically_on_projection_failure() {
    let service = GaryxDbService::memory().expect("memory db");
    let thread_id = "thread::atomic";

    // An invalid projection draft (blank run_state) fails inside the
    // transaction; the record write must roll back with it.
    let mut bad_recent = sample_recent_draft(thread_id);
    bad_recent.run_state = "  ".to_owned();
    let result = service.write_thread_record_with_projections(
        thread_id,
        r#"{"thread_id":"thread::atomic"}"#,
        None,
        Some(ThreadRecordProjections {
            thread_meta: None,
            task: None,
            recent: Some(bad_recent),
        }),
    );
    assert!(result.is_err(), "invalid projection draft must error");
    assert!(
        !service.thread_record_exists(thread_id).expect("exists"),
        "record write must roll back when a projection write fails"
    );
}

#[test]
fn clear_stale_active_runs_settles_by_recent_run_presence() {
    // Review #TASK-1927: an orphan with no committed run must settle to
    // idle (matching the retired reconcile's derivation), while one
    // with history settles to completed.
    let service = GaryxDbService::memory().expect("memory db");
    for (thread_id, recent) in [
        ("thread::orphan-no-history", None),
        ("thread::orphan-with-history", Some("run::done")),
    ] {
        service
            .upsert_recent_thread(RecentThreadDraft {
                thread_id: thread_id.to_owned(),
                title: "Orphan".to_owned(),
                workspace_dir: None,
                thread_type: "chat".to_owned(),
                provider_type: None,
                agent_id: None,
                message_count: 1,
                last_message_preview: String::new(),
                recent_run_id: recent.map(str::to_owned),
                active_run_id: Some("run::stale".to_owned()),
                run_state: "running".to_owned(),
                updated_at: None,
                last_active_at: "2026-07-08T00:00:00Z".to_owned(),
            })
            .expect("seed row");
    }

    let before = service
        .list_recent_threads(10, 0)
        .unwrap()
        .into_iter()
        .map(|row| (row.thread_id, row.activity_seq))
        .collect::<std::collections::BTreeMap<_, _>>();
    let meta_before: i64 = service
        .conn()
        .unwrap()
        .query_row(
            "SELECT activity_seq FROM recent_threads_meta WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();

    service.clear_stale_active_runs().expect("clear orphans");

    let rows = service.list_recent_threads(10, 0).expect("list");
    assert_eq!(
        rows.iter()
            .map(|row| (row.thread_id.clone(), row.activity_seq))
            .collect::<std::collections::BTreeMap<_, _>>(),
        before,
        "pre-bind orphan settlement must not move rows in activity order"
    );
    assert_eq!(
        service
            .conn()
            .unwrap()
            .query_row(
                "SELECT activity_seq FROM recent_threads_meta WHERE id = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        meta_before,
        "pre-bind orphan settlement must not allocate a sequence"
    );
    let state_of = |id: &str| {
        rows.iter()
            .find(|row| row.thread_id == id)
            .map(|row| (row.active_run_id.clone(), row.run_state.clone()))
            .expect("row")
    };
    assert_eq!(
        state_of("thread::orphan-no-history"),
        (None, "idle".to_owned())
    );
    assert_eq!(
        state_of("thread::orphan-with-history"),
        (None, "completed".to_owned())
    );
}

#[test]
fn memory_db_still_works_without_wal() {
    let service = GaryxDbService::memory().expect("memory db");
    service.pin_thread("thread::mem-check").expect("pin");
    let page = service.list_pinned_threads().expect("list");
    assert_eq!(page.pins.len(), 1);
    assert_eq!(page.pins[0].thread_id, "thread::mem-check");
}

#[test]
fn startup_migrations_purge_legacy_workflow_tables_records_and_projections() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garyx-db.sqlite3");
    drop(GaryxDbService::open(&path).expect("create current schema"));

    {
        let conn = Connection::open(&path).expect("legacy db");
        conn.execute_batch(
                r#"
                CREATE TABLE workflow_runs (
                    workflow_id TEXT PRIMARY KEY
                );
                CREATE TABLE workflow_child_runs (thread_id TEXT NOT NULL);
                CREATE TABLE workflow_events (event_seq INTEGER PRIMARY KEY);

                INSERT INTO workflow_runs (workflow_id)
                VALUES ('thread::legacy-workflow-run');
                INSERT INTO workflow_child_runs (thread_id)
                VALUES ('thread::legacy-workflow-child');
                INSERT INTO workflow_events (event_seq) VALUES (1);

                INSERT INTO thread_records (key, body, recorded_at) VALUES
                  ('thread::legacy-workflow-run',
                   '{"thread_kind":"workflow_run","workflow_run_id":"thread::legacy-workflow-run"}',
                   '2026-07-01T00:00:00.000Z'),
                  ('thread::legacy-workflow-task',
                   '{"task":{"executor":{"type":"workflow","workflow_id":"unit"}}}',
                   '2026-07-01T00:00:00.000Z'),
                  ('thread::legacy-workflow-child',
                   '{"source":"workflow","workflow_child_run_id":"child::legacy"}',
                   '2026-07-01T00:00:00.000Z'),
                  ('thread::legacy-workflow-metadata',
                   '{"metadata":{"workflow_thread":true,"workflow_run_id":"thread::legacy-workflow-metadata"}}',
                   '2026-07-01T00:00:00.000Z'),
                  ('thread::ordinary',
                   '{"label":"Discuss the ordinary deployment workflow"}',
                   '2026-07-01T00:00:00.000Z');

                INSERT INTO task_projection (
                    thread_id, number, status, title, creator_json, creator_id,
                    updated_by_json, executor_json, created_at, updated_at,
                    source_updated_at, source_events_len, projected_at
                ) VALUES (
                    'thread::legacy-workflow-task', 71, 'done', 'Legacy task',
                    '{"kind":"agent","agent_id":"legacy"}', 'legacy',
                    '{"kind":"agent","agent_id":"legacy"}',
                    '{"type":"workflow","workflow_id":"unit"}',
                    '2026-07-01T00:00:00.000Z', '2026-07-01T00:00:00.000Z',
                    '2026-07-01T00:00:00.000Z', 0, '2026-07-01T00:00:00.000Z'
                );

                INSERT INTO thread_meta (thread_id, thread_type, projected_at) VALUES
                  ('thread::legacy-workflow-run', 'workflow_run', '2026-07-01T00:00:00.000Z'),
                  ('thread::legacy-workflow-task', 'workflow_run', '2026-07-01T00:00:00.000Z'),
                  ('thread::legacy-workflow-child', 'chat', '2026-07-01T00:00:00.000Z'),
                  ('thread::legacy-workflow-metadata', 'chat', '2026-07-01T00:00:00.000Z'),
                  ('thread::ordinary', 'chat', '2026-07-01T00:00:00.000Z');

                INSERT INTO recent_threads (
                    thread_id, title, thread_type, message_count, last_message_preview,
                    run_state, last_active_at, recorded_at
                ) VALUES (
                    'thread::legacy-workflow-run', 'Legacy run', 'workflow_run', 0, '',
                    'idle', '2026-07-01T00:00:00.000Z', '2026-07-01T00:00:00.000Z'
                );
                INSERT INTO thread_pins (thread_id, pinned_at)
                VALUES ('thread::legacy-workflow-task', '2026-07-01T00:00:00.000Z');
                INSERT INTO thread_favorites (thread_id, favorited_at)
                VALUES ('thread::legacy-workflow-task', '2026-07-01T00:00:00.000Z');
                INSERT INTO archived_threads (thread_id, archived_at)
                VALUES ('thread::legacy-workflow-child', '2026-07-01T00:00:00.000Z');
                INSERT INTO thread_channel_endpoints (
                    endpoint_key, channel, account_id, binding_key, thread_id, projected_at
                ) VALUES (
                    'test::main::legacy', 'test', 'main', 'legacy',
                    'thread::legacy-workflow-child', '2026-07-01T00:00:00.000Z'
                );
                INSERT INTO automation_thread_runs (
                    automation_id, run_id, thread_id, mode, status, started_at, recorded_at
                ) VALUES (
                    'automation::legacy', 'run::legacy', 'thread::legacy-workflow-run',
                    'generated_thread', 'done', '2026-07-01T00:00:00.000Z',
                    '2026-07-01T00:00:00.000Z'
                );
                INSERT INTO capsules (
                    id, title, description, thread_id, html_sha256, byte_size,
                    revision, created_at, updated_at
                ) VALUES (
                    'capsule::legacy', 'Legacy capsule', '',
                    'thread::legacy-workflow-child', 'abc123', 1, 1,
                    '2026-07-01T00:00:00.000Z', '2026-07-01T00:00:00.000Z'
                );
                "#,
            )
            .expect("seed legacy workflow state");
    }

    let db = GaryxDbService::open(&path).expect("open migrated db");
    db.run_thread_data_startup_migrations()
        .expect("run destructive startup migrations");
    assert_eq!(
        db.list_pinned_threads()
            .expect("pins after cleanup")
            .revision,
        1,
        "startup cleanup must bump the collection exactly once"
    );
    assert_eq!(
        db.list_thread_favorites()
            .expect("favorites after cleanup")
            .revision,
        1,
        "startup cleanup must bump favorites exactly once when changed"
    );
    for table in ["workflow_runs", "workflow_child_runs", "workflow_events"] {
        assert!(!sqlite_table_exists(&db.conn().expect("conn"), table).expect("table check"));
    }
    for thread_id in [
        "thread::legacy-workflow-run",
        "thread::legacy-workflow-task",
        "thread::legacy-workflow-child",
        "thread::legacy-workflow-metadata",
    ] {
        assert_eq!(
            db.get_thread_record_body(thread_id).expect("record lookup"),
            None,
            "retired record survived: {thread_id}"
        );
    }
    assert!(
        db.get_thread_record_body("thread::ordinary")
            .expect("ordinary record")
            .is_some(),
        "plain-English workflow text must not delete an ordinary thread"
    );

    let conn = db.conn().expect("conn");
    for (table, column) in [
        ("task_projection", "thread_id"),
        ("thread_meta", "thread_id"),
        ("recent_threads", "thread_id"),
        ("thread_pins", "thread_id"),
        ("thread_favorites", "thread_id"),
        ("archived_threads", "thread_id"),
        ("thread_channel_endpoints", "thread_id"),
        ("automation_thread_runs", "thread_id"),
    ] {
        let sql =
            format!("SELECT COUNT(*) FROM {table} WHERE {column} LIKE 'thread::legacy-workflow%'");
        let count: i64 = conn.query_row(&sql, [], |row| row.get(0)).expect("count");
        assert_eq!(count, 0, "retired projection survived in {table}");
    }
    let capsule_thread_id: Option<String> = conn
        .query_row(
            "SELECT thread_id FROM capsules WHERE id = 'capsule::legacy'",
            [],
            |row| row.get(0),
        )
        .expect("capsule reference");
    assert_eq!(capsule_thread_id, None);
    drop(conn);
    drop(db);

    let reopened = GaryxDbService::open(&path).expect("cleanup is idempotent");
    reopened
        .run_thread_data_startup_migrations()
        .expect("rerun startup migrations");
    assert_eq!(
        reopened
            .list_pinned_threads()
            .expect("pins after idempotent cleanup")
            .revision,
        1,
        "a second startup cleanup must not bump an unchanged collection"
    );
    assert_eq!(
        reopened
            .list_thread_favorites()
            .expect("favorites after idempotent cleanup")
            .revision,
        1,
        "a no-op startup purge must preserve favorites revision"
    );
    assert!(
        reopened
            .get_thread_record_body("thread::ordinary")
            .expect("ordinary record after reopen")
            .is_some()
    );
}

#[tokio::test]
async fn run_blocking_round_trips_reads_and_writes() {
    let service = std::sync::Arc::new(GaryxDbService::memory().expect("memory db"));

    let page = service
        .run_blocking(|db| db.pin_thread("thread::async-entry"))
        .await
        .expect("async pin");
    assert_eq!(page.pins[0].thread_id, "thread::async-entry");

    let page = service
        .run_blocking(|db| db.list_pinned_threads())
        .await
        .expect("async list");
    assert_eq!(page.pins.len(), 1);
    assert_eq!(page.pins[0].thread_id, "thread::async-entry");
}

#[test]
fn opening_legacy_thread_meta_db_adds_projection_columns() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garyx-db.sqlite3");
    {
        let conn = Connection::open(&path).expect("legacy db");
        conn.execute_batch(
            r#"
                CREATE TABLE thread_meta (
                    thread_id TEXT PRIMARY KEY,
                    workspace_dir TEXT,
                    thread_type TEXT NOT NULL DEFAULT 'chat',
                    thread_label TEXT,
                    agent_id TEXT,
                    provider_type TEXT,
                    updated_at TEXT,
                    last_delivery_context_json TEXT,
                    last_delivery_updated_at TEXT,
                    default_list_hidden INTEGER NOT NULL DEFAULT 0,
                    projection_version INTEGER NOT NULL DEFAULT 2,
                    projected_at TEXT NOT NULL
                ) STRICT;

                INSERT INTO thread_meta (
                    thread_id, workspace_dir, thread_type, thread_label, agent_id,
                    provider_type, updated_at, default_list_hidden, projection_version,
                    projected_at
                ) VALUES (
                    'thread::legacy', '/workspace/legacy', 'chat', 'Legacy Thread',
                    'claude', 'claude_code', '2026-06-03T00:00:00.000Z',
                    0, 2, '2026-06-03T00:00:01.000Z'
                );
                "#,
        )
        .expect("legacy thread_meta");
    }

    let db = GaryxDbService::open(&path).expect("open migrated db");

    let rows = db.list_thread_meta().expect("list legacy meta");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].thread_id, "thread::legacy");
    assert_eq!(rows[0].created_at, None);
    assert_eq!(rows[0].message_count, 0);
    assert_eq!(rows[0].last_message_preview, None);
    assert_eq!(rows[0].projection_version, 2);

    let migration = db
        .migrate_thread_meta_schema_v2()
        .expect("canonicalize legacy column order");
    assert_eq!(migration.updated_row_count, 1);
    assert_eq!(
        thread_meta_column_names(&db.conn().unwrap()).unwrap(),
        THREAD_META_SCHEMA_V2_COLUMNS
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        db.list_thread_meta().unwrap()[0].projection_version,
        CURRENT_THREAD_META_PROJECTION_VERSION
    );
}

#[test]
fn opening_composite_endpoint_pk_db_restores_single_holder_upserts() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garyx-db.sqlite3");
    {
        let conn = Connection::open(&path).expect("legacy db");
        conn.execute_batch(
                r#"
                CREATE TABLE thread_records (
                    key TEXT PRIMARY KEY,
                    body TEXT NOT NULL,
                    updated_at TEXT,
                    recorded_at TEXT NOT NULL
                ) STRICT;

                CREATE TABLE projection_states (
                    projection_name TEXT PRIMARY KEY,
                    projection_version INTEGER NOT NULL,
                    source_row_count INTEGER NOT NULL,
                    projected_at TEXT NOT NULL
                ) STRICT;

                CREATE TABLE thread_channel_endpoints (
                    endpoint_key TEXT NOT NULL,
                    channel TEXT NOT NULL,
                    account_id TEXT NOT NULL,
                    binding_key TEXT NOT NULL,
                    chat_id TEXT NOT NULL DEFAULT '',
                    delivery_target_type TEXT NOT NULL DEFAULT 'chat_id',
                    delivery_target_id TEXT NOT NULL DEFAULT '',
                    display_label TEXT NOT NULL DEFAULT '',
                    thread_id TEXT NOT NULL,
                    thread_label TEXT,
                    workspace_dir TEXT,
                    thread_updated_at TEXT,
                    last_inbound_at TEXT,
                    last_delivery_at TEXT,
                    projected_at TEXT NOT NULL,
                    PRIMARY KEY (endpoint_key, thread_id)
                ) STRICT;

                INSERT INTO thread_records (key, body, updated_at, recorded_at)
                VALUES (
                    'thread::legacy-holder',
                    '{"thread_id":"thread::legacy-holder","updated_at":"2026-07-01T00:00:00Z","channel_bindings":[{"channel":"api","account_id":"main","binding_key":"client-1","chat_id":"client-1"}]}',
                    '2026-07-01T00:00:00Z',
                    '2026-07-01T00:00:00Z'
                );

                INSERT INTO projection_states (
                    projection_name, projection_version, source_row_count, projected_at
                ) VALUES (
                    'endpoint_holder_dedup_v1', 1, 1, '2026-07-01T00:00:00Z'
                );

                INSERT INTO thread_channel_endpoints (
                    endpoint_key, channel, account_id, binding_key, chat_id,
                    thread_id, projected_at
                ) VALUES (
                    'api::main::client-1', 'api', 'main', 'client-1', 'client-1',
                    'thread::legacy-holder', '2026-07-01T00:00:00Z'
                );
                "#,
            )
            .expect("legacy composite endpoint schema");
    }

    let db = GaryxDbService::open(&path).expect("open migrated db");
    db.run_thread_data_startup_migrations()
        .expect("run startup migrations");
    let rederived = db
        .list_thread_channel_endpoints()
        .expect("list rederived endpoints");
    assert_eq!(rederived.len(), 1);
    assert_eq!(
        rederived[0].thread_id.as_deref(),
        Some("thread::legacy-holder")
    );

    db.replace_thread_meta_projection(ThreadMetaProjectionDraft {
        thread_id: "thread::current-holder".to_owned(),
        thread_meta: ThreadMetaDraft {
            thread_id: "thread::current-holder".to_owned(),
            thread_type: "chat".to_owned(),
            ..Default::default()
        },
        channel_endpoints: vec![KnownChannelEndpoint {
            endpoint_key: "api::main::client-1".to_owned(),
            channel: "api".to_owned(),
            account_id: "main".to_owned(),
            binding_key: "client-1".to_owned(),
            chat_id: "client-1".to_owned(),
            delivery_target_type: "chat_id".to_owned(),
            delivery_target_id: "client-1".to_owned(),
            display_label: "Test Client".to_owned(),
            thread_id: Some("thread::current-holder".to_owned()),
            ..Default::default()
        }],
    })
    .expect("single-holder endpoint upsert");

    let endpoints = db
        .list_thread_channel_endpoints()
        .expect("list migrated endpoints");
    assert_eq!(endpoints.len(), 1);
    assert_eq!(
        endpoints[0].thread_id.as_deref(),
        Some("thread::current-holder")
    );
}

#[test]
fn fresh_thread_pins_schema_has_sort_order_revision_and_zero_row_marker() {
    let db = GaryxDbService::memory().expect("db opens");
    let column = db
        .conn()
        .expect("connection")
        .query_row(
            "SELECT \"notnull\", dflt_value
                   FROM pragma_table_info('thread_pins')
                  WHERE name = 'sort_order'",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .expect("sort_order column");
    assert_eq!(column, (1, Some("0".to_owned())));
    assert_eq!(db.list_pinned_threads().expect("fresh page").revision, 0);

    let summary = db
        .migrate_thread_pin_sort_order_v1()
        .expect("zero-row migration");
    assert_eq!(summary.source_row_count, 0);
    assert_eq!(summary.updated_row_count, 0);
    assert!(!summary.already_completed);
    assert!(
        db.projection_state_exists(
            THREAD_PIN_SORT_ORDER_MIGRATION_NAME,
            THREAD_PIN_SORT_ORDER_MIGRATION_VERSION,
        )
        .expect("migration marker")
    );
}

#[test]
fn legacy_thread_pin_backfill_preserves_display_order_and_runs_once() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garyx-db.sqlite3");
    {
        let conn = Connection::open(&path).expect("legacy db");
        conn.execute_batch(
            "CREATE TABLE thread_pins (
                     thread_id TEXT PRIMARY KEY,
                     pinned_at TEXT NOT NULL
                 ) STRICT;
                 INSERT INTO thread_pins (thread_id, pinned_at) VALUES
                   ('thread::oldest', '2026-01-01T00:00:01.000Z'),
                   ('thread::same-b', '2026-01-01T00:00:03.000Z'),
                   ('thread::same-a', '2026-01-01T00:00:03.000Z'),
                   ('thread::middle', '2026-01-01T00:00:02.000Z');",
        )
        .expect("legacy pins");
    }

    let db = GaryxDbService::open(&path).expect("open legacy db");
    let summary = db
        .migrate_thread_pin_sort_order_v1()
        .expect("backfill legacy pins");
    assert_eq!(summary.source_row_count, 4);
    assert_eq!(summary.updated_row_count, 4);
    assert!(!summary.already_completed);
    let page = db.list_pinned_threads().expect("backfilled page");
    assert_eq!(page.revision, 0);
    assert_eq!(
        page.pins
            .iter()
            .map(|pin| (pin.thread_id.as_str(), pin.sort_order))
            .collect::<Vec<_>>(),
        vec![
            ("thread::same-a", 0),
            ("thread::same-b", 1),
            ("thread::middle", 2),
            ("thread::oldest", 3),
        ]
    );

    db.conn()
        .expect("connection")
        .execute(
            "UPDATE thread_pins SET sort_order = 99 WHERE thread_id = 'thread::same-a'",
            [],
        )
        .expect("tamper after marker");
    drop(db);

    let reopened = GaryxDbService::open(&path).expect("second boot");
    let second = reopened
        .migrate_thread_pin_sort_order_v1()
        .expect("migration stays one-shot");
    assert!(second.already_completed);
    assert_eq!(second.updated_row_count, 0);
    let retained: i64 = reopened
        .conn()
        .expect("connection")
        .query_row(
            "SELECT sort_order FROM thread_pins WHERE thread_id = 'thread::same-a'",
            [],
            |row| row.get(0),
        )
        .expect("retained sort order");
    assert_eq!(retained, 99, "the marker must prevent a second backfill");
}

#[test]
fn failed_thread_pin_backfill_rolls_back_and_retries_cleanly() {
    let db = GaryxDbService::memory().expect("db opens");
    db.conn()
        .expect("connection")
        .execute_batch(
            "INSERT INTO thread_pins (thread_id, pinned_at) VALUES
                   ('thread::older', '2026-01-01T00:00:01.000Z'),
                   ('thread::newer', '2026-01-01T00:00:02.000Z');",
        )
        .expect("seed pins");

    let result = db.migrate_thread_pin_sort_order_v1_inner(|_| {
        Err(GaryxDbError::Configuration(
            "injected migration failure".to_owned(),
        ))
    });
    assert!(matches!(result, Err(GaryxDbError::Configuration(_))));
    assert!(
        !db.projection_state_exists(
            THREAD_PIN_SORT_ORDER_MIGRATION_NAME,
            THREAD_PIN_SORT_ORDER_MIGRATION_VERSION,
        )
        .expect("marker lookup")
    );
    let rolled_back = db.list_pinned_threads().expect("rolled-back page");
    assert!(rolled_back.pins.iter().all(|pin| pin.sort_order == 0));

    db.migrate_thread_pin_sort_order_v1()
        .expect("retry migration");
    assert_eq!(
        db.list_pinned_threads()
            .expect("retried page")
            .pins
            .iter()
            .map(|pin| (pin.thread_id.as_str(), pin.sort_order))
            .collect::<Vec<_>>(),
        vec![("thread::newer", 0), ("thread::older", 1)]
    );
}

#[test]
fn thread_pins_page_is_one_wal_snapshot_across_pins_and_revision() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garyx-db.sqlite3");
    let reader = GaryxDbService::open(&path).expect("reader opens");
    reader.pin_thread("thread::first").expect("first pin");
    // A raw SQLite connection is intentional: the production invariant
    // forbids a second GaryxDbService for the same data dir, while this
    // test still needs a commit between the page's two snapshot reads.
    let writer = Connection::open(&path).expect("test-only raw writer");

    let snapshot = reader
        .list_pinned_threads_inner(|| {
            writer.execute_batch(
                "BEGIN IMMEDIATE;
                     INSERT INTO thread_pins (thread_id, pinned_at, sort_order)
                     VALUES ('thread::second', '2026-07-16T00:00:00Z', -2);
                     UPDATE thread_pins_meta SET pins_revision = pins_revision + 1 WHERE id = 1;
                     COMMIT;",
            )?;
            Ok(())
        })
        .expect("snapshot page");
    assert_eq!(snapshot.revision, 1);
    assert_eq!(
        snapshot
            .pins
            .iter()
            .map(|pin| pin.thread_id.as_str())
            .collect::<Vec<_>>(),
        vec!["thread::first"]
    );

    let current = reader.list_pinned_threads().expect("current page");
    assert_eq!(current.revision, 2);
    assert_eq!(
        current
            .pins
            .iter()
            .map(|pin| pin.thread_id.as_str())
            .collect::<Vec<_>>(),
        vec!["thread::second", "thread::first"]
    );
}

#[test]
fn pin_unpin_and_idempotent_repin_use_atomic_pages_and_exact_revisions() {
    use std::time::Duration;

    let db = GaryxDbService::memory().expect("db opens");
    let first = db.pin_thread("thread::older").expect("pin older");
    assert_eq!(first.revision, 1);
    let first_pin = first.pins[0].clone();
    std::thread::sleep(Duration::from_millis(2));
    let second = db.pin_thread("thread::newer").expect("pin newer");
    assert_eq!(second.revision, 2);
    assert_eq!(
        second
            .pins
            .iter()
            .map(|pin| pin.thread_id.as_str())
            .collect::<Vec<_>>(),
        vec!["thread::newer", "thread::older"]
    );
    std::thread::sleep(Duration::from_millis(2));
    let repinned = db.pin_thread("thread::older").expect("repin older");
    assert_eq!(repinned.revision, 2);
    let preserved = repinned
        .pins
        .iter()
        .find(|pin| pin.thread_id == "thread::older")
        .expect("repinned record");
    assert_eq!(preserved.pinned_at, first_pin.pinned_at);
    assert_eq!(preserved.sort_order, first_pin.sort_order);

    let (removed, unpinned) = db.unpin_thread("thread::older").expect("unpin older");
    assert!(removed);
    assert_eq!(unpinned.revision, 3);
    assert_eq!(
        unpinned
            .pins
            .iter()
            .map(|pin| pin.thread_id.as_str())
            .collect::<Vec<_>>(),
        vec!["thread::newer"]
    );
    assert!(
        !db.unpin_thread("thread::older")
            .expect("unpin older again")
            .0
    );
    assert_eq!(db.list_pinned_threads().expect("final page").revision, 3);
}

#[test]
fn thread_favorites_schema_initializes_singleton_before_startup_cleanup_and_reopens_stably() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garyx-db.sqlite3");
    let db = GaryxDbService::open(&path).expect("database");
    assert_eq!(db.list_thread_favorites().unwrap().revision, 0);
    let conn = db.conn().expect("writer");
    assert!(sqlite_table_exists(&conn, "thread_favorites").unwrap());
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM thread_favorites_meta WHERE id = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        1
    );
    drop(conn);
    seed_favorite_thread(&db, "thread::reopen-favorite", false);
    let incarnation = db.store_incarnation_id().unwrap();
    db.set_thread_favorite("thread::reopen-favorite", true, 0, &incarnation)
        .expect("favorite");
    drop(db);

    let reopened = GaryxDbService::open(&path).expect("reopen");
    let page = reopened.list_thread_favorites().expect("reopened page");
    assert_eq!(page.revision, 1);
    assert_eq!(page.favorites.len(), 1);
    assert_eq!(page.store_incarnation_id, incarnation);
}

#[test]
fn thread_favorites_cas_fences_identity_revision_and_bumps_every_accepted_noop() {
    let db = GaryxDbService::memory().expect("database");
    seed_favorite_thread(&db, "thread::favorite-cas", false);
    let incarnation = db.store_incarnation_id().unwrap();
    let initial = db.list_thread_favorites().expect("initial page");
    assert_eq!(initial.revision, 0);
    assert!(initial.favorites.is_empty());

    let wrong = db
        .set_thread_favorite("thread::favorite-cas", true, 0, &Uuid::new_v4().to_string())
        .expect("wrong incarnation response");
    assert!(matches!(
        wrong,
        FavoriteThreadResult::WrongIncarnation(ref page) if page.revision == 0
    ));

    let first = db
        .set_thread_favorite("thread::favorite-cas", true, 0, &incarnation)
        .expect("favorite");
    let FavoriteThreadResult::Updated {
        changed: true,
        page: first,
    } = first
    else {
        panic!("expected changed favorite")
    };
    assert_eq!(first.revision, 1);
    assert_eq!(first.favorites.len(), 1);
    let favorited_at = first.favorites[0].favorited_at.clone();

    let repeated = db
        .set_thread_favorite("thread::favorite-cas", true, 1, &incarnation)
        .expect("repeat favorite");
    let FavoriteThreadResult::Updated {
        changed: false,
        page: repeated,
    } = repeated
    else {
        panic!("expected accepted no-op favorite")
    };
    assert_eq!(repeated.revision, 2);
    assert_eq!(repeated.favorites[0].favorited_at, favorited_at);

    let conflict = db
        .set_thread_favorite("thread::favorite-cas", false, 1, &incarnation)
        .expect("stale conflict");
    assert!(matches!(
        conflict,
        FavoriteThreadResult::Conflict(ref page)
            if page.revision == 2 && page.favorites.len() == 1
    ));

    let removed = db
        .set_thread_favorite("thread::favorite-cas", false, 2, &incarnation)
        .expect("unfavorite");
    assert!(matches!(
        removed,
        FavoriteThreadResult::Updated {
            changed: true,
            ref page,
        } if page.revision == 3 && page.favorites.is_empty()
    ));
    let repeated_delete = db
        .set_thread_favorite("thread::favorite-cas", false, 3, &incarnation)
        .expect("repeat unfavorite");
    assert!(matches!(
        repeated_delete,
        FavoriteThreadResult::Updated {
            changed: false,
            ref page,
        } if page.revision == 4 && page.favorites.is_empty()
    ));

    let missing = db
        .set_thread_favorite("thread::missing", true, 4, &incarnation)
        .expect("missing page");
    assert!(matches!(
        missing,
        FavoriteThreadResult::NotFound(ref page)
            if page.revision == 4 && page.favorites.is_empty()
    ));
}

#[test]
fn thread_favorites_get_page_is_one_wal_snapshot() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garyx-db.sqlite3");
    let db = GaryxDbService::open(&path).expect("database");
    seed_favorite_thread(&db, "thread::first-favorite", false);
    seed_favorite_thread(&db, "thread::second-favorite", false);
    let incarnation = db.store_incarnation_id().unwrap();
    db.set_thread_favorite("thread::first-favorite", true, 0, &incarnation)
        .expect("first favorite");
    let writer = Connection::open(&path).expect("test-only raw writer");

    let snapshot = db
        .list_thread_favorites_inner(|| {
            writer.execute_batch(
                "BEGIN IMMEDIATE;
                     INSERT INTO thread_favorites (thread_id, favorited_at)
                     VALUES ('thread::second-favorite', '2026-07-16T00:00:01Z');
                     UPDATE thread_favorites_meta
                        SET favorites_revision = favorites_revision + 1 WHERE id = 1;
                     COMMIT;",
            )?;
            Ok(())
        })
        .expect("snapshot page");
    assert_eq!(snapshot.revision, 1);
    assert_eq!(
        snapshot
            .favorites
            .iter()
            .map(|favorite| favorite.thread_id.as_str())
            .collect::<Vec<_>>(),
        vec!["thread::first-favorite"]
    );
    let current = db.list_thread_favorites().expect("current page");
    assert_eq!(current.revision, 2);
    assert_eq!(current.favorites.len(), 2);
}

#[test]
fn favorites_snapshot_membership_revision_and_recent_rows_share_one_wal_snapshot() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garyx-db.sqlite3");
    let db = GaryxDbService::open(&path).expect("database");
    seed_favorite_thread(&db, "thread::snapshot-first", true);
    seed_favorite_thread(&db, "thread::snapshot-second", true);
    let incarnation = db.store_incarnation_id().unwrap();
    db.set_thread_favorite("thread::snapshot-first", true, 0, &incarnation)
        .expect("first favorite");
    let writer = Connection::open(&path).expect("test-only raw writer");

    let snapshot = db
        .thread_favorites_snapshot_inner(|| {
            writer.execute_batch(
                "BEGIN IMMEDIATE;
                     INSERT INTO thread_favorites (thread_id, favorited_at)
                     VALUES ('thread::snapshot-second', '2026-07-16T00:00:01Z');
                     UPDATE thread_favorites_meta
                        SET favorites_revision = favorites_revision + 1 WHERE id = 1;
                     COMMIT;",
            )?;
            Ok(())
        })
        .expect("atomic snapshot");
    assert_eq!(snapshot.page.revision, 1);
    assert_eq!(snapshot.page.favorites.len(), 1);
    assert_eq!(snapshot.recent_total, 1);
    assert_eq!(snapshot.recent_threads.len(), 1);
    assert_eq!(
        snapshot.recent_threads[0].thread_id,
        "thread::snapshot-first"
    );

    let current = db.thread_favorites_snapshot().expect("next snapshot");
    assert_eq!(current.page.revision, 2);
    assert_eq!(current.page.favorites.len(), 2);
    assert_eq!(current.recent_total, 2);
}

#[test]
fn favorites_enhanced_membership_and_summaries_share_one_wal_snapshot() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garyx-db.sqlite3");
    let db = GaryxDbService::open(&path).expect("database");
    {
        let mut conn = db.conn().expect("writer");
        let tx = conn.transaction().expect("seed transaction");
        seed_summary_favorite_tx(&tx, "thread::enhanced-first", "2026-07-17T00:00:00Z", false);
        seed_summary_favorite_tx(
            &tx,
            "thread::enhanced-second",
            "2026-07-17T00:00:01Z",
            false,
        );
        tx.execute(
            "DELETE FROM thread_favorites WHERE thread_id = 'thread::enhanced-second'",
            [],
        )
        .unwrap();
        tx.commit().expect("seed commit");
    }
    let writer = Connection::open(&path).expect("test-only raw writer");

    let (snapshot, summaries) = db
        .thread_favorites_snapshot_with_options(true, || {
            writer.execute(
                "INSERT INTO thread_favorites (thread_id, favorited_at)
                     VALUES ('thread::enhanced-second', '2026-07-17T00:00:01Z')",
                [],
            )?;
            Ok(())
        })
        .expect("atomic enhanced snapshot");
    let (summaries, truncated) = summaries.expect("summary payload");
    assert_eq!(snapshot.page.favorites.len(), 1);
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].thread_id, "thread::enhanced-first");
    assert!(!truncated);

    let current = db
        .thread_favorites_snapshot_with_summaries()
        .expect("next enhanced snapshot");
    assert_eq!(current.snapshot.page.favorites.len(), 2);
    assert_eq!(current.summaries.len(), 2);
}

#[test]
fn terminal_tombstones_fence_favorite_cleanup_without_noop_revision_bumps() {
    let archived = GaryxDbService::memory().expect("archive database");
    seed_favorite_thread(&archived, "thread::archive-first", false);
    let incarnation = archived.store_incarnation_id().unwrap();
    assert!(
        archived
            .archive_thread_record("thread::archive-first")
            .expect("archive first")
    );
    assert!(matches!(
        archived
            .set_thread_favorite("thread::archive-first", true, 0, &incarnation)
            .expect("post-archive write"),
        FavoriteThreadResult::NotFound(ref page) if page.revision == 0
    ));

    let favorite_first = GaryxDbService::memory().expect("favorite-first database");
    seed_favorite_thread(&favorite_first, "thread::favorite-first", false);
    let incarnation = favorite_first.store_incarnation_id().unwrap();
    favorite_first
        .set_thread_favorite("thread::favorite-first", true, 0, &incarnation)
        .expect("favorite first");
    favorite_first
        .archive_thread_record("thread::favorite-first")
        .expect("archive cleans favorite");
    assert_eq!(
        favorite_first
            .list_thread_favorites()
            .expect("post archive")
            .revision,
        2
    );
    favorite_first
        .archive_thread_record("thread::favorite-first")
        .expect("repeat archive");
    assert_eq!(
        favorite_first
            .list_thread_favorites()
            .expect("repeat archive")
            .revision,
        2,
        "archive tombstone permits bump-on-change"
    );

    let deleted = GaryxDbService::memory().expect("delete database");
    seed_favorite_thread(&deleted, "thread::delete-recreate", false);
    let incarnation = deleted.store_incarnation_id().unwrap();
    assert!(
        deleted
            .delete_thread_record_with_projections(
                "thread::delete-recreate",
                &garyx_router::DrainedDeleteReservation::test_witness()
            )
            .expect("plain delete without favorite")
    );
    assert_eq!(deleted.list_thread_favorites().unwrap().revision, 0);
    assert!(matches!(
        deleted.write_thread_record_with_projections("thread::delete-recreate", "{}", None, None,),
        Err(GaryxDbError::ThreadArchived(_))
    ));
    assert!(matches!(
        deleted
            .set_thread_favorite("thread::delete-recreate", true, 0, &incarnation)
            .expect("post-delete favorite write"),
        FavoriteThreadResult::NotFound(ref page) if page.revision == 0
    ));

    let delete_with_favorite = GaryxDbService::memory().expect("favorite delete database");
    seed_favorite_thread(&delete_with_favorite, "thread::delete-favorite", false);
    let incarnation = delete_with_favorite.store_incarnation_id().unwrap();
    delete_with_favorite
        .set_thread_favorite("thread::delete-favorite", true, 0, &incarnation)
        .expect("favorite before delete");
    assert!(
        delete_with_favorite
            .delete_thread_record_with_projections(
                "thread::delete-favorite",
                &garyx_router::DrainedDeleteReservation::test_witness()
            )
            .expect("delete with favorite")
    );
    let page = delete_with_favorite.list_thread_favorites().unwrap();
    assert_eq!(
        page.revision, 2,
        "delete bumps only for the removed favorite"
    );
    assert!(page.favorites.is_empty());
    assert!(
        !delete_with_favorite
            .delete_thread_record_with_projections(
                "thread::delete-favorite",
                &garyx_router::DrainedDeleteReservation::test_witness()
            )
            .expect("repeat delete")
    );
    assert_eq!(
        delete_with_favorite
            .list_thread_favorites()
            .unwrap()
            .revision,
        2,
        "repeat terminal delete must not bump an unchanged collection"
    );
}

#[test]
fn favorites_snapshot_is_atomic_empty_and_capped_with_truncation() {
    let db = GaryxDbService::memory().expect("database");
    let empty = db.thread_favorites_snapshot().expect("empty snapshot");
    assert!(empty.page.favorites.is_empty());
    assert!(empty.recent_threads.is_empty());
    assert_eq!(empty.recent_total, 0);
    assert!(!empty.recent_truncated);

    let conn = db.conn().expect("writer");
    conn.execute_batch(
        "WITH RECURSIVE seq(x) AS (
                 VALUES(0) UNION ALL SELECT x + 1 FROM seq WHERE x < 500
             )
             INSERT INTO thread_records (key, body, updated_at, recorded_at)
             SELECT printf('thread::snapshot-%03d', x), '{}', NULL,
                    '2026-07-16T00:00:00Z' FROM seq;
             WITH RECURSIVE seq(x) AS (
                 VALUES(0) UNION ALL SELECT x + 1 FROM seq WHERE x < 500
             )
             INSERT INTO thread_favorites (thread_id, favorited_at)
             SELECT printf('thread::snapshot-%03d', x),
                    printf('2026-07-16T00:%02d:%02dZ', x / 60, x % 60) FROM seq;
             WITH RECURSIVE seq(x) AS (
                 VALUES(0) UNION ALL SELECT x + 1 FROM seq WHERE x < 500
             )
             INSERT INTO recent_threads (
                 thread_id, title, thread_type, message_count,
                 last_message_preview, run_state, last_active_at, recorded_at
             )
             SELECT printf('thread::snapshot-%03d', x), printf('Favorite %03d', x),
                    'chat', 1, '', 'idle',
                    printf('2026-07-16T00:%02d:%02dZ', x / 60, x % 60),
                    '2026-07-16T00:00:00Z' FROM seq;",
    )
    .expect("seed 501 joined favorites");
    drop(conn);

    let snapshot = db.thread_favorites_snapshot().expect("capped snapshot");
    assert_eq!(snapshot.page.favorites.len(), 501);
    assert_eq!(snapshot.recent_total, 501);
    assert_eq!(snapshot.recent_threads.len(), 500);
    assert!(snapshot.recent_truncated);
}

#[test]
fn favorites_summary_window_caps_501_all_raw_members() {
    let db = GaryxDbService::memory().expect("database");
    {
        let mut conn = db.conn().expect("writer");
        let tx = conn.transaction().expect("seed transaction");
        for index in 0..=500 {
            let thread_id = format!("thread::raw-{index:03}");
            seed_summary_favorite_tx(&tx, &thread_id, &format!("{index:03}"), false);
        }
        tx.commit().expect("seed commit");
    }

    let enhanced = db
        .thread_favorites_snapshot_with_summaries()
        .expect("enhanced snapshot");
    assert_eq!(enhanced.snapshot.page.favorites.len(), 501);
    assert_eq!(enhanced.snapshot.recent_total, 0);
    assert!(!enhanced.snapshot.recent_truncated);
    assert!(enhanced.summaries_truncated);
    assert_eq!(enhanced.summaries.len(), 500);
    assert_eq!(
        enhanced.summaries.first().unwrap().thread_id,
        "thread::raw-500"
    );
    assert!(
        enhanced
            .summaries
            .iter()
            .all(|row| row.thread_id != "thread::raw-000")
    );
}

#[test]
fn favorites_summary_window_appends_only_one_of_two_raw_members_after_499_recent() {
    let db = GaryxDbService::memory().expect("database");
    {
        let mut conn = db.conn().expect("writer");
        let tx = conn.transaction().expect("seed transaction");
        for index in 0..499 {
            let thread_id = format!("thread::recent-{index:03}");
            seed_summary_favorite_tx(&tx, &thread_id, "recent", false);
            seed_summary_recent_tx(&tx, &thread_id, i64::from(index) + 1);
        }
        seed_summary_favorite_tx(&tx, "thread::raw-newer", "2026-07-17T00:00:01.000Z", false);
        seed_summary_favorite_tx(&tx, "thread::raw-older", "2026-07-17T00:00:00.000Z", false);
        tx.commit().expect("seed commit");
    }

    let enhanced = db
        .thread_favorites_snapshot_with_summaries()
        .expect("enhanced snapshot");
    assert_eq!(enhanced.snapshot.recent_total, 499);
    assert!(enhanced.summaries_truncated);
    assert_eq!(enhanced.summaries.len(), 500);
    assert_eq!(
        enhanced.summaries.last().unwrap().thread_id,
        "thread::raw-newer"
    );
    assert!(
        enhanced
            .summaries
            .iter()
            .all(|row| row.thread_id != "thread::raw-older")
    );
}

#[test]
fn favorites_hidden_member_occupies_a_summary_window_slot() {
    let db = GaryxDbService::memory().expect("database");
    {
        let mut conn = db.conn().expect("writer");
        let tx = conn.transaction().expect("seed transaction");
        for index in 0..=500 {
            let thread_id = format!("thread::hidden-window-{index:03}");
            seed_summary_favorite_tx(&tx, &thread_id, &format!("{index:03}"), index == 500);
        }
        tx.commit().expect("seed commit");
    }

    let enhanced = db
        .thread_favorites_snapshot_with_summaries()
        .expect("enhanced snapshot");
    assert!(enhanced.summaries_truncated);
    assert_eq!(
        enhanced.summaries.len(),
        499,
        "the hidden member consumes one of the 500 window positions"
    );
    assert!(
        enhanced
            .summaries
            .iter()
            .all(|row| row.thread_id != "thread::hidden-window-500")
    );
    assert!(
        enhanced
            .summaries
            .iter()
            .all(|row| row.thread_id != "thread::hidden-window-000"),
        "the first visible member beyond the window must not leak in"
    );
}

#[test]
fn favorites_raw_same_millisecond_tiebreak_selects_ascending_id_at_position_500() {
    let db = GaryxDbService::memory().expect("database");
    {
        let mut conn = db.conn().expect("writer");
        let tx = conn.transaction().expect("seed transaction");
        for index in 0..499 {
            let thread_id = format!("thread::tie-recent-{index:03}");
            seed_summary_favorite_tx(&tx, &thread_id, "recent", false);
            seed_summary_recent_tx(&tx, &thread_id, i64::from(index) + 1);
        }
        // Reverse insertion is deliberate: ordering must come from the
        // raw fallback contract, not rowid/insertion order.
        seed_summary_favorite_tx(&tx, "thread::raw-z", "2026-07-17T00:00:00.123Z", false);
        seed_summary_favorite_tx(&tx, "thread::raw-a", "2026-07-17T00:00:00.123Z", false);
        tx.commit().expect("seed commit");
    }

    let enhanced = db
        .thread_favorites_snapshot_with_summaries()
        .expect("enhanced snapshot");
    assert!(enhanced.summaries_truncated);
    assert_eq!(enhanced.summaries.len(), 500);
    assert_eq!(
        enhanced.summaries.last().unwrap().thread_id,
        "thread::raw-a"
    );
    assert!(
        enhanced
            .summaries
            .iter()
            .all(|row| row.thread_id != "thread::raw-z")
    );
}

#[test]
fn reorder_thread_pins_handles_full_subset_unknown_and_stale_requests() {
    let db = GaryxDbService::memory().expect("db opens");
    db.pin_thread("thread::a").expect("pin a");
    db.pin_thread("thread::b").expect("pin b");
    let initial = db.pin_thread("thread::c").expect("pin c");
    assert_eq!(initial.revision, 3);
    let original_metadata = initial
        .pins
        .iter()
        .map(|pin| (pin.thread_id.clone(), pin.pinned_at.clone()))
        .collect::<BTreeSet<_>>();

    let full = match db
        .reorder_thread_pins(
            vec![
                "thread::a".to_owned(),
                "thread::c".to_owned(),
                "thread::b".to_owned(),
            ],
            3,
        )
        .expect("full reorder")
    {
        ReorderThreadPinsResult::Updated(page) => page,
        ReorderThreadPinsResult::Conflict(_) => panic!("fresh CAS conflicted"),
    };
    assert_eq!(full.revision, 4);
    assert_eq!(
        full.pins
            .iter()
            .map(|pin| (pin.thread_id.as_str(), pin.sort_order))
            .collect::<Vec<_>>(),
        vec![("thread::a", 0), ("thread::c", 1), ("thread::b", 2)]
    );

    let subset = match db
        .reorder_thread_pins(vec!["thread::b".to_owned()], 4)
        .expect("subset reorder")
    {
        ReorderThreadPinsResult::Updated(page) => page,
        ReorderThreadPinsResult::Conflict(_) => panic!("fresh CAS conflicted"),
    };
    assert_eq!(subset.revision, 5);
    assert_eq!(
        subset
            .pins
            .iter()
            .map(|pin| pin.thread_id.as_str())
            .collect::<Vec<_>>(),
        vec!["thread::b", "thread::a", "thread::c"]
    );

    let unknown = match db
        .reorder_thread_pins(
            vec!["thread::unknown".to_owned(), "thread::c".to_owned()],
            5,
        )
        .expect("unknown-id reorder")
    {
        ReorderThreadPinsResult::Updated(page) => page,
        ReorderThreadPinsResult::Conflict(_) => panic!("fresh CAS conflicted"),
    };
    assert_eq!(unknown.revision, 6);
    assert_eq!(
        unknown
            .pins
            .iter()
            .map(|pin| pin.thread_id.as_str())
            .collect::<Vec<_>>(),
        vec!["thread::c", "thread::b", "thread::a"]
    );
    assert_eq!(
        unknown
            .pins
            .iter()
            .map(|pin| (pin.thread_id.clone(), pin.pinned_at.clone()))
            .collect::<BTreeSet<_>>(),
        original_metadata,
        "reorder must preserve membership and pin metadata"
    );

    let conflict = match db
        .reorder_thread_pins(vec!["thread::a".to_owned()], 5)
        .expect("stale reorder")
    {
        ReorderThreadPinsResult::Conflict(page) => page,
        ReorderThreadPinsResult::Updated(_) => panic!("stale CAS unexpectedly succeeded"),
    };
    assert_eq!(conflict, unknown);
    assert_eq!(db.list_pinned_threads().expect("GET page"), unknown);

    assert!(matches!(
        db.reorder_thread_pins(Vec::new(), 6),
        Err(GaryxDbError::BadRequest(_))
    ));
    assert!(matches!(
        db.reorder_thread_pins(vec!["thread::a".to_owned(), " thread::a ".to_owned()], 6,),
        Err(GaryxDbError::BadRequest(_))
    ));
    assert_eq!(
        db.list_pinned_threads().expect("unchanged page").revision,
        6
    );
}

#[test]
fn archive_and_runtime_delete_each_bump_pin_revision_once() {
    let archived = GaryxDbService::memory().expect("archive db");
    archived
        .write_thread_record_with_projections(
            "thread::archived",
            r#"{"thread_id":"thread::archived"}"#,
            None,
            None,
        )
        .expect("archive candidate record");
    archived
        .pin_thread("thread::archived")
        .expect("archive candidate pin");
    archived
        .archive_thread_record("thread::archived")
        .expect("archive");
    assert_eq!(
        archived
            .list_pinned_threads()
            .expect("archive page")
            .revision,
        2
    );
    archived
        .archive_thread_record("thread::archived")
        .expect("repeat archive");
    assert_eq!(
        archived
            .list_pinned_threads()
            .expect("repeat archive page")
            .revision,
        2
    );

    let deleted = GaryxDbService::memory().expect("delete db");
    deleted
        .write_thread_record_with_projections(
            "thread::deleted",
            r#"{"thread_id":"thread::deleted"}"#,
            None,
            None,
        )
        .expect("delete candidate record");
    deleted
        .pin_thread("thread::deleted")
        .expect("delete candidate pin");
    deleted
        .delete_thread_record_with_projections(
            "thread::deleted",
            &garyx_router::DrainedDeleteReservation::test_witness(),
        )
        .expect("runtime delete");
    assert_eq!(
        deleted.list_pinned_threads().expect("delete page").revision,
        2
    );
    deleted
        .delete_thread_record_with_projections(
            "thread::deleted",
            &garyx_router::DrainedDeleteReservation::test_witness(),
        )
        .expect("repeat delete");
    assert_eq!(
        deleted
            .list_pinned_threads()
            .expect("repeat delete page")
            .revision,
        2
    );
}

#[test]
fn empty_thread_id_is_rejected() {
    let db = GaryxDbService::memory().expect("db opens");
    assert!(matches!(
        db.pin_thread("   "),
        Err(GaryxDbError::BadRequest(_))
    ));
}

#[test]
fn workspaces_round_trip_in_app_state_db() {
    let db = GaryxDbService::memory().expect("db opens");
    let first = db
        .upsert_workspace(WorkspaceDraft {
            name: Some(" Repo B ".to_owned()),
            path: " /workspace/repo-b ".to_owned(),
        })
        .expect("upsert first");
    assert_eq!(first.name.as_deref(), Some("Repo B"));
    assert_eq!(first.path, "/workspace/repo-b");

    db.upsert_workspace(WorkspaceDraft {
        name: None,
        path: "/workspace/repo-a".to_owned(),
    })
    .expect("upsert second");
    let updated = db
        .upsert_workspace(WorkspaceDraft {
            name: Some("Repo A".to_owned()),
            path: "/workspace/repo-a".to_owned(),
        })
        .expect("update second");
    assert_eq!(updated.name.as_deref(), Some("Repo A"));

    let workspaces = db.list_workspaces().expect("list workspaces");
    assert_eq!(
        workspaces
            .iter()
            .map(|workspace| workspace.path.as_str())
            .collect::<Vec<_>>(),
        vec!["/workspace/repo-a", "/workspace/repo-b"],
    );

    assert!(db.delete_workspace("/workspace/repo-a").expect("delete"));
    assert!(
        !db.delete_workspace("/workspace/repo-a")
            .expect("delete again")
    );
    assert_eq!(db.count_workspace_rows().expect("count rows"), 2);
    assert_eq!(
        db.list_workspaces()
            .expect("list remaining")
            .into_iter()
            .map(|workspace| workspace.path)
            .collect::<Vec<_>>(),
        vec!["/workspace/repo-b"],
    );
}

#[test]
fn workspace_seed_only_runs_before_any_workspace_row_exists() {
    let db = GaryxDbService::memory().expect("db opens");
    assert!(
        db.seed_workspaces_if_empty(vec![WorkspaceDraft {
            name: None,
            path: "/workspace/from-config".to_owned(),
        }])
        .expect("seed initial")
    );
    assert!(
        !db.seed_workspaces_if_empty(vec![WorkspaceDraft {
            name: None,
            path: "/workspace/ignored".to_owned(),
        }])
        .expect("skip second seed")
    );
    assert_eq!(
        db.list_workspaces()
            .expect("list active")
            .into_iter()
            .map(|workspace| workspace.path)
            .collect::<Vec<_>>(),
        vec!["/workspace/from-config"],
    );

    assert!(
        db.delete_workspace("/workspace/from-config")
            .expect("soft delete")
    );
    assert_eq!(db.count_workspace_rows().expect("count tombstone"), 1);
    assert!(db.list_workspaces().expect("list after delete").is_empty());
    assert!(
        !db.seed_workspaces_if_empty(vec![WorkspaceDraft {
            name: None,
            path: "/workspace/from-config".to_owned(),
        }])
        .expect("tombstone prevents reseed")
    );
    assert!(db.list_workspaces().expect("list remains empty").is_empty());
}

#[test]
fn empty_workspace_path_is_rejected() {
    let db = GaryxDbService::memory().expect("db opens");
    assert!(matches!(
        db.upsert_workspace(WorkspaceDraft {
            name: None,
            path: "   ".to_owned(),
        }),
        Err(GaryxDbError::BadRequest(_))
    ));
}

#[test]
fn relative_workspace_path_is_rejected() {
    let db = GaryxDbService::memory().expect("db opens");
    assert!(matches!(
        db.upsert_workspace(WorkspaceDraft {
            name: None,
            path: "relative/project".to_owned(),
        }),
        Err(GaryxDbError::BadRequest(_))
    ));
}

fn capsule_draft(id: &str, title: &str, thread_id: &str) -> CapsuleCreateDraft {
    CapsuleCreateDraft {
        id: id.to_owned(),
        title: title.to_owned(),
        description: format!("{} description", title.trim()),
        thread_id: Some(thread_id.to_owned()),
        run_id: Some(format!("run::{title}")),
        agent_id: Some("agent::capsule".to_owned()),
        provider_type: Some("codex_app_server".to_owned()),
        html_sha256: "a".repeat(64),
        byte_size: 42,
    }
}

fn capsule_table_columns(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(capsules)")
        .expect("inspect capsules schema");
    stmt.query_map([], |row| row.get::<_, String>(1))
        .expect("query capsules columns")
        .collect::<Result<Vec<_>, _>>()
        .expect("read capsules columns")
}

#[test]
fn capsules_schema_has_favorite_column_and_reinitialization_is_idempotent() {
    let db = GaryxDbService::memory().expect("db opens");
    let conn = db.conn().expect("db connection");
    assert!(capsule_table_columns(&conn).contains(&"favorited_at".to_owned()));

    initialize_connection(&conn).expect("schema reinitializes");
    let columns = capsule_table_columns(&conn);
    assert_eq!(
        columns
            .iter()
            .filter(|column| column.as_str() == "favorited_at")
            .count(),
        1
    );
}

#[test]
fn capsules_schema_migrates_existing_table_with_favorite_column() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garyx-db.sqlite3");
    let legacy = Connection::open(&path).expect("open legacy db");
    legacy
        .execute_batch(
            r#"
                CREATE TABLE capsules (
                    id            TEXT PRIMARY KEY,
                    title         TEXT NOT NULL DEFAULT '',
                    description   TEXT NOT NULL DEFAULT '',
                    thread_id     TEXT,
                    run_id        TEXT,
                    agent_id      TEXT,
                    provider_type TEXT,
                    html_sha256   TEXT NOT NULL,
                    byte_size     INTEGER NOT NULL DEFAULT 0,
                    revision      INTEGER NOT NULL DEFAULT 1,
                    created_at    TEXT NOT NULL,
                    updated_at    TEXT NOT NULL
                ) STRICT;
                "#,
        )
        .expect("create legacy capsules table");
    drop(legacy);

    let db = GaryxDbService::open(&path).expect("open migrated db");
    let conn = db.conn().expect("db connection");
    assert!(capsule_table_columns(&conn).contains(&"favorited_at".to_owned()));
    initialize_connection(&conn).expect("migrated schema reinitializes");
}

#[test]
fn capsules_crud_create_update_get_list_delete() {
    let db = GaryxDbService::memory().expect("db opens");
    let id = Uuid::new_v4().to_string();
    let created = db
        .create_capsule(capsule_draft(&id, " Demo ", "thread::capsules"))
        .expect("create capsule");
    assert_eq!(created.id, id);
    assert_eq!(created.title, "Demo");
    assert_eq!(created.description, "Demo description");
    assert_eq!(created.thread_id.as_deref(), Some("thread::capsules"));
    assert_eq!(created.run_id.as_deref(), Some("run:: Demo"));
    assert_eq!(created.agent_id.as_deref(), Some("agent::capsule"));
    assert_eq!(created.provider_type.as_deref(), Some("codex_app_server"));
    assert_eq!(created.byte_size, 42);
    assert_eq!(created.revision, 1);
    assert_eq!(created.created_at, created.updated_at);
    assert_eq!(created.favorited_at, None);

    let fetched = db
        .get_capsule(&id)
        .expect("get capsule")
        .expect("capsule exists");
    assert_eq!(fetched, created);

    let updated = db
        .update_capsule(
            &id,
            CapsuleUpdateDraft {
                title: Some("Updated".to_owned()),
                description: Some("New description".to_owned()),
                html_sha256: Some("b".repeat(64)),
                byte_size: Some(84),
            },
        )
        .expect("update capsule")
        .expect("updated capsule");
    assert_eq!(updated.title, "Updated");
    assert_eq!(updated.description, "New description");
    assert_eq!(updated.html_sha256, "b".repeat(64));
    assert_eq!(updated.byte_size, 84);
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.created_at, created.created_at);
    assert_eq!(updated.thread_id, created.thread_id);
    assert_eq!(updated.agent_id, created.agent_id);
    assert_eq!(
        db.list_capsules().expect("list capsules"),
        vec![updated.clone()]
    );

    assert!(db.delete_capsule(&id).expect("delete capsule"));
    assert!(!db.delete_capsule(&id).expect("delete missing capsule"));
    assert!(db.get_capsule(&id).expect("get after delete").is_none());
}

#[test]
fn set_capsule_favorite_is_idempotent_metadata_only_point_write() {
    let db = GaryxDbService::memory().expect("db opens");
    let id = Uuid::new_v4().to_string();
    let created = db
        .create_capsule(capsule_draft(&id, "Favorite", "thread::capsules"))
        .expect("create capsule");

    let favorited = db
        .set_capsule_favorite(&id, true)
        .expect("favorite capsule")
        .expect("capsule exists");
    let first_favorited_at = favorited
        .favorited_at
        .clone()
        .expect("favorite timestamp is set");
    assert_eq!(favorited.revision, created.revision);
    assert_eq!(favorited.updated_at, created.updated_at);

    let repeated = db
        .set_capsule_favorite(&id, true)
        .expect("repeat favorite")
        .expect("capsule exists");
    assert_eq!(
        repeated.favorited_at.as_deref(),
        Some(first_favorited_at.as_str())
    );
    assert_eq!(repeated.revision, created.revision);
    assert_eq!(repeated.updated_at, created.updated_at);

    let unfavorited = db
        .set_capsule_favorite(&id, false)
        .expect("unfavorite capsule")
        .expect("capsule exists");
    assert_eq!(unfavorited.favorited_at, None);
    assert_eq!(unfavorited.revision, created.revision);
    assert_eq!(unfavorited.updated_at, created.updated_at);

    let unknown_id = Uuid::new_v4().to_string();
    assert_eq!(
        db.set_capsule_favorite(&unknown_id, true)
            .expect("favorite unknown capsule"),
        None
    );
}

#[test]
fn capsules_list_orders_updated_desc_and_filters_thread() {
    let db = GaryxDbService::memory().expect("db opens");
    let first_id = Uuid::new_v4().to_string();
    let second_id = Uuid::new_v4().to_string();
    let other_id = Uuid::new_v4().to_string();
    db.create_capsule(capsule_draft(&first_id, "First", "thread::one"))
        .expect("create first");
    db.create_capsule(capsule_draft(&second_id, "Second", "thread::one"))
        .expect("create second");
    db.create_capsule(capsule_draft(&other_id, "Other", "thread::two"))
        .expect("create other");
    std::thread::sleep(std::time::Duration::from_millis(2));
    db.update_capsule(
        &first_id,
        CapsuleUpdateDraft {
            title: Some("First updated".to_owned()),
            ..Default::default()
        },
    )
    .expect("update first");

    let all = db.list_capsules().expect("list all");
    assert_eq!(all[0].id, first_id);
    let thread_one = db
        .list_capsules_for_thread("thread::one")
        .expect("list thread one");
    assert_eq!(thread_one.len(), 2);
    assert_eq!(thread_one[0].id, first_id);
    assert!(thread_one.iter().any(|record| record.id == first_id));
    assert!(thread_one.iter().any(|record| record.id == second_id));
    assert!(
        thread_one
            .iter()
            .all(|record| record.thread_id.as_deref() == Some("thread::one"))
    );
}

#[test]
fn capsules_reject_invalid_uuid_hash_and_size() {
    let db = GaryxDbService::memory().expect("db opens");
    assert!(matches!(
        db.create_capsule(capsule_draft("not-a-uuid", "Bad", "thread::bad")),
        Err(GaryxDbError::BadRequest(_))
    ));
    let id = Uuid::new_v4().to_string();
    let mut bad_hash = capsule_draft(&id, "Bad Hash", "thread::bad");
    bad_hash.html_sha256 = "not-hex".to_owned();
    assert!(matches!(
        db.create_capsule(bad_hash),
        Err(GaryxDbError::BadRequest(_))
    ));
    let mut bad_size = capsule_draft(&id, "Bad Size", "thread::bad");
    bad_size.byte_size = -1;
    assert!(matches!(
        db.create_capsule(bad_size),
        Err(GaryxDbError::BadRequest(_))
    ));
    assert!(matches!(
        db.get_capsule("../escape"),
        Err(GaryxDbError::BadRequest(_))
    ));
}

#[test]
fn recent_threads_upsert_list_and_remove() {
    let db = GaryxDbService::memory().expect("db opens");
    db.upsert_recent_thread(RecentThreadDraft {
        thread_id: "thread::older".to_owned(),
        title: "Older Thread".to_owned(),
        workspace_dir: Some("/work/test-older".to_owned()),
        thread_type: "chat".to_owned(),
        provider_type: Some("claude".to_owned()),
        agent_id: Some("agent::test".to_owned()),
        message_count: 3,
        last_message_preview: "older preview".to_owned(),
        recent_run_id: Some("run::older".to_owned()),
        active_run_id: None,
        run_state: "completed".to_owned(),
        updated_at: Some("2026-05-23T10:00:00.000Z".to_owned()),
        last_active_at: "2026-05-23T10:00:00.000Z".to_owned(),
    })
    .expect("upsert older");
    db.upsert_recent_thread(RecentThreadDraft {
        thread_id: "thread::newer".to_owned(),
        title: "Newer Thread".to_owned(),
        workspace_dir: None,
        thread_type: "chat".to_owned(),
        provider_type: None,
        agent_id: None,
        message_count: 1,
        last_message_preview: "newer preview".to_owned(),
        recent_run_id: None,
        active_run_id: Some("run::active".to_owned()),
        run_state: "running".to_owned(),
        updated_at: Some("2026-05-23T11:00:00.000Z".to_owned()),
        last_active_at: "2026-05-23T11:00:00.000Z".to_owned(),
    })
    .expect("upsert newer");
    db.upsert_recent_thread(RecentThreadDraft {
        thread_id: "thread::older".to_owned(),
        title: "Older Thread Renamed".to_owned(),
        workspace_dir: Some("/work/test-older-renamed".to_owned()),
        thread_type: "task".to_owned(),
        provider_type: Some("codex".to_owned()),
        agent_id: None,
        message_count: 4,
        last_message_preview: "updated preview".to_owned(),
        recent_run_id: Some("run::older-two".to_owned()),
        active_run_id: None,
        run_state: "completed".to_owned(),
        updated_at: Some("2026-05-23T12:00:00.000Z".to_owned()),
        last_active_at: "2026-05-23T12:00:00.000Z".to_owned(),
    })
    .expect("update older");

    let records = db.list_recent_threads(10, 0).expect("list recent threads");
    assert_eq!(
        records
            .iter()
            .map(|record| record.thread_id.as_str())
            .collect::<Vec<_>>(),
        vec!["thread::older", "thread::newer"],
    );
    assert_eq!(records[0].title, "Older Thread Renamed");
    assert_eq!(
        records[0].workspace_dir.as_deref(),
        Some("/work/test-older-renamed")
    );
    assert_eq!(records[0].thread_type, "task");
    assert_eq!(records[0].provider_type.as_deref(), Some("codex"));
    assert_eq!(records[0].message_count, 4);
    assert_eq!(records[0].last_message_preview, "updated preview");
    assert_eq!(records[0].recent_run_id.as_deref(), Some("run::older-two"));
    assert_eq!(records[0].run_state, "completed");

    let limited = db
        .list_recent_threads(1, 0)
        .expect("list limited recent threads");
    assert_eq!(limited.len(), 1);
    assert_eq!(limited[0].thread_id, "thread::older");
    let offset = db
        .list_recent_threads(1, 1)
        .expect("list offset recent threads");
    assert_eq!(offset.len(), 1);
    assert_eq!(offset[0].thread_id, "thread::newer");
    assert_eq!(db.count_recent_threads().expect("count recent threads"), 2);

    assert!(
        db.remove_recent_thread("thread::older")
            .expect("remove older")
    );
    assert!(
        !db.remove_recent_thread("thread::older")
            .expect("remove older again")
    );
    assert_eq!(
        db.list_recent_threads(10, 0)
            .expect("list remaining recent threads")
            .into_iter()
            .map(|record| record.thread_id)
            .collect::<Vec<_>>(),
        vec!["thread::newer"],
    );
}

#[test]
fn recent_threads_filtered_page_filters_before_pagination() {
    let db = GaryxDbService::memory().expect("db opens");
    for (thread_id, thread_type, timestamp) in [
        ("thread::task-middle", "task", "2026-05-23T12:00:00Z"),
        ("thread::chat-older", "chat", "2026-05-23T13:00:00Z"),
        ("thread::chat-newer", "chat", "2026-05-23T13:00:00Z"),
        ("thread::task-newest", "task", "2026-05-23T14:00:00Z"),
    ] {
        db.upsert_recent_thread(RecentThreadDraft {
            thread_id: thread_id.to_owned(),
            title: thread_id.to_owned(),
            workspace_dir: None,
            thread_type: thread_type.to_owned(),
            provider_type: None,
            agent_id: None,
            message_count: 0,
            last_message_preview: String::new(),
            recent_run_id: None,
            active_run_id: None,
            run_state: "idle".to_owned(),
            updated_at: Some(timestamp.to_owned()),
            last_active_at: timestamp.to_owned(),
        })
        .expect("seed recent row");
    }

    let excluded = db
        .list_recent_threads_page(RecentThreadTaskFilter::Exclude, 2, 0)
        .expect("exclude page");
    assert_eq!(excluded.total, 2);
    assert_eq!(excluded.offset, 0);
    assert!(!excluded.has_more);
    assert_eq!(
        excluded
            .records
            .iter()
            .map(|row| row.thread_id.as_str())
            .collect::<Vec<_>>(),
        vec!["thread::chat-newer", "thread::chat-older"],
        "task rows ahead of chats must not shorten the filtered page"
    );

    let only_first = db
        .list_recent_threads_page(RecentThreadTaskFilter::Only, 1, 0)
        .expect("only first page");
    assert_eq!(only_first.total, 2);
    assert!(only_first.has_more);
    assert_eq!(only_first.records[0].thread_id, "thread::task-newest");
    let only_second = db
        .list_recent_threads_page(RecentThreadTaskFilter::Only, 1, 1)
        .expect("only second page");
    assert_eq!(only_second.offset, 1);
    assert!(!only_second.has_more);
    assert_eq!(only_second.records[0].thread_id, "thread::task-middle");

    let included = db
        .list_recent_threads_page(RecentThreadTaskFilter::Include, 10, 0)
        .expect("include page");
    assert_eq!(included.total, 4);
    assert_eq!(
        included
            .records
            .iter()
            .map(|row| row.thread_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "thread::task-newest",
            "thread::chat-newer",
            "thread::chat-older",
            "thread::task-middle",
        ]
    );

    let clamped = db
        .list_recent_threads_page(RecentThreadTaskFilter::Exclude, 10, 99)
        .expect("clamped page");
    assert_eq!(clamped.total, 2);
    assert_eq!(clamped.offset, 2);
    assert!(clamped.records.is_empty());
    assert!(!clamped.has_more);
}

#[test]
fn recent_threads_keyset_does_not_skip_after_deletion_and_uses_n_plus_one() {
    let db = GaryxDbService::memory().expect("db opens");
    for thread_id in ["thread::oldest", "thread::middle", "thread::newest"] {
        db.upsert_recent_thread(sample_recent_draft(thread_id))
            .expect("seed recent row");
    }

    let first = db
        .list_recent_threads_keyset_page(RecentThreadTaskFilter::Include, 1, None)
        .expect("first keyset page");
    assert_eq!(first.total, 3);
    assert!(first.has_more, "N+1 must detect a second row");
    assert_eq!(first.records.len(), 1);
    assert_eq!(first.records[0].thread_id, "thread::newest");
    let cursor = first.records[0].activity_seq;

    db.remove_recent_thread("thread::newest")
        .expect("delete already-returned row");
    let second = db
        .list_recent_threads_keyset_page(RecentThreadTaskFilter::Include, 1, Some(cursor))
        .expect("second keyset page");
    assert_eq!(second.total, 2);
    assert!(second.has_more);
    assert_eq!(
        second.records[0].thread_id, "thread::middle",
        "deleting a row above the cursor must not skip the next row"
    );

    let last = db
        .list_recent_threads_keyset_page(
            RecentThreadTaskFilter::Include,
            1,
            Some(second.records[0].activity_seq),
        )
        .expect("last keyset page");
    assert_eq!(last.records[0].thread_id, "thread::oldest");
    assert!(!last.has_more, "exactly N remaining rows has no next page");

    let empty = db
        .list_recent_threads_keyset_page(
            RecentThreadTaskFilter::Include,
            1,
            Some(last.records[0].activity_seq),
        )
        .expect("empty tail page");
    assert!(empty.records.is_empty());
    assert!(!empty.has_more);
}

#[test]
fn recent_threads_keyset_count_and_rows_share_one_wal_snapshot() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garyx-db.sqlite3");
    let db = GaryxDbService::open(&path).expect("db opens");
    db.upsert_recent_thread(sample_recent_draft("thread::snapshot-before"))
        .expect("seed initial row");

    let page = db
        .list_recent_threads_keyset_page_inner(RecentThreadTaskFilter::Include, 10, None, || {
            let writer = Connection::open(&path)?;
            writer.execute_batch(
                "BEGIN IMMEDIATE;
                         UPDATE recent_threads_meta SET activity_seq = 2 WHERE id = 1;
                         INSERT INTO recent_threads (
                             thread_id, title, thread_type, last_active_at,
                             activity_seq, recorded_at
                         ) VALUES (
                             'thread::snapshot-after', 'After', 'chat',
                             '2026-07-16T01:00:00Z', 2,
                             '2026-07-16T01:00:00Z'
                         );
                         COMMIT;",
            )?;
            Ok(())
        })
        .expect("snapshot keyset page");

    assert_eq!(page.total, 1);
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].thread_id, "thread::snapshot-before");
    assert_eq!(db.count_recent_threads().unwrap(), 2);
}

#[test]
fn recent_threads_filtered_page_uses_one_read_snapshot() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garyx-db.sqlite3");
    let db = GaryxDbService::open(&path).expect("db opens");
    db.upsert_recent_thread(RecentThreadDraft {
        thread_id: "thread::snapshot-before".to_owned(),
        title: "Before".to_owned(),
        workspace_dir: None,
        thread_type: "chat".to_owned(),
        provider_type: None,
        agent_id: None,
        message_count: 0,
        last_message_preview: String::new(),
        recent_run_id: None,
        active_run_id: None,
        run_state: "idle".to_owned(),
        updated_at: Some("2026-05-23T10:00:00Z".to_owned()),
        last_active_at: "2026-05-23T10:00:00Z".to_owned(),
    })
    .expect("seed initial row");

    let page = db
        .list_recent_threads_page_inner(RecentThreadTaskFilter::Include, 10, 0, || {
            let writer = Connection::open(&path)?;
            writer.execute(
                "INSERT INTO recent_threads (
                            thread_id, title, thread_type, last_active_at, recorded_at
                         ) VALUES (
                            'thread::snapshot-after', 'After', 'chat',
                            '2026-05-23T11:00:00Z', '2026-05-23T11:00:00Z'
                         )",
                [],
            )?;
            Ok(())
        })
        .expect("snapshot page");

    assert_eq!(page.total, 1);
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].thread_id, "thread::snapshot-before");
    assert_eq!(
        db.count_recent_threads().expect("post-write count"),
        2,
        "the concurrent commit must exist after the read transaction closes"
    );
}

#[test]
fn recent_threads_filtered_queries_use_partial_order_indexes() {
    let db = GaryxDbService::memory().expect("db opens");
    db.migrate_recent_thread_activity_seq_v1()
        .expect("create activity indexes");
    let conn = db.conn().expect("conn");
    for (predicate, expected_index) in [
        (
            "thread_type = 'task'",
            "idx_recent_threads_task_activity_seq",
        ),
        (
            "thread_type <> 'task'",
            "idx_recent_threads_non_task_activity_seq",
        ),
    ] {
        let sql = format!(
            "EXPLAIN QUERY PLAN
                 SELECT thread_id FROM recent_threads
                  WHERE {predicate}
                  ORDER BY activity_seq DESC
                  LIMIT 10"
        );
        let mut stmt = conn.prepare(&sql).expect("prepare query plan");
        let details = stmt
            .query_map([], |row| row.get::<_, String>(3))
            .expect("query plan")
            .collect::<Result<Vec<_>, _>>()
            .expect("plan rows")
            .join("\n");
        assert!(
            details.contains(expected_index),
            "expected {expected_index} in query plan:\n{details}"
        );
    }
}

#[test]
fn thread_summary_keyset_branches_use_scoped_partial_indexes_without_temp_sort() {
    let db = GaryxDbService::memory().expect("db opens");
    let conn = db.conn().expect("connection");
    for (filter, suffix) in [
        (ThreadSummaryTaskFilter::Include, "visible"),
        (ThreadSummaryTaskFilter::Exclude, "non_task"),
        (ThreadSummaryTaskFilter::Only, "task"),
    ] {
        for scoped in [false, true] {
            for has_cursor in [false, true] {
                let expected_index = if scoped {
                    format!("idx_thread_meta_summary_workspace_{suffix}")
                } else {
                    format!("idx_thread_meta_summary_{suffix}")
                };
                let mut bind = Vec::new();
                if scoped {
                    bind.push(SqlValue::Text("/workspace/test".to_owned()));
                }
                if has_cursor {
                    bind.push(SqlValue::Integer(1));
                    bind.push(SqlValue::Text("thread::cursor".to_owned()));
                }
                bind.push(SqlValue::Integer(31));
                let sql = format!(
                    "EXPLAIN QUERY PLAN {}",
                    filter.page_sql(scoped, false, has_cursor)
                );
                let mut stmt = conn.prepare(&sql).expect("prepare query plan");
                let details = stmt
                    .query_map(params_from_iter(bind.iter()), |row| row.get::<_, String>(3))
                    .expect("query plan")
                    .collect::<Result<Vec<_>, _>>()
                    .expect("plan rows")
                    .join("\n");
                assert!(
                    details.contains("USING INDEX") && details.contains(&expected_index),
                    "expected {expected_index} for filter={filter:?} scoped={scoped} cursor={has_cursor}:\n{details}"
                );
                assert!(
                    !details.contains("USE TEMP B-TREE"),
                    "keyset branch must be index-ordered:\n{details}"
                );
            }
        }
    }
}

#[test]
fn thread_meta_summary_cutover_backfills_all_columns_once_and_is_idempotent() {
    let db = GaryxDbService::memory().expect("db opens");
    let records = [
        (
            "thread::summary-cutover-updated",
            json!({
                "thread_id": "thread::summary-cutover-updated",
                "label": "Straße",
                "workspace_dir": "/workspace/Équipe",
                "agent_id": "Σς",
                "updated_at": "2026-07-17T01:02:03.500+00:00",
                "created_at": "2020-01-01T00:00:00Z",
                "last_assistant_preview": "％＿＼"
            }),
        ),
        (
            "thread::summary-cutover-created",
            json!({
                "thread_id": "thread::summary-cutover-created",
                "label": "Created only",
                "created_at": "2026-07-17T01:02:03Z"
            }),
        ),
        (
            "thread::summary-cutover-null",
            json!({"thread_id": "thread::summary-cutover-null"}),
        ),
    ];
    {
        let conn = db.conn().expect("writer");
        for (thread_id, body) in &records {
            conn.execute(
                "INSERT INTO thread_records (key, body, updated_at, recorded_at)
                     VALUES (?1, ?2, NULL, '2026-07-17T00:00:00Z')",
                params![thread_id, body.to_string()],
            )
            .expect("seed canonical record");
            conn.execute(
                "INSERT INTO thread_meta (
                        thread_id, thread_label, sort_updated_at_us, search_text,
                        projected_at
                     ) VALUES (?1, 'stale', -1, 'stale', '2026-07-17T00:00:00Z')",
                params![thread_id],
            )
            .expect("seed stale projection");
        }
    }

    let first = db
        .migrate_thread_meta_summary_v1()
        .expect("summary cutover");
    assert_eq!(first.source_row_count, 3);
    assert_eq!(first.updated_row_count, 3);
    assert!(!first.already_completed);
    let rows = db.list_thread_meta().expect("backfilled rows");
    let updated = rows
        .iter()
        .find(|row| row.thread_id == "thread::summary-cutover-updated")
        .unwrap();
    assert_eq!(
        updated.sort_updated_at_us,
        DateTime::parse_from_rfc3339("2026-07-17T01:02:03.500Z")
            .unwrap()
            .timestamp_micros()
    );
    assert_eq!(
        updated.search_text,
        crate::thread_meta_projection::normalize_for_search(
            "Straße\n/workspace/Équipe\nΣς\n％＿＼",
        )
    );
    let created = rows
        .iter()
        .find(|row| row.thread_id == "thread::summary-cutover-created")
        .unwrap();
    assert_eq!(
        created.sort_updated_at_us,
        DateTime::parse_from_rfc3339("2026-07-17T01:02:03Z")
            .unwrap()
            .timestamp_micros()
    );
    let missing = rows
        .iter()
        .find(|row| row.thread_id == "thread::summary-cutover-null")
        .unwrap();
    assert_eq!(missing.sort_updated_at_us, 0);

    let second = db
        .migrate_thread_meta_summary_v1()
        .expect("idempotent summary cutover");
    assert_eq!(second.source_row_count, 3);
    assert_eq!(second.updated_row_count, 0);
    assert!(second.already_completed);
    assert_eq!(db.list_thread_meta().unwrap(), rows);
}

#[test]
fn thread_meta_schema_v2_rebuilds_real_legacy_shape_without_reusing_cutover_markers() {
    let db = GaryxDbService::memory().expect("db opens");
    {
        let conn = db.conn().expect("writer");
        conn.execute_batch(
            "ALTER TABLE thread_meta ADD COLUMN legacy_thread_binding_key TEXT;
                 ALTER TABLE thread_meta ADD COLUMN legacy_channel TEXT;
                 ALTER TABLE thread_meta ADD COLUMN legacy_account_id TEXT;
                 ALTER TABLE thread_meta
                    ADD COLUMN legacy_has_account INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE thread_meta
                    ADD COLUMN excluded_from_recent INTEGER NOT NULL DEFAULT 0;",
        )
        .expect("seed the retired production columns");
        conn.execute(
            "INSERT INTO thread_meta (
                    thread_id, workspace_dir, thread_label, sort_updated_at_us,
                    search_text, projection_version, projected_at,
                    legacy_thread_binding_key, legacy_channel, legacy_account_id,
                    legacy_has_account, excluded_from_recent
                 ) VALUES (
                    'thread::schema-v2', '/workspace/schema-v2', 'Schema v2',
                    42, 'schema v2', 5, '2026-07-17T00:00:00Z',
                    'binding', 'telegram', 'main', 1, 1
                 )",
            [],
        )
        .expect("seed legacy projection row");
        conn.execute(
            "INSERT INTO projection_states (
                    projection_name, projection_version, source_row_count,
                    projected_at, based_on_import_generation
                 ) VALUES (?1, ?2, 7, '2026-07-17T00:00:00Z', 3),
                          (?3, ?4, 8, '2026-07-17T00:00:00Z', 3)",
            params![
                THREAD_META_SUMMARY_MIGRATION_NAME,
                THREAD_META_SUMMARY_MIGRATION_VERSION,
                RECENT_MEMBERSHIP_MIGRATION_NAME,
                RECENT_MEMBERSHIP_MIGRATION_VERSION,
            ],
        )
        .expect("seed historical cutover markers");
    }

    let first = db
        .migrate_thread_meta_schema_v2()
        .expect("schema v2 migration");
    assert_eq!(first.source_row_count, 1);
    assert_eq!(first.updated_row_count, 1);
    assert!(!first.already_completed);
    assert_eq!(
        thread_meta_column_names(&db.conn().unwrap()).unwrap(),
        THREAD_META_SCHEMA_V2_COLUMNS
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    );
    let row = db.list_thread_meta().unwrap().pop().unwrap();
    assert_eq!(row.thread_id, "thread::schema-v2");
    assert_eq!(row.workspace_dir.as_deref(), Some("/workspace/schema-v2"));
    assert_eq!(row.thread_label.as_deref(), Some("Schema v2"));
    assert_eq!(row.sort_updated_at_us, 42);
    assert_eq!(
        row.projection_version,
        CURRENT_THREAD_META_PROJECTION_VERSION
    );

    let conn = db.conn().unwrap();
    let historical = [
        (
            THREAD_META_SUMMARY_MIGRATION_NAME,
            THREAD_META_SUMMARY_MIGRATION_VERSION,
            7,
        ),
        (
            RECENT_MEMBERSHIP_MIGRATION_NAME,
            RECENT_MEMBERSHIP_MIGRATION_VERSION,
            8,
        ),
    ];
    for (name, version, source_count) in historical {
        let marker: (i64, i64, i64) = conn
            .query_row(
                "SELECT projection_version, source_row_count,
                            based_on_import_generation
                       FROM projection_states
                      WHERE projection_name = ?1",
                params![name],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(marker, (version, source_count, 3), "{name}");
    }
    drop(conn);

    let second = db
        .migrate_thread_meta_schema_v2()
        .expect("idempotent schema v2 migration");
    assert!(second.already_completed);
    assert_eq!(second.updated_row_count, 0);
}

#[test]
fn thread_meta_schema_v2_rejects_unknown_extra_columns() {
    let db = GaryxDbService::memory().expect("db opens");
    {
        let conn = db.conn().expect("writer");
        conn.execute(
            "ALTER TABLE thread_meta ADD COLUMN unknown_projection_value TEXT",
            [],
        )
        .expect("seed unknown projection column");
    }

    let error = db
        .migrate_thread_meta_schema_v2()
        .expect_err("unknown columns must not be discarded");
    assert!(
        error.to_string().contains("unknown_projection_value"),
        "unexpected error: {error}"
    );
}

#[test]
fn thread_meta_projection_round_trip_and_remove() {
    let db = GaryxDbService::memory().expect("db opens");
    let delivery_json = r#"{"channel":"telegram","account_id":"main","chat_id":"42","user_id":"42","delivery_target_type":"chat_id","delivery_target_id":"42"}"#.to_owned();
    db.replace_thread_meta_projection(ThreadMetaProjectionDraft {
        thread_id: "thread::project".to_owned(),
        thread_meta: ThreadMetaDraft {
            thread_id: "thread::project".to_owned(),
            workspace_dir: Some("/work/project".to_owned()),
            thread_type: "chat".to_owned(),
            thread_label: Some("Project Thread".to_owned()),
            agent_id: Some("codex".to_owned()),
            provider_type: Some("codex".to_owned()),
            provider_key: None,
            selected_model: None,
            selected_model_reasoning_effort: None,
            selected_model_service_tier: None,
            sdk_session_id: None,
            created_at: Some("2026-06-03T07:59:00.000Z".to_owned()),
            updated_at: Some("2026-06-03T08:00:00.000Z".to_owned()),
            message_count: 2,
            last_user_message: Some("start review".to_owned()),
            last_assistant_message: Some("done".to_owned()),
            last_message_preview: Some("done".to_owned()),
            recent_run_id: Some("run::project".to_owned()),
            active_run_id: None,
            worktree_json: Some(r#"{"path":"/work/project"}"#.to_owned()),
            last_delivery_context_json: Some(delivery_json.clone()),
            last_delivery_updated_at: Some("2026-06-03T08:00:01.000Z".to_owned()),
            default_list_hidden: false,
            sort_updated_at_us: 1_780_473_600_000_000,
            search_text: "project thread\n/work/project\ncodex\ndone".to_owned(),
        },
        channel_endpoints: vec![KnownChannelEndpoint {
            endpoint_key: "telegram::main::42".to_owned(),
            channel: "telegram".to_owned(),
            account_id: "main".to_owned(),
            binding_key: "42".to_owned(),
            chat_id: "42".to_owned(),
            delivery_target_type: "chat_id".to_owned(),
            delivery_target_id: "42".to_owned(),
            display_label: "Test User".to_owned(),
            thread_id: Some("thread::project".to_owned()),
            thread_label: Some("Project Thread".to_owned()),
            workspace_dir: Some("/work/project".to_owned()),
            thread_updated_at: Some("2026-06-03T08:00:00.000Z".to_owned()),
            last_inbound_at: Some("2026-06-03T07:59:59.000Z".to_owned()),
            last_delivery_at: Some("2026-06-03T08:00:01.000Z".to_owned()),
        }],
    })
    .expect("project thread meta");

    let meta = db.list_thread_meta().expect("list meta");
    assert_eq!(meta.len(), 1);
    assert_eq!(meta[0].thread_id, "thread::project");
    assert_eq!(meta[0].thread_type, "chat");
    assert_eq!(meta[0].workspace_dir.as_deref(), Some("/work/project"));
    assert_eq!(
        meta[0].last_delivery_context_json.as_deref(),
        Some(delivery_json.as_str())
    );
    assert_eq!(
        meta[0].last_delivery_updated_at.as_deref(),
        Some("2026-06-03T08:00:01.000Z")
    );

    let endpoints = db
        .list_thread_channel_endpoints()
        .expect("list channel endpoints");
    assert_eq!(endpoints.len(), 1);
    assert_eq!(endpoints[0].endpoint_key, "telegram::main::42");
    assert_eq!(endpoints[0].thread_id.as_deref(), Some("thread::project"));

    assert!(
        db.remove_thread_meta_projection("thread::project")
            .expect("remove projection")
    );
    assert!(
        db.list_thread_meta()
            .expect("list meta after remove")
            .is_empty()
    );
    assert!(
        db.list_thread_channel_endpoints()
            .expect("list endpoints after remove")
            .is_empty()
    );
}

#[derive(Debug, Clone, Copy)]
enum LifecycleSeedState {
    Active,
    Missing,
    Archived,
    Deleted,
}

fn seed_lifecycle_state(db: &GaryxDbService, thread_id: &str, state: LifecycleSeedState) {
    match state {
        LifecycleSeedState::Active => db
            .write_thread_record_with_projections(
                thread_id,
                &json!({"thread_id": thread_id}).to_string(),
                None,
                None,
            )
            .expect("seed active lifecycle thread"),
        LifecycleSeedState::Missing => {}
        LifecycleSeedState::Archived => {
            db.archive_thread_record(thread_id)
                .expect("seed archived tombstone");
        }
        LifecycleSeedState::Deleted => {
            db.write_thread_record_with_projections(
                thread_id,
                &json!({"thread_id": thread_id}).to_string(),
                None,
                None,
            )
            .expect("seed thread before delete");
            assert!(
                db.delete_thread_record_with_projections(
                    thread_id,
                    &garyx_router::DrainedDeleteReservation::test_witness()
                )
                .expect("seed deleted tombstone")
            );
        }
    }
}

fn expected_lifecycle_matrix(
    kind: LifecycleOperationKind,
    state: LifecycleSeedState,
) -> (LifecycleOperationOutcome, Option<ThreadTerminalState>) {
    match (kind, state) {
        (LifecycleOperationKind::Archive, LifecycleSeedState::Active) => (
            LifecycleOperationOutcome::AppliedChanged,
            Some(ThreadTerminalState::Archived),
        ),
        (LifecycleOperationKind::Archive, LifecycleSeedState::Missing)
        | (LifecycleOperationKind::Archive, LifecycleSeedState::Archived) => (
            LifecycleOperationOutcome::AppliedNoop,
            Some(ThreadTerminalState::Archived),
        ),
        (LifecycleOperationKind::Archive, LifecycleSeedState::Deleted) => (
            LifecycleOperationOutcome::RejectedNotFound,
            Some(ThreadTerminalState::Deleted),
        ),
        (LifecycleOperationKind::Delete, LifecycleSeedState::Active)
        | (LifecycleOperationKind::Delete, LifecycleSeedState::Archived) => (
            LifecycleOperationOutcome::AppliedChanged,
            Some(ThreadTerminalState::Deleted),
        ),
        (LifecycleOperationKind::Delete, LifecycleSeedState::Missing) => {
            (LifecycleOperationOutcome::RejectedNotFound, None)
        }
        (LifecycleOperationKind::Delete, LifecycleSeedState::Deleted) => (
            LifecycleOperationOutcome::AppliedNoop,
            Some(ThreadTerminalState::Deleted),
        ),
    }
}

#[test]
fn lifecycle_matrix_is_identical_immediately_and_after_reopen() {
    let kinds = [
        LifecycleOperationKind::Archive,
        LifecycleOperationKind::Delete,
    ];
    let states = [
        LifecycleSeedState::Active,
        LifecycleSeedState::Missing,
        LifecycleSeedState::Archived,
        LifecycleSeedState::Deleted,
    ];
    let mut assertions = 0usize;
    for reopen in [false, true] {
        for kind in kinds {
            for seed_state in states {
                let dir = tempfile::tempdir().expect("temp dir");
                let path = dir.path().join("garyx-db.sqlite3");
                let mut db = Some(GaryxDbService::open(&path).expect("open lifecycle db"));
                let thread_id = "thread::matrix";
                seed_lifecycle_state(db.as_ref().unwrap(), thread_id, seed_state);
                let incarnation = db
                    .as_ref()
                    .unwrap()
                    .store_incarnation_id()
                    .expect("incarnation");
                if reopen {
                    drop(db.take());
                    db = Some(GaryxDbService::open(&path).expect("reopen lifecycle db"));
                }
                let db = db.unwrap();
                let expected = expected_lifecycle_matrix(kind, seed_state);
                let result = db
                    .execute_lifecycle_mutation(LifecycleMutationInput {
                        expected_store_incarnation: incarnation,
                        operation_id: format!(
                            "matrix-{}-{}-{}",
                            kind.as_str(),
                            match seed_state {
                                LifecycleSeedState::Active => "active",
                                LifecycleSeedState::Missing => "missing",
                                LifecycleSeedState::Archived => "archived",
                                LifecycleSeedState::Deleted => "deleted",
                            },
                            reopen
                        ),
                        kind,
                        thread_id: thread_id.to_owned(),
                        fingerprint: format!("fingerprint-{assertions}"),
                        endpoint_keys: Vec::new(),
                        enabled_channel_accounts: BTreeSet::new(),
                    })
                    .expect("execute lifecycle matrix cell");
                let LifecycleTransactionResult::Completed {
                    operation,
                    durable_terminal,
                } = result
                else {
                    panic!("matrix cell did not complete: {result:?}");
                };
                assert_eq!(operation.outcome, expected.0);
                assert_eq!(durable_terminal, expected.1);
                assert_eq!(
                    db.thread_terminal_state(thread_id).unwrap(),
                    expected.1,
                    "wrong durable tombstone for {kind:?}/{seed_state:?}/reopen={reopen}"
                );
                assertions += 1;
            }
        }
    }
    assert_eq!(assertions, 16);
}

#[test]
fn lifecycle_ledger_is_fingerprinted_incarnation_scoped_and_identity_first() {
    let db = GaryxDbService::memory().expect("db opens");
    let thread_id = "thread::ledger";
    seed_lifecycle_state(&db, thread_id, LifecycleSeedState::Active);
    let first_incarnation = db.store_incarnation_id().unwrap();
    let operation_id = "operation-ledger";
    let first = db
        .execute_lifecycle_mutation(LifecycleMutationInput {
            expected_store_incarnation: first_incarnation.clone(),
            operation_id: operation_id.to_owned(),
            kind: LifecycleOperationKind::Archive,
            thread_id: thread_id.to_owned(),
            fingerprint: "fingerprint-one".to_owned(),
            endpoint_keys: Vec::new(),
            enabled_channel_accounts: BTreeSet::new(),
        })
        .unwrap();
    assert!(matches!(
        first,
        LifecycleTransactionResult::Completed { .. }
    ));
    let replay = db
        .execute_lifecycle_mutation(LifecycleMutationInput {
            expected_store_incarnation: first_incarnation.clone(),
            operation_id: operation_id.to_owned(),
            kind: LifecycleOperationKind::Delete,
            thread_id: "thread::different".to_owned(),
            fingerprint: "fingerprint-two".to_owned(),
            endpoint_keys: Vec::new(),
            enabled_channel_accounts: BTreeSet::new(),
        })
        .unwrap();
    let LifecycleTransactionResult::Existing {
        operation: existing,
        ..
    } = replay
    else {
        panic!("ledger belt did not return existing row");
    };
    assert_eq!(existing.fingerprint, "fingerprint-one");

    let second_incarnation = db.rotate_store_incarnation().unwrap().store_incarnation_id;
    let wrong = db
        .execute_lifecycle_mutation(LifecycleMutationInput {
            expected_store_incarnation: first_incarnation,
            operation_id: operation_id.to_owned(),
            kind: LifecycleOperationKind::Archive,
            thread_id: thread_id.to_owned(),
            fingerprint: "fingerprint-one".to_owned(),
            endpoint_keys: Vec::new(),
            enabled_channel_accounts: BTreeSet::new(),
        })
        .unwrap();
    assert!(matches!(
        wrong,
        LifecycleTransactionResult::WrongIncarnation {
            current_store_incarnation
        } if current_store_incarnation == second_incarnation
    ));
    let second = db
        .execute_lifecycle_mutation(LifecycleMutationInput {
            expected_store_incarnation: second_incarnation.clone(),
            operation_id: operation_id.to_owned(),
            kind: LifecycleOperationKind::Archive,
            thread_id: thread_id.to_owned(),
            fingerprint: "fingerprint-one".to_owned(),
            endpoint_keys: Vec::new(),
            enabled_channel_accounts: BTreeSet::new(),
        })
        .unwrap();
    assert!(matches!(
        second,
        LifecycleTransactionResult::Completed { .. }
    ));
    assert!(
        db.lifecycle_operation(&second_incarnation, operation_id)
            .unwrap()
            .is_some()
    );
}

#[test]
fn lifecycle_ttl_prune_makes_same_id_a_fresh_matrix_request_and_prunes_done_jobs() {
    let db = GaryxDbService::memory().expect("db opens");
    let thread_id = "thread::ttl-matrix";
    seed_lifecycle_state(&db, thread_id, LifecycleSeedState::Active);
    let incarnation = db.store_incarnation_id().unwrap();
    let operation_id = "operation-ttl";
    let input = LifecycleMutationInput {
        expected_store_incarnation: incarnation.clone(),
        operation_id: operation_id.to_owned(),
        kind: LifecycleOperationKind::Archive,
        thread_id: thread_id.to_owned(),
        fingerprint: "fingerprint-ttl".to_owned(),
        endpoint_keys: Vec::new(),
        enabled_channel_accounts: BTreeSet::new(),
    };
    let first = db.execute_lifecycle_mutation(input.clone()).unwrap();
    assert!(matches!(
        first,
        LifecycleTransactionResult::Completed {
            operation: LifecycleOperationRecord {
                outcome: LifecycleOperationOutcome::AppliedChanged,
                ..
            },
            ..
        }
    ));

    let future = "2999-01-01T00:00:00.000Z";
    let mut settled_jobs = 0usize;
    while let Some(job) = db.next_cleanup_outbox_job(future).unwrap() {
        assert!(db.mark_cleanup_outbox_done(job.job_id).unwrap());
        settled_jobs += 1;
    }
    assert_eq!(settled_jobs, 3);
    let (operations, jobs) = db.prune_lifecycle_history(future).unwrap();
    assert_eq!((operations, jobs), (1, 3));
    assert!(
        db.lifecycle_operation(&incarnation, operation_id)
            .unwrap()
            .is_none()
    );

    let after_ttl = db.execute_lifecycle_mutation(input).unwrap();
    assert!(matches!(
        after_ttl,
        LifecycleTransactionResult::Completed {
            operation: LifecycleOperationRecord {
                outcome: LifecycleOperationOutcome::AppliedNoop,
                ..
            },
            durable_terminal: Some(ThreadTerminalState::Archived),
        }
    ));
}

#[test]
fn cleanup_outbox_skips_blocked_threads_but_never_overtakes_same_thread() {
    let db = GaryxDbService::memory().expect("db opens");
    let due = "2026-07-17T00:00:00.000Z";
    let future = "2999-01-01T00:00:00.000Z";
    let now = "2026-07-17T00:00:01.000Z";
    {
        let conn = db.conn().unwrap();
        conn.execute(
                "INSERT INTO cleanup_outbox (thread_id, step, status, attempt_count, next_attempt_at, created_at)
                 VALUES ('thread::blocked', 'runtime_teardown', 'pending', 0, ?1, ?2)",
                params![future, due],
            )
            .unwrap();
        conn.execute(
                "INSERT INTO cleanup_outbox (thread_id, step, status, attempt_count, next_attempt_at, created_at)
                 VALUES ('thread::blocked', 'transcript_remove', 'pending', 0, NULL, ?1)",
                params![due],
            )
            .unwrap();
        conn.execute(
                "INSERT INTO cleanup_outbox (thread_id, step, status, attempt_count, next_attempt_at, created_at)
                 VALUES ('thread::ready', 'runtime_teardown', 'pending', 0, NULL, ?1)",
                params![due],
            )
            .unwrap();
    }
    let ready = db.next_cleanup_outbox_job(now).unwrap().unwrap();
    assert_eq!(ready.thread_id, "thread::ready");
    db.retry_cleanup_outbox_job(ready.job_id, future).unwrap();
    assert!(db.next_cleanup_outbox_job(now).unwrap().is_none());
    let persisted = {
        let conn = db.read_conn().unwrap();
        conn.query_row(
            "SELECT attempt_count, next_attempt_at FROM cleanup_outbox WHERE job_id = ?1",
            params![ready.job_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .unwrap()
    };
    assert_eq!(persisted, (1, future.to_owned()));
}

#[test]
fn cleanup_outbox_attachment_step_migration_preserves_pending_jobs() {
    let db = GaryxDbService::memory().expect("db opens");
    {
        let conn = db.conn().unwrap();
        conn.execute_batch(
            "DROP INDEX idx_cleanup_outbox_pending;
             DROP TABLE cleanup_outbox;
             CREATE TABLE cleanup_outbox (
                job_id INTEGER PRIMARY KEY AUTOINCREMENT,
                thread_id TEXT NOT NULL,
                step TEXT NOT NULL CHECK (step IN (
                    'endpoint_runtime_invalidate', 'runtime_teardown',
                    'transcript_remove', 'thread_log_remove'
                )),
                payload TEXT,
                status TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'done')),
                attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
                next_attempt_at TEXT,
                created_at TEXT NOT NULL,
                settled_at TEXT
             ) STRICT;
             CREATE INDEX idx_cleanup_outbox_pending
                ON cleanup_outbox(status, next_attempt_at)
                WHERE status = 'pending';
             INSERT INTO cleanup_outbox (
                thread_id, step, status, attempt_count, created_at
             ) VALUES (
                'thread::migration', 'transcript_remove', 'pending', 2,
                '2026-07-20T00:00:00Z'
             );",
        )
        .unwrap();
    }

    db.migrate_cleanup_outbox_prompt_attachments_v1().unwrap();

    let preserved = db
        .next_cleanup_outbox_job("2026-07-21T00:00:00Z")
        .unwrap()
        .unwrap();
    assert_eq!(preserved.thread_id, "thread::migration");
    assert_eq!(preserved.step, CleanupOutboxStep::TranscriptRemove);
    assert_eq!(preserved.attempt_count, 2);
    db.conn()
        .unwrap()
        .execute(
            "INSERT INTO cleanup_outbox (
                thread_id, step, status, attempt_count, created_at
             ) VALUES (?1, 'prompt_attachments_remove', 'pending', 0, ?2)",
            params!["thread::new-step", "2026-07-20T00:00:01Z"],
        )
        .expect("migrated schema accepts the attachment cleanup step");
}

#[test]
fn startup_recovery_abandons_only_orphaned_queued_inputs_in_one_pass() {
    let db = GaryxDbService::memory().expect("db opens");
    let thread_id = "thread::orphaned-inputs";
    db.write_thread_record_with_projections(
        thread_id,
        &json!({
            "thread_id": thread_id,
            "pending_user_inputs": [
                {"id": "queued", "status": "queued"},
                {"id": "implicit"},
                {"id": "running", "status": "running"},
                {"id": "done", "status": "abandoned"}
            ]
        })
        .to_string(),
        None,
        None,
    )
    .unwrap();
    assert_eq!(db.recover_orphaned_pending_user_inputs().unwrap(), 2);
    assert_eq!(db.recover_orphaned_pending_user_inputs().unwrap(), 0);
    let body: Value =
        serde_json::from_str(&db.get_thread_record_body(thread_id).unwrap().unwrap()).unwrap();
    let statuses = body["pending_user_inputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|input| input["status"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        statuses,
        vec!["abandoned", "abandoned", "running", "abandoned"]
    );
}

#[test]
fn missing_raw_delete_does_not_create_a_tombstone_but_real_delete_does() {
    let db = GaryxDbService::memory().expect("db opens");
    assert!(
        !db.delete_thread_record_with_projections(
            "thread::missing-delete",
            &garyx_router::DrainedDeleteReservation::test_witness()
        )
        .unwrap()
    );
    assert_eq!(
        db.thread_terminal_state("thread::missing-delete").unwrap(),
        None
    );
    seed_lifecycle_state(&db, "thread::real-delete", LifecycleSeedState::Active);
    assert!(
        db.delete_thread_record_with_projections(
            "thread::real-delete",
            &garyx_router::DrainedDeleteReservation::test_witness()
        )
        .unwrap()
    );
    assert_eq!(
        db.thread_terminal_state("thread::real-delete").unwrap(),
        Some(ThreadTerminalState::Deleted)
    );
    assert!(matches!(
        db.write_thread_record_with_projections("thread::real-delete", "{}", None, None),
        Err(GaryxDbError::ThreadArchived(_))
    ));
}

#[test]
fn legacy_archive_tombstones_migrate_to_explicit_archived_kind() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("garyx-db.sqlite3");
    {
        let conn = Connection::open(&path).expect("open legacy db");
        conn.execute_batch(
            "CREATE TABLE archived_threads (
                    thread_id TEXT PRIMARY KEY,
                    archived_at TEXT NOT NULL
                 );
                 INSERT INTO archived_threads (thread_id, archived_at)
                 VALUES ('thread::legacy-archive', '2026-07-01T00:00:00Z');",
        )
        .unwrap();
    }
    let db = GaryxDbService::open(&path).expect("migrate legacy tombstone schema");
    assert_eq!(
        db.thread_terminal_state("thread::legacy-archive").unwrap(),
        Some(ThreadTerminalState::Archived)
    );
    let columns = {
        let conn = db.read_conn().unwrap();
        let mut stmt = conn.prepare("PRAGMA table_info(archived_threads)").unwrap();
        stmt.query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    assert!(columns.iter().any(|column| column == "kind"));
}

#[test]
fn automation_thread_runs_round_trip_and_finish() {
    let db = GaryxDbService::memory().expect("db opens");
    let record = db
        .upsert_automation_thread_run(AutomationThreadRunDraft {
            automation_id: "automation::daily".to_owned(),
            run_id: "run-1".to_owned(),
            thread_id: "thread::generated".to_owned(),
            workspace_dir: Some("/Users/test/project".to_owned()),
            agent_id: Some("claude".to_owned()),
            automation_label_snapshot: Some("Daily".to_owned()),
            mode: "generated_thread".to_owned(),
            status: "running".to_owned(),
            started_at: "2026-05-28T00:00:00Z".to_owned(),
            finished_at: None,
        })
        .expect("insert automation run");

    assert_eq!(record.status, "running");
    assert_eq!(
        db.count_automation_thread_runs("automation::daily", Some("generated_thread"))
            .expect("count"),
        1
    );

    assert!(
        db.finish_automation_thread_run(
            "automation::daily",
            "run-1",
            "success",
            "2026-05-28T00:00:05Z",
        )
        .expect("finish")
    );

    let records = db
        .list_automation_thread_runs("automation::daily", Some("generated_thread"), 10, 0)
        .expect("list runs");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].thread_id, "thread::generated");
    assert_eq!(records[0].status, "success");
    assert_eq!(
        records[0].automation_label_snapshot.as_deref(),
        Some("Daily")
    );
}
