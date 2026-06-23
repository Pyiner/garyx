import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent,
  type ReactNode,
  type WheelEvent,
} from "react";
import {
  CheckCircle2,
  GitBranch,
  Maximize2,
  MessageSquare,
  PanelRightClose,
  RefreshCcw,
  Send,
  Sparkles,
  type LucideIcon,
} from "lucide-react";

import type {
  DesktopTaskForestNode,
  DesktopTaskForestTaskNode,
  DesktopTaskStatus,
} from "@shared/contracts";

import { getDesktopApi } from "../../platform/desktop-api";
import { useI18n } from "../../i18n";
import type { ToastTone } from "../../toast";
import {
  buildTaskForestLayout,
  visibleTaskForestNodeNumbers,
} from "./task-forest-layout";

type TaskForestConsoleProps = {
  pinnedThreadIds: string[];
  pinnedThreadsVersion: number;
  selectedThreadId: string | null;
  selectedThreadPanel: ReactNode;
  sourceBot: string | null;
  onOpenThreadInPanel: (threadId: string) => Promise<boolean> | boolean;
  onToast: (message: string, tone?: ToastTone) => void;
};

type Camera = {
  x: number;
  y: number;
  z: number;
};

const STATUS_META: Record<
  DesktopTaskStatus,
  { label: string; tone: string; Icon: LucideIcon }
> = {
  todo: { label: "Todo", tone: "todo", Icon: GitBranch },
  in_progress: { label: "In Progress", tone: "progress", Icon: Sparkles },
  in_review: { label: "In Review", tone: "review", Icon: Send },
  done: { label: "Done", tone: "done", Icon: CheckCircle2 },
};

const STATUS_ORDER: DesktopTaskStatus[] = [
  "todo",
  "in_progress",
  "in_review",
  "done",
];

const REFRESH_INTERVAL_MS = 5000;
const CULLING_NODE_THRESHOLD = 120;
const CULLING_OVERSCAN_PX = 360;
const MINIMAP_PLOT_WIDTH = 150;
const MINIMAP_PLOT_HEIGHT = 82;
const SMOOTH_CAMERA_MS = 220;

function isTaskNode(node: DesktopTaskForestNode | null): node is DesktopTaskForestTaskNode {
  return Boolean(node && node.kind === "task");
}

function displayNodeId(node: DesktopTaskForestNode): string {
  return node.kind === "task" ? displayTaskId(node) : "Pinned";
}

function displayTaskId(task: DesktopTaskForestTaskNode): string {
  return task.taskId || `#TASK-${task.number}`;
}

function principalLabel(task: DesktopTaskForestTaskNode): string {
  if (task.assignee?.kind === "agent") {
    return task.assignee.agentId;
  }
  if (task.assignee?.kind === "human") {
    return `@${task.assignee.userId}`;
  }
  return task.runtimeAgentId || "unassigned";
}

function threadRootLabel(thread: DesktopTaskForestNode): string {
  if (thread.kind === "task") {
    return principalLabel(thread);
  }
  return thread.agentId || thread.providerType || "thread";
}

function initials(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) {
    return "?";
  }
  return trimmed
    .split(/[^a-zA-Z0-9]+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part.slice(0, 1).toUpperCase())
    .join("") || trimmed.slice(0, 1).toUpperCase();
}

function isActiveRun(task: DesktopTaskForestNode): boolean {
  const runState = task.runState.trim().toLowerCase();
  if (runState === "running" || runState === "streaming" || runState === "pending") {
    return true;
  }
  if (runState === "idle" || runState === "completed" || isFailedRun(task)) {
    return false;
  }
  return Boolean(task.activeRunId);
}

function isFailedRun(task: DesktopTaskForestNode): boolean {
  const runState = task.runState.trim().toLowerCase();
  return runState === "failed" || runState === "error" || runState === "aborted";
}

function taskNumberFromId(taskId?: string | null): number | null {
  const match = taskId?.trim().match(/^#?TASK-(\d+)$/i);
  if (!match) {
    return null;
  }
  const number = Number.parseInt(match[1], 10);
  return Number.isFinite(number) && number > 0 ? number : null;
}

function parentNumberForNavigation(task: DesktopTaskForestNode): number | null {
  if (task.kind !== "task") {
    return null;
  }
  if (typeof task.parentTaskNumber === "number" && task.parentTaskNumber > 0) {
    return task.parentTaskNumber;
  }
  return taskNumberFromId(task.source?.taskId);
}

function isEditableEventTarget(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLElement &&
    Boolean(
      target.closest("input, textarea, select, [contenteditable='true'], [role='textbox']"),
    )
  );
}

export function TaskForestConsole({
  pinnedThreadIds,
  pinnedThreadsVersion,
  selectedThreadId,
  selectedThreadPanel,
  sourceBot,
  onOpenThreadInPanel,
  onToast,
}: TaskForestConsoleProps) {
  const { t } = useI18n();
  const stageRef = useRef<HTMLDivElement | null>(null);
  const requestRef = useRef<Promise<void> | null>(null);
  const requestSequenceRef = useRef(0);
  const currentRequestSequenceRef = useRef<number | null>(null);
  const trailingRequestRef = useRef<Promise<void> | null>(null);
  const skipResultForRequestRef = useRef<number | null>(null);
  const fittedSignatureRef = useRef<string | null>(null);
  const mountedRef = useRef(true);
  const openTaskRequestRef = useRef(0);
  const smoothCameraTimeoutRef = useRef<number | null>(null);
  const dragRef = useRef<{
    pointerId: number;
    startX: number;
    startY: number;
    camera: Camera;
  } | null>(null);
  const [tasks, setTasks] = useState<DesktopTaskForestNode[]>([]);
  const [rootThreadIds, setRootThreadIds] = useState<string[]>([]);
  const [skippedPinnedThreadIds, setSkippedPinnedThreadIds] = useState<string[]>([]);
  const [projectionCurrent, setProjectionCurrent] = useState(true);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [statusFilter, setStatusFilter] = useState<DesktopTaskStatus | null>(null);
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [cursorNodeId, setCursorNodeId] = useState<string | null>(null);
  const [camera, setCamera] = useState<Camera>({ x: 48, y: 42, z: 1 });
  const [stageSize, setStageSize] = useState({ width: 0, height: 0 });
  const [smoothCamera, setSmoothCamera] = useState(false);
  const [spacePanning, setSpacePanning] = useState(false);
  const [mutatingTaskId, setMutatingTaskId] = useState<string | null>(null);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      if (smoothCameraTimeoutRef.current !== null) {
        window.clearTimeout(smoothCameraTimeoutRef.current);
      }
    };
  }, []);

  useEffect(() => {
    const stage = stageRef.current;
    if (!stage) {
      return undefined;
    }
    const updateSize = () => {
      const rect = stage.getBoundingClientRect();
      setStageSize({
        width: Math.max(0, rect.width),
        height: Math.max(0, rect.height),
      });
    };
    updateSize();
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", updateSize);
      return () => window.removeEventListener("resize", updateSize);
    }
    const observer = new ResizeObserver(updateSize);
    observer.observe(stage);
    return () => observer.disconnect();
  }, []);

  const moveCamera = useCallback((nextCamera: Camera, smooth = false) => {
    if (smoothCameraTimeoutRef.current !== null) {
      window.clearTimeout(smoothCameraTimeoutRef.current);
      smoothCameraTimeoutRef.current = null;
    }
    setSmoothCamera(smooth);
    setCamera(nextCamera);
    if (smooth) {
      smoothCameraTimeoutRef.current = window.setTimeout(() => {
        if (mountedRef.current) {
          setSmoothCamera(false);
        }
        smoothCameraTimeoutRef.current = null;
      }, SMOOTH_CAMERA_MS);
    }
  }, []);

  const layout = useMemo(() => buildTaskForestLayout(tasks), [tasks]);
  const pinnedThreadSignature = useMemo(
    () => pinnedThreadIds.map((threadId) => threadId.trim()).filter(Boolean).join("\n"),
    [pinnedThreadIds],
  );
  const forestFitSignature = useMemo(() => {
    if (!tasks.length) {
      return null;
    }
    const nodeIds = tasks
      .map((task) => task.nodeId)
      .sort()
      .join(",");
    return `${sourceBot || "all"}:${pinnedThreadSignature}:${nodeIds}`;
  }, [pinnedThreadSignature, sourceBot, tasks]);
  const nodesById = useMemo(
    () => new Map(layout.nodes.map((node) => [node.task.nodeId, node])),
    [layout.nodes],
  );
  const selectedTask = selectedNodeId ? nodesById.get(selectedNodeId)?.task ?? null : null;
  const selectedPath = useMemo(() => {
    if (!selectedTask) {
      return [];
    }
    const byId = new Map(tasks.map((task) => [task.nodeId, task]));
    const byNumber = new Map(
      tasks
        .filter(isTaskNode)
        .map((task) => [task.number, task] as const),
    );
    const path: DesktopTaskForestNode[] = [];
    let current: DesktopTaskForestNode | null = selectedTask;
    const seen = new Set<string>();
    while (current && !seen.has(current.nodeId)) {
      path.unshift(current);
      seen.add(current.nodeId);
      if (current.kind === "task" && current.parentNodeId) {
        current = byId.get(current.parentNodeId) ?? null;
        continue;
      }
      const parentNumber = parentNumberForNavigation(current);
      current = parentNumber ? byNumber.get(parentNumber) ?? null : null;
    }
    return path;
  }, [selectedTask, tasks]);

  const statusCounts = useMemo(() => {
    const counts: Record<DesktopTaskStatus, number> = {
      todo: 0,
      in_progress: 0,
      in_review: 0,
      done: 0,
    };
    for (const task of tasks) {
      if (task.kind !== "task") {
        continue;
      }
      counts[task.status] += 1;
    }
    return counts;
  }, [tasks]);

  const startForestRequest = useCallback(
    (silent: boolean) => {
      if (!pinnedThreadIds.length) {
        if (mountedRef.current) {
          setTasks([]);
          setRootThreadIds([]);
          setSkippedPinnedThreadIds([]);
          setProjectionCurrent(true);
          setSelectedNodeId(null);
          setCursorNodeId(null);
          setError(null);
          setLoading(false);
        }
        requestRef.current = null;
        currentRequestSequenceRef.current = null;
        return Promise.resolve();
      }
      const requestSequence = ++requestSequenceRef.current;
      currentRequestSequenceRef.current = requestSequence;
      const request = (async () => {
        if (!silent && mountedRef.current) {
          setLoading(true);
        }
        if (mountedRef.current) {
          setError(null);
        }
        try {
          const page = await getDesktopApi().listTaskForest({
            includeDone: true,
            sourceBot,
          });
          if (!mountedRef.current || skipResultForRequestRef.current === requestSequence) {
            return;
          }
          setTasks(page.tasks);
          setRootThreadIds(page.rootThreadIds);
          setSkippedPinnedThreadIds(page.skippedPinnedThreadIds);
          setProjectionCurrent(page.projectionCurrent);
          setSelectedNodeId((current) =>
            current && page.tasks.some((task) => task.nodeId === current)
              ? current
              : null,
          );
          setCursorNodeId((current) =>
            current && page.tasks.some((task) => task.nodeId === current)
              ? current
              : page.tasks[0]?.nodeId ?? null,
          );
        } catch (loadError) {
          if (!mountedRef.current || skipResultForRequestRef.current === requestSequence) {
            return;
          }
          setError(
            loadError instanceof Error
              ? loadError.message
              : String(loadError || "Failed to load task forest"),
          );
        } finally {
          if (skipResultForRequestRef.current === requestSequence) {
            skipResultForRequestRef.current = null;
          }
          if (currentRequestSequenceRef.current === requestSequence) {
            currentRequestSequenceRef.current = null;
            requestRef.current = null;
          }
          if (!silent && mountedRef.current) {
            setLoading(false);
          }
        }
      })();
      requestRef.current = request;
      return request;
    },
    [pinnedThreadIds.length, sourceBot],
  );

  const loadForest = useCallback(
    (options?: { silent?: boolean; force?: boolean }) => {
      const silent = Boolean(options?.silent);
      const currentRequest = requestRef.current;
      if (!pinnedThreadIds.length) {
        if (currentRequestSequenceRef.current !== null) {
          skipResultForRequestRef.current = currentRequestSequenceRef.current;
        }
        return startForestRequest(silent);
      }
      if (currentRequest) {
        if (!options?.force) {
          return currentRequest;
        }
        skipResultForRequestRef.current = currentRequestSequenceRef.current;
        if (!trailingRequestRef.current) {
          trailingRequestRef.current = currentRequest
            .catch(() => undefined)
            .then(() => startForestRequest(silent))
            .finally(() => {
              trailingRequestRef.current = null;
            });
        }
        return trailingRequestRef.current;
      }
      return startForestRequest(silent);
    },
    [pinnedThreadIds.length, startForestRequest],
  );

  useEffect(() => {
    void loadForest({ force: true });
  }, [loadForest, pinnedThreadSignature, pinnedThreadsVersion]);

  useEffect(() => {
    const interval = window.setInterval(() => {
      void loadForest({ silent: true });
    }, REFRESH_INTERVAL_MS);
    const refreshOnFocus = () => {
      void loadForest({ silent: true });
    };
    window.addEventListener("focus", refreshOnFocus);
    return () => {
      window.clearInterval(interval);
      window.removeEventListener("focus", refreshOnFocus);
    };
  }, [loadForest]);

  const fitForest = useCallback(() => {
    const stage = stageRef.current;
    if (!stage || layout.nodes.length === 0) {
      return;
    }
    const rect = stage.getBoundingClientRect();
    const width = Math.max(1, layout.bbox.maxX - layout.bbox.minX);
    const height = Math.max(1, layout.bbox.maxY - layout.bbox.minY);
    const z = Math.min(
      1.15,
      Math.max(0.34, Math.min((rect.width - 96) / width, (rect.height - 96) / height)),
    );
    moveCamera(
      {
        z,
        x: (rect.width - width * z) / 2 - layout.bbox.minX * z,
        y: (rect.height - height * z) / 2 - layout.bbox.minY * z,
      },
      true,
    );
  }, [layout, moveCamera]);

  useEffect(() => {
    if (!forestFitSignature) {
      fittedSignatureRef.current = null;
      return;
    }
    if (!loading && layout.nodes.length && fittedSignatureRef.current !== forestFitSignature) {
      fittedSignatureRef.current = forestFitSignature;
      fitForest();
    }
  }, [fitForest, forestFitSignature, layout.nodes.length, loading]);

  const openTask = useCallback(
    async (task: DesktopTaskForestNode) => {
      const requestSequence = ++openTaskRequestRef.current;
      setSelectedNodeId(task.nodeId);
      setCursorNodeId(task.nodeId);
      const opened = await onOpenThreadInPanel(task.threadId);
      if (requestSequence !== openTaskRequestRef.current) {
        return;
      }
      if (!opened) {
        setSelectedNodeId((current) => (current === task.nodeId ? null : current));
        onToast(t("Thread not found."), "error");
      }
    },
    [onOpenThreadInPanel, onToast, t],
  );

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (isEditableEventTarget(event.target)) {
        return;
      }
      if (event.key === " ") {
        event.preventDefault();
        setSpacePanning(true);
        return;
      }
      const ordered = layout.nodes
        .slice()
        .sort((left, right) => left.y - right.y || left.x - right.x);
      if (!ordered.length) {
        return;
      }
      const currentIndex = Math.max(
        0,
        ordered.findIndex((node) => node.task.nodeId === cursorNodeId),
      );
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        const delta = event.key === "ArrowDown" ? 1 : -1;
        const next = ordered[(currentIndex + delta + ordered.length) % ordered.length];
        setCursorNodeId(next.task.nodeId);
      } else if (event.key === "ArrowRight") {
        event.preventDefault();
        const current = cursorNodeId ? nodesById.get(cursorNodeId)?.task ?? null : null;
        const currentTaskNumber = isTaskNode(current) ? current.number : null;
        const child = layout.nodes.find((node) => {
          const candidate = node.task;
          if (candidate.kind !== "task") {
            return false;
          }
          if (candidate.parentNodeId) {
            return candidate.parentNodeId === cursorNodeId;
          }
          return (
            currentTaskNumber !== null &&
            parentNumberForNavigation(candidate) === currentTaskNumber
          );
        });
        if (child) {
          setCursorNodeId(child.task.nodeId);
        }
      } else if (event.key === "ArrowLeft") {
        event.preventDefault();
        const current = cursorNodeId ? nodesById.get(cursorNodeId)?.task : null;
        const parent =
          current?.kind === "task" && current.parentNodeId
            ? nodesById.get(current.parentNodeId)
            : current?.kind === "task"
              ? layout.nodes.find(
                  (node) =>
                    node.task.kind === "task" &&
                    node.task.number === parentNumberForNavigation(current),
                )
              : null;
        if (parent) {
          setCursorNodeId(parent.task.nodeId);
        }
      } else if (event.key === "Enter") {
        const task = cursorNodeId ? nodesById.get(cursorNodeId)?.task : null;
        if (task) {
          event.preventDefault();
          void openTask(task);
        }
      }
    };
    window.addEventListener("keydown", onKeyDown);
    const onKeyUp = (event: KeyboardEvent) => {
      if (event.key === " ") {
        setSpacePanning(false);
      }
    };
    window.addEventListener("keyup", onKeyUp);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
    };
  }, [cursorNodeId, layout.nodes, nodesById, openTask]);

  function zoomAt(event: WheelEvent<HTMLDivElement>) {
    if (!event.ctrlKey && !event.metaKey) {
      event.preventDefault();
      moveCamera({
        ...camera,
        x: camera.x - event.deltaX,
        y: camera.y - event.deltaY,
      });
      return;
    }
    event.preventDefault();
    const rect = event.currentTarget.getBoundingClientRect();
    const nextZ = Math.max(0.32, Math.min(1.5, camera.z * (event.deltaY > 0 ? 0.9 : 1.1)));
    const worldX = (event.clientX - rect.left - camera.x) / camera.z;
    const worldY = (event.clientY - rect.top - camera.y) / camera.z;
    moveCamera({
      z: nextZ,
      x: event.clientX - rect.left - worldX * nextZ,
      y: event.clientY - rect.top - worldY * nextZ,
    });
  }

  async function updateSelectedStatus(status: DesktopTaskStatus) {
    if (!isTaskNode(selectedTask) || selectedTask.status === status) {
      return;
    }
    setMutatingTaskId(displayTaskId(selectedTask));
    try {
      await getDesktopApi().updateTaskStatus({
        taskId: displayTaskId(selectedTask),
        status,
      });
      await loadForest({ silent: true, force: true });
      onToast(t("Task updated."), "success");
    } catch (statusError) {
      onToast(
        statusError instanceof Error ? statusError.message : t("Task update failed."),
        "error",
      );
    } finally {
      setMutatingTaskId(null);
    }
  }

  const taskNodeCount = tasks.filter(isTaskNode).length;
  const activeNodeCount = tasks.filter(isActiveRun).length;
  const readout = `${Math.round(camera.z * 100)}%`;
  const worldWidth = Math.max(1, layout.bbox.maxX + 80);
  const worldHeight = Math.max(1, layout.bbox.maxY + 80);
  const birdseye = camera.z < 0.58;
  const worldViewport = useMemo(() => {
    if (!stageSize.width || !stageSize.height || camera.z <= 0) {
      return null;
    }
    return {
      minX: -camera.x / camera.z,
      minY: -camera.y / camera.z,
      maxX: (stageSize.width - camera.x) / camera.z,
      maxY: (stageSize.height - camera.y) / camera.z,
    };
  }, [camera, stageSize.height, stageSize.width]);
  const visibleNodeIds = useMemo(() => {
    if (!worldViewport || layout.nodes.length <= CULLING_NODE_THRESHOLD) {
      return null;
    }
    return visibleTaskForestNodeNumbers(
      layout.nodes,
      worldViewport,
      CULLING_OVERSCAN_PX / camera.z,
    );
  }, [camera.z, layout.nodes, worldViewport]);
  const visibleNodes = useMemo(
    () =>
      visibleNodeIds
        ? layout.nodes.filter((node) => visibleNodeIds.has(node.task.nodeId))
        : layout.nodes,
    [layout.nodes, visibleNodeIds],
  );
  const visibleEdges = useMemo(
    () =>
      visibleNodeIds
        ? layout.edges.filter(
            (edge) => visibleNodeIds.has(edge.from) || visibleNodeIds.has(edge.to),
          )
        : layout.edges,
    [layout.edges, visibleNodeIds],
  );
  const minimapViewport = useMemo(() => {
    if (!worldViewport) {
      return null;
    }
    const bboxWidth = Math.max(1, layout.bbox.maxX - layout.bbox.minX);
    const bboxHeight = Math.max(1, layout.bbox.maxY - layout.bbox.minY);
    const rawLeft = ((worldViewport.minX - layout.bbox.minX) / bboxWidth) * MINIMAP_PLOT_WIDTH;
    const rawTop = ((worldViewport.minY - layout.bbox.minY) / bboxHeight) * MINIMAP_PLOT_HEIGHT;
    const rawRight = ((worldViewport.maxX - layout.bbox.minX) / bboxWidth) * MINIMAP_PLOT_WIDTH;
    const rawBottom =
      ((worldViewport.maxY - layout.bbox.minY) / bboxHeight) * MINIMAP_PLOT_HEIGHT;
    const left = Math.max(0, Math.min(MINIMAP_PLOT_WIDTH, rawLeft));
    const top = Math.max(0, Math.min(MINIMAP_PLOT_HEIGHT, rawTop));
    const right = Math.max(0, Math.min(MINIMAP_PLOT_WIDTH, rawRight));
    const bottom = Math.max(0, Math.min(MINIMAP_PLOT_HEIGHT, rawBottom));
    if (right <= 0 || bottom <= 0 || left >= MINIMAP_PLOT_WIDTH || top >= MINIMAP_PLOT_HEIGHT) {
      return null;
    }
    return {
      left,
      top,
      width: Math.max(4, right - left),
      height: Math.max(4, bottom - top),
    };
  }, [layout.bbox, worldViewport]);

  const centerMinimapAt = useCallback(
    (event: MouseEvent<HTMLDivElement>) => {
      if (!stageSize.width || !stageSize.height) {
        return;
      }
      const rect = event.currentTarget.getBoundingClientRect();
      const plotX = Math.max(
        0,
        Math.min(MINIMAP_PLOT_WIDTH, ((event.clientX - rect.left) / rect.width) * MINIMAP_PLOT_WIDTH),
      );
      const plotY = Math.max(
        0,
        Math.min(
          MINIMAP_PLOT_HEIGHT,
          ((event.clientY - rect.top) / rect.height) * MINIMAP_PLOT_HEIGHT,
        ),
      );
      const bboxWidth = Math.max(1, layout.bbox.maxX - layout.bbox.minX);
      const bboxHeight = Math.max(1, layout.bbox.maxY - layout.bbox.minY);
      const worldX = layout.bbox.minX + (plotX / MINIMAP_PLOT_WIDTH) * bboxWidth;
      const worldY = layout.bbox.minY + (plotY / MINIMAP_PLOT_HEIGHT) * bboxHeight;
      moveCamera(
        {
          z: camera.z,
          x: stageSize.width / 2 - worldX * camera.z,
          y: stageSize.height / 2 - worldY * camera.z,
        },
        true,
      );
    },
    [camera.z, layout.bbox, moveCamera, stageSize.height, stageSize.width],
  );

  return (
    <div
      className={`task-forest-console ${selectedTask ? "panel-open" : ""} ${spacePanning ? "space-panning" : ""}`}
    >
      <header className="task-forest-topbar">
        <div className="task-forest-brand">
          <span className="task-forest-brand-mark">
            <GitBranch aria-hidden size={14} strokeWidth={2} />
          </span>
          <span>{t("Task Forest")}</span>
        </div>
        <div className="task-forest-status-chips">
          {STATUS_ORDER.map((status) => {
            const meta = STATUS_META[status];
            return (
              <button
                className={`task-forest-chip tone-${meta.tone} ${statusFilter === status ? "active" : ""}`}
                key={status}
                onClick={() =>
                  setStatusFilter((current) => (current === status ? null : status))
                }
                type="button"
              >
                <span className="task-forest-chip-dot" />
                {t(meta.label)}
                <span className="mono">{statusCounts[status]}</span>
              </button>
            );
          })}
        </div>
        <nav className="task-forest-breadcrumb" aria-label={t("Task path")}>
          {selectedPath.length ? (
            selectedPath.map((task, index) => (
              <button
                key={task.nodeId}
                onClick={() => void openTask(task)}
                type="button"
              >
                {index > 0 ? <span aria-hidden>/</span> : null}
                {displayNodeId(task)}
              </button>
            ))
          ) : (
            <span>{projectionCurrent ? t("Pinned roots") : t("Projection refreshing")}</span>
          )}
        </nav>
        <div className="task-forest-actions">
          <span className="task-forest-readout">{readout}</span>
          <button className="task-forest-icon-button" onClick={fitForest} title={t("Fit forest")} type="button">
            <Maximize2 aria-hidden size={14} />
          </button>
          <button
            className="task-forest-icon-button"
            disabled={loading}
            onClick={() => void loadForest({ force: true })}
            title={t("Refresh")}
            type="button"
          >
            <RefreshCcw aria-hidden size={14} />
          </button>
        </div>
      </header>

      <section
        className="task-forest-stage"
        onPointerDown={(event) => {
          const target = event.target;
          const isCanvasSurface =
            target === event.currentTarget ||
            (target instanceof HTMLElement &&
              target.classList.contains("task-forest-world"));
          if (!spacePanning && !isCanvasSurface) {
            return;
          }
          event.preventDefault();
          event.currentTarget.setPointerCapture(event.pointerId);
          dragRef.current = {
            pointerId: event.pointerId,
            startX: event.clientX,
            startY: event.clientY,
            camera,
          };
        }}
        onPointerMove={(event) => {
          const drag = dragRef.current;
          if (!drag || drag.pointerId !== event.pointerId) {
            return;
          }
          moveCamera({
            ...drag.camera,
            x: drag.camera.x + event.clientX - drag.startX,
            y: drag.camera.y + event.clientY - drag.startY,
          });
        }}
        onPointerUp={() => {
          dragRef.current = null;
        }}
        onPointerCancel={() => {
          dragRef.current = null;
        }}
        onWheel={zoomAt}
        ref={stageRef}
      >
        {loading && !tasks.length ? (
          <div className="task-forest-state">{t("Loading tasks…")}</div>
        ) : error ? (
          <div className="task-forest-state error">{error}</div>
        ) : !tasks.length ? (
          <div className="task-forest-state">
            {pinnedThreadIds.length
              ? t("Pinned conversations with tasks will appear here.")
              : t("Pin conversations to add them to the operation room.")}
          </div>
        ) : null}
        <div
          className={`task-forest-world ${birdseye ? "birdseye" : ""} ${smoothCamera ? "smooth-camera" : ""}`}
          role="tree"
          style={{
            width: worldWidth,
            height: worldHeight,
            transform: `translate(${camera.x}px, ${camera.y}px) scale(${camera.z})`,
          }}
        >
          <svg
            aria-hidden
            className="task-forest-edges"
            height={worldHeight}
            viewBox={`0 0 ${worldWidth} ${worldHeight}`}
            width={worldWidth}
          >
            {visibleEdges.map((edge) => (
              <path
                className={`task-forest-edge ${edge.active ? "active" : ""}`}
                d={edge.path}
                key={`${edge.from}:${edge.to}`}
              />
            ))}
          </svg>
          {visibleNodes.map((node) => {
            const task = node.task;
            const taskNode = isTaskNode(task);
            const meta = taskNode ? STATUS_META[task.status] : null;
            const dimmed = Boolean(taskNode && statusFilter && task.status !== statusFilter);
            const active = isActiveRun(task);
            const failed = isFailedRun(task);
            return (
              <button
                aria-current={selectedNodeId === task.nodeId ? "true" : undefined}
                className={`task-forest-node ${taskNode ? `tone-${meta?.tone}` : "kind-thread tone-thread"} ${selectedNodeId === task.nodeId ? "selected" : ""} ${cursorNodeId === task.nodeId ? "cursor" : ""} ${active ? "active-run" : ""} ${failed ? "failed-run" : ""} ${dimmed ? "dimmed" : ""}`}
                key={task.nodeId}
                onClick={() => void openTask(task)}
                role="treeitem"
                style={{
                  left: node.x,
                  top: node.y,
                  width: node.width,
                  minHeight: node.height,
                }}
                type="button"
              >
                <span className="task-forest-node-row">
                  <span className="task-forest-pill">
                    {taskNode && meta ? (
                      <meta.Icon aria-hidden size={11} strokeWidth={2} />
                    ) : (
                      <MessageSquare aria-hidden size={11} strokeWidth={2} />
                    )}
                    {taskNode && meta ? t(meta.label) : t("Thread")}
                    {active ? <span className="task-forest-pulse-dot" /> : null}
                  </span>
                  <span className="task-forest-node-id">{displayNodeId(task)}</span>
                </span>
                <span className="task-forest-node-title">{task.title}</span>
                <span className="task-forest-node-meta">
                  <span className={`task-forest-avatar ${active ? "active" : ""}`}>
                    {initials(threadRootLabel(task))}
                  </span>
                  <span>{threadRootLabel(task)}</span>
                  <span>{taskNode ? task.replyCount : task.messageCount}</span>
                  {node.hiddenDescendantCount ? (
                    <span className="task-forest-rollup">
                      +{node.hiddenDescendantCount}
                    </span>
                  ) : null}
                </span>
              </button>
            );
          })}
        </div>
        <div className="task-forest-legend">
          <span>{t("{count} tasks", { count: taskNodeCount })}</span>
          <span>{t("{count} roots", { count: rootThreadIds.length })}</span>
          <span>{activeNodeCount ? t("{count} running", { count: activeNodeCount }) : t("Idle")}</span>
          {skippedPinnedThreadIds.length ? (
            <span>{t("{count} chats skipped", { count: skippedPinnedThreadIds.length })}</span>
          ) : null}
        </div>
        <div
          aria-hidden
          className="task-forest-minimap"
          onClick={centerMinimapAt}
        >
          {layout.nodes.map((node) => {
            const left =
              ((node.x - layout.bbox.minX) /
                Math.max(1, layout.bbox.maxX - layout.bbox.minX)) *
              150;
            const top =
              ((node.y - layout.bbox.minY) /
                Math.max(1, layout.bbox.maxY - layout.bbox.minY)) *
              82;
            return (
              <span
                className={`${
                  isTaskNode(node.task) ? `tone-${STATUS_META[node.task.status].tone}` : "tone-thread"
                } ${isActiveRun(node.task) ? "active" : ""} ${isFailedRun(node.task) ? "failed-run" : ""} ${statusFilter && isTaskNode(node.task) && node.task.status !== statusFilter ? "dimmed" : ""}`}
                key={node.task.nodeId}
                style={{ left, top }}
              />
            );
          })}
          {minimapViewport ? (
            <div
              className="task-forest-minimap-viewport"
              style={{
                left: minimapViewport.left,
                top: minimapViewport.top,
                width: minimapViewport.width,
                height: minimapViewport.height,
              }}
            />
          ) : null}
        </div>
      </section>

      {selectedTask ? (
        <aside className="task-forest-thread-panel">
          <header className="task-forest-panel-header">
            <div className="task-forest-panel-title">
              <span className="task-forest-panel-id">{displayNodeId(selectedTask)}</span>
              <h2>{selectedTask.title}</h2>
              <span>{threadRootLabel(selectedTask)}</span>
            </div>
            <div className="task-forest-panel-actions">
              {isTaskNode(selectedTask) ? (
                <select
                  aria-label={t("Task status")}
                  className="task-forest-status-select"
                  disabled={mutatingTaskId === displayTaskId(selectedTask)}
                  onChange={(event) =>
                    void updateSelectedStatus(event.target.value as DesktopTaskStatus)
                  }
                  value={selectedTask.status}
                >
                  {STATUS_ORDER.map((status) => (
                    <option key={status} value={status}>
                      {t(STATUS_META[status].label)}
                    </option>
                  ))}
                </select>
              ) : null}
              <button
                className="task-forest-icon-button"
                onClick={() => setSelectedNodeId(null)}
                title={t("Close")}
                type="button"
              >
                <PanelRightClose aria-hidden size={14} />
              </button>
            </div>
          </header>
          <div className="task-forest-thread-body">
            {selectedThreadId === selectedTask.threadId ? (
              selectedThreadPanel
            ) : (
              <div className="task-forest-thread-empty">
                <MessageSquare aria-hidden size={18} />
                {t("Opening thread…")}
              </div>
            )}
          </div>
        </aside>
      ) : null}
    </div>
  );
}
