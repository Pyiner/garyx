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

export function taskTreeBadgeCount(nodes: DesktopTaskForestNode[]): number {
  return nodes.filter((node) => isTaskNode(node) && isActiveTaskStatus(node.status)).length;
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
  threadLogsOpen: boolean;
}): boolean {
  return Boolean(
    input.selectedThreadId &&
      !input.hasWorkflowRunContent &&
      !input.inspectorOpen &&
      !input.threadLogsOpen,
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
  nodes: DesktopTaskForestNode[],
): TaskTreeRow[] {
  const ids = new Set(nodes.map((node) => node.nodeId));
  const originalIndex = new Map(nodes.map((node, index) => [node.nodeId, index]));
  const childrenByParent = new Map<string, DesktopTaskForestNode[]>();
  for (const node of nodes) {
    const parent =
      node.kind === "task" && node.parentNodeId && ids.has(node.parentNodeId)
        ? node.parentNodeId
        : "";
    const list = childrenByParent.get(parent) ?? [];
    list.push(node);
    childrenByParent.set(parent, list);
  }
  for (const list of childrenByParent.values()) {
    list.sort((a, b) => {
      if (a.kind === "task" && b.kind === "task") {
        return a.number - b.number;
      }
      if (a.kind === "thread" && b.kind === "task") {
        return -1;
      }
      if (a.kind === "task" && b.kind === "thread") {
        return 1;
      }
      return (originalIndex.get(a.nodeId) ?? 0) - (originalIndex.get(b.nodeId) ?? 0);
    });
  }
  const rows: TaskTreeRow[] = [];
  const visited = new Set<string>();
  const walk = (parent: string, depth: number) => {
    for (const node of childrenByParent.get(parent) ?? []) {
      if (visited.has(node.nodeId)) {
        continue;
      }
      visited.add(node.nodeId);
      if (node.kind === "thread") {
        walk(node.nodeId, depth);
      } else {
        rows.push({ task: node, depth });
        walk(node.nodeId, Math.min(depth + 1, 4));
      }
    }
  };
  walk("", 0);
  return rows;
}
