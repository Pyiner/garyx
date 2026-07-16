use std::collections::VecDeque;
use std::fs;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use garyx_channels::{
    JoinedMeeting, MeetingApiError, MeetingEventSink, MeetingInvite, MeetingPlatformClient,
};
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

#[derive(Clone)]
struct FakeMeetingClient {
    joins: Arc<Mutex<VecDeque<Result<JoinedMeeting, MeetingApiError>>>>,
    default_join: Result<JoinedMeeting, MeetingApiError>,
    join_calls: Arc<AtomicUsize>,
    leave_calls: Arc<AtomicUsize>,
    hang_join: Arc<AtomicBool>,
    hang_leave: Arc<AtomicBool>,
    leave_result: Result<(), MeetingApiError>,
    bot_open_id: String,
}

impl FakeMeetingClient {
    fn successful(feishu_meeting_id: &str, bot_open_id: &str) -> Self {
        Self {
            joins: Arc::new(Mutex::new(VecDeque::new())),
            default_join: Ok(JoinedMeeting {
                feishu_meeting_id: feishu_meeting_id.to_owned(),
            }),
            join_calls: Arc::new(AtomicUsize::new(0)),
            leave_calls: Arc::new(AtomicUsize::new(0)),
            hang_join: Arc::new(AtomicBool::new(false)),
            hang_leave: Arc::new(AtomicBool::new(false)),
            leave_result: Ok(()),
            bot_open_id: bot_open_id.to_owned(),
        }
    }

    fn failing(error: MeetingApiError, bot_open_id: &str) -> Self {
        Self {
            joins: Arc::new(Mutex::new(VecDeque::new())),
            default_join: Err(error),
            join_calls: Arc::new(AtomicUsize::new(0)),
            leave_calls: Arc::new(AtomicUsize::new(0)),
            hang_join: Arc::new(AtomicBool::new(false)),
            hang_leave: Arc::new(AtomicBool::new(false)),
            leave_result: Ok(()),
            bot_open_id: bot_open_id.to_owned(),
        }
    }
}

#[async_trait]
impl MeetingPlatformClient for FakeMeetingClient {
    async fn join(
        &self,
        _meeting_no: &str,
        _password: Option<&str>,
    ) -> Result<JoinedMeeting, MeetingApiError> {
        self.join_calls.fetch_add(1, Ordering::AcqRel);
        if self.hang_join.load(Ordering::Acquire) {
            return std::future::pending().await;
        }
        self.joins
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front()
            .unwrap_or_else(|| self.default_join.clone())
    }

    async fn leave(&self, _feishu_meeting_id: &str) -> Result<(), MeetingApiError> {
        self.leave_calls.fetch_add(1, Ordering::AcqRel);
        if self.hang_leave.load(Ordering::Acquire) {
            std::future::pending().await
        } else {
            self.leave_result.clone()
        }
    }

    fn bot_open_id(&self) -> Option<String> {
        Some(self.bot_open_id.clone())
    }
}

fn start_test_ingestion(service: &Arc<MeetingService>, finalizing_grace: Duration) {
    service.start_ingestion(1);
    service.set_test_ingestion_timing(
        Duration::from_millis(20),
        Duration::from_millis(300),
        finalizing_grace,
        Duration::from_millis(20),
    );
}

fn synthetic_invite(account_id: &str, event_id: &str) -> MeetingInvite {
    MeetingInvite {
        account_id: account_id.to_owned(),
        event_id: event_id.to_owned(),
        meeting_reference_id: "9007199254740993001".to_owned(),
        meeting_no: "123456789".to_owned(),
        topic: "Synthetic push meeting".to_owned(),
        bot_id: "bot_1000000001".to_owned(),
        inviter_id: "user_1000000001".to_owned(),
    }
}

async fn wait_for_record(
    db: &GaryxDbService,
    predicate: impl Fn(&MeetingRecord) -> bool,
) -> MeetingRecord {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(record) = db
                .list_all_meetings()
                .expect("list meeting records")
                .into_iter()
                .find(&predicate)
            {
                return record;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("meeting state convergence")
}

async fn wait_for_counter(counter: &AtomicUsize, minimum: usize) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while counter.load(Ordering::Acquire) < minimum {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("meeting platform call convergence");
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
async fn storage_fault_before_checkpoint_truncates_whole_batch_and_retry_deduplicates() {
    let fixture = Fixture::new(65_536);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Storage", "");
    fixture
        .service
        .append_batch(&meeting.id, vec![segment("one", "committed")], "event-1")
        .await
        .expect("first batch");
    fixture
        .service
        .append_batch_inner(
            &meeting.id,
            vec![
                segment("two", "uncommitted-a"),
                segment("three", "uncommitted-b"),
            ],
            "event-2",
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
        .append_batch(
            &meeting.id,
            vec![
                segment("two", "uncommitted-a"),
                segment("three", "uncommitted-b"),
            ],
            "event-2",
        )
        .await
        .expect("redelivered batch");
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
        .append_batch_inner(
            &meeting.id,
            vec![segment("one", "batch-one")],
            "event-1",
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
        .append_batch(&meeting.id, vec![segment("two", "batch-two")], "event-2")
        .await
        .expect("second batch");
    assert!(
        !fixture
            .db
            .update_meeting_cache_guarded(&meeting.id, 0, 1, 1, 1)
            .expect("delayed repair")
    );
    let record = fixture
        .db
        .get_meeting(&meeting.id)
        .expect("row")
        .expect("meeting");
    assert_eq!(record.cache_generation, 2);
    assert_eq!(record.closed_segment_count, 2);
    let scan = scan_log(&repaired.log_path(&meeting.id), 0, false).expect("canonical scan");
    assert_eq!(record.cache_generation, scan.generation);
    assert_eq!(record.closed_segment_count, scan.latest_seq);
    assert_eq!(record.byte_size, scan.byte_len as i64);
}

#[tokio::test]
async fn checkpoint_is_never_cached_before_fdatasync_and_torn_tail_is_discarded() {
    let fixture = Fixture::new(65_536);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Sync order", "");
    fixture
        .service
        .append_batch_inner(
            &meeting.id,
            vec![segment("one", "not durable yet")],
            "event-1",
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
        .append_batch(
            &meeting.id,
            vec![segment("one", "not durable yet")],
            "event-1",
        )
        .await
        .expect("redelivered batch");
    let scan = scan_log(&path, 0, false).expect("scan redelivery");
    assert_eq!(scan.generation, 1);
    assert_eq!(scan.latest_seq, 1);
}

#[tokio::test]
async fn split_chunk_crash_is_uncommitted_and_terminal_barrier_repairs_fsynced_cache_gap() {
    let split_text = "\"\\\n".repeat(30_000);
    let normalized = normalize_batch(vec![segment("split-source", split_text.clone())], 1)
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
            .append_batch_inner(
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
        .append_batch_inner(
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
        .append_batch(&meeting.id, Vec::new(), "empty-cursor")
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
}

#[tokio::test]
async fn concurrent_first_fetches_share_one_cursor_span_and_receipt() {
    let fixture = Fixture::new(65_536);
    let meeting = create_meeting(&fixture.db, MeetingStatus::Live, "Concurrent", "");
    fixture
        .service
        .append_batch(
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
        .append_batch(
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
        .append_batch(
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
        .append_batch(
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
        .append_batch(&meeting.id, vec![segment("old-one", "old one")], "old-1")
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
        .append_batch(&meeting.id, vec![segment("old-two", "old two")], "old-2")
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
        .append_batch(&meeting.id, vec![segment("new-one", "new one")], "new-1")
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
        .append_batch(
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
        .append_batch(&meeting.id, vec![segment("old", "lost content")], "cursor")
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
            "push",
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
        .append_batch(
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
        .append_batch(&meeting.id, vec![segment("four", "new append")], "cursor-2")
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
        .append_batch(
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
        .append_batch(
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
        .append_batch(
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
        .append_batch(
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
        .append_batch(&meeting.id, vec![segment("one", "body")], "cursor")
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
        .append_batch(&meeting.id, vec![segment("one", "body")], "cursor")
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
        .append_batch(&meeting.id, vec![segment("one", "body")], "cursor")
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
        .append_batch(&meeting.id, vec![segment("one", "body")], "cursor")
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

#[tokio::test]
async fn real_http_abort_route_covers_durable_state_table() {
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
    let joining = create_meeting(&db, MeetingStatus::Joining, "Joining", "");
    let finalizing = create_meeting(&db, MeetingStatus::Finalizing, "Finalizing", "");
    let aborted = create_meeting(&db, MeetingStatus::Aborted, "Aborted", "");
    let router = build_router(state);

    let post_abort = |id: &str| {
        crate::test_support::authed_request()
            .method("POST")
            .uri(format!("/api/meetings/{id}/abort"))
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .expect("request")
    };
    let response = router
        .clone()
        .oneshot(post_abort(&joining.id))
        .await
        .expect("joining abort");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024).await.expect("body");
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["status"],
        "aborting"
    );
    assert_eq!(
        router
            .clone()
            .oneshot(post_abort(&joining.id))
            .await
            .expect("idempotent abort")
            .status(),
        StatusCode::OK
    );

    let response = router
        .clone()
        .oneshot(post_abort(&finalizing.id))
        .await
        .expect("finalizing refusal");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), 1024).await.expect("body");
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["error"]["code"],
        "abort_refused_finalizing"
    );
    assert_eq!(
        router
            .clone()
            .oneshot(post_abort(&aborted.id))
            .await
            .expect("aborted no-op")
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        router
            .oneshot(post_abort("00000000-0000-7000-8000-000000000099"))
            .await
            .expect("deleted")
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn push_capture_is_batch_atomic_deduplicated_exact_and_trails_to_finalize() {
    let fixture = Fixture::new(65_536);
    start_test_ingestion(&fixture.service, Duration::from_millis(150));
    let account_id = "synthetic-push-account";
    let bot_open_id = "ou_bot_1000000001";
    let exact_meeting_id = "9007199254740993123";
    let client = Arc::new(FakeMeetingClient::successful(exact_meeting_id, bot_open_id));
    fixture.service.register_client(account_id, client.clone());
    fixture
        .service
        .on_meeting_invited(synthetic_invite(account_id, "evt-invite-1"));
    let live = wait_for_record(&fixture.db, |record| record.status == "live").await;
    assert_eq!(live.feishu_meeting_id, exact_meeting_id);

    let first_event = json!({
        "meeting_activity_items": [
            {
                "activity_event_type": "transcript_received",
                "meeting": { "id": 9007199254740993123u64 },
                "transcript_received_items": [
                    {
                        "text": "exact transcript",
                        "language": "en",
                        "sentence_id": 9007199254740993u64,
                        "start_time_ms": 1000,
                        "end_time_ms": 2000,
                        "speaker": {
                            "id": { "open_id": "ou_user_1000000001" },
                            "user_name": "Test Speaker"
                        }
                    },
                    {
                        "text": "bot-authored transcript remains a 1:1 item",
                        "language": "en",
                        "sentence_id": "sentence-from-bot",
                        "start_time_ms": 2000,
                        "end_time_ms": 3000,
                        "speaker": {
                            "id": { "open_id": bot_open_id },
                            "user_name": "Meeting Bot"
                        }
                    }
                ]
            },
            {
                "activity_event_type": "chat_received",
                "meeting": { "id": 9007199254740993123u64 },
                "chat_received_items": [{
                    "message_id": "om_synthetic_1",
                    "content": "exact chat",
                    "sent_timestamp": 3000,
                    "operator": {
                        "id": { "open_id": "ou_user_1000000002" },
                        "name": "Test Operator"
                    }
                }]
            },
            {
                "activity_event_type": "participant_left",
                "meeting": { "id": 9007199254740993123u64 },
                "participant_left_items": [{
                    "participant": { "id": { "open_id": "ou_user_1000000003" } }
                }]
            }
        ]
    });
    fixture
        .service
        .on_meeting_activity(account_id, "evt-activity-1", first_event.clone());
    let committed = wait_for_record(&fixture.db, |record| {
        record.id == live.id && record.cache_generation == 1
    })
    .await;
    assert_eq!(committed.closed_segment_count, 3);
    let first_scan = scan_log(&fixture.service.log_path(&live.id), 0, false).expect("scan");
    assert!(first_scan.source_ids.contains("9007199254740993"));
    assert!(first_scan.source_ids.contains("sentence-from-bot"));

    fixture
        .service
        .on_meeting_activity(account_id, "evt-activity-1", first_event);
    tokio::time::sleep(Duration::from_millis(30)).await;
    let whole_redelivery = fixture
        .db
        .get_meeting(&live.id)
        .expect("record")
        .expect("meeting");
    assert_eq!(whole_redelivery.cache_generation, 1);
    assert_eq!(whole_redelivery.closed_segment_count, 3);

    fixture.service.on_meeting_activity(
        account_id,
        "evt-activity-2",
        json!({
            "meeting_activity_items": [{
                "activity_event_type": "chat_received",
                "meeting": { "id": 9007199254740993123u64 },
                "chat_received_items": [
                    {
                        "message_id": "om_synthetic_1",
                        "content": "duplicate source",
                        "sent_timestamp": 4000,
                        "operator": { "id": { "open_id": "ou_user_1000000002" } }
                    },
                    {
                        "message_id": "om_synthetic_2",
                        "content": "new source",
                        "sent_timestamp": 5000,
                        "operator": { "id": { "open_id": "ou_user_1000000002" } }
                    }
                ]
            }]
        }),
    );
    let item_dedup = wait_for_record(&fixture.db, |record| {
        record.id == live.id && record.cache_generation == 2
    })
    .await;
    assert_eq!(item_dedup.closed_segment_count, 4);

    fixture.service.on_meeting_activity(
        account_id,
        "evt-activity-left",
        json!({
            "meeting_activity_items": [{
                "activity_event_type": "participant_left",
                "meeting": { "id": 9007199254740993123u64 },
                "participant_left_items": [{
                    "participant": { "id": { "open_id": bot_open_id } }
                }]
            }]
        }),
    );
    wait_for_record(&fixture.db, |record| {
        record.id == live.id && record.status == "finalizing"
    })
    .await;
    fixture.service.on_meeting_activity(
        account_id,
        "evt-activity-trailing",
        json!({
            "meeting_activity_items": [{
                "activity_event_type": "transcript_received",
                "meeting": { "id": 9007199254740993123u64 },
                "transcript_received_items": [{
                    "text": "trailing transcript",
                    "language": "en",
                    "sentence_id": "sentence-trailing",
                    "start_time_ms": 6000,
                    "end_time_ms": 7000,
                    "speaker": {
                        "id": { "open_id": "ou_user_1000000001" },
                        "user_name": "Test Speaker"
                    }
                }]
            }]
        }),
    );
    let finalized = wait_for_record(&fixture.db, |record| {
        record.id == live.id && record.status == "finalized"
    })
    .await;
    assert_eq!(finalized.end_source, "participant_left");
    assert_eq!(finalized.closed_segment_count, 5);
    assert_eq!(finalized.cache_generation, 4);

    fixture.service.on_meeting_activity(
        account_id,
        "evt-after-finalized",
        json!({ "meeting_activity_items": [] }),
    );
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(
        fixture
            .db
            .get_meeting(&live.id)
            .expect("record")
            .expect("meeting")
            .cache_generation,
        4
    );
}

#[tokio::test]
async fn end_signal_wins_same_scheduling_point_over_abort_domain() {
    let fixture = Fixture::new(65_536);
    start_test_ingestion(&fixture.service, Duration::from_millis(60));
    let account_id = "end-wins-account";
    let client = Arc::new(FakeMeetingClient::successful(
        "9007199254740993222",
        "ou_bot_end_wins",
    ));
    fixture.service.register_client(account_id, client.clone());
    fixture
        .service
        .on_meeting_invited(synthetic_invite(account_id, "evt-end-wins"));
    let live = wait_for_record(&fixture.db, |record| record.status == "live").await;

    // No await occurs between these two sends on the current-thread runtime,
    // so both terminal inputs are present at one coordinator scheduling point.
    let abort = fixture
        .service
        .linearize_abort_for_test(&live.id)
        .expect("abort operation");
    fixture.service.enqueue_end_for_test(&live.id);
    assert_eq!(
        abort.await.expect("abort outcome"),
        AbortMeetingOutcome::RefusedFinalizing
    );
    let finalizing = wait_for_record(&fixture.db, |record| {
        record.id == live.id && record.status == "finalizing"
    })
    .await;
    assert_eq!(finalizing.end_source, "push");
    assert_eq!(client.leave_calls.load(Ordering::Acquire), 0);
    wait_for_record(&fixture.db, |record| {
        record.id == live.id && record.status == "finalized"
    })
    .await;
}

#[tokio::test]
async fn ended_push_uses_join_identity_and_drives_the_second_end_path() {
    let fixture = Fixture::new(65_536);
    start_test_ingestion(&fixture.service, Duration::from_millis(50));
    let account_id = "ended-push-account";
    let feishu_meeting_id = "9007199254740993555";
    let client = Arc::new(FakeMeetingClient::successful(
        feishu_meeting_id,
        "ou_bot_ended_push",
    ));
    fixture.service.register_client(account_id, client);
    fixture
        .service
        .on_meeting_invited(synthetic_invite(account_id, "evt-ended-push"));
    let live = wait_for_record(&fixture.db, |record| record.status == "live").await;

    fixture
        .service
        .on_meeting_ended(account_id, feishu_meeting_id);
    let finalized = wait_for_record(&fixture.db, |record| {
        record.id == live.id && record.status == "finalized"
    })
    .await;
    assert_eq!(finalized.end_source, "push");
}

#[tokio::test]
async fn hanging_join_is_cancelled_by_abort_deadline_and_registry_replacement() {
    let fixture = Fixture::new(65_536);
    start_test_ingestion(&fixture.service, Duration::from_millis(50));

    let abort_account = "hanging-join-abort-account";
    let abort_client = Arc::new(FakeMeetingClient::successful(
        "9007199254740993666",
        "ou_bot_hanging_abort",
    ));
    abort_client.hang_join.store(true, Ordering::Release);
    fixture
        .service
        .register_client(abort_account, abort_client.clone());
    fixture
        .service
        .on_meeting_invited(synthetic_invite(abort_account, "evt-hanging-abort"));
    let joining = wait_for_record(&fixture.db, |record| {
        record.account_id == abort_account && record.status == "joining"
    })
    .await;
    wait_for_counter(&abort_client.join_calls, 1).await;
    assert_eq!(
        tokio::time::timeout(
            Duration::from_millis(100),
            fixture.service.abort_meeting(&joining.id),
        )
        .await
        .expect("admin abort cancels hanging join"),
        AbortMeetingOutcome::Aborting
    );
    wait_for_record(&fixture.db, |record| {
        record.id == joining.id && record.status == "aborted"
    })
    .await;
    assert_eq!(abort_client.leave_calls.load(Ordering::Acquire), 0);

    let paced_account = "hanging-join-paced-nudge-account";
    let paced = Arc::new(FakeMeetingClient::successful(
        "9007199254740993667",
        "ou_bot_hanging_paced",
    ));
    paced.hang_join.store(true, Ordering::Release);
    fixture
        .service
        .register_client(paced_account, paced.clone());
    fixture
        .service
        .on_meeting_invited(synthetic_invite(paced_account, "evt-paced-initial"));
    let paced_joining = wait_for_record(&fixture.db, |record| {
        record.account_id == paced_account && record.status == "joining"
    })
    .await;
    wait_for_counter(&paced.join_calls, 1).await;
    for index in 0..8 {
        fixture.service.on_meeting_invited(synthetic_invite(
            paced_account,
            &format!("evt-paced-redelivery-{index}"),
        ));
    }
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if fixture
                .db
                .count_meeting_invite_keys(&paced_joining.id)
                .expect("paced invite keys")
                == 9
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("all paced nudges admitted");
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        paced.join_calls.load(Ordering::Acquire),
        1,
        "same-generation admission nudges must not cancel or hot-loop join"
    );
    assert_eq!(
        fixture.service.abort_meeting(&paced_joining.id).await,
        AbortMeetingOutcome::Aborting
    );

    let replacement_account = "hanging-join-replacement-account";
    let stale = Arc::new(FakeMeetingClient::successful(
        "9007199254740993777",
        "ou_bot_stale",
    ));
    stale.hang_join.store(true, Ordering::Release);
    fixture
        .service
        .register_client(replacement_account, stale.clone());
    fixture.service.on_meeting_invited(synthetic_invite(
        replacement_account,
        "evt-hanging-replacement",
    ));
    wait_for_counter(&stale.join_calls, 1).await;
    let replacement = Arc::new(FakeMeetingClient::successful(
        "9007199254740993888",
        "ou_bot_replacement",
    ));
    fixture
        .service
        .register_client(replacement_account, replacement.clone());
    let live = wait_for_record(&fixture.db, |record| {
        record.account_id == replacement_account && record.status == "live"
    })
    .await;
    assert_eq!(live.feishu_meeting_id, "9007199254740993888");
    assert_eq!(replacement.join_calls.load(Ordering::Acquire), 1);

    let deadline_account = "hanging-join-deadline-account";
    let deadline_client = Arc::new(FakeMeetingClient::successful(
        "9007199254740993999",
        "ou_bot_hanging_deadline",
    ));
    deadline_client.hang_join.store(true, Ordering::Release);
    fixture
        .service
        .register_client(deadline_account, deadline_client.clone());
    fixture
        .service
        .on_meeting_invited(synthetic_invite(deadline_account, "evt-hanging-deadline"));
    wait_for_counter(&deadline_client.join_calls, 1).await;
    let aborted = wait_for_record(&fixture.db, |record| {
        record.account_id == deadline_account && record.status == "aborted"
    })
    .await;
    assert_eq!(aborted.status_detail, "join deadline exceeded");
}

#[tokio::test]
async fn platform_identity_error_converges_join_and_not_in_meeting_aborts_calls() {
    let fixture = Fixture::new(65_536);
    start_test_ingestion(&fixture.service, Duration::from_millis(50));

    let identity_account = "identity-error-account";
    let identity_client = Arc::new(FakeMeetingClient::failing(
        MeetingApiError::Other {
            code: 20002,
            message: "synthetic already joined".to_owned(),
            meeting_id: Some("9007199254740994001".to_owned()),
        },
        "ou_bot_identity",
    ));
    fixture
        .service
        .register_client(identity_account, identity_client);
    fixture
        .service
        .on_meeting_invited(synthetic_invite(identity_account, "evt-identity-success"));
    let live = wait_for_record(&fixture.db, |record| {
        record.account_id == identity_account && record.status == "live"
    })
    .await;
    assert_eq!(live.feishu_meeting_id, "9007199254740994001");

    let join_10005_account = "join-10005-account";
    let join_10005 = Arc::new(FakeMeetingClient::failing(
        MeetingApiError::NotInMeeting,
        "ou_bot_join_10005",
    ));
    fixture
        .service
        .register_client(join_10005_account, join_10005);
    fixture
        .service
        .on_meeting_invited(synthetic_invite(join_10005_account, "evt-join-10005"));
    let aborted = wait_for_record(&fixture.db, |record| {
        record.account_id == join_10005_account && record.status == "aborted"
    })
    .await;
    assert_eq!(aborted.status_detail, "platform reports bot not in meeting");

    let leave_10005_account = "leave-10005-account";
    let mut leave_10005_client =
        FakeMeetingClient::successful("9007199254740994002", "ou_bot_leave_10005");
    leave_10005_client.leave_result = Err(MeetingApiError::NotInMeeting);
    let leave_10005_client = Arc::new(leave_10005_client);
    fixture
        .service
        .register_client(leave_10005_account, leave_10005_client.clone());
    fixture
        .service
        .on_meeting_invited(synthetic_invite(leave_10005_account, "evt-leave-10005"));
    let live = wait_for_record(&fixture.db, |record| {
        record.account_id == leave_10005_account && record.status == "live"
    })
    .await;
    assert_eq!(
        fixture.service.abort_meeting(&live.id).await,
        AbortMeetingOutcome::Aborting
    );
    wait_for_record(&fixture.db, |record| {
        record.id == live.id && record.status == "aborted"
    })
    .await;
    assert_eq!(leave_10005_client.leave_calls.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn abort_domain_answers_duplicates_before_one_bounded_leave_finishes() {
    let fixture = Fixture::new(65_536);
    start_test_ingestion(&fixture.service, Duration::from_millis(80));
    let account_id = "abort-domain-account";
    let client = Arc::new(FakeMeetingClient::successful(
        "9007199254740993333",
        "ou_bot_abort",
    ));
    client.hang_leave.store(true, Ordering::Release);
    fixture.service.register_client(account_id, client.clone());
    fixture
        .service
        .on_meeting_invited(synthetic_invite(account_id, "evt-abort-domain"));
    let live = wait_for_record(&fixture.db, |record| record.status == "live").await;

    let started = Instant::now();
    let first = tokio::time::timeout(
        Duration::from_millis(150),
        fixture.service.abort_meeting(&live.id),
    )
    .await
    .expect("abort response precedes leave timeout");
    assert_eq!(first, AbortMeetingOutcome::Aborting);
    assert!(started.elapsed() < Duration::from_millis(300));

    let mut duplicates = JoinSet::new();
    for _ in 0..8 {
        let service = fixture.service.clone();
        let id = live.id.clone();
        duplicates.spawn(async move { service.abort_meeting(&id).await });
    }
    while let Some(outcome) = duplicates.join_next().await {
        assert_eq!(
            outcome.expect("duplicate task"),
            AbortMeetingOutcome::Aborting
        );
    }
    wait_for_record(&fixture.db, |record| {
        record.id == live.id && record.status == "aborted"
    })
    .await;
    assert_eq!(client.leave_calls.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn registry_replacement_resets_failure_and_no_client_deadline_recovers_or_aborts() {
    let fixture = Fixture::new(65_536);
    start_test_ingestion(&fixture.service, Duration::from_millis(60));

    let boot_gap = create_meeting(&fixture.db, MeetingStatus::Live, "Boot gap", "");
    fixture
        .db
        .record_meeting_failure(&boot_gap.id, "auth")
        .expect("persist pre-register failure");
    fixture.service.register_client(
        &boot_gap.account_id,
        Arc::new(FakeMeetingClient::successful(
            "9007199254740993443",
            "ou_bot_boot_gap",
        )),
    );
    let reset_after_boot_gap = fixture
        .db
        .get_meeting(&boot_gap.id)
        .expect("record")
        .expect("boot-gap meeting");
    assert_eq!(reset_after_boot_gap.failure_kind, "");
    assert!(reset_after_boot_gap.failure_since.is_none());

    let account_id = "registry-account";
    let failing = Arc::new(FakeMeetingClient::failing(
        MeetingApiError::RetriableTransport("synthetic transport".to_owned()),
        "ou_bot_registry",
    ));
    fixture.service.register_client(account_id, failing.clone());
    fixture
        .service
        .on_meeting_invited(synthetic_invite(account_id, "evt-registry"));
    let failed = wait_for_record(&fixture.db, |record| {
        record.account_id == account_id && record.failure_kind == "transport"
    })
    .await;
    fixture.service.unregister_client(account_id);
    let no_client = fixture
        .db
        .get_meeting(&failed.id)
        .expect("record")
        .expect("meeting");
    assert_eq!(fixture.service.stalled_reason(&no_client), "no_client");
    assert_eq!(no_client.failure_kind, "");
    assert!(no_client.failure_since.is_none());

    let replacement = Arc::new(FakeMeetingClient::successful(
        "9007199254740993444",
        "ou_bot_registry",
    ));
    fixture
        .service
        .register_client(account_id, replacement.clone());
    let live = wait_for_record(&fixture.db, |record| {
        record.id == failed.id && record.status == "live"
    })
    .await;
    assert_eq!(live.failure_kind, "");
    assert!(failing.join_calls.load(Ordering::Acquire) >= 1);
    assert_eq!(replacement.join_calls.load(Ordering::Acquire), 1);

    let no_client_account = "deadline-no-client-account";
    fixture.service.on_meeting_invited(synthetic_invite(
        no_client_account,
        "evt-no-client-deadline",
    ));
    let aborted = wait_for_record(&fixture.db, |record| {
        record.account_id == no_client_account && record.status == "aborted"
    })
    .await;
    assert_eq!(aborted.status_detail, "join deadline exceeded");
    assert!(aborted.feishu_meeting_id.is_empty());
}

#[tokio::test]
async fn boot_resumes_every_persisted_intent_stage_without_remote_leave_replay() {
    let fixture = Fixture::new(65_536);
    let joining = create_meeting(&fixture.db, MeetingStatus::Joining, "Boot joining", "");
    let aborting = create_meeting(
        &fixture.db,
        MeetingStatus::Aborting,
        "Boot aborting",
        "boot abort",
    );
    let finalizing_live = create_meeting(&fixture.db, MeetingStatus::Live, "Boot finalizing", "");
    let now = chrono::Utc::now();
    fixture
        .db
        .begin_meeting_finalizing(
            &finalizing_live.id,
            "push",
            &now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            &(now - chrono::Duration::milliseconds(1))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        )
        .expect("persist finalizing")
        .expect("live CAS");

    let recovered = Arc::new(
        MeetingService::new(
            fixture.db.clone(),
            fixture.service.root().to_path_buf(),
            65_536,
        )
        .expect("recovered service"),
    );
    start_test_ingestion(&recovered, Duration::from_millis(50));
    for id in [&joining.id, &aborting.id] {
        wait_for_record(&fixture.db, |record| {
            record.id == *id && record.status == "aborted"
        })
        .await;
    }
    wait_for_record(&fixture.db, |record| {
        record.id == finalizing_live.id && record.status == "finalized"
    })
    .await;
    assert!(index::index_path(&recovered.entity_dir(&joining.id)).exists());
    assert!(index::index_path(&recovered.entity_dir(&aborting.id)).exists());
    assert!(index::index_path(&recovered.entity_dir(&finalizing_live.id)).exists());
}

#[test]
fn activity_normalization_rejects_f64_ids_and_timestamps() {
    let error = ingest::normalize_activity(
        json!({
            "meeting_activity_items": [{
                "activity_event_type": "transcript_received",
                "meeting": { "id": "meeting" },
                "transcript_received_items": [{
                    "text": "float precision is forbidden",
                    "language": "en",
                    "sentence_id": 9007199254740992.0,
                    "start_time_ms": 1000.5,
                    "end_time_ms": 2000,
                    "speaker": { "id": { "open_id": "ou_test" } }
                }]
            }]
        }),
        None,
    )
    .expect_err("floating point identifiers and timestamps must be rejected");
    assert!(error.to_string().contains("sentence_id") || error.to_string().contains("integer"));
}
