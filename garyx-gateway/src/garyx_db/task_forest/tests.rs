use super::super::RecentThreadDraft;
use super::*;

fn task_projection_draft(
    thread_id: &str,
    number: u64,
    status: TaskStatus,
    updated_at: &str,
    source: Option<TaskSource>,
    source_events_len: usize,
) -> TaskProjectionDraft {
    let creator = Principal::Agent {
        agent_id: "test-agent".to_owned(),
    };
    let assignee = Principal::Human {
        user_id: "1000000001".to_owned(),
    };
    let updated_by = creator.clone();
    let parent_task_number = source
        .as_ref()
        .and_then(|source| source.task_id.as_deref())
        .and_then(|task_id| task_id.strip_prefix("#TASK-"))
        .and_then(|number| number.parse::<u64>().ok());
    let source_bot_id = source
        .as_ref()
        .and_then(|source| source.bot_id.clone())
        .or_else(|| {
            source.as_ref().and_then(|source| {
                Some(format!(
                    "{}:{}",
                    source.channel.as_ref()?,
                    source.account_id.as_ref()?
                ))
            })
        });
    TaskProjectionDraft {
        thread_id: thread_id.to_owned(),
        number,
        status: status.as_str().to_owned(),
        title: format!("Task {number}"),
        creator_json: serde_json::to_string(&creator).expect("creator json"),
        creator_id: creator.id().to_owned(),
        assignee_json: Some(serde_json::to_string(&assignee).expect("assignee json")),
        assignee_id: Some(assignee.id().to_owned()),
        updated_by_json: serde_json::to_string(&updated_by).expect("updated_by json"),
        executor_json: None,
        source_json: source
            .as_ref()
            .map(|source| serde_json::to_string(source).expect("source json")),
        source_thread_id: source.as_ref().and_then(|source| source.thread_id.clone()),
        source_task_thread_id: source
            .as_ref()
            .and_then(|source| source.task_thread_id.clone()),
        source_task_id: source.as_ref().and_then(|source| source.task_id.clone()),
        parent_task_number,
        source_bot_id,
        notification_thread_id: None,
        created_at: "2026-01-01T00:00:00.000Z".to_owned(),
        updated_at: updated_at.to_owned(),
        source_updated_at: updated_at.to_owned(),
        source_events_len,
    }
}

fn thread_source(thread_id: &str, task_id: &str) -> TaskSource {
    TaskSource {
        thread_id: Some(thread_id.to_owned()),
        task_id: Some(task_id.to_owned()),
        task_thread_id: Some(thread_id.to_owned()),
        bot_id: None,
        channel: None,
        account_id: None,
    }
}

fn chat_source(thread_id: &str) -> TaskSource {
    TaskSource {
        thread_id: Some(thread_id.to_owned()),
        task_id: None,
        task_thread_id: None,
        bot_id: None,
        channel: None,
        account_id: None,
    }
}

fn bot_thread_source(thread_id: &str, task_id: &str, bot_id: &str) -> TaskSource {
    TaskSource {
        thread_id: Some(thread_id.to_owned()),
        task_id: Some(task_id.to_owned()),
        task_thread_id: Some(thread_id.to_owned()),
        bot_id: Some(bot_id.to_owned()),
        channel: None,
        account_id: None,
    }
}

fn with_creator(mut draft: TaskProjectionDraft, creator: &Principal) -> TaskProjectionDraft {
    draft.creator_json = serde_json::to_string(creator).expect("creator json");
    draft.creator_id = creator.id().to_owned();
    draft
}

fn with_assignee(mut draft: TaskProjectionDraft, assignee: &Principal) -> TaskProjectionDraft {
    draft.assignee_json = Some(serde_json::to_string(assignee).expect("assignee json"));
    draft.assignee_id = Some(assignee.id().to_owned());
    draft
}

#[test]
fn allocate_task_number_is_unique_and_contiguous_under_concurrency() {
    let db = std::sync::Arc::new(GaryxDbService::memory().expect("db opens"));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let db = db.clone();
        handles.push(std::thread::spawn(move || {
            (0..5)
                .map(|_| db.allocate_task_number().expect("allocate"))
                .collect::<Vec<_>>()
        }));
    }
    let mut numbers = Vec::new();
    for handle in handles {
        numbers.extend(handle.join().expect("allocator thread"));
    }
    numbers.sort_unstable();
    assert_eq!(numbers, (1..=40).collect::<Vec<_>>());
}

#[test]
fn allocate_task_number_floors_against_projection_max() {
    let db = GaryxDbService::memory().expect("db opens");
    db.replace_task_projection(task_projection_draft(
        "thread::existing",
        41,
        TaskStatus::Todo,
        "2026-06-01T00:00:00.000Z",
        None,
        1,
    ))
    .expect("seed projection");
    assert_eq!(db.allocate_task_number().expect("allocate"), 42);
    assert_eq!(db.allocate_task_number().expect("allocate again"), 43);
}

#[test]
fn seed_task_counter_migrates_file_floor_and_record_bodies_once() {
    let db = GaryxDbService::memory().expect("db opens");
    // A record body with an embedded task (e.g. an archived thread whose
    // projection rows were removed) must still floor the allocator.
    db.write_thread_record_with_projections(
        "thread::archived-task",
        r#"{"task":{"number":7,"title":"old"}}"#,
        None,
        None,
    )
    .expect("write record");

    assert!(
        db.seed_task_counter_if_missing(5)
            .expect("seed with file floor 5")
    );
    assert_eq!(
        db.allocate_task_number().expect("allocate after seed"),
        8,
        "seed takes max(file floor, record-body task numbers)"
    );

    // Seeding is one-shot: an existing row is never overwritten.
    assert!(!db.seed_task_counter_if_missing(100).expect("second seed"));
    assert_eq!(db.allocate_task_number().expect("allocate again"), 9);
}

#[test]
fn task_projection_list_filters_and_dedupes_duplicate_numbers() {
    let db = GaryxDbService::memory().expect("db opens");
    let source = TaskSource {
        thread_id: Some("thread::origin".to_owned()),
        task_id: Some("#TASK-1".to_owned()),
        task_thread_id: Some("thread::parent".to_owned()),
        bot_id: None,
        channel: Some("api".to_owned()),
        account_id: Some("main".to_owned()),
    };
    db.replace_task_projection(task_projection_draft(
        "thread::older",
        42,
        TaskStatus::InProgress,
        "2026-01-01T00:00:01.000Z",
        Some(source.clone()),
        1,
    ))
    .expect("insert older duplicate");
    db.replace_task_projection(task_projection_draft(
        "thread::newer",
        42,
        TaskStatus::InReview,
        "2026-01-01T00:00:02.000Z",
        Some(source),
        2,
    ))
    .expect("insert newer duplicate");
    db.replace_task_projection(task_projection_draft(
        "thread::done",
        43,
        TaskStatus::Done,
        "2026-01-01T00:00:03.000Z",
        None,
        1,
    ))
    .expect("insert done row");

    let (tasks, total, has_more) = db
        .list_task_summaries(&TaskListFilter {
            source_thread_id: Some("thread::parent".to_owned()),
            source_task_id: Some("#task-1".to_owned()),
            source_bot_id: Some("api:main".to_owned()),
            include_done: false,
            limit: Some(10),
            offset: Some(0),
            ..Default::default()
        })
        .expect("list filtered task projection");

    assert_eq!(total, 1);
    assert!(!has_more);
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].thread_id, "thread::newer");
    assert_eq!(tasks[0].number, 42);
    assert_eq!(tasks[0].status, TaskStatus::InReview);
}

#[test]
fn task_projection_recursive_ctes_use_thread_identity_and_guard_cycles() {
    let db = GaryxDbService::memory().expect("db opens");
    db.replace_task_projection(task_projection_draft(
        "thread::parent",
        1,
        TaskStatus::InProgress,
        "2026-01-01T00:00:01.000Z",
        None,
        1,
    ))
    .expect("insert parent");
    db.replace_task_projection(task_projection_draft(
        "thread::child",
        2,
        TaskStatus::InProgress,
        "2026-01-01T00:00:02.000Z",
        Some(thread_source("thread::parent", "#TASK-1")),
        1,
    ))
    .expect("insert child");
    db.replace_task_projection(task_projection_draft(
        "thread::grandchild",
        3,
        TaskStatus::Todo,
        "2026-01-01T00:00:03.000Z",
        Some(thread_source("thread::child", "#TASK-2")),
        1,
    ))
    .expect("insert grandchild");
    db.replace_task_projection(task_projection_draft(
        "thread::cycle",
        4,
        TaskStatus::Todo,
        "2026-01-01T00:00:04.000Z",
        Some(thread_source("thread::cycle", "#TASK-4")),
        1,
    ))
    .expect("insert self cycle");

    let subtree = db
        .task_subtree_summaries("thread::parent")
        .expect("subtree");
    assert_eq!(
        subtree
            .iter()
            .map(|task| task.thread_id.as_str())
            .collect::<Vec<_>>(),
        vec!["thread::parent", "thread::child", "thread::grandchild"]
    );

    let ancestors = db
        .task_ancestor_summaries("thread::grandchild")
        .expect("ancestors");
    assert_eq!(
        ancestors
            .iter()
            .map(|task| task.thread_id.as_str())
            .collect::<Vec<_>>(),
        vec!["thread::parent", "thread::child", "thread::grandchild"]
    );

    let cycle = db.task_subtree_summaries("thread::cycle").expect("cycle");
    assert_eq!(cycle.len(), 1);
}

#[test]
fn task_forest_includes_parent_and_run_state_fields() {
    let db = GaryxDbService::memory().expect("db opens");
    db.replace_task_projection(task_projection_draft(
        "thread::parent",
        1,
        TaskStatus::InProgress,
        "2026-01-01T00:00:01.000Z",
        None,
        1,
    ))
    .expect("insert parent");
    db.replace_task_projection(task_projection_draft(
        "thread::child",
        2,
        TaskStatus::Todo,
        "2026-01-01T00:00:02.000Z",
        Some(thread_source("thread::parent", "#TASK-1")),
        1,
    ))
    .expect("insert child");
    db.replace_task_projection(task_projection_draft(
        "thread::legacy-child",
        3,
        TaskStatus::Todo,
        "2026-01-01T00:00:03.000Z",
        Some(TaskSource {
            thread_id: Some("thread::origin".to_owned()),
            task_id: Some("#TASK-1".to_owned()),
            task_thread_id: None,
            bot_id: None,
            channel: None,
            account_id: None,
        }),
        1,
    ))
    .expect("insert legacy child");
    db.upsert_recent_thread(RecentThreadDraft {
        thread_id: "thread::child".to_owned(),
        title: "Child".to_owned(),
        workspace_dir: None,
        root_workspace_path: None,
        workspace_origin: None,
        thread_type: "chat".to_owned(),
        provider_type: Some("claude_code".to_owned()),
        agent_id: Some("claude".to_owned()),
        message_count: 4,
        last_message_preview: "Working".to_owned(),
        recent_run_id: Some("run::recent".to_owned()),
        active_run_id: Some("run::active".to_owned()),
        run_state: "running".to_owned(),
        updated_at: Some("2026-01-01T00:00:03.000Z".to_owned()),
        last_active_at: "2026-01-01T00:00:04.000Z".to_owned(),
    })
    .expect("insert recent thread");

    let page = db
        .list_task_forest(
            &TaskListFilter {
                include_done: true,
                ..Default::default()
            },
            TaskForestScope::All,
        )
        .expect("list forest");

    assert_eq!(page.total, 3);
    assert!(page.root_thread_ids.is_empty());
    assert!(page.skipped_pinned_thread_ids.is_empty());
    let child = page
        .tasks
        .iter()
        .find(|node| node.thread_id() == "thread::child")
        .expect("child node");
    assert_eq!(child.parent_task_number(), Some(1));
    assert_eq!(child.parent_thread_id(), Some("thread::parent"));
    assert_eq!(child.active_run_id(), Some("run::active"));
    assert_eq!(child.run_state(), "running");
    assert_eq!(child.last_active_at(), Some("2026-01-01T00:00:04.000Z"));
    let legacy_child = page
        .tasks
        .iter()
        .find(|node| node.thread_id() == "thread::legacy-child")
        .expect("legacy child node");
    assert_eq!(legacy_child.parent_task_number(), Some(1));
    assert_eq!(legacy_child.parent_thread_id(), Some("thread::parent"));
    let parent = page
        .tasks
        .iter()
        .find(|node| node.thread_id() == "thread::parent")
        .expect("parent node");
    assert_eq!(parent.parent_task_number(), None);
    assert_eq!(parent.run_state(), "idle");
}

#[test]
fn pinned_task_forest_returns_pinned_roots_and_descendants() {
    let db = GaryxDbService::memory().expect("db opens");
    db.upsert_recent_thread(RecentThreadDraft {
        thread_id: "thread::chat-a".to_owned(),
        title: "Chat A".to_owned(),
        workspace_dir: None,
        root_workspace_path: None,
        workspace_origin: None,
        thread_type: "chat".to_owned(),
        provider_type: Some("codex".to_owned()),
        agent_id: Some("codex".to_owned()),
        message_count: 7,
        last_message_preview: "Coordinate A".to_owned(),
        recent_run_id: None,
        active_run_id: None,
        run_state: "idle".to_owned(),
        updated_at: Some("2026-01-01T00:00:01.500Z".to_owned()),
        last_active_at: "2026-01-01T00:00:01.500Z".to_owned(),
    })
    .expect("insert chat a");
    db.upsert_recent_thread(RecentThreadDraft {
        thread_id: "thread::chat-b".to_owned(),
        title: "Chat B".to_owned(),
        workspace_dir: None,
        root_workspace_path: None,
        workspace_origin: None,
        thread_type: "chat".to_owned(),
        provider_type: Some("claude_code".to_owned()),
        agent_id: Some("claude".to_owned()),
        message_count: 3,
        last_message_preview: "Coordinate B".to_owned(),
        recent_run_id: None,
        active_run_id: Some("run::chat-b".to_owned()),
        run_state: "running".to_owned(),
        updated_at: Some("2026-01-01T00:00:03.500Z".to_owned()),
        last_active_at: "2026-01-01T00:00:03.500Z".to_owned(),
    })
    .expect("insert chat b");
    db.replace_task_projection(task_projection_draft(
        "thread::child-a",
        11,
        TaskStatus::InProgress,
        "2026-01-01T00:00:02.000Z",
        Some(chat_source("thread::chat-a")),
        1,
    ))
    .expect("insert child a");
    db.replace_task_projection(task_projection_draft(
        "thread::grandchild-a",
        12,
        TaskStatus::InReview,
        "2026-01-01T00:00:03.000Z",
        Some(thread_source("thread::child-a", "#TASK-11")),
        1,
    ))
    .expect("insert grandchild a");
    db.replace_task_projection(task_projection_draft(
        "thread::child-b",
        21,
        TaskStatus::InProgress,
        "2026-01-01T00:00:05.000Z",
        Some(chat_source("thread::chat-b")),
        1,
    ))
    .expect("insert child b");
    db.replace_task_projection(task_projection_draft(
        "thread::unrelated",
        99,
        TaskStatus::Todo,
        "2026-01-01T00:00:06.000Z",
        None,
        1,
    ))
    .expect("insert unrelated");
    db.conn()
        .expect("db connection")
        .execute_batch(
            "INSERT INTO thread_pins (thread_id, pinned_at, sort_order)
             VALUES
               ('thread::chat-a', '2026-01-01T00:00:03.000Z', 2),
               ('thread::chat', '2026-01-01T00:00:02.000Z', 1),
               ('thread::chat-b', '2026-01-01T00:00:01.000Z', 0)",
        )
        .expect("insert pins");

    let page = db
        .list_task_forest(
            &TaskListFilter {
                include_done: true,
                ..Default::default()
            },
            TaskForestScope::Pinned,
        )
        .expect("list pinned forest");

    assert_eq!(
        page.tasks
            .iter()
            .map(|node| node.thread_id())
            .collect::<Vec<_>>(),
        vec![
            "thread::chat-b",
            "thread::child-b",
            "thread::chat-a",
            "thread::child-a",
            "thread::grandchild-a"
        ]
    );
    assert_eq!(page.total, 5);
    assert_eq!(
        page.root_thread_ids,
        vec!["thread::chat-b".to_owned(), "thread::chat-a".to_owned()]
    );
    assert_eq!(page.skipped_pinned_thread_ids, vec!["thread::chat"]);
    let root_b = page
        .tasks
        .iter()
        .find(|node| node.thread_id() == "thread::chat-b")
        .expect("chat b root");
    match root_b {
        TaskForestNode::Thread {
            title,
            active_run_id,
            ..
        } => {
            assert_eq!(title, "Chat B");
            assert_eq!(active_run_id.as_deref(), Some("run::chat-b"));
        }
        TaskForestNode::Task { .. } => panic!("chat root should be a thread node"),
    }
    let child_a = page
        .tasks
        .iter()
        .find(|node| node.thread_id() == "thread::child-a")
        .expect("child a");
    assert_eq!(child_a.parent_thread_id(), Some("thread::chat-a"));
    assert_eq!(child_a.parent_node_id(), Some("thread-root:thread::chat-a"));
}

#[test]
fn pinned_task_forest_filters_inactive_tasks_and_reparents_active_descendants() {
    let db = GaryxDbService::memory().expect("db opens");
    db.replace_task_projection(task_projection_draft(
        "thread::done-parent",
        31,
        TaskStatus::Done,
        "2026-01-01T00:00:01.000Z",
        Some(chat_source("thread::chat-active")),
        1,
    ))
    .expect("insert done parent");
    db.replace_task_projection(task_projection_draft(
        "thread::todo-middle",
        32,
        TaskStatus::Todo,
        "2026-01-01T00:00:02.000Z",
        Some(thread_source("thread::done-parent", "#TASK-31")),
        1,
    ))
    .expect("insert todo middle");
    db.replace_task_projection(task_projection_draft(
        "thread::active-leaf",
        33,
        TaskStatus::InProgress,
        "2026-01-01T00:00:03.000Z",
        Some(thread_source("thread::todo-middle", "#TASK-32")),
        1,
    ))
    .expect("insert active leaf");
    db.replace_task_projection(task_projection_draft(
        "thread::review-child",
        34,
        TaskStatus::InReview,
        "2026-01-01T00:00:04.000Z",
        Some(thread_source("thread::active-leaf", "#TASK-33")),
        1,
    ))
    .expect("insert review child");
    db.replace_task_projection(task_projection_draft(
        "thread::done-child",
        35,
        TaskStatus::Done,
        "2026-01-01T00:00:05.000Z",
        Some(thread_source("thread::active-leaf", "#TASK-33")),
        1,
    ))
    .expect("insert done child");
    db.replace_task_projection(task_projection_draft(
        "thread::inactive-only",
        41,
        TaskStatus::Done,
        "2026-01-01T00:00:06.000Z",
        Some(chat_source("thread::chat-inactive")),
        1,
    ))
    .expect("insert inactive-only child");
    db.conn()
        .expect("db connection")
        .execute_batch(
            "INSERT INTO thread_pins (thread_id, pinned_at, sort_order)
             VALUES
               ('thread::chat-active', '2026-01-01T00:00:02.000Z', 1),
               ('thread::chat-inactive', '2026-01-01T00:00:01.000Z', 0)",
        )
        .expect("insert pins");

    let page = db
        .list_task_forest(
            &TaskListFilter {
                include_done: true,
                ..Default::default()
            },
            TaskForestScope::Pinned,
        )
        .expect("list pinned forest");

    assert_eq!(
        page.tasks
            .iter()
            .map(|node| node.thread_id())
            .collect::<Vec<_>>(),
        vec![
            "thread::chat-inactive",
            "thread::chat-active",
            "thread::active-leaf",
            "thread::review-child"
        ]
    );
    assert_eq!(
        page.root_thread_ids,
        vec![
            "thread::chat-inactive".to_owned(),
            "thread::chat-active".to_owned()
        ]
    );
    assert!(page.skipped_pinned_thread_ids.is_empty());
    for node in &page.tasks {
        if let TaskForestNode::Task { task, .. } = node {
            assert!(
                matches!(task.status, TaskStatus::InProgress | TaskStatus::InReview),
                "inactive task leaked into pinned forest: {:?}",
                task.status
            );
        }
    }
    let active_leaf = page
        .tasks
        .iter()
        .find(|node| node.thread_id() == "thread::active-leaf")
        .expect("active leaf");
    assert_eq!(active_leaf.parent_task_number(), None);
    assert_eq!(active_leaf.parent_thread_id(), Some("thread::chat-active"));
    assert_eq!(
        active_leaf.parent_node_id(),
        Some("thread-root:thread::chat-active")
    );
    let review_child = page
        .tasks
        .iter()
        .find(|node| node.thread_id() == "thread::review-child")
        .expect("review child");
    assert_eq!(review_child.parent_task_number(), Some(33));
    assert_eq!(review_child.parent_thread_id(), Some("thread::active-leaf"));
    assert_eq!(
        review_child.parent_node_id(),
        Some("task:thread::active-leaf")
    );
}

#[test]
fn pinned_task_forest_wire_json_regression_for_thread_root_helper() {
    let db = GaryxDbService::memory().expect("db opens");
    db.upsert_recent_thread(RecentThreadDraft {
        thread_id: "thread::wire-chat".to_owned(),
        title: "Wire Chat".to_owned(),
        workspace_dir: None,
        root_workspace_path: None,
        workspace_origin: None,
        thread_type: "chat".to_owned(),
        provider_type: Some("codex".to_owned()),
        agent_id: Some("codex".to_owned()),
        message_count: 2,
        last_message_preview: "Wire preview".to_owned(),
        recent_run_id: None,
        active_run_id: None,
        run_state: "idle".to_owned(),
        updated_at: Some("2026-01-01T00:00:01.000Z".to_owned()),
        last_active_at: "2026-01-01T00:00:01.000Z".to_owned(),
    })
    .expect("insert wire chat");
    db.replace_task_projection(task_projection_draft(
        "thread::wire-child",
        61,
        TaskStatus::InProgress,
        "2026-01-01T00:00:02.000Z",
        Some(chat_source("thread::wire-chat")),
        1,
    ))
    .expect("insert wire child");
    db.pin_thread("thread::wire-chat").expect("pin wire chat");

    let page = db
        .list_task_forest(
            &TaskListFilter {
                include_done: true,
                ..Default::default()
            },
            TaskForestScope::Pinned,
        )
        .expect("list pinned forest");
    let wire = serde_json::to_string(&page).expect("serialize pinned forest");

    assert_eq!(
        wire,
        r##"{"tasks":[{"kind":"thread","node_id":"thread-root:thread::wire-chat","thread_id":"thread::wire-chat","title":"Wire Chat","thread_type":"chat","provider_type":"codex","agent_id":"codex","message_count":2,"last_message_preview":"Wire preview","active_run_id":null,"run_state":"idle","updated_at":"2026-01-01T00:00:01.000Z","last_active_at":"2026-01-01T00:00:01.000Z"},{"kind":"task","node_id":"task:thread::wire-child","parent_node_id":"thread-root:thread::wire-chat","thread_id":"thread::wire-child","task_id":"#TASK-61","number":61,"title":"Task 61","status":"in_progress","creator":{"kind":"agent","agent_id":"test-agent"},"assignee":{"kind":"human","user_id":"1000000001"},"source":{"thread_id":"thread::wire-chat"},"updated_at":"2026-01-01T00:00:02Z","updated_by":{"kind":"agent","agent_id":"test-agent"},"runtime_agent_id":"","reply_count":0,"parent_task_number":null,"parent_thread_id":"thread::wire-chat","active_run_id":null,"run_state":"idle","last_active_at":null}],"total":2,"root_thread_ids":["thread::wire-chat"],"skipped_pinned_thread_ids":[]}"##
    );
}

#[test]
fn anchored_task_forest_climbs_to_root_and_keeps_done_ancestors() {
    let db = GaryxDbService::memory().expect("db opens");
    db.replace_task_projection(task_projection_draft(
        "thread::1254",
        1254,
        TaskStatus::InProgress,
        "2026-01-01T00:00:01.000Z",
        None,
        1,
    ))
    .expect("insert root");
    db.replace_task_projection(task_projection_draft(
        "thread::1261",
        1261,
        TaskStatus::InReview,
        "2026-01-01T00:00:02.000Z",
        Some(thread_source("thread::1254", "#TASK-1254")),
        1,
    ))
    .expect("insert active child");
    db.replace_task_projection(task_projection_draft(
        "thread::1262",
        1262,
        TaskStatus::Done,
        "2026-01-01T00:00:03.000Z",
        Some(thread_source("thread::1254", "#TASK-1254")),
        1,
    ))
    .expect("insert done structural child");
    db.replace_task_projection(task_projection_draft(
        "thread::1263",
        1263,
        TaskStatus::Done,
        "2026-01-01T00:00:04.000Z",
        Some(thread_source("thread::1254", "#TASK-1254")),
        1,
    ))
    .expect("insert done branch");
    db.replace_task_projection(task_projection_draft(
        "thread::1270",
        1270,
        TaskStatus::InProgress,
        "2026-01-01T00:00:05.000Z",
        Some(thread_source("thread::1262", "#TASK-1262")),
        1,
    ))
    .expect("insert active grandchild");
    db.replace_task_projection(task_projection_draft(
        "thread::1271",
        1271,
        TaskStatus::Done,
        "2026-01-01T00:00:06.000Z",
        Some(thread_source("thread::1263", "#TASK-1263")),
        1,
    ))
    .expect("insert done dead leaf");

    let page = db
        .list_task_forest_anchored("thread::1270", &TaskListFilter::default())
        .expect("anchored forest");

    assert_eq!(page.root_thread_ids, vec!["thread::1254"]);
    assert_eq!(page.skipped_pinned_thread_ids, Vec::<String>::new());
    assert_eq!(page.total, 6);
    assert_eq!(page.active_count, Some(3));
    // Full retention in DFS pre-order: the done 1263 branch and its done
    // leaf 1271 stay visible.
    assert_eq!(
        page.tasks
            .iter()
            .map(|node| node.thread_id())
            .collect::<Vec<_>>(),
        vec![
            "thread::1254",
            "thread::1261",
            "thread::1262",
            "thread::1270",
            "thread::1263",
            "thread::1271"
        ]
    );
    assert_eq!(
        page.tasks
            .iter()
            .map(|node| node.depth())
            .collect::<Vec<_>>(),
        vec![Some(0), Some(1), Some(1), Some(2), Some(1), Some(2)]
    );
    assert!(
        page.tasks
            .iter()
            .all(|node| matches!(node, TaskForestNode::Task { .. })),
        "origin-less anchored trees must not synthesize thread roots"
    );
    let active_grandchild = page
        .tasks
        .iter()
        .find(|node| node.thread_id() == "thread::1270")
        .expect("active grandchild");
    assert_eq!(active_grandchild.parent_task_number(), Some(1262));
    assert_eq!(active_grandchild.parent_thread_id(), Some("thread::1262"));
    assert_eq!(
        active_grandchild.parent_node_id(),
        Some("task:thread::1262")
    );
}

#[test]
fn anchored_task_forest_is_anchor_independent_across_the_tree() {
    let db = GaryxDbService::memory().expect("db opens");
    for (thread_id, number, status, source) in [
        ("thread::1254", 1254, TaskStatus::InProgress, None),
        (
            "thread::1261",
            1261,
            TaskStatus::InReview,
            Some(thread_source("thread::1254", "#TASK-1254")),
        ),
        (
            "thread::1262",
            1262,
            TaskStatus::Done,
            Some(thread_source("thread::1254", "#TASK-1254")),
        ),
        (
            "thread::1263",
            1263,
            TaskStatus::Done,
            Some(thread_source("thread::1254", "#TASK-1254")),
        ),
        (
            "thread::1270",
            1270,
            TaskStatus::InProgress,
            Some(thread_source("thread::1262", "#TASK-1262")),
        ),
        (
            "thread::1271",
            1271,
            TaskStatus::Done,
            Some(thread_source("thread::1263", "#TASK-1263")),
        ),
    ] {
        db.replace_task_projection(task_projection_draft(
            thread_id,
            number,
            status,
            &format!("2026-01-01T00:00:{:02}.000Z", number - 1250),
            source,
            1,
        ))
        .expect("insert task");
    }

    let expected_thread_ids = vec![
        "thread::1254",
        "thread::1261",
        "thread::1262",
        "thread::1270",
        "thread::1263",
        "thread::1271",
    ];
    // Every anchor inside the tree sees the identical forest; only the
    // client-side highlight differs.
    for anchor in ["thread::1254", "thread::1270", "thread::1271"] {
        let page = db
            .list_task_forest_anchored(anchor, &TaskListFilter::default())
            .expect("anchored forest");
        assert_eq!(page.total, 6, "anchor {anchor}");
        assert_eq!(page.active_count, Some(3), "anchor {anchor}");
        assert_eq!(
            page.root_thread_ids,
            vec!["thread::1254"],
            "anchor {anchor}"
        );
        assert_eq!(
            page.tasks
                .iter()
                .map(|node| node.thread_id())
                .collect::<Vec<_>>(),
            expected_thread_ids,
            "anchor {anchor}"
        );
        let current_dead_leaf = page
            .tasks
            .iter()
            .find(|node| node.thread_id() == "thread::1271")
            .expect("done dead leaf");
        assert_eq!(current_dead_leaf.parent_thread_id(), Some("thread::1263"));
    }
}

#[test]
fn anchored_task_forest_keeps_all_done_trees_and_hides_bare_threads() {
    let db = GaryxDbService::memory().expect("db opens");
    db.replace_task_projection(task_projection_draft(
        "thread::done-root",
        200,
        TaskStatus::Done,
        "2026-01-01T00:00:01.000Z",
        None,
        1,
    ))
    .expect("insert done root");
    db.replace_task_projection(task_projection_draft(
        "thread::done-child",
        201,
        TaskStatus::Done,
        "2026-01-01T00:00:02.000Z",
        Some(thread_source("thread::done-root", "#TASK-200")),
        1,
    ))
    .expect("insert done child");

    // All-done trees stay visible now; only the active badge drops to 0.
    let all_done = db
        .list_task_forest_anchored("thread::done-child", &TaskListFilter::default())
        .expect("all done anchored forest");
    assert_eq!(
        all_done
            .tasks
            .iter()
            .map(|node| node.thread_id())
            .collect::<Vec<_>>(),
        vec!["thread::done-root", "thread::done-child"]
    );
    assert_eq!(all_done.total, 2);
    assert_eq!(all_done.active_count, Some(0));
    assert_eq!(all_done.root_thread_ids, vec!["thread::done-root"]);

    let bare = db
        .list_task_forest_anchored("thread::bare", &TaskListFilter::default())
        .expect("bare anchored forest");
    assert!(bare.tasks.is_empty());
    assert!(bare.root_thread_ids.is_empty());
    assert_eq!(bare.active_count, Some(0));
}

#[test]
fn anchored_task_forest_ignores_caller_filters_for_raw_tree() {
    let db = GaryxDbService::memory().expect("db opens");
    let target_creator = Principal::Agent {
        agent_id: "target-creator".to_owned(),
    };
    let target_assignee = Principal::Human {
        user_id: "1000000002".to_owned(),
    };

    db.replace_task_projection(task_projection_draft(
        "thread::filter-root",
        300,
        TaskStatus::Done,
        "2026-01-01T00:00:01.000Z",
        None,
        1,
    ))
    .expect("insert root");
    db.replace_task_projection(task_projection_draft(
        "thread::filter-sibling",
        301,
        TaskStatus::InReview,
        "2026-01-01T00:00:02.000Z",
        Some(thread_source("thread::filter-root", "#TASK-300")),
        1,
    ))
    .expect("insert sibling");
    db.replace_task_projection(task_projection_draft(
        "thread::filter-child",
        302,
        TaskStatus::InProgress,
        "2026-01-01T00:00:03.000Z",
        Some(thread_source("thread::filter-root", "#TASK-300")),
        1,
    ))
    .expect("insert child");
    db.replace_task_projection(with_assignee(
        with_creator(
            task_projection_draft(
                "thread::filter-leaf",
                303,
                TaskStatus::InProgress,
                "2026-01-01T00:00:04.000Z",
                Some(bot_thread_source(
                    "thread::filter-child",
                    "#TASK-302",
                    "api:target",
                )),
                1,
            ),
            &target_creator,
        ),
        &target_assignee,
    ))
    .expect("insert leaf");

    let page = db
        .list_task_forest_anchored(
            "thread::filter-leaf",
            &TaskListFilter {
                status: Some(TaskStatus::Done),
                assignee: Some(target_assignee),
                creator: Some(target_creator),
                source_thread_id: Some("thread::filter-child".to_owned()),
                source_task_id: Some("#TASK-302".to_owned()),
                source_bot_id: Some("api:target".to_owned()),
                include_done: false,
                limit: Some(1),
                offset: Some(99),
            },
        )
        .expect("anchored forest");

    assert_eq!(page.root_thread_ids, vec!["thread::filter-root"]);
    assert_eq!(page.total, 4);
    assert_eq!(page.active_count, Some(3));
    assert_eq!(
        page.tasks
            .iter()
            .map(|node| node.thread_id())
            .collect::<Vec<_>>(),
        vec![
            "thread::filter-root",
            "thread::filter-child",
            "thread::filter-leaf",
            "thread::filter-sibling",
        ]
    );
    assert_eq!(
        page.tasks
            .iter()
            .map(|node| node.depth())
            .collect::<Vec<_>>(),
        vec![Some(0), Some(1), Some(2), Some(1)]
    );
    let leaf = page
        .tasks
        .iter()
        .find(|node| node.thread_id() == "thread::filter-leaf")
        .expect("leaf");
    assert_eq!(leaf.parent_task_number(), Some(302));
    assert_eq!(leaf.parent_thread_id(), Some("thread::filter-child"));
}

#[test]
fn anchored_task_forest_source_thread_anchor_returns_thread_root_and_subtree() {
    let db = GaryxDbService::memory().expect("db opens");
    db.upsert_recent_thread(RecentThreadDraft {
        thread_id: "thread::origin-chat".to_owned(),
        title: "Origin chat".to_owned(),
        workspace_dir: None,
        root_workspace_path: None,
        workspace_origin: None,
        thread_type: "chat".to_owned(),
        provider_type: Some("codex".to_owned()),
        agent_id: Some("codex".to_owned()),
        message_count: 8,
        last_message_preview: "Spawned task work".to_owned(),
        recent_run_id: None,
        active_run_id: None,
        run_state: "idle".to_owned(),
        updated_at: Some("2026-01-01T00:00:00.500Z".to_owned()),
        last_active_at: "2026-01-01T00:00:00.500Z".to_owned(),
    })
    .expect("insert origin chat");
    db.replace_task_projection(task_projection_draft(
        "thread::derived-root",
        401,
        TaskStatus::InProgress,
        "2026-01-01T00:00:01.000Z",
        Some(chat_source("thread::origin-chat")),
        1,
    ))
    .expect("insert derived root");
    db.replace_task_projection(task_projection_draft(
        "thread::derived-child",
        402,
        TaskStatus::InReview,
        "2026-01-01T00:00:02.000Z",
        Some(thread_source("thread::derived-root", "#TASK-401")),
        1,
    ))
    .expect("insert derived child");
    db.replace_task_projection(task_projection_draft(
        "thread::other-derived",
        499,
        TaskStatus::InProgress,
        "2026-01-01T00:00:03.000Z",
        Some(chat_source("thread::other-chat")),
        1,
    ))
    .expect("insert other conversation task");

    let page = db
        .list_task_forest_anchored(
            "thread::origin-chat",
            &TaskListFilter {
                status: Some(TaskStatus::Done),
                assignee: Some(Principal::Human {
                    user_id: "1000000009".to_owned(),
                }),
                creator: Some(Principal::Agent {
                    agent_id: "hostile-creator".to_owned(),
                }),
                source_thread_id: Some("thread::other-chat".to_owned()),
                source_task_id: Some("#TASK-499".to_owned()),
                source_bot_id: Some("api:hostile".to_owned()),
                include_done: false,
                limit: Some(1),
                offset: Some(99),
            },
        )
        .expect("source-thread anchored forest");

    assert_eq!(page.root_thread_ids, vec!["thread::origin-chat"]);
    assert_eq!(page.skipped_pinned_thread_ids, Vec::<String>::new());
    assert_eq!(page.total, 3);
    assert_eq!(page.active_count, Some(2));
    assert_eq!(
        page.tasks
            .iter()
            .map(|node| node.thread_id())
            .collect::<Vec<_>>(),
        vec![
            "thread::origin-chat",
            "thread::derived-root",
            "thread::derived-child",
        ]
    );
    assert_eq!(
        page.tasks
            .iter()
            .map(|node| node.depth())
            .collect::<Vec<_>>(),
        vec![Some(0), Some(0), Some(1)]
    );
    match &page.tasks[0] {
        TaskForestNode::Thread { title, .. } => assert_eq!(title, "Origin chat"),
        TaskForestNode::Task { .. } => panic!("source-thread anchor must prepend thread root"),
    }
    let derived_root = page
        .tasks
        .iter()
        .find(|node| node.thread_id() == "thread::derived-root")
        .expect("derived root");
    assert_eq!(
        derived_root.parent_node_id(),
        Some("thread-root:thread::origin-chat")
    );
    assert_eq!(derived_root.parent_thread_id(), Some("thread::origin-chat"));
    assert_eq!(derived_root.parent_task_number(), None);
    let derived_child = page
        .tasks
        .iter()
        .find(|node| node.thread_id() == "thread::derived-child")
        .expect("derived child");
    assert_eq!(
        derived_child.parent_node_id(),
        Some("task:thread::derived-root")
    );
    assert_eq!(
        derived_child.parent_thread_id(),
        Some("thread::derived-root")
    );
    assert_eq!(derived_child.parent_task_number(), Some(401));
    assert!(
        !page
            .tasks
            .iter()
            .any(|node| node.thread_id() == "thread::other-derived"),
        "source-thread anchor must exclude tasks from other conversations"
    );

    // Task anchors resolve the same origin-rooted forest: hydrated thread
    // root first, identical node set, whether anchored on the root task or
    // a deep descendant.
    let expected_node_ids = page
        .tasks
        .iter()
        .map(|node| node.thread_id().to_owned())
        .collect::<Vec<_>>();
    for anchor in ["thread::derived-root", "thread::derived-child"] {
        let task_anchor = db
            .list_task_forest_anchored(anchor, &TaskListFilter::default())
            .expect("task anchored forest");
        assert_eq!(
            task_anchor
                .tasks
                .iter()
                .map(|node| node.thread_id().to_owned())
                .collect::<Vec<_>>(),
            expected_node_ids,
            "anchor {anchor}"
        );
        assert_eq!(task_anchor.root_thread_ids, vec!["thread::origin-chat"]);
        assert_eq!(task_anchor.active_count, Some(2));
        match &task_anchor.tasks[0] {
            TaskForestNode::Thread { title, depth, .. } => {
                assert_eq!(title, "Origin chat");
                assert_eq!(*depth, Some(0));
            }
            TaskForestNode::Task { .. } => {
                panic!("task anchor with known origin must prepend the thread root")
            }
        }
    }

    db.replace_task_projection(task_projection_draft(
        "thread::done-derived",
        501,
        TaskStatus::Done,
        "2026-01-01T00:00:04.000Z",
        Some(chat_source("thread::done-chat")),
        1,
    ))
    .expect("insert done source-thread root");
    db.replace_task_projection(task_projection_draft(
        "thread::done-derived-child",
        502,
        TaskStatus::Done,
        "2026-01-01T00:00:05.000Z",
        Some(thread_source("thread::done-derived", "#TASK-501")),
        1,
    ))
    .expect("insert done source-thread child");
    // All-done derived trees remain visible behind their thread root.
    let done_page = db
        .list_task_forest_anchored("thread::done-chat", &TaskListFilter::default())
        .expect("done source-thread anchored forest");
    assert_eq!(done_page.total, 3);
    assert_eq!(done_page.active_count, Some(0));
    assert_eq!(done_page.root_thread_ids, vec!["thread::done-chat"]);
    assert_eq!(
        done_page
            .tasks
            .iter()
            .map(|node| node.thread_id())
            .collect::<Vec<_>>(),
        vec![
            "thread::done-chat",
            "thread::done-derived",
            "thread::done-derived-child"
        ]
    );
}

#[test]
fn pinned_task_forest_prefers_pinned_seed_over_newer_duplicate_number() {
    let db = GaryxDbService::memory().expect("db opens");
    db.replace_task_projection(task_projection_draft(
        "thread::pinned-direct",
        1,
        TaskStatus::InProgress,
        "2026-01-01T00:00:01.000Z",
        Some(chat_source("thread::pinned-chat")),
        1,
    ))
    .expect("insert pinned direct duplicate");
    db.replace_task_projection(task_projection_draft(
        "thread::newer-duplicate",
        1,
        TaskStatus::InReview,
        "2026-01-01T00:00:02.000Z",
        None,
        2,
    ))
    .expect("insert newer duplicate");
    db.replace_task_projection(task_projection_draft(
        "thread::child",
        2,
        TaskStatus::Todo,
        "2026-01-01T00:00:03.000Z",
        Some(thread_source("thread::pinned-direct", "#TASK-1")),
        1,
    ))
    .expect("insert child");
    db.pin_thread("thread::pinned-chat").expect("pin chat");

    let page = db
        .list_task_forest(
            &TaskListFilter {
                include_done: true,
                ..Default::default()
            },
            TaskForestScope::Pinned,
        )
        .expect("list pinned forest");

    assert_eq!(
        page.tasks
            .iter()
            .map(|node| node.thread_id())
            .collect::<Vec<_>>(),
        vec!["thread::pinned-chat", "thread::pinned-direct"]
    );
    assert_eq!(page.root_thread_ids, vec!["thread::pinned-chat"]);
    assert!(
        !page
            .tasks
            .iter()
            .any(|node| node.thread_id() == "thread::newer-duplicate")
    );
}

#[test]
fn pinned_task_forest_skips_only_pins_without_any_projection() {
    let db = GaryxDbService::memory().expect("db opens");
    db.replace_task_projection(task_projection_draft(
        "thread::other-bot-direct",
        1,
        TaskStatus::InProgress,
        "2026-01-01T00:00:01.000Z",
        Some(TaskSource {
            thread_id: Some("thread::other-bot-chat".to_owned()),
            task_id: None,
            task_thread_id: None,
            bot_id: Some("api:other".to_owned()),
            channel: None,
            account_id: None,
        }),
        1,
    ))
    .expect("insert other bot root");
    db.replace_task_projection(task_projection_draft(
        "thread::main-child",
        2,
        TaskStatus::InProgress,
        "2026-01-01T00:00:02.000Z",
        Some(TaskSource {
            thread_id: Some("thread::other-bot-chat".to_owned()),
            task_id: Some("#TASK-1".to_owned()),
            task_thread_id: Some("thread::other-bot-direct".to_owned()),
            bot_id: Some("api:main".to_owned()),
            channel: None,
            account_id: None,
        }),
        1,
    ))
    .expect("insert child");
    db.pin_thread("thread::other-bot-chat")
        .expect("pin filtered chat");
    db.pin_thread("thread::chat").expect("pin chat");

    let page = db
        .list_task_forest(
            &TaskListFilter {
                include_done: true,
                source_bot_id: Some("api:main".to_owned()),
                ..Default::default()
            },
            TaskForestScope::Pinned,
        )
        .expect("list filtered pinned forest");

    assert!(page.tasks.is_empty());
    assert!(page.root_thread_ids.is_empty());
    assert_eq!(page.skipped_pinned_thread_ids, vec!["thread::chat"]);
}

#[test]
fn anchored_forest_resolves_legacy_task_ref_parent_without_parent_number() {
    // Legacy projection rows can carry a `#TASK-n` source_task_id without a
    // derived parent_task_number; the anchored forest must resolve that
    // parent through the string fallback edge (#TASK-1956 rewrite guard).
    let db = GaryxDbService::memory().expect("db opens");
    db.replace_task_projection(task_projection_draft(
        "thread::p2root",
        7,
        TaskStatus::InProgress,
        "2026-01-01T00:00:01.000Z",
        Some(TaskSource {
            thread_id: Some("thread::origin".to_owned()),
            task_id: None,
            task_thread_id: None,
            bot_id: None,
            channel: None,
            account_id: None,
        }),
        1,
    ))
    .expect("insert origin-seeded root");

    // Fallback child: no parent_task_number, lowercase task ref, and no
    // source_task_thread_id so the string edge is its only way into the tree.
    let mut legacy_child = task_projection_draft(
        "thread::p2legacy",
        8,
        TaskStatus::InProgress,
        "2026-01-01T00:00:02.000Z",
        Some(TaskSource {
            thread_id: None,
            task_id: Some("#task-7".to_owned()),
            task_thread_id: None,
            bot_id: None,
            channel: None,
            account_id: None,
        }),
        1,
    );
    legacy_child.parent_task_number = None;
    legacy_child.source_task_thread_id = None;
    db.replace_task_projection(legacy_child)
        .expect("insert legacy fallback child");

    // Zero-padded ref must NOT match number 7: the historical contract is
    // exact string equality against '#TASK-' || number (NOCASE only).
    let mut padded_orphan = task_projection_draft(
        "thread::p2padded",
        9,
        TaskStatus::InProgress,
        "2026-01-01T00:00:03.000Z",
        Some(TaskSource {
            thread_id: None,
            task_id: Some("#TASK-007".to_owned()),
            task_thread_id: None,
            bot_id: None,
            channel: None,
            account_id: None,
        }),
        1,
    );
    padded_orphan.parent_task_number = None;
    padded_orphan.source_task_thread_id = None;
    db.replace_task_projection(padded_orphan)
        .expect("insert zero-padded orphan");

    let page = db
        .list_task_forest_anchored("thread::origin", &TaskListFilter::default())
        .expect("anchored forest");

    assert_eq!(
        page.tasks
            .iter()
            .map(|node| node.thread_id())
            .collect::<Vec<_>>(),
        vec!["thread::origin", "thread::p2root", "thread::p2legacy"],
        "lowercase legacy ref hangs under its parent; padded ref stays out"
    );
    let legacy = page
        .tasks
        .iter()
        .find(|node| node.thread_id() == "thread::p2legacy")
        .expect("legacy child present");
    assert_eq!(legacy.parent_thread_id(), Some("thread::p2root"));
    // Depth convention (pre-existing): thread root and seeds both 0,
    // descendants count from the seed.
    assert_eq!(
        page.tasks
            .iter()
            .map(|node| node.depth())
            .collect::<Vec<_>>(),
        vec![Some(0), Some(0), Some(1)]
    );
}
