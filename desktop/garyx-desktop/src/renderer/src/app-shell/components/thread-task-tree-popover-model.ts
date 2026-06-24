import type {
  DesktopTaskForestNode,
  DesktopTaskForestTaskNode,
  DesktopTaskStatus,
} from "@shared/contracts";

export type TaskTreeRow = { task: DesktopTaskForestTaskNode; depth: number };

export function isTaskNode(
  node: DesktopTaskForestNode,
): node is DesktopTaskForestTaskNode {
  return node.kind === "task";
}

export function visibleTaskTreeTasks(
  nodes: DesktopTaskForestNode[],
): DesktopTaskForestTaskNode[] {
  return nodes.filter(isTaskNode);
}

export function isActiveTaskStatus(status: DesktopTaskStatus): boolean {
  return status === "in_progress" || status === "in_review";
}

export function taskTreeBadgeCount(tasks: DesktopTaskForestTaskNode[]): number {
  return tasks.filter((task) => isActiveTaskStatus(task.status)).length;
}

export function isCurrentTaskTreeNode(
  task: DesktopTaskForestTaskNode,
  threadId: string | null,
): boolean {
  return !!threadId && task.threadId === threadId;
}

export function shouldShowThreadTaskTreePopover(input: {
  hasWorkflowRunContent: boolean;
  inspectorOpen: boolean;
  selectedThreadId: string | null;
}): boolean {
  return Boolean(
    input.selectedThreadId &&
      !input.hasWorkflowRunContent &&
      !input.inspectorOpen,
  );
}

export function taskStatusTone(status: DesktopTaskStatus): string {
  switch (status) {
    case "in_review":
      return "review";
    case "done":
      return "done";
    case "todo":
      return "todo";
    case "in_progress":
    default:
      return "progress";
  }
}

export function taskStatusLabel(status: DesktopTaskStatus): string {
  switch (status) {
    case "in_review":
      return "In Review";
    case "done":
      return "Done";
    case "todo":
      return "Todo";
    case "in_progress":
    default:
      return "In Progress";
  }
}

/** Depth-order the tasks into a tree via parentNodeId; nodes whose parent
 *  isn't in the set become roots (depth 0). */
export function buildTaskRows(
  tasks: DesktopTaskForestTaskNode[],
): TaskTreeRow[] {
  const ids = new Set(tasks.map((task) => task.nodeId));
  const childrenByParent = new Map<string, DesktopTaskForestTaskNode[]>();
  for (const task of tasks) {
    const parent =
      task.parentNodeId && ids.has(task.parentNodeId) ? task.parentNodeId : "";
    const list = childrenByParent.get(parent) ?? [];
    list.push(task);
    childrenByParent.set(parent, list);
  }
  for (const list of childrenByParent.values()) {
    list.sort((a, b) => a.number - b.number);
  }
  const rows: TaskTreeRow[] = [];
  const visited = new Set<string>();
  const walk = (parent: string, depth: number) => {
    for (const task of childrenByParent.get(parent) ?? []) {
      if (visited.has(task.nodeId)) {
        continue;
      }
      visited.add(task.nodeId);
      rows.push({ task, depth });
      walk(task.nodeId, Math.min(depth + 1, 4));
    }
  };
  walk("", 0);
  return rows;
}
