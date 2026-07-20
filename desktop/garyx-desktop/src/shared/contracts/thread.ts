import type {
  DesktopSessionProviderHint,
  DesktopThreadProviderType,
} from "./provider.ts";
import type {
  MessageFileAttachment,
  MessageImageAttachment,
  RenderState,
  TranscriptMessage,
} from "./transcript.ts";
import type { DesktopWorkspaceMode } from "./workspace.ts";

export interface DesktopThreadSummary {
  id: string;
  title: string;
  threadType?: string | null;
  createdAt: string;
  updatedAt: string;
  lastMessagePreview: string;
  workspacePath?: string | null;
  /** Server-derived root-workspace membership: worktree threads map back to
   *  their source workspace, implicit threads to null. Clients never derive
   *  this locally. */
  rootWorkspacePath?: string | null;
  /** Server-owned provenance: "explicit" (user-chosen directory, including
   *  worktrees) or "implicit" (Garyx-managed No-workspace directory). */
  workspaceOrigin?: string | null;
  messageCount?: number;
  agentId?: string | null;
  recentRunId?: string | null;
  runState?: string | null;
  /** Monotonic gateway ordering key, present on recent/snapshot rows. */
  activitySeq?: number | null;
  worktree?: ThreadWorktreeInfo | null;
}

export type RecentThreadTaskFilter = "include" | "exclude";

export interface ListRecentThreadsInput {
  /** Normalized Gateway URL expected by the renderer-owned feed ticket. */
  gatewayScope: string;
  tasks: RecentThreadTaskFilter;
  limit: number;
  cursor: string | null;
}

export interface DesktopRecentThreadsPage {
  /** Normalized Gateway URL actually used by the main-process request. */
  gatewayScope: string;
  storeIncarnationId: string;
  serverBootId: string;
  threads: DesktopThreadSummary[];
  count: number;
  total: number;
  limit: number;
  hasMore: boolean;
  nextCursor: string | null;
}

export interface DesktopThreadFavoriteRecord {
  threadId: string;
  favoritedAt: string;
}

export interface DesktopThreadFavoritesPage {
  storeIncarnationId: string;
  serverBootId: string;
  revision: number;
  threadIds: string[];
  favorites: DesktopThreadFavoriteRecord[];
}

export interface DesktopThreadFavoritesSnapshot
  extends DesktopThreadFavoritesPage {
  recent: {
    threads: DesktopThreadSummary[];
    total: number;
    truncated: boolean;
  };
}

export interface ThreadFavoritesReadInput {
  /** Normalized Gateway URL captured by the renderer-owned request ticket. */
  gatewayScope: string;
}

export interface SetThreadFavoriteInput extends ThreadFavoritesReadInput {
  threadId: string;
  favorited: boolean;
  expectedRevision: number;
  expectedStoreIncarnation: string;
}

export interface DesktopTaggedApiError {
  kind: "garyx_api_error";
  operation: string;
  code: string;
  message?: string;
  [key: string]: unknown;
}

/** Serializable four-way mutation transport result crossing Electron IPC. */
export type DesktopGatewayMutationResult<T> =
  | { kind: "ok"; value: T; status: number }
  | {
      kind: "definitiveEndpointResponse";
      status: number;
      error: DesktopTaggedApiError;
      value: T | null;
      body: string;
    }
  | {
      kind: "ambiguous";
      message: string;
      status?: number;
      body?: string;
    }
  | { kind: "notSent"; message: string };

export interface ThreadWorktreeInfo {
  mode?: string | null;
  enabled?: boolean | null;
  branch?: string | null;
  sourceBranch?: string | null;
  path?: string | null;
  worktreeDir?: string | null;
  sourceWorkspaceDir?: string | null;
  sourceRepoRoot?: string | null;
}

export interface ThreadChannelBindingInfo {
  channel: string;
  accountId: string;
  bindingKey: string;
  chatId: string;
  deliveryTargetType: string;
  deliveryTargetId: string;
  displayLabel: string;
  lastInboundAt?: string | null;
  lastDeliveryAt?: string | null;
}

export interface ThreadRuntimeInfo {
  agentId?: string | null;
  providerType?: DesktopThreadProviderType | null;
  providerLabel?: string | null;
  model?: string | null;
  modelReasoningEffort?: string | null;
  modelServiceTier?: string | null;
  modelOverride?: string | null;
  modelReasoningEffortOverride?: string | null;
  modelServiceTierOverride?: string | null;
  sdkSessionId?: string | null;
  workspacePath?: string | null;
  worktree?: ThreadWorktreeInfo | null;
  activeRun?: ThreadActiveRunInfo | null;
  channelBindings: ThreadChannelBindingInfo[];
}

export interface ThreadActiveRunInfo {
  runId: string;
  providerType?: DesktopThreadProviderType | null;
  providerLabel?: string | null;
  assistantResponse?: string | null;
  updatedAt?: string | null;
  pendingUserInputCount?: number;
}

// Offline cache bundle: committed transcript plus the last render snapshot, so a
// cold/offline thread open can render folded history before the first live frame.
export interface CachedThreadTranscript {
  transcript: ThreadTranscript;
  renderState: RenderState | null;
}

export type DesktopDeepLinkEvent =
  | {
      type: "open-thread";
      url: string;
      threadId: string;
    }
  | {
      type: "new-thread";
      url: string;
      workspacePath?: string | null;
      agentId?: string | null;
    }
  | {
      type: "resume-session";
      url: string;
      sessionId: string;
      providerHint?: DesktopSessionProviderHint | null;
    }
  | {
      type: "open-capsule";
      url: string;
      capsuleId: string;
    }
  | {
      type: "error";
      url: string;
      error: string;
    };

export type DesktopDeepLinkListener = (event: DesktopDeepLinkEvent) => void;

export interface ThreadTranscript {
  threadId: string;
  remoteFound: boolean;
  messages: TranscriptMessage[];
  pendingInputs: PendingThreadInput[];
  thread?: DesktopThreadSummary | null;
  threadInfo?: ThreadRuntimeInfo | null;
  pageInfo?: ThreadTranscriptPageInfo | null;
}

export interface ThreadTranscriptPageInfo {
  totalMessages: number;
  committedMessages?: number | null;
  returnedMessages: number;
  returnedUserQueries?: number | null;
  startIndex: number;
  endIndex: number;
  hasMoreBefore: boolean;
  nextBeforeIndex?: number | null;
  hasMoreAfter?: boolean;
  nextAfterIndex?: number | null;
  reset?: boolean;
  limit: number;
  userQueryLimit?: number | null;
}

export interface GetThreadHistoryInput {
  threadId: string;
  beforeIndex?: number | null;
  afterIndex?: number | null;
  limit?: number | null;
  userQueryLimit?: number | null;
}

export interface StartThreadStreamInput {
  threadId: string;
  /** Renderer-owned logical stream request identity. Main-process transport
   * retries preserve it and stamp it on every locally forwarded event. */
  requestId: string;
  afterSeq?: number | null;
  consumerId?: string | null;
  /** Render window floor requested for this logical stream. */
  renderFloor?: number | null;
}

export interface StopThreadStreamInput {
  threadId?: string | null;
  consumerId?: string | null;
}

export interface PendingThreadInput {
  id: string;
  runId?: string | null;
  text: string;
  content?: unknown;
  timestamp?: string | null;
  status: "awaiting_ack" | "orphaned";
  active: boolean;
}

export interface ThreadLogChunk {
  threadId: string;
  path: string;
  text: string;
  cursor: number;
  reset: boolean;
}

export interface CreateThreadInput {
  title?: string;
  workspacePath?: string | null;
  /** Explicit No-workspace draft: create without a workspace_dir so the
   *  gateway provisions its private Garyx-managed thread workspace. */
  noWorkspace?: boolean;
  workspaceMode?: DesktopWorkspaceMode;
  /** Agent ID. */
  agentId?: string | null;
  /** Optional per-thread model override; wins over the agent's configured model. */
  model?: string | null;
  /** Optional per-thread reasoning/thinking level override. */
  modelReasoningEffort?: string | null;
  /** Optional per-thread service tier override. */
  modelServiceTier?: string | null;
  /** Optional Claude/Codex provider session id to resume from. Garyx resolves the real local provider/workspace from it. */
  sdkSessionId?: string | null;
  /** Optional provider hint for sdkSessionId. Supported values are claude and codex. */
  sdkSessionProviderHint?: DesktopSessionProviderHint | null;
  /** Optional Garyx thread id to fork from using the provider-native session fork. */
  forkFromThreadId?: string | null;
  /** Optional thread metadata forwarded to the gateway. */
  metadata?: Record<string, unknown> | null;
}

export interface RenameThreadInput {
  threadId: string;
  // Compatibility fallback for older callers. Prefer `threadId`.
  sessionId?: string;
  title: string;
}

export interface UpdateThreadRuntimeSettingsInput {
  threadId: string;
  model?: string | null;
  modelReasoningEffort?: string | null;
  modelServiceTier?: string | null;
}

export interface DeleteThreadInput {
  threadId: string;
  operationId: string;
  expectedStoreIncarnation: string;
}

export interface ArchiveThreadInput {
  threadId: string;
  operationId: string;
  expectedStoreIncarnation: string;
  endpointKeys?: string[];
}

export interface SetThreadPinnedInput {
  threadId: string;
  pinned: boolean;
}

export interface SetThreadPinOrderInput {
  threadIds: string[];
}

export interface DesktopThreadPinsPage {
  threadIds: string[];
  revision: number;
}

export type DesktopThreadPinOrderSyncState =
  | "settled"
  | "ready"
  | "in_flight"
  | "waiting_for_membership"
  | "coalesced_behind_flight"
  | "retry_scheduled"
  | "paused_permanent";

export interface DesktopThreadPinOrderSnapshot {
  gatewayIdentity: string;
  desiredOrder: string[];
  highestObservedRevision: number;
  unsettled: boolean;
  syncState: DesktopThreadPinOrderSyncState;
}

export interface SendMessageInput {
  threadId: string;
  // Compatibility fallback for older callers. Prefer `threadId`.
  sessionId?: string;
  // Stable frontend identity for queued/in-flight user intents.
  clientIntentId?: string;
  message: string;
  images?: MessageImageAttachment[];
  files?: MessageFileAttachment[];
}

export interface OpenChatStreamResult {
  runId: string;
  threadId: string;
  // Compatibility mirror for older responses. Prefer `threadId`.
  sessionId?: string;
  response: string;
  status: "accepted" | "completed" | "disconnected";
  thread: DesktopThreadSummary;
  // Compatibility mirror for older responses. Prefer `thread`.
  session?: DesktopThreadSummary;
}

export interface SendStreamingInputResult {
  status: string;
  threadId: string;
  // Compatibility mirror for older responses. Prefer `threadId`.
  sessionId?: string;
  clientIntentId?: string;
  pendingInputId?: string;
}

export interface InterruptResult {
  status: string;
  threadId: string;
  // Compatibility mirror for older responses. Prefer `threadId`.
  sessionId?: string;
  abortedRuns: string[];
}
