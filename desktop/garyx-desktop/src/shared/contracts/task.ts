import type { DesktopWorkspaceMode } from "./workspace.ts";

export type DesktopTaskStatus = "todo" | "in_progress" | "in_review" | "done";

export type DesktopTaskPrincipal =
  | {
      kind: "human";
      userId: string;
    }
  | {
      kind: "agent";
      agentId: string;
    };

export interface DesktopTaskSummary {
  threadId: string;
  taskId: string;
  number: number;
  title: string;
  status: DesktopTaskStatus;
  creator: DesktopTaskPrincipal;
  assignee?: DesktopTaskPrincipal | null;
  source?: DesktopTaskSource | null;
  executor?: DesktopTaskExecutor | null;
  updatedAt: string;
  updatedBy: DesktopTaskPrincipal;
  runtimeAgentId: string;
  replyCount: number;
}

export interface DesktopTaskForestTaskNode extends DesktopTaskSummary {
  kind: 'task';
  nodeId: string;
  parentNodeId?: string | null;
  parentTaskNumber?: number | null;
  parentThreadId?: string | null;
  activeRunId?: string | null;
  runState: string;
  lastActiveAt?: string | null;
  // Server DFS depth in anchored mode; null from console modes / old gateways.
  depth?: number | null;
}

export interface DesktopTaskForestThreadNode {
  kind: 'thread';
  nodeId: string;
  threadId: string;
  title: string;
  threadType: string;
  providerType?: string | null;
  agentId?: string | null;
  messageCount: number;
  lastMessagePreview: string;
  activeRunId?: string | null;
  runState: string;
  updatedAt?: string | null;
  lastActiveAt?: string | null;
  depth?: number | null;
}

export type DesktopTaskForestNode =
  | DesktopTaskForestTaskNode
  | DesktopTaskForestThreadNode;

export type DesktopTaskExecutor =
  | { type: 'agent'; agentId: string }
  | { type: 'workflow'; workflowId: string; workflowVersion?: number | null };

export interface DesktopTaskSource {
  threadId?: string | null;
  taskId?: string | null;
  taskThreadId?: string | null;
  botId?: string | null;
  channel?: string | null;
  accountId?: string | null;
}

export interface DesktopTasksPage {
  tasks: DesktopTaskSummary[];
  total: number;
  hasMore: boolean;
}

export interface DesktopTaskForestPage {
  tasks: DesktopTaskForestNode[];
  total: number;
  projectionCurrent: boolean;
  rootThreadIds: string[];
  skippedPinnedThreadIds: string[];
  // Server-computed active badge count in anchored mode; null elsewhere.
  activeCount?: number | null;
}

export type DesktopTaskForestScope = 'pinned' | 'all';

export interface ListTasksInput {
  status?: DesktopTaskStatus | null;
  assignee?: string | null;
  sourceThread?: string | null;
  sourceBot?: string | null;
  includeDone?: boolean;
  limit?: number;
  offset?: number;
}

export interface ListTaskForestInput extends ListTasksInput {
  scope?: DesktopTaskForestScope;
  /** When set, return the stable task tree containing this task thread. */
  anchorThreadId?: string | null;
}

export interface GetTaskInput {
  taskId: string;
}

export type DesktopTaskNotificationTarget =
  | { kind: "none" }
  | { kind: "bot"; channel: string; accountId: string };

export interface CreateTaskInput {
  title?: string | null;
  body?: string | null;
  source?: DesktopTaskSource | null;
  executor?: CreateTaskExecutorInput | null;
  /** @deprecated New task creation should use `executor`. */
  assignee?: string | null;
  start?: boolean;
  workspaceDir?: string | null;
  workspaceMode?: DesktopWorkspaceMode;
  notificationTarget: DesktopTaskNotificationTarget;
}

export type CreateTaskExecutorInput =
  | { type: 'agent'; agentId: string }
  | { type: 'workflow'; workflowId: string; input?: unknown };

export interface UpdateTaskStatusInput {
  taskId: string;
  status: DesktopTaskStatus;
  note?: string | null;
  force?: boolean;
}

export interface AssignTaskInput {
  taskId: string;
  principal: string;
}

export interface UnassignTaskInput {
  taskId: string;
}

export interface StopTaskInput {
  taskId: string;
}

export interface DeleteTaskInput {
  taskId: string;
}

export interface UpdateTaskTitleInput {
  taskId: string;
  title: string;
}
