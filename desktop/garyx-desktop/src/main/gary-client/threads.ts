import type {
  DesktopSettings,
  DesktopThreadProviderType,
  DesktopThreadSummary,
  GetThreadHistoryInput,
  PendingThreadInput,
  ThreadActiveRunInfo,
  ThreadChannelBindingInfo,
  ThreadLogChunk,
  ThreadRuntimeInfo,
  ThreadTeamBlock,
  ThreadTranscript,
  TranscriptMessage,
} from "@shared/contracts";
import { REMOTE_STATE_FETCH_TIMEOUT_MS, asBoolean, asFiniteNumber, asString, parseRecord, requestJson } from "./http.ts";

const DEFAULT_THREAD_HISTORY_PAGE_SIZE = 100;

const DEFAULT_THREAD_HISTORY_USER_QUERY_LIMIT = 10;

const MAX_THREAD_HISTORY_PAGE_SIZE = 500;

const MAX_THREAD_HISTORY_USER_QUERY_LIMIT = 50;

interface HistoryPayload {
  ok?: boolean;
  messages?: Array<{
    index?: number;
    role?: string;
    kind?: string;
    timestamp?: string | null;
    text?: string | null;
    content?: unknown;
    input?: unknown;
    result?: unknown;
    message?: Record<string, unknown> | null;
  }>;
  pending_user_inputs?: Array<{
    id?: string;
    run_id?: string | null;
    timestamp?: string | null;
    status?: string;
    active?: boolean;
    text?: string | null;
    content?: unknown;
  }>;
  message_stats?: {
    total_messages_in_thread?: number;
    total_messages_in_session?: number;
    committed_message_count?: number;
    returned_messages?: number;
    returned_user_queries?: number;
    returned_start_index?: number;
    returned_end_index?: number;
    has_more_before?: boolean;
    next_before_index?: number | null;
    has_more_after?: boolean;
    next_after_index?: number | null;
    reset?: boolean;
    user_query_limit?: number | null;
  };
  team?: ThreadTeamBlockPayload | null;
  thread_runtime?: ThreadRuntimePayload | null;
}

interface ThreadMetadataPayload extends ThreadSummaryPayload {
  sdk_session_id?: string | null;
  provider_type?: string | null;
  active_run?: ThreadActiveRunPayload | null;
  thread_runtime?: ThreadRuntimePayload | null;
}

interface ThreadRuntimePayload {
  agent_id?: string | null;
  provider_type?: string | null;
  provider_label?: string | null;
  model?: string | null;
  model_reasoning_effort?: string | null;
  model_service_tier?: string | null;
  model_override?: string | null;
  model_reasoning_effort_override?: string | null;
  model_service_tier_override?: string | null;
  sdk_session_id?: string | null;
  active_run?: ThreadActiveRunPayload | null;
}

function mapThreadWorktreeInfo(value: unknown) {
  const record = parseRecord(value);
  if (!Object.keys(record).length) {
    return null;
  }
  return {
    mode: asString(record.mode) || null,
    enabled: asBoolean(record.enabled) ?? null,
    branch: asString(record.branch) || null,
    sourceBranch:
      asString(record.source_branch) || asString(record.sourceBranch) || null,
    path: asString(record.path) || null,
    worktreeDir: asString(record.worktree_dir) || asString(record.worktreeDir) || null,
    sourceWorkspaceDir:
      asString(record.source_workspace_dir) ||
      asString(record.sourceWorkspaceDir) ||
      null,
    sourceRepoRoot:
      asString(record.source_repo_root) || asString(record.sourceRepoRoot) || null,
  };
}

interface ThreadActiveRunPayload {
  run_id?: string | null;
  provider_type?: string | null;
  provider_label?: string | null;
  assistant_response?: string | null;
  updated_at?: string | null;
  pending_user_input_count?: number;
}

interface ThreadTeamBlockPayload {
  team_id?: string;
  display_name?: string;
  leader_agent_id?: string;
  member_agent_ids?: string[];
  child_thread_ids?: Record<string, string>;
}

interface ThreadLogPayload {
  threadId?: string;
  thread_id?: string;
  path?: string;
  text?: string;
  cursor?: number;
  reset?: boolean;
}

export interface ThreadSummaryPayload {
  thread_key?: string;
  session_key?: string;
  thread_id?: string;
  thread_type?: string | null;
  threadType?: string | null;
  session_type?: string | null;
  agent_id?: string | null;
  agentId?: string | null;
  label?: string | null;
  title?: string | null;
  workspace_dir?: string | null;
  channel_bindings?: Array<{
    channel?: string;
    account_id?: string;
    binding_key?: string;
    peer_id?: string;
    chat_id?: string;
    thread_scope?: string | null;
    delivery_target_type?: string;
    delivery_target_id?: string;
    display_label?: string;
    last_inbound_at?: string | null;
    last_delivery_at?: string | null;
  }>;
  updated_at?: string | null;
  created_at?: string | null;
  last_active_at?: string | null;
  recorded_at?: string | null;
  message_count?: number;
  last_user_message?: string | null;
  last_assistant_message?: string | null;
  last_message_preview?: string | null;
  team_id?: string | null;
  team_display_name?: string | null;
  teamDisplayName?: string | null;
  team?: ThreadTeamBlockPayload | null;
  recent_run_id?: string | null;
  recentRunId?: string | null;
  run_state?: string | null;
  runState?: string | null;
  sdk_session_id?: string | null;
  worktree?: unknown;
}

interface ThreadsPayload {
  threads?: ThreadSummaryPayload[];
  sessions?: ThreadSummaryPayload[];
}

interface ThreadPinsPayload {
  thread_ids?: string[];
  threadIds?: string[];
  pins?: Array<{
    thread_id?: string;
    threadId?: string;
  }>;
}

function parseThreadProviderType(
  value: unknown,
): DesktopThreadProviderType | null {
  if (
    value === "claude_code" ||
    value === "claude_tty" ||
    value === "codex_app_server" ||
    value === "antigravity" ||
    value === "traex" ||
    value === "gemini_cli" ||
    value === "gpt" ||
    value === "anthropic" ||
    value === "google" ||
    value === "claude_llm" ||
    value === "gemini_llm" ||
    value === "garyx_native" ||
    value === "agent_team"
  ) {
    if (value === "claude_tty") {
      return "claude_code";
    }
    if (value === "garyx_native") {
      return "gpt";
    }
    if (value === "claude_llm") {
      return "anthropic";
    }
    if (value === "gemini_llm") {
      return "google";
    }
    return value;
  }
  return null;
}

function providerLabelForThread(
  value: DesktopThreadProviderType | null | undefined,
): string | null {
  switch (value) {
    case "claude_code":
      return "Claude";
    case "codex_app_server":
      return "Codex";
    case "antigravity":
      return "Antigravity";
    case "traex":
      return "Traex";
    case "gemini_cli":
      return "Gemini";
    case "gpt":
      return "GPT";
    case "anthropic":
    case "claude_llm":
      return "Claude";
    case "google":
    case "gemini_llm":
      return "Gemini";
    case "agent_team":
      return "Team";
    default:
      return null;
  }
}

function mapHistoryMessage(
  sessionId: string,
  value: NonNullable<HistoryPayload["messages"]>[number],
) {
  const normalized = parseRecord(value.message);
  const isControlRecord =
    value.kind === "control" ||
    asString(normalized.kind) === "control" ||
    asString(normalized.internal_kind) === "control";
  const isLoopContinuation =
    Boolean((value as { internal?: boolean }).internal) &&
    (value as { internal_kind?: unknown }).internal_kind ===
      "loop_continuation";
  const sourceRole = asString(value.role) || asString(normalized.role);
  const role = isLoopContinuation
    ? "system"
    : sourceRole === "assistant"
      ? "assistant"
      : sourceRole === "user"
        ? "user"
        : sourceRole === "tool"
          ? "tool"
        : sourceRole === "tool_use"
          ? "tool_use"
        : sourceRole === "tool_result"
          ? "tool_result"
          : "system";
  const content = isControlRecord
    ? normalized
    : "content" in normalized
      ? normalized.content
      : value.content;
  const metadataValue = normalized.metadata;
  const input = Object.prototype.hasOwnProperty.call(normalized, "input")
    ? normalized.input
    : value.input;
  const result = Object.prototype.hasOwnProperty.call(normalized, "result")
    ? normalized.result
    : value.result;
  const metadataRecord =
    metadataValue && typeof metadataValue === "object"
      ? (metadataValue as Record<string, unknown>)
      : null;
  const contentRecord = parseRecord(content);
  const fallbackText =
    isLoopContinuation && value.role === "user"
      ? "System triggered an automatic continuation."
      : "";
  const text = isControlRecord
    ? ""
    : asString(normalized.text) ||
      (typeof value.text === "string" ? value.text.trim() : "") ||
      (typeof value.content === "string" ? value.content.trim() : "") ||
      fallbackText;
  const hasStructuredContent =
    isControlRecord ||
    content !== null && content !== undefined ||
    input !== null && input !== undefined ||
    result !== null && result !== undefined;

  if (!text && !hasStructuredContent) {
    return null;
  }

  const visibleKinds = new Set([
    "assistant_reply",
    "user_input",
    "tool_trace",
    "system",
    "internal",
  ]);
  if (
    !isControlRecord &&
    !visibleKinds.has(value.kind || "") &&
    role === "system"
  ) {
    return null;
  }

  const message: TranscriptMessage = {
    id: `${sessionId}:${value.index ?? Math.random().toString(16).slice(2)}`,
    // History exposes a 0-based global `index`; the raw record seq is index + 1
    // (seq is 1-based and gapless across all records, control included).
    seq: typeof value.index === "number" ? value.index + 1 : undefined,
    role,
    text,
    content,
    input,
    result,
    toolUseId:
      asString(normalized.tool_use_id) ||
      asString(normalized.toolUseId) ||
      null,
    toolName:
      asString(normalized.tool_name) ||
      asString(normalized.toolName) ||
      asString(metadataRecord?.item_type) ||
      asString(metadataRecord?.itemType) ||
      asString(contentRecord.type) ||
      null,
    isError: asBoolean(normalized.is_error) ?? asBoolean(normalized.isError),
    metadata: metadataRecord,
    timestamp: value.timestamp,
    kind: value.kind,
    internal: Boolean((value as { internal?: boolean }).internal),
    internalKind:
      typeof (value as { internal_kind?: unknown }).internal_kind === "string"
        ? ((value as { internal_kind?: string }).internal_kind ?? null)
        : null,
    loopOrigin:
      typeof (value as { loop_origin?: unknown }).loop_origin === "string"
        ? ((value as { loop_origin?: string }).loop_origin ?? null)
        : null,
  };
  return message;
}

function mapPendingUserInput(
  value: NonNullable<HistoryPayload["pending_user_inputs"]>[number],
): PendingThreadInput | null {
  const id = asString(value.id);
  const status = value.status === "orphaned" ? "orphaned" : "awaiting_ack";
  const content = value.content;
  const text =
    asString(value.text) || (typeof content === "string" ? content.trim() : "");

  if (!id || (!text && (content === null || content === undefined))) {
    return null;
  }

  return {
    id,
    runId: asString(value.run_id) || null,
    text,
    content,
    timestamp: asString(value.timestamp) || null,
    status,
    active: value.active !== false && status === "awaiting_ack",
  };
}

function mapThreadTeamBlock(
  value: ThreadTeamBlockPayload | null | undefined,
): ThreadTeamBlock | null {
  if (!value || typeof value !== "object") {
    return null;
  }
  const teamId = typeof value.team_id === "string" ? value.team_id : "";
  if (!teamId) {
    return null;
  }
  const memberIds = Array.isArray(value.member_agent_ids)
    ? value.member_agent_ids.filter(
        (entry): entry is string => typeof entry === "string",
      )
    : [];
  const childThreadIds: Record<string, string> = {};
  if (value.child_thread_ids && typeof value.child_thread_ids === "object") {
    for (const [agentId, threadId] of Object.entries(value.child_thread_ids)) {
      if (
        typeof agentId === "string" &&
        typeof threadId === "string" &&
        threadId
      ) {
        childThreadIds[agentId] = threadId;
      }
    }
  }
  return {
    team_id: teamId,
    display_name:
      typeof value.display_name === "string" ? value.display_name : "",
    leader_agent_id:
      typeof value.leader_agent_id === "string" ? value.leader_agent_id : "",
    member_agent_ids: memberIds,
    child_thread_ids: childThreadIds,
  };
}

function mapThreadChannelBinding(
  value:
    | NonNullable<ThreadSummaryPayload["channel_bindings"]>[number]
    | null
    | undefined,
): ThreadChannelBindingInfo | null {
  if (!value || typeof value !== "object") {
    return null;
  }
  const channel = typeof value.channel === "string" ? value.channel : "";
  const accountId =
    typeof value.account_id === "string" ? value.account_id : "";
  const bindingKey =
    typeof value.binding_key === "string"
      ? value.binding_key
      : typeof value.peer_id === "string"
        ? value.peer_id
        : typeof value.thread_scope === "string"
          ? value.thread_scope
          : "";
  const chatId = typeof value.chat_id === "string" ? value.chat_id : "";
  const deliveryTargetType =
    typeof value.delivery_target_type === "string"
      ? value.delivery_target_type
      : "chat_id";
  const deliveryTargetId =
    typeof value.delivery_target_id === "string"
      ? value.delivery_target_id
      : chatId;
  return {
    channel,
    accountId,
    bindingKey,
    chatId,
    deliveryTargetType,
    deliveryTargetId,
    displayLabel:
      typeof value.display_label === "string" ? value.display_label : "",
    lastInboundAt:
      typeof value.last_inbound_at === "string" ? value.last_inbound_at : null,
    lastDeliveryAt:
      typeof value.last_delivery_at === "string"
        ? value.last_delivery_at
        : null,
  };
}

function mapThreadRuntimeInfo(
  value: ThreadMetadataPayload | null | undefined,
): ThreadRuntimeInfo | null {
  if (!value || typeof value !== "object") {
    return null;
  }
  const runtime = parseRecord(value.thread_runtime);
  const providerType = parseThreadProviderType(
    value.provider_type || asString(runtime.provider_type),
  );
  const channelBindings = Array.isArray(value.channel_bindings)
    ? value.channel_bindings
        .map((entry) => mapThreadChannelBinding(entry))
        .filter((entry): entry is ThreadChannelBindingInfo => Boolean(entry))
    : [];
  const activeRun = mapThreadActiveRun(
    (value.active_run as ThreadActiveRunPayload | null | undefined) ||
      (runtime.active_run as ThreadActiveRunPayload | null | undefined),
  );
  return {
    agentId:
      typeof value.agent_id === "string"
        ? value.agent_id
        : asString(runtime.agent_id) || value.agentId || null,
    providerType,
    providerLabel: asString(runtime.provider_label) || providerLabelForThread(providerType),
    model: asString(runtime.model) || null,
    modelReasoningEffort: asString(runtime.model_reasoning_effort) || null,
    modelServiceTier: asString(runtime.model_service_tier) || null,
    modelOverride: asString(runtime.model_override) || null,
    modelReasoningEffortOverride:
      asString(runtime.model_reasoning_effort_override) || null,
    modelServiceTierOverride: asString(runtime.model_service_tier_override) || null,
    sdkSessionId:
      typeof value.sdk_session_id === "string"
        ? value.sdk_session_id
        : asString(runtime.sdk_session_id) || null,
    workspacePath:
      typeof value.workspace_dir === "string" ? value.workspace_dir : null,
    worktree: mapThreadWorktreeInfo(value.worktree),
    activeRun,
    channelBindings,
  };
}

function mapThreadActiveRun(
  value: ThreadActiveRunPayload | null | undefined,
): ThreadActiveRunInfo | null {
  if (!value || typeof value !== "object") {
    return null;
  }
  const runId = asString(value.run_id);
  if (!runId) {
    return null;
  }
  const providerType = parseThreadProviderType(value.provider_type);
  return {
    runId,
    providerType,
    providerLabel: asString(value.provider_label) || providerLabelForThread(providerType),
    assistantResponse: asString(value.assistant_response) || null,
    updatedAt: asString(value.updated_at) || null,
    pendingUserInputCount:
      typeof value.pending_user_input_count === "number" &&
      Number.isFinite(value.pending_user_input_count)
        ? value.pending_user_input_count
        : undefined,
  };
}

export function mapThreadSummary(value: ThreadSummaryPayload): DesktopThreadSummary {
  const id = value.thread_id || value.thread_key || value.session_key || "";
  const team = mapThreadTeamBlock(value.team);
  const teamDisplayName =
    (team && team.display_name.trim()) ||
    (typeof value.team_display_name === "string"
      ? value.team_display_name.trim()
      : "") ||
    (typeof value.teamDisplayName === "string"
      ? value.teamDisplayName.trim()
      : "");
  const labelTrimmed =
    typeof value.label === "string" && value.label.trim()
      ? value.label.trim()
      : "";
  const titleTrimmed =
    typeof value.title === "string" && value.title.trim()
      ? value.title.trim()
      : "";
  // Title fallback chain: explicit label wins; otherwise a team thread prefers
  // the team's display_name so the thread list/header renders the team name.
  // ThreadsListPage + ThreadPage both consume `DesktopThreadSummary.title`
  // directly, so this fallback is the single source of truth for that branding.
  const title = labelTrimmed || titleTrimmed || teamDisplayName || id;
  const lastMessagePreview =
    (typeof value.last_message_preview === "string" &&
      value.last_message_preview.trim()) ||
    (typeof value.last_assistant_message === "string" &&
      value.last_assistant_message.trim()) ||
    (typeof value.last_user_message === "string" &&
      value.last_user_message.trim()) ||
    "";
  const createdAt =
    value.created_at ||
    value.recorded_at ||
    value.last_active_at ||
    new Date(0).toISOString();
  const updatedAt =
    value.updated_at ||
    value.last_active_at ||
    value.recorded_at ||
    value.created_at ||
    new Date(0).toISOString();
  return {
    id,
    title,
    threadType:
      asString(value.thread_type) ||
      asString(value.threadType) ||
      asString(value.session_type) ||
      null,
    createdAt,
    updatedAt,
    lastMessagePreview,
    workspacePath: value.workspace_dir ?? null,
    messageCount:
      typeof value.message_count === "number" &&
      Number.isFinite(value.message_count)
        ? value.message_count
        : undefined,
    agentId:
      typeof (value as { agent_id?: unknown }).agent_id === "string"
        ? ((value as { agent_id?: string }).agent_id ?? null)
        : null,
    teamId:
      typeof (value as { team_id?: unknown }).team_id === "string"
        ? ((value as { team_id?: string }).team_id ?? null)
        : null,
    teamName:
      (team && team.display_name) ||
      (typeof value.team_display_name === "string"
        ? value.team_display_name
        : typeof value.teamDisplayName === "string"
          ? value.teamDisplayName
          : null),
    team,
    recentRunId:
      asString(value.recent_run_id) || asString(value.recentRunId) || null,
    runState: asString(value.run_state) || asString(value.runState) || null,
    worktree: mapThreadWorktreeInfo(value.worktree),
  };
}

function normalizeThreadHistoryInput(
  input: string | GetThreadHistoryInput,
): {
  threadId: string;
  beforeIndex?: number;
  afterIndex?: number;
  limit: number;
  userQueryLimit: number;
} {
  const raw: {
    threadId: string;
    beforeIndex?: number | null;
    afterIndex?: number | null;
    limit?: number | null;
    userQueryLimit?: number | null;
  } =
    typeof input === "string"
      ? { threadId: input }
      : {
          threadId: input.threadId,
          beforeIndex: input.beforeIndex,
          afterIndex: input.afterIndex,
          limit: input.limit,
          userQueryLimit: input.userQueryLimit,
        };
  const limit =
    typeof raw.limit === "number" && Number.isFinite(raw.limit)
      ? Math.max(
          1,
          Math.min(MAX_THREAD_HISTORY_PAGE_SIZE, Math.floor(raw.limit)),
        )
      : DEFAULT_THREAD_HISTORY_PAGE_SIZE;
  const beforeIndex =
    typeof raw.beforeIndex === "number" &&
    Number.isFinite(raw.beforeIndex) &&
    raw.beforeIndex >= 0
      ? Math.floor(raw.beforeIndex)
      : undefined;
  const afterIndex =
    typeof raw.afterIndex === "number" &&
    Number.isFinite(raw.afterIndex) &&
    raw.afterIndex >= 0
      ? Math.floor(raw.afterIndex)
      : undefined;
  const userQueryLimit =
    typeof raw.userQueryLimit === "number" && Number.isFinite(raw.userQueryLimit)
      ? Math.max(
          1,
          Math.min(
            MAX_THREAD_HISTORY_USER_QUERY_LIMIT,
            Math.floor(raw.userQueryLimit),
          ),
        )
      : DEFAULT_THREAD_HISTORY_USER_QUERY_LIMIT;
  return {
    threadId: raw.threadId,
    beforeIndex,
    afterIndex,
    limit,
    userQueryLimit,
  };
}

function mapThreadTranscriptPageInfo(
  payload: HistoryPayload,
  limit: number,
): ThreadTranscript["pageInfo"] {
  const stats = payload.message_stats;
  if (!stats) {
    return null;
  }
  const totalMessages =
    asFiniteNumber(stats.total_messages_in_thread) ??
    asFiniteNumber(stats.total_messages_in_session) ??
    0;
  const committedMessages = asFiniteNumber(stats.committed_message_count);
  const returnedMessages = asFiniteNumber(stats.returned_messages) ?? 0;
  const returnedUserQueries = asFiniteNumber(stats.returned_user_queries);
  const startIndex = asFiniteNumber(stats.returned_start_index) ?? 0;
  const endIndex = asFiniteNumber(stats.returned_end_index) ?? startIndex;
  const nextBeforeIndex = asFiniteNumber(stats.next_before_index);
  const nextAfterIndex = asFiniteNumber(stats.next_after_index);
  const userQueryLimit = asFiniteNumber(stats.user_query_limit);
  return {
    totalMessages,
    committedMessages: committedMessages ?? null,
    returnedMessages,
    returnedUserQueries: returnedUserQueries ?? null,
    startIndex,
    endIndex,
    hasMoreBefore: Boolean(stats.has_more_before),
    nextBeforeIndex: nextBeforeIndex ?? null,
    hasMoreAfter: Boolean(stats.has_more_after),
    nextAfterIndex: nextAfterIndex ?? null,
    reset: Boolean(stats.reset),
    limit,
    userQueryLimit: userQueryLimit ?? null,
  };
}

export async function fetchThreadHistory(
  settings: DesktopSettings,
  input: string | GetThreadHistoryInput,
): Promise<ThreadTranscript> {
  const { threadId, beforeIndex, afterIndex, limit, userQueryLimit } =
    normalizeThreadHistoryInput(input);
  const query = new URLSearchParams({
    thread_id: threadId,
    limit: String(limit),
    user_query_limit: String(userQueryLimit),
    include_tool_messages: "true",
  });
  if (afterIndex !== undefined) {
    query.set("after_index", String(afterIndex));
  } else if (beforeIndex !== undefined) {
    query.set("before_index", String(beforeIndex));
  }
  const [payload, detail] = await Promise.all([
    requestJson<HistoryPayload>(
      settings,
      `/api/threads/history?${query.toString()}`,
      {
        signal: AbortSignal.timeout(8000),
      },
    ),
    beforeIndex === undefined && afterIndex === undefined
      ? requestJson<ThreadMetadataPayload>(
          settings,
          `/api/threads/${encodeURIComponent(threadId)}`,
          {
            signal: AbortSignal.timeout(8000),
          },
        ).catch(() => null)
      : Promise.resolve(null),
  ]);

  const messages =
    payload.messages
      ?.map((value) => mapHistoryMessage(threadId, value))
      .filter((value): value is TranscriptMessage => Boolean(value)) ?? [];
  const pendingInputs =
    payload.pending_user_inputs
      ?.map((value) => mapPendingUserInput(value))
      .filter((value): value is PendingThreadInput => Boolean(value)) ?? [];
  const threadInfoPayload =
    detail || payload.thread_runtime
      ? ({
          ...(detail ?? {}),
          thread_runtime: payload.thread_runtime ?? detail?.thread_runtime ?? null,
        } as ThreadMetadataPayload)
      : null;

  return {
    threadId,
    remoteFound: Boolean(payload.ok),
    messages,
    pendingInputs,
    thread: detail ? mapThreadSummary(detail) : null,
    threadInfo: mapThreadRuntimeInfo(threadInfoPayload),
    pageInfo: mapThreadTranscriptPageInfo(payload, limit),
    team: mapThreadTeamBlock(payload.team),
  };
}

export async function fetchThreadLogs(
  settings: DesktopSettings,
  threadId: string,
  cursor?: number,
): Promise<ThreadLogChunk> {
  const query = new URLSearchParams();
  if (typeof cursor === "number" && Number.isFinite(cursor) && cursor >= 0) {
    query.set("cursor", String(Math.floor(cursor)));
  }
  const suffix = query.size ? `?${query.toString()}` : "";
  const payload = await requestJson<ThreadLogPayload>(
    settings,
    `/api/threads/${encodeURIComponent(threadId)}/logs${suffix}`,
    {
      signal: AbortSignal.timeout(8000),
    },
  );

  return {
    threadId: payload.threadId || payload.thread_id || threadId,
    path: typeof payload.path === "string" ? payload.path : "",
    text: typeof payload.text === "string" ? payload.text : "",
    cursor:
      typeof payload.cursor === "number" &&
      Number.isFinite(payload.cursor) &&
      payload.cursor >= 0
        ? payload.cursor
        : 0,
    reset: payload.reset !== false,
  };
}

export async function fetchThreads(
  settings: DesktopSettings,
  options?: { limit?: number },
): Promise<DesktopThreadSummary[]> {
  const limit = options?.limit ?? 1000;
  const payload = await requestJson<ThreadsPayload>(
    settings,
    `/api/threads?limit=${limit}`,
    {
      signal: AbortSignal.timeout(REMOTE_STATE_FETCH_TIMEOUT_MS),
    },
  );

  const threads = Array.isArray(payload.threads)
    ? payload.threads
    : Array.isArray(payload.sessions)
      ? payload.sessions
      : [];
  return threads.map(mapThreadSummary);
}

/**
 * Single-thread summary fetch used to repair ids that must resolve in a
 * fast (truncated) state page, e.g. pinned threads older than the page.
 * Resolves null when the thread does not exist or the request fails.
 */
export async function fetchThreadSummary(
  settings: DesktopSettings,
  threadId: string,
): Promise<DesktopThreadSummary | null> {
  try {
    const payload = await requestJson<ThreadMetadataPayload>(
      settings,
      `/api/threads/${encodeURIComponent(threadId)}`,
      {
        signal: AbortSignal.timeout(8000),
      },
    );
    return mapThreadSummary(payload);
  } catch {
    return null;
  }
}

function mapThreadPinIds(payload: ThreadPinsPayload): string[] {
  const rawIds = Array.isArray(payload.thread_ids)
    ? payload.thread_ids
    : Array.isArray(payload.threadIds)
      ? payload.threadIds
      : Array.isArray(payload.pins)
        ? payload.pins.map((pin) => pin.thread_id || pin.threadId || "")
        : [];
  const seen = new Set<string>();
  const ids: string[] = [];
  for (const rawId of rawIds) {
    if (typeof rawId !== "string") {
      continue;
    }
    const id = rawId.trim();
    if (!id || seen.has(id)) {
      continue;
    }
    seen.add(id);
    ids.push(id);
  }
  return ids;
}

export async function fetchThreadPins(
  settings: DesktopSettings,
): Promise<string[]> {
  const payload = await requestJson<ThreadPinsPayload>(settings, "/api/thread-pins", {
    signal: AbortSignal.timeout(REMOTE_STATE_FETCH_TIMEOUT_MS),
  });
  return mapThreadPinIds(payload);
}

export async function setRemoteThreadPinned(
  settings: DesktopSettings,
  threadId: string,
  pinned: boolean,
): Promise<string[]> {
  const payload = await requestJson<ThreadPinsPayload>(
    settings,
    `/api/thread-pins/${encodeURIComponent(threadId)}`,
    {
      method: pinned ? "PUT" : "DELETE",
      signal: AbortSignal.timeout(8000),
    },
  );
  return mapThreadPinIds(payload);
}

export async function createRemoteThread(
  settings: DesktopSettings,
  input?: {
    title?: string;
    workspacePath?: string | null;
    workspaceMode?: "local" | "worktree";
    agentId?: string | null;
    model?: string | null;
    modelReasoningEffort?: string | null;
    modelServiceTier?: string | null;
    sdkSessionId?: string | null;
    sdkSessionProviderHint?: "claude" | "codex" | "gemini" | null;
    forkFromThreadId?: string | null;
    metadata?: Record<string, unknown> | null;
  },
): Promise<DesktopThreadSummary> {
  const payload = await requestJson<ThreadSummaryPayload>(
    settings,
    "/api/threads",
    {
      method: "POST",
      // Creating a thread can wait behind gateway thread-store write locks
      // (10s lock timeout); aborting earlier than that surfaces an opaque
      // TimeoutError instead of the gateway's own error.
      signal: AbortSignal.timeout(20000),
      body: JSON.stringify({
        label: input?.title || undefined,
        workspaceDir: input?.workspacePath || undefined,
        workspaceMode: input?.workspaceMode || undefined,
        agentId: input?.agentId || undefined,
        model: input?.model || undefined,
        modelReasoningEffort: input?.modelReasoningEffort || undefined,
        modelServiceTier: input?.modelServiceTier || undefined,
        sdkSessionId: input?.sdkSessionId || undefined,
        sdkSessionProviderHint: input?.sdkSessionProviderHint || undefined,
        forkFromThreadId: input?.forkFromThreadId || undefined,
        metadata: input?.metadata || undefined,
      }),
    },
  );
  return mapThreadSummary(payload);
}

export async function updateRemoteThread(
  settings: DesktopSettings,
  threadId: string,
  input: {
    title?: string;
    workspacePath?: string | null;
    model?: string | null;
    modelReasoningEffort?: string | null;
    modelServiceTier?: string | null;
  },
): Promise<DesktopThreadSummary> {
  const body: Record<string, unknown> = {
    label: input.title || undefined,
    workspaceDir: input.workspacePath || undefined,
  };
  if (Object.prototype.hasOwnProperty.call(input, "model")) {
    body.model = input.model || "";
  }
  if (Object.prototype.hasOwnProperty.call(input, "modelReasoningEffort")) {
    body.modelReasoningEffort = input.modelReasoningEffort || "";
  }
  if (Object.prototype.hasOwnProperty.call(input, "modelServiceTier")) {
    body.modelServiceTier = input.modelServiceTier || "";
  }
  const payload = await requestJson<ThreadSummaryPayload>(
    settings,
    `/api/threads/${encodeURIComponent(threadId)}`,
    {
      method: "PATCH",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify(body),
    },
  );
  return mapThreadSummary(payload);
}

export async function deleteRemoteThread(
  settings: DesktopSettings,
  threadId: string,
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/threads/${encodeURIComponent(threadId)}`,
    {
      method: "DELETE",
      signal: AbortSignal.timeout(8000),
    },
  );
}

export async function archiveRemoteThread(
  settings: DesktopSettings,
  threadId: string,
  endpointKeys: string[] = [],
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/threads/${encodeURIComponent(threadId)}/archive`,
    {
      method: "POST",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({ endpointKeys }),
    },
  );
}

export const fetchSessions = fetchThreads;

export const createRemoteSession = createRemoteThread;

export const updateRemoteSession = updateRemoteThread;

export const deleteRemoteSession = deleteRemoteThread;
