use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use garyx_channels::{
    JoinedMeeting, MeetingApiError, MeetingEventSink, MeetingInvite, MeetingPlatformClient,
};
use serde_json::{Map, Value};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use super::{MeetingError, MeetingService, SegmentDraft, SegmentKind, log};
use crate::garyx_db::{
    MeetingAdmissionDraft, MeetingAdmissionOutcome, MeetingRecord, MeetingStatus,
    normalize_meeting_id,
};

const INGRESS_CAPACITY: usize = 256;
const COORDINATOR_CAPACITY: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbortMeetingOutcome {
    Aborting,
    RefusedFinalizing,
    Deleted,
    Retryable(String),
}

#[derive(Clone)]
struct RegisteredClient {
    client: Arc<dyn MeetingPlatformClient>,
    generation: u64,
}

#[derive(Clone)]
struct CoordinatorHandle {
    tx: mpsc::Sender<CoordinatorCommand>,
}

enum IngressCommand {
    Invite(MeetingInvite),
    Activity {
        account_id: String,
        event_id: String,
        payload: Value,
    },
    Ended {
        account_id: String,
        feishu_meeting_id: String,
    },
    NudgeAccount(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndSignalSource {
    Push,
    ParticipantLeft,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinalizeIntentOutcome {
    EndWon,
    AbortWon,
    Deleted,
    Shutdown,
}

impl EndSignalSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Push => "push",
            Self::ParticipantLeft => "participant_left",
        }
    }
}

enum CoordinatorCommand {
    Nudge,
    EndedSignal(EndSignalSource),
    AbortRequest,
    ActivityBatch {
        event_id: String,
        payload: Value,
        bot_open_id: Option<String>,
        received_at: Instant,
    },
    Shutdown,
}

enum JoinAttemptOutcome {
    Completed(Result<JoinedMeeting, MeetingApiError>),
    TimedOut,
    RegistryChanged,
}

struct AbortOperation {
    completed: Option<AbortMeetingOutcome>,
    waiters: Vec<oneshot::Sender<AbortMeetingOutcome>>,
}

enum AbortAdmission {
    Immediate(AbortMeetingOutcome),
    Wait(oneshot::Receiver<AbortMeetingOutcome>),
}

#[derive(Debug, Clone)]
struct MeetingTiming {
    join_retry_window: Duration,
    join_slot: Duration,
    platform_timeout: Duration,
    finalizing_grace: Duration,
    no_client_check: Duration,
    barrier_retry: Duration,
}

impl Default for MeetingTiming {
    fn default() -> Self {
        Self {
            join_retry_window: Duration::from_secs(300),
            join_slot: Duration::from_secs(20),
            platform_timeout: Duration::from_secs(20),
            // Push model: waiting costs nothing, so cover the platform's full
            // 5-minute post-end window plus delivery slack (owner-picked
            // 5.5 min). (4 min was a pull-era value where the last fetch
            // needed headroom to finish inside the window — inverted
            // semantics under push.)
            finalizing_grace: Duration::from_secs(330),
            no_client_check: Duration::from_secs(60),
            barrier_retry: Duration::from_secs(1),
        }
    }
}

pub(crate) struct IngestionState {
    service: OnceLock<Weak<MeetingService>>,
    runtime: OnceLock<tokio::runtime::Handle>,
    ingress: OnceLock<mpsc::Sender<IngressCommand>>,
    registry: Mutex<HashMap<String, RegisteredClient>>,
    registry_generation: AtomicU64,
    coordinators: Mutex<HashMap<String, CoordinatorHandle>>,
    abort_domains: Mutex<HashMap<String, AbortOperation>>,
    timing: Mutex<MeetingTiming>,
    shutdown: CancellationToken,
}

impl IngestionState {
    pub(super) fn new() -> Self {
        Self {
            service: OnceLock::new(),
            runtime: OnceLock::new(),
            ingress: OnceLock::new(),
            registry: Mutex::new(HashMap::new()),
            registry_generation: AtomicU64::new(0),
            coordinators: Mutex::new(HashMap::new()),
            abort_domains: Mutex::new(HashMap::new()),
            timing: Mutex::new(MeetingTiming::default()),
            shutdown: CancellationToken::new(),
        }
    }

    fn timing(&self) -> MeetingTiming {
        self.timing
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn current_client(&self, account_id: &str) -> Option<RegisteredClient> {
        self.registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(account_id)
            .cloned()
    }

    fn enqueue_ingress(&self, command: IngressCommand, kind: &'static str) {
        let Some(tx) = self.ingress.get() else {
            error!(
                kind,
                "meeting ingestion is not started; dropping pushed event"
            );
            return;
        };
        if let Err(send_error) = tx.try_send(command) {
            error!(kind, error = %send_error, "meeting ingress queue rejected pushed event");
        }
    }
}

impl MeetingService {
    pub fn start_ingestion(self: &Arc<Self>, join_retry_window_secs: u64) {
        self.ingestion
            .timing
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .join_retry_window = Duration::from_secs(join_retry_window_secs.max(1));
        let _ = self.ingestion.service.set(Arc::downgrade(self));
        let runtime = match tokio::runtime::Handle::try_current() {
            Ok(runtime) => runtime,
            Err(error) => {
                warn!(error = %error, "meeting ingestion start requires a Tokio runtime");
                return;
            }
        };
        let _ = self.ingestion.runtime.set(runtime.clone());
        if self.ingestion.ingress.get().is_none() {
            let (tx, rx) = mpsc::channel(INGRESS_CAPACITY);
            if self.ingestion.ingress.set(tx).is_ok() {
                let service = Arc::downgrade(self);
                runtime.spawn(async move { run_ingress(service, rx).await });
            }
        }

        match self.db.list_non_terminal_meetings() {
            Ok(records) => {
                for record in records {
                    self.ensure_coordinator(&record.id);
                }
            }
            Err(error) => error!(error = %error, "failed to resume meeting coordinators at boot"),
        }
    }

    pub fn shutdown_ingestion(&self) {
        self.ingestion.shutdown.cancel();
        let handles = self
            .ingestion
            .coordinators
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for handle in handles {
            let _ = handle.tx.try_send(CoordinatorCommand::Shutdown);
        }
    }

    pub async fn abort_meeting(self: &Arc<Self>, id: &str) -> AbortMeetingOutcome {
        let id = match normalize_meeting_id(id) {
            Ok(id) => id,
            Err(error) => return AbortMeetingOutcome::Retryable(error.to_string()),
        };
        let service = self.clone();
        let admission = tokio::task::spawn_blocking(move || service.linearize_abort(&id)).await;
        let admission = match admission {
            Ok(admission) => admission,
            Err(error) => {
                return AbortMeetingOutcome::Retryable(format!(
                    "meeting abort admission task failed: {error}"
                ));
            }
        };
        match admission {
            AbortAdmission::Immediate(outcome) => outcome,
            AbortAdmission::Wait(receiver) => receiver.await.unwrap_or_else(|_| {
                AbortMeetingOutcome::Retryable(
                    "meeting abort coordinator stopped before publishing a result".to_owned(),
                )
            }),
        }
    }

    fn linearize_abort(self: &Arc<Self>, id: &str) -> AbortAdmission {
        let mut domains = self
            .ingestion
            .abort_domains
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = match self.db.get_meeting(id) {
            Ok(Some(record)) => record,
            Ok(None) => return AbortAdmission::Immediate(AbortMeetingOutcome::Deleted),
            Err(error) => {
                return AbortAdmission::Immediate(AbortMeetingOutcome::Retryable(
                    error.to_string(),
                ));
            }
        };
        let status = match record.parsed_status() {
            Ok(status) => status,
            Err(error) => {
                return AbortAdmission::Immediate(AbortMeetingOutcome::Retryable(
                    error.to_string(),
                ));
            }
        };
        match status {
            MeetingStatus::Aborting | MeetingStatus::Aborted => {
                return AbortAdmission::Immediate(AbortMeetingOutcome::Aborting);
            }
            MeetingStatus::Finalizing | MeetingStatus::Finalized => {
                return AbortAdmission::Immediate(AbortMeetingOutcome::RefusedFinalizing);
            }
            MeetingStatus::Joining | MeetingStatus::Live => {}
        }

        if let Some(operation) = domains.get_mut(id) {
            if let Some(outcome) = &operation.completed {
                if !matches!(outcome, AbortMeetingOutcome::Retryable(_)) {
                    return AbortAdmission::Immediate(outcome.clone());
                }
            } else {
                let (sender, receiver) = oneshot::channel();
                operation.waiters.push(sender);
                return AbortAdmission::Wait(receiver);
            }
        }
        // A retryable completed operation has no durable intent behind it;
        // discard it so this request may create one fresh command.
        domains.remove(id);

        self.ensure_coordinator(id);
        let handle = self
            .ingestion
            .coordinators
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(id)
            .cloned();
        let Some(handle) = handle else {
            return AbortAdmission::Immediate(AbortMeetingOutcome::Retryable(
                "meeting coordinator is unavailable".to_owned(),
            ));
        };
        let (sender, receiver) = oneshot::channel();
        // The domain lock stays held while the operation is installed and its
        // sole command is enqueued. Completion takes the same lock, so the
        // coordinator cannot publish into an invisible or orphaned operation.
        domains.insert(
            id.to_owned(),
            AbortOperation {
                completed: None,
                waiters: vec![sender],
            },
        );
        if let Err(send_error) = handle.tx.try_send(CoordinatorCommand::AbortRequest) {
            let operation = domains.remove(id).expect("just-inserted abort operation");
            let outcome = AbortMeetingOutcome::Retryable(format!(
                "meeting abort enqueue failed: {send_error}"
            ));
            for waiter in operation.waiters {
                let _ = waiter.send(outcome.clone());
            }
            return AbortAdmission::Immediate(outcome);
        }
        AbortAdmission::Wait(receiver)
    }

    fn publish_abort(&self, id: &str, outcome: AbortMeetingOutcome) {
        let mut domains = self
            .ingestion
            .abort_domains
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(operation) = domains.get_mut(id) else {
            debug!(
                meeting_id = id,
                "automatic meeting abort has no HTTP abort operation to publish"
            );
            return;
        };
        operation.completed = Some(outcome.clone());
        for waiter in operation.waiters.drain(..) {
            let _ = waiter.send(outcome.clone());
        }
        // The durable status is now the handler fast path. Removing the
        // completed operation bounds this per-entity domain without opening a
        // second-command window: retries cannot acquire the domain lock until
        // after removal, and then observe the CAS result in SQLite.
        domains.remove(id);
    }

    fn ensure_coordinator(self: &Arc<Self>, id: &str) {
        let id = match normalize_meeting_id(id) {
            Ok(id) => id,
            Err(error) => {
                warn!(error = %error, "refusing to start coordinator for invalid meeting id");
                return;
            }
        };
        let mut coordinators = self
            .ingestion
            .coordinators
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if coordinators
            .get(&id)
            .is_some_and(|handle| !handle.tx.is_closed())
        {
            return;
        }
        let Some(runtime) = self.ingestion.runtime.get().cloned() else {
            warn!(meeting_id = %id, "meeting ingestion runtime is unavailable");
            return;
        };
        let (tx, rx) = mpsc::channel(COORDINATOR_CAPACITY);
        coordinators.insert(id.clone(), CoordinatorHandle { tx });
        let service = self.clone();
        runtime.spawn(async move { run_coordinator(service, id, rx).await });
    }

    fn nudge_account(self: &Arc<Self>, account_id: &str) {
        match self.db.list_non_terminal_meetings_for_account(account_id) {
            Ok(records) => {
                for record in records {
                    self.ensure_coordinator(&record.id);
                    let handle = self
                        .ingestion
                        .coordinators
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .get(&record.id)
                        .cloned();
                    if let Some(handle) = handle
                        && let Err(error) = handle.tx.try_send(CoordinatorCommand::Nudge)
                    {
                        warn!(meeting_id = %record.id, error = %error, "meeting register nudge was not queued");
                    }
                }
            }
            Err(error) => warn!(account_id, error = %error, "failed to nudge account meetings"),
        }
    }

    pub(super) fn stalled_reason(&self, record: &MeetingRecord) -> String {
        let Ok(status) = record.parsed_status() else {
            return String::new();
        };
        if !matches!(
            status,
            MeetingStatus::Joining | MeetingStatus::Live | MeetingStatus::Finalizing
        ) {
            return String::new();
        }
        if self.ingestion.current_client(&record.account_id).is_none() {
            return "no_client".to_owned();
        }
        let Some(since) = record.failure_since.as_deref() else {
            return String::new();
        };
        let Ok(since) = DateTime::parse_from_rfc3339(since) else {
            return String::new();
        };
        if Utc::now().signed_duration_since(since.with_timezone(&Utc))
            > chrono::Duration::minutes(15)
        {
            match record.failure_kind.as_str() {
                "auth" => "auth_failed".to_owned(),
                "transport" => "transport".to_owned(),
                _ => String::new(),
            }
        } else {
            String::new()
        }
    }

    #[cfg(test)]
    pub(super) fn set_test_ingestion_timing(
        &self,
        join_slot: Duration,
        platform_timeout: Duration,
        finalizing_grace: Duration,
        no_client_check: Duration,
    ) {
        let mut timing = self
            .ingestion
            .timing
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        timing.join_retry_window = Duration::from_millis(200);
        timing.join_slot = join_slot;
        timing.platform_timeout = platform_timeout;
        timing.finalizing_grace = finalizing_grace;
        timing.no_client_check = no_client_check;
        timing.barrier_retry = Duration::from_millis(10);
    }

    #[cfg(test)]
    pub(super) fn linearize_abort_for_test(
        self: &Arc<Self>,
        id: &str,
    ) -> Result<oneshot::Receiver<AbortMeetingOutcome>, AbortMeetingOutcome> {
        match self.linearize_abort(id) {
            AbortAdmission::Immediate(outcome) => Err(outcome),
            AbortAdmission::Wait(receiver) => Ok(receiver),
        }
    }

    #[cfg(test)]
    pub(super) fn enqueue_end_for_test(&self, id: &str) {
        let handle = self
            .ingestion
            .coordinators
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(id)
            .cloned()
            .expect("test meeting coordinator");
        handle
            .tx
            .try_send(CoordinatorCommand::EndedSignal(EndSignalSource::Push))
            .expect("test end signal enqueue");
    }
}

impl MeetingEventSink for MeetingService {
    fn register_client(&self, account_id: &str, client: Arc<dyn MeetingPlatformClient>) {
        let account_id = account_id.trim();
        if account_id.is_empty() {
            warn!("refusing to register an empty meeting account id");
            return;
        }
        let generation = self
            .ingestion
            .registry_generation
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        let was_missing = self
            .ingestion
            .registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                account_id.to_owned(),
                RegisteredClient { client, generation },
            )
            .is_none();
        if was_missing && let Err(error) = self.db.clear_meeting_failures_for_account(account_id) {
            warn!(account_id, error = %error, "failed to reset meeting failure clock on register");
        }
        self.ingestion.enqueue_ingress(
            IngressCommand::NudgeAccount(account_id.to_owned()),
            "register_client",
        );
    }

    fn unregister_client(&self, account_id: &str) {
        let account_id = account_id.trim();
        self.ingestion
            .registry
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(account_id);
        if let Err(error) = self.db.clear_meeting_failures_for_account(account_id) {
            warn!(account_id, error = %error, "failed to reset meeting failure clock on unregister");
        }
        self.ingestion.enqueue_ingress(
            IngressCommand::NudgeAccount(account_id.to_owned()),
            "unregister_client",
        );
    }

    fn on_meeting_invited(&self, invite: MeetingInvite) {
        self.ingestion
            .enqueue_ingress(IngressCommand::Invite(invite), "invited");
    }

    fn on_meeting_activity(&self, account_id: &str, event_id: &str, payload: Value) {
        self.ingestion.enqueue_ingress(
            IngressCommand::Activity {
                account_id: account_id.to_owned(),
                event_id: event_id.to_owned(),
                payload,
            },
            "activity",
        );
    }

    fn on_meeting_ended(&self, account_id: &str, feishu_meeting_id: &str) {
        self.ingestion.enqueue_ingress(
            IngressCommand::Ended {
                account_id: account_id.to_owned(),
                feishu_meeting_id: feishu_meeting_id.to_owned(),
            },
            "ended",
        );
    }
}

async fn run_ingress(service: Weak<MeetingService>, mut rx: mpsc::Receiver<IngressCommand>) {
    while let Some(command) = rx.recv().await {
        let Some(service) = service.upgrade() else {
            return;
        };
        if service.ingestion.shutdown.is_cancelled() {
            return;
        }
        match command {
            IngressCommand::Invite(invite) => admit_invite(&service, invite).await,
            IngressCommand::Activity {
                account_id,
                event_id,
                payload,
            } => route_activity(&service, &account_id, &event_id, payload),
            IngressCommand::Ended {
                account_id,
                feishu_meeting_id,
            } => route_ended(&service, &account_id, &feishu_meeting_id),
            IngressCommand::NudgeAccount(account_id) => service.nudge_account(&account_id),
        }
    }
}

async fn admit_invite(service: &Arc<MeetingService>, invite: MeetingInvite) {
    let timing = service.ingestion.timing();
    let observed = Utc::now();
    let observed_at = timestamp(observed);
    let deadline = timestamp(
        observed
            + chrono::Duration::from_std(timing.join_retry_window)
                .unwrap_or_else(|_| chrono::Duration::seconds(300)),
    );
    let draft = MeetingAdmissionDraft {
        account_id: invite.account_id.clone(),
        meeting_no: invite.meeting_no,
        invite_event_id: invite.event_id,
        topic: invite.topic,
        invited_by: invite.inviter_id,
        join_deadline_at: deadline,
        observed_at,
    };
    let mut last_error = None;
    for (attempt, delay) in [
        Duration::ZERO,
        Duration::from_millis(100),
        Duration::from_secs(1),
    ]
    .into_iter()
    .enumerate()
    {
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        match service.db.admit_meeting_invite(draft.clone()) {
            Ok(MeetingAdmissionOutcome::Created(record)) => {
                info!(meeting_id = %record.id, account_id = %record.account_id, "admitted Feishu meeting invitation");
                service.ensure_coordinator(&record.id);
                return;
            }
            Ok(MeetingAdmissionOutcome::Existing(record)) => {
                debug!(meeting_id = %record.id, "meeting invitation delivery already admitted");
                service.ensure_coordinator(&record.id);
                if let Some(handle) = service
                    .ingestion
                    .coordinators
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(&record.id)
                    .cloned()
                {
                    let _ = handle.tx.try_send(CoordinatorCommand::Nudge);
                }
                return;
            }
            Err(error) => {
                warn!(attempt = attempt + 1, error = %error, "meeting admission transaction failed");
                last_error = Some(error);
            }
        }
    }
    error!(
        account_id = %invite.account_id,
        error = %last_error.map(|error| error.to_string()).unwrap_or_default(),
        "meeting admission retries exhausted; durable invitation fact was not stored"
    );
}

fn route_activity(service: &Arc<MeetingService>, account_id: &str, event_id: &str, payload: Value) {
    if event_id.is_empty() || event_id.len() > 256 {
        warn!(
            account_id,
            "dropping meeting activity with invalid event_id bound"
        );
        return;
    }
    let Some(items) = payload
        .get("meeting_activity_items")
        .and_then(Value::as_array)
    else {
        warn!(
            account_id,
            event_id, "meeting activity payload has no item array"
        );
        return;
    };
    let mut grouped = HashMap::<String, Vec<Value>>::new();
    for item in items {
        let Some(meeting_id) = item
            .get("meeting")
            .and_then(|meeting| meeting.get("id"))
            .and_then(json_exact_string_or_integer)
        else {
            warn!(
                account_id,
                event_id, "meeting activity item has no exact meeting id"
            );
            continue;
        };
        grouped.entry(meeting_id).or_default().push(item.clone());
    }
    for (meeting_id, items) in grouped {
        let record = match service
            .db
            .get_active_meeting_by_feishu_id(account_id, &meeting_id)
        {
            Ok(Some(record)) => record,
            Ok(None) => {
                debug!(account_id, event_id, feishu_meeting_id = %meeting_id, "dropping activity for unknown or terminal meeting");
                continue;
            }
            Err(error) => {
                warn!(account_id, event_id, error = %error, "failed to route meeting activity");
                continue;
            }
        };
        service.ensure_coordinator(&record.id);
        let handle = service
            .ingestion
            .coordinators
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&record.id)
            .cloned();
        let bot_open_id = service
            .ingestion
            .current_client(account_id)
            .and_then(|registered| registered.client.bot_open_id());
        let command = CoordinatorCommand::ActivityBatch {
            event_id: event_id.to_owned(),
            payload: serde_json::json!({ "meeting_activity_items": items }),
            bot_open_id,
            received_at: Instant::now(),
        };
        if let Some(handle) = handle
            && let Err(error) = handle.tx.try_send(command)
        {
            error!(meeting_id = %record.id, event_id, error = %error, "meeting activity queue is full or closed; batch is lost per push capture semantics");
        }
    }
}

fn route_ended(service: &Arc<MeetingService>, account_id: &str, feishu_meeting_id: &str) {
    let record = match service
        .db
        .get_active_meeting_by_feishu_id(account_id, feishu_meeting_id)
    {
        Ok(Some(record)) => record,
        Ok(None) => {
            debug!(
                account_id,
                feishu_meeting_id, "dropping end signal for unknown or terminal meeting"
            );
            return;
        }
        Err(error) => {
            warn!(account_id, feishu_meeting_id, error = %error, "failed to route meeting end signal");
            return;
        }
    };
    service.ensure_coordinator(&record.id);
    if let Some(handle) = service
        .ingestion
        .coordinators
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&record.id)
        .cloned()
        && let Err(error) = handle
            .tx
            .try_send(CoordinatorCommand::EndedSignal(EndSignalSource::Push))
    {
        error!(meeting_id = %record.id, error = %error, "meeting end signal queue is full or closed");
    }
}

async fn run_coordinator(
    service: Arc<MeetingService>,
    id: String,
    mut rx: mpsc::Receiver<CoordinatorCommand>,
) {
    loop {
        let record = match service.db.get_meeting(&id) {
            Ok(Some(record)) => record,
            Ok(None) => break,
            Err(error) => {
                warn!(meeting_id = %id, error = %error, "meeting coordinator failed to load entity");
                tokio::time::sleep(service.ingestion.timing().barrier_retry).await;
                continue;
            }
        };
        let status = match record.parsed_status() {
            Ok(status) => status,
            Err(error) => {
                error!(meeting_id = %id, error = %error, "meeting coordinator found invalid status");
                break;
            }
        };
        let keep_running = match status {
            MeetingStatus::Joining => run_joining(&service, &record, &mut rx).await,
            MeetingStatus::Live => run_live(&service, &record, &mut rx).await,
            MeetingStatus::Finalizing => run_finalizing(&service, &record, &mut rx).await,
            MeetingStatus::Aborting => {
                run_terminal_barrier(
                    &service,
                    &record.id,
                    MeetingStatus::Aborting,
                    MeetingStatus::Aborted,
                )
                .await
            }
            MeetingStatus::Finalized | MeetingStatus::Aborted => false,
        };
        if !keep_running {
            break;
        }
    }
    service
        .ingestion
        .coordinators
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&id);
}

async fn run_joining(
    service: &Arc<MeetingService>,
    record: &MeetingRecord,
    rx: &mut mpsc::Receiver<CoordinatorCommand>,
) -> bool {
    let deadline = match parse_deadline(&record.join_deadline_at) {
        Ok(deadline) => deadline,
        Err(error) => {
            error!(meeting_id = %record.id, error = %error, "invalid persisted join deadline");
            return false;
        }
    };
    loop {
        if Instant::now() >= deadline {
            return begin_abort(service, record, "join deadline exceeded", None).await;
        }
        let timing = service.ingestion.timing();
        let Some(registered) = service.ingestion.current_client(&record.account_id) else {
            let wake = (Instant::now() + timing.no_client_check).min(deadline);
            tokio::select! {
                _ = service.ingestion.shutdown.cancelled() => return false,
                _ = tokio::time::sleep_until(wake) => continue,
                command = rx.recv() => {
                    if !handle_joining_command(service, record, command).await {
                        return false;
                    }
                    if !matches!(service.db.get_meeting(&record.id), Ok(Some(ref current)) if current.status == "joining") {
                        return true;
                    }
                    continue;
                }
            }
        };

        let attempt_started = Instant::now();
        debug!(meeting_id = %record.id, client_generation = registered.generation, "starting paced meeting join attempt");
        let remaining = deadline.saturating_duration_since(attempt_started);
        let timeout = timing.platform_timeout.min(remaining);
        let join = registered.client.join(&record.meeting_no, None);
        tokio::pin!(join);
        let attempt_timeout = tokio::time::sleep_until(attempt_started + timeout);
        tokio::pin!(attempt_timeout);
        let result = loop {
            tokio::select! {
                biased;
                _ = service.ingestion.shutdown.cancelled() => return false,
                _ = tokio::time::sleep_until(deadline) => {
                    return begin_abort(service, record, "join deadline exceeded", None).await;
                }
                command = rx.recv() => match command {
                    None | Some(CoordinatorCommand::Shutdown) => return false,
                    Some(CoordinatorCommand::AbortRequest) => {
                        let _ = begin_abort(service, record, "aborted by administrator", None).await;
                        return false;
                    }
                    Some(CoordinatorCommand::Nudge) => {
                        let generation = service
                            .ingestion
                            .current_client(&record.account_id)
                            .map(|current| current.generation);
                        if generation != Some(registered.generation) {
                            break JoinAttemptOutcome::RegistryChanged;
                        }
                    }
                    Some(CoordinatorCommand::EndedSignal(_)) => {}
                    Some(CoordinatorCommand::ActivityBatch { .. }) => {
                        debug!(meeting_id = %record.id, "dropping activity before join identity is known");
                    }
                },
                result = &mut join => break JoinAttemptOutcome::Completed(result),
                _ = &mut attempt_timeout => break JoinAttemptOutcome::TimedOut,
            }
        };
        match result {
            JoinAttemptOutcome::Completed(Ok(joined)) => {
                if service
                    .db
                    .mark_meeting_live(&record.id, &joined.feishu_meeting_id, &record.topic)
                    .is_ok_and(|updated| updated.is_some())
                {
                    info!(meeting_id = %record.id, feishu_meeting_id = %joined.feishu_meeting_id, "meeting bot joined");
                    return true;
                }
                return true;
            }
            JoinAttemptOutcome::Completed(Err(error)) => {
                if let Some(meeting_id) = error.meeting_id() {
                    if service
                        .db
                        .mark_meeting_live(&record.id, meeting_id, &record.topic)
                        .is_ok_and(|updated| updated.is_some())
                    {
                        info!(meeting_id = %record.id, feishu_meeting_id = meeting_id, "join error carried meeting identity; treated as success");
                    }
                    return true;
                }
                if matches!(error, MeetingApiError::NotInMeeting) {
                    return begin_abort(
                        service,
                        record,
                        "platform reports bot not in meeting",
                        None,
                    )
                    .await;
                }
                record_platform_failure(service, &record.id, &error);
                if matches!(error, MeetingApiError::Other { .. }) {
                    error!(meeting_id = %record.id, error = %error, "unclassified join error observed; meeting rollout must stop for a design amendment if this is duplicate-join behavior");
                }
            }
            JoinAttemptOutcome::TimedOut => {
                if let Err(error) = service.db.record_meeting_failure(&record.id, "transport") {
                    warn!(meeting_id = %record.id, error = %error, "failed to record join timeout");
                }
            }
            JoinAttemptOutcome::RegistryChanged => continue,
        }

        let slot_end = (attempt_started + timing.join_slot).min(deadline);
        while Instant::now() < slot_end {
            tokio::select! {
                biased;
                _ = service.ingestion.shutdown.cancelled() => return false,
                command = rx.recv() => match command {
                    None | Some(CoordinatorCommand::Shutdown) => return false,
                    Some(CoordinatorCommand::AbortRequest) => {
                        let _ = begin_abort(service, record, "aborted by administrator", None).await;
                        return false;
                    }
                    Some(CoordinatorCommand::Nudge) => {
                        let generation = service
                            .ingestion
                            .current_client(&record.account_id)
                            .map(|current| current.generation);
                        if generation != Some(registered.generation) {
                            return true;
                        }
                    }
                    Some(CoordinatorCommand::EndedSignal(_)) => {}
                    Some(CoordinatorCommand::ActivityBatch { .. }) => {
                        debug!(meeting_id = %record.id, "dropping activity before join identity is known");
                    }
                },
                _ = tokio::time::sleep_until(slot_end) => break,
            }
        }
    }
}

async fn handle_joining_command(
    service: &Arc<MeetingService>,
    record: &MeetingRecord,
    command: Option<CoordinatorCommand>,
) -> bool {
    match command {
        None | Some(CoordinatorCommand::Shutdown) => false,
        Some(CoordinatorCommand::AbortRequest) => {
            let _ = begin_abort(service, record, "aborted by administrator", None).await;
            false
        }
        Some(CoordinatorCommand::EndedSignal(_)) => true,
        Some(CoordinatorCommand::ActivityBatch { .. }) => {
            debug!(meeting_id = %record.id, "dropping activity before join identity is known");
            true
        }
        Some(CoordinatorCommand::Nudge) => true,
    }
}

async fn run_live(
    service: &Arc<MeetingService>,
    record: &MeetingRecord,
    rx: &mut mpsc::Receiver<CoordinatorCommand>,
) -> bool {
    let command = tokio::select! {
        _ = service.ingestion.shutdown.cancelled() => return false,
        command = rx.recv() => command,
    };
    let Some(first) = command else {
        return false;
    };
    let commands = drain_scheduling_point(first, rx);
    if commands
        .iter()
        .any(|command| matches!(command, CoordinatorCommand::Shutdown))
    {
        return false;
    }
    let source = commands
        .iter()
        .find_map(|command| match command {
            CoordinatorCommand::EndedSignal(source) => Some(*source),
            _ => None,
        })
        .or_else(|| {
            commands.iter().find_map(|command| match command {
                CoordinatorCommand::ActivityBatch {
                    payload,
                    bot_open_id,
                    ..
                } if activity_signals_own_participant_left(payload, bot_open_id.as_deref()) => {
                    Some(EndSignalSource::ParticipantLeft)
                }
                _ => None,
            })
        });
    if let Some(source) = source {
        match begin_finalizing(service, record, source).await {
            FinalizeIntentOutcome::EndWon => {
                if commands
                    .iter()
                    .any(|command| matches!(command, CoordinatorCommand::AbortRequest))
                {
                    service.publish_abort(&record.id, AbortMeetingOutcome::RefusedFinalizing);
                }
                for command in commands {
                    if let CoordinatorCommand::ActivityBatch {
                        event_id,
                        payload,
                        bot_open_id,
                        ..
                    } = command
                    {
                        let _ =
                            commit_activity(service, record, &event_id, payload, bot_open_id).await;
                    }
                }
            }
            FinalizeIntentOutcome::AbortWon => {
                service.publish_abort(&record.id, AbortMeetingOutcome::Aborting)
            }
            FinalizeIntentOutcome::Deleted => {
                service.publish_abort(&record.id, AbortMeetingOutcome::Deleted);
                return false;
            }
            FinalizeIntentOutcome::Shutdown => return false,
        }
        return true;
    }
    if commands
        .iter()
        .any(|command| matches!(command, CoordinatorCommand::AbortRequest))
    {
        return begin_abort(
            service,
            record,
            "aborted by administrator",
            service.ingestion.current_client(&record.account_id),
        )
        .await;
    }
    // A Nudge on a live entity verifies membership by re-joining (idempotent
    // on the platform). This is the durable recovery for entities stuck live
    // after their end signal was consumed pre-crash (2026-07-17 incident:
    // kick -> CHECK-blocked finalize -> restart -> re-invite folded into the
    // active entity with no join). Registry registration after boot and
    // distinct re-invites both arrive here as Nudge.
    if commands
        .iter()
        .any(|command| matches!(command, CoordinatorCommand::Nudge))
        && let Some(registered) = service.ingestion.current_client(&record.account_id)
    {
        let timeout = service.ingestion.timing().platform_timeout;
        let join = tokio::time::timeout(timeout, registered.client.join(&record.meeting_no, None));
        match join.await {
            Ok(Ok(joined)) => {
                info!(
                    meeting_id = %record.id,
                    feishu_meeting_id = %joined.feishu_meeting_id,
                    "live nudge re-join verified membership"
                );
                let _ = service.db.mark_meeting_live(
                    &record.id,
                    &joined.feishu_meeting_id,
                    &record.topic,
                );
            }
            Ok(Err(error)) => {
                if let Some(meeting_id) = error.meeting_id() {
                    info!(
                        meeting_id = %record.id,
                        feishu_meeting_id = meeting_id,
                        "live nudge re-join carried identity; treated as verified"
                    );
                } else if matches!(error, MeetingApiError::NotInMeeting) {
                    return begin_abort(
                        service,
                        record,
                        "re-join verify: platform reports meeting unavailable",
                        service.ingestion.current_client(&record.account_id),
                    )
                    .await;
                } else {
                    warn!(meeting_id = %record.id, error = %error, "live nudge re-join verify failed; will retry on next nudge");
                }
            }
            Err(_) => {
                warn!(meeting_id = %record.id, "live nudge re-join verify timed out; will retry on next nudge");
            }
        }
    }
    for command in commands {
        if let CoordinatorCommand::ActivityBatch {
            event_id,
            payload,
            bot_open_id,
            ..
        } = command
            && commit_activity(service, record, &event_id, payload, bot_open_id).await
                == Some(EndSignalSource::ParticipantLeft)
        {
            return !matches!(
                begin_finalizing(service, record, EndSignalSource::ParticipantLeft).await,
                FinalizeIntentOutcome::Deleted | FinalizeIntentOutcome::Shutdown
            );
        }
    }
    true
}

async fn run_finalizing(
    service: &Arc<MeetingService>,
    record: &MeetingRecord,
    rx: &mut mpsc::Receiver<CoordinatorCommand>,
) -> bool {
    let Some(deadline_text) = record.grace_deadline_at.as_deref() else {
        error!(meeting_id = %record.id, "finalizing meeting has no grace deadline");
        return false;
    };
    let deadline = match parse_deadline(deadline_text) {
        Ok(deadline) => deadline,
        Err(error) => {
            error!(meeting_id = %record.id, error = %error, "invalid finalizing deadline");
            return false;
        }
    };
    loop {
        if Instant::now() >= deadline {
            while let Ok(command) = rx.try_recv() {
                match command {
                    CoordinatorCommand::ActivityBatch {
                        event_id,
                        payload,
                        bot_open_id,
                        received_at,
                    } => {
                        if received_at <= deadline {
                            let _ =
                                commit_activity(service, record, &event_id, payload, bot_open_id)
                                    .await;
                        } else {
                            debug!(meeting_id = %record.id, event_id, "dropping activity received after trailing-capture deadline");
                        }
                    }
                    CoordinatorCommand::AbortRequest => {
                        service.publish_abort(&record.id, AbortMeetingOutcome::RefusedFinalizing)
                    }
                    CoordinatorCommand::Shutdown => return false,
                    CoordinatorCommand::Nudge | CoordinatorCommand::EndedSignal(_) => {}
                }
            }
            return run_terminal_barrier(
                service,
                &record.id,
                MeetingStatus::Finalizing,
                MeetingStatus::Finalized,
            )
            .await;
        }
        tokio::select! {
            _ = service.ingestion.shutdown.cancelled() => return false,
            _ = tokio::time::sleep_until(deadline) => continue,
            command = rx.recv() => match command {
                None | Some(CoordinatorCommand::Shutdown) => return false,
                Some(CoordinatorCommand::AbortRequest) => service.publish_abort(&record.id, AbortMeetingOutcome::RefusedFinalizing),
                Some(CoordinatorCommand::ActivityBatch { event_id, payload, bot_open_id, received_at }) => {
                    if received_at <= deadline {
                        let _ = commit_activity(service, record, &event_id, payload, bot_open_id).await;
                    } else {
                        debug!(meeting_id = %record.id, event_id, "dropping activity received after trailing-capture deadline");
                    }
                }
                Some(CoordinatorCommand::Nudge | CoordinatorCommand::EndedSignal(_)) => {}
            }
        }
    }
}

fn drain_scheduling_point(
    first: CoordinatorCommand,
    rx: &mut mpsc::Receiver<CoordinatorCommand>,
) -> Vec<CoordinatorCommand> {
    let mut commands = vec![first];
    while let Ok(command) = rx.try_recv() {
        commands.push(command);
    }
    commands
}

async fn begin_finalizing(
    service: &Arc<MeetingService>,
    record: &MeetingRecord,
    source: EndSignalSource,
) -> FinalizeIntentOutcome {
    let now = Utc::now();
    let timing = service.ingestion.timing();
    let deadline = now
        + chrono::Duration::from_std(timing.finalizing_grace)
            .unwrap_or_else(|_| chrono::Duration::minutes(4));
    let ended_at = timestamp(now);
    let grace_deadline_at = timestamp(deadline);
    let mut attempt: u32 = 0;
    loop {
        match service.db.begin_meeting_finalizing(
            &record.id,
            source.as_str(),
            &ended_at,
            &grace_deadline_at,
        ) {
            Ok(Some(_)) => return FinalizeIntentOutcome::EndWon,
            Ok(None) => match service.db.get_meeting(&record.id) {
                Ok(None) => return FinalizeIntentOutcome::Deleted,
                Ok(Some(current)) => match current.parsed_status() {
                    Ok(MeetingStatus::Finalizing | MeetingStatus::Finalized) => {
                        return FinalizeIntentOutcome::EndWon;
                    }
                    Ok(MeetingStatus::Aborting | MeetingStatus::Aborted) => {
                        return FinalizeIntentOutcome::AbortWon;
                    }
                    Ok(MeetingStatus::Live) => {
                        warn!(meeting_id = %record.id, "finalizing CAS reported no row while meeting remained live; retrying");
                    }
                    Ok(MeetingStatus::Joining) => {
                        error!(meeting_id = %record.id, "live coordinator observed a joining row during finalizing CAS");
                    }
                    Err(error) => {
                        warn!(meeting_id = %record.id, error = %error, "failed to parse state after finalizing CAS miss");
                    }
                },
                Err(error) => {
                    warn!(meeting_id = %record.id, error = %error, "failed to read state after finalizing CAS miss");
                }
            },
            Err(error) => {
                if attempt >= 10 {
                    // A write that fails this persistently is almost certainly
                    // permanent (e.g. a schema-shape mismatch — the 2026-07-17
                    // CHECK incident retried every second forever). Keep
                    // retrying (the durable intent must eventually land, e.g.
                    // after a migration+restart), but loudly and slowly.
                    error!(meeting_id = %record.id, attempt, error = %error, "finalizing intent persistently failing; likely permanent (schema/constraint) — backing off");
                } else {
                    warn!(meeting_id = %record.id, attempt, error = %error, "failed to persist finalizing intent; retrying");
                }
            }
        }
        attempt = attempt.saturating_add(1);
        let base = service.ingestion.timing().barrier_retry;
        let backoff = base
            .saturating_mul(1u32 << attempt.min(6))
            .min(Duration::from_secs(60));
        tokio::select! {
            _ = service.ingestion.shutdown.cancelled() => return FinalizeIntentOutcome::Shutdown,
            _ = tokio::time::sleep(backoff) => {}
        }
    }
}

async fn begin_abort(
    service: &Arc<MeetingService>,
    record: &MeetingRecord,
    detail: &str,
    live_client: Option<RegisteredClient>,
) -> bool {
    let expected = match record.parsed_status() {
        Ok(MeetingStatus::Joining) => MeetingStatus::Joining,
        Ok(MeetingStatus::Live) => MeetingStatus::Live,
        Ok(MeetingStatus::Finalizing | MeetingStatus::Finalized) => {
            service.publish_abort(&record.id, AbortMeetingOutcome::RefusedFinalizing);
            return true;
        }
        Ok(MeetingStatus::Aborting | MeetingStatus::Aborted) => {
            service.publish_abort(&record.id, AbortMeetingOutcome::Aborting);
            return true;
        }
        Err(error) => {
            service.publish_abort(
                &record.id,
                AbortMeetingOutcome::Retryable(error.to_string()),
            );
            return true;
        }
    };
    match service.db.begin_meeting_abort(&record.id, expected, detail) {
        Ok(Some(_)) => {
            // Publish the durable CAS outcome before any best-effort leave.
            service.publish_abort(&record.id, AbortMeetingOutcome::Aborting);
        }
        Ok(None) => {
            let outcome = match service.db.get_meeting(&record.id) {
                Ok(None) => AbortMeetingOutcome::Deleted,
                Ok(Some(current))
                    if matches!(current.status.as_str(), "finalizing" | "finalized") =>
                {
                    AbortMeetingOutcome::RefusedFinalizing
                }
                Ok(Some(current)) if matches!(current.status.as_str(), "aborting" | "aborted") => {
                    AbortMeetingOutcome::Aborting
                }
                Ok(Some(_)) => AbortMeetingOutcome::Retryable(
                    "meeting abort CAS lost to an unexpected state".to_owned(),
                ),
                Err(error) => AbortMeetingOutcome::Retryable(error.to_string()),
            };
            service.publish_abort(&record.id, outcome);
            return true;
        }
        Err(error) => {
            service.publish_abort(
                &record.id,
                AbortMeetingOutcome::Retryable(error.to_string()),
            );
            return true;
        }
    }

    if expected == MeetingStatus::Live
        && !record.feishu_meeting_id.is_empty()
        && let Some(registered) = live_client
    {
        let timeout = service.ingestion.timing().platform_timeout;
        match tokio::time::timeout(timeout, registered.client.leave(&record.feishu_meeting_id))
            .await
        {
            Ok(Ok(())) => {
                if let Err(error) = service.db.clear_meeting_failure(&record.id) {
                    warn!(meeting_id = %record.id, error = %error, "failed to clear leave success state");
                }
            }
            Ok(Err(error)) => {
                warn!(meeting_id = %record.id, error = %error, "best-effort meeting leave failed; not retrying")
            }
            Err(_) => {
                warn!(meeting_id = %record.id, "best-effort meeting leave timed out; not retrying")
            }
        }
    }
    run_terminal_barrier(
        service,
        &record.id,
        MeetingStatus::Aborting,
        MeetingStatus::Aborted,
    )
    .await
}

async fn run_terminal_barrier(
    service: &Arc<MeetingService>,
    id: &str,
    expected: MeetingStatus,
    terminal: MeetingStatus,
) -> bool {
    loop {
        if service.ingestion.shutdown.is_cancelled() {
            return false;
        }
        match service.persist_terminal_index(id).await {
            Ok(()) => match service.db.complete_meeting_terminal(
                id,
                expected,
                terminal,
                &log::now_timestamp(),
            ) {
                Ok(Some(_)) | Ok(None) => return false,
                Err(error) => {
                    warn!(meeting_id = id, error = %error, "terminal meeting CAS failed after barrier")
                }
            },
            Err(error) => {
                warn!(meeting_id = id, error = %error, "meeting terminal cache/index barrier failed; retrying")
            }
        }
        tokio::select! {
            _ = service.ingestion.shutdown.cancelled() => return false,
            _ = tokio::time::sleep(service.ingestion.timing().barrier_retry) => {}
        }
    }
}

async fn commit_activity(
    service: &Arc<MeetingService>,
    record: &MeetingRecord,
    event_id: &str,
    payload: Value,
    bot_open_id: Option<String>,
) -> Option<EndSignalSource> {
    let normalized = match normalize_activity(payload, bot_open_id.as_deref()) {
        Ok(normalized) => normalized,
        Err(error) => {
            warn!(meeting_id = %record.id, event_id, error = %error, "failed to normalize meeting activity batch");
            return None;
        }
    };
    match service
        .append_batch(&record.id, normalized.drafts, event_id)
        .await
    {
        Ok(_) => normalized
            .own_participant_left
            .then_some(EndSignalSource::ParticipantLeft),
        Err(error) => {
            warn!(meeting_id = %record.id, event_id, error = %error, "meeting activity batch commit failed");
            // A checkpoint may already be durable even when only the cache
            // update failed. Repair is forward-only and guarded by epoch and
            // generation; a terminal barrier will repeat this if needed.
            if let Err(repair_error) = service.repair_log_cache(&record.id).await {
                warn!(meeting_id = %record.id, error = %repair_error, "meeting cache repair remains pending");
            }
            None
        }
    }
}

#[derive(Debug)]
pub(super) struct NormalizedActivity {
    drafts: Vec<SegmentDraft>,
    own_participant_left: bool,
}

fn activity_signals_own_participant_left(payload: &Value, bot_open_id: Option<&str>) -> bool {
    let Some(bot_open_id) = bot_open_id.filter(|value| !value.is_empty()) else {
        return false;
    };
    payload
        .get("meeting_activity_items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| {
            item.get("activity_event_type").and_then(Value::as_str) == Some("participant_left")
        })
        .filter_map(|item| item.get("participant_left_items").and_then(Value::as_array))
        .flatten()
        .any(|item| {
            item.get("participant")
                .and_then(Value::as_object)
                .and_then(|participant| nested_exact_id(participant, "id", "open_id"))
                .is_some_and(|open_id| open_id == bot_open_id)
        })
}

pub(super) fn normalize_activity(
    payload: Value,
    bot_open_id: Option<&str>,
) -> Result<NormalizedActivity, MeetingError> {
    let items = payload
        .get("meeting_activity_items")
        .and_then(Value::as_array)
        .ok_or_else(|| MeetingError::bad_request("meeting activity items must be an array"))?;
    let mut drafts = Vec::new();
    let mut item_count = 0usize;
    let mut own_participant_left = false;
    for item in items {
        let object = item
            .as_object()
            .ok_or_else(|| MeetingError::bad_request("meeting activity item must be an object"))?;
        match object
            .get("activity_event_type")
            .and_then(Value::as_str)
            .unwrap_or("")
        {
            "transcript_received" => {
                for transcript in exact_array(object, "transcript_received_items")? {
                    item_count = item_count.saturating_add(1);
                    enforce_batch_item_count(item_count)?;
                    let transcript = exact_object(transcript, "transcript item")?;
                    let source_id = exact_id_field(transcript, "sentence_id")?;
                    let text = exact_string_field(transcript, "text")?;
                    let start_ms = exact_i64_field(transcript, "start_time_ms")?;
                    let end_ms = exact_i64_field(transcript, "end_time_ms")?;
                    let speaker = transcript.get("speaker").and_then(Value::as_object);
                    let speaker_open_id = speaker
                        .and_then(|speaker| nested_exact_id(speaker, "id", "open_id"))
                        .unwrap_or_default();
                    let speaker_name = speaker
                        .and_then(|speaker| speaker.get("user_name"))
                        .and_then(Value::as_str)
                        .filter(|name| !name.is_empty())
                        .unwrap_or(&speaker_open_id)
                        .to_owned();
                    drafts.push(SegmentDraft {
                        kind: SegmentKind::Transcript,
                        speaker: speaker_name,
                        start: timestamp_from_millis(start_ms)?,
                        end: timestamp_from_millis(end_ms)?,
                        text,
                        source_id,
                    });
                }
            }
            "chat_received" => {
                for chat in exact_array(object, "chat_received_items")? {
                    item_count = item_count.saturating_add(1);
                    enforce_batch_item_count(item_count)?;
                    let chat = exact_object(chat, "chat item")?;
                    let source_id = exact_id_field(chat, "message_id")?;
                    let text = exact_string_field(chat, "content")?;
                    let sent_ms = exact_i64_field(chat, "sent_timestamp")?;
                    let operator = chat.get("operator").and_then(Value::as_object);
                    let operator_open_id = operator
                        .and_then(|operator| nested_exact_id(operator, "id", "open_id"))
                        .unwrap_or_default();
                    let speaker = operator
                        .and_then(|operator| {
                            operator
                                .get("user_name")
                                .and_then(Value::as_str)
                                .filter(|name| !name.is_empty())
                                .or_else(|| {
                                    operator
                                        .get("name")
                                        .and_then(Value::as_str)
                                        .filter(|name| !name.is_empty())
                                })
                        })
                        .unwrap_or(&operator_open_id)
                        .to_owned();
                    let timestamp = timestamp_from_millis(sent_ms)?;
                    drafts.push(SegmentDraft {
                        kind: SegmentKind::Chat,
                        speaker,
                        start: timestamp.clone(),
                        end: timestamp,
                        text,
                        source_id,
                    });
                }
            }
            "participant_left" => {
                for participant in exact_array(object, "participant_left_items")? {
                    item_count = item_count.saturating_add(1);
                    enforce_batch_item_count(item_count)?;
                    let participant = exact_object(participant, "participant-left item")?;
                    let open_id = participant
                        .get("participant")
                        .and_then(Value::as_object)
                        .and_then(|participant| nested_exact_id(participant, "id", "open_id"))
                        .unwrap_or_default();
                    if bot_open_id.is_some_and(|bot| !bot.is_empty() && bot == open_id) {
                        own_participant_left = true;
                    }
                }
            }
            other => debug!(
                activity_event_type = other,
                "ignoring unknown meeting activity item type"
            ),
        }
    }
    Ok(NormalizedActivity {
        drafts,
        own_participant_left,
    })
}

fn exact_array<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a [Value], MeetingError> {
    object
        .get(field)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| MeetingError::bad_request(format!("{field} must be an array")))
}

fn exact_object<'a>(value: &'a Value, name: &str) -> Result<&'a Map<String, Value>, MeetingError> {
    value
        .as_object()
        .ok_or_else(|| MeetingError::bad_request(format!("{name} must be an object")))
}

fn exact_string_field(object: &Map<String, Value>, field: &str) -> Result<String, MeetingError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| MeetingError::bad_request(format!("{field} must be a string")))
}

fn exact_id_field(object: &Map<String, Value>, field: &str) -> Result<String, MeetingError> {
    object
        .get(field)
        .and_then(json_exact_string_or_integer)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            MeetingError::bad_request(format!("{field} must be a non-empty string or integer"))
        })
}

fn exact_i64_field(object: &Map<String, Value>, field: &str) -> Result<i64, MeetingError> {
    let value = object
        .get(field)
        .ok_or_else(|| MeetingError::bad_request(format!("{field} is required")))?;
    match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok()))
            .ok_or_else(|| MeetingError::bad_request(format!("{field} must be an integer"))),
        Value::String(value) => value
            .parse::<i64>()
            .map_err(|_| MeetingError::bad_request(format!("{field} must be an integer"))),
        _ => Err(MeetingError::bad_request(format!(
            "{field} must be an integer"
        ))),
    }
}

fn nested_exact_id(object: &Map<String, Value>, outer: &str, inner: &str) -> Option<String> {
    object
        .get(outer)
        .and_then(Value::as_object)
        .and_then(|nested| nested.get(inner))
        .and_then(json_exact_string_or_integer)
}

fn json_exact_string_or_integer(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) if value.is_i64() || value.is_u64() => Some(value.to_string()),
        _ => None,
    }
}

fn enforce_batch_item_count(item_count: usize) -> Result<(), MeetingError> {
    if item_count > super::log::MAX_BATCH_ITEMS {
        Err(MeetingError::bad_request(
            "meeting activity batch exceeds 100 items",
        ))
    } else {
        Ok(())
    }
}

fn timestamp_from_millis(milliseconds: i64) -> Result<String, MeetingError> {
    DateTime::<Utc>::from_timestamp_millis(milliseconds)
        .map(timestamp)
        .ok_or_else(|| MeetingError::bad_request("meeting timestamp milliseconds are out of range"))
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn parse_deadline(value: &str) -> Result<Instant, MeetingError> {
    let deadline = DateTime::parse_from_rfc3339(value)
        .map_err(|_| MeetingError::storage("persisted meeting deadline is invalid"))?
        .with_timezone(&Utc);
    let remaining = deadline.signed_duration_since(Utc::now());
    Ok(if remaining <= chrono::Duration::zero() {
        Instant::now()
    } else {
        Instant::now()
            + remaining
                .to_std()
                .map_err(|_| MeetingError::storage("meeting deadline is out of range"))?
    })
}

fn record_platform_failure(service: &MeetingService, id: &str, error: &MeetingApiError) {
    if let Some(kind) = error.failure_kind()
        && let Err(db_error) = service.db.record_meeting_failure(id, kind)
    {
        warn!(meeting_id = id, error = %db_error, "failed to persist meeting platform failure");
    }
}
