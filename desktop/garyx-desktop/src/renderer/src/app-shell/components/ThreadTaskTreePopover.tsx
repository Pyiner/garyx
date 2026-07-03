import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
  threadId: string | null;
  threadAvatarCatalog: ThreadAvatarCatalog;
  onOpenThread: (threadId: string) => void;
};

type ForestSnapshot = {
  nodes: DesktopTaskForestNode[];
  activeCount: number | null;
};

const EMPTY_SNAPSHOT: ForestSnapshot = { nodes: [], activeCount: null };

export function ThreadTaskTreePopover({
  threadId,
  threadAvatarCatalog,
  onOpenThread,
}: ThreadTaskTreePopoverProps) {
  const { t } = useI18n();
  const [snapshot, setSnapshot] = useState<ForestSnapshot>(EMPTY_SNAPSHOT);
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
  }, [threadId]);

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
      setSnapshot({ nodes: page.tasks, activeCount: page.activeCount ?? null });
    } catch {
      /* leave previous state on transient errors */
    }
  }, [threadId]);

  // Reset + reload whenever the active thread changes so the list always
  // reflects the conversation currently open in the detail pane.
  useEffect(() => {
    setSnapshot(EMPTY_SNAPSHOT);
    void load();
  }, [load]);

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
        onClick={() => onOpenThread(thread.threadId)}
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
            <span className="thread-subtask-agent">{t("Conversation")}</span>
          </span>
        </span>
      </button>
    );
  };

  const renderTaskRow = (
    task: DesktopTaskForestTaskNode,
    depth: number,
    isDeemphasized: boolean,
  ) => {
    const tone = taskStatusTone(task.status);
    const current = isCurrentTaskTreeNode(task, threadId);
    const avatar = resolveTaskAvatarIdentity(task, threadAvatarCatalog);
    return (
      <button
        key={task.nodeId}
        className={`thread-subtask-item depth-${depth} tone-${tone}${current ? " current" : ""}${isDeemphasized && !current ? " done" : ""}`}
        onClick={() => onOpenThread(task.threadId)}
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
    <div className="thread-subtask-pop">
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
            : renderTaskRow(row.task, row.depth, row.isDeemphasized),
        )}
      </div>
    </div>
  );
}
