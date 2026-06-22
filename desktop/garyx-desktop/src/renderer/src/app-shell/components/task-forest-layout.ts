import type { DesktopTaskForestNode, DesktopTaskStatus } from "@shared/contracts";

export type TaskForestLayoutNode = {
  task: DesktopTaskForestNode;
  x: number;
  y: number;
  width: number;
  height: number;
  depth: number;
  hiddenDescendantCount: number;
  descendantStatusCounts: Record<DesktopTaskStatus, number>;
};

export type TaskForestLayoutEdge = {
  from: string;
  to: string;
  path: string;
  active: boolean;
};

export type TaskForestLayout = {
  nodes: TaskForestLayoutNode[];
  edges: TaskForestLayoutEdge[];
  bbox: {
    minX: number;
    minY: number;
    maxX: number;
    maxY: number;
  };
};

export type TaskForestViewport = {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
};

type BuildTaskForestLayoutOptions = {
  maxDepth?: number;
  nodeWidth?: number;
  nodeHeight?: number;
  columnGap?: number;
  rowGap?: number;
  rootGap?: number;
};

const STATUS_ORDER: Record<DesktopTaskStatus, number> = {
  in_progress: 0,
  in_review: 1,
  todo: 2,
  done: 3,
};

const EMPTY_STATUS_COUNTS: Record<DesktopTaskStatus, number> = {
  todo: 0,
  in_progress: 0,
  in_review: 0,
  done: 0,
};

function parseTaskNumber(taskId?: string | null): number | null {
  const match = taskId?.trim().match(/^#?TASK-(\d+)$/i);
  if (!match) {
    return null;
  }
  const number = Number.parseInt(match[1], 10);
  return Number.isFinite(number) && number > 0 ? number : null;
}

function parentNumberFor(task: DesktopTaskForestNode): number | null {
  if (task.kind !== "task") {
    return null;
  }
  if (typeof task.parentTaskNumber === "number" && task.parentTaskNumber > 0) {
    return task.parentTaskNumber;
  }
  return parseTaskNumber(task.source?.taskId);
}

function updatedTime(task: DesktopTaskForestNode): number {
  const timestamp = Date.parse(task.updatedAt || "");
  return Number.isFinite(timestamp) ? timestamp : 0;
}

function compareTasks(left: DesktopTaskForestNode, right: DesktopTaskForestNode): number {
  if (left.kind !== "task" && right.kind === "task") {
    return -1;
  }
  if (left.kind === "task" && right.kind !== "task") {
    return 1;
  }
  if (left.kind !== "task" || right.kind !== "task") {
    return updatedTime(right) - updatedTime(left) || left.nodeId.localeCompare(right.nodeId);
  }
  return (
    STATUS_ORDER[left.status] - STATUS_ORDER[right.status] ||
    updatedTime(right) - updatedTime(left) ||
    left.number - right.number
  );
}

function isRunning(task: DesktopTaskForestNode): boolean {
  const runState = task.runState.trim().toLowerCase();
  if (runState === "running" || runState === "streaming" || runState === "pending") {
    return true;
  }
  if (runState === "idle" || runState === "completed" || runState === "failed" || runState === "error" || runState === "aborted") {
    return false;
  }
  return Boolean(task.activeRunId);
}

function nodeIdFor(task: DesktopTaskForestNode): string {
  return task.nodeId || `${task.kind}:${task.threadId}`;
}

function roundedOrthogonalPath(
  startX: number,
  startY: number,
  endX: number,
  endY: number,
): string {
  const midX = startX + Math.max(28, (endX - startX) / 2);
  const radius = Math.min(18, Math.abs(endY - startY) / 2, Math.max(0, endX - startX) / 4);
  if (radius <= 0.5) {
    return `M ${startX} ${startY} L ${endX} ${endY}`;
  }
  const direction = endY >= startY ? 1 : -1;
  return [
    `M ${startX} ${startY}`,
    `L ${midX - radius} ${startY}`,
    `Q ${midX} ${startY} ${midX} ${startY + direction * radius}`,
    `L ${midX} ${endY - direction * radius}`,
    `Q ${midX} ${endY} ${midX + radius} ${endY}`,
    `L ${endX} ${endY}`,
  ].join(" ");
}

function mergeCounts(
  left: Record<DesktopTaskStatus, number>,
  right: Record<DesktopTaskStatus, number>,
): Record<DesktopTaskStatus, number> {
  return {
    todo: left.todo + right.todo,
    in_progress: left.in_progress + right.in_progress,
    in_review: left.in_review + right.in_review,
    done: left.done + right.done,
  };
}

export function buildTaskForestLayout(
  tasks: DesktopTaskForestNode[],
  options: BuildTaskForestLayoutOptions = {},
): TaskForestLayout {
  const nodeWidth = options.nodeWidth ?? 264;
  const nodeHeight = options.nodeHeight ?? 96;
  const columnGap = options.columnGap ?? 92;
  const rowGap = options.rowGap ?? 28;
  const rootGap = options.rootGap ?? 44;
  const maxDepth = Math.max(1, options.maxDepth ?? 3);
  const inputIndex = new Map<string, number>();
  tasks.forEach((task, index) => {
    inputIndex.set(nodeIdFor(task), index);
  });
  const byId = new Map<string, DesktopTaskForestNode>();
  for (const task of tasks) {
    const nodeId = nodeIdFor(task);
    if (!byId.has(nodeId)) {
      byId.set(nodeId, task);
    }
  }
  const byNumber = new Map<number, DesktopTaskForestNode>();
  for (const task of tasks) {
    if (task.kind === "task" && task.number > 0 && !byNumber.has(task.number)) {
      byNumber.set(task.number, task);
    }
  }

  function parentNodeIdFor(task: DesktopTaskForestNode): string | null {
    if (task.kind === "task" && task.parentNodeId) {
      return task.parentNodeId;
    }
    const parentNumber = parentNumberFor(task);
    if (!parentNumber) {
      return null;
    }
    const parent = byNumber.get(parentNumber);
    return parent ? nodeIdFor(parent) : null;
  }

  const childrenByParent = new Map<string, DesktopTaskForestNode[]>();
  const roots: DesktopTaskForestNode[] = [];
  for (const task of tasks) {
    const parentNodeId = parentNodeIdFor(task);
    if (!parentNodeId || !byId.has(parentNodeId)) {
      roots.push(task);
      continue;
    }
    const children = childrenByParent.get(parentNodeId) ?? [];
    children.push(task);
    childrenByParent.set(parentNodeId, children);
  }
  for (const children of childrenByParent.values()) {
    children.sort(compareTasks);
  }

  const subtreeValues = new Map<
    string,
    { size: number; counts: Record<DesktopTaskStatus, number> }
  >();
  const visiting = new Set<string>();

  function collect(task: DesktopTaskForestNode): {
    size: number;
    counts: Record<DesktopTaskStatus, number>;
  } {
    const nodeId = nodeIdFor(task);
    const memoized = subtreeValues.get(nodeId);
    if (memoized) {
      return memoized;
    }
    if (visiting.has(nodeId)) {
      return { size: 0, counts: { ...EMPTY_STATUS_COUNTS } };
    }
    visiting.add(nodeId);
    let size = 1;
    let counts = { ...EMPTY_STATUS_COUNTS };
    for (const child of childrenByParent.get(nodeId) ?? []) {
      const childNodeId = nodeIdFor(child);
      if (visiting.has(childNodeId)) {
        continue;
      }
      if (child.kind === "task") {
        counts[child.status] += 1;
      }
      const childValue = collect(child);
      size += childValue.size;
      counts = mergeCounts(counts, childValue.counts);
    }
    visiting.delete(nodeId);
    const value = { size, counts };
    subtreeValues.set(nodeId, value);
    return value;
  }

  for (const root of roots) {
    collect(root);
  }
  for (const task of tasks) {
    if (!subtreeValues.has(nodeIdFor(task))) {
      roots.push(task);
      collect(task);
    }
  }

  roots.sort((left, right) => {
    return (inputIndex.get(nodeIdFor(left)) ?? 0) - (inputIndex.get(nodeIdFor(right)) ?? 0);
  });

  const nodes: TaskForestLayoutNode[] = [];
  const placed = new Set<string>();
  let nextY = 24;

  function place(
    task: DesktopTaskForestNode,
    depth: number,
    yHint: number,
    path = new Set<string>(),
  ): number {
    const nodeId = nodeIdFor(task);
    if (placed.has(nodeId) || path.has(nodeId)) {
      return yHint;
    }
    path.add(nodeId);
    const visibleChildren =
      depth + 1 < maxDepth
        ? (childrenByParent.get(nodeId) ?? []).filter(
            (child) => !path.has(nodeIdFor(child)),
          )
        : [];
    const subtree = subtreeValues.get(nodeId);
    if (visibleChildren.length === 0) {
      nodes.push({
        task,
        x: 24 + depth * (nodeWidth + columnGap),
        y: yHint,
        width: nodeWidth,
        height: nodeHeight,
        depth,
        hiddenDescendantCount:
          depth + 1 >= maxDepth ? Math.max(0, (subtree?.size ?? 1) - 1) : 0,
        descendantStatusCounts: subtree?.counts ?? { ...EMPTY_STATUS_COUNTS },
      });
      placed.add(nodeId);
      path.delete(nodeId);
      return yHint + nodeHeight + rowGap;
    }

    let childY = yHint;
    const childStart = childY;
    for (const child of visibleChildren) {
      childY = place(child, depth + 1, childY, path);
    }
    const childEnd = childY - rowGap;
    const y = Math.max(yHint, childStart + (childEnd - childStart - nodeHeight) / 2);
    nodes.push({
      task,
      x: 24 + depth * (nodeWidth + columnGap),
      y,
      width: nodeWidth,
      height: nodeHeight,
      depth,
      hiddenDescendantCount: 0,
      descendantStatusCounts: subtree?.counts ?? { ...EMPTY_STATUS_COUNTS },
    });
    placed.add(nodeId);
    path.delete(nodeId);
    return Math.max(childY, y + nodeHeight + rowGap);
  }

  for (const root of roots) {
    const before = nextY;
    nextY = place(root, 0, nextY);
    if (nextY === before) {
      nextY += nodeHeight + rowGap;
    }
    nextY += rootGap - rowGap;
  }

  const byPlacedId = new Map(nodes.map((node) => [nodeIdFor(node.task), node]));
  const edges: TaskForestLayoutEdge[] = [];
  for (const node of nodes) {
    const parentNodeId = parentNodeIdFor(node.task);
    const parent = parentNodeId ? byPlacedId.get(parentNodeId) : null;
    if (!parent) {
      continue;
    }
    const startX = parent.x + parent.width;
    const startY = parent.y + parent.height / 2;
    const endX = node.x;
    const endY = node.y + node.height / 2;
    edges.push({
      from: nodeIdFor(parent.task),
      to: nodeIdFor(node.task),
      active: isRunning(parent.task) || isRunning(node.task),
      path: roundedOrthogonalPath(startX, startY, endX, endY),
    });
  }

  if (nodes.length === 0) {
    return {
      nodes,
      edges,
      bbox: { minX: 0, minY: 0, maxX: 1, maxY: 1 },
    };
  }

  return {
    nodes: nodes.sort(
      (left, right) =>
        (inputIndex.get(nodeIdFor(left.task)) ?? 0) -
        (inputIndex.get(nodeIdFor(right.task)) ?? 0),
    ),
    edges,
    bbox: {
      minX: Math.min(...nodes.map((node) => node.x)),
      minY: Math.min(...nodes.map((node) => node.y)),
      maxX: Math.max(...nodes.map((node) => node.x + node.width)),
      maxY: Math.max(...nodes.map((node) => node.y + node.height)),
    },
  };
}

export function taskForestParentNumberForTest(task: DesktopTaskForestNode): number | null {
  return parentNumberFor(task);
}

export function visibleTaskForestNodeNumbers(
  nodes: TaskForestLayoutNode[],
  viewport: TaskForestViewport,
  overscan = 0,
): Set<string> {
  const minX = viewport.minX - overscan;
  const minY = viewport.minY - overscan;
  const maxX = viewport.maxX + overscan;
  const maxY = viewport.maxY + overscan;
  const visible = new Set<string>();
  for (const node of nodes) {
    const nodeMaxX = node.x + node.width;
    const nodeMaxY = node.y + node.height;
    if (nodeMaxX >= minX && node.x <= maxX && nodeMaxY >= minY && node.y <= maxY) {
      visible.add(nodeIdFor(node.task));
    }
  }
  return visible;
}
