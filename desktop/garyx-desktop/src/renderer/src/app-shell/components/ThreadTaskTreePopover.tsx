import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ListTree } from "lucide-react";

import type { DesktopTaskForestTaskNode } from "@shared/contracts";

import { getDesktopApi } from "../../platform/desktop-api";
import { useI18n } from "../../i18n";
import { AgentOptionAvatar } from "./AgentOptionAvatar";
import {
  buildTaskRows,
  isCurrentTaskTreeNode,
  taskStatusLabel,
  taskStatusTone,
  taskTreeBadgeCount,
  visibleTaskTreeTasks,
} from "./thread-task-tree-popover-model";

const REFRESH_MS = 5000;

function displayTaskId(task: DesktopTaskForestTaskNode): string {
  return task.taskId || `#TASK-${task.number}`;
}

function assigneeAgentId(task: DesktopTaskForestTaskNode): string | null {
  if (task.assignee?.kind === "agent") {
    return task.assignee.agentId;
  }
  return task.runtimeAgentId ?? null;
}

function assigneeLabel(task: DesktopTaskForestTaskNode): string {
  if (task.assignee?.kind === "agent") {
    return task.assignee.agentId;
  }
  if (task.assignee?.kind === "human") {
    return `@${task.assignee.userId}`;
  }
  return task.runtimeAgentId || "unassigned";
}

type ThreadTaskTreePopoverProps = {
  threadId: string | null;
  onOpenThread: (threadId: string) => void;
};

export function ThreadTaskTreePopover({
  threadId,
  onOpenThread,
}: ThreadTaskTreePopoverProps) {
  const { t } = useI18n();
  const [tasks, setTasks] = useState<DesktopTaskForestTaskNode[]>([]);
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
      setTasks([]);
      return;
    }
    try {
      const page = await getDesktopApi().listTaskForest({
        anchorThreadId: threadId,
      });
      if (!mountedRef.current || currentThreadRef.current !== threadId) {
        return;
      }
      setTasks(visibleTaskTreeTasks(page.tasks));
    } catch {
      /* leave previous state on transient errors */
    }
  }, [threadId]);

  // Reset + reload whenever the active thread changes so the list always
  // reflects the conversation currently open in the detail pane.
  useEffect(() => {
    setTasks([]);
    void load();
  }, [load]);

  useEffect(() => {
    const interval = window.setInterval(() => void load(), REFRESH_MS);
    return () => window.clearInterval(interval);
  }, [load]);

  const rows = useMemo(() => buildTaskRows(tasks), [tasks]);
  const activeCount = useMemo(() => taskTreeBadgeCount(tasks), [tasks]);

  // Nothing to show when this conversation has no anchored active task tree.
  if (!threadId || tasks.length === 0) {
    return null;
  }

  return (
    <div className="thread-subtask-pop">
      <div className="thread-subtask-head">
        <ListTree aria-hidden size={13} />
        <span className="thread-subtask-head-title">{t("Task tree")}</span>
        <span className="thread-subtask-count">{activeCount}</span>
      </div>
      <div className="thread-subtask-list">
        {rows.map(({ task, depth }) => {
          const tone = taskStatusTone(task.status);
          const current = isCurrentTaskTreeNode(task, threadId);
          return (
            <button
              key={task.nodeId}
              className={`thread-subtask-item depth-${depth} tone-${tone}${current ? " current" : ""}`}
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
                    agentId={assigneeAgentId(task)}
                    size="sm"
                  />
                  <span className="thread-subtask-agent">
                    {assigneeLabel(task)}
                  </span>
                  <span className={`thread-subtask-status tone-${tone}`}>
                    {t(taskStatusLabel(task.status))}
                  </span>
                </span>
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
