import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ListTree } from "lucide-react";

import type {
  DesktopTaskForestNode,
  DesktopTaskForestTaskNode,
} from "@shared/contracts";

import { getDesktopApi } from "../../platform/desktop-api";
import { useI18n } from "../../i18n";
import { AgentOptionAvatar } from "./AgentOptionAvatar";

const REFRESH_MS = 5000;

function isTaskNode(
  node: DesktopTaskForestNode,
): node is DesktopTaskForestTaskNode {
  return node.kind === "task";
}

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

type TaskRow = { task: DesktopTaskForestTaskNode; depth: number };

/** Depth-order the tasks into a tree via parentNodeId; nodes whose parent
 *  isn't in the set become roots (depth 0). */
function buildTaskRows(tasks: DesktopTaskForestTaskNode[]): TaskRow[] {
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
  const rows: TaskRow[] = [];
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

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const load = useCallback(async () => {
    if (!threadId) {
      setTasks([]);
      return;
    }
    try {
      const page = await getDesktopApi().listTaskForest({
        rootThreadId: threadId,
      });
      if (!mountedRef.current) {
        return;
      }
      setTasks(
        page.tasks
          .filter(isTaskNode)
          .filter(
            (task) =>
              task.status === "in_progress" || task.status === "in_review",
          ),
      );
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

  // Nothing to show when this conversation hasn't spawned any active task.
  if (!threadId || tasks.length === 0) {
    return null;
  }

  return (
    <div className="thread-subtask-pop">
      <div className="thread-subtask-head">
        <ListTree aria-hidden size={13} />
        <span className="thread-subtask-head-title">{t("Subtask tree")}</span>
        <span className="thread-subtask-count">{tasks.length}</span>
      </div>
      <div className="thread-subtask-list">
        {rows.map(({ task, depth }) => {
          const tone = task.status === "in_review" ? "review" : "progress";
          return (
            <button
              key={task.nodeId}
              className={`thread-subtask-item depth-${depth}`}
              onClick={() => onOpenThread(task.threadId)}
              type="button"
            >
              <span className="thread-subtask-main">
                <span className="thread-subtask-row1">
                  <span className="thread-subtask-num mono">{displayTaskId(task)}</span>
                  <span className="thread-subtask-title">{task.title}</span>
                </span>
                <span className="thread-subtask-row2">
                  <AgentOptionAvatar agentId={assigneeAgentId(task)} size="sm" />
                  <span className="thread-subtask-agent">{assigneeLabel(task)}</span>
                  <span className={`thread-subtask-status tone-${tone}`}>
                    {task.status === "in_review" ? t("In Review") : t("In Progress")}
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
