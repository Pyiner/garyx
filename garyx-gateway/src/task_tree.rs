use std::collections::{BTreeSet, HashMap};

use garyx_models::TaskStatus;

use crate::garyx_db::TaskForestNode;

#[derive(Debug, Clone)]
pub(crate) struct RawTaskNode {
    pub(crate) thread_id: String,
    pub(crate) number: u64,
    pub(crate) status: TaskStatus,
    pub(crate) parent_task_number: Option<u64>,
    pub(crate) source_task_number: Option<u64>,
    pub(crate) source_task_thread_id: Option<String>,
    pub(crate) node: TaskForestNode,
}

impl RawTaskNode {
    fn is_active(&self) -> bool {
        matches!(self.status, TaskStatus::InProgress | TaskStatus::InReview)
    }
}

pub(crate) fn prune_anchored_task_tree(
    raw: Vec<RawTaskNode>,
    anchor_thread_id: &str,
) -> Vec<TaskForestNode> {
    if raw.is_empty() {
        return Vec::new();
    }

    let mut by_number = HashMap::new();
    let mut by_thread = HashMap::new();
    for (index, node) in raw.iter().enumerate() {
        by_number.entry(node.number).or_insert(index);
        by_thread.entry(node.thread_id.clone()).or_insert(index);
    }

    let anchor_is_task = by_thread.contains_key(anchor_thread_id);
    let active_indices = raw
        .iter()
        .enumerate()
        .filter_map(|(index, node)| node.is_active().then_some(index))
        .collect::<Vec<_>>();
    if active_indices.is_empty() {
        return Vec::new();
    }

    let parent_indices = raw
        .iter()
        .map(|node| immediate_parent_index(node, &by_number, &by_thread))
        .collect::<Vec<_>>();

    let mut retained = BTreeSet::new();
    for index in active_indices {
        retain_path(index, &parent_indices, &mut retained);
    }
    if anchor_is_task {
        let anchor_index = by_thread
            .get(anchor_thread_id)
            .copied()
            .expect("anchor_is_task was derived from by_thread");
        retain_path(anchor_index, &parent_indices, &mut retained);
    }

    raw.iter()
        .enumerate()
        .filter_map(|(index, row)| {
            if !retained.contains(&index) {
                return None;
            }
            let mut node = row.node.clone();
            let parent_task = parent_indices[index]
                .filter(|parent_index| retained.contains(parent_index))
                .map(|parent_index| &raw[parent_index]);
            let parent = match parent_task {
                Some(parent) => ResolvedParent::Task(parent),
                None if !anchor_is_task => ResolvedParent::Thread {
                    node_id: thread_root_node_id(anchor_thread_id),
                    thread_id: anchor_thread_id.to_owned(),
                },
                None => ResolvedParent::None,
            };
            set_original_parent(&mut node, parent);
            Some(node)
        })
        .collect()
}

fn immediate_parent_index(
    node: &RawTaskNode,
    by_number: &HashMap<u64, usize>,
    by_thread: &HashMap<String, usize>,
) -> Option<usize> {
    node.parent_task_number
        .or(node.source_task_number)
        .and_then(|number| by_number.get(&number).copied())
        .or_else(|| {
            node.source_task_thread_id
                .as_ref()
                .and_then(|thread_id| by_thread.get(thread_id).copied())
        })
}

fn retain_path(
    start_index: usize,
    parent_indices: &[Option<usize>],
    retained: &mut BTreeSet<usize>,
) {
    let mut seen = BTreeSet::new();
    let mut current = Some(start_index);
    while let Some(index) = current {
        if index >= parent_indices.len() || !seen.insert(index) {
            break;
        }
        retained.insert(index);
        current = parent_indices[index];
    }
}

enum ResolvedParent<'a> {
    Task(&'a RawTaskNode),
    Thread { node_id: String, thread_id: String },
    None,
}

fn set_original_parent(node: &mut TaskForestNode, parent: ResolvedParent<'_>) {
    let TaskForestNode::Task {
        parent_node_id,
        parent_task_number,
        parent_thread_id,
        ..
    } = node
    else {
        return;
    };
    match parent {
        ResolvedParent::Task(parent) => {
            *parent_node_id = Some(task_node_id(&parent.thread_id));
            *parent_task_number = Some(parent.number);
            *parent_thread_id = Some(parent.thread_id.clone());
        }
        ResolvedParent::Thread { node_id, thread_id } => {
            *parent_node_id = Some(node_id);
            *parent_task_number = None;
            *parent_thread_id = Some(thread_id);
        }
        ResolvedParent::None => {
            *parent_node_id = None;
            *parent_task_number = None;
            *parent_thread_id = None;
        }
    }
}

pub(crate) fn task_node_id(thread_id: &str) -> String {
    format!("task:{thread_id}")
}

pub(crate) fn thread_root_node_id(thread_id: &str) -> String {
    format!("thread-root:{thread_id}")
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use garyx_models::{Principal, TaskStatus};
    use garyx_router::tasks::TaskSummary;

    use super::*;

    fn principal() -> Principal {
        Principal::Agent {
            agent_id: "test-agent".to_owned(),
        }
    }

    fn updated_at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-01-01T00:00:00.000Z")
            .expect("timestamp")
            .with_timezone(&Utc)
    }

    fn node(number: u64, status: TaskStatus, parent: Option<u64>) -> RawTaskNode {
        custom_node(
            number,
            status,
            parent,
            parent,
            parent.map(|number| format!("thread::{number}")),
        )
    }

    fn custom_node(
        number: u64,
        status: TaskStatus,
        parent_task_number: Option<u64>,
        source_task_number: Option<u64>,
        source_task_thread_id: Option<String>,
    ) -> RawTaskNode {
        let thread_id = format!("thread::{number}");
        let task = TaskSummary {
            thread_id: thread_id.clone(),
            task_id: format!("#TASK-{number}"),
            number,
            title: format!("Task {number}"),
            status,
            creator: principal(),
            assignee: Some(principal()),
            source: None,
            executor: None,
            updated_at: updated_at(),
            updated_by: principal(),
            runtime_agent_id: "test-agent".to_owned(),
            reply_count: 0,
        };
        RawTaskNode {
            thread_id: thread_id.clone(),
            number,
            status,
            parent_task_number,
            source_task_number,
            source_task_thread_id: source_task_thread_id.clone(),
            node: TaskForestNode::Task {
                node_id: task_node_id(&thread_id),
                parent_node_id: source_task_thread_id.as_deref().map(task_node_id),
                task,
                parent_task_number,
                parent_thread_id: source_task_thread_id,
                active_run_id: None,
                run_state: "idle".to_owned(),
                last_active_at: None,
            },
        }
    }

    fn numbers(nodes: &[TaskForestNode]) -> Vec<u64> {
        nodes
            .iter()
            .map(|node| match node {
                TaskForestNode::Task { task, .. } => task.number,
                TaskForestNode::Thread { .. } => panic!("anchored tree must be task-only"),
            })
            .collect()
    }

    fn active_count(nodes: &[TaskForestNode]) -> usize {
        nodes
            .iter()
            .filter(|node| match node {
                TaskForestNode::Task { task, .. } => {
                    matches!(task.status, TaskStatus::InProgress | TaskStatus::InReview)
                }
                TaskForestNode::Thread { .. } => false,
            })
            .count()
    }

    fn parent_node_id(nodes: &[TaskForestNode], number: u64) -> Option<String> {
        nodes.iter().find_map(|node| match node {
            TaskForestNode::Task {
                task,
                parent_node_id,
                ..
            } if task.number == number => parent_node_id.clone(),
            _ => None,
        })
    }

    fn parent_thread_id(nodes: &[TaskForestNode], number: u64) -> Option<String> {
        nodes.iter().find_map(|node| match node {
            TaskForestNode::Task {
                task,
                parent_thread_id,
                ..
            } if task.number == number => parent_thread_id.clone(),
            _ => None,
        })
    }

    fn parent_task_number(nodes: &[TaskForestNode], number: u64) -> Option<u64> {
        nodes.iter().find_map(|node| match node {
            TaskForestNode::Task {
                task,
                parent_task_number,
                ..
            } if task.number == number => *parent_task_number,
            _ => None,
        })
    }

    #[test]
    fn scenario_01_chain_current_root() {
        let out = prune_anchored_task_tree(
            vec![
                node(1254, TaskStatus::InProgress, None),
                node(1261, TaskStatus::InProgress, Some(1254)),
            ],
            "thread::1254",
        );
        assert_eq!(numbers(&out), vec![1254, 1261]);
        assert_eq!(active_count(&out), 2);
    }

    #[test]
    fn scenario_02_chain_current_child_is_stable() {
        let out = prune_anchored_task_tree(
            vec![
                node(1254, TaskStatus::InProgress, None),
                node(1261, TaskStatus::InProgress, Some(1254)),
            ],
            "thread::1261",
        );
        assert_eq!(numbers(&out), vec![1254, 1261]);
        assert_eq!(active_count(&out), 2);
    }

    #[test]
    fn scenario_03_done_ancestor_is_retained() {
        let out = prune_anchored_task_tree(
            vec![
                node(1254, TaskStatus::Done, None),
                node(1261, TaskStatus::InReview, Some(1254)),
            ],
            "thread::1261",
        );
        assert_eq!(numbers(&out), vec![1254, 1261]);
        assert_eq!(active_count(&out), 1);
    }

    #[test]
    fn scenario_04_done_leaf_is_pruned() {
        let out = prune_anchored_task_tree(
            vec![
                node(1254, TaskStatus::InProgress, None),
                node(1261, TaskStatus::InReview, Some(1254)),
                node(1262, TaskStatus::Done, Some(1254)),
            ],
            "thread::1254",
        );
        assert_eq!(numbers(&out), vec![1254, 1261]);
        assert_eq!(active_count(&out), 2);
    }

    #[test]
    fn scenario_05_done_middle_is_structural() {
        let out = prune_anchored_task_tree(
            vec![
                node(1254, TaskStatus::InProgress, None),
                node(1300, TaskStatus::Done, Some(1254)),
                node(1305, TaskStatus::InProgress, Some(1300)),
            ],
            "thread::1305",
        );
        assert_eq!(numbers(&out), vec![1254, 1300, 1305]);
        assert_eq!(
            parent_node_id(&out, 1305).as_deref(),
            Some("task:thread::1300")
        );
        assert!(
            out.iter().all(|node| match node {
                TaskForestNode::Task { parent_node_id, .. } => !parent_node_id
                    .as_deref()
                    .unwrap_or_default()
                    .starts_with("thread-root:"),
                TaskForestNode::Thread { .. } => true,
            }),
            "task anchors must not point at synthetic thread roots"
        );
        assert_eq!(active_count(&out), 2);
    }

    fn mixed_branch_raw() -> Vec<RawTaskNode> {
        vec![
            node(1254, TaskStatus::InProgress, None),
            node(1261, TaskStatus::InReview, Some(1254)),
            node(1262, TaskStatus::Done, Some(1254)),
            node(1263, TaskStatus::Done, Some(1254)),
            node(1270, TaskStatus::InProgress, Some(1262)),
            node(1271, TaskStatus::Done, Some(1263)),
        ]
    }

    #[test]
    fn scenario_06_mixed_branch_prunes_dead_branch() {
        let out = prune_anchored_task_tree(mixed_branch_raw(), "thread::1270");
        assert_eq!(numbers(&out), vec![1254, 1261, 1262, 1270]);
        assert_eq!(
            parent_node_id(&out, 1270).as_deref(),
            Some("task:thread::1262")
        );
        assert_eq!(active_count(&out), 3);
    }

    #[test]
    fn scenario_07_plan_b_keeps_current_dead_branch_path() {
        let out = prune_anchored_task_tree(mixed_branch_raw(), "thread::1271");
        assert_eq!(numbers(&out), vec![1254, 1261, 1262, 1263, 1270, 1271]);
        assert_eq!(
            parent_node_id(&out, 1271).as_deref(),
            Some("task:thread::1263")
        );
        assert_eq!(active_count(&out), 3);
    }

    #[test]
    fn scenario_08_all_done_is_empty() {
        let out = prune_anchored_task_tree(
            vec![
                node(1254, TaskStatus::Done, None),
                node(1261, TaskStatus::Done, Some(1254)),
            ],
            "thread::1254",
        );
        assert!(out.is_empty());
    }

    #[test]
    fn scenario_09_bare_anchor_is_empty() {
        let out = prune_anchored_task_tree(Vec::new(), "thread::bare");
        assert!(out.is_empty());
    }

    #[test]
    fn scenario_10_active_root_only() {
        let out = prune_anchored_task_tree(
            vec![node(1280, TaskStatus::InProgress, None)],
            "thread::1280",
        );
        assert_eq!(numbers(&out), vec![1280]);
        assert_eq!(active_count(&out), 1);
    }

    #[test]
    fn parent_priority_matches_existing_task_forest_order() {
        let out = prune_anchored_task_tree(
            vec![
                node(1, TaskStatus::Done, None),
                node(2, TaskStatus::Done, None),
                node(3, TaskStatus::Done, None),
                custom_node(
                    4,
                    TaskStatus::InProgress,
                    Some(1),
                    Some(2),
                    Some("thread::3".to_owned()),
                ),
                custom_node(
                    5,
                    TaskStatus::InProgress,
                    None,
                    Some(2),
                    Some("thread::3".to_owned()),
                ),
                custom_node(
                    6,
                    TaskStatus::InProgress,
                    None,
                    Some(99),
                    Some("thread::3".to_owned()),
                ),
            ],
            "thread::4",
        );

        assert_eq!(parent_node_id(&out, 4).as_deref(), Some("task:thread::1"));
        assert_eq!(parent_node_id(&out, 5).as_deref(), Some("task:thread::2"));
        assert_eq!(parent_node_id(&out, 6).as_deref(), Some("task:thread::3"));
    }

    #[test]
    fn conversation_anchor_single_derived_root_points_to_thread_root() {
        let out = prune_anchored_task_tree(
            vec![node(1400, TaskStatus::InProgress, None)],
            "thread::conversation",
        );

        assert_eq!(numbers(&out), vec![1400]);
        assert_eq!(
            parent_node_id(&out, 1400).as_deref(),
            Some("thread-root:thread::conversation")
        );
        assert_eq!(
            parent_thread_id(&out, 1400).as_deref(),
            Some("thread::conversation")
        );
        assert_eq!(parent_task_number(&out, 1400), None);
    }

    #[test]
    fn conversation_anchor_keeps_multi_level_task_parents() {
        let out = prune_anchored_task_tree(
            vec![
                node(1400, TaskStatus::InProgress, None),
                node(1401, TaskStatus::InProgress, Some(1400)),
                node(1402, TaskStatus::InReview, Some(1401)),
            ],
            "thread::conversation",
        );

        assert_eq!(numbers(&out), vec![1400, 1401, 1402]);
        assert_eq!(
            parent_node_id(&out, 1400).as_deref(),
            Some("thread-root:thread::conversation")
        );
        assert_eq!(
            parent_node_id(&out, 1401).as_deref(),
            Some("task:thread::1400")
        );
        assert_eq!(
            parent_node_id(&out, 1402).as_deref(),
            Some("task:thread::1401")
        );
    }

    #[test]
    fn conversation_anchor_retains_done_ancestor_but_prunes_done_leaf() {
        let done_ancestor = prune_anchored_task_tree(
            vec![
                node(1400, TaskStatus::Done, None),
                node(1401, TaskStatus::InProgress, Some(1400)),
            ],
            "thread::conversation",
        );
        assert_eq!(numbers(&done_ancestor), vec![1400, 1401]);
        assert_eq!(
            parent_node_id(&done_ancestor, 1400).as_deref(),
            Some("thread-root:thread::conversation")
        );
        assert_eq!(active_count(&done_ancestor), 1);

        let done_leaf = prune_anchored_task_tree(
            vec![
                node(1400, TaskStatus::InProgress, None),
                node(1401, TaskStatus::Done, Some(1400)),
            ],
            "thread::conversation",
        );
        assert_eq!(numbers(&done_leaf), vec![1400]);
    }

    #[test]
    fn conversation_anchor_all_done_is_empty_without_thread_node() {
        let out = prune_anchored_task_tree(
            vec![
                node(1400, TaskStatus::Done, None),
                node(1401, TaskStatus::Done, Some(1400)),
            ],
            "thread::conversation",
        );

        assert!(out.is_empty());
    }

    #[test]
    fn conversation_anchor_supports_multiple_derived_roots() {
        let out = prune_anchored_task_tree(
            vec![
                node(1400, TaskStatus::InProgress, None),
                node(1500, TaskStatus::InReview, None),
            ],
            "thread::conversation",
        );

        assert_eq!(numbers(&out), vec![1400, 1500]);
        assert_eq!(
            parent_node_id(&out, 1400).as_deref(),
            Some("thread-root:thread::conversation")
        );
        assert_eq!(
            parent_node_id(&out, 1500).as_deref(),
            Some("thread-root:thread::conversation")
        );
    }
}
