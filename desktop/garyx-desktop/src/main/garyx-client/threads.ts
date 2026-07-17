import type {
  DesktopSettings,
  DesktopRecentThreadsPage,
  DesktopThreadProviderType,
  DesktopThreadFavoritesPage,
  DesktopThreadFavoritesSnapshot,
  DesktopThreadPinsPage,
  DesktopThreadSummary,
  GetThreadHistoryInput,
  ListRecentThreadsInput,
  PendingThreadInput,
  ThreadActiveRunInfo,
  ThreadChannelBindingInfo,
  ThreadLogChunk,
  ThreadRuntimeInfo,
  ThreadTranscript,
  TranscriptMessage,
} from "@shared/contracts";
import {
  GatewayRequestError,
  GatewayContractError,
  REMOTE_STATE_FETCH_TIMEOUT_MS,
  asBoolean,
  asString,
  hasContractField,
  normalizeGatewayUrl,
  parseRecord,
  requestJson,
  requestMutationJson,
  requireContractArray,
  requireContractBoolean,
  requireContractField,
  requireContractNonEmptyString,
  requireContractNonNegativeInteger,
  requireContractRecord,
  requireContractString,
  tryParseJson,
  type GatewayMutationResult,
} from "./http.ts";

const DEFAULT_THREAD_HISTORY_PAGE_SIZE = 100;

const DEFAULT_THREAD_HISTORY_USER_QUERY_LIMIT = 10;

const MAX_THREAD_HISTORY_PAGE_SIZE = 500;

const MAX_THREAD_HISTORY_USER_QUERY_LIMIT = 50;

const MAX_RECENT_THREAD_PAGE_SIZE = 200;

const EPOCH_ISO = new Date(0).toISOString();

function requireNullableStringField(
  record: Record<string, unknown>,
  field: string,
  context: string,
): string | null {
  const value = requireContractField(record, field, context);
  if (value === null) {
    return null;
  }
  return requireContractString(value, `${context}.${field}`);
}

function optionalNullableStringField(
  record: Record<string, unknown>,
  field: string,
  path: string,
): string | null {
  if (!hasContractField(record, field) || record[field] === null) {
    return null;
  }
  return requireContractString(record[field], path);
}

function mapThreadWorktreeInfo(value: unknown) {
  if (value === null) {
    return null;
  }
  const record = requireContractRecord(value, "thread worktree");
  const nullableString = (field: string): string | null => {
    const raw = requireContractField(record, field, "thread worktree");
    if (raw === null) {
      return null;
    }
    return requireContractString(raw, `thread worktree.${field}`);
  };
  return {
    mode: requireContractString(
      requireContractField(record, "mode", "thread worktree"),
      "thread worktree.mode",
    ),
    enabled: requireContractBoolean(
      requireContractField(record, "enabled", "thread worktree"),
      "thread worktree.enabled",
    ),
    branch: requireContractString(
      requireContractField(record, "branch", "thread worktree"),
      "thread worktree.branch",
    ),
    sourceBranch: nullableString("source_branch"),
    path: requireContractString(
      requireContractField(record, "path", "thread worktree"),
      "thread worktree.path",
    ),
    worktreeDir: requireContractString(
      requireContractField(record, "worktree_dir", "thread worktree"),
      "thread worktree.worktree_dir",
    ),
    sourceWorkspaceDir: requireContractString(
      requireContractField(record, "source_workspace_dir", "thread worktree"),
      "thread worktree.source_workspace_dir",
    ),
    sourceRepoRoot: requireContractString(
      requireContractField(record, "source_repo_root", "thread worktree"),
      "thread worktree.source_repo_root",
    ),
  };
}

function parseThreadProviderType(
  value: unknown,
  path: string,
): DesktopThreadProviderType | null {
  if (value === null) {
    return null;
  }
  if (
    value === "claude_code" ||
    value === "codex_app_server" ||
    value === "antigravity" ||
    value === "traex"
  ) {
    return value;
  }
  throw new GatewayContractError(
    path,
    "must be null or a current provider type",
  );
}

function mapHistoryMessage(
  sessionId: string,
  value: unknown,
  path: string,
) {
  const envelope = requireContractRecord(value, path);
  const index = requireContractNonNegativeInteger(
    requireContractField(envelope, "index", path),
    `${path}.index`,
  );
  const envelopeRole = requireContractString(
    requireContractField(envelope, "role", path),
    `${path}.role`,
  );
  const kind = requireContractString(
    requireContractField(envelope, "kind", path),
    `${path}.kind`,
  );
  const timestamp = requireNullableStringField(envelope, "timestamp", path);
  const envelopeText = requireContractString(
    requireContractField(envelope, "text", path),
    `${path}.text`,
  );
  const envelopeContent = requireContractField(envelope, "content", path);
  const internal = requireContractBoolean(
    requireContractField(envelope, "internal", path),
    `${path}.internal`,
  );
  const internalKind = requireNullableStringField(envelope, "internal_kind", path);
  const loopOrigin = requireNullableStringField(envelope, "loop_origin", path);
  const normalized = requireContractRecord(
    requireContractField(envelope, "message", path),
    `${path}.message`,
  );
  const isControlRecord =
    kind === "control" ||
    asString(normalized.kind) === "control" ||
    asString(normalized.internal_kind) === "control";
  const isLoopContinuation =
    internal && internalKind === "loop_continuation";
  const sourceRole = envelopeRole || asString(normalized.role);
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
      : envelopeContent;
  const metadataValue = normalized.metadata;
  const input = Object.prototype.hasOwnProperty.call(normalized, "input")
    ? normalized.input
    : undefined;
  const result = Object.prototype.hasOwnProperty.call(normalized, "result")
    ? normalized.result
    : undefined;
  const metadataRecord =
    metadataValue && typeof metadataValue === "object"
      ? (metadataValue as Record<string, unknown>)
      : null;
  const contentRecord = parseRecord(content);
  const fallbackText =
    isLoopContinuation && envelopeRole === "user"
      ? "System triggered an automatic continuation."
      : "";
  const text = isControlRecord
    ? ""
    : asString(normalized.text) ||
      envelopeText.trim() ||
      (typeof envelopeContent === "string" ? envelopeContent.trim() : "") ||
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
    !visibleKinds.has(kind) &&
    role === "system"
  ) {
    return null;
  }

  const message: TranscriptMessage = {
    id: `${sessionId}:${index}`,
    // History exposes a 0-based global `index`; the raw record seq is index + 1
    // (seq is 1-based and gapless across all records, control included).
    seq: index + 1,
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
    timestamp,
    kind,
    internal,
    internalKind,
    loopOrigin,
  };
  return message;
}

function mapPendingUserInput(
  value: unknown,
  path: string,
): PendingThreadInput {
  const record = requireContractRecord(value, path);
  const id = requireContractNonEmptyString(
    requireContractField(record, "id", path),
    `${path}.id`,
  );
  const statusValue = requireContractField(record, "status", path);
  if (statusValue !== "awaiting_ack") {
    throw new GatewayContractError(
      `${path}.status`,
      'must be "awaiting_ack"',
    );
  }
  const content = requireContractField(record, "content", path);
  const text = requireContractString(
    requireContractField(record, "text", path),
    `${path}.text`,
  );
  const active = requireContractBoolean(
    requireContractField(record, "active", path),
    `${path}.active`,
  );

  return {
    id,
    runId: requireNullableStringField(record, "run_id", path),
    text,
    content,
    timestamp: requireNullableStringField(record, "timestamp", path),
    status: "awaiting_ack",
    active,
  };
}

function mapThreadChannelBinding(
  value: unknown,
  path: string,
): ThreadChannelBindingInfo {
  const record = requireContractRecord(value, path);
  const optionalString = (field: string): string | null =>
    optionalNullableStringField(record, field, `${path}.${field}`);
  const chatId = optionalString("chat_id") ?? "";
  return {
    channel: optionalString("channel") ?? "",
    accountId: optionalString("account_id") ?? "",
    bindingKey: optionalString("binding_key") ?? "",
    chatId,
    deliveryTargetType: optionalString("delivery_target_type") ?? "chat_id",
    deliveryTargetId: optionalString("delivery_target_id") ?? chatId,
    displayLabel: optionalString("display_label") ?? "",
    lastInboundAt: optionalString("last_inbound_at"),
    lastDeliveryAt: optionalString("last_delivery_at"),
  };
}

function mapThreadRuntimeInfo(
  value: unknown,
  detail: Record<string, unknown> | null,
): ThreadRuntimeInfo | null {
  if (value === null) {
    return null;
  }
  const runtime = requireContractRecord(value, "thread runtime");
  const providerType = parseThreadProviderType(
    requireContractField(runtime, "provider_type", "thread runtime"),
    "thread runtime.provider_type",
  );
  const channelBindings = detail && hasContractField(detail, "channel_bindings")
    ? requireContractArray(
        detail.channel_bindings,
        "thread metadata.channel_bindings",
      ).map((entry, index) =>
        mapThreadChannelBinding(
          entry,
          `thread metadata.channel_bindings[${index}]`,
        ),
      )
    : [];
  const activeRun = mapThreadActiveRun(
    requireContractField(runtime, "active_run", "thread runtime"),
    "thread runtime.active_run",
  );
  return {
    agentId: requireNullableStringField(runtime, "agent_id", "thread runtime"),
    providerType,
    providerLabel: requireContractString(
      requireContractField(runtime, "provider_label", "thread runtime"),
      "thread runtime.provider_label",
    ),
    model: requireNullableStringField(runtime, "model", "thread runtime"),
    modelReasoningEffort: requireNullableStringField(
      runtime,
      "model_reasoning_effort",
      "thread runtime",
    ),
    modelServiceTier: requireNullableStringField(
      runtime,
      "model_service_tier",
      "thread runtime",
    ),
    modelOverride: requireNullableStringField(
      runtime,
      "model_override",
      "thread runtime",
    ),
    modelReasoningEffortOverride: requireNullableStringField(
      runtime,
      "model_reasoning_effort_override",
      "thread runtime",
    ),
    modelServiceTierOverride: requireNullableStringField(
      runtime,
      "model_service_tier_override",
      "thread runtime",
    ),
    sdkSessionId: requireNullableStringField(
      runtime,
      "sdk_session_id",
      "thread runtime",
    ),
    workspacePath: detail
      ? optionalNullableStringField(
          detail,
          "workspace_dir",
          "thread metadata.workspace_dir",
        )
      : null,
    worktree: detail && hasContractField(detail, "worktree")
      ? mapThreadWorktreeInfo(detail.worktree)
      : null,
    activeRun,
    channelBindings,
  };
}

function mapThreadActiveRun(
  value: unknown,
  path: string,
): ThreadActiveRunInfo | null {
  if (value === null) {
    return null;
  }
  const record = requireContractRecord(value, path);
  const runId = requireContractNonEmptyString(
    requireContractField(record, "run_id", path),
    `${path}.run_id`,
  );
  const providerType = parseThreadProviderType(
    requireContractField(record, "provider_type", path),
    `${path}.provider_type`,
  );
  const pendingUserInputCount = requireContractNonNegativeInteger(
    requireContractField(record, "pending_user_input_count", path),
    `${path}.pending_user_input_count`,
  );
  return {
    runId,
    providerType,
    providerLabel: requireNullableStringField(record, "provider_label", path),
    assistantResponse: requireNullableStringField(
      record,
      "assistant_response",
      path,
    ),
    updatedAt: requireNullableStringField(record, "updated_at", path),
    pendingUserInputCount,
  };
}

function mapStandardThreadSummary(
  value: unknown,
  context: string,
  requireMetaPreview: boolean,
): DesktopThreadSummary {
  const record = requireContractRecord(value, context);
  const id = requireContractNonEmptyString(
    requireContractField(record, "thread_id", context),
    `${context}.thread_id`,
  );
  requireContractNonEmptyString(
    requireContractField(record, "thread_key", context),
    `${context}.thread_key`,
  );
  const threadType = requireContractNonEmptyString(
    requireContractField(record, "thread_type", context),
    `${context}.thread_type`,
  );
  const label = requireNullableStringField(record, "label", context);
  const workspacePath = requireNullableStringField(
    record,
    "workspace_dir",
    context,
  );
  requireContractArray(
    requireContractField(record, "channel_bindings", context),
    `${context}.channel_bindings`,
  );
  const updatedAt = requireNullableStringField(record, "updated_at", context);
  const createdAt = requireNullableStringField(record, "created_at", context);
  const messageCount = requireContractNonNegativeInteger(
    requireContractField(record, "message_count", context),
    `${context}.message_count`,
  );
  const lastUserMessage = requireNullableStringField(
    record,
    "last_user_message",
    context,
  );
  const lastAssistantMessage = requireNullableStringField(
    record,
    "last_assistant_message",
    context,
  );
  const agentId = requireNullableStringField(record, "agent_id", context);
  requireNullableStringField(record, "provider_type", context);
  const worktreeValue = requireContractField(record, "worktree", context);
  const recentRunId = requireNullableStringField(
    record,
    "recent_run_id",
    context,
  );
  requireNullableStringField(record, "active_run_id", context);
  const lastMessagePreview = requireMetaPreview
    ? requireNullableStringField(record, "last_message_preview", context)
    : null;

  return {
    id,
    title: label?.trim() || id,
    threadType,
    createdAt: createdAt || EPOCH_ISO,
    updatedAt: updatedAt || createdAt || EPOCH_ISO,
    lastMessagePreview:
      lastMessagePreview?.trim() ||
      lastAssistantMessage?.trim() ||
      lastUserMessage?.trim() ||
      "",
    workspacePath,
    messageCount,
    agentId,
    recentRunId,
    runState: null,
    worktree: mapThreadWorktreeInfo(worktreeValue),
  };
}

function mapRecentThreadSummary(
  value: unknown,
  context: string,
): DesktopThreadSummary {
  const record = requireContractRecord(value, context);
  const id = requireContractNonEmptyString(
    requireContractField(record, "thread_id", context),
    `${context}.thread_id`,
  );
  const title = requireContractString(
    requireContractField(record, "title", context),
    `${context}.title`,
  );
  const workspacePath = requireNullableStringField(
    record,
    "workspace_dir",
    context,
  );
  const threadType = requireContractNonEmptyString(
    requireContractField(record, "thread_type", context),
    `${context}.thread_type`,
  );
  requireNullableStringField(record, "provider_type", context);
  const agentId = requireNullableStringField(record, "agent_id", context);
  const messageCount = requireContractNonNegativeInteger(
    requireContractField(record, "message_count", context),
    `${context}.message_count`,
  );
  const lastMessagePreview = requireContractString(
    requireContractField(record, "last_message_preview", context),
    `${context}.last_message_preview`,
  );
  const recentRunId = requireNullableStringField(
    record,
    "recent_run_id",
    context,
  );
  requireNullableStringField(record, "active_run_id", context);
  const runState = requireContractNonEmptyString(
    requireContractField(record, "run_state", context),
    `${context}.run_state`,
  );
  const updatedAt = requireNullableStringField(record, "updated_at", context);
  const lastActiveAt = requireContractNonEmptyString(
    requireContractField(record, "last_active_at", context),
    `${context}.last_active_at`,
  );
  const recordedAt = requireContractNonEmptyString(
    requireContractField(record, "recorded_at", context),
    `${context}.recorded_at`,
  );
  const activitySeq = requireContractNonNegativeInteger(
    requireContractField(record, "activity_seq", context),
    `${context}.activity_seq`,
  );

  return {
    id,
    title: title.trim() || id,
    threadType,
    createdAt: recordedAt,
    updatedAt: updatedAt || lastActiveAt,
    lastMessagePreview: lastMessagePreview.trim(),
    workspacePath,
    messageCount,
    agentId,
    recentRunId,
    runState,
    activitySeq,
    worktree: null,
  };
}

function mapThreadMetadataSummary(
  value: unknown,
  context: string,
): DesktopThreadSummary {
  const record = requireContractRecord(value, context);
  const id = requireContractNonEmptyString(
    requireContractField(record, "thread_id", context),
    `${context}.thread_id`,
  );
  requireContractNonEmptyString(
    requireContractField(record, "thread_key", context),
    `${context}.thread_key`,
  );
  const threadType = requireContractNonEmptyString(
    requireContractField(record, "thread_type", context),
    `${context}.thread_type`,
  );
  const label = optionalNullableStringField(record, "label", `${context}.label`);
  const createdAt = optionalNullableStringField(
    record,
    "created_at",
    `${context}.created_at`,
  );
  const updatedAt = optionalNullableStringField(
    record,
    "updated_at",
    `${context}.updated_at`,
  );
  const lastMessagePreview = optionalNullableStringField(
    record,
    "last_message_preview",
    `${context}.last_message_preview`,
  );
  const lastAssistantMessage = optionalNullableStringField(
    record,
    "last_assistant_message",
    `${context}.last_assistant_message`,
  );
  const lastUserMessage = optionalNullableStringField(
    record,
    "last_user_message",
    `${context}.last_user_message`,
  );
  const messageCount = hasContractField(record, "message_count")
    ? requireContractNonNegativeInteger(record.message_count, `${context}.message_count`)
    : undefined;
  const worktree = hasContractField(record, "worktree")
    ? mapThreadWorktreeInfo(record.worktree)
    : null;

  return {
    id,
    title: label?.trim() || id,
    threadType,
    createdAt: createdAt || EPOCH_ISO,
    updatedAt: updatedAt || createdAt || EPOCH_ISO,
    lastMessagePreview:
      lastMessagePreview?.trim() ||
      lastAssistantMessage?.trim() ||
      lastUserMessage?.trim() ||
      "",
    workspacePath: optionalNullableStringField(
      record,
      "workspace_dir",
      `${context}.workspace_dir`,
    ),
    messageCount,
    agentId: optionalNullableStringField(record, "agent_id", `${context}.agent_id`),
    recentRunId: optionalNullableStringField(
      record,
      "recent_run_id",
      `${context}.recent_run_id`,
    ),
    runState: optionalNullableStringField(
      record,
      "run_state",
      `${context}.run_state`,
    ),
    worktree,
  };
}

export function mapThreadSummary(value: unknown): DesktopThreadSummary {
  return mapStandardThreadSummary(value, "standard thread summary", false);
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
  payload: Record<string, unknown>,
  limit: number,
  remoteFound: boolean,
): ThreadTranscript["pageInfo"] {
  const stats = requireContractRecord(
    requireContractField(payload, "message_stats", "thread history"),
    "thread history.message_stats",
  );
  const returnedMessages = requireContractNonNegativeInteger(
    requireContractField(
      stats,
      "returned_messages",
      "thread history.message_stats",
    ),
    "thread history.message_stats.returned_messages",
  );
  if (!remoteFound) {
    return {
      totalMessages: 0,
      committedMessages: null,
      returnedMessages,
      returnedUserQueries: null,
      startIndex: 0,
      endIndex: 0,
      hasMoreBefore: false,
      nextBeforeIndex: null,
      hasMoreAfter: false,
      nextAfterIndex: null,
      reset: false,
      limit,
      userQueryLimit: null,
    };
  }
  const statsContext = "thread history.message_stats";
  const requiredCount = (field: string) =>
    requireContractNonNegativeInteger(
      requireContractField(stats, field, statsContext),
      `${statsContext}.${field}`,
    );
  const nullableCount = (field: string) => {
    const value = requireContractField(stats, field, statsContext);
    return value === null
      ? null
      : requireContractNonNegativeInteger(value, `${statsContext}.${field}`);
  };
  const totalMessages = requiredCount("total_messages_in_thread");
  requiredCount("total_messages_in_session");
  const committedMessages = requiredCount("committed_message_count");
  const returnedUserQueries = requiredCount("returned_user_queries");
  const startIndex = requiredCount("returned_start_index");
  const endIndex = requiredCount("returned_end_index");
  const nextBeforeIndex = nullableCount("next_before_index");
  const nextAfterIndex = nullableCount("next_after_index");
  const userQueryLimit = nullableCount("user_query_limit");
  return {
    totalMessages,
    committedMessages,
    returnedMessages,
    returnedUserQueries,
    startIndex,
    endIndex,
    hasMoreBefore: requireContractBoolean(
      requireContractField(stats, "has_more_before", statsContext),
      `${statsContext}.has_more_before`,
    ),
    nextBeforeIndex,
    hasMoreAfter: requireContractBoolean(
      requireContractField(stats, "has_more_after", statsContext),
      `${statsContext}.has_more_after`,
    ),
    nextAfterIndex,
    reset: requireContractBoolean(
      requireContractField(stats, "reset", statsContext),
      `${statsContext}.reset`,
    ),
    limit,
    userQueryLimit,
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
  const [payloadValue, detailValue] = await Promise.all([
    requestJson<unknown>(
      settings,
      `/api/threads/history?${query.toString()}`,
      "readRetryable",
      {
        signal: AbortSignal.timeout(8000),
      },
    ),
    beforeIndex === undefined && afterIndex === undefined
      ? requestJson<unknown>(
          settings,
          `/api/threads/${encodeURIComponent(threadId)}`,
          "readRetryable",
          {
            signal: AbortSignal.timeout(8000),
          },
        ).catch((error) => {
          if (error instanceof GatewayContractError) {
            throw error;
          }
          return null;
        })
      : Promise.resolve(null),
  ]);

  const payload = requireContractRecord(payloadValue, "thread history");
  const remoteFound = requireContractBoolean(
    requireContractField(payload, "ok", "thread history"),
    "thread history.ok",
  );
  const rawMessages = requireContractArray(
    requireContractField(payload, "messages", "thread history"),
    "thread history.messages",
  );
  const rawPendingInputs = requireContractArray(
    requireContractField(payload, "pending_user_inputs", "thread history"),
    "thread history.pending_user_inputs",
  );
  const historyRuntime = requireContractField(
    payload,
    "thread_runtime",
    "thread history",
  );
  if (historyRuntime !== null) {
    requireContractRecord(historyRuntime, "thread history.thread_runtime");
  }
  const detail = detailValue
    ? requireContractRecord(detailValue, "thread metadata")
    : null;
  const messages = rawMessages
    .map((value, index) =>
      mapHistoryMessage(threadId, value, `thread history.messages[${index}]`),
    )
    .filter((value): value is TranscriptMessage => Boolean(value));
  const pendingInputs = rawPendingInputs.map((value, index) =>
    mapPendingUserInput(
      value,
      `thread history.pending_user_inputs[${index}]`,
    ),
  );
  return {
    threadId,
    remoteFound,
    messages,
    pendingInputs,
    thread: detail
      ? mapThreadMetadataSummary(detail, "thread metadata")
      : null,
    threadInfo: mapThreadRuntimeInfo(historyRuntime, detail),
    pageInfo: mapThreadTranscriptPageInfo(payload, limit, remoteFound),
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
  const payloadValue = await requestJson<unknown>(
    settings,
    `/api/threads/${encodeURIComponent(threadId)}/logs${suffix}`,
    "readRetryable",
    {
      signal: AbortSignal.timeout(8000),
    },
  );
  const payload = requireContractRecord(payloadValue, "thread log chunk");

  return {
    threadId: requireContractNonEmptyString(
      requireContractField(payload, "threadId", "thread log chunk"),
      "thread log chunk.threadId",
    ),
    path: requireContractString(
      requireContractField(payload, "path", "thread log chunk"),
      "thread log chunk.path",
    ),
    text: requireContractString(
      requireContractField(payload, "text", "thread log chunk"),
      "thread log chunk.text",
    ),
    cursor: requireContractNonNegativeInteger(
      requireContractField(payload, "cursor", "thread log chunk"),
      "thread log chunk.cursor",
    ),
    reset: requireContractBoolean(
      requireContractField(payload, "reset", "thread log chunk"),
      "thread log chunk.reset",
    ),
  };
}

export async function fetchThreads(
  settings: DesktopSettings,
  options?: { limit?: number },
): Promise<DesktopThreadSummary[]> {
  const limit = options?.limit ?? 1000;
  const payloadValue = await requestJson<unknown>(
    settings,
    `/api/threads?limit=${limit}`,
    "readRetryable",
    {
      signal: AbortSignal.timeout(REMOTE_STATE_FETCH_TIMEOUT_MS),
    },
  );
  const payload = requireContractRecord(payloadValue, "thread page");
  const threads = requireContractArray(
    requireContractField(payload, "threads", "thread page"),
    "thread page.threads",
  );
  for (const field of ["count", "total", "limit", "offset"] as const) {
    requireContractNonNegativeInteger(
      requireContractField(payload, field, "thread page"),
      `thread page.${field}`,
    );
  }
  return threads.map((thread, index) =>
    mapStandardThreadSummary(
      thread,
      `thread page.threads[${index}]`,
      true,
    ),
  );
}

export function validateListRecentThreadsInput(
  value: unknown,
): ListRecentThreadsInput {
  const input = parseRecord(value);
  const gatewayScope = normalizeGatewayUrl(asString(input.gatewayScope) || "");
  if (!gatewayScope) {
    throw new Error("gatewayScope is required");
  }
  const tasks = input.tasks;
  if (tasks !== "include" && tasks !== "exclude") {
    throw new Error("tasks must be include or exclude");
  }
  const limit = input.limit;
  if (
    typeof limit !== "number" ||
    !Number.isSafeInteger(limit) ||
    limit < 1 ||
    limit > MAX_RECENT_THREAD_PAGE_SIZE
  ) {
    throw new Error("limit must be an integer between 1 and 200");
  }
  const cursor = input.cursor;
  if (cursor !== null && (typeof cursor !== "string" || !cursor.trim())) {
    throw new Error("cursor must be null or a non-empty opaque string");
  }
  return { gatewayScope, tasks, limit, cursor };
}

export function assertRecentThreadGatewayScope(
  settings: DesktopSettings,
  expectedGatewayScope: string,
): string {
  const gatewayScope = normalizeGatewayUrl(settings.gatewayUrl);
  if (gatewayScope !== expectedGatewayScope) {
    throw new Error("Gateway changed before the Recent request started");
  }
  return gatewayScope;
}

export async function fetchRecentThreads(
  settings: DesktopSettings,
  options: Pick<ListRecentThreadsInput, "tasks" | "limit" | "cursor">,
): Promise<DesktopRecentThreadsPage> {
  const query = new URLSearchParams({
    tasks: options.tasks,
    limit: String(options.limit),
  });
  if (options.cursor !== null) {
    query.set("cursor", options.cursor);
  }
  const payloadValue = await requestJson<unknown>(
    settings,
    `/api/recent-threads?${query.toString()}`,
    "readRetryable",
    {
      signal: AbortSignal.timeout(REMOTE_STATE_FETCH_TIMEOUT_MS),
    },
  );
  const payload = requireContractRecord(payloadValue, "recent thread page");
  const rawThreads = requireContractArray(
    requireContractField(payload, "threads", "recent thread page"),
    "recent thread page.threads",
  );
  const threads = rawThreads.map((thread, index) =>
    mapRecentThreadSummary(
      thread,
      `recent thread page.threads[${index}]`,
    ),
  );
  const count = requireContractNonNegativeInteger(
    requireContractField(payload, "count", "recent thread page"),
    "recent thread page.count",
  );
  const limit = requireContractNonNegativeInteger(
    requireContractField(payload, "limit", "recent thread page"),
    "recent thread page.limit",
  );
  const total = requireContractNonNegativeInteger(
    requireContractField(payload, "total", "recent thread page"),
    "recent thread page.total",
  );
  const hasMore = requireContractBoolean(
    requireContractField(payload, "has_more", "recent thread page"),
    "recent thread page.has_more",
  );
  const nextCursorValue = requireContractField(
    payload,
    "next_cursor",
    "recent thread page",
  );
  const nextCursor = nextCursorValue === null
    ? null
    : requireContractNonEmptyString(
        nextCursorValue,
        "recent thread page.next_cursor",
      );
  if (
    count !== threads.length ||
    limit < 1 ||
    limit > 200 ||
    total < count ||
    hasMore !== (nextCursor !== null)
  ) {
    throw new GatewayContractError(
      "recent thread page",
      "violates the cursor/count contract",
    );
  }
  return {
    gatewayScope: normalizeGatewayUrl(settings.gatewayUrl),
    storeIncarnationId: requireContractNonEmptyString(
      requireContractField(
        payload,
        "store_incarnation_id",
        "recent thread page",
      ),
      "recent thread page.store_incarnation_id",
    ),
    serverBootId: requireContractNonEmptyString(
      requireContractField(payload, "server_boot_id", "recent thread page"),
      "recent thread page.server_boot_id",
    ),
    threads,
    count,
    total,
    limit,
    hasMore,
    nextCursor,
  };
}

function mapThreadFavoritesPage(
  value: unknown,
  context = "thread favorites",
): DesktopThreadFavoritesPage {
  const payload = requireContractRecord(value, context);
  const rawFavorites = requireContractArray(
    requireContractField(payload, "favorites", context),
    `${context}.favorites`,
  );
  const favorites = rawFavorites.map((favorite, index) => {
    const path = `${context}.favorites[${index}]`;
    const record = requireContractRecord(favorite, path);
    return {
      threadId: requireContractNonEmptyString(
        requireContractField(record, "thread_id", path),
        `${path}.thread_id`,
      ),
      favoritedAt: requireContractNonEmptyString(
        requireContractField(record, "favorited_at", path),
        `${path}.favorited_at`,
      ),
    };
  });
  const rawThreadIds = requireContractArray(
    requireContractField(payload, "thread_ids", context),
    `${context}.thread_ids`,
  );
  const threadIds = rawThreadIds.map((threadId, index) =>
    requireContractNonEmptyString(threadId, `${context}.thread_ids[${index}]`),
  );
  if (
    new Set(threadIds).size !== threadIds.length ||
    threadIds.length !== favorites.length ||
    threadIds.some((threadId, index) => favorites[index]?.threadId !== threadId)
  ) {
    throw new GatewayContractError(
      context,
      "must expose the same unique ordered membership in thread_ids and favorites",
    );
  }
  return {
    storeIncarnationId: requireContractNonEmptyString(
      requireContractField(payload, "store_incarnation_id", context),
      `${context}.store_incarnation_id`,
    ),
    serverBootId: requireContractNonEmptyString(
      requireContractField(payload, "server_boot_id", context),
      `${context}.server_boot_id`,
    ),
    revision: requireContractNonNegativeInteger(
      requireContractField(payload, "revision", context),
      `${context}.revision`,
    ),
    threadIds,
    favorites,
  };
}

export async function fetchThreadFavorites(
  settings: DesktopSettings,
): Promise<DesktopThreadFavoritesPage> {
  const payload = await requestJson<unknown>(
    settings,
    "/api/thread-favorites",
    "readRetryable",
    { signal: AbortSignal.timeout(REMOTE_STATE_FETCH_TIMEOUT_MS) },
  );
  return mapThreadFavoritesPage(payload);
}

export async function fetchThreadFavoritesSnapshot(
  settings: DesktopSettings,
): Promise<DesktopThreadFavoritesSnapshot> {
  const payloadValue = await requestJson<unknown>(
    settings,
    "/api/thread-favorites/snapshot",
    "readRetryable",
    { signal: AbortSignal.timeout(REMOTE_STATE_FETCH_TIMEOUT_MS) },
  );
  const payload = requireContractRecord(payloadValue, "thread favorites snapshot");
  const page = mapThreadFavoritesPage(payload, "thread favorites snapshot");
  const recent = requireContractRecord(
    requireContractField(payload, "recent", "thread favorites snapshot"),
    "thread favorites snapshot.recent",
  );
  const rawThreads = requireContractArray(
    requireContractField(recent, "threads", "thread favorites snapshot.recent"),
    "thread favorites snapshot.recent.threads",
  );
  const recentThreads = rawThreads.map((thread, index) =>
    mapRecentThreadSummary(
      thread,
      `thread favorites snapshot.recent.threads[${index}]`,
    ),
  );
  const recentTotal = requireContractNonNegativeInteger(
    requireContractField(recent, "total", "thread favorites snapshot.recent"),
    "thread favorites snapshot.recent.total",
  );
  const recentTruncated = requireContractBoolean(
    requireContractField(
      recent,
      "truncated",
      "thread favorites snapshot.recent",
    ),
    "thread favorites snapshot.recent.truncated",
  );
  if (
    recentTotal < recentThreads.length ||
    (!recentTruncated && recentTotal !== recentThreads.length)
  ) {
    throw new GatewayContractError(
      "thread favorites snapshot.recent",
      "violates the total/truncated contract",
    );
  }
  return {
    ...page,
    recent: {
      threads: recentThreads,
      total: recentTotal,
      truncated: recentTruncated,
    },
  };
}

export async function setRemoteThreadFavorite(
  settings: DesktopSettings,
  input: {
    threadId: string;
    favorited: boolean;
    expectedRevision: number;
    expectedStoreIncarnation: string;
  },
): Promise<GatewayMutationResult<DesktopThreadFavoritesPage>> {
  const threadId = input.threadId.trim();
  const storeIncarnation = input.expectedStoreIncarnation.trim();
  if (
    !threadId.startsWith("thread::") ||
    typeof input.favorited !== "boolean" ||
    !Number.isSafeInteger(input.expectedRevision) ||
    input.expectedRevision < 0 ||
    !storeIncarnation
  ) {
    return {
      kind: "notSent",
      message: "The favorites mutation is missing a valid precondition.",
    };
  }
  const query = new URLSearchParams({
    expected_revision: String(input.expectedRevision),
    expected_store_incarnation: storeIncarnation,
  });
  const operation = input.favorited
    ? "thread_favorites_put"
    : "thread_favorites_delete";
  return requestMutationJson<DesktopThreadFavoritesPage>(
    settings,
    `/api/thread-favorites/${encodeURIComponent(threadId)}?${query.toString()}`,
    "mutationSingleAttempt",
    operation,
    {
      method: input.favorited ? "PUT" : "DELETE",
      signal: AbortSignal.timeout(8000),
    },
    (payload) => mapThreadFavoritesPage(payload, "thread favorites mutation"),
  );
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
    const payload = await requestJson<unknown>(
      settings,
      `/api/threads/${encodeURIComponent(threadId)}`,
      "readRetryable",
      {
        signal: AbortSignal.timeout(8000),
      },
    );
    return mapThreadMetadataSummary(payload, "thread metadata");
  } catch (error) {
    if (error instanceof GatewayContractError) {
      throw error;
    }
    return null;
  }
}

export function mapThreadPinsPage(value: unknown): DesktopThreadPinsPage {
  const payload = requireContractRecord(value, "thread pins");
  const rawIds = requireContractArray(
    requireContractField(payload, "thread_ids", "thread pins"),
    "thread pins.thread_ids",
  );
  const seen = new Set<string>();
  const ids: string[] = [];
  for (const [index, rawId] of rawIds.entries()) {
    const id = requireContractNonEmptyString(
      rawId,
      `thread pins.thread_ids[${index}]`,
    );
    if (seen.has(id)) {
      continue;
    }
    seen.add(id);
    ids.push(id);
  }
  return {
    threadIds: ids,
    revision: requireContractNonNegativeInteger(
      requireContractField(payload, "revision", "thread pins"),
      "thread pins.revision",
    ),
  };
}

export async function fetchThreadPins(
  settings: DesktopSettings,
): Promise<DesktopThreadPinsPage> {
  const payload = await requestJson<unknown>(
    settings,
    "/api/thread-pins",
    "readRetryable",
    {
      signal: AbortSignal.timeout(REMOTE_STATE_FETCH_TIMEOUT_MS),
    },
  );
  return mapThreadPinsPage(payload);
}

export async function setRemoteThreadPinned(
  settings: DesktopSettings,
  threadId: string,
  pinned: boolean,
): Promise<DesktopThreadPinsPage> {
  const payload = await requestJson<unknown>(
    settings,
    `/api/thread-pins/${encodeURIComponent(threadId)}`,
    "mutationSingleAttempt",
    {
      method: pinned ? "PUT" : "DELETE",
      signal: AbortSignal.timeout(8000),
    },
  );
  return mapThreadPinsPage(payload);
}

export async function reorderRemoteThreadPins(
  settings: DesktopSettings,
  threadIds: string[],
  expectedRevision: number,
): Promise<{ kind: "accepted" | "conflict"; page: DesktopThreadPinsPage }> {
  try {
    const payload = await requestJson<unknown>(
      settings,
      "/api/thread-pins",
      "mutationSingleAttempt",
      {
        method: "PUT",
        signal: AbortSignal.timeout(8000),
        body: JSON.stringify({
          thread_ids: threadIds,
          expected_revision: expectedRevision,
        }),
      },
    );
    return { kind: "accepted", page: mapThreadPinsPage(payload) };
  } catch (error) {
    if (error instanceof GatewayRequestError && error.status === 409) {
      const payload = tryParseJson<unknown>(error.body);
      if (payload !== null) {
        return { kind: "conflict", page: mapThreadPinsPage(payload) };
      }
    }
    throw error;
  }
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
    sdkSessionProviderHint?: "claude" | "codex" | null;
    forkFromThreadId?: string | null;
    metadata?: Record<string, unknown> | null;
  },
): Promise<DesktopThreadSummary> {
  const payload = await requestJson<unknown>(
    settings,
    "/api/threads",
    "mutationSingleAttempt",
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
  return mapStandardThreadSummary(payload, "created thread summary", false);
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
  const payload = await requestJson<unknown>(
    settings,
    `/api/threads/${encodeURIComponent(threadId)}`,
    "mutationSingleAttempt",
    {
      method: "PATCH",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify(body),
    },
  );
  return mapStandardThreadSummary(payload, "updated thread summary", false);
}

export async function deleteRemoteThread(
  settings: DesktopSettings,
  threadId: string,
  operationId: string,
  expectedStoreIncarnation: string,
): Promise<GatewayMutationResult<unknown>> {
  const normalizedThreadId = threadId.trim();
  const normalizedOperationId = operationId.trim();
  const normalizedIncarnation = expectedStoreIncarnation.trim();
  if (
    !normalizedThreadId.startsWith("thread::") ||
    !isUuid(normalizedOperationId) ||
    !isUuid(normalizedIncarnation)
  ) {
    return {
      kind: "notSent",
      message: "The thread lifecycle mutation is missing a valid identity.",
    };
  }
  return requestMutationJson<unknown>(
    settings,
    `/api/threads/${encodeURIComponent(normalizedThreadId)}`,
    "mutationSingleAttempt",
    "thread_delete",
    {
      method: "DELETE",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        operationId: normalizedOperationId,
        expectedStoreIncarnation: normalizedIncarnation,
      }),
    },
    (payload) =>
      decodeLifecycleSuccessPayload(
        payload,
        "delete",
        normalizedOperationId,
        normalizedThreadId,
      ),
  );
}

export async function archiveRemoteThread(
  settings: DesktopSettings,
  threadId: string,
  operationId: string,
  expectedStoreIncarnation: string,
  endpointKeys: string[] = [],
): Promise<GatewayMutationResult<unknown>> {
  const normalizedThreadId = threadId.trim();
  const normalizedOperationId = operationId.trim();
  const normalizedIncarnation = expectedStoreIncarnation.trim();
  if (
    !normalizedThreadId.startsWith("thread::") ||
    !isUuid(normalizedOperationId) ||
    !isUuid(normalizedIncarnation)
  ) {
    return {
      kind: "notSent",
      message: "The thread lifecycle mutation is missing a valid identity.",
    };
  }
  return requestMutationJson<unknown>(
    settings,
    `/api/threads/${encodeURIComponent(normalizedThreadId)}/archive`,
    "mutationSingleAttempt",
    "thread_archive",
    {
      method: "POST",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        operationId: normalizedOperationId,
        expectedStoreIncarnation: normalizedIncarnation,
        endpointKeys,
      }),
    },
    (payload) =>
      decodeLifecycleSuccessPayload(
        payload,
        "archive",
        normalizedOperationId,
        normalizedThreadId,
      ),
  );
}

function decodeLifecycleSuccessPayload(
  payload: unknown,
  kind: "archive" | "delete",
  operationId: string,
  threadId: string,
): unknown {
  const context = `thread ${kind} response`;
  const record = requireContractRecord(payload, context);
  const echoedOperationId = requireContractNonEmptyString(
    requireContractField(record, "operation_id", context),
    `${context}.operation_id`,
  );
  if (echoedOperationId !== operationId) {
    throw new GatewayContractError(
      `${context}.operation_id`,
      "must match the request operation ID",
    );
  }
  const echoedThreadId = requireContractNonEmptyString(
    requireContractField(record, "thread_id", context),
    `${context}.thread_id`,
  );
  if (echoedThreadId !== threadId) {
    throw new GatewayContractError(
      `${context}.thread_id`,
      "must match the requested thread",
    );
  }
  const outcome = requireContractNonEmptyString(
    requireContractField(record, "outcome", context),
    `${context}.outcome`,
  );
  if (outcome !== "applied_changed" && outcome !== "applied_noop") {
    throw new GatewayContractError(
      `${context}.outcome`,
      "must be an applied lifecycle outcome",
    );
  }
  const changed = requireContractBoolean(
    requireContractField(record, "changed", context),
    `${context}.changed`,
  );
  if (changed !== (outcome === "applied_changed")) {
    throw new GatewayContractError(
      `${context}.changed`,
      "must agree with the lifecycle outcome",
    );
  }
  if (
    requireContractBoolean(
      requireContractField(record, "deleted", context),
      `${context}.deleted`,
    ) !== true
  ) {
    throw new GatewayContractError(`${context}.deleted`, "must be true");
  }
  if (
    kind === "archive" &&
    requireContractBoolean(
      requireContractField(record, "archived", context),
      `${context}.archived`,
    ) !== true
  ) {
    throw new GatewayContractError(`${context}.archived`, "must be true");
  }
  requireContractArray(
    requireContractField(record, "detached_endpoint_keys", context),
    `${context}.detached_endpoint_keys`,
  ).forEach((value, index) => {
    requireContractString(value, `${context}.detached_endpoint_keys[${index}]`);
  });
  return payload;
}

function isUuid(value: string): boolean {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/iu.test(
    value.trim(),
  );
}

export const fetchSessions = fetchThreads;

export const createRemoteSession = createRemoteThread;

export const updateRemoteSession = updateRemoteThread;

export const deleteRemoteSession = deleteRemoteThread;
