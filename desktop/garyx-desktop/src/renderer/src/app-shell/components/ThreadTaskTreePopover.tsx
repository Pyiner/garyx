import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { ListTree, MessageSquare } from "lucide-react";

import type {
  DesktopTaskForestNode,
  DesktopTaskForestTaskNode,
  DesktopTaskForestThreadNode,
} from "@shared/contracts";

import { getDesktopApi } from "../../platform/desktop-api";
import { useI18n } from "../../i18n";
import {
  resolveTaskAvatarIdentity,
  resolveThreadAvatarIdentity,
  type ThreadAvatarCatalog,
} from "../../thread-avatar";
import { AgentOptionAvatar } from "./AgentOptionAvatar";
import {
  buildTaskRows,
  isCurrentTaskTreeNode,
  resolveTaskTreeActiveCount,
  taskStatusLabel,
  taskStatusTone,
} from "./thread-task-tree-popover-model";

const REFRESH_MS = 5000;

function displayTaskId(task: DesktopTaskForestTaskNode): string {
  return task.taskId || `#TASK-${task.number}`;
}

type ThreadTaskTreePopoverProps = {
  taskTreeDocked: boolean;
  threadId: string | null;
  threadAvatarCatalog: ThreadAvatarCatalog;
  onOpenThread: (threadId: string) => void;
};

type ForestSnapshot = {
  nodes: DesktopTaskForestNode[];
  activeCount: number | null;
  /** Cache identity of the fetched tree (origin thread, else task root). */
  treeKey: string | null;
};

const EMPTY_SNAPSHOT: ForestSnapshot = {
  nodes: [],
  activeCount: null,
  treeKey: null,
};

// The origin-rooted forest is anchor-independent: every thread of one tree
// resolves to the same node set, so snapshots cache per tree and switching
// between threads of one tree renders instantly from cache while the live
// fetch revalidates in place (stale-while-revalidate). `treeKeyByAnchor`
// learns anchor→tree from responses and is pre-seeded on row clicks so
// in-tree navigation never starts from an empty popover.
const treeKeyByAnchor = new Map<string, string>();
const treeSnapshotByKey = new Map<string, ForestSnapshot>();
const MAX_CACHED_TREES = 16;

function cachedSnapshotFor(anchorThreadId: string): ForestSnapshot | null {
  const key = treeKeyByAnchor.get(anchorThreadId);
  return (key ? treeSnapshotByKey.get(key) : undefined) ?? null;
}

function storeTreeSnapshot(anchorThreadId: string, snapshot: ForestSnapshot) {
  const key = snapshot.treeKey;
  if (!key) {
    return;
  }
  treeKeyByAnchor.set(anchorThreadId, key);
  // Re-insert to keep Map iteration order as a recency list for eviction.
  treeSnapshotByKey.delete(key);
  treeSnapshotByKey.set(key, snapshot);
  while (treeSnapshotByKey.size > MAX_CACHED_TREES) {
    const oldest = treeSnapshotByKey.keys().next().value;
    if (oldest === undefined) {
      break;
    }
    treeSnapshotByKey.delete(oldest);
  }
}

export function ThreadTaskTreePopover({
  taskTreeDocked,
  threadId,
  threadAvatarCatalog,
  onOpenThread,
}: ThreadTaskTreePopoverProps) {
  const { t } = useI18n();
  const [snapshot, setSnapshot] = useState<ForestSnapshot>(EMPTY_SNAPSHOT);
  const [compactOpen, setCompactOpen] = useState(false);
  const [triggerHost, setTriggerHost] = useState<HTMLElement | null>(null);
  const compactTriggerRef = useRef<HTMLButtonElement | null>(null);
  const popoverRef = useRef<HTMLDivElement | null>(null);
  const popoverId = useId();
  const mountedRef = useRef(true);
  const currentThreadRef = useRef<string | null>(threadId);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    currentThreadRef.current = threadId;
    setCompactOpen(false);
  }, [threadId]);

  useEffect(() => {
    if (taskTreeDocked) {
      setCompactOpen(false);
    }
  }, [taskTreeDocked]);

  useEffect(() => {
    setTriggerHost(
      document.querySelector<HTMLElement>(
        "[data-thread-task-tree-trigger-host]",
      ),
    );
  }, []);

  useEffect(() => {
    if (!compactOpen) {
      return;
    }

    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) {
        return;
      }
      if (
        compactTriggerRef.current?.contains(target) ||
        popoverRef.current?.contains(target)
      ) {
        return;
      }
      setCompactOpen(false);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") {
        return;
      }
      setCompactOpen(false);
      compactTriggerRef.current?.focus();
    };

    document.addEventListener("pointerdown", handlePointerDown, true);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown, true);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [compactOpen]);

  const load = useCallback(async () => {
    if (!threadId) {
      setSnapshot(EMPTY_SNAPSHOT);
      return;
    }
    try {
      const page = await getDesktopApi().listTaskForest({
        anchorThreadId: threadId,
      });
      if (!mountedRef.current || currentThreadRef.current !== threadId) {
        return;
      }
      const next: ForestSnapshot = {
        nodes: page.tasks,
        activeCount: page.activeCount ?? null,
        treeKey: page.rootThreadIds?.[0] || threadId,
      };
      storeTreeSnapshot(threadId, next);
      setSnapshot(next);
    } catch {
      /* leave previous state on transient errors */
    }
  }, [threadId]);

  // Thread switches restore the cached tree snapshot (no flicker, no refetch
  // gap) and revalidate it with a live fetch; unknown trees start empty.
  useEffect(() => {
    setSnapshot(threadId ? (cachedSnapshotFor(threadId) ?? EMPTY_SNAPSHOT) : EMPTY_SNAPSHOT);
    void load();
  }, [threadId, load]);

  useEffect(() => {
    const interval = window.setInterval(() => void load(), REFRESH_MS);
    return () => window.clearInterval(interval);
  }, [load]);

  const rows = useMemo(() => buildTaskRows(snapshot.nodes), [snapshot.nodes]);
  const activeCount = useMemo(
    () =>
      resolveTaskTreeActiveCount({
        tasks: snapshot.nodes,
        activeCount: snapshot.activeCount,
      }),
    [snapshot],
  );

  // Nothing to show when this conversation has no anchored task tree.
  if (!threadId || rows.length === 0) {
    return null;
  }

  // Pre-seed the anchor→tree index before navigating: the clicked row belongs
  // to the tree already on screen, so the popover anchored at the target
  // thread renders this snapshot instantly instead of refetching.
  const openFromRow = (targetThreadId: string) => {
    if (snapshot.treeKey) {
      treeKeyByAnchor.set(targetThreadId, snapshot.treeKey);
    }
    setCompactOpen(false);
    onOpenThread(targetThreadId);
  };

  const renderThreadRow = (thread: DesktopTaskForestThreadNode, depth: number) => {
    const current = isCurrentTaskTreeNode(thread, threadId);
    const avatar = resolveThreadAvatarIdentity(
      { title: thread.title, agentId: thread.agentId },
      threadAvatarCatalog,
    );
    return (
      <button
        key={thread.nodeId}
        className={`thread-subtask-item thread-subtask-thread depth-${depth}${current ? " current" : ""}`}
        onClick={() => openFromRow(thread.threadId)}
        type="button"
      >
        <span className="thread-subtask-main">
          <span className="thread-subtask-row1">
            <MessageSquare aria-hidden className="thread-subtask-thread-glyph" size={13} />
            <span className="thread-subtask-title">{thread.title}</span>
            {current ? (
              <span className="thread-subtask-current">{t("Current")}</span>
            ) : null}
          </span>
          <span className="thread-subtask-row2">
            <AgentOptionAvatar
              agentId={avatar.agentId}
              avatarDataUrl={avatar.avatarDataUrl}
              kind={avatar.kind}
              label={avatar.label}
              providerIcon={avatar.providerIcon}
              providerType={avatar.providerType}
              size="sm"
            />
            <span className="thread-subtask-agent">{avatar.label}</span>
          </span>
        </span>
      </button>
    );
  };

  const renderTaskRow = (task: DesktopTaskForestTaskNode, depth: number) => {
    const tone = taskStatusTone(task.status);
    const current = isCurrentTaskTreeNode(task, threadId);
    const avatar = resolveTaskAvatarIdentity(task, threadAvatarCatalog);
    return (
      <button
        key={task.nodeId}
        className={`thread-subtask-item depth-${depth} tone-${tone}${current ? " current" : ""}`}
        onClick={() => openFromRow(task.threadId)}
        type="button"
      >
        <span className="thread-subtask-main">
          <span className="thread-subtask-row1">
            <span className="thread-subtask-num mono">
              {displayTaskId(task)}
            </span>
            <span className="thread-subtask-title">{task.title}</span>
            {current ? (
              <span className="thread-subtask-current">
                {t("Current")}
              </span>
            ) : null}
          </span>
          <span className="thread-subtask-row2">
            <AgentOptionAvatar
              agentId={avatar.agentId}
              avatarDataUrl={avatar.avatarDataUrl}
              kind={avatar.kind}
              label={avatar.label}
              providerIcon={avatar.providerIcon}
              providerType={avatar.providerType}
              size="sm"
            />
            <span className="thread-subtask-agent">
              {avatar.label}
            </span>
            <span className={`thread-subtask-status tone-${tone}`}>
              {t(taskStatusLabel(task.status))}
            </span>
          </span>
        </span>
      </button>
    );
  };

  return (
    <>
      {triggerHost && !taskTreeDocked
        ? createPortal(
            <button
              aria-controls={popoverId}
              aria-expanded={compactOpen}
              aria-label={t("Task tree")}
              className={`thread-subtask-toggle${compactOpen ? " is-open" : ""}${activeCount > 0 ? " has-active" : ""}`}
              onClick={() => setCompactOpen((current) => !current)}
              ref={compactTriggerRef}
              type="button"
            >
              <ListTree aria-hidden size={14} />
            </button>,
            triggerHost,
          )
        : null}
      <div
        className={`thread-subtask-pop${compactOpen ? " is-compact-open" : ""}`}
        id={popoverId}
        ref={popoverRef}
      >
        <div className="thread-subtask-head">
          <ListTree aria-hidden size={13} />
          <span className="thread-subtask-head-title">{t("Task tree")}</span>
          {activeCount > 0 ? (
            <span className="thread-subtask-count">{activeCount}</span>
          ) : null}
        </div>
        <div className="thread-subtask-list">
          {rows.map((row) =>
            row.kind === "thread"
              ? renderThreadRow(row.thread, row.depth)
              : renderTaskRow(row.task, row.depth),
          )}
        </div>
      </div>
    </>
  );
}
