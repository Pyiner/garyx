import type {
  DesktopTaskForestNode,
  DesktopTaskForestPage,
  DesktopTaskForestTaskNode,
  DesktopTaskForestThreadNode,
  DesktopTaskStatus,
} from "@shared/contracts";

const MAX_ROW_DEPTH = 4;

export type TaskForestPollingState = {
  awaitingFirstSnapshot: boolean;
  stopped: boolean;
  threadId: string | null;
};

export function createTaskForestPollingState(
  threadId: string | null,
): TaskForestPollingState {
  return {
    awaitingFirstSnapshot: Boolean(threadId),
    stopped: false,
    threadId,
  };
}

export function shouldLoadTaskForest(input: {
  hidden: boolean;
  state: TaskForestPollingState;
  threadId: string | null;
}): boolean {
  return Boolean(
    input.threadId &&
      input.state.threadId === input.threadId &&
      !input.state.stopped &&
      !input.hidden,
  );
}

/** Only the first successful live snapshot can quiesce an empty tree. */
export function taskForestPollingStateAfterSnapshot(
  state: TaskForestPollingState,
  input: { nodeCount: number; threadId: string },
): TaskForestPollingState {
  if (
    state.threadId !== input.threadId ||
    state.stopped ||
    !state.awaitingFirstSnapshot
  ) {
    return state;
  }
  return {
    ...state,
    awaitingFirstSnapshot: false,
    stopped: input.nodeCount === 0,
  };
}

/** Keep the tree snapshot LRU and its reverse anchor index consistent. */
export function evictTaskTreeSnapshots<T>(
  treeKeyByAnchor: Map<string, string>,
  treeSnapshotByKey: Map<string, T>,
  maxCachedTrees: number,
): void {
  const boundedMax = Math.max(0, maxCachedTrees);
  while (treeSnapshotByKey.size > boundedMax) {
    const oldest = treeSnapshotByKey.keys().next().value;
    if (oldest === undefined) {
      break;
    }
    treeSnapshotByKey.delete(oldest);
    for (const [anchor, treeKey] of treeKeyByAnchor) {
      if (treeKey === oldest) {
        treeKeyByAnchor.delete(anchor);
      }
    }
  }
}

export type TaskTreeRow =
  | { kind: "task"; task: DesktopTaskForestTaskNode; depth: number }
  | { kind: "thread"; thread: DesktopTaskForestThreadNode; depth: number };

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

/** Badge count: prefer the server-computed page count, recount locally when
 *  an old gateway omits it. */
export function resolveTaskTreeActiveCount(
  page: Pick<DesktopTaskForestPage, "tasks" | "activeCount">,
): number {
  return page.activeCount ?? taskTreeBadgeCount(page.tasks);
}

export function isCurrentTaskTreeNode(
  node: { threadId: string },
  threadId: string | null,
): boolean {
  return !!threadId && node.threadId === threadId;
}

export function shouldShowThreadTaskTreePopover(input: {
  inspectorOpen: boolean;
  isSideChatSurface: boolean;
  selectedThreadId: string | null;
}): boolean {
  return Boolean(
    input.selectedThreadId &&
      !input.inspectorOpen &&
      !input.isSideChatSurface,
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

function taskRow(node: DesktopTaskForestTaskNode, depth: number): TaskTreeRow {
  return { kind: "task", task: node, depth: Math.min(depth, MAX_ROW_DEPTH) };
}

/** Sibling sort rank: working tasks float to the top, finished ones sink.
 *  Mirrors the gateway layout order for the old-gateway fallback. */
function taskStatusSortRank(status: DesktopTaskStatus): number {
  switch (status) {
    case "in_progress":
      return 0;
    case "in_review":
      return 1;
    case "todo":
      return 2;
    case "done":
    default:
      return 3;
  }
}

function threadRow(node: DesktopTaskForestThreadNode, depth: number): TaskTreeRow {
  return { kind: "thread", thread: node, depth: Math.min(depth, MAX_ROW_DEPTH) };
}

function rowFromNode(node: DesktopTaskForestNode, depth: number): TaskTreeRow {
  return node.kind === "thread" ? threadRow(node, depth) : taskRow(node, depth);
}

function hasServerLayout(nodes: DesktopTaskForestNode[]): boolean {
  return nodes.every((node) => Number.isFinite(node.depth ?? NaN));
}

/** Rows for the popover, thread roots included as visible rows.
 *
 *  New gateways send DFS pre-order plus per-node `depth`; render the wire
 *  order as-is. Old gateways omit `depth`, so fall back to a local tree
 *  build via parentNodeId — nodes whose parent isn't in the set (orphans)
 *  become roots. `depth` is the visual indent level: the thread root row and
 *  top-level tasks both sit flush at 0 (the root row is distinguished by
 *  styling, not indentation), so the fallback keeps thread children at their
 *  parent thread's depth. */
export function buildTaskRows(
  nodes: DesktopTaskForestNode[],
): TaskTreeRow[] {
  if (nodes.length === 0) {
    return [];
  }
  if (hasServerLayout(nodes)) {
    return nodes.map((node) => rowFromNode(node, node.depth ?? 0));
  }
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
        const rank = taskStatusSortRank(a.status) - taskStatusSortRank(b.status);
        if (rank !== 0) {
          return rank;
        }
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
      rows.push(rowFromNode(node, depth));
      walk(node.nodeId, node.kind === "thread" ? depth : depth + 1);
    }
  };
  walk("", 0);
  return rows;
}
