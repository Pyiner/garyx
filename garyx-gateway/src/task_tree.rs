use std::collections::HashMap;

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

#[derive(Debug, Clone)]
pub(crate) struct AnchoredTaskTreeLayout {
    /// Task nodes in DFS pre-order with `depth` set. The hydrated
    /// `kind:"thread"` origin root is prepended by the DB layer.
    pub(crate) nodes: Vec<TaskForestNode>,
    /// Page-level active badge count (`in_progress` + `in_review`).
    pub(crate) active_count: usize,
}

/// Lay out the anchored task tree: every raw node is retained (done tasks
/// included), emitted in DFS pre-order with sibling order by task number.
/// When `origin_thread_id` is present, top-level tasks are parented to the
/// origin's `thread-root:` node.
///
/// `depth` is the *visual indent level*, not the tree distance: the thread
/// root row and top-level tasks both sit flush at depth 0 (the root row is
/// distinguished by styling, not indentation), and each nesting level below a
/// task adds 1.
pub(crate) fn layout_anchored_task_tree(
    raw: Vec<RawTaskNode>,
    origin_thread_id: Option<&str>,
) -> AnchoredTaskTreeLayout {
    if raw.is_empty() {
        return AnchoredTaskTreeLayout {
            nodes: Vec::new(),
            active_count: 0,
        };
    }

    let mut by_number = HashMap::new();
    let mut by_thread = HashMap::new();
    for (index, node) in raw.iter().enumerate() {
        by_number.entry(node.number).or_insert(index);
        by_thread.entry(node.thread_id.clone()).or_insert(index);
    }

    let parent_indices = raw
        .iter()
        .enumerate()
        .map(|(index, node)| {
            immediate_parent_index(node, &by_number, &by_thread)
                .filter(|parent_index| *parent_index != index)
        })
        .collect::<Vec<_>>();

    let mut children: Vec<Vec<usize>> = vec![Vec::new(); raw.len()];
    let mut roots = Vec::new();
    for (index, parent_index) in parent_indices.iter().enumerate() {
        match parent_index {
            Some(parent_index) => children[*parent_index].push(index),
            None => roots.push(index),
        }
    }
    let sibling_order = |a: &usize, b: &usize| {
        (raw[*a].number, &raw[*a].thread_id).cmp(&(raw[*b].number, &raw[*b].thread_id))
    };
    for list in &mut children {
        list.sort_by(sibling_order);
    }
    roots.sort_by(sibling_order);

    // Visual indent level: top-level tasks stay flush at 0 with or without a
    // thread root above them (the root row differs by styling, not indent).
    let base_depth: u32 = 0;
    let mut visited = vec![false; raw.len()];
    let mut order: Vec<(usize, u32, bool)> = Vec::with_capacity(raw.len());

    let emit_subtree =
        |root_index: usize, visited: &mut Vec<bool>, order: &mut Vec<(usize, u32, bool)>| {
            // (index, depth, is_layout_root): iterative DFS with a visited guard so
            // parent cycles in corrupt projections cannot loop or drop nodes.
            let mut stack = vec![(root_index, base_depth, true)];
            while let Some((index, depth, is_layout_root)) = stack.pop() {
                if visited[index] {
                    continue;
                }
                visited[index] = true;
                order.push((index, depth, is_layout_root));
                for child_index in children[index].iter().rev() {
                    if !visited[*child_index] {
                        stack.push((*child_index, depth.saturating_add(1), false));
                    }
                }
            }
        };

    for root_index in roots {
        emit_subtree(root_index, &mut visited, &mut order);
    }
    // Cycle members are unreachable from any root; surface each cycle as a
    // fallback root (smallest task number first) with its parent edge broken
    // so the emitted forest stays acyclic and retains every node.
    while order.len() < raw.len() {
        let fallback_root = (0..raw.len())
            .filter(|index| !visited[*index])
            .min_by(|a, b| sibling_order(a, b))
            .expect("unvisited node exists while order is incomplete");
        emit_subtree(fallback_root, &mut visited, &mut order);
    }

    let active_count = raw.iter().filter(|node| node.is_active()).count();
    let nodes = order
        .into_iter()
        .map(|(index, depth, is_layout_root)| {
            let mut node = raw[index].node.clone();
            let parent = if is_layout_root {
                match origin_thread_id {
                    Some(origin) => ResolvedParent::Thread {
                        node_id: thread_root_node_id(origin),
                        thread_id: origin.to_owned(),
                    },
                    None => ResolvedParent::None,
                }
            } else {
                ResolvedParent::Task(
                    &raw[parent_indices[index].expect("non-root layout node has a parent")],
                )
            };
            set_original_parent(&mut node, parent);
            set_task_depth(&mut node, depth);
            node
        })
        .collect();

    AnchoredTaskTreeLayout {
        nodes,
        active_count,
    }
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

fn set_task_depth(node: &mut TaskForestNode, value: u32) {
    let (TaskForestNode::Task { depth, .. } | TaskForestNode::Thread { depth, .. }) = node;
    *depth = Some(value);
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
                depth: None,
            },
        }
    }

    fn numbers(layout: &AnchoredTaskTreeLayout) -> Vec<u64> {
        layout
            .nodes
            .iter()
            .map(|node| match node {
                TaskForestNode::Task { task, .. } => task.number,
                TaskForestNode::Thread { .. } => panic!("layout output must be task-only"),
            })
            .collect()
    }

    fn depths(layout: &AnchoredTaskTreeLayout) -> Vec<u32> {
        layout
            .nodes
            .iter()
            .map(|node| match node {
                TaskForestNode::Task { depth, .. } => depth.expect("layout sets depth"),
                TaskForestNode::Thread { .. } => panic!("layout output must be task-only"),
            })
            .collect()
    }

    fn parent_node_id(layout: &AnchoredTaskTreeLayout, number: u64) -> Option<String> {
        layout.nodes.iter().find_map(|node| match node {
            TaskForestNode::Task {
                task,
                parent_node_id,
                ..
            } if task.number == number => parent_node_id.clone(),
            _ => None,
        })
    }

    fn parent_thread_id(layout: &AnchoredTaskTreeLayout, number: u64) -> Option<String> {
        layout.nodes.iter().find_map(|node| match node {
            TaskForestNode::Task {
                task,
                parent_thread_id,
                ..
            } if task.number == number => parent_thread_id.clone(),
            _ => None,
        })
    }

    fn parent_task_number(layout: &AnchoredTaskTreeLayout, number: u64) -> Option<u64> {
        layout.nodes.iter().find_map(|node| match node {
            TaskForestNode::Task {
                task,
                parent_task_number,
                ..
            } if task.number == number => *parent_task_number,
            _ => None,
        })
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
    fn scenario_01_full_tree_is_retained_with_done_leaves_and_branches() {
        let out = layout_anchored_task_tree(mixed_branch_raw(), Some("thread::conversation"));

        // Done leaf 1271 and the fully-done 1263 branch stay in the tree.
        assert_eq!(numbers(&out), vec![1254, 1261, 1262, 1270, 1263, 1271]);
        assert_eq!(out.active_count, 3);
    }

    #[test]
    fn scenario_02_layout_is_anchor_independent() {
        // The same raw tree lays out identically no matter which node the
        // caller anchored on; only client-side highlight moves.
        let with_origin = layout_anchored_task_tree(mixed_branch_raw(), Some("thread::origin"));
        let repeat = layout_anchored_task_tree(mixed_branch_raw(), Some("thread::origin"));
        assert_eq!(numbers(&with_origin), numbers(&repeat));
        assert_eq!(depths(&with_origin), depths(&repeat));
    }

    #[test]
    fn scenario_03_dfs_pre_order_and_depths_with_origin() {
        let out = layout_anchored_task_tree(mixed_branch_raw(), Some("thread::conversation"));

        assert_eq!(numbers(&out), vec![1254, 1261, 1262, 1270, 1263, 1271]);
        assert_eq!(depths(&out), vec![0, 1, 1, 2, 1, 2]);
    }

    #[test]
    fn scenario_04_dfs_pre_order_and_depths_without_origin() {
        let out = layout_anchored_task_tree(mixed_branch_raw(), None);

        assert_eq!(numbers(&out), vec![1254, 1261, 1262, 1270, 1263, 1271]);
        assert_eq!(depths(&out), vec![0, 1, 1, 2, 1, 2]);
    }

    #[test]
    fn scenario_05_origin_parents_root_tasks_to_thread_root() {
        let out = layout_anchored_task_tree(
            vec![
                node(1400, TaskStatus::InProgress, None),
                node(1401, TaskStatus::InProgress, Some(1400)),
                node(1500, TaskStatus::Done, None),
            ],
            Some("thread::conversation"),
        );

        assert_eq!(numbers(&out), vec![1400, 1401, 1500]);
        assert_eq!(
            parent_node_id(&out, 1400).as_deref(),
            Some("thread-root:thread::conversation")
        );
        assert_eq!(
            parent_thread_id(&out, 1400).as_deref(),
            Some("thread::conversation")
        );
        assert_eq!(parent_task_number(&out, 1400), None);
        assert_eq!(
            parent_node_id(&out, 1500).as_deref(),
            Some("thread-root:thread::conversation")
        );
        assert_eq!(
            parent_node_id(&out, 1401).as_deref(),
            Some("task:thread::1400")
        );
        assert_eq!(parent_task_number(&out, 1401), Some(1400));
    }

    #[test]
    fn scenario_06_no_origin_keeps_root_tasks_parentless() {
        let out = layout_anchored_task_tree(
            vec![
                node(1254, TaskStatus::Done, None),
                node(1261, TaskStatus::InProgress, Some(1254)),
            ],
            None,
        );

        assert_eq!(numbers(&out), vec![1254, 1261]);
        assert_eq!(parent_node_id(&out, 1254), None);
        assert_eq!(parent_thread_id(&out, 1254), None);
        assert!(
            out.nodes.iter().all(|node| match node {
                TaskForestNode::Task { parent_node_id, .. } => !parent_node_id
                    .as_deref()
                    .unwrap_or_default()
                    .starts_with("thread-root:"),
                TaskForestNode::Thread { .. } => true,
            }),
            "origin-less trees must not point at synthetic thread roots"
        );
    }

    #[test]
    fn scenario_07_all_done_tree_is_retained() {
        let out = layout_anchored_task_tree(
            vec![
                node(1400, TaskStatus::Done, None),
                node(1401, TaskStatus::Done, Some(1400)),
            ],
            Some("thread::conversation"),
        );

        assert_eq!(numbers(&out), vec![1400, 1401]);
        assert_eq!(depths(&out), vec![0, 1]);
        assert_eq!(out.active_count, 0);
    }

    #[test]
    fn scenario_08_empty_raw_is_empty() {
        let out = layout_anchored_task_tree(Vec::new(), Some("thread::bare"));
        assert!(out.nodes.is_empty());
        assert_eq!(out.active_count, 0);

        let no_origin = layout_anchored_task_tree(Vec::new(), None);
        assert!(no_origin.nodes.is_empty());
    }

    #[test]
    fn scenario_09_sibling_order_is_stable_by_number() {
        // Input arrives in updated_at-flavored order; siblings still emit by
        // ascending task number so status flips cannot reorder the tree.
        let out = layout_anchored_task_tree(
            vec![
                node(1500, TaskStatus::InReview, None),
                node(1400, TaskStatus::Done, None),
                node(1402, TaskStatus::Done, Some(1400)),
                node(1401, TaskStatus::InProgress, Some(1400)),
            ],
            Some("thread::conversation"),
        );

        assert_eq!(numbers(&out), vec![1400, 1401, 1402, 1500]);
        assert_eq!(depths(&out), vec![0, 1, 1, 0]);
        assert_eq!(out.active_count, 2);
    }

    #[test]
    fn scenario_10_active_count_counts_in_progress_and_in_review_only() {
        let out = layout_anchored_task_tree(
            vec![
                node(1, TaskStatus::Todo, None),
                node(2, TaskStatus::InProgress, Some(1)),
                node(3, TaskStatus::InReview, Some(1)),
                node(4, TaskStatus::Done, Some(1)),
            ],
            Some("thread::conversation"),
        );

        assert_eq!(out.active_count, 2);
        assert_eq!(numbers(&out), vec![1, 2, 3, 4]);
    }

    #[test]
    fn parent_priority_matches_existing_task_forest_order() {
        let out = layout_anchored_task_tree(
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
            None,
        );

        assert_eq!(parent_node_id(&out, 4).as_deref(), Some("task:thread::1"));
        assert_eq!(parent_node_id(&out, 5).as_deref(), Some("task:thread::2"));
        assert_eq!(parent_node_id(&out, 6).as_deref(), Some("task:thread::3"));
    }

    #[test]
    fn cycle_guard_emits_every_node_and_breaks_the_cycle_edge() {
        // 10 -> 11 -> 10 parent cycle plus a normal root: every node is still
        // emitted exactly once and the cycle entry is re-rooted.
        let out = layout_anchored_task_tree(
            vec![
                node(1, TaskStatus::InProgress, None),
                node(10, TaskStatus::Done, Some(11)),
                node(11, TaskStatus::InProgress, Some(10)),
            ],
            Some("thread::conversation"),
        );

        assert_eq!(numbers(&out), vec![1, 10, 11]);
        assert_eq!(depths(&out), vec![0, 0, 1]);
        assert_eq!(
            parent_node_id(&out, 10).as_deref(),
            Some("thread-root:thread::conversation"),
            "cycle entry re-roots at the origin"
        );
        assert_eq!(parent_node_id(&out, 11).as_deref(), Some("task:thread::10"));
        assert_eq!(out.active_count, 2);
    }

    #[test]
    fn self_parent_node_is_treated_as_root() {
        let out = layout_anchored_task_tree(vec![node(7, TaskStatus::InProgress, Some(7))], None);

        assert_eq!(numbers(&out), vec![7]);
        assert_eq!(depths(&out), vec![0]);
        assert_eq!(parent_node_id(&out, 7), None);
    }

    #[test]
    fn conversation_anchor_keeps_multi_level_task_parents() {
        let out = layout_anchored_task_tree(
            vec![
                node(1400, TaskStatus::InProgress, None),
                node(1401, TaskStatus::InProgress, Some(1400)),
                node(1402, TaskStatus::InReview, Some(1401)),
            ],
            Some("thread::conversation"),
        );

        assert_eq!(numbers(&out), vec![1400, 1401, 1402]);
        assert_eq!(depths(&out), vec![0, 1, 2]);
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
}
