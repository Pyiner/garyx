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
  ListTaskForestInput,
  ListTasksInput,
  StopTaskInput,
  UpdateTaskStatusInput,
} from "@shared/contracts";
import {
  GatewayContractError,
  hasContractField,
  requestJson,
  requireContractArray,
  requireContractBoolean,
  requireContractField,
  requireContractNonEmptyString,
  requireContractNonNegativeInteger,
  requireContractRecord,
  requireContractString,
} from "./http.ts";

interface TaskPrincipalPayload {
  kind?: string;
  user_id?: string;
  agent_id?: string;
}

interface TaskSummaryPayload {
  thread_id?: string;
  task_id?: string;
  number?: number;
  title?: string | null;
  status?: string | null;
  creator?: TaskPrincipalPayload | null;
  assignee?: TaskPrincipalPayload | null;
  source?: TaskSourcePayload | null;
  updated_at?: string | null;
  updated_by?: TaskPrincipalPayload | null;
  runtime_agent_id?: string | null;
  reply_count?: number;
  executor?: TaskExecutorPayload | null;
  task?: TaskSummaryPayload | null;
}

interface TaskExecutorPayload {
  type?: string | null;
  agent_id?: string | null;
}

interface TaskSourcePayload {
  thread_id?: string | null;
  task_id?: string | null;
  task_thread_id?: string | null;
  bot_id?: string | null;
  channel?: string | null;
  account_id?: string | null;
}

interface TasksPayload {
  tasks?: TaskSummaryPayload[];
  total?: number;
  has_more?: boolean;
}

interface TaskForestNodePayload extends TaskSummaryPayload {
  kind?: string | null;
  node_id?: string | null;
  parent_node_id?: string | null;
  parent_task_number?: number | null;
  parent_thread_id?: string | null;
  active_run_id?: string | null;
  run_state?: string | null;
  last_active_at?: string | null;
  thread_type?: string | null;
  provider_type?: string | null;
  agent_id?: string | null;
  message_count?: number | null;
  last_message_preview?: string | null;
  depth?: number | null;
}

interface TaskForestPayload {
  tasks?: TaskForestNodePayload[];
  total?: number;
  active_count?: number | null;
  root_thread_ids?: unknown[];
  skipped_pinned_thread_ids?: unknown[];
}

function mapTaskStatus(value: unknown, path: string): DesktopTaskStatus {
  switch (value) {
    case "todo":
    case "in_progress":
    case "in_review":
    case "done":
      return value;
    default:
      throw new GatewayContractError(path, "must be a current task status");
  }
}

function mapTaskPrincipal(value: unknown, path: string): DesktopTaskPrincipal {
  const record = requireContractRecord(value, path);
  if (record.kind === "human") {
    return {
      kind: "human",
      userId: requireContractNonEmptyString(
        requireContractField(record, "user_id", path),
        `${path}.user_id`,
      ),
    };
  }
  if (record.kind === "agent") {
    return {
      kind: "agent",
      agentId: requireContractNonEmptyString(
        requireContractField(record, "agent_id", path),
        `${path}.agent_id`,
      ),
    };
  }
  throw new GatewayContractError(`${path}.kind`, "must be human or agent");
}

function mapTaskSource(value: unknown, path: string): DesktopTaskSource | null {
  if (value === undefined || value === null) {
    return null;
  }
  const record = requireContractRecord(value, path);
  const optionalString = (field: string): string | null => {
    if (!hasContractField(record, field)) {
      return null;
    }
    return requireContractString(record[field], `${path}.${field}`);
  };
  const source: DesktopTaskSource = {
    threadId: optionalString("thread_id"),
    taskId: optionalString("task_id"),
    taskThreadId: optionalString("task_thread_id"),
    botId: optionalString("bot_id"),
    channel: optionalString("channel"),
    accountId: optionalString("account_id"),
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

function mapTaskExecutor(
  value: unknown,
  path: string,
): DesktopTaskSummary["executor"] {
  if (value === undefined || value === null) {
    return null;
  }
  const record = requireContractRecord(value, path);
  if (record.type !== "agent") {
    throw new GatewayContractError(`${path}.type`, "must be agent");
  }
  return {
    type: "agent",
    agentId: requireContractNonEmptyString(
      requireContractField(record, "agent_id", path),
      `${path}.agent_id`,
    ),
  };
}

function optionalTaskPrincipal(
  record: Record<string, unknown>,
  field: string,
  path: string,
): DesktopTaskPrincipal | null {
  if (!hasContractField(record, field)) {
    return null;
  }
  return mapTaskPrincipal(record[field], `${path}.${field}`);
}

function mapTaskSummaryRecord(
  value: unknown,
  path: string,
): DesktopTaskSummary {
  const record = requireContractRecord(value, path);
  return {
    threadId: requireContractNonEmptyString(
      requireContractField(record, "thread_id", path),
      `${path}.thread_id`,
    ),
    taskId: requireContractNonEmptyString(
      requireContractField(record, "task_id", path),
      `${path}.task_id`,
    ),
    number: requireContractNonNegativeInteger(
      requireContractField(record, "number", path),
      `${path}.number`,
    ),
    title: requireContractString(
      requireContractField(record, "title", path),
      `${path}.title`,
    ),
    status: mapTaskStatus(
      requireContractField(record, "status", path),
      `${path}.status`,
    ),
    creator: mapTaskPrincipal(
      requireContractField(record, "creator", path),
      `${path}.creator`,
    ),
    assignee: optionalTaskPrincipal(record, "assignee", path),
    source: mapTaskSource(record.source, `${path}.source`),
    executor: mapTaskExecutor(record.executor, `${path}.executor`),
    updatedAt: requireContractNonEmptyString(
      requireContractField(record, "updated_at", path),
      `${path}.updated_at`,
    ),
    updatedBy: mapTaskPrincipal(
      requireContractField(record, "updated_by", path),
      `${path}.updated_by`,
    ),
    runtimeAgentId: requireContractString(
      requireContractField(record, "runtime_agent_id", path),
      `${path}.runtime_agent_id`,
    ),
    replyCount: requireContractNonNegativeInteger(
      requireContractField(record, "reply_count", path),
      `${path}.reply_count`,
    ),
  };
}

type TaskEnvelopeFields = Omit<
  DesktopTaskSummary,
  "threadId" | "taskId" | "runtimeAgentId" | "replyCount"
>;

function mapTaskEnvelopeFields(
  record: Record<string, unknown>,
  path: string,
): TaskEnvelopeFields {
  const task = requireContractRecord(
    requireContractField(record, "task", path),
    `${path}.task`,
  );
  const taskPath = `${path}.task`;
  return {
    number: requireContractNonNegativeInteger(
      requireContractField(task, "number", taskPath),
      `${taskPath}.number`,
    ),
    title: requireContractString(
      requireContractField(task, "title", taskPath),
      `${taskPath}.title`,
    ),
    status: mapTaskStatus(
      requireContractField(task, "status", taskPath),
      `${taskPath}.status`,
    ),
    creator: mapTaskPrincipal(
      requireContractField(task, "creator", taskPath),
      `${taskPath}.creator`,
    ),
    assignee: optionalTaskPrincipal(task, "assignee", taskPath),
    source: mapTaskSource(task.source, `${taskPath}.source`),
    executor: mapTaskExecutor(task.executor, `${taskPath}.executor`),
    updatedAt: requireContractNonEmptyString(
      requireContractField(task, "updated_at", taskPath),
      `${taskPath}.updated_at`,
    ),
    updatedBy: mapTaskPrincipal(
      requireContractField(task, "updated_by", taskPath),
      `${taskPath}.updated_by`,
    ),
  };
}

function mapTaskEnvelopeIdentity(
  record: Record<string, unknown>,
  path: string,
): Pick<DesktopTaskSummary, "threadId" | "taskId"> {
  return {
    threadId: requireContractNonEmptyString(
      requireContractField(record, "thread_id", path),
      `${path}.thread_id`,
    ),
    taskId: requireContractNonEmptyString(
      requireContractField(record, "task_id", path),
      `${path}.task_id`,
    ),
  };
}

function mapCreatedTaskEnvelope(value: unknown, path: string): DesktopTaskSummary {
  const record = requireContractRecord(value, path);
  const fields = mapTaskEnvelopeFields(record, path);
  const number = requireContractNonNegativeInteger(
    requireContractField(record, "number", path),
    `${path}.number`,
  );
  const status = mapTaskStatus(
    requireContractField(record, "status", path),
    `${path}.status`,
  );
  if (number !== fields.number || status !== fields.status) {
    throw new GatewayContractError(
      path,
      "must keep its task number and status projections consistent",
    );
  }
  return {
    ...mapTaskEnvelopeIdentity(record, path),
    ...fields,
    runtimeAgentId: requireContractString(
      requireContractField(record, "runtime_agent_id", path),
      `${path}.runtime_agent_id`,
    ),
    // A successful create returns a brand-new backing thread, so no replies can
    // predate this response; the create envelope intentionally has no counter.
    replyCount: 0,
  };
}

function requiredNullableString(
  record: Record<string, unknown>,
  field: string,
  path: string,
): string | null {
  const value = requireContractField(record, field, path);
  return value === null
    ? null
    : requireContractString(value, `${path}.${field}`);
}

function requiredNullableInteger(
  record: Record<string, unknown>,
  field: string,
  path: string,
): number | null {
  const value = requireContractField(record, field, path);
  return value === null
    ? null
    : requireContractNonNegativeInteger(value, `${path}.${field}`);
}

function mapTaskForestNode(value: unknown, index: number): DesktopTaskForestNode {
  const path = `task forest.tasks[${index}]`;
  const record = requireContractRecord(value, path);
  const kind = requireContractString(
    requireContractField(record, "kind", path),
    `${path}.kind`,
  );
  if (kind === "thread") {
    return {
      kind: "thread",
      nodeId: requireContractNonEmptyString(
        requireContractField(record, "node_id", path),
        `${path}.node_id`,
      ),
      threadId: requireContractNonEmptyString(
        requireContractField(record, "thread_id", path),
        `${path}.thread_id`,
      ),
      title: requireContractString(
        requireContractField(record, "title", path),
        `${path}.title`,
      ),
      threadType: requireContractNonEmptyString(
        requireContractField(record, "thread_type", path),
        `${path}.thread_type`,
      ),
      providerType: requiredNullableString(record, "provider_type", path),
      agentId: requiredNullableString(record, "agent_id", path),
      messageCount: requireContractNonNegativeInteger(
        requireContractField(record, "message_count", path),
        `${path}.message_count`,
      ),
      lastMessagePreview: requireContractString(
        requireContractField(record, "last_message_preview", path),
        `${path}.last_message_preview`,
      ),
      activeRunId: requiredNullableString(record, "active_run_id", path),
      runState: requireContractNonEmptyString(
        requireContractField(record, "run_state", path),
        `${path}.run_state`,
      ),
      updatedAt: requiredNullableString(record, "updated_at", path),
      lastActiveAt: requiredNullableString(record, "last_active_at", path),
      depth: hasContractField(record, "depth")
        ? requireContractNonNegativeInteger(record.depth, `${path}.depth`)
        : null,
    };
  }
  if (kind !== "task") {
    throw new GatewayContractError(`${path}.kind`, "must be thread or task");
  }
  const task = mapTaskSummaryRecord(record, path);
  return {
    ...task,
    kind: "task",
    nodeId: requireContractNonEmptyString(
      requireContractField(record, "node_id", path),
      `${path}.node_id`,
    ),
    parentNodeId: requiredNullableString(record, "parent_node_id", path),
    parentTaskNumber: requiredNullableInteger(
      record,
      "parent_task_number",
      path,
    ),
    parentThreadId: requiredNullableString(record, "parent_thread_id", path),
    activeRunId: requiredNullableString(record, "active_run_id", path),
    runState: requireContractNonEmptyString(
      requireContractField(record, "run_state", path),
      `${path}.run_state`,
    ),
    lastActiveAt: requiredNullableString(record, "last_active_at", path),
    depth: hasContractField(record, "depth")
      ? requireContractNonNegativeInteger(record.depth, `${path}.depth`)
      : null,
  };
}

function mapRequiredStringList(value: unknown, path: string): string[] {
  return requireContractArray(value, path).map((entry, index) =>
    requireContractNonEmptyString(entry, `${path}[${index}]`),
  );
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

  const record = requireContractRecord(payload, "task list");
  const tasks = requireContractArray(
    requireContractField(record, "tasks", "task list"),
    "task list.tasks",
  ).map((task, index) => mapTaskSummaryRecord(task, `task list.tasks[${index}]`));
  return {
    tasks,
    total: requireContractNonNegativeInteger(
      requireContractField(record, "total", "task list"),
      "task list.total",
    ),
    hasMore: requireContractBoolean(
      requireContractField(record, "has_more", "task list"),
      "task list.has_more",
    ),
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

  const record = requireContractRecord(payload, "task forest");
  const tasks = requireContractArray(
    requireContractField(record, "tasks", "task forest"),
    "task forest.tasks",
  ).map(mapTaskForestNode);
  return {
    tasks,
    total: requireContractNonNegativeInteger(
      requireContractField(record, "total", "task forest"),
      "task forest.total",
    ),
    activeCount: hasContractField(record, "active_count")
      ? requireContractNonNegativeInteger(
          record.active_count,
          "task forest.active_count",
        )
      : null,
    rootThreadIds: mapRequiredStringList(
      requireContractField(record, "root_thread_ids", "task forest"),
      "task forest.root_thread_ids",
    ),
    skippedPinnedThreadIds: mapRequiredStringList(
      requireContractField(
        record,
        "skipped_pinned_thread_ids",
        "task forest",
      ),
      "task forest.skipped_pinned_thread_ids",
    ),
  };
}

export async function createTask(
  settings: DesktopSettings,
  input: CreateTaskInput,
): Promise<DesktopTaskSummary> {
  const executorPayload =
    input.executor?.type === "agent" && input.executor.agentId.trim()
      ? { type: "agent", agent_id: input.executor.agentId.trim() }
      : null;
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
      assignee: null,
      start: input.start === true || executorPayload !== null,
      workspace_dir: null,
      runtime: {
        agent_id: executorPayload?.agent_id || null,
        workspace_dir: workspaceDir || null,
        workspace_mode: input.workspaceMode || "local",
      },
      notification_target: taskNotificationTargetPayload(input.notificationTarget),
    }),
  });
  return mapCreatedTaskEnvelope(payload, "create task response");
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
