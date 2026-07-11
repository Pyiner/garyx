import type {
  AssignTaskInput,
  CreateTaskInput,
  DeleteTaskInput,
  DesktopSettings,
  DesktopTaskForestNode,
  DesktopTaskForestPage,
  DesktopTaskPrincipal,
  DesktopTaskSource,
  DesktopTaskStatus,
  DesktopTaskSummary,
  DesktopTasksPage,
  GetTaskInput,
  ListTaskForestInput,
  ListTasksInput,
  StopTaskInput,
  UnassignTaskInput,
  UpdateTaskStatusInput,
  UpdateTaskTitleInput,
} from "@shared/contracts";
import { asFiniteNumber, asString, asStringList, parseRecord, requestJson } from "./http.ts";

interface TaskPrincipalPayload {
  kind?: string;
  user_id?: string;
  userId?: string;
  agent_id?: string;
  agentId?: string;
}

interface TaskSummaryPayload {
  thread_id?: string;
  threadId?: string;
  task_id?: string;
  taskId?: string;
  number?: number;
  title?: string | null;
  status?: string | null;
  creator?: TaskPrincipalPayload | null;
  assignee?: TaskPrincipalPayload | null;
  source?: TaskSourcePayload | null;
  updated_at?: string | null;
  updatedAt?: string | null;
  updated_by?: TaskPrincipalPayload | null;
  updatedBy?: TaskPrincipalPayload | null;
  runtime_agent_id?: string | null;
  runtimeAgentId?: string | null;
  reply_count?: number;
  replyCount?: number;
  executor?: TaskExecutorPayload | null;
  task?: TaskSummaryPayload | null;
}

interface TaskExecutorPayload {
  type?: string | null;
  agent_id?: string | null;
  agentId?: string | null;
}

interface TaskSourcePayload {
  thread_id?: string | null;
  threadId?: string | null;
  task_id?: string | null;
  taskId?: string | null;
  task_thread_id?: string | null;
  taskThreadId?: string | null;
  bot_id?: string | null;
  botId?: string | null;
  channel?: string | null;
  account_id?: string | null;
  accountId?: string | null;
}

interface TasksPayload {
  tasks?: TaskSummaryPayload[];
  total?: number;
  has_more?: boolean;
  hasMore?: boolean;
}

interface TaskForestNodePayload extends TaskSummaryPayload {
  kind?: string | null;
  node_id?: string | null;
  nodeId?: string | null;
  parent_node_id?: string | null;
  parentNodeId?: string | null;
  parent_task_number?: number | null;
  parentTaskNumber?: number | null;
  parent_thread_id?: string | null;
  parentThreadId?: string | null;
  active_run_id?: string | null;
  activeRunId?: string | null;
  run_state?: string | null;
  runState?: string | null;
  last_active_at?: string | null;
  lastActiveAt?: string | null;
  thread_type?: string | null;
  threadType?: string | null;
  provider_type?: string | null;
  providerType?: string | null;
  agent_id?: string | null;
  agentId?: string | null;
  message_count?: number | null;
  messageCount?: number | null;
  last_message_preview?: string | null;
  lastMessagePreview?: string | null;
  depth?: number | null;
}

interface TaskForestPayload {
  tasks?: TaskForestNodePayload[];
  total?: number;
  active_count?: number | null;
  activeCount?: number | null;
  root_thread_ids?: unknown[];
  rootThreadIds?: unknown[];
  skipped_pinned_thread_ids?: unknown[];
  skippedPinnedThreadIds?: unknown[];
}

function normalizeTaskStatus(value: unknown): DesktopTaskStatus {
  switch (value) {
    case "in_progress":
    case "in_review":
    case "done":
      return value;
    default:
      return "todo";
  }
}

function mapTaskPrincipal(value: unknown): DesktopTaskPrincipal {
  const record = parseRecord(value);
  if (record.kind === "human") {
    return {
      kind: "human",
      userId: asString(record.user_id) || asString(record.userId) || "owner",
    };
  }
  return {
    kind: "agent",
    agentId: asString(record.agent_id) || asString(record.agentId) || "claude",
  };
}

function mapTaskSource(value: unknown): DesktopTaskSource | null {
  if (!value || typeof value !== "object") {
    return null;
  }
  const record = parseRecord(value);
  const source: DesktopTaskSource = {
    threadId: asString(record.thread_id) || asString(record.threadId) || null,
    taskId: asString(record.task_id) || asString(record.taskId) || null,
    taskThreadId:
      asString(record.task_thread_id) || asString(record.taskThreadId) || null,
    botId: asString(record.bot_id) || asString(record.botId) || null,
    channel: asString(record.channel) || null,
    accountId: asString(record.account_id) || asString(record.accountId) || null,
  };
  return source.threadId ||
    source.taskId ||
    source.taskThreadId ||
    source.botId ||
    source.channel ||
    source.accountId
    ? source
    : null;
}

function mapTaskExecutor(value: unknown): DesktopTaskSummary["executor"] {
  if (!value || typeof value !== "object") {
    return null;
  }
  const record = parseRecord(value);
  const type = asString(record.type);
  if (type === "agent") {
    const agentId = asString(record.agent_id) || asString(record.agentId);
    return agentId ? { type: "agent", agentId } : null;
  }
  return null;
}

function mapTaskSummary(value: TaskSummaryPayload): DesktopTaskSummary {
  const task: TaskSummaryPayload =
    value.task && typeof value.task === "object" ? value.task : {};
  const number = asFiniteNumber(value.number) ?? asFiniteNumber(task.number) ?? 0;
  const title =
    asString(value.title) ||
    asString(task.title) ||
    (number > 0 ? `#TASK-${number}` : "") ||
    "Untitled task";
  return {
    threadId:
      asString(value.thread_id) ||
      asString(value.threadId) ||
      asString(task.thread_id) ||
      asString(task.threadId) ||
      "",
    taskId:
      asString(value.task_id) ||
      asString(value.taskId) ||
      asString(task.task_id) ||
      asString(task.taskId) ||
      (number > 0 ? `#TASK-${number}` : ""),
    number,
    title,
    status: normalizeTaskStatus(value.status ?? task.status),
    creator: mapTaskPrincipal(value.creator ?? task.creator),
    assignee:
      value.assignee || task.assignee
        ? mapTaskPrincipal(value.assignee ?? task.assignee)
        : null,
    source: mapTaskSource(value.source ?? task.source),
    executor: mapTaskExecutor(value.executor ?? task.executor),
    updatedAt:
      asString(value.updated_at) ||
      asString(value.updatedAt) ||
      asString(task.updated_at) ||
      asString(task.updatedAt) ||
      new Date(0).toISOString(),
    updatedBy: mapTaskPrincipal(
      value.updated_by ?? value.updatedBy ?? task.updated_by ?? task.updatedBy,
    ),
    runtimeAgentId:
      asString(value.runtime_agent_id) ||
      asString(value.runtimeAgentId) ||
      asString(task.runtime_agent_id) ||
      asString(task.runtimeAgentId) ||
      "",
    replyCount:
      asFiniteNumber(value.reply_count) ??
      asFiniteNumber(value.replyCount) ??
      asFiniteNumber(task.reply_count) ??
      asFiniteNumber(task.replyCount) ??
      0,
  };
}

function mapTaskForestNode(value: TaskForestNodePayload): DesktopTaskForestNode {
  const kind = (asString(value.kind) || "").trim().toLowerCase();
  if (kind === "thread") {
    const threadId = asString(value.thread_id) || asString(value.threadId) || "";
    return {
      kind: "thread",
      nodeId:
        asString(value.node_id) ||
        asString(value.nodeId) ||
        `thread-root:${threadId}`,
      threadId,
      title: asString(value.title) || threadId || "Pinned thread",
      threadType:
        asString(value.thread_type) || asString(value.threadType) || "chat",
      providerType:
        asString(value.provider_type) || asString(value.providerType) || null,
      agentId: asString(value.agent_id) || asString(value.agentId) || null,
      messageCount:
        asFiniteNumber(value.message_count) ??
        asFiniteNumber(value.messageCount) ??
        0,
      lastMessagePreview:
        asString(value.last_message_preview) ||
        asString(value.lastMessagePreview) ||
        "",
      activeRunId:
        asString(value.active_run_id) || asString(value.activeRunId) || null,
      runState: asString(value.run_state) || asString(value.runState) || "idle",
      updatedAt:
        asString(value.updated_at) || asString(value.updatedAt) || null,
      lastActiveAt:
        asString(value.last_active_at) || asString(value.lastActiveAt) || null,
      depth: asFiniteNumber(value.depth) ?? null,
    };
  }
  const task = mapTaskSummary(value);
  return {
    ...task,
    kind: "task",
    nodeId:
      asString(value.node_id) ||
      asString(value.nodeId) ||
      `task:${task.threadId}`,
    parentNodeId:
      asString(value.parent_node_id) || asString(value.parentNodeId) || null,
    parentTaskNumber:
      asFiniteNumber(value.parent_task_number) ??
      asFiniteNumber(value.parentTaskNumber) ??
      null,
    parentThreadId:
      asString(value.parent_thread_id) || asString(value.parentThreadId) || null,
    activeRunId:
      asString(value.active_run_id) || asString(value.activeRunId) || null,
    runState: asString(value.run_state) || asString(value.runState) || "idle",
    lastActiveAt:
      asString(value.last_active_at) || asString(value.lastActiveAt) || null,
    depth: asFiniteNumber(value.depth) ?? null,
  };
}

function principalPayload(principal: string): TaskPrincipalPayload {
  const trimmed = principal.trim();
  if (!trimmed) {
    throw new Error("principal cannot be empty");
  }
  const human = trimmed.match(/^human:(.+)$/);
  if (human) {
    const userId = human[1].trim();
    if (!userId) {
      throw new Error("human principal cannot be empty");
    }
    return { kind: "human", user_id: userId };
  }
  const agent = trimmed.match(/^agent:(.+)$/);
  if (agent) {
    const agentId = agent[1].trim();
    if (!agentId) {
      throw new Error("agent principal cannot be empty");
    }
    return { kind: "agent", agent_id: agentId };
  }
  return { kind: "agent", agent_id: trimmed };
}

export async function listTasks(
  settings: DesktopSettings,
  input: ListTasksInput = {},
): Promise<DesktopTasksPage> {
  const query = new URLSearchParams();
  if (input.status) {
    query.set("status", input.status);
  }
  const assignee = input.assignee?.trim() || "";
  if (assignee) {
    query.set("assignee", assignee);
  }
  const sourceThread = input.sourceThread?.trim() || "";
  if (sourceThread) {
    query.set("source_thread_id", sourceThread);
  }
  const sourceBot = input.sourceBot?.trim() || "";
  if (sourceBot) {
    query.set("source_bot_id", sourceBot);
  }
  if (input.includeDone) {
    query.set("include_done", "true");
  }
  query.set("limit", String(Math.max(1, Math.min(200, input.limit || 100))));
  query.set("offset", String(Math.max(0, input.offset || 0)));

  const payload = await requestJson<TasksPayload>(
    settings,
    `/api/tasks?${query.toString()}`,
    {
      signal: AbortSignal.timeout(8000),
    },
  );

  const tasks = Array.isArray(payload.tasks)
    ? payload.tasks.map(mapTaskSummary)
    : [];
  return {
    tasks,
    total: asFiniteNumber(payload.total) ?? tasks.length,
    hasMore: payload.has_more ?? payload.hasMore ?? false,
  };
}

export async function listTaskForest(
  settings: DesktopSettings,
  input: ListTaskForestInput = {},
): Promise<DesktopTaskForestPage> {
  const query = new URLSearchParams();
  if (input.status) {
    query.set("status", input.status);
  }
  const sourceBot = input.sourceBot?.trim() || "";
  if (sourceBot) {
    query.set("source_bot_id", sourceBot);
  }
  if (input.includeDone !== false) {
    query.set("include_done", "true");
  }
  if (input.scope) {
    query.set("scope", input.scope);
  }
  const anchorThreadId = input.anchorThreadId?.trim() || "";
  if (anchorThreadId) {
    query.set("anchor_thread_id", anchorThreadId);
  }

  const suffix = query.toString();
  const payload = await requestJson<TaskForestPayload>(
    settings,
    `/api/tasks/forest${suffix ? `?${suffix}` : ""}`,
    {
      signal: AbortSignal.timeout(8000),
    },
  );

  const tasks = Array.isArray(payload.tasks)
    ? payload.tasks.map(mapTaskForestNode)
    : [];
  return {
    tasks,
    total: asFiniteNumber(payload.total) ?? tasks.length,
    activeCount:
      asFiniteNumber(payload.active_count) ??
      asFiniteNumber(payload.activeCount) ??
      null,
    rootThreadIds: asStringList(payload.root_thread_ids ?? payload.rootThreadIds),
    skippedPinnedThreadIds: asStringList(
      payload.skipped_pinned_thread_ids ?? payload.skippedPinnedThreadIds,
    ),
  };
}

export async function getTask(
  settings: DesktopSettings,
  input: GetTaskInput,
): Promise<DesktopTaskSummary> {
  const taskId = input.taskId?.trim() || "";
  if (!taskId) {
    throw new Error("taskId is required");
  }
  const payload = await requestJson<TaskSummaryPayload>(
    settings,
    `/api/tasks/${encodeURIComponent(taskId)}`,
    {
      signal: AbortSignal.timeout(8000),
    },
  );
  return mapTaskSummary(payload);
}

export async function createTask(
  settings: DesktopSettings,
  input: CreateTaskInput,
): Promise<DesktopTaskSummary> {
  const executorPayload =
    input.executor?.type === "agent" && input.executor.agentId.trim()
      ? { type: "agent", agent_id: input.executor.agentId.trim() }
      : null;
  const assignee = input.assignee?.trim()
    ? principalPayload(input.assignee)
    : null;
  const runtimeAgentId =
    executorPayload?.type === "agent"
      ? executorPayload.agent_id
      : assignee?.kind === "agent"
        ? assignee.agent_id
        : "";
  const workspaceDir = input.workspaceDir?.trim() || "";
  const source = taskSourcePayload(input.source);
  const payload = await requestJson<TaskSummaryPayload>(settings, "/api/tasks", {
    method: "POST",
    signal: AbortSignal.timeout(8000),
    body: JSON.stringify({
      title: input.title?.trim() || null,
      body: input.body?.trim() || null,
      ...(source ? { source } : {}),
      executor: executorPayload,
      assignee: executorPayload ? null : assignee,
      start: input.start === true || executorPayload !== null || assignee !== null,
      workspace_dir: null,
      runtime: {
        agent_id: runtimeAgentId || null,
        workspace_dir: workspaceDir || null,
        workspace_mode: input.workspaceMode || "local",
      },
      notification_target: taskNotificationTargetPayload(input.notificationTarget),
    }),
  });
  return mapTaskSummary(payload);
}

function taskSourcePayload(
  source: DesktopTaskSource | null | undefined,
): Record<string, string> | null {
  if (!source) {
    return null;
  }
  const payload: Record<string, string> = {};
  const threadId = source.threadId?.trim();
  const taskId = source.taskId?.trim();
  const taskThreadId = source.taskThreadId?.trim();
  const botId = source.botId?.trim();
  const channel = source.channel?.trim();
  const accountId = source.accountId?.trim();
  if (threadId) {
    payload.thread_id = threadId;
  }
  if (taskId) {
    payload.task_id = taskId;
  }
  if (taskThreadId) {
    payload.task_thread_id = taskThreadId;
  }
  if (botId) {
    payload.bot_id = botId;
  }
  if (channel) {
    payload.channel = channel;
  }
  if (accountId) {
    payload.account_id = accountId;
  }
  return Object.keys(payload).length ? payload : null;
}

function taskNotificationTargetPayload(
  target: CreateTaskInput["notificationTarget"],
): Record<string, string> {
  if (target.kind === "none") {
    return { kind: "none" };
  }
  return {
    kind: "bot",
    channel: target.channel,
    account_id: target.accountId,
  };
}

export async function updateTaskStatus(
  settings: DesktopSettings,
  input: UpdateTaskStatusInput,
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/tasks/${encodeURIComponent(input.taskId)}/status`,
    {
      method: "PATCH",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        to: input.status,
        note: input.note?.trim() || null,
        force: input.force === true,
      }),
    },
  );
}

export async function assignTask(
  settings: DesktopSettings,
  input: AssignTaskInput,
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/tasks/${encodeURIComponent(input.taskId)}/assign`,
    {
      method: "PATCH",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        to: principalPayload(input.principal),
      }),
    },
  );
}

export async function unassignTask(
  settings: DesktopSettings,
  input: UnassignTaskInput,
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/tasks/${encodeURIComponent(input.taskId)}/assign`,
    {
      method: "DELETE",
      signal: AbortSignal.timeout(8000),
    },
  );
}

export async function stopTask(
  settings: DesktopSettings,
  input: StopTaskInput,
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/tasks/${encodeURIComponent(input.taskId)}/stop`,
    {
      method: "POST",
      signal: AbortSignal.timeout(8000),
    },
  );
}

export async function deleteTask(
  settings: DesktopSettings,
  input: DeleteTaskInput,
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/tasks/${encodeURIComponent(input.taskId)}`,
    {
      method: "DELETE",
      signal: AbortSignal.timeout(8000),
    },
  );
}

export async function updateTaskTitle(
  settings: DesktopSettings,
  input: UpdateTaskTitleInput,
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/tasks/${encodeURIComponent(input.taskId)}/title`,
    {
      method: "PATCH",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        title: input.title.trim(),
      }),
    },
  );
}
