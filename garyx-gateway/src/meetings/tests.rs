use std::fs;
use std::sync::Arc;

use tempfile::TempDir;
use tokio::task::JoinSet;
use uuid::Uuid;

use super::*;
use crate::garyx_db::{
    MeetingConfirmOutcome, MeetingCreateDraft, MeetingReadCursor, MeetingStatus,
};

fn timestamp(second: u8) -> String {
    format!("2026-07-16T02:35:{second:02}.123Z")
}

fn create_meeting(
    db: &GaryxDbService,
    status: MeetingStatus,
    topic: &str,
    status_detail: &str,
) -> MeetingRecord {
    let id = Uuid::now_v7().to_string();
    db.create_meeting(MeetingCreateDraft {
        id: Some(id),
        account_id: format!("test-account-{}", Uuid::new_v4()),
        meeting_no: "123456789".to_owned(),
        feishu_meeting_id: String::new(),
        invite_event_id: format!("invite-{}", Uuid::new_v4()),
        call_id: String::new(),
        topic: topic.to_owned(),
        invited_by: "Test User".to_owned(),
        status,
        status_detail: status_detail.to_owned(),
        join_deadline_at: timestamp(59),
        grace_deadline_at: None,
        started_at: timestamp(0),
        ended_at: status.is_terminal().then(|| timestamp(2)),
        finalized_at: status.is_terminal().then(|| timestamp(3)),
        created_at: timestamp(0),
    })
    .expect("create meeting")
}

fn segment(source: &str, text: impl Into<String>) -> SegmentDraft {
    SegmentDraft {
        kind: SegmentKind::Chat,
        speaker: "Test Speaker".to_owned(),
        start: timestamp(0),
        end: timestamp(1),
        text: text.into(),
        source_id: source.to_owned(),
    }
}

fn incremental(reader_id: &str, max_bytes: Option<usize>) -> MeetingReadRequest {
    MeetingReadRequest {
        mode: read::MeetingReadMode::Incremental,
        reader_id: Some(reader_id.to_owned()),
        range_start: None,
        range_end: None,
        epoch: None,
        continue_token: None,
        max_bytes,
    }
}

fn full(max_bytes: Option<usize>) -> MeetingReadRequest {
    MeetingReadRequest {
        mode: read::MeetingReadMode::Full,
        reader_id: None,
        range_start: None,
        range_end: None,
        epoch: None,
        continue_token: None,
        max_bytes,
    }
}

fn range(start: i64, end: i64, epoch: Option<i64>) -> MeetingReadRequest {
    MeetingReadRequest {
        mode: read::MeetingReadMode::Range,
        reader_id: None,
        range_start: Some(start),
        range_end: Some(end),
        epoch,
        continue_token: None,
        max_bytes: Some(65_536),
    }
}

struct Fixture {
    _temp: TempDir,
    db: Arc<GaryxDbService>,
    service: Arc<MeetingService>,
}

impl Fixture {
    fn new(read_page_bytes: usize) -> Self {
        let temp = tempfile::tempdir().expect("temp");
        let db = Arc::new(GaryxDbService::memory().expect("db"));
        let service = Arc::new(
            MeetingService::new(db.clone(), temp.path().join("meetings"), read_page_bytes)
                .expect("service"),
        );
        Self {
            _temp: temp,
            db,
            service,
        }
    }
}

#[tokio::test]
async fn storage_fault_before_checkpoint_truncates_whole_page_and_repulls_without_duplicates() {
    let fixture = Fixture::new(65_536);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Storage", "");
    fixture
        .service
        .append_page(&meeting.id, vec![segment("one", "committed")], "cursor-1")
        .await
        .expect("first page");
    fixture
        .service
        .append_page_inner(
            &meeting.id,
            vec![
                segment("two", "uncommitted-a"),
                segment("three", "uncommitted-b"),
            ],
            "cursor-2",
            Some(AppendTestFault::Segment(2)),
        )
        .await
        .expect_err("injected crash");

    let repaired = Arc::new(
        MeetingService::new(
            fixture.db.clone(),
            fixture.service.root().to_path_buf(),
            65_536,
        )
        .expect("boot repair"),
    );
    let first = repaired
        .read(&meeting.id, incremental("reader", None))
        .await
        .expect("read");
    assert_eq!(
        first
            .segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>(),
        vec!["committed"]
    );
    repaired
        .append_page(
            &meeting.id,
            vec![
                segment("two", "uncommitted-a"),
                segment("three", "uncommitted-b"),
            ],
            "cursor-2",
        )
        .await
        .expect("repull");
    let scan = scan_log(&repaired.log_path(&meeting.id), 0, false).expect("scan");
    assert_eq!(scan.latest_seq, 3);
    assert_eq!(scan.generation, 2);
}

#[tokio::test]
async fn checkpoint_before_sqlite_repairs_forward_and_delayed_generation_cannot_regress() {
    let fixture = Fixture::new(65_536);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Repair", "");
    fixture
        .service
        .append_page_inner(
            &meeting.id,
            vec![segment("one", "page-one")],
            "cursor-1",
            Some(AppendTestFault::SyncBeforeCache),
        )
        .await
        .expect_err("cache crash");
    assert_eq!(
        fixture
            .db
            .get_meeting(&meeting.id)
            .expect("row")
            .expect("meeting")
            .cache_generation,
        0
    );
    let repaired = Arc::new(
        MeetingService::new(
            fixture.db.clone(),
            fixture.service.root().to_path_buf(),
            65_536,
        )
        .expect("repair"),
    );
    repaired
        .append_page(&meeting.id, vec![segment("two", "page-two")], "cursor-2")
        .await
        .expect("second page");
    assert!(
        !fixture
            .db
            .update_meeting_cache_guarded(&meeting.id, 0, 1, "cursor-1", 1, 1)
            .expect("delayed repair")
    );
    let record = fixture
        .db
        .get_meeting(&meeting.id)
        .expect("row")
        .expect("meeting");
    assert_eq!(record.cache_generation, 2);
    assert_eq!(record.poll_cursor, "cursor-2");
    assert_eq!(record.closed_segment_count, 2);
    let scan = scan_log(&repaired.log_path(&meeting.id), 0, false).expect("canonical scan");
    assert_eq!(record.cache_generation, scan.generation);
    assert_eq!(record.poll_cursor, scan.cursor);
    assert_eq!(record.closed_segment_count, scan.latest_seq);
    assert_eq!(record.byte_size, scan.byte_len as i64);
}

#[tokio::test]
async fn checkpoint_is_never_cached_before_fdatasync_and_torn_tail_is_discarded() {
    let fixture = Fixture::new(65_536);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Sync order", "");
    fixture
        .service
        .append_page_inner(
            &meeting.id,
            vec![segment("one", "not durable yet")],
            "cursor-1",
            Some(AppendTestFault::CheckpointBeforeSync),
        )
        .await
        .expect_err("fault before fdatasync");
    let record = fixture
        .db
        .get_meeting(&meeting.id)
        .expect("row")
        .expect("meeting");
    assert_eq!(record.cache_generation, 0);
    assert_eq!(record.closed_segment_count, 0);
    assert_eq!(record.byte_size, 0);

    let path = fixture.service.log_path(&meeting.id);
    let written_len = fs::metadata(&path).expect("faulted log").len();
    assert!(written_len > 8);
    let file = fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("open faulted log");
    file.set_len(written_len - 7)
        .expect("simulate torn storage");
    file.sync_data().expect("persist torn fixture");

    let repaired = Arc::new(
        MeetingService::new(
            fixture.db.clone(),
            fixture.service.root().to_path_buf(),
            65_536,
        )
        .expect("boot repair"),
    );
    assert_eq!(fs::metadata(&path).expect("repaired log").len(), 0);
    repaired
        .append_page(
            &meeting.id,
            vec![segment("one", "not durable yet")],
            "cursor-1",
        )
        .await
        .expect("repull page");
    let scan = scan_log(&path, 0, false).expect("scan repull");
    assert_eq!(scan.generation, 1);
    assert_eq!(scan.latest_seq, 1);
}

#[tokio::test]
async fn split_chunk_crash_is_uncommitted_and_terminal_barrier_repairs_fsynced_cache_gap() {
    let split_text = "\"\\\n".repeat(30_000);
    let normalized = normalize_page(vec![segment("split-source", split_text.clone())], 1)
        .expect("split normalization");
    assert!(normalized.len() > 1);
    assert!(
        normalized
            .iter()
            .all(|segment| { segment.cont && segment.sources.as_slice() == ["split-source"] })
    );
    for stop_after in 1..=normalized.len() {
        let fixture = Fixture::new(65_536);
        let split = create_meeting(&fixture.db, MeetingStatus::Live, "Split", "");
        fixture
            .service
            .append_page_inner(
                &split.id,
                vec![segment("split-source", split_text.clone())],
                "cursor",
                Some(AppendTestFault::Segment(stop_after)),
            )
            .await
            .expect_err("split crash");
        let repaired = Arc::new(
            MeetingService::new(
                fixture.db.clone(),
                fixture.service.root().to_path_buf(),
                65_536,
            )
            .expect("repair"),
        );
        assert_eq!(
            fs::metadata(repaired.log_path(&split.id))
                .expect("metadata")
                .len(),
            0,
            "chunk {stop_after} must remain uncommitted without its checkpoint"
        );
        assert_eq!(
            fixture
                .db
                .get_meeting(&split.id)
                .expect("row")
                .expect("meeting")
                .closed_segment_count,
            0
        );
    }

    let fixture = Fixture::new(65_536);
    let repaired = fixture.service.clone();
    let barrier = create_meeting(&fixture.db, MeetingStatus::Live, "Barrier", "");
    repaired
        .append_page_inner(
            &barrier.id,
            vec![segment("one", "committed before cache")],
            "cursor",
            Some(AppendTestFault::SyncBeforeCache),
        )
        .await
        .expect_err("cache gap");
    repaired
        .persist_terminal_index(&barrier.id)
        .await
        .expect("terminal barrier repairs cache and index");
    let record = fixture
        .db
        .get_meeting(&barrier.id)
        .expect("row")
        .expect("meeting");
    assert_eq!(record.cache_generation, 1);
    assert_eq!(record.closed_segment_count, 1);
    assert_eq!(
        record.byte_size,
        fs::metadata(repaired.log_path(&barrier.id))
            .expect("metadata")
            .len() as i64
    );
    assert!(index::index_path(&repaired.entity_dir(&barrier.id)).exists());
}

#[tokio::test]
async fn empty_checkpoint_and_foreign_epoch_tail_are_repaired_at_boot() {
    let fixture = Fixture::new(65_536);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Epoch", "");
    fixture
        .service
        .append_page(&meeting.id, Vec::new(), "empty-cursor")
        .await
        .expect("empty checkpoint");
    let valid_len = fs::metadata(fixture.service.log_path(&meeting.id))
        .expect("metadata")
        .len();
    let foreign = checkpoint_line(1, "foreign", &timestamp(2)).expect("checkpoint");
    {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(fixture.service.log_path(&meeting.id))
            .expect("open");
        file.write_all(&foreign).expect("foreign");
        file.write_all(b"\n").expect("newline");
    }
    let _repaired = MeetingService::new(
        fixture.db.clone(),
        fixture.service.root().to_path_buf(),
        65_536,
    )
    .expect("repair");
    assert_eq!(
        fs::metadata(fixture.service.log_path(&meeting.id))
            .expect("metadata")
            .len(),
        valid_len
    );
    let record = fixture
        .db
        .get_meeting(&meeting.id)
        .expect("row")
        .expect("meeting");
    assert_eq!(record.cache_generation, 1);
    assert_eq!(record.closed_segment_count, 0);
    assert_eq!(record.poll_cursor, "empty-cursor");
}

#[tokio::test]
async fn concurrent_first_fetches_share_one_cursor_span_and_receipt() {
    let fixture = Fixture::new(65_536);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Concurrent", "");
    fixture
        .service
        .append_page(
            &meeting.id,
            vec![segment("one", "one"), segment("two", "two")],
            "cursor",
        )
        .await
        .expect("page");
    let mut set = JoinSet::new();
    for _ in 0..2 {
        let service = fixture.service.clone();
        let id = meeting.id.clone();
        set.spawn(async move { service.read(&id, incremental("shared-reader", None)).await });
    }
    let one = set
        .join_next()
        .await
        .expect("one")
        .expect("task")
        .expect("read");
    let two = set
        .join_next()
        .await
        .expect("two")
        .expect("task")
        .expect("read");
    assert_eq!(one.meta.receipt, two.meta.receipt);
    assert_eq!(one.meta.span_from, two.meta.span_from);
    assert_eq!(one.meta.span_to, two.meta.span_to);
    assert_eq!(one.segments, two.segments);
    assert_eq!(
        fixture
            .db
            .count_meeting_cursors(&meeting.id)
            .expect("count"),
        1
    );
}

#[tokio::test]
async fn concurrent_fetches_with_different_budgets_replay_the_winners_indivisible_span() {
    let fixture = Fixture::new(65_536);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Budgets", "");
    fixture
        .service
        .append_page(
            &meeting.id,
            vec![
                segment("one", "a".repeat(3_000)),
                segment("two", "b".repeat(3_000)),
                segment("three", "c".repeat(3_000)),
            ],
            "cursor",
        )
        .await
        .expect("page");
    let mut set = JoinSet::new();
    for budget in [4_096, 65_536] {
        let service = fixture.service.clone();
        let id = meeting.id.clone();
        set.spawn(async move {
            service
                .read(&id, incremental("shared-budget-reader", Some(budget)))
                .await
        });
    }
    let one = set
        .join_next()
        .await
        .expect("one")
        .expect("task")
        .expect("read");
    let two = set
        .join_next()
        .await
        .expect("two")
        .expect("task")
        .expect("read");
    assert_eq!(one.meta.receipt, two.meta.receipt);
    assert_eq!(one.meta.span_from, two.meta.span_from);
    assert_eq!(one.meta.span_to, two.meta.span_to);
    assert_eq!(one.segments, two.segments);
}

#[tokio::test]
async fn two_readers_confirm_independently_and_lost_response_replays_exact_span() {
    let fixture = Fixture::new(65_536);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Readers", "");
    fixture
        .service
        .append_page(
            &meeting.id,
            vec![segment("one", "one"), segment("two", "two")],
            "cursor",
        )
        .await
        .expect("page");
    let a = fixture
        .service
        .read(&meeting.id, incremental("reader-a", None))
        .await
        .expect("reader a");
    let replay = fixture
        .service
        .read(&meeting.id, incremental("reader-a", Some(4_096)))
        .await
        .expect("forced lost-response replay");
    assert_eq!(a.meta.receipt, replay.meta.receipt);
    assert_eq!(a.segments, replay.segments);
    assert!(
        replay
            .meta
            .notes
            .iter()
            .any(|note| note.contains("pending span replay"))
    );
    let b = fixture
        .service
        .read(&meeting.id, incremental("reader-b", None))
        .await
        .expect("reader b");
    assert_ne!(a.meta.receipt, b.meta.receipt);

    for (reader, response) in [("reader-a", &a), ("reader-b", &b)] {
        assert_eq!(
            fixture
                .service
                .confirm(
                    &meeting.id,
                    ConfirmMeetingReadRequest {
                        reader_id: reader.to_owned(),
                        receipt: response.meta.receipt.clone().expect("receipt"),
                        log_epoch: response.meta.log_epoch,
                    },
                )
                .await
                .expect("confirm"),
            MeetingConfirmOutcome::Confirmed
        );
    }
    assert_eq!(
        fixture
            .db
            .get_meeting_cursor(&meeting.id, "reader-a")
            .expect("cursor")
            .expect("reader")
            .confirmed_seq,
        2
    );
    assert_eq!(
        fixture
            .db
            .get_meeting_cursor(&meeting.id, "reader-b")
            .expect("cursor")
            .expect("reader")
            .confirmed_seq,
        2
    );
}

#[tokio::test]
async fn confirm_committed_with_a_lost_response_advances_without_regression() {
    let fixture = Fixture::new(65_536);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Confirm loss", "");
    fixture
        .service
        .append_page(
            &meeting.id,
            vec![
                segment("one", "a".repeat(3_000)),
                segment("two", "b".repeat(3_000)),
            ],
            "cursor",
        )
        .await
        .expect("page");
    let first = fixture
        .service
        .read(&meeting.id, incremental("reader", Some(4_096)))
        .await
        .expect("first fetch");
    assert_eq!(
        (first.meta.span_from, first.meta.span_to),
        (Some(1), Some(1))
    );
    let first_receipt = first.meta.receipt.expect("first receipt");
    let _lost_confirm_response = fixture
        .service
        .confirm(
            &meeting.id,
            ConfirmMeetingReadRequest {
                reader_id: "reader".to_owned(),
                receipt: first_receipt.clone(),
                log_epoch: first.meta.log_epoch,
            },
        )
        .await
        .expect("confirm committed");

    let next = fixture
        .service
        .read(&meeting.id, incremental("reader", Some(4_096)))
        .await
        .expect("next fetch after lost confirm response");
    assert_eq!((next.meta.span_from, next.meta.span_to), (Some(2), Some(2)));
    assert_ne!(next.meta.receipt.as_deref(), Some(first_receipt.as_str()));
    assert_eq!(
        fixture
            .service
            .confirm(
                &meeting.id,
                ConfirmMeetingReadRequest {
                    reader_id: "reader".to_owned(),
                    receipt: first_receipt,
                    log_epoch: next.meta.log_epoch,
                },
            )
            .await
            .expect("stale same-epoch confirm"),
        MeetingConfirmOutcome::AlreadyConfirmed
    );
    let cursor = fixture
        .db
        .get_meeting_cursor(&meeting.id, "reader")
        .expect("cursor")
        .expect("reader");
    assert_eq!(cursor.confirmed_seq, 1);
    assert_eq!(cursor.pending_from, Some(2));
}

#[tokio::test]
async fn epoch_rollover_resets_confirmed_and_pending_domains_and_distinguishes_stale_receipts() {
    let fixture = Fixture::new(65_536);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Rollover", "");
    fixture
        .service
        .append_page(&meeting.id, vec![segment("old-one", "old one")], "old-1")
        .await
        .expect("old page");
    let first = fixture
        .service
        .read(&meeting.id, incremental("reader", None))
        .await
        .expect("first");
    fixture
        .service
        .confirm(
            &meeting.id,
            ConfirmMeetingReadRequest {
                reader_id: "reader".to_owned(),
                receipt: first.meta.receipt.expect("receipt"),
                log_epoch: 0,
            },
        )
        .await
        .expect("confirm");
    fixture
        .service
        .append_page(&meeting.id, vec![segment("old-two", "old two")], "old-2")
        .await
        .expect("second page");
    let pending = fixture
        .service
        .read(&meeting.id, incremental("reader", None))
        .await
        .expect("pending");
    let old_receipt = pending.meta.receipt.expect("old receipt");

    fs::remove_dir_all(fixture.service.entity_dir(&meeting.id)).expect("remove log");
    let error = fixture
        .service
        .read(&meeting.id, incremental("reader", None))
        .await
        .expect_err("rollover response");
    assert_eq!(error.code(), "snapshot_invalidated_by_content_loss");
    let cursor = fixture
        .db
        .get_meeting_cursor(&meeting.id, "reader")
        .expect("cursor")
        .expect("recognition survives");
    assert_eq!(
        cursor,
        MeetingReadCursor {
            meeting_id: meeting.id.clone(),
            reader_id: "reader".to_owned(),
            log_epoch: 1,
            confirmed_seq: 0,
            pending_from: None,
            pending_to: None,
            receipt: None,
            updated_at: cursor.updated_at.clone(),
        }
    );
    let old_confirm = fixture
        .service
        .confirm(
            &meeting.id,
            ConfirmMeetingReadRequest {
                reader_id: "reader".to_owned(),
                receipt: old_receipt,
                log_epoch: 0,
            },
        )
        .await
        .expect_err("old epoch receipt");
    assert_eq!(old_confirm.code(), "snapshot_invalidated_by_content_loss");

    fixture
        .service
        .append_page(&meeting.id, vec![segment("new-one", "new one")], "new-1")
        .await
        .expect("new epoch page");
    let epoch_one_reader = fixture
        .service
        .read(
            &meeting.id,
            incremental("reader-created-at-epoch-one", None),
        )
        .await
        .expect("new reader at epoch one");
    assert_eq!(epoch_one_reader.meta.log_epoch, 1);
    assert_eq!(
        fixture
            .service
            .confirm(
                &meeting.id,
                ConfirmMeetingReadRequest {
                    reader_id: "reader-created-at-epoch-one".to_owned(),
                    receipt: epoch_one_reader.meta.receipt.expect("epoch-one receipt"),
                    log_epoch: 1,
                },
            )
            .await
            .expect("epoch-one confirm"),
        MeetingConfirmOutcome::Confirmed
    );
    let new = fixture
        .service
        .read(&meeting.id, incremental("reader", None))
        .await
        .expect("new epoch read");
    assert_eq!(new.meta.log_epoch, 1);
    assert_eq!(new.meta.span_from, Some(1));
    let new_receipt = new.meta.receipt.clone().expect("new receipt");
    fixture
        .service
        .confirm(
            &meeting.id,
            ConfirmMeetingReadRequest {
                reader_id: "reader".to_owned(),
                receipt: new_receipt.clone(),
                log_epoch: 1,
            },
        )
        .await
        .expect("new confirm");
    assert_eq!(
        fixture
            .service
            .confirm(
                &meeting.id,
                ConfirmMeetingReadRequest {
                    reader_id: "reader".to_owned(),
                    receipt: new_receipt,
                    log_epoch: 1,
                },
            )
            .await
            .expect("same epoch stale receipt"),
        MeetingConfirmOutcome::AlreadyConfirmed
    );
    let old_range = fixture
        .service
        .read(&meeting.id, range(1, 1, Some(0)))
        .await
        .expect_err("old range epoch");
    assert_eq!(old_range.code(), "snapshot_invalidated_by_content_loss");
}

#[tokio::test]
async fn empty_increment_recognizes_reader_while_stateless_peeks_never_do() {
    let fixture = Fixture::new(65_536);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Empty", "");
    let empty = fixture
        .service
        .read(&meeting.id, incremental("recognized", None))
        .await
        .expect("empty incremental");
    assert!(empty.segments.is_empty());
    assert!(empty.meta.receipt.is_none());
    assert!(
        empty
            .meta
            .notes
            .iter()
            .any(|note| note == "first read for this reader")
    );
    assert_eq!(
        fixture
            .db
            .count_meeting_cursors(&meeting.id)
            .expect("one recognition"),
        1
    );
    fixture
        .service
        .read(&meeting.id, full(None))
        .await
        .expect("full empty");
    fixture
        .service
        .read(&meeting.id, range(1, 10, None))
        .await
        .expect("range empty");
    assert_eq!(
        fixture
            .db
            .count_meeting_cursors(&meeting.id)
            .expect("stateless untouched"),
        1
    );
}

#[tokio::test]
async fn old_snapshot_token_is_invalidated_by_missing_log_epoch_rollover() {
    let fixture = Fixture::new(4_096);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Old token", "");
    fixture
        .service
        .append_page(
            &meeting.id,
            vec![
                segment("one", "a".repeat(2_000)),
                segment("two", "b".repeat(2_000)),
                segment("three", "c".repeat(2_000)),
            ],
            "cursor",
        )
        .await
        .expect("page");
    let first = fixture
        .service
        .read(&meeting.id, full(Some(4_096)))
        .await
        .expect("first page");
    let token = first.meta.continue_token.expect("token");
    fs::remove_dir_all(fixture.service.entity_dir(&meeting.id)).expect("remove content");
    let first_failure = fixture
        .service
        .read(
            &meeting.id,
            MeetingReadRequest {
                mode: MeetingReadMode::Full,
                reader_id: None,
                range_start: None,
                range_end: None,
                epoch: None,
                continue_token: Some(token.clone()),
                max_bytes: Some(4_096),
            },
        )
        .await
        .expect_err("rollover");
    assert_eq!(first_failure.code(), "snapshot_invalidated_by_content_loss");
    let stale = fixture
        .service
        .read(
            &meeting.id,
            MeetingReadRequest {
                mode: MeetingReadMode::Full,
                reader_id: None,
                range_start: None,
                range_end: None,
                epoch: None,
                continue_token: Some(token),
                max_bytes: Some(4_096),
            },
        )
        .await
        .expect_err("stale token");
    assert_eq!(stale.code(), "snapshot_invalidated_by_content_loss");
}

#[tokio::test]
async fn missing_log_rolls_epoch_inside_terminal_barrier_and_empty_finalize_reads_back() {
    let fixture = Fixture::new(65_536);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Barrier rollover", "");
    fixture
        .service
        .append_page(&meeting.id, vec![segment("old", "lost content")], "cursor")
        .await
        .expect("old page");
    fs::remove_dir_all(fixture.service.entity_dir(&meeting.id)).expect("remove content");
    fixture
        .service
        .persist_terminal_index(&meeting.id)
        .await
        .expect("barrier rolls epoch and verifies empty state");
    let rolled = fixture
        .db
        .get_meeting(&meeting.id)
        .expect("row")
        .expect("meeting");
    assert_eq!(rolled.log_epoch, 1);
    assert_eq!(rolled.cache_generation, 0);
    assert_eq!(rolled.closed_segment_count, 0);
    assert_eq!(rolled.byte_size, 0);
    fixture
        .db
        .transition_meeting_status(
            &meeting.id,
            MeetingStatus::Live,
            MeetingStatus::Finalized,
            "finalized after content-loss rollover",
            "grace_expired",
            Some(&timestamp(2)),
            Some(&timestamp(3)),
        )
        .expect("finalize");

    let cold = Arc::new(
        MeetingService::new(
            fixture.db.clone(),
            fixture.service.root().to_path_buf(),
            65_536,
        )
        .expect("restart"),
    );
    let empty = cold
        .read(&meeting.id, full(None))
        .await
        .expect("empty finalized snapshot");
    assert_eq!(empty.meta.log_epoch, 1);
    assert_eq!(empty.meta.closed_total, 0);
    assert!(empty.segments.is_empty());
}

#[tokio::test]
async fn stateless_snapshot_stays_pinned_across_appends_and_never_creates_cursors() {
    let fixture = Fixture::new(4_096);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Snapshot", "");
    fixture
        .service
        .append_page(
            &meeting.id,
            vec![
                segment("one", "a".repeat(2_000)),
                segment("two", "b".repeat(2_000)),
                segment("three", "c".repeat(2_000)),
            ],
            "cursor-1",
        )
        .await
        .expect("page");
    let first = fixture
        .service
        .read(&meeting.id, full(Some(4_096)))
        .await
        .expect("full first");
    let token = first.meta.continue_token.clone().expect("continuation");
    let pinned_total = first.meta.closed_total;
    fixture
        .service
        .append_page(&meeting.id, vec![segment("four", "new append")], "cursor-2")
        .await
        .expect("append");
    let mode = continuation_mode_unverified(&token).expect("mode");
    let continued = fixture
        .service
        .read(
            &meeting.id,
            MeetingReadRequest {
                mode,
                reader_id: None,
                range_start: None,
                range_end: None,
                epoch: None,
                continue_token: Some(token),
                max_bytes: Some(4_096),
            },
        )
        .await
        .expect("continue");
    assert_eq!(continued.meta.closed_total, pinned_total);
    assert!(
        continued
            .segments
            .iter()
            .all(|segment| segment.seq <= pinned_total)
    );
    assert_eq!(
        fixture
            .db
            .count_meeting_cursors(&meeting.id)
            .expect("cursor count"),
        0
    );
}

#[tokio::test]
async fn range_snapshot_pages_exact_closed_interval_without_recognition_state() {
    let fixture = Fixture::new(4_096);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Range", "");
    fixture
        .service
        .append_page(
            &meeting.id,
            (1..=6)
                .map(|seq| {
                    segment(
                        &format!("source-{seq}"),
                        format!("{seq}-{}", "x".repeat(2_000)),
                    )
                })
                .collect(),
            "cursor",
        )
        .await
        .expect("page");
    let mut request = range(2, 5, Some(0));
    request.max_bytes = Some(4_096);
    let mut observed = Vec::new();
    loop {
        let response = fixture
            .service
            .read(&meeting.id, request)
            .await
            .expect("range page");
        assert_eq!(response.meta.mode, "range");
        assert_eq!(response.meta.log_epoch, 0);
        observed.extend(response.segments.iter().map(|segment| segment.seq));
        let Some(token) = response.meta.continue_token else {
            break;
        };
        request = MeetingReadRequest {
            mode: continuation_mode_unverified(&token).expect("continuation mode"),
            reader_id: None,
            range_start: None,
            range_end: None,
            epoch: None,
            continue_token: Some(token),
            max_bytes: Some(4_096),
        };
    }
    assert_eq!(observed, vec![2, 3, 4, 5]);
    assert_eq!(
        fixture
            .db
            .count_meeting_cursors(&meeting.id)
            .expect("cursor count"),
        0
    );
}

#[tokio::test]
async fn json_budget_serves_one_large_segment_and_pending_replay_is_indivisible() {
    let fixture = Fixture::new(65_536);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Budget", "");
    fixture
        .service
        .append_page(
            &meeting.id,
            vec![
                segment("large", "\"\\\n".repeat(4_000)),
                segment("small", "tail"),
            ],
            "cursor",
        )
        .await
        .expect("page");
    let first = fixture
        .service
        .read(&meeting.id, incremental("reader", Some(4_096)))
        .await
        .expect("first");
    assert_eq!(first.segments.len(), 1);
    assert!(
        first
            .meta
            .notes
            .iter()
            .any(|note| note.contains("minimum-progress overshoot"))
    );
    assert!(serde_json::to_vec(&first).expect("wire").len() > 4_096);
    let replay = fixture
        .service
        .read(&meeting.id, incremental("reader", Some(4_096)))
        .await
        .expect("replay");
    assert_eq!(first.meta.receipt, replay.meta.receipt);
    assert_eq!(first.segments, replay.segments);
    assert!(
        replay
            .meta
            .notes
            .iter()
            .any(|note| note == "pending replay exceeds requested budget")
    );
}

#[tokio::test]
async fn server_page_cap_applies_even_to_gigabyte_stateless_request() {
    let fixture = Fixture::new(4_096);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Hard cap", "");
    fixture
        .service
        .append_page(
            &meeting.id,
            (0..20)
                .map(|index| segment(&format!("source-{index}"), "x".repeat(400)))
                .collect(),
            "cursor",
        )
        .await
        .expect("page");
    let response = fixture
        .service
        .read(&meeting.id, full(Some(1024 * 1024 * 1024)))
        .await
        .expect("full");
    assert!(response.meta.continue_token.is_some());
    assert!(serde_json::to_vec(&response).expect("wire").len() <= 4_096);
}

#[tokio::test]
async fn cold_terminal_index_build_is_preflight_and_single_flight() {
    let fixture = Fixture::new(65_536);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Index", "");
    fixture
        .service
        .append_page(
            &meeting.id,
            (0..100)
                .map(|index| segment(&format!("source-{index}"), format!("segment-{index}")))
                .collect(),
            "cursor",
        )
        .await
        .expect("page");
    fixture
        .db
        .transition_meeting_status(
            &meeting.id,
            MeetingStatus::Live,
            MeetingStatus::Finalized,
            "",
            "push",
            Some(&timestamp(2)),
            Some(&timestamp(3)),
        )
        .expect("terminal");
    let cold = Arc::new(
        MeetingService::new(
            fixture.db.clone(),
            fixture.service.root().to_path_buf(),
            65_536,
        )
        .expect("cold service"),
    );
    let mut set = JoinSet::new();
    for reader in ["one", "two"] {
        let service = cold.clone();
        let id = meeting.id.clone();
        set.spawn(async move { service.read(&id, incremental(reader, None)).await });
    }
    let mut building = 0;
    while let Some(result) = set.join_next().await {
        match result.expect("task") {
            Err(error) if error.code() == "index_building" => building += 1,
            Ok(_) => {}
            Err(error) => panic!("unexpected read error: {error}"),
        }
    }
    assert!(building >= 1);
    assert_eq!(
        fixture
            .db
            .count_meeting_cursors(&meeting.id)
            .expect("preflight count"),
        0,
        "index-building preflight must not create recognition state"
    );
    let successful = loop {
        match cold.read(&meeting.id, incremental("one", None)).await {
            Ok(response) => break response,
            Err(error) if error.code() == "index_building" => {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            Err(error) => panic!("index retry failed: {error}"),
        }
    };
    assert_eq!(successful.meta.span_from, Some(1));
    assert!(index::index_path(&cold.entity_dir(&meeting.id)).exists());
    let high = cold
        .read(&meeting.id, range(96, 100, Some(0)))
        .await
        .expect("cold high-sequence range");
    assert_eq!(
        high.segments
            .iter()
            .map(|segment| segment.seq)
            .collect::<Vec<_>>(),
        vec![96, 97, 98, 99, 100]
    );
}

#[tokio::test]
async fn terminal_missing_log_marks_content_lost_without_overwriting_status_detail() {
    let fixture = Fixture::new(65_536);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Lost", "");
    fixture
        .service
        .append_page(&meeting.id, vec![segment("one", "body")], "cursor")
        .await
        .expect("page");
    fixture
        .service
        .persist_terminal_index(&meeting.id)
        .await
        .expect("barrier");
    fixture
        .db
        .transition_meeting_status(
            &meeting.id,
            MeetingStatus::Live,
            MeetingStatus::Aborted,
            "owner requested abort",
            "",
            Some(&timestamp(2)),
            Some(&timestamp(3)),
        )
        .expect("terminal");
    fs::remove_file(fixture.service.log_path(&meeting.id)).expect("remove log");
    let error = fixture
        .service
        .read(&meeting.id, incremental("reader", None))
        .await
        .expect_err("lost");
    assert_eq!(error.code(), "content_lost");
    let record = fixture
        .db
        .get_meeting(&meeting.id)
        .expect("row")
        .expect("meeting");
    assert_eq!(record.content_state, "lost");
    assert!(record.content_lost_at.is_some());
    assert_eq!(record.status_detail, "owner requested abort");
}

#[tokio::test]
async fn concurrent_content_loss_detection_and_delete_are_idempotent_and_serialized() {
    let fixture = Fixture::new(65_536);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Loss race", "");
    fixture
        .service
        .append_page(&meeting.id, vec![segment("one", "body")], "cursor")
        .await
        .expect("page");
    fixture
        .service
        .persist_terminal_index(&meeting.id)
        .await
        .expect("barrier");
    fixture
        .db
        .transition_meeting_status(
            &meeting.id,
            MeetingStatus::Live,
            MeetingStatus::Aborted,
            "preserved detail",
            "",
            Some(&timestamp(2)),
            Some(&timestamp(3)),
        )
        .expect("terminal");
    fs::remove_file(fixture.service.log_path(&meeting.id)).expect("remove log");

    let mut reads = JoinSet::new();
    for reader in ["reader-a", "reader-b"] {
        let service = fixture.service.clone();
        let id = meeting.id.clone();
        reads.spawn(async move { service.read(&id, incremental(reader, None)).await });
    }
    let delete_service = fixture.service.clone();
    let delete_id = meeting.id.clone();
    let deletion = tokio::spawn(async move { delete_service.delete(&delete_id).await });
    while let Some(result) = reads.join_next().await {
        let error = result
            .expect("read task")
            .expect_err("missing terminal content cannot be read");
        assert!(
            matches!(error.code(), "content_lost" | "entity_deleted"),
            "unexpected loss/delete race error: {error}"
        );
    }
    deletion
        .await
        .expect("delete task")
        .expect("terminal deletion");
    assert!(fixture.db.get_meeting(&meeting.id).expect("row").is_none());
    assert!(!fixture.service.entity_dir(&meeting.id).exists());
    assert!(
        !fixture
            .service
            .root()
            .join(format!("{}.tombstone", meeting.id))
            .exists()
    );
}

#[tokio::test]
async fn tombstone_first_delete_recovers_every_crash_boundary_and_cascades() {
    let fixture = Fixture::new(65_536);
    let restored = create_meeting(&fixture.db, MeetingStatus::Aborted, "Delete one", "done");
    fs::create_dir_all(fixture.service.entity_dir(&restored.id)).expect("dir");
    fs::write(fixture.service.log_path(&restored.id), b"").expect("log");
    fixture
        .db
        .prepare_meeting_cursor(&restored.id, "reader", 0)
        .expect("cursor")
        .expect("domain");
    for index in 0..32 {
        fixture
            .db
            .prepare_meeting_cursor(&restored.id, &format!("minted-reader-{index}"), 0)
            .expect("minted cursor")
            .expect("minted cursor domain");
    }
    fixture
        .service
        .delete_inner(&restored.id, Some(DeleteTestFault::AfterRename))
        .await
        .expect_err("rename crash");
    assert!(
        fixture
            .service
            .root()
            .join(format!("{}.tombstone", restored.id))
            .exists()
    );
    let recovered = Arc::new(
        MeetingService::new(
            fixture.db.clone(),
            fixture.service.root().to_path_buf(),
            65_536,
        )
        .expect("restore boot"),
    );
    assert!(recovered.entity_dir(&restored.id).exists());
    assert!(fixture.db.get_meeting(&restored.id).expect("row").is_some());

    let committed = create_meeting(&fixture.db, MeetingStatus::Finalized, "Delete two", "");
    fs::create_dir_all(recovered.entity_dir(&committed.id)).expect("dir");
    recovered
        .delete_inner(&committed.id, Some(DeleteTestFault::AfterDatabaseCommit))
        .await
        .expect_err("commit crash");
    assert!(
        recovered
            .root()
            .join(format!("{}.tombstone", committed.id))
            .exists()
    );
    let reconciled = Arc::new(
        MeetingService::new(fixture.db.clone(), recovered.root().to_path_buf(), 65_536)
            .expect("remove boot"),
    );
    assert!(
        !reconciled
            .root()
            .join(format!("{}.tombstone", committed.id))
            .exists()
    );

    let empty = create_meeting(&fixture.db, MeetingStatus::Aborted, "Empty", "");
    reconciled
        .delete(&empty.id)
        .await
        .expect("missing-dir delete");
    reconciled
        .delete(&restored.id)
        .await
        .expect("delete restored");
    assert_eq!(
        fixture
            .db
            .count_meeting_cursors(&restored.id)
            .expect("cascade cursor"),
        0
    );
    assert_eq!(
        fixture
            .db
            .count_meeting_invite_keys(&restored.id)
            .expect("cascade invite"),
        0
    );
    let deleted_read = reconciled
        .read(&restored.id, incremental("reader", None))
        .await
        .expect_err("deleted read");
    assert_eq!(deleted_read.code(), "entity_deleted");
    let deleted_confirm = reconciled
        .confirm(
            &restored.id,
            ConfirmMeetingReadRequest {
                reader_id: "reader".to_owned(),
                receipt: "stale-receipt".to_owned(),
                log_epoch: 0,
            },
        )
        .await
        .expect_err("deleted confirm");
    assert_eq!(deleted_confirm.code(), "entity_deleted");
}

#[tokio::test]
async fn delete_waits_for_inflight_entity_read_lock_before_tombstoning() {
    let fixture = Fixture::new(65_536);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Aborted, "Wait", "");
    let state = fixture.service.entity_state(&meeting.id);
    let guard = state.lock.clone().read_owned().await;
    let service = fixture.service.clone();
    let id = meeting.id.clone();
    let mut deletion = tokio::spawn(async move { service.delete(&id).await });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut deletion)
            .await
            .is_err(),
        "delete must wait behind readers/index rebuilds"
    );
    drop(guard);
    deletion
        .await
        .expect("delete task")
        .expect("delete after read");
    assert!(fixture.db.get_meeting(&meeting.id).expect("row").is_none());
}

#[tokio::test]
async fn fetch_confirm_delete_tripartite_race_has_only_linearized_outcomes() {
    let fixture = Fixture::new(65_536);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Tripartite", "");
    fixture
        .service
        .append_page(&meeting.id, vec![segment("one", "body")], "cursor")
        .await
        .expect("content");
    fixture
        .service
        .persist_terminal_index(&meeting.id)
        .await
        .expect("terminal barrier");
    fixture
        .db
        .transition_meeting_status(
            &meeting.id,
            MeetingStatus::Live,
            MeetingStatus::Aborted,
            "done",
            "",
            Some(&timestamp(2)),
            Some(&timestamp(3)),
        )
        .expect("terminal transition");
    let fetched = loop {
        match fixture
            .service
            .read(&meeting.id, incremental("reader", None))
            .await
        {
            Ok(response) => break response,
            Err(error) if error.code() == "index_building" => {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            Err(error) => panic!("unexpected fetch error: {error}"),
        }
    };
    let receipt = fetched.meta.receipt.expect("pending receipt");

    let state = fixture.service.entity_state(&meeting.id);
    let blocker = state.lock.clone().read_owned().await;
    let delete_service = fixture.service.clone();
    let delete_id = meeting.id.clone();
    let deletion = tokio::spawn(async move { delete_service.delete(&delete_id).await });
    tokio::task::yield_now().await;
    let confirm_service = fixture.service.clone();
    let confirm_id = meeting.id.clone();
    let confirmation = tokio::spawn(async move {
        confirm_service
            .confirm(
                &confirm_id,
                ConfirmMeetingReadRequest {
                    reader_id: "reader".to_owned(),
                    receipt,
                    log_epoch: 0,
                },
            )
            .await
    });
    drop(blocker);

    deletion
        .await
        .expect("delete task")
        .expect("delete linearized");
    match confirmation.await.expect("confirm task") {
        Ok(MeetingConfirmOutcome::Confirmed) => {}
        Err(error) if error.code() == "entity_deleted" => {}
        other => panic!("unexpected confirm/delete race outcome: {other:?}"),
    }
    assert!(fixture.db.get_meeting(&meeting.id).expect("row").is_none());
    assert_eq!(
        fixture
            .db
            .count_meeting_cursors(&meeting.id)
            .expect("cursor cascade"),
        0
    );
}

#[test]
fn keyset_list_has_weak_insert_semantics_and_updates_never_move_rows() {
    let db = GaryxDbService::memory().expect("db");
    let high_id = "00000000-0000-7000-9000-000000000003".to_owned();
    let middle_id = "00000000-0000-7000-9000-000000000002".to_owned();
    for (id, topic) in [(&high_id, "high"), (&middle_id, "middle")] {
        db.create_meeting(MeetingCreateDraft {
            id: Some(id.clone()),
            account_id: format!("account-{id}"),
            meeting_no: "123456789".to_owned(),
            feishu_meeting_id: String::new(),
            invite_event_id: format!("invite-{id}"),
            call_id: String::new(),
            topic: topic.to_owned(),
            invited_by: "Test User".to_owned(),
            status: MeetingStatus::Aborted,
            status_detail: String::new(),
            join_deadline_at: timestamp(59),
            grace_deadline_at: None,
            started_at: timestamp(0),
            ended_at: Some(timestamp(1)),
            finalized_at: Some(timestamp(2)),
            created_at: timestamp(0),
        })
        .expect("row");
    }
    let first = db.list_meetings_page(1, None).expect("first");
    assert_eq!(first[0].id, high_id);

    let low_id = "00000000-0000-7000-9000-000000000001".to_owned();
    db.create_meeting(MeetingCreateDraft {
        id: Some(low_id.clone()),
        account_id: "account-low".to_owned(),
        meeting_no: "123456789".to_owned(),
        feishu_meeting_id: String::new(),
        invite_event_id: "invite-low".to_owned(),
        call_id: String::new(),
        topic: "low".to_owned(),
        invited_by: "Test User".to_owned(),
        status: MeetingStatus::Aborted,
        status_detail: String::new(),
        join_deadline_at: timestamp(59),
        grace_deadline_at: None,
        started_at: timestamp(0),
        ended_at: Some(timestamp(1)),
        finalized_at: Some(timestamp(2)),
        created_at: timestamp(0),
    })
    .expect("low insert");
    db.transition_meeting_status(
        &middle_id,
        MeetingStatus::Aborted,
        MeetingStatus::Finalized,
        "updated",
        "push",
        Some(&timestamp(1)),
        Some(&timestamp(2)),
    )
    .expect("update");
    db.delete_terminal_meeting_row(&middle_id)
        .expect("delete middle");
    let later = db
        .list_meetings_page(10, Some((timestamp(0), high_id)))
        .expect("later");
    assert_eq!(
        later
            .into_iter()
            .map(|record| record.id)
            .collect::<Vec<_>>(),
        vec![low_id.clone()]
    );

    let skew_id = "00000000-0000-7000-9000-000000000000".to_owned();
    db.create_meeting(MeetingCreateDraft {
        id: Some(skew_id.clone()),
        account_id: "account-skew".to_owned(),
        meeting_no: "123456789".to_owned(),
        feishu_meeting_id: String::new(),
        invite_event_id: "invite-skew".to_owned(),
        call_id: String::new(),
        topic: "skew".to_owned(),
        invited_by: "Test User".to_owned(),
        status: MeetingStatus::Aborted,
        status_detail: String::new(),
        join_deadline_at: timestamp(59),
        grace_deadline_at: None,
        started_at: timestamp(0),
        ended_at: Some(timestamp(1)),
        finalized_at: Some(timestamp(2)),
        created_at: "2026-07-16T02:34:59.123Z".to_owned(),
    })
    .expect("clock-skew insert");
    let skew_page = db
        .list_meetings_page(10, Some((timestamp(0), low_id)))
        .expect("skew page");
    assert_eq!(
        skew_page
            .into_iter()
            .map(|record| record.id)
            .collect::<Vec<_>>(),
        vec![skew_id]
    );
}

#[tokio::test]
async fn real_http_routes_enforce_wire_schema_confirm_and_terminal_delete() {
    use axum::body::{Body, to_bytes};
    use tower::ServiceExt;

    use crate::route_graph::build_router;
    use crate::server::AppStateBuilder;

    let temp = tempfile::tempdir().expect("temp");
    let db = Arc::new(GaryxDbService::memory().expect("db"));
    let state = AppStateBuilder::new(crate::test_support::with_gateway_auth(
        garyx_models::config::GaryxConfig::default(),
    ))
    .with_garyx_db(db.clone())
    .with_meetings_dir(temp.path().join("meetings"))
    .build();
    let meeting = create_meeting(&db, MeetingStatus::Live, "HTTP topic", "");
    state
        .ops
        .meetings
        .append_page(&meeting.id, vec![segment("one", "body")], "cursor")
        .await
        .expect("page");
    let other = create_meeting(&db, MeetingStatus::Aborted, "Other", "");
    let router = build_router(state.clone());

    let unauthenticated = axum::http::Request::builder()
        .uri("/api/meetings")
        .body(Body::empty())
        .expect("request");
    assert_eq!(
        router
            .clone()
            .oneshot(unauthenticated)
            .await
            .expect("response")
            .status(),
        StatusCode::UNAUTHORIZED
    );

    let list = crate::test_support::authed_request()
        .uri("/api/meetings?limit=1")
        .body(Body::empty())
        .expect("request");
    let response = router.clone().oneshot(list).await.expect("list");
    assert_eq!(response.status(), StatusCode::OK);
    let list_body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body");
    let list_json: Value = serde_json::from_slice(&list_body).expect("json");
    assert_eq!(list_json["meetings"].as_array().expect("meetings").len(), 1);
    assert!(list_json["next_page_token"].is_string());

    let invalid = crate::test_support::authed_request()
        .method("POST")
        .uri(format!("/api/meetings/{}/read", meeting.id))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "mode": "full",
                "epoch": 0
            }))
            .expect("json"),
        ))
        .expect("request");
    let response = router.clone().oneshot(invalid).await.expect("invalid");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body");
    let error: Value = serde_json::from_slice(&body).expect("error");
    assert_eq!(error["error"]["code"], "invalid_epoch");

    let fetch = crate::test_support::authed_request()
        .method("POST")
        .uri(format!("/api/meetings/{}/read", meeting.id))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "mode": "incremental",
                "reader_id": "http-reader",
                "max_bytes": 4096
            }))
            .expect("json"),
        ))
        .expect("request");
    let response = router.clone().oneshot(fetch).await.expect("fetch");
    assert_eq!(response.status(), StatusCode::OK);
    let raw = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body");
    let wire: Value = serde_json::from_slice(&raw).expect("wire JSON");
    let meta = wire["meta"].as_object().expect("meta object");
    for field in [
        "mode",
        "entity_id",
        "log_epoch",
        "status",
        "status_detail",
        "end_source",
        "stalled_reason",
        "content_state",
        "topic",
        "started_at",
        "ended_at",
        "finalized_at",
        "content_lost_at",
        "updated_at",
        "span_from",
        "span_to",
        "closed_total",
        "receipt",
        "continue_token",
        "notes",
    ] {
        assert!(
            meta.contains_key(field),
            "missing complete DTO field {field}"
        );
    }
    assert_eq!(meta.len(), 20);
    let segment = wire["segments"][0].as_object().expect("segment object");
    for field in [
        "seq", "kind", "speaker", "start", "end", "text", "sources", "cont",
    ] {
        assert!(
            segment.contains_key(field),
            "missing segment DTO field {field}"
        );
    }
    assert_eq!(segment.len(), 8);
    assert!(!segment.contains_key("t"));
    let fetched: MeetingReadResponse = serde_json::from_slice(&raw).expect("DTO");
    assert_eq!(
        serde_json::to_vec(&fetched).expect("DTO round trip").len(),
        raw.len(),
        "budget algebra must equal the exact structured HTTP body bytes"
    );
    assert_eq!(fetched.meta.mode, "incremental");
    assert_eq!(fetched.meta.span_from, Some(1));
    assert_eq!(fetched.meta.closed_total, 1);
    assert_eq!(fetched.segments[0].text, "body");
    let receipt = fetched.meta.receipt.expect("receipt");

    let confirm = crate::test_support::authed_request()
        .method("POST")
        .uri(format!("/api/meetings/{}/read/confirm", meeting.id))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&json!({
                "reader_id": "http-reader",
                "receipt": receipt,
                "log_epoch": 0
            }))
            .expect("json"),
        ))
        .expect("request");
    let response = router.clone().oneshot(confirm).await.expect("confirm");
    assert_eq!(response.status(), StatusCode::OK);
    let cursor = db
        .get_meeting_cursor(&meeting.id, "http-reader")
        .expect("cursor")
        .expect("row");
    assert_eq!(cursor.confirmed_seq, 1);
    assert!(cursor.receipt.is_none());

    let refused_delete = crate::test_support::authed_request()
        .method("DELETE")
        .uri(format!("/api/meetings/{}", meeting.id))
        .body(Body::empty())
        .expect("request");
    let response = router
        .clone()
        .oneshot(refused_delete)
        .await
        .expect("delete");
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let delete = crate::test_support::authed_request()
        .method("DELETE")
        .uri(format!("/api/meetings/{}", other.id))
        .body(Body::empty())
        .expect("request");
    assert_eq!(
        router
            .clone()
            .oneshot(delete)
            .await
            .expect("delete")
            .status(),
        StatusCode::OK
    );
    let deleted_read = crate::test_support::authed_request()
        .method("POST")
        .uri(format!("/api/meetings/{}/read", other.id))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"mode":"full"}"#))
        .expect("request");
    let response = router.oneshot(deleted_read).await.expect("deleted read");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body");
    let error: Value = serde_json::from_slice(&body).expect("error");
    assert_eq!(error["error"]["code"], "entity_deleted");
}
