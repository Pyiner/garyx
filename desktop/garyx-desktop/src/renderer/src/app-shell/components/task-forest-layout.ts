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
  from: number;
  to: number;
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
  if (typeof task.parentTaskNumber === "number" && task.parentTaskNumber > 0) {
    return task.parentTaskNumber;
  }
  return parseTaskNumber(task.source?.taskId);
}

function updatedTime(task: DesktopTaskForestNode): number {
  const timestamp = Date.parse(task.updatedAt);
  return Number.isFinite(timestamp) ? timestamp : 0;
}

function compareTasks(left: DesktopTaskForestNode, right: DesktopTaskForestNode): number {
  return (
    STATUS_ORDER[left.status] - STATUS_ORDER[right.status] ||
    updatedTime(right) - updatedTime(left) ||
    left.number - right.number
  );
}

function isRunning(task: DesktopTaskForestNode): boolean {
  return task.runState === "running" || Boolean(task.activeRunId);
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
  const byNumber = new Map<number, DesktopTaskForestNode>();
  for (const task of tasks) {
    if (task.number > 0 && !byNumber.has(task.number)) {
      byNumber.set(task.number, task);
    }
  }

  const childrenByParent = new Map<number, DesktopTaskForestNode[]>();
  const roots: DesktopTaskForestNode[] = [];
  for (const task of tasks) {
    const parentNumber = parentNumberFor(task);
    if (!parentNumber || !byNumber.has(parentNumber)) {
      roots.push(task);
      continue;
    }
    const children = childrenByParent.get(parentNumber) ?? [];
    children.push(task);
    childrenByParent.set(parentNumber, children);
  }
  for (const children of childrenByParent.values()) {
    children.sort(compareTasks);
  }

  const subtreeCounts = new Map<number, Record<DesktopTaskStatus, number>>();
  const subtreeSizes = new Map<number, number>();

  function collect(task: DesktopTaskForestNode, seen = new Set<number>()): {
    size: number;
    counts: Record<DesktopTaskStatus, number>;
  } {
    if (seen.has(task.number)) {
      return { size: 0, counts: { ...EMPTY_STATUS_COUNTS } };
    }
    seen.add(task.number);
    let size = 1;
    let counts = { ...EMPTY_STATUS_COUNTS };
    for (const child of childrenByParent.get(task.number) ?? []) {
      counts[child.status] += 1;
      const childValue = collect(child, new Set(seen));
      size += childValue.size;
      counts = mergeCounts(counts, childValue.counts);
    }
    subtreeCounts.set(task.number, counts);
    subtreeSizes.set(task.number, size);
    return { size, counts };
  }

  for (const root of roots) {
    collect(root);
  }

  roots.sort((left, right) => {
    const runningDelta = Number(isRunning(right)) - Number(isRunning(left));
    return (
      runningDelta ||
      (subtreeSizes.get(right.number) ?? 1) - (subtreeSizes.get(left.number) ?? 1) ||
      updatedTime(right) - updatedTime(left) ||
      left.number - right.number
    );
  });

  const nodes: TaskForestLayoutNode[] = [];
  let nextY = 24;

  function place(task: DesktopTaskForestNode, depth: number, yHint: number): number {
    const visibleChildren =
      depth + 1 < maxDepth ? childrenByParent.get(task.number) ?? [] : [];
    if (visibleChildren.length === 0) {
      nodes.push({
        task,
        x: 24 + depth * (nodeWidth + columnGap),
        y: yHint,
        width: nodeWidth,
        height: nodeHeight,
        depth,
        hiddenDescendantCount:
          depth + 1 >= maxDepth ? Math.max(0, (subtreeSizes.get(task.number) ?? 1) - 1) : 0,
        descendantStatusCounts: subtreeCounts.get(task.number) ?? { ...EMPTY_STATUS_COUNTS },
      });
      return yHint + nodeHeight + rowGap;
    }

    let childY = yHint;
    const childStart = childY;
    for (const child of visibleChildren) {
      childY = place(child, depth + 1, childY);
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
      descendantStatusCounts: subtreeCounts.get(task.number) ?? { ...EMPTY_STATUS_COUNTS },
    });
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

  const byPlacedNumber = new Map(nodes.map((node) => [node.task.number, node]));
  const edges: TaskForestLayoutEdge[] = [];
  for (const node of nodes) {
    const parentNumber = parentNumberFor(node.task);
    const parent = parentNumber ? byPlacedNumber.get(parentNumber) : null;
    if (!parent) {
      continue;
    }
    const startX = parent.x + parent.width;
    const startY = parent.y + parent.height / 2;
    const endX = node.x;
    const endY = node.y + node.height / 2;
    edges.push({
      from: parent.task.number,
      to: node.task.number,
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
    nodes: nodes.sort((left, right) => left.task.number - right.task.number),
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
