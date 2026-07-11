use super::*;
use crate::{InMemoryTaskCounterStore, InMemoryThreadStore, update_thread_record};

struct StaticProjectionReader {
    running_subtask: bool,
}

struct ListProjectionReader {
    summary: TaskSummary,
}

#[async_trait]
impl TaskProjectionReader for StaticProjectionReader {
    async fn thread_id_for_number(&self, _number: u64) -> Result<Option<String>, String> {
        Ok(None)
    }

    async fn has_running_subtask_targeting(&self, _thread_id: &str) -> Result<bool, String> {
        Ok(self.running_subtask)
    }

    async fn list_task_summaries(
        &self,
        _filter: &TaskListFilter,
    ) -> Result<(Vec<TaskSummary>, usize, bool), String> {
        Ok((Vec::new(), 0, false))
    }
}

#[async_trait]
impl TaskProjectionReader for ListProjectionReader {
    async fn thread_id_for_number(&self, number: u64) -> Result<Option<String>, String> {
        Ok((number == self.summary.number).then(|| self.summary.thread_id.clone()))
    }

    async fn has_running_subtask_targeting(&self, _thread_id: &str) -> Result<bool, String> {
        Ok(false)
    }

    async fn list_task_summaries(
        &self,
        _filter: &TaskListFilter,
    ) -> Result<(Vec<TaskSummary>, usize, bool), String> {
        Ok((vec![self.summary.clone()], 1, false))
    }
}

fn service() -> TaskService {
    // Condition queries fall back to the lib ScanTaskProjectionReader for
    // stores without their own SQL projection — no explicit wiring needed.
    TaskService::new(
        Arc::new(InMemoryThreadStore::new()),
        Arc::new(InMemoryTaskCounterStore::new()),
    )
}

fn actor() -> Principal {
    Principal::Agent {
        agent_id: "cindy".to_owned(),
    }
}

/// Store wrapper that provides its own task projection reader — the shape
/// SqliteThreadStore uses in production.
struct StoreWithTaskProjection {
    inner: InMemoryThreadStore,
    reader: Arc<dyn TaskProjectionReader>,
}

#[async_trait]
impl ThreadStore for StoreWithTaskProjection {
    async fn get(&self, thread_id: &str) -> Result<Option<Value>, crate::ThreadStoreError> {
        self.inner.get(thread_id).await
    }
    async fn set(&self, thread_id: &str, data: Value) -> Result<(), crate::ThreadStoreError> {
        self.inner.set(thread_id, data).await
    }
    async fn delete(&self, thread_id: &str) -> Result<bool, crate::ThreadStoreError> {
        self.inner.delete(thread_id).await
    }
    async fn list_keys(
        &self,
        prefix: Option<&str>,
    ) -> Result<Vec<String>, crate::ThreadStoreError> {
        self.inner.list_keys(prefix).await
    }
    async fn exists(&self, thread_id: &str) -> Result<bool, crate::ThreadStoreError> {
        self.inner.exists(thread_id).await
    }
    async fn update(&self, thread_id: &str, updates: Value) -> Result<(), crate::ThreadStoreError> {
        self.inner.update(thread_id, updates).await
    }
    fn task_projection(&self) -> Option<Arc<dyn TaskProjectionReader>> {
        Some(self.reader.clone())
    }
}

#[tokio::test]
async fn running_subtask_gate_uses_store_provided_projection_reader() {
    let thread_store: Arc<dyn ThreadStore> = Arc::new(StoreWithTaskProjection {
        inner: InMemoryThreadStore::new(),
        reader: Arc::new(StaticProjectionReader {
            running_subtask: true,
        }),
    });

    assert!(
        thread_task_has_running_subtasks(&thread_store, "thread::parent")
            .await
            .expect("projection-backed subtask gate"),
        "the store-provided projection reader should answer the gate"
    );
}

#[tokio::test]
async fn list_tasks_uses_current_projection_reader_without_file_scan() {
    let summary = TaskSummary {
        thread_id: "thread::projected".to_owned(),
        task_id: "#TASK-100".to_owned(),
        number: 100,
        title: "Projected".to_owned(),
        status: TaskStatus::InProgress,
        creator: actor(),
        assignee: None,
        source: None,
        executor: None,
        updated_at: Utc::now(),
        updated_by: actor(),
        runtime_agent_id: "cindy".to_owned(),
        reply_count: 0,
    };
    let service = TaskService::new(
        Arc::new(InMemoryThreadStore::new()),
        Arc::new(InMemoryTaskCounterStore::new()),
    )
    .with_projection_reader(Arc::new(ListProjectionReader {
        summary: summary.clone(),
    }));

    let (tasks, total, has_more) = service
        .list_tasks(TaskListFilter {
            include_done: true,
            ..Default::default()
        })
        .await
        .expect("list from projection reader");

    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].thread_id, summary.thread_id);
    assert_eq!(tasks[0].number, summary.number);
    assert_eq!(tasks[0].title, summary.title);
    assert_eq!(total, 1);
    assert!(!has_more);
}

#[tokio::test]
async fn projected_task_mutations_increment_events_len() {
    let service = service();
    let (thread_id, created) = service
        .create_task(CreateTaskInput {
            title: Some("Projected task".to_owned()),
            body: None,
            assignee: None,
            notification_target: None,
            source: None,
            executor: None,
            start: false,
            actor: Some(actor()),
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();
    let mut previous_events_len = created.events.len();

    let titled = service
        .set_title(&thread_id, "Projected title".to_owned(), Some(actor()))
        .await
        .unwrap();
    assert!(titled.events.len() > previous_events_len);
    previous_events_len = titled.events.len();

    let assigned = service
        .assign_task(
            &thread_id,
            Principal::Agent {
                agent_id: "reviewer".to_owned(),
            },
            Some(actor()),
        )
        .await
        .unwrap();
    assert!(assigned.events.len() > previous_events_len);
    previous_events_len = assigned.events.len();

    let unassigned = service
        .unassign_task(&thread_id, Some(actor()))
        .await
        .unwrap();
    assert!(unassigned.events.len() > previous_events_len);
    previous_events_len = unassigned.events.len();

    let stopped = service.stop_task(&thread_id, Some(actor())).await.unwrap();
    assert!(stopped.events.len() > previous_events_len);
    previous_events_len = stopped.events.len();

    let restarted = service
        .update_status(UpdateTaskStatusInput {
            task_id: thread_id.clone(),
            to: TaskStatus::InProgress,
            note: None,
            force: false,
            actor: Some(actor()),
        })
        .await
        .unwrap();
    assert!(restarted.events.len() > previous_events_len);
    previous_events_len = restarted.events.len();

    let review = mark_thread_task_in_review_if_in_progress(
        &service.thread_store,
        &thread_id,
        actor(),
        None,
        None,
    )
    .await
    .unwrap()
    .expect("task enters review");
    assert!(review.task.events.len() > previous_events_len);
    previous_events_len = review.task.events.len();

    let woken = mark_thread_task_in_progress_on_wake(&service.thread_store, &thread_id, actor())
        .await
        .unwrap()
        .expect("task wakes to in progress");
    assert!(woken.events.len() > previous_events_len);
}

#[tokio::test]
async fn task_create_stores_task_overlay_without_task_messages() {
    let service = service();
    let (thread_id, task) = service
        .create_task(CreateTaskInput {
            title: Some("Audit daemons".to_owned()),
            body: Some("Look at launchctl".to_owned()),
            assignee: None,
            notification_target: None,
            source: None,
            executor: None,
            start: false,
            actor: Some(Principal::Agent {
                agent_id: "cindy".to_owned(),
            }),
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();
    assert!(task.number > 0);
    let record = service.thread_store.get(&thread_id).await.unwrap().unwrap();
    assert!(record.get("task").is_some());
    assert_eq!(record["thread_kind"], "task");
    // The body is no longer seeded into a record messages copy
    // (#TASK-1864 batch 1c): task.body is the canonical source and the
    // dispatch run writes it to the transcript.
    assert!(record.get("messages").is_none());
    assert_eq!(record["task"]["body"], "Look at launchctl");
}

#[tokio::test]
async fn task_create_stores_prefixed_thread_title() {
    let service = service();
    let (thread_id, task) = service
        .create_task(CreateTaskInput {
            title: Some("Audit daemons".to_owned()),
            body: None,
            assignee: None,
            notification_target: None,
            source: None,
            executor: None,
            start: false,
            actor: None,
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();

    let record = service.thread_store.get(&thread_id).await.unwrap().unwrap();
    assert_eq!(task.title, "Audit daemons");
    assert_eq!(
        record["label"],
        Value::String(format!("{} Audit daemons", canonical_task_id(&task)))
    );
    assert_eq!(record["thread_title_source"], "task");
}

#[tokio::test]
async fn set_title_updates_managed_thread_title() {
    let service = service();
    let (thread_id, task) = service
        .create_task(CreateTaskInput {
            title: Some("Original title".to_owned()),
            body: None,
            assignee: None,
            notification_target: None,
            source: None,
            executor: None,
            start: false,
            actor: None,
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();
    let task_id = canonical_task_id(&task);

    let updated = service
        .set_title(&task_id, "Updated title".to_owned(), None)
        .await
        .unwrap();

    let record = service.thread_store.get(&thread_id).await.unwrap().unwrap();
    assert_eq!(updated.title, "Updated title");
    assert_eq!(
        record["label"],
        Value::String(format!("{task_id} Updated title"))
    );
    assert_eq!(record["thread_title_source"], "task");
}

#[tokio::test]
async fn set_title_does_not_overwrite_manually_renamed_thread() {
    let service = service();
    let (thread_id, task) = service
        .create_task(CreateTaskInput {
            title: Some("Task title".to_owned()),
            body: None,
            assignee: None,
            notification_target: None,
            source: None,
            executor: None,
            start: false,
            actor: None,
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();
    let task_id = canonical_task_id(&task);
    update_thread_record(
        &service.thread_store,
        &thread_id,
        Some("Manual thread title".to_owned()),
        None,
    )
    .await
    .unwrap();

    let updated = service
        .set_title(&task_id, "New task title".to_owned(), None)
        .await
        .unwrap();

    let record = service.thread_store.get(&thread_id).await.unwrap().unwrap();
    assert_eq!(updated.title, "New task title");
    assert_eq!(record["label"], "Manual thread title");
    assert_eq!(record["thread_title_source"], "explicit");
}

#[tokio::test]
async fn set_title_leaves_legacy_unmanaged_thread_title_unchanged() {
    let service = service();
    let (thread_id, task) = service
        .create_task(CreateTaskInput {
            title: Some("Legacy task title".to_owned()),
            body: None,
            assignee: None,
            notification_target: None,
            source: None,
            executor: None,
            start: false,
            actor: None,
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();
    let task_id = canonical_task_id(&task);
    let mut record = service.thread_store.get(&thread_id).await.unwrap().unwrap();
    let obj = record.as_object_mut().unwrap();
    obj.insert(
        "label".to_owned(),
        Value::String("Legacy thread title".to_owned()),
    );
    obj.remove("thread_title_source");
    service.thread_store.set(&thread_id, record).await.unwrap();

    let updated = service
        .set_title(&task_id, "Retitled legacy task".to_owned(), None)
        .await
        .unwrap();

    let record = service.thread_store.get(&thread_id).await.unwrap().unwrap();
    assert_eq!(updated.title, "Retitled legacy task");
    assert_eq!(record["label"], "Legacy thread title");
    assert!(record.get("thread_title_source").is_none());
}

#[tokio::test]
async fn task_create_stores_source_and_list_filters_it() {
    let service = service();
    let (_thread_id, task) = service
        .create_task(CreateTaskInput {
            title: Some("Child task".to_owned()),
            body: None,
            assignee: None,
            notification_target: None,
            source: Some(TaskSource {
                thread_id: Some("thread::origin".to_owned()),
                task_id: Some("#TASK-7".to_owned()),
                task_thread_id: Some("thread::origin".to_owned()),
                bot_id: Some("telegram:main".to_owned()),
                channel: Some("telegram".to_owned()),
                account_id: Some("main".to_owned()),
            }),
            executor: None,
            start: false,
            actor: None,
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();
    assert_eq!(
        task.source
            .as_ref()
            .and_then(|source| source.task_id.as_deref()),
        Some("#TASK-7")
    );

    let (filtered, total, has_more) = service
        .list_tasks(TaskListFilter {
            source_thread_id: Some("thread::origin".to_owned()),
            source_task_id: Some("#TASK-7".to_owned()),
            source_bot_id: Some("telegram:main".to_owned()),
            include_done: true,
            limit: None,
            offset: None,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert!(!has_more);
    assert_eq!(filtered[0].task_id, canonical_task_id(&task));
    assert_eq!(
        filtered[0]
            .source
            .as_ref()
            .and_then(|source| source.bot_id.as_deref()),
        Some("telegram:main")
    );

    let (filtered, total, _) = service
        .list_tasks(TaskListFilter {
            source_bot_id: Some("telegram:other".to_owned()),
            include_done: true,
            limit: None,
            offset: None,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(total, 0);
    assert!(filtered.is_empty());
}

#[tokio::test]
async fn task_create_binds_agent_executor_to_thread() {
    let service = service();
    let (thread_id, task) = service
        .create_task(CreateTaskInput {
            title: Some("Run agent".to_owned()),
            body: None,
            assignee: None,
            notification_target: None,
            source: None,
            executor: Some(TaskExecutor::Agent {
                agent_id: "agent::reviewer".to_owned(),
            }),
            start: false,
            actor: None,
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();
    let record = service.thread_store.get(&thread_id).await.unwrap().unwrap();
    assert_eq!(record["agent_id"], "agent::reviewer");
    assert_eq!(
        task.executor,
        Some(TaskExecutor::Agent {
            agent_id: "agent::reviewer".to_owned(),
        })
    );
    assert_eq!(task.status, TaskStatus::InProgress);
}

#[tokio::test]
async fn status_machine_rejects_illegal_transition() {
    let service = service();
    let (_thread_id, task) = service
        .create_task(CreateTaskInput {
            title: Some("Review".to_owned()),
            body: None,
            assignee: None,
            notification_target: None,
            source: None,
            executor: None,
            start: false,
            actor: None,
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();
    let error = service
        .update_status(UpdateTaskStatusInput {
            task_id: canonical_task_id(&task),
            to: TaskStatus::Done,
            note: None,
            force: false,
            actor: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(error, TaskServiceError::InvalidTransition { .. }));
}

#[tokio::test]
async fn status_update_does_not_assign_todo_task() {
    let service = service();
    let (_thread_id, task) = service
        .create_task(CreateTaskInput {
            title: Some("Claim me".to_owned()),
            body: None,
            assignee: None,
            notification_target: None,
            source: None,
            executor: None,
            start: false,
            actor: None,
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();
    let updated = service
        .update_status(UpdateTaskStatusInput {
            task_id: canonical_task_id(&task),
            to: TaskStatus::InProgress,
            note: None,
            force: false,
            actor: Some(Principal::Agent {
                agent_id: "cindy".to_owned(),
            }),
        })
        .await
        .unwrap();
    assert_eq!(updated.status, TaskStatus::InProgress);
    assert_eq!(updated.assignee, None);
}

#[tokio::test]
async fn run_completion_marks_in_progress_task_in_review() {
    let service = service();
    let (thread_id, task) = service
        .create_task(CreateTaskInput {
            title: Some("Review when idle".to_owned()),
            body: None,
            assignee: Some(Principal::Agent {
                agent_id: "codex".to_owned(),
            }),
            notification_target: None,
            source: None,
            executor: None,
            start: true,
            actor: None,
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();

    let updated = mark_thread_task_in_review_if_in_progress(
        &service.thread_store,
        &thread_id,
        Principal::Agent {
            agent_id: "garyx".to_owned(),
        },
        Some("agent run completed".to_owned()),
        Some("handoff text".to_owned()),
    )
    .await
    .unwrap()
    .expect("in-progress task should move to review");

    assert_eq!(updated.handoff.as_deref(), Some("handoff text"));
    let updated = updated.task;
    assert_eq!(updated.status, TaskStatus::InReview);
    let (_, _, persisted) = service.get_task(&canonical_task_id(&task)).await.unwrap();
    assert_eq!(persisted.status, TaskStatus::InReview);
    assert!(matches!(
        persisted.events.last().map(|event| &event.kind),
        Some(TaskEventKind::StatusChanged {
            from: TaskStatus::InProgress,
            to: TaskStatus::InReview,
            note: Some(note),
        }) if note == "agent run completed"
    ));
}

#[tokio::test]
async fn run_completion_defers_review_while_subtasks_run() {
    let service = service();
    // The run-end transition is a free function; without a store-provided
    // SQL reader it falls back to the scan reader automatically.
    let (parent_thread_id, _parent_task) = service
        .create_task(CreateTaskInput {
            title: Some("Parent work".to_owned()),
            body: None,
            assignee: Some(Principal::Agent {
                agent_id: "codex".to_owned(),
            }),
            notification_target: None,
            source: None,
            executor: None,
            start: true,
            actor: None,
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();
    let (_child_thread_id, child_task) = service
        .create_task(CreateTaskInput {
            title: Some("Child review".to_owned()),
            body: None,
            assignee: Some(Principal::Agent {
                agent_id: "reviewer".to_owned(),
            }),
            notification_target: Some(TaskNotificationTarget::Thread {
                thread_id: parent_thread_id.clone(),
            }),
            source: None,
            executor: None,
            start: true,
            actor: None,
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();
    assert_eq!(child_task.status, TaskStatus::InProgress);

    // The parent's run ends while the child still runs: the parent task
    // must stay in progress so the parent notification defers until the
    // child has returned.
    let gated = mark_thread_task_in_review_if_in_progress(
        &service.thread_store,
        &parent_thread_id,
        Principal::Agent {
            agent_id: "garyx".to_owned(),
        },
        Some("agent run completed".to_owned()),
        Some("parent handoff".to_owned()),
    )
    .await
    .unwrap();
    assert!(gated.is_none());
    let (_, _, parent) = service
        .get_task(&format!("#TASK-{}", _parent_task.number))
        .await
        .unwrap();
    assert_eq!(parent.status, TaskStatus::InProgress);

    // Once the child has returned (in review), the parent's next run end
    // transitions normally.
    service
        .update_status(UpdateTaskStatusInput {
            task_id: canonical_task_id(&child_task),
            to: TaskStatus::InReview,
            note: None,
            force: false,
            actor: Some(Principal::Agent {
                agent_id: "reviewer".to_owned(),
            }),
        })
        .await
        .unwrap();
    let released = mark_thread_task_in_review_if_in_progress(
        &service.thread_store,
        &parent_thread_id,
        Principal::Agent {
            agent_id: "garyx".to_owned(),
        },
        Some("agent run completed".to_owned()),
        Some("parent handoff".to_owned()),
    )
    .await
    .unwrap()
    .expect("parent task should move to review once no subtasks run");
    assert_eq!(released.handoff.as_deref(), Some("parent handoff"));
    let released = released.task;
    assert_eq!(released.status, TaskStatus::InReview);
}

#[tokio::test]
async fn run_completion_leaves_non_progress_task_status_unchanged() {
    let service = service();
    let (thread_id, task) = service
        .create_task(CreateTaskInput {
            title: Some("Already reviewed".to_owned()),
            body: None,
            assignee: Some(Principal::Agent {
                agent_id: "codex".to_owned(),
            }),
            notification_target: None,
            source: None,
            executor: None,
            start: true,
            actor: None,
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();
    service
        .update_status(UpdateTaskStatusInput {
            task_id: canonical_task_id(&task),
            to: TaskStatus::InReview,
            note: None,
            force: false,
            actor: None,
        })
        .await
        .unwrap();

    let updated = mark_thread_task_in_review_if_in_progress(
        &service.thread_store,
        &thread_id,
        Principal::Agent {
            agent_id: "garyx".to_owned(),
        },
        Some("agent run completed".to_owned()),
        Some("handoff text".to_owned()),
    )
    .await
    .unwrap();

    assert!(updated.is_none());
    let (_, _, persisted) = service.get_task(&canonical_task_id(&task)).await.unwrap();
    assert_eq!(persisted.status, TaskStatus::InReview);
}

#[tokio::test]
async fn run_wake_revives_in_review_task_to_in_progress() {
    let service = service();
    let (thread_id, task) = service
        .create_task(CreateTaskInput {
            title: Some("Wake reviewed task".to_owned()),
            body: None,
            assignee: Some(Principal::Agent {
                agent_id: "codex".to_owned(),
            }),
            notification_target: None,
            source: None,
            executor: None,
            start: true,
            actor: None,
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();
    service
        .update_status(UpdateTaskStatusInput {
            task_id: canonical_task_id(&task),
            to: TaskStatus::InReview,
            note: None,
            force: false,
            actor: None,
        })
        .await
        .unwrap();

    let updated = mark_thread_task_in_progress_on_wake(
        &service.thread_store,
        &thread_id,
        Principal::Agent {
            agent_id: "garyx".to_owned(),
        },
    )
    .await
    .unwrap()
    .expect("in-review task should revive");

    assert_eq!(updated.status, TaskStatus::InProgress);
    assert!(matches!(
        updated.events.last().map(|event| &event.kind),
        Some(TaskEventKind::StatusChanged {
            from: TaskStatus::InReview,
            to: TaskStatus::InProgress,
            note: None,
        })
    ));
}

#[tokio::test]
async fn run_wake_revives_done_task_to_in_progress() {
    let service = service();
    let (thread_id, task) = service
        .create_task(CreateTaskInput {
            title: Some("Wake done task".to_owned()),
            body: None,
            assignee: Some(Principal::Agent {
                agent_id: "codex".to_owned(),
            }),
            notification_target: None,
            source: None,
            executor: None,
            start: true,
            actor: None,
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();
    let task_id = canonical_task_id(&task);
    service
        .update_status(UpdateTaskStatusInput {
            task_id: task_id.clone(),
            to: TaskStatus::InReview,
            note: None,
            force: false,
            actor: None,
        })
        .await
        .unwrap();
    service
        .update_status(UpdateTaskStatusInput {
            task_id: task_id.clone(),
            to: TaskStatus::Done,
            note: None,
            force: false,
            actor: None,
        })
        .await
        .unwrap();

    let updated = mark_thread_task_in_progress_on_wake(
        &service.thread_store,
        &thread_id,
        Principal::Agent {
            agent_id: "garyx".to_owned(),
        },
    )
    .await
    .unwrap()
    .expect("done task should revive");

    assert_eq!(updated.status, TaskStatus::InProgress);
    assert!(matches!(
        updated.events.last().map(|event| &event.kind),
        Some(TaskEventKind::StatusChanged {
            from: TaskStatus::Done,
            to: TaskStatus::InProgress,
            note: None,
        })
    ));
}

#[tokio::test]
async fn status_machine_allows_done_to_in_progress() {
    let service = service();
    let (_thread_id, task) = service
        .create_task(CreateTaskInput {
            title: Some("Resume done task".to_owned()),
            body: None,
            assignee: Some(Principal::Agent {
                agent_id: "codex".to_owned(),
            }),
            notification_target: None,
            source: None,
            executor: None,
            start: true,
            actor: None,
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();
    let task_id = canonical_task_id(&task);
    service
        .update_status(UpdateTaskStatusInput {
            task_id: task_id.clone(),
            to: TaskStatus::InReview,
            note: None,
            force: false,
            actor: None,
        })
        .await
        .unwrap();
    service
        .update_status(UpdateTaskStatusInput {
            task_id: task_id.clone(),
            to: TaskStatus::Done,
            note: None,
            force: false,
            actor: None,
        })
        .await
        .unwrap();

    let updated = service
        .update_status(UpdateTaskStatusInput {
            task_id,
            to: TaskStatus::InProgress,
            note: None,
            force: false,
            actor: Some(Principal::Agent {
                agent_id: "garyx".to_owned(),
            }),
        })
        .await
        .unwrap();

    assert_eq!(updated.status, TaskStatus::InProgress);
}

#[tokio::test]
async fn assignee_can_mark_done_after_explicit_review_confirmation() {
    let service = service();
    let assignee = Principal::Agent {
        agent_id: "codex".to_owned(),
    };
    let (_thread_id, task) = service
        .create_task(CreateTaskInput {
            title: Some("Review gate".to_owned()),
            body: None,
            assignee: Some(assignee.clone()),
            notification_target: None,
            source: None,
            executor: None,
            start: true,
            actor: Some(Principal::Human {
                user_id: "owner".to_owned(),
            }),
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();
    let task_id = canonical_task_id(&task);

    service
        .update_status(UpdateTaskStatusInput {
            task_id: task_id.clone(),
            to: TaskStatus::InReview,
            note: None,
            force: false,
            actor: Some(assignee.clone()),
        })
        .await
        .unwrap();

    let updated = service
        .update_status(UpdateTaskStatusInput {
            task_id,
            to: TaskStatus::Done,
            note: Some("review approved by owner".to_owned()),
            force: false,
            actor: Some(assignee),
        })
        .await
        .unwrap();

    assert_eq!(updated.status, TaskStatus::Done);
}

#[tokio::test]
async fn reviewer_can_mark_reviewed_task_done() {
    let service = service();
    let assignee = Principal::Agent {
        agent_id: "codex".to_owned(),
    };
    let (_thread_id, task) = service
        .create_task(CreateTaskInput {
            title: Some("Review pass".to_owned()),
            body: None,
            assignee: Some(assignee.clone()),
            notification_target: None,
            source: None,
            executor: None,
            start: true,
            actor: Some(Principal::Human {
                user_id: "owner".to_owned(),
            }),
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();
    let task_id = canonical_task_id(&task);

    service
        .update_status(UpdateTaskStatusInput {
            task_id: task_id.clone(),
            to: TaskStatus::InReview,
            note: None,
            force: false,
            actor: Some(assignee),
        })
        .await
        .unwrap();

    let updated = service
        .update_status(UpdateTaskStatusInput {
            task_id,
            to: TaskStatus::Done,
            note: None,
            force: false,
            actor: Some(Principal::Human {
                user_id: "owner".to_owned(),
            }),
        })
        .await
        .unwrap();

    assert_eq!(updated.status, TaskStatus::Done);
}

#[tokio::test]
async fn assign_starts_todo_task() {
    let service = service();
    let (_thread_id, task) = service
        .create_task(CreateTaskInput {
            title: Some("Assign me".to_owned()),
            body: None,
            assignee: None,
            notification_target: None,
            source: None,
            executor: None,
            start: false,
            actor: None,
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();
    let assignee = Principal::Agent {
        agent_id: "cindy".to_owned(),
    };
    let updated = service
        .assign_task(&canonical_task_id(&task), assignee.clone(), Some(assignee))
        .await
        .unwrap();
    assert_eq!(updated.status, TaskStatus::InProgress);
    assert_eq!(
        updated.assignee,
        Some(Principal::Agent {
            agent_id: "cindy".to_owned()
        })
    );
    assert_eq!(updated.events.len(), 3);
}

#[tokio::test]
async fn stop_running_task_moves_to_todo_and_releases_assignee() {
    let service = service();
    let (_thread_id, task) = service
        .create_task(CreateTaskInput {
            title: Some("Stop me".to_owned()),
            body: None,
            assignee: Some(Principal::Agent {
                agent_id: "codex".to_owned(),
            }),
            notification_target: None,
            source: None,
            executor: None,
            start: true,
            actor: None,
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();

    let stopped = service
        .stop_task(
            &canonical_task_id(&task),
            Some(Principal::Human {
                user_id: "tester".to_owned(),
            }),
        )
        .await
        .unwrap();

    assert_eq!(stopped.status, TaskStatus::Todo);
    assert_eq!(stopped.assignee, None);
    assert!(matches!(
        stopped.events.iter().rev().nth(1).map(|event| &event.kind),
        Some(TaskEventKind::StatusChanged {
            from: TaskStatus::InProgress,
            to: TaskStatus::Todo,
            note: Some(note),
        }) if note == "stopped"
    ));
    assert!(matches!(
        stopped.events.last().map(|event| &event.kind),
        Some(TaskEventKind::Released {
            previous_assignee: Some(Principal::Agent { agent_id }),
        }) if agent_id == "codex"
    ));
}

#[tokio::test]
async fn delete_task_removes_overlay_from_list_but_keeps_thread_record() {
    let service = service();
    let (thread_id, task) = service
        .create_task(CreateTaskInput {
            title: Some("Delete task metadata".to_owned()),
            body: Some("Keep the backing thread for audit.".to_owned()),
            assignee: None,
            notification_target: None,
            source: None,
            executor: None,
            start: false,
            actor: None,
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();
    let task_id = canonical_task_id(&task);

    let (deleted_thread_id, deleted_task) = service.delete_task(&task_id).await.unwrap();

    assert_eq!(deleted_thread_id, thread_id);
    assert_eq!(canonical_task_id(&deleted_task), task_id);
    let record = service
        .thread_store
        .get(&thread_id)
        .await
        .unwrap()
        .expect("backing thread remains");
    assert!(record.get("task").is_none());
    assert_eq!(record["thread_kind"], "task");
    let (listed, total, has_more) = service
        .list_tasks(TaskListFilter {
            include_done: true,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(total, 0);
    assert!(!has_more);
    assert!(listed.is_empty());
    assert!(matches!(
        service.get_task(&task_id).await.unwrap_err(),
        TaskServiceError::NotFound(_)
    ));
}

#[tokio::test]
async fn concurrent_mutations_preserve_both_events() {
    let service = Arc::new(service());
    let (_thread_id, task) = service
        .create_task(CreateTaskInput {
            title: Some("Concurrent".to_owned()),
            body: None,
            assignee: None,
            notification_target: None,
            source: None,
            executor: None,
            start: false,
            actor: None,
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();
    let task_id = canonical_task_id(&task);

    let left_service = service.clone();
    let left_id = task_id.clone();
    let left = tokio::spawn(async move {
        left_service
            .assign_task(
                &left_id,
                Principal::Agent {
                    agent_id: "cindy".to_owned(),
                },
                None,
            )
            .await
            .unwrap();
    });
    let right_service = service.clone();
    let right = tokio::spawn(async move {
        right_service
            .set_title(&task_id, "Retitled".to_owned(), None)
            .await
            .unwrap();
    });
    left.await.unwrap();
    right.await.unwrap();

    let (_, _, task) = service.get_task(&canonical_task_id(&task)).await.unwrap();
    assert_eq!(task.events.len(), 4);
    assert_eq!(task.title, "Retitled");
    assert_eq!(
        task.assignee,
        Some(Principal::Agent {
            agent_id: "cindy".to_owned()
        })
    );
}

#[tokio::test]
async fn task_history_supports_before_cursor() {
    let service = service();
    let (_thread_id, task) = service
        .create_task(CreateTaskInput {
            title: Some("History".to_owned()),
            body: None,
            assignee: None,
            notification_target: None,
            source: None,
            executor: None,
            start: false,
            actor: None,
            workspace_dir: None,
            runtime: None,
        })
        .await
        .unwrap();
    let task_id = canonical_task_id(&task);
    service
        .assign_task(
            &task_id,
            Principal::Agent {
                agent_id: "cindy".to_owned(),
            },
            None,
        )
        .await
        .unwrap();
    service
        .set_title(&task_id, "History updated".to_owned(), None)
        .await
        .unwrap();

    let first_page = service.task_history(&task_id, Some(1), None).await.unwrap();
    assert_eq!(first_page.events.len(), 1);
    assert!(first_page.has_more);
    let second_page = service
        .task_history(&task_id, Some(10), Some(&first_page.events[0].event_id))
        .await
        .unwrap();
    assert_eq!(second_page.events.len(), 3);
    assert!(!second_page.has_more);
}

#[tokio::test]
async fn task_create_persists_runtime_fields() {
    let service = service();
    let (thread_id, _task) = service
        .create_task(CreateTaskInput {
            title: Some("Runtime".to_owned()),
            body: None,
            assignee: None,
            notification_target: None,
            source: None,
            executor: None,
            start: false,
            actor: None,
            workspace_dir: None,
            runtime: Some(TaskRuntimeInput {
                agent_id: Some("codex".to_owned()),
                workspace_dir: Some("/tmp/garyx-task".to_owned()),
                workspace_mode: WorkspaceMode::Local,
                worktree_base_dir: None,
            }),
        })
        .await
        .unwrap();
    let record = service.thread_store.get(&thread_id).await.unwrap().unwrap();
    assert_eq!(record["agent_id"], Value::String("codex".to_owned()));
    assert_eq!(
        record["workspace_dir"],
        Value::String("/tmp/garyx-task".to_owned())
    );
}
