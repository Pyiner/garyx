import {
  Suspense,
  lazy,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { startTransition } from "react";

import {
  DEFAULT_DESKTOP_SETTINGS,
  DEFAULT_SESSION_TITLE,
  type CreateAutomationInput,
  type DesktopApiProviderType,
  type DesktopAutomationActivityEntry,
  type DesktopAutomationActivityFeed,
  type DesktopMcpServer,
  type DesktopMemoryDocument,
  type DesktopAutomationSchedule,
  type DesktopAutomationSummary,
  type DesktopBotConsoleSummary,
  type DesktopCustomAgent,
  type DesktopTeam,
  type GatewaySettingsPayload,
  type GatewaySettingsSource,
  type ConfiguredBot,
  type ConnectionStatus,
  type DesktopChatStreamEvent,
  type DesktopChannelEndpoint,
  type DesktopDeepLinkEvent,
  type DesktopSettings,
  type DesktopSessionProviderHint,
  type DesktopState,
  type DesktopTaskSummary,
  type DesktopThreadSummary,
  type DesktopWorkspace,
  type DesktopWorkspaceFileEntry,
  type DesktopWorkspaceFileListing,
  type DesktopWorkspaceFilePreview,
  type DesktopWorkflowDefinition,
  type DesktopWorkspaceMode,
  type MessageFileAttachment,
  type MessageImageAttachment,
  type PendingThreadInput,
  type SlashCommand,
  type ThreadRuntimeInfo,
  type ThreadTranscript,
  type TranscriptMessage,
  type UpdateMcpServerInput,
  type UpdateSlashCommandInput,
  type UpsertMcpServerInput,
  type UpsertSlashCommandInput,
} from "@shared/contracts";

import {
  buildIntent,
  findPendingAckIntentIndex,
  initialMessageMachineState,
  isRuntimeBusy,
  messageMachineReducer,
  selectGlobalActiveThreadId,
  selectQueueIntentIds,
  selectThreadRuntime,
  type MessageMachineAction,
  type MessageIntent,
  type ThreadRuntimeState,
} from "../message-machine";
import type { SettingsTabId } from "../settings-tabs";
import { GatewayProfileHistoryButton } from "../GatewayProfileHistoryButton";
import { SettingsErrorBoundary } from "../SettingsErrorBoundary";
import { Input } from "../components/ui/input";
import { WorkspacePathPickerDialog } from "../components/WorkspacePathPicker";
import { AddBotDialog } from "./components/AddBotDialog";
import { DreamsPanel } from "./components/DreamsPanel";
import { BotConversationSidebar } from "../BotConversationSidebar";
import { WorkspaceConversationSidebar } from "../WorkspaceConversationSidebar";
import { buildComposerWorkflowOptions } from "../ComposerForm";
import { ComposerQueue } from "../ComposerQueue";
import { ConversationHeaderActions } from "../ConversationHeaderActions";
import { ConversationHeaderTitle } from "../ConversationHeaderTitle";
import { NewThreadEmptyState } from "../NewThreadEmptyState";
import { ToastViewport, type ToastItem, type ToastTone } from "../toast";
import { ToolTraceGroup, shouldRenderToolTraceMessage } from "../tool-trace";
import {
  RichMessageContent,
  buildOptimisticTranscriptContent,
  countTranscriptFiles,
  countTranscriptImages,
  extractTranscriptText,
} from "../message-rich-content";
import {
  buildRenderableTranscript,
  buildRenderTranscriptBlocks,
  extractToolUseId,
  isToolRole,
  toolMessagesEquivalent,
} from "../transcript-render";
import {
  deriveThreadActivityModel,
  threadActivitySignature,
} from "./thread-activity";
import { measureUiAction } from "../perf-metrics";
import {
  activateBotDraftThread,
  openThreadFromEndpoint,
} from "../bot-console-controller";
import { getDesktopApi } from "../platform/desktop-api";
import {
  botGroupIdForEndpoint,
  buildBotGroups,
  channelDisplayName,
  latestEndpointActivity,
  primaryBotEndpoint,
} from "../bot-console-model";
import {
  deriveThreadTeamView,
  automationForLatestThread,
  buildWorkspaceThreadGroups,
  endpointThreadTitle,
  isSelectableNewThreadWorkspace,
  mergeThread,
  newThreadWorkspaceOptions,
  pickPreferredWorkspace,
  selectedAutomation,
  selectedThread,
  selectedWorkspace,
  teamBlocksEqual,
  visibleWorkspaceList,
  workspaceForThread,
  workspaceSuggestionFromPath,
} from "../thread-model";
import {
  bindEndpointToThread,
  deleteThread,
  detachEndpointFromThread,
  ensureWorkspaceForNewThread,
  ensureThread,
  loadThreadHistory,
  saveThreadTitle,
  scheduleThreadHistoryRefresh,
  selectWorkspaceForThread,
  startNewThreadDraft,
  updateThreadBotBinding,
} from "../thread-controller";
import {
  AutomationIcon,
  AutoResearchIcon,
  BackIcon,
  NewThreadIcon,
  SettingsIcon,
  SkillsIcon,
  WorkspaceFileIcon,
  isLocalSettingsTab,
} from "./icons";
import type {
  AutomationDraft,
  AutomationDialogState,
  BoundBot,
  ClientLogEntry,
  ContentView,
  GatewayIndicatorTone,
  LiveStreamState,
  LiveStreamStatus,
  MessageMap,
  PendingAutomationRun,
  PendingThreadInputMap,
  ThreadLogLine,
  ThreadLogTab,
  TranscriptEntryState,
  UiTranscriptMessage,
  WorkspaceDirectoryState,
} from "./types";
import { ThreadLogPanel } from "./components/ThreadLogPanel";
import { AppLeftRail } from "./components/AppLeftRail";
import { ThreadPage } from "./components/ThreadPage";
import { useAutomationController } from "./useAutomationController";
import { useAutoResearchController } from "./useAutoResearchController";
import {
  MAX_CLIENT_STREAM_LOG_ENTRIES,
  appendClientStreamLogEntry,
  THREAD_LOG_PANEL_MAX_WIDTH,
  THREAD_LOG_PANEL_MIN_WIDTH,
  buildClientStreamLogEntry,
  buildThreadLogLines,
  clampThreadLogsPanelWidth,
  computeGatewayIndicator,
  keepRecentThreadLogLines,
} from "./diagnostics-helpers";
import { useSettingsController } from "./useSettingsController";
import { useWorkspaceController } from "./useWorkspaceController";
import {
  compactPathLabel,
  expandWorkspaceDirectoryState,
  findWorkspaceFileEntry,
  parentDirectoryPath,
  resolveLocalFilePreviewTarget,
  workspaceDirectoryKey,
  workspaceFileAbsolutePath,
} from "./workspace-helpers";
import {
  isTransientGatewayErrorMessage,
  summarizeRemoteStateErrors,
} from "./gateway-errors";
import { buildAgentOptions, buildAgentTargetOptions } from "./agent-options";
import {
  I18nProvider,
  createTranslator,
  useResolvedLocale,
} from "../i18n";
import { isRunLoadingPlaceholderMessage } from "./loading-labels";
import {
  contentViewForDesktopRoute,
  parseDesktopRoute,
  replaceDesktopRoute,
  type DesktopRoute,
} from "./desktop-route";

const NEW_THREAD_DRAFT_THREAD_ID = "__garyx_new_thread_draft__";
const MESSAGES_BOTTOM_THRESHOLD_PX = 48;
const MESSAGES_TOP_PAGINATION_PREFETCH_MIN_PX = 640;
const MESSAGES_TOP_PAGINATION_PREFETCH_VIEWPORTS = 1.5;
const THREAD_HISTORY_PAGE_SIZE = 100;
const THREAD_HISTORY_USER_QUERY_LIMIT = 10;
const USER_TURN_PREFETCH_THRESHOLD = 3;
const HIDDEN_TOOL_USE_STATUS_TEXT = "Garyx is thinking through the next step…";
const HIDDEN_TOOL_RESULT_STATUS_TEXT = "Garyx finished a reasoning step…";

type ThreadEntrySelectionSource =
  | "pinned"
  | "bot-root"
  | "bot-conversation"
  | "workspace-conversation"
  | "dreams";

type ThreadHistoryPaginationState = {
  hasMoreBefore: boolean;
  nextBeforeIndex: number | null;
  loadingBefore: boolean;
};

const GatewaySettingsPanel = lazy(() =>
  import("../GatewaySettingsPanel").then((module) => ({
    default: module.GatewaySettingsPanel,
  })),
);
const BrowserPage = lazy(() =>
  import("../BrowserPage").then((module) => ({
    default: module.BrowserPage,
  })),
);
const BotConsolePage = lazy(() =>
  import("../BotConsolePage").then((module) => ({
    default: module.BotConsolePage,
  })),
);
const SkillsPanel = lazy(() =>
  import("../SkillsPanel").then((module) => ({
    default: module.SkillsPanel,
  })),
);
const AutomationDialog = lazy(() =>
  import("../components/AutomationDialog").then((module) => ({
    default: module.AutomationDialog,
  })),
);
const AutomationListPage = lazy(() =>
  import("../components/AutomationListPage").then((module) => ({
    default: module.AutomationListPage,
  })),
);
const MemoryDialog = lazy(() =>
  import("../components/MemoryDialog").then((module) => ({
    default: module.MemoryDialog,
  })),
);
const WorkspacePreviewModal = lazy(() =>
  import("./components/WorkspacePreviewModal").then((module) => ({
    default: module.WorkspacePreviewModal,
  })),
);
const AgentsHubPanel = lazy(() =>
  import("./components/AgentsHubPanel").then((module) => ({
    default: module.AgentsHubPanel,
  })),
);
const AutoResearchPanel = lazy(() =>
  import("./components/auto-research").then((module) => ({
    default: module.AutoResearchPanel,
  })),
);
const TasksPanel = lazy(() =>
  import("./components/TasksPanel").then((module) => ({
    default: module.TasksPanel,
  })),
);
const WorkflowRunsPanel = lazy(() =>
  import("./components/WorkflowRunsPanel").then((module) => ({
    default: module.WorkflowRunsPanel,
  })),
);
const EMPTY_UI_TRANSCRIPT_MESSAGES: UiTranscriptMessage[] = [];

function messagesNearBottom(node: HTMLDivElement | null): boolean {
  if (!node) {
    return true;
  }
  return (
    node.scrollHeight - node.scrollTop - node.clientHeight <
    MESSAGES_BOTTOM_THRESHOLD_PX
  );
}

function messagesNearEarlierUserTurnBoundary(
  node: HTMLDivElement | null,
): boolean {
  if (!node) {
    return false;
  }
  const pixelPrefetchDistance = Math.max(
    MESSAGES_TOP_PAGINATION_PREFETCH_MIN_PX,
    node.clientHeight * MESSAGES_TOP_PAGINATION_PREFETCH_VIEWPORTS,
  );
  if (node.scrollTop <= pixelPrefetchDistance) {
    return true;
  }
  const viewportTop = node.getBoundingClientRect().top;
  const userTurnStarts = node.querySelectorAll<HTMLElement>(
    "[data-user-turn-start='true']",
  );
  if (userTurnStarts.length === 0) {
    return false;
  }
  let userTurnsBeforeViewport = 0;
  for (const turnStart of userTurnStarts) {
    if (turnStart.getBoundingClientRect().bottom <= viewportTop) {
      userTurnsBeforeViewport += 1;
      continue;
    }
    break;
  }
  return userTurnsBeforeViewport <= USER_TURN_PREFETCH_THRESHOLD;
}

type MemoryDialogTarget =
  | {
      scope: "agent";
      agentId: string;
      title: string;
    }
  | {
      scope: "automation";
      automationId: string;
      title: string;
    };

function memoryKey(value: string, fallback: string): string {
  const trimmed = value.trim();
  const base = trimmed || fallback;
  let sanitized = "";
  for (const ch of base) {
    if (/^[a-z0-9]$/i.test(ch)) {
      sanitized += ch.toLowerCase();
    } else if (!sanitized.endsWith("-")) {
      sanitized += "-";
    }
  }
  const normalized = sanitized.replace(/^-+|-+$/g, "");
  return normalized || fallback;
}

function normalizeLocalPathForMatch(value: string): string {
  return value.replace(/\\/g, "/").replace(/\/+/g, "/");
}

function resolveMemoryDialogTargetFromPath(
  absolutePath: string,
  automations: DesktopAutomationSummary[],
  agents: DesktopCustomAgent[],
): MemoryDialogTarget | null {
  const normalizedPath = normalizeLocalPathForMatch(absolutePath);

  const matchedAgent = agents.find((agent) => {
    return normalizedPath.endsWith(
      `/.garyx/agents/${memoryKey(agent.agentId, "agent")}/memory.md`,
    );
  });
  if (matchedAgent) {
    return {
      scope: "agent",
      agentId: matchedAgent.agentId,
      title: `${matchedAgent.displayName || matchedAgent.agentId} memory.md`,
    };
  }

  const matchedAutomation = automations.find((automation) => {
    return normalizedPath.endsWith(
      `/.garyx/automations/${memoryKey(automation.id, "automation")}/memory.md`,
    );
  });
  if (matchedAutomation) {
    return {
      scope: "automation",
      automationId: matchedAutomation.id,
      title: `${matchedAutomation.label} memory.md`,
    };
  }

  return null;
}

function scrollMessagesToLatest(
  node: HTMLDivElement | null,
  behavior: ScrollBehavior = "auto",
) {
  node?.scrollTo({
    top: node.scrollHeight,
    behavior,
  });
}

function messageTailSignature(messages: UiTranscriptMessage[]): string {
  const lastMessage = messages[messages.length - 1];
  if (!lastMessage) {
    return "0";
  }
  return [
    messages.length,
    lastMessage.id,
    lastMessage.role,
    lastMessage.text.length,
    lastMessage.pending ? "1" : "0",
    lastMessage.localState || "",
  ].join(":");
}

function transcriptEntryHistoryIndex(
  message: Pick<UiTranscriptMessage, "id" | "localState">,
): number | null {
  if (message.localState !== "remote_final") {
    return null;
  }
  const suffix = message.id.split(":").pop();
  if (!suffix || !/^\d+$/.test(suffix)) {
    return null;
  }
  return Number(suffix);
}

function earliestRemoteHistoryIndex(messages: UiTranscriptMessage[]): number | null {
  let earliest: number | null = null;
  for (const message of messages) {
    const historyIndex = transcriptEntryHistoryIndex(message);
    if (historyIndex === null) {
      continue;
    }
    if (earliest === null || historyIndex < earliest) {
      earliest = historyIndex;
    }
  }
  return earliest;
}

function transcriptHasAutomationResponse(
  messages: TranscriptMessage[],
): boolean {
  return messages.some(
    (message) => message.role === "assistant" || isToolRole(message.role),
  );
}

function formatThreadTimestamp(value?: string | null): string {
  if (!value) {
    return "";
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return "";
  }

  const now = Date.now();
  const diffMs = now - date.getTime();
  const diffMin = Math.floor(diffMs / 60_000);
  const diffHr = Math.floor(diffMs / 3_600_000);
  const diffDay = Math.floor(diffMs / 86_400_000);
  const diffMon = Math.floor(diffDay / 30);

  if (diffMin < 1) return "now";
  if (diffMin < 60) return `${diffMin}m`;
  if (diffHr < 24) return `${diffHr}h`;
  if (diffDay < 30) return `${diffDay}d`;
  if (diffMon < 12) return `${diffMon}mo`;
  return `${Math.floor(diffDay / 365)}y`;
}

function botLabel(channel: string, accountId: string): string {
  return accountId?.trim() || channelDisplayName(channel);
}

function boundBotsForThread(endpoints: DesktopChannelEndpoint[]): BoundBot[] {
  const bindings = new Map<string, BoundBot>();

  for (const endpoint of endpoints) {
    const channel = endpoint.channel || "unknown";
    const accountId = endpoint.accountId || "default";
    const id = `${channel}::${accountId}`;
    const existing = bindings.get(id) || {
      id,
      channel,
      accountId,
      label: botLabel(channel, accountId),
      endpointCount: 0,
    };
    existing.endpointCount += 1;
    bindings.set(id, existing);
  }

  return [...bindings.values()].sort((left, right) => {
    return (
      left.label.localeCompare(right.label) ||
      left.channel.localeCompare(right.channel)
    );
  });
}

function normalizeMessageText(value: string | undefined): string {
  return value?.trim() || "";
}

function normalizeGatewayUrlForMatch(value: string): string {
  return value.trim().replace(/\/+$/, "").toLowerCase();
}

function isConnectionValidForSettings(
  status: ConnectionStatus | null,
  settings: DesktopSettings | null | undefined,
): boolean {
  const savedGatewayUrl = normalizeGatewayUrlForMatch(settings?.gatewayUrl || "");
  if (!savedGatewayUrl || !status?.ok) {
    return false;
  }
  return normalizeGatewayUrlForMatch(status.gatewayUrl) === savedGatewayUrl;
}

function transcriptMessageImageCount(message: TranscriptMessage): number {
  return countTranscriptImages(message.content);
}

function transcriptMessageFileCount(message: TranscriptMessage): number {
  return countTranscriptFiles(message.content);
}

function transcriptMessageComparableText(message: TranscriptMessage): string {
  const structuredText = normalizeMessageText(
    extractTranscriptText(message.content),
  );
  if (structuredText) {
    return structuredText;
  }
  if (
    transcriptMessageImageCount(message) > 0 ||
    transcriptMessageFileCount(message) > 0
  ) {
    return "";
  }
  return normalizeMessageText(message.text);
}

function uiTranscriptMessageComparableText(
  message: UiTranscriptMessage,
): string {
  const structuredText = normalizeMessageText(
    extractTranscriptText(message.content),
  );
  if (structuredText) {
    return structuredText;
  }
  if (
    transcriptMessageImageCount(message) > 0 ||
    transcriptMessageFileCount(message) > 0
  ) {
    return "";
  }
  return normalizeMessageText(message.text);
}

function pendingThreadInputImageCount(input: PendingThreadInput): number {
  return countTranscriptImages(input.content);
}

function pendingThreadInputFileCount(input: PendingThreadInput): number {
  return countTranscriptFiles(input.content);
}

function pendingThreadInputComparableText(input: PendingThreadInput): string {
  const structuredText = normalizeMessageText(
    extractTranscriptText(input.content),
  );
  if (structuredText) {
    return structuredText;
  }
  if (
    pendingThreadInputImageCount(input) > 0 ||
    pendingThreadInputFileCount(input) > 0
  ) {
    return "";
  }
  return normalizeMessageText(input.text);
}

function pendingThreadInputMatchesMessage(
  input: PendingThreadInput,
  message: TranscriptMessage,
): boolean {
  if (message.role !== "user") {
    return false;
  }

  const queuedInputId = queuedInputIdFromMessage(message);
  if (queuedInputId) {
    return queuedInputId === input.id;
  }

  return (
    pendingThreadInputComparableText(input) ===
      transcriptMessageComparableText(message) &&
    pendingThreadInputImageCount(input) === transcriptMessageImageCount(message) &&
    pendingThreadInputFileCount(input) === transcriptMessageFileCount(message)
  );
}

function isRecoverableAssistantEntry(
  entry: UiTranscriptMessage,
  intentId: string,
  candidateEntryIds: Set<string>,
): boolean {
  if (entry.role !== "assistant" || entry.intentId !== intentId) {
    return false;
  }
  return (
    entry.pending ||
    entry.localState === "optimistic" ||
    entry.localState === "remote_partial" ||
    candidateEntryIds.has(entry.id)
  );
}

function reconcileAssistantEntriesForGatewayRecovery(
  entries: UiTranscriptMessage[],
  intentId: string,
  candidateEntryIds: Iterable<string | null | undefined>,
): { entries: UiTranscriptMessage[]; matched: boolean } {
  const normalizedCandidateEntryIds = new Set(
    [...candidateEntryIds]
      .map((value) => value?.trim() || "")
      .filter((value) => value.length > 0),
  );
  let matched = false;
  const nextEntries: UiTranscriptMessage[] = [];

  for (const entry of entries) {
    if (
      !isRecoverableAssistantEntry(entry, intentId, normalizedCandidateEntryIds)
    ) {
      nextEntries.push(entry);
      continue;
    }

    matched = true;
    const visibleText = uiTranscriptMessageComparableText(entry);
    if (!visibleText) {
      continue;
    }

    nextEntries.push({
      ...entry,
      pending: false,
      error: false,
      localState:
        entry.localState === "optimistic" ? "remote_partial" : entry.localState,
    });
  }

  return {
    entries: nextEntries,
    matched,
  };
}

function transcriptMessageMatchesIntent(
  message: TranscriptMessage,
  intent: MessageIntent,
): boolean {
  return (
    message.role === "user" &&
    transcriptMessageComparableText(message) ===
      normalizeMessageText(intent.text) &&
    transcriptMessageImageCount(message) === intent.images.length &&
    transcriptMessageFileCount(message) === intent.files.length
  );
}

function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => {
      reject(new Error(`Failed to read ${file.name}`));
    };
    reader.onload = () => {
      const result = typeof reader.result === "string" ? reader.result : "";
      const commaIndex = result.indexOf(",");
      if (commaIndex < 0) {
        reject(new Error(`Failed to decode ${file.name}`));
        return;
      }
      resolve(result.slice(commaIndex + 1));
    };
    reader.readAsDataURL(file);
  });
}

function isImageFile(file: File): boolean {
  if (/^image\/(png|jpe?g|gif|webp)$/i.test(file.type || "")) {
    return true;
  }
  return /\.(png|jpe?g|gif|webp)$/i.test(file.name || "");
}

function inferImageMediaType(file: File): string {
  if (/^image\/(png|jpe?g|gif|webp)$/i.test(file.type || "")) {
    return file.type;
  }
  const lowerName = (file.name || "").toLowerCase();
  if (lowerName.endsWith(".png")) {
    return "image/png";
  }
  if (lowerName.endsWith(".gif")) {
    return "image/gif";
  }
  if (lowerName.endsWith(".webp")) {
    return "image/webp";
  }
  return "image/jpeg";
}

function inferFileMediaType(file: File): string {
  return (file.type || "").trim();
}

type PreparedLocalAttachmentUpload = {
  id: string;
  kind: "image" | "file";
  name: string;
  mediaType: string;
  dataBase64: string;
};

async function prepareAttachmentUploads(
  files: File[],
): Promise<PreparedLocalAttachmentUpload[]> {
  const attachments = await Promise.all(
    files.map(async (file) => {
      const kind: PreparedLocalAttachmentUpload["kind"] = isImageFile(file)
        ? "image"
        : "file";
      return {
        id: `${kind}:${crypto.randomUUID()}`,
        kind,
        name: file.name || kind,
        mediaType:
          kind === "image" ? inferImageMediaType(file) : inferFileMediaType(file),
        dataBase64: await fileToBase64(file),
      };
    }),
  );
  return attachments.filter((attachment) => attachment.dataBase64.trim() !== "");
}

function toRemoteTranscript(
  messages: TranscriptMessage[],
): UiTranscriptMessage[] {
  return messages.map((message) => ({
    ...message,
    localState: "remote_final",
  }));
}

function isLoopContinuationMessage(message: TranscriptMessage): boolean {
  return (
    Boolean(message.internal) && message.internalKind === "loop_continuation"
  );
}

function displayTranscriptMessageText(message: UiTranscriptMessage): string {
  if (isLoopContinuationMessage(message) && message.role === "system") {
    return (
      normalizeMessageText(message.text) ||
      "System triggered an automatic continuation."
    );
  }
  return message.text;
}

function seededUserBubble(intent: MessageIntent): UiTranscriptMessage {
  return {
    id: `user:${intent.intentId}`,
    role: "user",
    text: intent.text,
    content: buildOptimisticTranscriptContent(
      intent.text,
      intent.images,
      intent.files,
    ),
    timestamp: new Date().toISOString(),
    intentId: intent.intentId,
    localState: "optimistic",
  };
}

function seededAckedUserBubble(intent: MessageIntent): UiTranscriptMessage {
  return {
    ...seededUserBubble(intent),
    localState: "remote_partial",
  };
}

type SeededTurn = {
  assistantEntryId: string | null;
  legacyPendingAssistantId: string | null;
};

type PendingAssistantDelta = {
  threadId: string;
  intentId?: string;
  runId: string;
  text: string;
  metadata?: Record<string, unknown> | null;
};

function queuedInputIdFromMessage(
  message: Pick<TranscriptMessage, "metadata">,
): string {
  const metadata = message.metadata;
  if (!metadata || typeof metadata !== "object") {
    return "";
  }
  const snakeCase = metadata.queued_input_id;
  if (typeof snakeCase === "string" && snakeCase.trim()) {
    return snakeCase;
  }
  const camelCase = metadata.queuedInputId;
  return typeof camelCase === "string" && camelCase.trim() ? camelCase : "";
}

function buildStreamingToolBubble(
  event: Extract<DesktopChatStreamEvent, { type: "tool_use" | "tool_result" }>,
  intentId?: string,
): UiTranscriptMessage {
  return {
    id: `${event.type}:${event.message.toolUseId || crypto.randomUUID()}:${crypto.randomUUID()}`,
    role: event.type,
    text: "",
    content: event.message.content,
    toolUseId: event.message.toolUseId ?? null,
    toolName: event.message.toolName ?? null,
    isError: event.message.isError,
    metadata: event.message.metadata ?? null,
    timestamp: event.message.timestamp || new Date().toISOString(),
    intentId,
    remoteRunId: event.runId,
    kind: "tool_trace",
    localState: "remote_partial",
  };
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  return value as Record<string, unknown>;
}

function speakerIdentityKey(
  metadata: Record<string, unknown> | null | undefined,
): string {
  const record = asRecord(metadata);
  if (!record) {
    return "";
  }
  const agentId =
    typeof record.agent_id === "string" ? record.agent_id.trim() : "";
  const displayName =
    typeof record.agent_display_name === "string"
      ? record.agent_display_name.trim()
      : "";
  return `${agentId}::${displayName}`;
}

function isMessageToolName(value: unknown): boolean {
  if (typeof value !== "string") {
    return false;
  }
  const trimmed = value.trim();
  if (!trimmed) {
    return false;
  }
  const tail = trimmed.split(":").pop() || trimmed;
  return tail.toLowerCase() === "message";
}

function valueMarksMessageTool(value: unknown): boolean {
  if (Array.isArray(value)) {
    return value.some((entry) => valueMarksMessageTool(entry));
  }
  const record = asRecord(value);
  if (!record) {
    return false;
  }
  if (
    isMessageToolName(record.tool) ||
    isMessageToolName(record.tool_name) ||
    isMessageToolName(record.toolName) ||
    isMessageToolName(record.name)
  ) {
    return true;
  }
  return Object.values(record).some((entry) => valueMarksMessageTool(entry));
}

function extractMessageToolImageBlocks(
  value: unknown,
  blocks: Array<Record<string, unknown>>,
) {
  if (Array.isArray(value)) {
    for (const entry of value) {
      extractMessageToolImageBlocks(entry, blocks);
    }
    return;
  }
  const record = asRecord(value);
  if (!record) {
    return;
  }
  const type =
    typeof record.type === "string" ? record.type.trim().toLowerCase() : "";
  if (type === "image") {
    const source = asRecord(record.source);
    const hasData =
      typeof source?.data === "string" && source.data.trim().length > 0;
    const hasUrl =
      typeof record.url === "string" && record.url.trim().length > 0;
    if (hasData || hasUrl) {
      if (hasUrl) {
        blocks.push({
          type: "image",
          url: record.url,
        });
      } else if (source) {
        blocks.push({
          type: "image",
          source: {
            type: source.type || "base64",
            media_type:
              (typeof source.media_type === "string" &&
                source.media_type.trim()) ||
              (typeof source.mediaType === "string" &&
                source.mediaType.trim()) ||
              "image/png",
            data: source.data,
          },
        });
      }
    }
  }
  for (const entry of Object.values(record)) {
    extractMessageToolImageBlocks(entry, blocks);
  }
}

type GeneratedImageToolMessage = Pick<
  TranscriptMessage,
  "content" | "metadata" | "toolName" | "toolUseId"
>;

const GENERATED_IMAGE_TOOL_USE_METADATA_KEY = "generated_image_tool_use_id";

function recordString(
  record: Record<string, unknown> | null | undefined,
  ...keys: string[]
): string {
  for (const key of keys) {
    const value = record?.[key];
    if (typeof value === "string" && value.trim()) {
      return value.trim();
    }
  }
  return "";
}

function codexItemTypeFromToolMessage(
  message: GeneratedImageToolMessage,
): string {
  const content = asRecord(message.content);
  const metadata = asRecord(message.metadata);
  return (
    recordString(metadata, "item_type", "itemType") ||
    recordString(content, "type") ||
    message.toolName?.trim() ||
    ""
  );
}

function isImageGenerationToolMessage(
  message: GeneratedImageToolMessage,
): boolean {
  return codexItemTypeFromToolMessage(message).toLowerCase() === "imagegeneration";
}

function mediaTypeFromGeneratedImageResult(
  result: string,
  content: Record<string, unknown> | null,
): string {
  const explicit =
    recordString(content, "media_type", "mediaType", "mime_type", "mimeType", "contentType") ||
    "";
  if (explicit) {
    return explicit;
  }
  const match = result.match(/^data:([^;,]+)(?:;[^,]*)?,/i);
  return match?.[1]?.trim() || "image/png";
}

function base64FromGeneratedImageResult(result: string): string {
  const match = result.match(/^data:[^,]*,(.*)$/is);
  return (match?.[1] || result).trim();
}

function generatedImageName(message: GeneratedImageToolMessage): string {
  const content = asRecord(message.content);
  const id =
    recordString(content, "id") ||
    message.toolUseId?.trim() ||
    "generated-image";
  return `${id}.png`;
}

function extractImageGenerationImageContent(
  message: GeneratedImageToolMessage,
): unknown[] | null {
  if (!isImageGenerationToolMessage(message)) {
    return null;
  }
  const content = asRecord(message.content);
  const result = recordString(content, "result");
  if (!result) {
    return null;
  }
  const data = base64FromGeneratedImageResult(result);
  if (!data) {
    return null;
  }
  return [
    {
      type: "image",
      name: generatedImageName(message),
      source: {
        type: "base64",
        media_type: mediaTypeFromGeneratedImageResult(result, content),
        data,
      },
    },
  ];
}

function extractStreamingMessageToolImageContent(
  event: Extract<DesktopChatStreamEvent, { type: "tool_result" }>,
): unknown[] | null {
  if (
    !isMessageToolName(event.message.toolName) &&
    !valueMarksMessageTool(event.message.content)
  ) {
    return null;
  }
  const blocks: Array<Record<string, unknown>> = [];
  extractMessageToolImageBlocks(event.message.content, blocks);
  if (!blocks.length) {
    return null;
  }
  return blocks;
}

function transcriptMessagesSemanticallyMatch(
  local: UiTranscriptMessage,
  remote: TranscriptMessage,
): boolean {
  if (local.role !== remote.role) {
    return false;
  }

  const localQueuedInputId = queuedInputIdFromMessage(local);
  const remoteQueuedInputId = queuedInputIdFromMessage(remote);
  if (localQueuedInputId && remoteQueuedInputId) {
    return localQueuedInputId === remoteQueuedInputId;
  }

  if (isToolRole(local.role) && isToolRole(remote.role)) {
    return toolMessagesEquivalent(local, remote);
  }

  if (local.role === "user") {
    return (
      transcriptMessageComparableText(local) ===
        transcriptMessageComparableText(remote) &&
      transcriptMessageImageCount(local) === transcriptMessageImageCount(remote) &&
      transcriptMessageFileCount(local) === transcriptMessageFileCount(remote)
    );
  }

  if (local.role === "assistant") {
    const localText = transcriptMessageComparableText(local);
    const remoteText = transcriptMessageComparableText(remote);
    if (!localText || !remoteText) {
      return localText === remoteText;
    }
    return (
      localText === remoteText ||
      remoteText.startsWith(localText) ||
      localText.startsWith(remoteText)
    );
  }

  if (local.role === "system") {
    return (
      local.internalKind === remote.internalKind &&
      transcriptMessageComparableText(local) ===
        transcriptMessageComparableText(remote)
    );
  }

  return false;
}

function jsonValuesEqual(left: unknown, right: unknown): boolean {
  return JSON.stringify(left ?? null) === JSON.stringify(right ?? null);
}

function remoteTranscriptMessageCanReuseExisting(
  existing: UiTranscriptMessage,
  remote: TranscriptMessage,
  options?: { ignoreTimestamp?: boolean },
): boolean {
  return (
    existing.localState === "remote_final" &&
    existing.role === remote.role &&
    existing.text === remote.text &&
    jsonValuesEqual(existing.content, remote.content) &&
    (options?.ignoreTimestamp || existing.timestamp === remote.timestamp) &&
    existing.toolUseId === remote.toolUseId &&
    existing.toolName === remote.toolName &&
    existing.isError === remote.isError &&
    jsonValuesEqual(existing.metadata, remote.metadata) &&
    existing.kind === remote.kind &&
    existing.internal === remote.internal &&
    existing.internalKind === remote.internalKind &&
    existing.loopOrigin === remote.loopOrigin &&
    existing.pending !== true &&
    existing.error === remote.error
  );
}

function materializeRemoteTranscript(
  transcript: TranscriptMessage[],
  existing: UiTranscriptMessage[],
  options?: { ignoreTimestampForStableMessages?: boolean },
): UiTranscriptMessage[] {
  const usedExistingIndexes = new Set<number>();

  const materializeMessage = (
    message: TranscriptMessage,
    matchSemantic: boolean,
  ): UiTranscriptMessage => {
    let matchedIndex = existing.findIndex((entry, index) => {
      return !usedExistingIndexes.has(index) && entry.id === message.id;
    });

    if (matchedIndex < 0 && matchSemantic) {
      matchedIndex = existing.findIndex((entry, index) => {
        return (
          !usedExistingIndexes.has(index) &&
          transcriptMessagesSemanticallyMatch(entry, message)
        );
      });
    }

    const matchedEntry = matchedIndex >= 0 ? existing[matchedIndex] : null;
    if (matchedIndex >= 0) {
      usedExistingIndexes.add(matchedIndex);
    }

    if (
      matchedEntry &&
      remoteTranscriptMessageCanReuseExisting(matchedEntry, message, {
        ignoreTimestamp: options?.ignoreTimestampForStableMessages,
      })
    ) {
      return matchedEntry;
    }

    return {
      ...message,
      id: matchedEntry?.id || message.id,
      intentId: matchedEntry?.intentId,
      remoteRunId: matchedEntry?.remoteRunId,
      localState: "remote_final" as const,
      pending: false,
      error: message.error,
    };
  };

  const materializeGeneratedImageMessage = (
    sourceMessage: TranscriptMessage,
    content: unknown[],
  ): UiTranscriptMessage => {
    const toolUseId = sourceMessage.toolUseId?.trim() || "";
    const synthetic: TranscriptMessage = {
      id: `generated-image:${sourceMessage.id}`,
      role: "assistant",
      text: "",
      content,
      timestamp: sourceMessage.timestamp,
      metadata: {
        source: "codex_app_server",
        item_type: "imageGeneration",
        [GENERATED_IMAGE_TOOL_USE_METADATA_KEY]: toolUseId,
      },
      kind: "assistant_reply",
    };
    let matchedIndex = existing.findIndex((entry, index) => {
      return !usedExistingIndexes.has(index) && entry.id === synthetic.id;
    });
    if (matchedIndex < 0 && toolUseId) {
      matchedIndex = existing.findIndex((entry, index) => {
        const metadata = asRecord(entry.metadata);
        return (
          !usedExistingIndexes.has(index) &&
          entry.role === "assistant" &&
          metadata?.[GENERATED_IMAGE_TOOL_USE_METADATA_KEY] === toolUseId
        );
      });
    }
    if (matchedIndex < 0) {
      const contentSignature = JSON.stringify(content);
      matchedIndex = existing.findIndex((entry, index) => {
        return (
          !usedExistingIndexes.has(index) &&
          entry.role === "assistant" &&
          !entry.text.trim() &&
          JSON.stringify(entry.content) === contentSignature
        );
      });
    }

    const matchedEntry = matchedIndex >= 0 ? existing[matchedIndex] : null;
    if (matchedIndex >= 0) {
      usedExistingIndexes.add(matchedIndex);
    }

    if (
      matchedEntry &&
      remoteTranscriptMessageCanReuseExisting(matchedEntry, synthetic, {
        ignoreTimestamp: options?.ignoreTimestampForStableMessages,
      })
    ) {
      return matchedEntry;
    }

    return {
      ...synthetic,
      id: matchedEntry?.id || synthetic.id,
      intentId: matchedEntry?.intentId,
      remoteRunId: matchedEntry?.remoteRunId,
      localState: "remote_final" as const,
      pending: false,
      error: false,
    };
  };

  const materializedRemote: UiTranscriptMessage[] = [];
  for (const message of transcript) {
    if (isRunLoadingPlaceholderMessage(message)) {
      continue;
    }
    materializedRemote.push(materializeMessage(message, true));
    if (message.role === "tool_result") {
      const imageContent = extractImageGenerationImageContent(message);
      if (imageContent) {
        materializedRemote.push(
          materializeGeneratedImageMessage(message, imageContent),
        );
      }
    }
  }
  return materializedRemote;
}

function resolveIntentHistoryMatch(
  intent: MessageIntent,
  messages: TranscriptMessage[],
) {
  const userIndex =
    [...messages]
      .map((message, index) => ({ message, index }))
      .reverse()
      .find(({ message }) => {
        return transcriptMessageMatchesIntent(message, intent);
      })?.index ?? -1;

  if (userIndex < 0) {
    return {
      userVisible: false,
      assistantVisible: false,
    };
  }

  const followUpMessages = messages.slice(userIndex + 1);
  const assistantMessages = followUpMessages.filter(
    (message) => message.role === "assistant",
  );
  const expectedResponse = normalizeMessageText(intent.responseText);
  const assistantVisible = expectedResponse
    ? assistantMessages.some(
        (message) => normalizeMessageText(message.text) === expectedResponse,
      )
    : assistantMessages.length > 0 ||
      followUpMessages.some((message) => isToolRole(message.role));

  return {
    userVisible: true,
    assistantVisible,
  };
}

function isKnownThreadId(
  state: DesktopState | null,
  threadId: string | null,
): boolean {
  if (!state || !threadId) {
    return false;
  }
  return (
    state.threads.some((thread) => thread.id === threadId) ||
    state.sessions.some((thread) => thread.id === threadId) ||
    state.automations.some((automation) => automation.threadId === threadId)
  );
}

const STARTUP_HYDRATION_RETRY_DELAYS_MS = [0, 300, 650, 1_100, 1_700];
const DEEP_LINK_GATEWAY_RETRY_DELAYS_MS = [0, 300, 650, 1_100, 1_700, 2_500];
const TRANSIENT_STATUS_MS = 3200;
const ERROR_TOAST_MS = 4400;
const GATEWAY_HEALTHY_POLL_MS = 12000;
const SILENT_DESKTOP_STATE_REFRESH_MS = 15000;
const GATEWAY_RETRY_BACKOFF_MS = [2500, 4000, 6500, 10000, 15000];

function savedContentView(): ContentView {
  const saved = sessionStorage.getItem("gary-content-view");
  const valid: ContentView[] = [
    "thread",
    "browser",
    "bots",
    "automation",
    "auto_research",
    "agents",
    "teams",
    "skills",
    "tasks",
    "workflow",
    "dreams",
    "settings",
  ];
  return saved && valid.includes(saved as ContentView)
    ? (saved as ContentView)
    : "thread";
}

function initialContentView(route: DesktopRoute): ContentView {
  return contentViewForDesktopRoute(route) || savedContentView();
}

function currentDesktopRoute(input: {
  contentView: ContentView;
  newThreadDraftActive: boolean;
  pendingAgentId: string | null;
  pendingWorkflowId: string | null;
  pendingWorkspacePath: string | null;
  selectedAutomationId: string | null;
  selectedWorkflowTaskId: string | null;
  selectedThreadId: string | null;
  settingsActiveTab: SettingsTabId;
}): DesktopRoute {
  if (input.contentView === "thread") {
    if (input.selectedThreadId) {
      return { kind: "thread", threadId: input.selectedThreadId };
    }
    if (input.newThreadDraftActive || input.pendingWorkspacePath) {
      return {
        kind: "new-thread",
        workspacePath: input.pendingWorkspacePath,
        agentId: input.pendingAgentId,
        workflowId: input.pendingWorkflowId,
      };
    }
    return { kind: "thread-home" };
  }
  if (input.contentView === "automation") {
    return { kind: "automation", automationId: input.selectedAutomationId };
  }
  if (input.contentView === "settings") {
    return { kind: "settings", tabId: input.settingsActiveTab };
  }
  if (input.contentView === "workflow") {
    return input.selectedWorkflowTaskId
      ? { kind: "workflow-task", taskId: input.selectedWorkflowTaskId }
      : { kind: "view", view: "tasks" };
  }
  return { kind: "view", view: input.contentView };
}

function waitForMs(ms: number): Promise<void> {
  return new Promise((resolve) => {
    window.setTimeout(resolve, ms);
  });
}

function hasRemoteDesktopContent(state: DesktopState | null): boolean {
  if (!state) {
    return false;
  }
  return Boolean(
    state.threads.length ||
    state.endpoints.length ||
    state.configuredBots.length ||
    state.automations.length,
  );
}

function shouldRetryStartupHydration(
  state: DesktopState | null,
  status: ConnectionStatus | null,
): boolean {
  if (!state) {
    return true;
  }
  if (hasRemoteDesktopContent(state)) {
    return false;
  }
  if (!state.workspaces.length) {
    return false;
  }
  if (!status?.ok) {
    return true;
  }
  return (
    (status.threadCount || status.sessionCount || 0) > 0 ||
    state.workspaces.some((workspace) => workspace.available)
  );
}

function gatewaySetupMessageForAuthError(
  message: string | null | undefined,
): string | null {
  const normalized = message?.trim().toLowerCase() || "";
  if (!normalized) {
    return null;
  }
  const mentionsGatewayToken =
    normalized.includes("gateway authorization token") ||
    normalized.includes("gateway token") ||
    normalized.includes("garyx gateway token");
  const unauthorized =
    normalized === "unauthorized" ||
    normalized.includes("401") ||
    normalized.includes("valid gateway authorization token required");
  if (!mentionsGatewayToken && !unauthorized) {
    return null;
  }
  if (normalized.includes("not configured")) {
    return "Gateway token is not configured on the gateway host. Run `garyx gateway token` there, paste the token here, then save and continue.";
  }
  return "Gateway token is missing or invalid. Run `garyx gateway token` on the gateway host, paste the token here, then save and continue.";
}

function presentProviderReadyError(
  message: string,
  providerType?: DesktopApiProviderType | null,
): string {
  const normalized = message.trim().toLowerCase();
  if (!normalized.includes("provider not ready")) {
    return message;
  }
  if (providerType === "codex_app_server") {
    return "Codex is not ready on this Mac. Check that the codex CLI is installed, logged in, and available on the Garyx gateway PATH.";
  }
  if (providerType === "gemini_cli") {
    return "Gemini CLI is not ready on this Mac. Check that the gemini CLI is installed and available on the Garyx gateway PATH.";
  }
  if (providerType === "gpt") {
    return "GPT provider is not ready on this Mac. Check the gateway status and Codex/OpenAI auth configuration.";
  }
  if (providerType === "anthropic" || providerType === "claude_llm") {
    return "Claude model provider is not ready on this Mac. Check the gateway status and Anthropic auth configuration.";
  }
  if (providerType === "google" || providerType === "gemini_llm") {
    return "Gemini model provider is not ready on this Mac. Check the gateway status and Gemini auth configuration.";
  }
  if (providerType === "claude_code") {
    return "Claude Code is not ready on this Mac. Check the local Claude CLI auth and environment settings.";
  }
  return "The selected provider is not ready on this Mac. Open Status and verify the provider shows Ready.";
}

function inferProviderTypeForThread(
  threadId: string,
  threadInfoByThread: Record<string, ThreadRuntimeInfo | null>,
  desktopState: DesktopState | null,
  desktopAgents: DesktopCustomAgent[],
): DesktopApiProviderType | null {
  const runtimeProvider = threadInfoByThread[threadId]?.providerType;
  if (
    runtimeProvider === "claude_code" ||
    runtimeProvider === "codex_app_server" ||
    runtimeProvider === "gemini_cli" ||
    runtimeProvider === "gpt" ||
    runtimeProvider === "anthropic" ||
    runtimeProvider === "google" ||
    runtimeProvider === "claude_llm" ||
    runtimeProvider === "gemini_llm"
  ) {
    if (runtimeProvider === "claude_llm") {
      return "anthropic";
    }
    if (runtimeProvider === "gemini_llm") {
      return "google";
    }
    return runtimeProvider;
  }

  const agentId =
    desktopState?.threads.find((entry) => entry.id === threadId)?.agentId || "";
  if (!agentId) {
    return null;
  }
  if (agentId === "codex") {
    return "codex_app_server";
  }
  if (agentId === "gemini") {
    return "gemini_cli";
  }
  if (agentId === "claude") {
    return "claude_code";
  }
  return (
    desktopAgents.find((agent) => agent.agentId === agentId)?.providerType ||
    null
  );
}

export function AppShell() {
  const initialRouteRef = useRef<DesktopRoute | null>(null);
  if (!initialRouteRef.current) {
    initialRouteRef.current = parseDesktopRoute();
  }
  const initialRouteValue = initialRouteRef.current;
  const [desktopState, setDesktopState] = useState<DesktopState | null>(null);
  const [desktopAgents, setDesktopAgents] = useState<DesktopCustomAgent[]>([]);
  const [desktopTeams, setDesktopTeams] = useState<DesktopTeam[]>([]);
  const [desktopWorkflows, setDesktopWorkflows] = useState<
    DesktopWorkflowDefinition[]
  >([]);
  const [workflowDefinitionsLoading, setWorkflowDefinitionsLoading] =
    useState(false);
  const [connection, setConnection] = useState<ConnectionStatus | null>(null);
  const [gatewayStatusHint, setGatewayStatusHint] = useState<string | null>(
    "Connecting to gateway…",
  );
  const [gatewayFailureCount, setGatewayFailureCount] = useState(0);
  const [gatewaySetupForced, setGatewaySetupForced] = useState(false);
  const [gatewaySetupCanCancel, setGatewaySetupCanCancel] = useState(false);
  const [selectedThreadId, setSelectedThreadId] = useState<string | null>(() =>
    initialRouteValue.kind === "thread" ? initialRouteValue.threadId : null,
  );
  const [selectedWorkflowTask, setSelectedWorkflowTask] =
    useState<DesktopTaskSummary | null>(null);
  const [selectedWorkflowTaskId, setSelectedWorkflowTaskId] = useState<
    string | null
  >(() =>
    initialRouteValue.kind === "workflow-task" ? initialRouteValue.taskId : null,
  );
  const [threadEntrySelectionSource, setThreadEntrySelectionSource] =
    useState<ThreadEntrySelectionSource | null>(null);
  const [newThreadDraftActive, setNewThreadDraftActive] = useState(
    initialRouteValue.kind === "new-thread",
  );
  const [pendingWorkspacePath, setPendingWorkspacePath] = useState<string | null>(
    initialRouteValue.kind === "new-thread"
      ? initialRouteValue.workspacePath || null
      : null,
  );
  const [pendingWorkspaceMode, setPendingWorkspaceMode] =
    useState<DesktopWorkspaceMode>("local");
  const [pendingBotId, setPendingBotId] = useState<string | null>(null);
  const [optimisticThreadBotBinding, setOptimisticThreadBotBinding] = useState<{
    botId: string | null;
    threadId: string;
  } | null>(null);
  const [pendingAgentId, setPendingAgentId] = useState<string>(
    initialRouteValue.kind === "new-thread" && initialRouteValue.agentId
      ? initialRouteValue.agentId
      : "claude",
  );
  const [pendingWorkflowId, setPendingWorkflowId] = useState<string | null>(
    initialRouteValue.kind === "new-thread" && initialRouteValue.workflowId
      ? initialRouteValue.workflowId
      : null,
  );
  const [workflowThreadStarting, setWorkflowThreadStarting] = useState(false);
  const [messagesByThread, setMessagesByThread] = useState<MessageMap>({});
  const [threadInfoByThread, setThreadInfoByThread] = useState<
    Record<string, ThreadRuntimeInfo | null>
  >({});
  const [messageState, reactDispatchMessageState] = useReducer(
    messageMachineReducer,
    initialMessageMachineState,
  );
  const [composer, setComposer] = useState("");
  const [composerResetKey, setComposerResetKey] = useState(0);
  const [composerTextPresent, setComposerTextPresent] = useState(false);
  const [composerImages, setComposerImages] = useState<
    MessageImageAttachment[]
  >([]);
  const [composerFiles, setComposerFiles] = useState<MessageFileAttachment[]>(
    [],
  );
  const [composerAttachmentUploadPending, setComposerAttachmentUploadPending] =
    useState(false);
  const [titleDraft, setTitleDraft] = useState(DEFAULT_SESSION_TITLE);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [historyPaginationByThread, setHistoryPaginationByThread] = useState<
    Record<string, ThreadHistoryPaginationState>
  >({});
  const [savingTitle, setSavingTitle] = useState(false);
  const [editingThreadTitle, setEditingThreadTitle] = useState(false);
  const [deletingThreadId, setDeletingThreadId] = useState<string | null>(null);
  const [bindingMutation, setBindingMutation] = useState<string | null>(null);
  const [inspectorOpen, setInspectorOpen] = useState(false);
  const [threadLogsOpen, setThreadLogsOpen] = useState(false);
  const [threadLogsActiveTab, setThreadLogsActiveTab] =
    useState<ThreadLogTab>("client");
  const [threadLogsText, setThreadLogsText] = useState("");
  const [threadLogsPath, setThreadLogsPath] = useState("");
  const [threadLogsCursor, setThreadLogsCursor] = useState(0);
  const [threadLogsLoading, setThreadLogsLoading] = useState(false);
  const [threadLogsError, setThreadLogsError] = useState<string | null>(null);
  const [threadLogsHasUnread, setThreadLogsHasUnread] = useState(false);
  const [clientLogsByThread, setClientLogsByThread] = useState<
    Record<string, ClientLogEntry[]>
  >({});
  const [clientLogsHasUnread, setClientLogsHasUnread] = useState(false);
  const [expandedClientLogEntries, setExpandedClientLogEntries] = useState<
    Record<string, boolean>
  >({});
  const [threadLogsPanelWidth, setThreadLogsPanelWidth] = useState(
    DEFAULT_DESKTOP_SETTINGS.threadLogsPanelWidth,
  );
  const [threadLogsResizing, setThreadLogsResizing] = useState(false);
  const [sidebarWidth, setSidebarWidth] = useState(245);
  const [sidebarResizing, setSidebarResizing] = useState(false);
  const [railWidth, setRailWidth] = useState(258);
  const [railResizing, setRailResizing] = useState(false);
  const [botConversationGroupId, setBotConversationGroupId] = useState<string | null>(null);
  const [workspaceConversationPath, setWorkspaceConversationPath] =
    useState<string | null>(null);
  const sidebarResizeStateRef = useRef<{
    startX: number;
    startWidth: number;
  } | null>(null);
  const railResizeStateRef = useRef<{
    startX: number;
    startWidth: number;
  } | null>(null);

  useLayoutEffect(() => {
    const root = document.documentElement;
    root.style.setProperty("--app-sidebar-width", `${sidebarWidth}px`);
    return () => {
      root.style.removeProperty("--app-sidebar-width");
    };
  }, [sidebarWidth]);

  useLayoutEffect(() => {
    const root = document.documentElement;
    root.style.setProperty("--spacing-token-rail", `${railWidth}px`);
    return () => {
      root.style.removeProperty("--spacing-token-rail");
    };
  }, [railWidth]);
  const [contentView, setContentViewRaw] = useState<ContentView>(() =>
    initialContentView(initialRouteValue),
  );
  useEffect(() => {
    if (contentView !== "thread" || !selectedThreadId) {
      setThreadEntrySelectionSource(null);
    }
  }, [contentView, selectedThreadId]);
  const setContentView: typeof setContentViewRaw = (action) => {
    setContentViewRaw((prev) => {
      const next = typeof action === "function" ? action(prev) : action;
      sessionStorage.setItem("gary-content-view", next);
      return next;
    });
  };
  const [addBotDialogOpen, setAddBotDialogOpen] = useState(false);
  const [addBotInitialValues, setAddBotInitialValues] = useState<{
    channel?: "telegram" | "feishu" | "weixin";
    accountId?: string;
    name?: string;
    token?: string;
    baseUrl?: string;
  } | null>(null);
  const [workspaceMutation, setWorkspaceMutation] = useState<
    "add" | "assign" | "relink" | "remove" | null
  >(null);
  const [addWorkspaceDialog, setAddWorkspaceDialog] = useState<{
    source: "new-thread" | "task";
    initialPath?: string;
    resolve?: (workspace: DesktopWorkspace | null) => void;
  } | null>(null);
  const [workspaceMenuOpenPath, setWorkspaceMenuOpenPath] = useState<string | null>(
    null,
  );
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const [liveStreamStateByThread, setLiveStreamStateByThread] = useState<
    Record<string, LiveStreamState>
  >({});
  const [pendingRemoteInputsByThread, setPendingRemoteInputsByThread] =
    useState<PendingThreadInputMap>({});
  const [pendingAutomationRunsByThread, setPendingAutomationRunsByThread] =
    useState<Record<string, PendingAutomationRun>>({});
  const [draggedQueueIntentId, setDraggedQueueIntentId] = useState<
    string | null
  >(null);
  const [queueDropTarget, setQueueDropTarget] = useState<{
    intentId: string;
    position: "before" | "after";
  } | null>(null);
  const composerAttachmentInputRef = useRef<HTMLInputElement | null>(null);
  const composerTextareaRef = useRef<HTMLTextAreaElement | null>(null);
  const threadTitleInputRef = useRef<HTMLInputElement | null>(null);
  const messagesRef = useRef<HTMLDivElement | null>(null);
  const threadLogsRef = useRef<HTMLDivElement | null>(null);
  const threadLayoutRef = useRef<HTMLDivElement | null>(null);
  const selectedThreadIdRef = useRef<string | null>(null);
  const newThreadDraftActiveRef = useRef(false);
  const pendingWorkspacePathRef = useRef<string | null>(null);
  const pendingWorkspaceModeRef = useRef<DesktopWorkspaceMode>("local");
  const pendingBotIdRef = useRef<string | null>(null);
  const composerHasPayloadRef = useRef(false);
  const newThreadInitialDispatchLockRef = useRef(false);
  const threadLogsOpenRef = useRef(false);
  const threadLogsActiveTabRef = useRef<ThreadLogTab>("client");
  const clientLogSequenceRef = useRef(1);
  const pendingClientLogEventsRef = useRef<DesktopChatStreamEvent[]>([]);
  const clientLogFlushFrameRef = useRef<number | null>(null);
  const pendingAssistantDeltaByThreadRef = useRef<
    Record<string, PendingAssistantDelta>
  >({});
  const assistantDeltaFlushFrameRef = useRef<number | null>(null);
  const messagesByThreadRef = useRef<MessageMap>({});
  const historyPaginationByThreadRef = useRef<
    Record<string, ThreadHistoryPaginationState>
  >({});
  const messageStateRef = useRef(initialMessageMachineState);
  const liveStreamStateRef = useRef<Record<string, LiveStreamState>>({});
  const pendingRemoteInputsRef = useRef<PendingThreadInputMap>({});
  const deferredQueueDrainByThreadRef = useRef<Record<string, boolean>>({});
  const queueDrainInFlightByThreadRef = useRef<Record<string, boolean>>({});
  const pendingAutomationRunsRef = useRef<Record<string, PendingAutomationRun>>(
    {},
  );
  const threadTitleOverridesRef = useRef<Record<string, string>>({});
  const streamEventHandlerRef = useRef<(event: DesktopChatStreamEvent) => void>(
    () => {},
  );
  const deepLinkEventHandlerRef = useRef<(event: DesktopDeepLinkEvent) => void>(
    () => {},
  );
  const isComposingRef = useRef(false);
  const composerSubmitLockRef = useRef(false);
  const ignoreComposerSubmitUntilRef = useRef(0);
  const threadLogsCursorRef = useRef(0);
  const toastSequenceRef = useRef(1);
  const toastTimeoutsRef = useRef<Record<number, number>>({});
  const composerDraftRef = useRef("");
  const composerPhaseSyncKeyRef = useRef("");
  const gatewayRetryStepRef = useRef(0);
  const gatewaySetupSavedConnectionRef = useRef<ConnectionStatus | null>(null);
  const botBindingRequestSequenceRef = useRef(0);
  const previousConnectionOkRef = useRef<boolean | null>(null);
  const lastRemoteStateWarningKeyRef = useRef<string | null>(null);
  const threadLogsPanelWidthRef = useRef(
    DEFAULT_DESKTOP_SETTINGS.threadLogsPanelWidth,
  );
  const threadLogsResizeStateRef = useRef<{
    startX: number;
    startWidth: number;
  } | null>(null);
  const pendingThreadBottomSnapRef = useRef<string | null>(null);
  const pendingMessagesPrependAnchorRef = useRef<{
    threadId: string;
    scrollHeight: number;
    scrollTop: number;
  } | null>(null);
  const forceMessagesBottomSnapRef = useRef(false);
  const shouldStickMessagesToBottomRef = useRef(true);
  const messagesStickScrollFrameRef = useRef<number | null>(null);
  const lastRenderedMessageThreadRef = useRef<string | null>(null);
  const lastRenderedMessageCountRef = useRef(0);
  const lastRenderedMessageTailSignatureRef = useRef("0");
  const remoteTranscriptSignatureByThreadRef = useRef<Record<string, string>>(
    {},
  );
  const shouldFocusComposerRef = useRef(false);
  const memoryDialogRequestIdRef = useRef(0);
  const [memoryDialogTarget, setMemoryDialogTarget] =
    useState<MemoryDialogTarget | null>(null);
  // The browser tab content is an Electron `WebContentsView` — an
  // OS-level layer that sits above every renderer-DOM modal regardless
  // of CSS z-index. Pause it while the Memory dialog is open so the
  // dialog isn't covered; bounds stay set, so unpausing re-mounts at
  // the same rect without BrowserPage having to re-sync.
  useEffect(() => {
    const open = Boolean(memoryDialogTarget);
    void window.garyxDesktop.setBrowserOverlayPaused(open);
    return () => {
      if (open) {
        void window.garyxDesktop.setBrowserOverlayPaused(false);
      }
    };
  }, [memoryDialogTarget]);
  const [memoryDialogDocument, setMemoryDialogDocument] =
    useState<DesktopMemoryDocument | null>(null);
  const [memoryDialogDraft, setMemoryDialogDraft] = useState("");
  const [memoryDialogSavedContent, setMemoryDialogSavedContent] = useState("");
  const [memoryDialogLoading, setMemoryDialogLoading] = useState(false);
  const [memoryDialogSaving, setMemoryDialogSaving] = useState(false);
  const [memoryDialogError, setMemoryDialogError] = useState<string | null>(
    null,
  );
  const [memoryDialogStatus, setMemoryDialogStatus] = useState<string | null>(
    null,
  );
  const {
    automationDialog,
    automationMutation,
    automationStatus,
    automationAgentOptions,
    automations,
    handleDeleteAutomation,
    handleOpenAutomationThread,
    handleRunAutomationNow,
    handleSelectAutomation,
    handleSubmitAutomationDialog,
    handleToggleAutomationEnabled,
    openAutomationDialog,
    selectedAutomationId,
    setAutomationDialog,
    setAutomationStatus,
    updateAutomationDialogDraft,
  } = useAutomationController({
    contentView,
    desktopState,
    desktopAgents,
    desktopTeams,
    pendingThreadBottomSnapRef,
    selectedThreadId,
    setContentView,
    setDesktopState,
    setError,
    setNewThreadDraftActive,
    setSelectedThreadId,
    setPendingAutomationRun,
    reconcilePendingAutomationRun,
  });
  const {
    loading: autoResearchLoading,
    saving: autoResearchSaving,
    runs: autoResearchRuns,
    runDetail: autoResearchRunDetail,
    iterations: autoResearchIterations,
    candidatesResponse: autoResearchCandidatesResponse,
    createRun: createAutoResearchRun,
    loadRun: loadAutoResearchRun,
    stopRun: stopAutoResearchRun,
    deleteRun: deleteAutoResearchRun,
    selectCandidate: selectAutoResearchCandidate,
  } = useAutoResearchController(contentView === "auto_research", setError);
  const {
    commands,
    commandsLoaded,
    commandsLoading,
    commandsSaving,
    gatewaySettingsDirty,
    gatewaySettingsDraft,
    gatewaySettingsLoading,
    gatewaySettingsSaving,
    gatewaySettingsSource,
    gatewaySettingsStatus,
    handleCreateMcpServer,
    handleCreateSlashCommand,
    handleDeleteMcpServer,
    handleDeleteSlashCommand,
    handleRetrySettingsView,
    handleSaveGatewaySettings,
    handleSaveGatewaySettingsPatch,
    handleSaveLocalSettingsDraft,
    handleSaveLocalSettingsNow,
    handleSelectSettingsTab,
    handleToggleMcpServer,
    handleUpdateMcpServer,
    handleUpdateSlashCommand,
    loadGatewaySettings,
    loadSlashCommands,
    localSettingsDirty,
    localSettingsStatus,
    mcpServers,
    mcpServersLoading,
    mcpServersSaving,
    mutateGatewaySettingsDraft,
    persistLocalSettings,
    refreshSettingsTabResources,
    savingSettings,
    setGatewaySettingsStatus,
    setLocalSettingsStatus,
    setSettingsDraft,
    settingsActiveTab,
    settingsDraft,
  } = useSettingsController({
    desktopState,
    initialSettingsTab:
      initialRouteValue.kind === "settings" ? initialRouteValue.tabId : null,
    setConnection,
    setDesktopState,
    setError,
  });
  const locale = useResolvedLocale(settingsDraft.languagePreference);
  const t = useMemo(() => createTranslator(locale), [locale]);

  const dismissToast = useCallback((id: number) => {
    const timeoutId = toastTimeoutsRef.current[id];
    if (timeoutId) {
      window.clearTimeout(timeoutId);
      delete toastTimeoutsRef.current[id];
    }
    setToasts((current) => current.filter((toast) => toast.id !== id));
  }, []);

  const pushToast = useCallback(
    (
      message: string,
      tone: ToastTone = "info",
      durationMs = tone === "error" ? ERROR_TOAST_MS : TRANSIENT_STATUS_MS,
    ) => {
      const normalizedMessage = message.trim();
      if (!normalizedMessage) {
        return;
      }

      const id = toastSequenceRef.current;
      toastSequenceRef.current += 1;
      setToasts((current) => [
        ...current.slice(-2),
        { id, message: normalizedMessage, tone },
      ]);
      const timeoutId = window.setTimeout(() => {
        delete toastTimeoutsRef.current[id];
        setToasts((current) => current.filter((toast) => toast.id !== id));
      }, durationMs);
      toastTimeoutsRef.current[id] = timeoutId;
    },
    [],
  );

  useEffect(() => {
    return () => {
      Object.values(toastTimeoutsRef.current).forEach((timeoutId) => {
        window.clearTimeout(timeoutId);
      });
      toastTimeoutsRef.current = {};
    };
  }, []);

  useEffect(() => {
    if (!error) {
      return undefined;
    }
    const gatewaySetupMessage = gatewaySetupMessageForAuthError(error);
    if (gatewaySetupMessage) {
      setConnection({
        ok: false,
        bridgeReady: false,
        gatewayUrl: connection?.gatewayUrl || settingsDraft.gatewayUrl,
        error: gatewaySetupMessage,
      });
      setError(null);
      return undefined;
    }
    if (isTransientGatewayErrorMessage(error)) {
      recordGatewayStatusObservation(
        {
          ok: false,
          bridgeReady: false,
          gatewayUrl: connection?.gatewayUrl || settingsDraft.gatewayUrl,
          error,
        },
        hasGatewayRecoveryActivity()
          ? "Connection unstable. Waiting for gateway updates…"
          : "Reconnecting to gateway…",
      );
      setError(null);
      return undefined;
    }
    pushToast(error, "error");
    setError(null);
    return undefined;
  }, [connection?.gatewayUrl, error, pushToast, settingsDraft.gatewayUrl]);

  useEffect(() => {
    if (!gatewaySettingsStatus) {
      return undefined;
    }
    const gatewaySetupMessage =
      gatewaySetupMessageForAuthError(gatewaySettingsStatus);
    if (gatewaySetupMessage) {
      setConnection({
        ok: false,
        bridgeReady: false,
        gatewayUrl: connection?.gatewayUrl || settingsDraft.gatewayUrl,
        error: gatewaySetupMessage,
      });
      setGatewaySettingsStatus(null);
      return undefined;
    }
    pushToast(
      t(gatewaySettingsStatus),
      /(cannot|error|failed|failure|invalid|missing|unable)/i.test(gatewaySettingsStatus)
        ? "error"
        : "success",
    );
    setGatewaySettingsStatus(null);
    return undefined;
  }, [
    connection?.gatewayUrl,
    gatewaySettingsStatus,
    pushToast,
    settingsDraft.gatewayUrl,
    t,
  ]);

  async function handleOpenGatewaySetup() {
    setLocalSettingsStatus(null);
    const savedSettings = desktopState?.settings;
    const savedConnection = isConnectionValidForSettings(connection, savedSettings)
      ? connection
      : null;
    gatewaySetupSavedConnectionRef.current = savedConnection;
    setGatewaySetupCanCancel(Boolean(savedConnection));
    setGatewaySetupForced(true);

    if (!savedSettings?.gatewayUrl.trim()) {
      gatewaySetupSavedConnectionRef.current = null;
      setGatewaySetupCanCancel(false);
      return;
    }

    try {
      const status = await window.garyxDesktop.checkConnection({
        gatewayUrl: savedSettings.gatewayUrl,
        gatewayAuthToken: savedSettings.gatewayAuthToken,
      });
      setConnection(status);
      if (isConnectionValidForSettings(status, savedSettings)) {
        gatewaySetupSavedConnectionRef.current = status;
        setGatewaySetupCanCancel(true);
      } else {
        gatewaySetupSavedConnectionRef.current = null;
        setGatewaySetupCanCancel(false);
      }
    } catch {
      gatewaySetupSavedConnectionRef.current = null;
      setGatewaySetupCanCancel(false);
    }
  }

  function handleCancelGatewaySetup() {
    const savedSettings = desktopState?.settings;
    const savedConnection = gatewaySetupSavedConnectionRef.current;
    if (
      !gatewaySetupCanCancel ||
      !savedSettings ||
      !isConnectionValidForSettings(savedConnection, savedSettings)
    ) {
      return;
    }

    setSettingsDraft((current) => ({
      ...current,
      gatewayUrl: savedSettings.gatewayUrl,
      gatewayAuthToken: savedSettings.gatewayAuthToken,
    }));
    setConnection(savedConnection);
    setError(null);
    setGatewaySettingsStatus(null);
    setLocalSettingsStatus(null);
    setGatewaySetupForced(false);
    setGatewaySetupCanCancel(false);
    gatewaySetupSavedConnectionRef.current = null;
  }

  useEffect(() => {
    if (!automationStatus) {
      return undefined;
    }
    pushToast(automationStatus, "success");
    setAutomationStatus(null);
    return undefined;
  }, [automationStatus, pushToast]);

  useEffect(() => {
    recordGatewayStatusObservation(connection, connection?.error);
  }, [connection]);

  function dispatchMessageState(action: MessageMachineAction) {
    messageStateRef.current = messageMachineReducer(
      messageStateRef.current,
      action,
    );
    reactDispatchMessageState(action);
  }

  function currentThreadLayoutWidth(): number | null {
    return threadLayoutRef.current?.clientWidth || null;
  }

  function hasGatewayRecoveryActivity(): boolean {
    const hasBusyStream = Object.values(liveStreamStateRef.current).some(
      (stream) => {
        return [
          "connecting",
          "streaming",
          "reconciling",
          "disconnected",
        ].includes(stream.streamStatus);
      },
    );
    if (hasBusyStream) {
      return true;
    }
    return Object.values(messageStateRef.current.intentsById).some((intent) => {
      return [
        "dispatching",
        "remote_accepted",
        "awaiting_provider_ack",
        "awaiting_response",
        "awaiting_history",
      ].includes(intent.state);
    });
  }

  function recoveryThreadIds(): string[] {
    const ids = new Set<string>();
    for (const stream of Object.values(liveStreamStateRef.current)) {
      if (
        ["connecting", "reconciling", "disconnected"].includes(
          stream.streamStatus,
        )
      ) {
        ids.add(stream.threadId);
      }
    }
    for (const intent of Object.values(messageStateRef.current.intentsById)) {
      if (
        intent.threadId &&
        [
          "remote_accepted",
          "awaiting_provider_ack",
          "awaiting_response",
          "awaiting_history",
        ].includes(intent.state)
      ) {
        ids.add(intent.threadId);
      }
    }
    if (selectedThreadId) {
      const runtime = selectThreadRuntime(
        messageStateRef.current,
        selectedThreadId,
      );
      if (runtime?.state === "reconciling_history") {
        ids.add(selectedThreadId);
      }
    }
    return [...ids];
  }

  function recordGatewayStatusObservation(
    status: ConnectionStatus | null,
    reason?: string | null,
  ) {
    if (status?.ok) {
      setGatewayFailureCount(0);
      setGatewayStatusHint(null);
      return;
    }

    setGatewayFailureCount((current) => current + 1);
    setGatewayStatusHint(reason || null);
  }

  async function persistThreadLogsPanelWidth(nextWidth: number) {
    const clampedWidth = clampThreadLogsPanelWidth(
      nextWidth,
      currentThreadLayoutWidth(),
    );
    setThreadLogsPanelWidth(clampedWidth);
    setSettingsDraft((current) => ({
      ...current,
      threadLogsPanelWidth: clampedWidth,
    }));

    const persistedWidth = desktopState?.settings.threadLogsPanelWidth;
    if (persistedWidth === clampedWidth) {
      return;
    }

    try {
      const nextState = await window.garyxDesktop.saveSettings({
        ...(desktopState?.settings || DEFAULT_DESKTOP_SETTINGS),
        threadLogsPanelWidth: clampedWidth,
      });
      setDesktopState(nextState);
    } catch {
      // Keep the local width even if persistence fails; this is a non-blocking UI preference.
    }
  }

  function threadLogsNearBottom() {
    const node = threadLogsRef.current;
    if (!node) {
      return true;
    }
    return node.scrollHeight - node.scrollTop - node.clientHeight < 48;
  }

  function scrollThreadLogsToLatest(behavior: ScrollBehavior = "auto") {
    const node = threadLogsRef.current;
    if (!node) {
      return;
    }
    node.scrollTo({
      top: node.scrollHeight,
      behavior,
    });
  }

  function handleSidebarResizeStart(event: React.PointerEvent<HTMLDivElement>) {
    sidebarResizeStateRef.current = {
      startX: event.clientX,
      startWidth: sidebarWidth,
    };
    setSidebarResizing(true);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    event.preventDefault();
  }

  function handleRailResizeStart(event: React.PointerEvent<HTMLDivElement>) {
    railResizeStateRef.current = {
      startX: event.clientX,
      startWidth: railWidth,
    };
    setRailResizing(true);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    event.preventDefault();
  }

  function handleThreadLogsResizeStart(
    event: React.PointerEvent<HTMLDivElement>,
  ) {
    if (!threadLogsOpen) {
      return;
    }
    threadLogsResizeStateRef.current = {
      startX: event.clientX,
      startWidth: threadLogsPanelWidthRef.current,
    };
    setThreadLogsResizing(true);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    event.preventDefault();
  }

  function handleThreadLogsResizeKeyDown(
    event: React.KeyboardEvent<HTMLDivElement>,
  ) {
    if (!threadLogsOpen) {
      return;
    }
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) {
      return;
    }

    event.preventDefault();
    const step = event.shiftKey ? 48 : 24;
    const nextWidth =
      event.key === "Home"
        ? THREAD_LOG_PANEL_MIN_WIDTH
        : event.key === "End"
          ? clampThreadLogsPanelWidth(
              THREAD_LOG_PANEL_MAX_WIDTH,
              currentThreadLayoutWidth(),
            )
          : event.key === "ArrowLeft"
            ? clampThreadLogsPanelWidth(
                threadLogsPanelWidthRef.current + step,
                currentThreadLayoutWidth(),
              )
            : clampThreadLogsPanelWidth(
                threadLogsPanelWidthRef.current - step,
                currentThreadLayoutWidth(),
              );
    void persistThreadLogsPanelWidth(nextWidth);
  }

  const activeThread = selectedThread(desktopState, selectedThreadId);
  const threadSummaryById = useMemo(() => {
    const summaries = new Map<string, DesktopThreadSummary>();
    for (const thread of desktopState?.threads || []) {
      summaries.set(thread.id, thread);
    }
    for (const session of desktopState?.sessions || []) {
      summaries.set(session.id, session);
    }
    if (activeThread) {
      summaries.set(activeThread.id, activeThread);
    }
    return summaries;
  }, [activeThread, desktopState]);
  const pinnedThreadIds = desktopState?.pinnedThreadIds || [];
  const pinnedThreadIdSet = useMemo(
    () => new Set(pinnedThreadIds),
    [pinnedThreadIds],
  );
  const selectedThreadPinned = selectedThreadId
    ? pinnedThreadIdSet.has(selectedThreadId)
    : false;
  const activeThreadTeamView = deriveThreadTeamView(activeThread);
  const desktopAgentMap = new Map(
    desktopAgents.map((agent) => [agent.agentId, agent] as const),
  );
  const teamAgentDisplayNamesById = useMemo(
    () =>
      Object.fromEntries(
        desktopAgents.map((agent) => [agent.agentId, agent.displayName]),
      ),
    [desktopAgents],
  );
  const desktopTeamMap = new Map(
    desktopTeams.map((team) => [team.teamId, team] as const),
  );
  const activeAgentId = activeThread?.agentId || null;
  const pendingAgent = desktopAgentMap.get(pendingAgentId) || null;
  const pendingTeam = desktopTeamMap.get(pendingAgentId) || null;
  const activeAgent = activeAgentId
    ? desktopAgentMap.get(activeAgentId) || null
    : null;
  const pendingTeamLeader = pendingTeam
    ? desktopAgentMap.get(pendingTeam.leaderAgentId) || null
    : null;
  const activeThreadTeamId =
    activeThread?.team?.team_id?.trim() || activeThread?.teamId?.trim() || "";
  const activeSourceTeam = activeThreadTeamId
    ? desktopTeamMap.get(activeThreadTeamId) || null
    : null;
  const activeTeamLeaderId =
    activeThread?.team?.leader_agent_id?.trim() ||
    activeSourceTeam?.leaderAgentId?.trim() ||
    "";
  const activeTeamLeader = activeTeamLeaderId
    ? desktopAgentMap.get(activeTeamLeaderId) || null
    : null;
  const activeTeamSummary = (() => {
    const teamBlock = activeThread?.team || null;
    const teamId = teamBlock?.team_id?.trim() || activeThread?.teamId?.trim() || "";
    if (!teamId) {
      return null;
    }

    const sourceTeam = desktopTeamMap.get(teamId) || null;
    const leaderAgentId =
      teamBlock?.leader_agent_id?.trim() ||
      sourceTeam?.leaderAgentId?.trim() ||
      activeThread?.agentId?.trim() ||
      "";
    const memberAgentIds = teamBlock?.member_agent_ids?.length
      ? teamBlock.member_agent_ids
      : sourceTeam?.memberAgentIds?.length
        ? sourceTeam.memberAgentIds
        : leaderAgentId
          ? [leaderAgentId]
          : [];
    const childThreadIds = teamBlock?.child_thread_ids || {};
    const orderedAgentIds = Array.from(
      new Set([...(leaderAgentId ? [leaderAgentId] : []), ...memberAgentIds]),
    );

    if (!orderedAgentIds.length) {
      return null;
    }

    return {
      teamId,
      teamName:
        sourceTeam?.displayName?.trim() ||
        activeThread?.teamName?.trim() ||
        activeThread?.title ||
        teamId,
      members: orderedAgentIds.map((agentId) => ({
        agentId,
        displayName: desktopAgentMap.get(agentId)?.displayName || agentId,
        role:
          agentId === leaderAgentId ? ("leader" as const) : ("member" as const),
        isCurrentAgent: agentId === leaderAgentId,
        threadId:
          childThreadIds[agentId] ||
          (agentId === leaderAgentId ? activeThread?.id || null : null),
      })),
    };
  })();

  useEffect(() => {
    const teamId =
      activeThread?.team?.team_id?.trim() || activeThread?.teamId?.trim() || "";
    if (!teamId || desktopTeamMap.has(teamId)) {
      return undefined;
    }

    let cancelled = false;
    void window.garyxDesktop
      .listTeams()
      .then((teams) => {
        if (cancelled || !teams.some((team) => team.teamId === teamId)) {
          return;
        }
        startTransition(() => {
          setDesktopTeams(teams);
        });
      })
      .catch(() => {});

    return () => {
      cancelled = true;
    };
  }, [activeThread?.teamId, desktopTeams]);

  const composerAgentOptions = useMemo(
    () => buildAgentOptions(desktopAgents, desktopTeams),
    [desktopAgents, desktopTeams],
  );
  const composerWorkflowOptions = useMemo(
    () => buildComposerWorkflowOptions(desktopWorkflows),
    [desktopWorkflows],
  );
  useEffect(() => {
    if (!newThreadDraftActive) {
      return undefined;
    }
    let cancelled = false;
    setWorkflowDefinitionsLoading(true);
    void window.garyxDesktop
      .listWorkflowDefinitions()
      .then((workflows) => {
        if (cancelled) {
          return;
        }
        startTransition(() => {
          setDesktopWorkflows(workflows);
        });
      })
      .catch(() => {
        if (!cancelled) {
          startTransition(() => {
            setDesktopWorkflows([]);
          });
        }
      })
      .finally(() => {
        if (!cancelled) {
          setWorkflowDefinitionsLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [newThreadDraftActive]);
  const addBotAgentTargets = useMemo(() => {
    const options = buildAgentTargetOptions(desktopAgents, desktopTeams);
    return options.length
      ? options
      : [{ id: "claude", value: "claude", label: "Claude", kind: "builtin" as const, providerType: "claude_code" as const }];
  }, [desktopAgents, desktopTeams]);
  const pendingAgentLabel =
    pendingTeam?.displayName?.trim() ||
    pendingAgent?.displayName?.trim() ||
    pendingAgentId ||
    null;
  const pendingWorkflow =
    composerWorkflowOptions.find((workflow) => workflow.id === pendingWorkflowId) ||
    null;
  const activeAgentLabel =
    activeThreadTeamView.teamDisplayName ||
    activeSourceTeam?.displayName?.trim() ||
    activeAgent?.displayName ||
    activeThread?.agentId ||
    activeThread?.teamId ||
    null;
  const composerProviderType: DesktopApiProviderType = selectedThreadId
    ? activeThreadTeamView.isTeam
      ? activeTeamLeader?.providerType || "claude_code"
      : activeAgent?.providerType || "claude_code"
    : pendingTeam
      ? pendingTeamLeader?.providerType || "claude_code"
      : pendingAgent?.providerType || "claude_code";
  const composerAgentLabel = selectedThreadId
    ? activeAgentLabel
    : pendingWorkflow?.label || pendingAgentLabel;
  const gatewayIndicator = computeGatewayIndicator({
    status: connection,
    failureCount: gatewayFailureCount,
    recovering: hasGatewayRecoveryActivity(),
    reason: gatewayStatusHint || connection?.error || null,
  });
  const mobileThreadLogLines = useMemo(
    () => buildThreadLogLines(threadLogsText),
    [threadLogsText],
  );
  const clientThreadLogEntries = selectedThreadId
    ? clientLogsByThread[selectedThreadId] || []
    : [];
  const activeThreadLogsPath =
    threadLogsActiveTab === "client"
      ? "Renderer stream events received by desktop app"
      : threadLogsPath || "Waiting for log file";
  const activeThreadLogsHasUnread =
    threadLogsActiveTab === "client"
      ? clientLogsHasUnread
      : threadLogsHasUnread;
  const selectedWorkspaceEntry = selectedWorkspace(
    desktopState,
    desktopState?.selectedWorkspacePath || null,
  );
  const activeThreadWorkspace = workspaceForThread(
    desktopState,
    selectedThreadId,
  );
  const pendingWorkspaceEntry = selectedWorkspace(
    desktopState,
    pendingWorkspacePath,
  );
  const hasNewThreadDraft = newThreadDraftActive && !selectedThreadId;
  const activeThreadMessageKey =
    selectedThreadId ||
    (hasNewThreadDraft ? NEW_THREAD_DRAFT_THREAD_ID : null);
  const rawActiveMessages = activeThreadMessageKey
    ? messagesByThread[activeThreadMessageKey] || EMPTY_UI_TRANSCRIPT_MESSAGES
    : EMPTY_UI_TRANSCRIPT_MESSAGES;
  const activeMessages = useMemo(
    () =>
      rawActiveMessages.filter(
        (message) => !isRunLoadingPlaceholderMessage(message),
      ),
    [rawActiveMessages],
  );
  const activeHistoryPagination = activeThreadMessageKey
    ? historyPaginationByThread[activeThreadMessageKey] || null
    : null;
  const activeThreadInfo = selectedThreadId
    ? threadInfoByThread[selectedThreadId] || null
    : null;
  const activeThreadInfoLoaded = selectedThreadId
    ? Object.prototype.hasOwnProperty.call(threadInfoByThread, selectedThreadId)
    : false;
  const activeThreadWorktree =
    activeThreadInfo?.worktree || activeThread?.worktree || null;
  const composerWorkspaceMode: DesktopWorkspaceMode | null =
    selectedThreadId && activeThreadWorktree ? "worktree" : null;
  const composerWorkspaceBranch = activeThreadWorktree?.branch?.trim() || null;
  const activePendingAutomationRun = selectedThreadId
    ? pendingAutomationRunsByThread[selectedThreadId] || null
    : null;
  const activeHasAssistantOrToolMessage = useMemo(
    () =>
      activeMessages.some((message) => {
        return message.role === "assistant" || isToolRole(message.role);
      }),
    [activeMessages],
  );
  const activeRenderableMessages = useMemo(
    () => buildRenderableTranscript(activeMessages),
    [activeMessages],
  );
  const activeRenderableBlocks = useMemo(
    () => buildRenderTranscriptBlocks(activeRenderableMessages),
    [activeRenderableMessages],
  );
  const lastActiveRenderableBlock =
    activeRenderableBlocks[activeRenderableBlocks.length - 1] || null;
  const activeTailToolTraceBlockKey =
    lastActiveRenderableBlock?.kind === "tool_group"
      ? lastActiveRenderableBlock.key
      : null;
  const activeQueue = selectQueueIntentIds(messageState, activeThreadMessageKey)
    .map((intentId) => messageState.intentsById[intentId])
    .filter((intent): intent is MessageIntent => Boolean(intent));
  const activeRuntime = selectThreadRuntime(
    messageState,
    activeThreadMessageKey,
  );
  const activeLiveStream = activeThreadMessageKey
    ? liveStreamStateByThread[activeThreadMessageKey] || null
    : null;
  const activePendingAckIntents = (activeLiveStream?.pendingAckIntentIds || [])
    .map((intentId) => messageState.intentsById[intentId])
    .filter((intent): intent is MessageIntent => Boolean(intent));
  const visiblePendingAckIntents = activePendingAckIntents.filter((intent) => {
    return !activeMessages.some((message) => {
      return (
        message.role === "user" &&
        (message.intentId === intent.intentId ||
          transcriptMessageMatchesIntent(message, intent))
      );
    });
  });
  const activeThreadRunId =
    activeLiveStream?.runId || activeThread?.recentRunId || null;
  const activeRemotePendingInputs = selectedThreadId
    ? pendingRemoteInputsByThread[selectedThreadId] || []
    : [];
  const visibleRemotePendingInputs =
    activePendingAckIntents.length > 0
      ? []
      : activeRemotePendingInputs.filter((input) => {
          if (input.status !== "awaiting_ack") {
            return false;
          }
          // Follow gateway pending-input state, but suppress duplicate UI when the
          // same queued input has already landed in the visible transcript.
          return !activeMessages.some((message) =>
            pendingThreadInputMatchesMessage(input, message),
          );
        });
  const visibleRemoteAwaitingAckInputs = visibleRemotePendingInputs;
  const activePendingHistoryIntent = activeThreadMessageKey
    ? Object.values(messageState.intentsById).some((intent) => {
        return (
          intent.threadId === activeThreadMessageKey &&
          [
            "dispatching",
            "remote_accepted",
            "awaiting_provider_ack",
            "awaiting_response",
            "awaiting_history",
          ].includes(intent.state)
        );
      })
    : false;
  const threadActivity = deriveThreadActivityModel({
    messages: activeMessages,
    threadInfo: activeThreadInfo,
    liveStream: activeLiveStream,
    runtimeBusy: Boolean(activeRuntime && isRuntimeBusy(activeRuntime.state)),
    pendingAckIntentCount: activePendingAckIntents.length,
    remoteAwaitingAckInputCount: visibleRemoteAwaitingAckInputs.length,
    pendingHistoryIntent: activePendingHistoryIntent,
  });
  const showPendingAckLoading = threadActivity.showPendingAckLoading;
  const activeRunLoading = threadActivity.showRunLoading;
  const canSteerQueuedPrompt = threadActivity.canSteerQueuedPrompt;
  const isActiveStreamingThread = Boolean(
    activeLiveStream &&
    ["connecting", "streaming", "reconciling"].includes(
      activeLiveStream.streamStatus,
    ),
  );
  const isDraftSendingThread = Boolean(
    !selectedThreadId &&
      ((activeRuntime && isRuntimeBusy(activeRuntime.state)) ||
        isActiveStreamingThread),
  );
  const activeThreadId = selectGlobalActiveThreadId(messageState);
  const workspacePickerWorkspaces = useMemo(
    () => visibleWorkspaceList(desktopState),
    [desktopState],
  );
  const pendingWorkspaceSuggestion = useMemo(
    () =>
      pendingWorkspaceEntry || workspaceSuggestionFromPath(pendingWorkspacePath),
    [pendingWorkspaceEntry, pendingWorkspacePath],
  );
  const activeThreadWorkspaceSuggestion = useMemo(
    () =>
      activeThreadWorkspace ||
      workspaceSuggestionFromPath(activeThread?.workspacePath, {
        createdAt: activeThread?.createdAt,
        updatedAt: activeThread?.updatedAt,
      }),
    [
      activeThreadWorkspace,
      activeThread?.workspacePath,
      activeThread?.createdAt,
      activeThread?.updatedAt,
    ],
  );
  const selectableNewThreadWorkspaces = useMemo(
    () =>
      newThreadWorkspaceOptions(
        workspacePickerWorkspaces,
      ),
    [workspacePickerWorkspaces],
  );
  const availableWorkspaceCount = selectableNewThreadWorkspaces.length;
  const activeAutomationThread = automationForLatestThread(
    desktopState,
    selectedThreadId,
  );
  const pendingNewThreadWorkspaceEntry = isSelectableNewThreadWorkspace(
    pendingWorkspaceSuggestion,
  )
    ? pendingWorkspaceSuggestion
    : null;
  const activeThreadNewThreadWorkspace = isSelectableNewThreadWorkspace(
    activeThreadWorkspaceSuggestion,
  )
    ? activeThreadWorkspaceSuggestion
    : null;
  const selectedNewThreadWorkspaceEntry = isSelectableNewThreadWorkspace(
    selectedWorkspaceEntry,
  )
    ? selectedWorkspaceEntry
    : null;
  const preferredWorkspaceForNewThread = pickPreferredWorkspace(
    selectableNewThreadWorkspaces,
    pendingNewThreadWorkspaceEntry,
    selectedNewThreadWorkspaceEntry,
  );
  const newThreadWorkspaceEntry =
    pendingNewThreadWorkspaceEntry || preferredWorkspaceForNewThread;
  const activeWorkspace =
    activeThreadWorkspace || pendingWorkspaceEntry || selectedWorkspaceEntry;
  const workspaceSelectionEntry =
    activeThreadWorkspace || pendingWorkspaceEntry || selectedWorkspaceEntry;
  const workspaceThreadGroups = useMemo(
    () =>
      buildWorkspaceThreadGroups({
        state: desktopState,
        activeThread,
        selectedThreadId,
        workspaceSelectionEntry,
      }),
    [activeThread, desktopState, selectedThreadId, workspaceSelectionEntry],
  );
  const activeWorkspacePath =
    activeWorkspace?.available && activeWorkspace?.path
      ? activeWorkspace.path
      : "";
  const {
    activeWorkspaceDirectoryState,
    expandedWorkspaceDirectories,
    handleLocalWorkspaceFileLinkClick,
    handleRefreshWorkspaceFiles,
    handleWorkspaceFileEntryActivate,
    loadWorkspaceDirectory,
    loadWorkspaceFilePreview,
    selectedWorkspaceFile,
    selectedWorkspaceFileEntry,
    setExpandedWorkspaceDirectories,
    setWorkspacePreviewModalOpen,
    uploadWorkspaceFilesToActiveWorkspace,
    workspaceDirectories,
    workspaceFilePreview,
    workspaceFilePreviewError,
    workspaceFilePreviewLoading,
    workspaceFileUploadPending,
    workspacePreviewModalOpen,
    workspacePreviewTitle,
    workspaceUploadDirectoryPath,
    workspaceUploadInputRef,
  } = useWorkspaceController({
    activeWorkspacePath,
    pushToast,
    setError,
    workspaces: desktopState?.workspaces || [],
  });
  const isActiveSendingThread = Boolean(
    selectedThreadId &&
      (threadActivity.runActive || showPendingAckLoading),
  );
  const composerLocked =
    composerAttachmentUploadPending ||
    isDraftSendingThread ||
    workflowThreadStarting;
  const botGroups = useMemo(
    () =>
      buildBotGroups(
        desktopState?.endpoints || [],
        desktopState?.configuredBots || [],
        desktopState?.botMainThreads || {},
        desktopState?.botConsoles || [],
      ),
    [
      desktopState?.botConsoles,
      desktopState?.botMainThreads,
      desktopState?.configuredBots,
      desktopState?.endpoints,
    ],
  );
  const visibleBotGroups = useMemo(() => {
    if (!desktopState) {
      return botGroups;
    }
    const visibleThreadIds = new Set(
      [...(desktopState.threads || []), ...(desktopState.sessions || [])].map(
        (thread) => thread.id,
      ),
    );
    return botGroups.map((group) => {
      const conversationNodes = (group.conversationNodes || []).filter(
        (entry) => {
          const threadId = entry.endpoint.threadId;
          return Boolean(
            threadId &&
              threadId !== deletingThreadId &&
              visibleThreadIds.has(threadId),
          );
        },
      );
      return conversationNodes.length === (group.conversationNodes || []).length
        ? group
        : { ...group, conversationNodes };
    });
  }, [botGroups, deletingThreadId, desktopState]);
  const activeThreadEndpoints =
    activeThread && !activeAutomationThread
      ? (desktopState?.endpoints || []).filter(
          (endpoint) => endpoint.threadId === activeThread.id,
        )
      : [];
  const activeThreadBots = boundBotsForThread(activeThreadEndpoints);
  const mappedThreadBotId = activeThread
    ? (Object.entries(desktopState?.botMainThreads || {}).find(
        ([, threadId]) => threadId === activeThread.id,
      )?.[0] ?? null)
    : null;
  const hasOptimisticActiveThreadBotBinding = Boolean(
    activeThread &&
      optimisticThreadBotBinding?.threadId === activeThread.id,
  );
  const optimisticActiveThreadBotId = hasOptimisticActiveThreadBotBinding
    ? (optimisticThreadBotBinding?.botId ?? null)
    : undefined;
  const explicitThreadBotId = activeThread
    ? (optimisticActiveThreadBotId !== undefined
        ? optimisticActiveThreadBotId
        : mappedThreadBotId)
    : pendingBotId;
  const inferredThreadBotId =
    !hasOptimisticActiveThreadBotBinding &&
    !explicitThreadBotId &&
    activeThreadBots.length === 1
      ? (activeThreadBots[0]?.id ?? null)
      : null;
  const activeThreadBotId = explicitThreadBotId ?? inferredThreadBotId;
  const activeThreadBot = activeThreadBotId
    ? (botGroups.find((g) => g.id === activeThreadBotId) ?? null)
    : null;
  const composerHasText = composerTextPresent;
  const composerHasImages = composerImages.length > 0;
  const composerHasFiles = composerFiles.length > 0;
  const composerHasPayload =
    composerHasText || composerHasImages || composerHasFiles;

  useEffect(() => {
    composerHasPayloadRef.current = composerHasPayload;
  }, [composerHasPayload]);

  const activeThreadHasMessages = Boolean(
    (activeThread?.messageCount ?? 0) > 0 || activeMessages.length > 0,
  );
  const providerSelectorLocked = Boolean(
    composerLocked ||
    isActiveSendingThread ||
    activeThreadHasMessages ||
    (historyLoading && Boolean(activeThread?.messageCount)),
  );
  const isSettingsView = contentView === "settings";
  const isBrowserView = contentView === "browser";
  const isBotsView = contentView === "bots";
  const isAutomationView = contentView === "automation";
  const isAutoResearchView = contentView === "auto_research";
  const showAutoResearchLab =
    typeof gatewaySettingsDraft?.desktop?.labs?.auto_research === "boolean"
      ? gatewaySettingsDraft.desktop.labs.auto_research
      : true;
  const showDreamsFeature = Boolean(gatewaySettingsDraft?.dreams?.enabled);
  const isAgentsView = contentView === "agents";
  const isTeamsView = contentView === "teams";
  const isSkillsView = contentView === "skills";
  const isTasksView = contentView === "tasks";
  const isWorkflowView = contentView === "workflow";
  const selectedWorkflowRunThreadId =
    contentView === "thread" && activeThread?.threadType === "workflow_run"
      ? activeThread.id
      : null;
  const isDreamsView = contentView === "dreams" && showDreamsFeature;
  const shouldShowConversationRail = contentView === "thread";
  const visibleSelectedThreadId = shouldShowConversationRail ? selectedThreadId : null;
  const visibleThreadEntrySelectionSource = shouldShowConversationRail
    ? threadEntrySelectionSource
    : null;

  useLayoutEffect(() => {
    if (contentView === "dreams" && !showDreamsFeature) {
      setContentView("thread");
    }
  }, [contentView, setContentView, showDreamsFeature]);

  const botRootSelectedThreadId =
    visibleThreadEntrySelectionSource === "bot-root" ? visibleSelectedThreadId : null;
  const botConversationSelectedThreadId =
    visibleThreadEntrySelectionSource === "bot-conversation"
      ? visibleSelectedThreadId
      : null;
  const workspaceConversationSelectedThreadId =
    visibleThreadEntrySelectionSource === "workspace-conversation"
      ? visibleSelectedThreadId
      : null;
  const pinnedThreadRows = useMemo(
    () =>
      pinnedThreadIds
        .map((threadId) => threadSummaryById.get(threadId) || null)
        .filter((thread): thread is DesktopThreadSummary => Boolean(thread))
        .map((thread) => ({
          thread,
          isActive:
            visibleThreadEntrySelectionSource === "pinned" &&
            visibleSelectedThreadId === thread.id,
          isBusy: isRuntimeBusy(selectThreadRuntime(messageState, thread.id)?.state),
        })),
    [
      messageState,
      pinnedThreadIds,
      threadSummaryById,
      visibleSelectedThreadId,
      visibleThreadEntrySelectionSource,
    ],
  );
  function pinnedThreadIdsWith(
    ids: string[],
    threadId: string,
    pinned: boolean,
  ): string[] {
    const normalizedId = threadId.trim();
    if (!normalizedId) {
      return ids;
    }
    const withoutThread = ids.filter((id) => id !== normalizedId);
    return pinned ? [normalizedId, ...withoutThread] : withoutThread;
  }

  async function setThreadPinned(threadId: string, pinned: boolean) {
    const normalizedId = threadId.trim();
    if (!normalizedId) {
      return;
    }
    setDesktopState((current) =>
      current
        ? {
            ...current,
            pinnedThreadIds: pinnedThreadIdsWith(
              current.pinnedThreadIds || [],
              normalizedId,
              pinned,
            ),
          }
        : current,
    );
    try {
      const nextState = await window.garyxDesktop.setThreadPinned({
        threadId: normalizedId,
        pinned,
      });
      setDesktopState(nextState);
    } catch (error) {
      setError(
        error instanceof Error
          ? error.message
          : pinned
            ? t("Failed to pin thread")
            : t("Failed to unpin thread"),
      );
      void refreshDesktopState().catch(() => null);
    }
  }

  function togglePinnedThread(threadId: string) {
    const pinned = (desktopState?.pinnedThreadIds || []).includes(threadId);
    void setThreadPinned(threadId, !pinned);
  }
  useEffect(() => {
    if (shouldShowConversationRail) {
      return;
    }
    setBotConversationGroupId((current) => (current ? null : current));
    setWorkspaceConversationPath((current) => (current ? null : current));
  }, [shouldShowConversationRail]);
  useEffect(() => {
    if (!botConversationGroupId) {
      return;
    }
    const groupExists = visibleBotGroups.some(
      (group) =>
        group.id === botConversationGroupId &&
        (group.conversationNodes || []).length > 0,
    );
    if (!groupExists) {
      setBotConversationGroupId(null);
    }
  }, [botConversationGroupId, visibleBotGroups]);
  const activeBotConversationGroup = useMemo(() => {
    if (!shouldShowConversationRail || !botConversationGroupId) {
      return null;
    }
    return (
      visibleBotGroups.find(
        (group) =>
          group.id === botConversationGroupId &&
          (group.conversationNodes || []).length > 0,
      ) || null
    );
  }, [botConversationGroupId, shouldShowConversationRail, visibleBotGroups]);
  useEffect(() => {
    if (!workspaceConversationPath) {
      return;
    }
    const workspaceExists = workspaceThreadGroups.some((group) => {
      const workspacePath = group.workspace.path || group.workspace.name;
      return (
        workspacePath.trim().toLowerCase() ===
        workspaceConversationPath.trim().toLowerCase()
      );
    });
    if (!workspaceExists) {
      setWorkspaceConversationPath(null);
    }
  }, [workspaceConversationPath, workspaceThreadGroups]);
  const activeWorkspaceThreadGroup = useMemo(() => {
    if (
      !shouldShowConversationRail ||
      activeBotConversationGroup ||
      !workspaceConversationPath
    ) {
      return null;
    }
    return (
      workspaceThreadGroups.find((group) => {
        const workspacePath = group.workspace.path || group.workspace.name;
        return (
          workspacePath.trim().toLowerCase() ===
          workspaceConversationPath.trim().toLowerCase()
        );
      }) || null
    );
  }, [
    activeBotConversationGroup,
    shouldShowConversationRail,
    workspaceConversationPath,
    workspaceThreadGroups,
  ]);
  const appShellClassName = [
    "app-shell",
    activeBotConversationGroup || activeWorkspaceThreadGroup
      ? "with-bot-conversation-rail"
      : null,
  ]
    .filter(Boolean)
    .join(" ");
  const conversationClassName = [
    "conversation",
    isSettingsView ? "settings-view" : null,
    isTasksView ? "tasks-view" : null,
    isWorkflowView ? "workflow-view" : null,
    isDreamsView ? "dreams-view" : null,
  ]
    .filter(Boolean)
    .join(" ");
  const showStaticWindowToolbar =
    isSettingsView ||
    isAutomationView ||
    isAutoResearchView ||
    isAgentsView ||
    isTeamsView ||
    isSkillsView;
  const canEditThreadTitle = Boolean(
    activeThread &&
    !activeAutomationThread &&
    !isAutomationView &&
    !isSkillsView &&
    !isTasksView &&
    !isWorkflowView &&
    !isDreamsView &&
    !isBotsView &&
    !isAgentsView &&
    !isTeamsView,
  );
  const composerPlaceholder =
    isActiveSendingThread || isDraftSendingThread || activeQueue.length > 0
      ? "Queue another follow-up for Garyx..."
      : preferredWorkspaceForNewThread
        ? "Describe what you want Garyx to build..."
        : "Choose a folder to start a Garyx thread.";
  const showAutomationRunInitialPlaceholder = Boolean(
    activePendingAutomationRun &&
    !activeMessages.length &&
    !activeHasAssistantOrToolMessage,
  );
  const showAutomationRunTailLoading = Boolean(
    (activePendingAutomationRun &&
      activeMessages.length > 0 &&
      !activeHasAssistantOrToolMessage) ||
      (activeRunLoading && !activeTailToolTraceBlockKey),
  );
  const activeToolTraceLoadingKey = threadActivity.runActive
    ? activeTailToolTraceBlockKey
    : null;
  useEffect(() => {
    if (contentView === "auto_research" && !showAutoResearchLab) {
      setContentView("thread");
    }
  }, [contentView, setContentView, showAutoResearchLab]);
  const showHistoryLoadingPlaceholder = Boolean(
    historyLoading &&
    !activeMessages.length &&
    !showAutomationRunInitialPlaceholder,
  );
  const conversationContextText = isAutomationView
    ? `${desktopState?.automations.length || 0} scheduled runs`
    : isSkillsView
      ? "Local and project skill registry"
    : isTasksView
        ? "Global task board"
      : isWorkflowView
        ? "Workflow run detail"
      : isAgentsView || isTeamsView
        ? "Agents and reusable teams"
        : isBotsView
          ? `${desktopState?.endpoints.length || 0} connected endpoints`
          : null;
  const memoryDialogDirty = memoryDialogDraft !== memoryDialogSavedContent;
  const remoteStateWarning = useMemo(
    () => summarizeRemoteStateErrors(desktopState?.remoteErrors),
    [desktopState?.remoteErrors],
  );

  useEffect(() => {
    if (!remoteStateWarning) {
      lastRemoteStateWarningKeyRef.current = null;
      return;
    }
    if (lastRemoteStateWarningKeyRef.current === remoteStateWarning.key) {
      return;
    }
    lastRemoteStateWarningKeyRef.current = remoteStateWarning.key;
    pushToast(remoteStateWarning.message, "error");
  }, [pushToast, remoteStateWarning]);

  function memoryDialogInput(target: MemoryDialogTarget) {
    if (target.scope === "agent") {
      return {
        scope: "agent" as const,
        agentId: target.agentId,
      };
    }
    if (target.scope === "automation") {
      return {
        scope: "automation" as const,
        automationId: target.automationId,
      };
    }
    const exhaustive: never = target;
    return exhaustive;
  }

  const confirmDiscardMemoryChanges = useCallback((): boolean => {
    if (!memoryDialogDirty) {
      return true;
    }
    return window.confirm("Discard unsaved memory changes?");
  }, [memoryDialogDirty]);

  const openMemoryDialog = useCallback(async (target: MemoryDialogTarget) => {
    if (memoryDialogTarget && !confirmDiscardMemoryChanges()) {
      return;
    }

    const requestId = memoryDialogRequestIdRef.current + 1;
    memoryDialogRequestIdRef.current = requestId;
    setMemoryDialogTarget(target);
    setMemoryDialogDocument(null);
    setMemoryDialogDraft("");
    setMemoryDialogSavedContent("");
    setMemoryDialogLoading(true);
    setMemoryDialogSaving(false);
    setMemoryDialogError(null);
    setMemoryDialogStatus(null);

    try {
      const document = await window.garyxDesktop.readMemoryDocument(
        memoryDialogInput(target),
      );
      if (memoryDialogRequestIdRef.current !== requestId) {
        return;
      }
      setMemoryDialogDocument(document);
      setMemoryDialogDraft(document.content);
      setMemoryDialogSavedContent(document.content);
    } catch (memoryError) {
      if (memoryDialogRequestIdRef.current !== requestId) {
        return;
      }
      setMemoryDialogError(
        memoryError instanceof Error
          ? memoryError.message
          : "Failed to load memory.md.",
      );
    } finally {
      if (memoryDialogRequestIdRef.current === requestId) {
        setMemoryDialogLoading(false);
      }
    }
  }, [confirmDiscardMemoryChanges, memoryDialogTarget]);

  function closeMemoryDialog() {
    if (!confirmDiscardMemoryChanges()) {
      return;
    }

    memoryDialogRequestIdRef.current += 1;
    setMemoryDialogTarget(null);
    setMemoryDialogDocument(null);
    setMemoryDialogDraft("");
    setMemoryDialogSavedContent("");
    setMemoryDialogLoading(false);
    setMemoryDialogSaving(false);
    setMemoryDialogError(null);
    setMemoryDialogStatus(null);
  }

  async function saveMemoryDialog() {
    if (!memoryDialogTarget) {
      return;
    }

    const requestId = memoryDialogRequestIdRef.current + 1;
    memoryDialogRequestIdRef.current = requestId;
    setMemoryDialogSaving(true);
    setMemoryDialogError(null);
    setMemoryDialogStatus(null);

    try {
      const document = await window.garyxDesktop.saveMemoryDocument({
        ...memoryDialogInput(memoryDialogTarget),
        content: memoryDialogDraft,
      });
      if (memoryDialogRequestIdRef.current !== requestId) {
        return;
      }
      setMemoryDialogDocument(document);
      setMemoryDialogDraft(document.content);
      setMemoryDialogSavedContent(document.content);
      setMemoryDialogStatus("Saved memory.md.");
    } catch (memoryError) {
      if (memoryDialogRequestIdRef.current !== requestId) {
        return;
      }
      setMemoryDialogError(
        memoryError instanceof Error
          ? memoryError.message
          : "Failed to save memory.md.",
      );
    } finally {
      if (memoryDialogRequestIdRef.current === requestId) {
        setMemoryDialogSaving(false);
      }
    }
  }

  const handleLocalFileLinkClick = useCallback((absolutePath: string) => {
    const memoryTarget = resolveMemoryDialogTargetFromPath(
      absolutePath,
      automations,
      desktopAgents,
    );
    if (memoryTarget) {
      void openMemoryDialog(memoryTarget);
      return;
    }
    handleLocalWorkspaceFileLinkClick(absolutePath);
  }, [
    automations,
    desktopAgents,
    handleLocalWorkspaceFileLinkClick,
    openMemoryDialog,
  ]);

  function openSettingsView() {
    setContentView("settings");
    if (!isLocalSettingsTab(settingsActiveTab)) {
      void refreshSettingsTabResources(settingsActiveTab);
    }
  }

  async function refreshDesktopState() {
    const [nextState, nextAgents, nextTeams, nextWorkflows] = await Promise.all([
      window.garyxDesktop.getState(),
      window.garyxDesktop
        .listCustomAgents()
        .catch(() => [] as DesktopCustomAgent[]),
      window.garyxDesktop.listTeams().catch(() => [] as DesktopTeam[]),
      window.garyxDesktop
        .listWorkflowDefinitions()
        .catch(() => [] as DesktopWorkflowDefinition[]),
    ]);
    startTransition(() => {
      setDesktopState(nextState);
      setDesktopAgents(nextAgents);
      setDesktopTeams(nextTeams);
      setDesktopWorkflows(nextWorkflows);
    });
    return nextState;
  }

  async function refreshAgentTargets() {
    const [nextAgents, nextTeams] = await Promise.all([
      window.garyxDesktop
        .listCustomAgents()
        .catch(() => [] as DesktopCustomAgent[]),
      window.garyxDesktop.listTeams().catch(() => [] as DesktopTeam[]),
    ]);
    startTransition(() => {
      setDesktopAgents(nextAgents);
      setDesktopTeams(nextTeams);
    });
  }

  async function openAddBotDialog() {
    setAddBotDialogOpen(true);
    void measureUiAction("bot.add_dialog.refresh_agent_targets", () =>
      refreshAgentTargets(),
    );
  }

  async function handleAddChannelAccount(input: {
    // `channel` can now be any plugin id (built-in or subprocess);
    // the main-process IPC decides which config slot to write.
    channel: "telegram" | "feishu" | "weixin" | string;
    accountId: string;
    name?: string | null;
    workspaceDir?: string | null;
    workspaceMode?: "local" | "worktree";
    agentId?: string | null;
    token?: string | null;
    appId?: string | null;
    appSecret?: string | null;
    baseUrl?: string | null;
    domain?: "feishu" | "lark" | null;
    /** Opaque plugin config for subprocess plugins. */
    config?: Record<string, unknown> | null;
  }) {
    await measureUiAction("bot.add_channel_account", async () => {
      const nextState = await window.garyxDesktop.addChannelAccount(input);
      startTransition(() => {
        setDesktopState(nextState);
      });
    });
    await measureUiAction("bot.add_channel_account.reload_settings", () =>
      loadGatewaySettings({ clearStatus: true }),
    );
    pushToast(t("Bot added."), "success");
  }

  async function handleStartWeixinChannelAuth(input: {
    accountId?: string | null;
    name?: string | null;
    workspaceDir?: string | null;
    baseUrl?: string | null;
  }) {
    return window.garyxDesktop.startWeixinChannelAuth(input);
  }

  async function handlePollWeixinChannelAuth(input: { sessionId: string }) {
    const result = await window.garyxDesktop.pollWeixinChannelAuth(input);
    if (result.status === "confirmed") {
      await refreshDesktopState();
      await loadGatewaySettings({ clearStatus: true });
      pushToast(t("Weixin bot connected."), "success");
    }
    return result;
  }

  async function handleStartFeishuChannelAuth(input: {
    accountId?: string | null;
    name?: string | null;
    workspaceDir?: string | null;
    domain?: "feishu" | "lark" | null;
  }) {
    return window.garyxDesktop.startFeishuChannelAuth(input);
  }

  async function handlePollFeishuChannelAuth(input: { sessionId: string }) {
    const result = await window.garyxDesktop.pollFeishuChannelAuth(input);
    if (result.status === "confirmed") {
      await refreshDesktopState();
      await loadGatewaySettings({ clearStatus: true });
      pushToast(t("Feishu bot connected."), "success");
    }
    return result;
  }

  async function waitForGatewayReadyForDeepLink(): Promise<void> {
    let lastError = "Gateway is still starting.";
    for (const delayMs of DEEP_LINK_GATEWAY_RETRY_DELAYS_MS) {
      if (delayMs > 0) {
        await waitForMs(delayMs);
      }
      try {
        const status = await window.garyxDesktop.checkConnection();
        if (status.ok) {
          setConnection(status);
          return;
        }
        lastError = status.error || lastError;
      } catch (connectionError) {
        lastError =
          connectionError instanceof Error
            ? connectionError.message
            : "Gateway is still starting.";
      }
    }
    throw new Error(lastError);
  }

  async function ensureThreadOpenable(threadId: string): Promise<boolean> {
    if (isKnownThreadId(desktopState, threadId)) {
      return true;
    }

    const refreshedState = await refreshDesktopState();
    if (isKnownThreadId(refreshedState, threadId)) {
      return true;
    }

    const transcript = await window.garyxDesktop.getThreadHistory(threadId);
    if (
      !transcript.remoteFound &&
      transcript.messages.length === 0 &&
      transcript.pendingInputs.length === 0 &&
      !transcript.threadInfo
    ) {
      return false;
    }

    applyRemoteTranscript(threadId, transcript);
    return true;
  }

  async function openThreadFromDeepLink(threadId: string): Promise<void> {
    if (!(await ensureThreadOpenable(threadId))) {
      throw new Error(`Thread not found for garyx:// link: ${threadId}`);
    }
    await openExistingThread(threadId);
  }

  function resetComposerAttachmentPicker() {
    if (composerAttachmentInputRef.current) {
      composerAttachmentInputRef.current.value = "";
    }
  }

  function clearComposerDraft() {
    composerDraftRef.current = "";
    setComposer("");
    setComposerTextPresent(false);
    setComposerResetKey((current) => current + 1);
    setComposerImages([]);
    setComposerFiles([]);
    resetComposerAttachmentPicker();
  }

  async function openExistingThread(
    threadId: string,
    entrySource: ThreadEntrySelectionSource | null = null,
  ): Promise<boolean> {
    setError(null);
    setContentView("thread");
    setNewThreadDraftActive(false);

    try {
      if (!(await ensureThreadOpenable(threadId))) {
        setError(`Thread not found: ${threadId}`);
        return false;
      }
    } catch (error) {
      setError(
        error instanceof Error
          ? error.message
          : `Failed to open thread: ${threadId}`,
      );
      return false;
    }

    setSelectedThreadId(threadId);
    setThreadEntrySelectionSource(entrySource);
    return true;
  }

  function openWorkflowTask(task: DesktopTaskSummary) {
    const taskId = task.taskId || `#TASK-${task.number}`;
    setError(null);
    setSelectedWorkflowTask(task);
    setSelectedWorkflowTaskId(taskId);
    setContentView("workflow");
  }

  const applyDesktopRoute = useCallback(
    async (route: DesktopRoute): Promise<void> => {
      switch (route.kind) {
        case "thread":
          await openExistingThread(route.threadId);
          return;
        case "new-thread":
          setError(null);
          setContentView("thread");
          setNewThreadDraftActive(true);
          setSelectedThreadId(null);
          setPendingWorkspacePath(route.workspacePath || null);
          setPendingWorkspaceMode("local");
          setPendingBotId(null);
          setPendingAgentId(route.agentId || "claude");
          setPendingWorkflowId(route.workflowId || null);
          clearComposerDraft();
          requestComposerFocus();
          return;
        case "automation":
          if (route.automationId) {
            await handleSelectAutomation(route.automationId);
          } else {
            setContentView("automation");
          }
          return;
        case "settings":
          setContentView("settings");
          if (route.tabId) {
            await handleSelectSettingsTab(route.tabId);
          }
          return;
        case "workflow-task":
          setError(null);
          setSelectedWorkflowTask(null);
          setSelectedWorkflowTaskId(route.taskId);
          setContentView("workflow");
          return;
        case "view":
          setContentView(route.view);
          return;
        case "thread-home":
          setContentView("thread");
          setNewThreadDraftActive(false);
          setPendingWorkspacePath(null);
          setPendingWorkspaceMode("local");
          setSelectedThreadId((current) =>
            isKnownThreadId(desktopState, current)
              ? current
              : desktopState?.threads[0]?.id || null,
          );
          return;
      }
    },
    [
      desktopState,
      handleSelectAutomation,
      handleSelectSettingsTab,
      openExistingThread,
      setContentView,
    ],
  );

  useEffect(() => {
    const handleRouteChange = () => {
      void applyDesktopRoute(parseDesktopRoute());
    };
    window.addEventListener("hashchange", handleRouteChange);
    window.addEventListener("popstate", handleRouteChange);
    return () => {
      window.removeEventListener("hashchange", handleRouteChange);
      window.removeEventListener("popstate", handleRouteChange);
    };
  }, [applyDesktopRoute]);

  useEffect(() => {
    if (loading) {
      return;
    }
    replaceDesktopRoute(
      currentDesktopRoute({
        contentView,
        newThreadDraftActive,
        pendingAgentId,
        pendingWorkflowId,
        pendingWorkspacePath,
        selectedAutomationId,
        selectedWorkflowTaskId,
        selectedThreadId,
        settingsActiveTab,
      }),
    );
  }, [
    contentView,
    loading,
    newThreadDraftActive,
    pendingAgentId,
    pendingWorkflowId,
    pendingWorkspacePath,
    selectedAutomationId,
    selectedWorkflowTaskId,
    selectedThreadId,
    settingsActiveTab,
  ]);

  function requestComposerFocus() {
    shouldFocusComposerRef.current = true;
  }

  function removeComposerImage(imageId: string) {
    setComposerImages((current) =>
      current.filter((image) => image.id !== imageId),
    );
  }

  function removeComposerFile(fileId: string) {
    setComposerFiles((current) => current.filter((file) => file.id !== fileId));
  }

  async function appendComposerAttachments(files: File[]) {
    if (!files.length) {
      return;
    }

    setComposerAttachmentUploadPending(true);
    try {
      const prepared = await prepareAttachmentUploads(files);
      if (!prepared.length) {
        setError("No attachments could be loaded.");
        return;
      }
      const uploaded = await window.garyxDesktop.uploadChatAttachments({
        files: prepared.map((file) => ({
          kind: file.kind,
          name: file.name,
          mediaType: file.mediaType,
          dataBase64: file.dataBase64,
        })),
      });
      if (uploaded.files.length !== prepared.length) {
        throw new Error("Gateway returned an incomplete attachment upload result.");
      }

      const nextImages: MessageImageAttachment[] = [];
      const nextFiles: MessageFileAttachment[] = [];
      prepared.forEach((file, index) => {
        const stored = uploaded.files[index];
        if (!stored?.path?.trim()) {
          return;
        }
        if (file.kind === "image") {
          nextImages.push({
            id: file.id,
            name: stored.name,
            mediaType: stored.mediaType || file.mediaType,
            path: stored.path,
            data: file.dataBase64,
          });
          return;
        }
        nextFiles.push({
          id: file.id,
          name: stored.name,
          mediaType: stored.mediaType || file.mediaType,
          path: stored.path,
        });
      });

      if (!nextImages.length && !nextFiles.length) {
        throw new Error("Gateway did not return any uploaded attachments.");
      }
      if (nextImages.length) {
        setComposerImages((current) => [...current, ...nextImages]);
      }
      if (nextFiles.length) {
        setComposerFiles((current) => [...current, ...nextFiles]);
      }
      setError(null);
    } catch (attachmentError) {
      setError(
        attachmentError instanceof Error
          ? attachmentError.message
          : "Failed to load attachment",
      );
    } finally {
      setComposerAttachmentUploadPending(false);
      resetComposerAttachmentPicker();
    }
  }

  useEffect(() => {
    messageStateRef.current = messageState;
  }, [messageState]);

  useEffect(() => {
    return () => {
      if (messagesStickScrollFrameRef.current !== null) {
        window.cancelAnimationFrame(messagesStickScrollFrameRef.current);
        messagesStickScrollFrameRef.current = null;
      }
      if (clientLogFlushFrameRef.current !== null) {
        window.cancelAnimationFrame(clientLogFlushFrameRef.current);
        clientLogFlushFrameRef.current = null;
      }
      if (assistantDeltaFlushFrameRef.current !== null) {
        window.cancelAnimationFrame(assistantDeltaFlushFrameRef.current);
        assistantDeltaFlushFrameRef.current = null;
      }
    };
  }, []);

  useEffect(() => {
    threadLogsPanelWidthRef.current = threadLogsPanelWidth;
  }, [threadLogsPanelWidth]);

  useEffect(() => {
    const nextWidth = clampThreadLogsPanelWidth(
      desktopState?.settings.threadLogsPanelWidth ??
        DEFAULT_DESKTOP_SETTINGS.threadLogsPanelWidth,
      currentThreadLayoutWidth(),
    );
    setThreadLogsPanelWidth(nextWidth);
    setSettingsDraft((current) => {
      if (current.threadLogsPanelWidth === nextWidth) {
        return current;
      }
      return {
        ...current,
        threadLogsPanelWidth: nextWidth,
      };
    });
  }, [desktopState?.settings.threadLogsPanelWidth]);

  useEffect(() => {
    streamEventHandlerRef.current = handleChatStreamEvent;
  });

  useEffect(() => {
    selectedThreadIdRef.current = selectedThreadId;
  }, [selectedThreadId]);

  useEffect(() => {
    newThreadDraftActiveRef.current = newThreadDraftActive;
  }, [newThreadDraftActive]);

  useEffect(() => {
    pendingWorkspacePathRef.current = pendingWorkspacePath;
  }, [pendingWorkspacePath]);

  useEffect(() => {
    pendingWorkspaceModeRef.current = pendingWorkspaceMode;
  }, [pendingWorkspaceMode]);

  useEffect(() => {
    pendingBotIdRef.current = pendingBotId;
  }, [pendingBotId]);

  useEffect(() => {
    threadLogsOpenRef.current = threadLogsOpen;
  }, [threadLogsOpen]);

  useEffect(() => {
    threadLogsActiveTabRef.current = threadLogsActiveTab;
  }, [threadLogsActiveTab]);

  useEffect(() => {
    const listener = (event: DesktopChatStreamEvent) => {
      enqueueClientLogEvent(event);
      streamEventHandlerRef.current(event);
    };
    window.garyxDesktop.subscribeChatStream(listener);
    return () => {
      window.garyxDesktop.unsubscribeChatStream(listener);
    };
  }, []);

  useEffect(() => {
    const listener = (event: DesktopDeepLinkEvent) => {
      deepLinkEventHandlerRef.current(event);
    };
    window.garyxDesktop.subscribeDeepLinks(listener);
    return () => {
      window.garyxDesktop.unsubscribeDeepLinks(listener);
    };
  }, []);

  useEffect(() => {
    const handlePointerDown = (event: MouseEvent) => {
      const target = event.target;
      if (!(target instanceof Element)) {
        return;
      }
      if (target.closest(".workspace-actions")) {
        return;
      }
      setWorkspaceMenuOpenPath(null);
    };
    window.addEventListener("pointerdown", handlePointerDown);
    return () => {
      window.removeEventListener("pointerdown", handlePointerDown);
    };
  }, []);

  useEffect(() => {
    const handleResize = () => {
      const nextWidth = clampThreadLogsPanelWidth(
        threadLogsPanelWidthRef.current,
        currentThreadLayoutWidth(),
      );
      if (nextWidth !== threadLogsPanelWidthRef.current) {
        setThreadLogsPanelWidth(nextWidth);
        setSettingsDraft((current) => ({
          ...current,
          threadLogsPanelWidth: nextWidth,
        }));
      }
    };
    window.addEventListener("resize", handleResize);
    return () => {
      window.removeEventListener("resize", handleResize);
    };
  }, []);

  useEffect(() => {
    if (!sidebarResizing) {
      return;
    }
    const handlePointerMove = (event: PointerEvent) => {
      const state = sidebarResizeStateRef.current;
      if (!state) return;
      const next = Math.max(
        245,
        Math.min(520, state.startWidth + (event.clientX - state.startX)),
      );
      setSidebarWidth(next);
    };
    const finishResize = () => {
      sidebarResizeStateRef.current = null;
      setSidebarResizing(false);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", finishResize);
    window.addEventListener("pointercancel", finishResize);
    return () => {
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", finishResize);
      window.removeEventListener("pointercancel", finishResize);
    };
  }, [sidebarResizing]);

  useEffect(() => {
    if (!railResizing) {
      return;
    }
    let lastNext = railResizeStateRef.current?.startWidth ?? railWidth;
    let rafId: number | null = null;
    const flush = () => {
      rafId = null;
      document.documentElement.style.setProperty(
        "--spacing-token-rail",
        `${lastNext}px`,
      );
    };
    const handlePointerMove = (event: PointerEvent) => {
      const state = railResizeStateRef.current;
      if (!state) return;
      lastNext = Math.max(
        220,
        Math.min(420, state.startWidth + (event.clientX - state.startX)),
      );
      if (rafId === null) {
        rafId = requestAnimationFrame(flush);
      }
    };
    const finishResize = () => {
      if (rafId !== null) {
        cancelAnimationFrame(rafId);
        rafId = null;
      }
      railResizeStateRef.current = null;
      setRailResizing(false);
      setRailWidth(lastNext);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };
    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", finishResize);
    window.addEventListener("pointercancel", finishResize);
    return () => {
      if (rafId !== null) {
        cancelAnimationFrame(rafId);
      }
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", finishResize);
      window.removeEventListener("pointercancel", finishResize);
    };
  }, [railResizing]);

  useEffect(() => {
    if (!threadLogsResizing) {
      return;
    }

    const handlePointerMove = (event: PointerEvent) => {
      const resizeState = threadLogsResizeStateRef.current;
      if (!resizeState) {
        return;
      }
      const nextWidth = clampThreadLogsPanelWidth(
        resizeState.startWidth + (resizeState.startX - event.clientX),
        currentThreadLayoutWidth(),
      );
      setThreadLogsPanelWidth(nextWidth);
      setSettingsDraft((current) => ({
        ...current,
        threadLogsPanelWidth: nextWidth,
      }));
    };

    const finishResize = () => {
      const nextWidth = threadLogsPanelWidthRef.current;
      threadLogsResizeStateRef.current = null;
      setThreadLogsResizing(false);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      void persistThreadLogsPanelWidth(nextWidth);
    };

    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", finishResize);
    window.addEventListener("pointercancel", finishResize);
    return () => {
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", finishResize);
      window.removeEventListener("pointercancel", finishResize);
    };
  }, [threadLogsResizing, desktopState?.settings.threadLogsPanelWidth]);

  useEffect(() => {
    syncComposerPhase(composer, isComposingRef.current);
  }, [composer, composerFiles.length, composerImages.length, composerLocked]);

  useEffect(() => {
    if (!/^\/[a-z0-9_]*$/i.test(composer)) {
      return;
    }
    if (commandsLoaded || commandsLoading) {
      return;
    }
    void loadSlashCommands();
  }, [commandsLoaded, commandsLoading, composer]);

  useEffect(() => {
    const workspacePaths = new Set(
      (desktopState?.workspaces || [])
        .map((workspace) => workspace.path)
        .filter((path): path is string => Boolean(path)),
    );
    if (pendingWorkspacePath && !workspacePaths.has(pendingWorkspacePath)) {
      setPendingWorkspacePath(null);
    }
    if (workspaceMenuOpenPath && !workspacePaths.has(workspaceMenuOpenPath)) {
      setWorkspaceMenuOpenPath(null);
    }
  }, [
    desktopState,
    pendingWorkspacePath,
    workspaceMenuOpenPath,
  ]);

  useEffect(() => {
    if (selectedThreadId && pendingWorkspacePath) {
      setPendingWorkspacePath(null);
    }
  }, [pendingWorkspacePath, selectedThreadId]);

  useEffect(() => {
    if (!shouldFocusComposerRef.current) {
      return;
    }
    if (contentView !== "thread") {
      return;
    }
    if (!selectedThreadId && !preferredWorkspaceForNewThread?.available) {
      return;
    }
    const textarea = composerTextareaRef.current;
    if (!textarea) {
      return;
    }
    shouldFocusComposerRef.current = false;
    const focusFrame = window.requestAnimationFrame(() => {
      textarea.focus();
      const cursor = textarea.value.length;
      textarea.setSelectionRange(cursor, cursor);
    });
    return () => {
      window.cancelAnimationFrame(focusFrame);
    };
  }, [
    composerLocked,
    contentView,
    preferredWorkspaceForNewThread?.available,
    selectedThreadId,
  ]);

  useEffect(() => {
    let cancelled = false;

    void (async () => {
      setLoading(true);
      setError(null);
      try {
        let state: DesktopState | null = null;

        for (const delayMs of STARTUP_HYDRATION_RETRY_DELAYS_MS) {
          if (delayMs > 0) {
            await waitForMs(delayMs);
          }
          if (cancelled) {
            return;
          }

          const [nextState, nextStatus, nextAgents, nextTeams, nextWorkflows] =
            await Promise.all([
              window.garyxDesktop.getState(),
              window.garyxDesktop.checkConnection(),
              window.garyxDesktop
                .listCustomAgents()
                .catch(() => [] as DesktopCustomAgent[]),
              window.garyxDesktop.listTeams().catch(() => [] as DesktopTeam[]),
              window.garyxDesktop
                .listWorkflowDefinitions()
                .catch(() => [] as DesktopWorkflowDefinition[]),
            ]);
          if (cancelled) {
            return;
          }

          state = nextState;

          startTransition(() => {
            setDesktopState(nextState);
            setDesktopAgents(nextAgents);
            setDesktopTeams(nextTeams);
            setDesktopWorkflows(nextWorkflows);
            setSettingsDraft(nextState.settings);
            setConnection(nextStatus);
          });

          if (!shouldRetryStartupHydration(nextState, nextStatus)) {
            break;
          }
        }

        if (!state) {
          throw new Error("Failed to load desktop state");
        }

        let hydratedState = state;
        const startupRoute = initialRouteRef.current || { kind: "thread-home" };
        if (startupRoute.kind === "automation" && startupRoute.automationId) {
          try {
            hydratedState = await window.garyxDesktop.selectAutomation({
              automationId: startupRoute.automationId,
            });
            if (cancelled) {
              return;
            }
            setDesktopState(hydratedState);
          } catch (automationRouteError) {
            if (!cancelled) {
              setError(
                automationRouteError instanceof Error
                  ? automationRouteError.message
                  : `Automation not found: ${startupRoute.automationId}`,
              );
            }
          }
        }

        if (startupRoute.kind === "thread") {
          if (isKnownThreadId(hydratedState, startupRoute.threadId)) {
            setSelectedThreadId(startupRoute.threadId);
          } else {
            setError(`Thread not found: ${startupRoute.threadId}`);
            setSelectedThreadId(hydratedState.threads[0]?.id || null);
          }
        } else if (startupRoute.kind === "new-thread") {
          setContentView("thread");
          setNewThreadDraftActive(true);
          setSelectedThreadId(null);
          setPendingWorkspacePath(startupRoute.workspacePath || null);
          setPendingWorkspaceMode("local");
          setPendingAgentId(startupRoute.agentId || "claude");
          setPendingWorkflowId(startupRoute.workflowId || null);
        } else {
          setSelectedThreadId((current) =>
            isKnownThreadId(hydratedState, current)
              ? current
              : hydratedState.threads[0]?.id || null,
          );
        }
        await loadGatewaySettings();
      } catch (bootstrapError) {
        if (!cancelled) {
          setError(
            bootstrapError instanceof Error
              ? bootstrapError.message
              : "Failed to load desktop state",
          );
        }
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    let timeoutId = 0;

    const pollConnection = async () => {
      let nextOk = false;
      try {
        const status = await window.garyxDesktop.checkConnection();
        if (cancelled) {
          return;
        }
        nextOk = Boolean(status.ok);
        setConnection(status);
      } catch {
        if (cancelled) {
          return;
        }
        nextOk = false;
        setConnection({
          ok: false,
          bridgeReady: false,
          gatewayUrl: settingsDraft.gatewayUrl,
          error: "Unable to reach Garyx gateway",
        });
      } finally {
        if (cancelled) {
          return;
        }
        if (nextOk) {
          gatewayRetryStepRef.current = 0;
        } else {
          gatewayRetryStepRef.current = Math.min(
            gatewayRetryStepRef.current + 1,
            GATEWAY_RETRY_BACKOFF_MS.length - 1,
          );
        }
        timeoutId = window.setTimeout(
          pollConnection,
          nextOk
            ? GATEWAY_HEALTHY_POLL_MS
            : GATEWAY_RETRY_BACKOFF_MS[gatewayRetryStepRef.current],
        );
      }
    };

    timeoutId = window.setTimeout(
      pollConnection,
      connection?.ok
        ? GATEWAY_HEALTHY_POLL_MS
        : GATEWAY_RETRY_BACKOFF_MS[gatewayRetryStepRef.current],
    );

    return () => {
      cancelled = true;
      window.clearTimeout(timeoutId);
    };
  }, [connection?.ok, settingsDraft.gatewayUrl]);

  useEffect(() => {
    if (!connection?.ok || loading) {
      return;
    }

    let cancelled = false;
    let refreshing = false;

    const refreshSilently = async () => {
      if (cancelled || document.hidden || refreshing) {
        return;
      }

      refreshing = true;
      try {
        await refreshDesktopState();
      } catch (refreshError) {
        console.debug("Silent desktop state refresh failed.", refreshError);
      } finally {
        refreshing = false;
      }
    };

    const timer = window.setInterval(() => {
      void refreshSilently();
    }, SILENT_DESKTOP_STATE_REFRESH_MS);

    const handleVisibilityChange = () => {
      if (!document.hidden) {
        void refreshSilently();
      }
    };

    document.addEventListener("visibilitychange", handleVisibilityChange);

    return () => {
      cancelled = true;
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, [connection?.ok, loading]);

  useEffect(() => {
    const previousOk = previousConnectionOkRef.current;
    previousConnectionOkRef.current = connection?.ok ?? null;
    if (!connection?.ok || previousOk !== false) {
      return;
    }

    const threadsToRecover = recoveryThreadIds();
    if (!threadsToRecover.length) {
      void refreshDesktopState().catch(() => null);
      return;
    }

    void (async () => {
      try {
        await refreshDesktopState();
      } catch {
        // Best-effort reconnect recovery; history refresh below can still reconcile transcript state.
      }
      for (const threadId of threadsToRecover) {
        scheduleHistoryRefresh(threadId, 6, 350, true);
      }
    })();
  }, [connection?.ok, selectedThreadId]);

  useEffect(() => {
    if (!desktopState) {
      return;
    }

    if (hasNewThreadDraft) {
      return;
    }

    if (isKnownThreadId(desktopState, selectedThreadId)) {
      return;
    }

    const nextSelected = desktopState.threads[0]?.id || null;
    if (nextSelected !== selectedThreadId) {
      setSelectedThreadId(nextSelected);
    }
  }, [desktopState, hasNewThreadDraft, selectedThreadId]);

  useEffect(() => {
    if (newThreadDraftActive && selectedThreadId) {
      setSelectedThreadId(null);
    }
  }, [newThreadDraftActive, selectedThreadId]);

  useEffect(() => {
    setEditingThreadTitle(false);
  }, [contentView, selectedThreadId]);

  useEffect(() => {
    if (!canEditThreadTitle && editingThreadTitle) {
      setEditingThreadTitle(false);
    }
  }, [canEditThreadTitle, editingThreadTitle]);

  useEffect(() => {
    if (!selectedThreadId || !desktopState) {
      return;
    }

    if (!editingThreadTitle) {
      setTitleDraft(activeThread?.title || DEFAULT_SESSION_TITLE);
    }

    void loadThreadHistory({
      api: getDesktopApi(),
      threadId: selectedThreadId,
      onBeforeLoad: (threadId) => {
        requestMessagesBottomSnap(threadId);
      },
      onTranscript: applyRemoteTranscript,
      onAutomationResponseDetected: (threadId) => {
        setPendingAutomationRun(threadId, null);
      },
      hasAutomationResponse: transcriptHasAutomationResponse,
      setHistoryLoading,
      setError,
    });
  }, [desktopState, editingThreadTitle, selectedThreadId, activeThread?.title]);

  useEffect(() => {
    if (contentView !== "thread" || !selectedThreadId) {
      return;
    }

    let cancelled = false;
    let polling = false;

    const pollSelectedThreadHistory = async () => {
      if (cancelled || document.hidden || polling) {
        return;
      }

      const liveStream = liveStreamStateRef.current[selectedThreadId] || null;
      if (
        liveStream &&
        ["connecting", "streaming", "reconciling"].includes(
          liveStream.streamStatus,
        )
      ) {
        return;
      }

      polling = true;
      try {
        const transcript =
          await window.garyxDesktop.getThreadHistory(selectedThreadId);
        const nextSignature = threadActivitySignature(
          transcript.messages,
          transcript.pendingInputs,
          transcript.threadInfo,
        );
        if (
          nextSignature ===
          remoteTranscriptSignatureByThreadRef.current[selectedThreadId]
        ) {
          return;
        }
        requestMessagesBottomSnap(selectedThreadId);
        applyRemoteTranscript(selectedThreadId, transcript);
      } catch {
        // Best-effort reconcile loop for passive inbound messages.
      } finally {
        polling = false;
      }
    };

    const timer = window.setInterval(() => {
      void pollSelectedThreadHistory();
    }, 1500);

    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [contentView, selectedThreadId]);

  useLayoutEffect(() => {
    const currentThreadId = activeThreadMessageKey;
    const currentCount = activeMessages.length;
    const currentTailSignature = messageTailSignature(activeMessages);
    const prependAnchor = pendingMessagesPrependAnchorRef.current;
    if (prependAnchor) {
      if (prependAnchor.threadId === currentThreadId) {
        const node = messagesRef.current;
        if (node) {
          node.scrollTop =
            node.scrollHeight -
            prependAnchor.scrollHeight +
            prependAnchor.scrollTop;
          shouldStickMessagesToBottomRef.current = false;
        }
      }
      pendingMessagesPrependAnchorRef.current = null;
      lastRenderedMessageThreadRef.current = currentThreadId;
      lastRenderedMessageCountRef.current = currentCount;
      lastRenderedMessageTailSignatureRef.current = currentTailSignature;
      return;
    }
    const previousThreadId = lastRenderedMessageThreadRef.current;
    const previousTailSignature = lastRenderedMessageTailSignatureRef.current;
    const threadChanged = currentThreadId !== previousThreadId;
    const tailChanged = currentTailSignature !== previousTailSignature;
    const pendingSnapMatches =
      pendingThreadBottomSnapRef.current === currentThreadId;
    const forceSnap =
      pendingSnapMatches && forceMessagesBottomSnapRef.current;
    const shouldSnapToBottom = Boolean(
      currentThreadId &&
      currentCount > 0 &&
      !historyLoading &&
      (threadChanged ||
        forceSnap ||
        (pendingSnapMatches && shouldStickMessagesToBottomRef.current)),
    );

    if (shouldSnapToBottom) {
      scheduleMessagesScrollToLatest("auto");
      pendingThreadBottomSnapRef.current = null;
      forceMessagesBottomSnapRef.current = false;
      if (threadChanged || forceSnap) {
        shouldStickMessagesToBottomRef.current = true;
      }
    } else if (
      currentThreadId &&
      !historyLoading &&
      tailChanged &&
      shouldStickMessagesToBottomRef.current
    ) {
      scheduleMessagesScrollToLatest("auto");
    } else if (pendingSnapMatches && currentCount > 0 && !historyLoading) {
      pendingThreadBottomSnapRef.current = null;
      forceMessagesBottomSnapRef.current = false;
    }

    lastRenderedMessageThreadRef.current = currentThreadId;
    lastRenderedMessageCountRef.current = currentCount;
    lastRenderedMessageTailSignatureRef.current = currentTailSignature;
  }, [activeThreadMessageKey, activeMessages, historyLoading]);

  useEffect(() => {
    if (
      !activeThreadMessageKey ||
      historyLoading ||
      !activeHistoryPagination?.hasMoreBefore ||
      activeHistoryPagination.loadingBefore
    ) {
      return;
    }

    const node = messagesRef.current;
    if (!messagesNearEarlierUserTurnBoundary(node)) {
      return;
    }

    const threadId = activeThreadMessageKey;
    const timer = window.setTimeout(() => {
      if (selectedThreadIdRef.current === threadId) {
        void loadOlderThreadHistoryPage(threadId);
      }
    }, 0);

    return () => {
      window.clearTimeout(timer);
    };
  }, [
    activeThreadMessageKey,
    activeMessages.length,
    activeHistoryPagination?.hasMoreBefore,
    activeHistoryPagination?.loadingBefore,
    activeHistoryPagination?.nextBeforeIndex,
    historyLoading,
  ]);

  useEffect(() => {
    const node = messagesRef.current;
    if (!node || !activeThreadMessageKey) {
      return;
    }

    const scrollIfSticky = () => {
      if (shouldStickMessagesToBottomRef.current) {
        scheduleMessagesScrollToLatest("auto");
      }
    };
    const resizeObserver =
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver(scrollIfSticky);
    const observedChildren = new Set<Element>();

    const syncObservedChildren = () => {
      if (!resizeObserver) {
        return;
      }
      resizeObserver.observe(node);
      for (const child of Array.from(node.children)) {
        if (observedChildren.has(child)) {
          continue;
        }
        observedChildren.add(child);
        resizeObserver.observe(child);
      }
      for (const child of Array.from(observedChildren)) {
        if (child.parentElement !== node) {
          observedChildren.delete(child);
          resizeObserver.unobserve(child);
        }
      }
    };

    syncObservedChildren();
    const mutationObserver = new MutationObserver(() => {
      syncObservedChildren();
      scrollIfSticky();
    });
    mutationObserver.observe(node, { childList: true });

    return () => {
      mutationObserver.disconnect();
      resizeObserver?.disconnect();
    };
  }, [activeThreadMessageKey]);

  useEffect(() => {
    threadLogsCursorRef.current = threadLogsCursor;
  }, [threadLogsCursor]);

  useEffect(() => {
    if (activeThreadMessageKey == null) {
      pendingThreadBottomSnapRef.current = null;
      forceMessagesBottomSnapRef.current = false;
      return;
    }
    requestMessagesBottomSnap(activeThreadMessageKey, true);
  }, [activeThreadMessageKey]);

  useEffect(() => {
    if (!inspectorOpen && !threadLogsOpen) {
      return;
    }

    function handleKeydown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        if (threadLogsOpen) {
          setThreadLogsOpen(false);
          return;
        }
        setInspectorOpen(false);
      }
    }

    window.addEventListener("keydown", handleKeydown);
    return () => {
      window.removeEventListener("keydown", handleKeydown);
    };
  }, [inspectorOpen, threadLogsOpen]);

  useEffect(() => {
    if (!editingThreadTitle) {
      return;
    }
    const node = threadTitleInputRef.current;
    if (!node) {
      return;
    }
    node.focus();
    node.select();
  }, [editingThreadTitle]);

  useEffect(() => {
    if (contentView !== "thread") {
      setInspectorOpen(false);
      setThreadLogsOpen(false);
    }
  }, [contentView]);

  useEffect(() => {
    if (!activeWorkspacePath) {
      setInspectorOpen(false);
      return;
    }

    setExpandedWorkspaceDirectories((current) => ({
      ...current,
      [workspaceDirectoryKey(activeWorkspacePath, "")]: true,
    }));
  }, [activeWorkspacePath]);

  useEffect(() => {
    if (!inspectorOpen || contentView !== "thread" || !activeWorkspacePath) {
      return;
    }
    if (
      activeWorkspaceDirectoryState?.loaded ||
      activeWorkspaceDirectoryState?.loading
    ) {
      return;
    }
    void loadWorkspaceDirectory(activeWorkspacePath, "");
  }, [
    activeWorkspaceDirectoryState?.loaded,
    activeWorkspaceDirectoryState?.loading,
    activeWorkspacePath,
    contentView,
    inspectorOpen,
  ]);

  useEffect(() => {
    if (threadLogsOpen && !selectedThreadId) {
      setThreadLogsOpen(false);
    }
  }, [selectedThreadId, threadLogsOpen]);

  useEffect(() => {
    if (!selectedThreadId) {
      setClientLogsHasUnread(false);
    }
  }, [selectedThreadId]);

  useEffect(() => {
    if (!threadLogsOpen || contentView !== "thread" || !selectedThreadId) {
      return;
    }

    let cancelled = false;
    let polling = false;

    setThreadLogsLoading(true);
    setThreadLogsError(null);
    setThreadLogsHasUnread(false);
    setThreadLogsText("");
    setThreadLogsPath("");
    setThreadLogsCursor(0);
    threadLogsCursorRef.current = 0;

    const loadLogs = async (cursor?: number) => {
      if (cancelled || polling) {
        return;
      }
      polling = true;
      const wasNearBottom = threadLogsNearBottom();
      try {
        const chunk = await window.garyxDesktop.getThreadLogs(
          selectedThreadId,
          cursor,
        );
        if (cancelled) {
          return;
        }
        setThreadLogsPath(chunk.path);
        setThreadLogsCursor(chunk.cursor);
        threadLogsCursorRef.current = chunk.cursor;
        setThreadLogsError(null);
        setThreadLogsLoading(false);
        if (chunk.reset) {
          setThreadLogsText(keepRecentThreadLogLines(chunk.text));
          setThreadLogsHasUnread(false);
          window.requestAnimationFrame(() => {
            scrollThreadLogsToLatest("auto");
          });
          return;
        }
        if (!chunk.text) {
          return;
        }
        setThreadLogsText((current) =>
          keepRecentThreadLogLines(current + chunk.text),
        );
        if (wasNearBottom) {
          setThreadLogsHasUnread(false);
          window.requestAnimationFrame(() => {
            scrollThreadLogsToLatest("auto");
          });
        } else {
          setThreadLogsHasUnread(true);
        }
      } catch (loadError) {
        if (!cancelled) {
          setThreadLogsLoading(false);
          setThreadLogsError(
            loadError instanceof Error
              ? loadError.message
              : "Failed to load thread logs",
          );
        }
      } finally {
        polling = false;
      }
    };

    void loadLogs();
    const timer = window.setInterval(() => {
      if (document.hidden) {
        return;
      }
      void loadLogs(threadLogsCursorRef.current);
    }, 1000);

    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [contentView, selectedThreadId, threadLogsOpen]);

  useEffect(() => {
    if (
      !threadLogsOpen ||
      !selectedThreadId
    ) {
      return;
    }
    if (threadLogsActiveTab === "client") {
      setClientLogsHasUnread(false);
    } else {
      setThreadLogsHasUnread(false);
    }
    window.requestAnimationFrame(() => {
      scrollThreadLogsToLatest("auto");
    });
  }, [selectedThreadId, threadLogsActiveTab, threadLogsOpen]);

  function syncComposerPhase(
    nextText: string,
    isComposing = isComposingRef.current,
  ) {
    const hasText =
      nextText.trim().length > 0 ||
      composerImages.length > 0 ||
      composerFiles.length > 0;
    const syncKey = `${hasText}:${isComposing}:${composerLocked}`;
    if (composerPhaseSyncKeyRef.current === syncKey) {
      return;
    }
    composerPhaseSyncKeyRef.current = syncKey;
    dispatchMessageState({
      type: "composer/sync",
      hasText,
      isComposing,
      locked: composerLocked,
    });
  }

  function queueIntentIdsForThread(threadId: string): string[] {
    return selectQueueIntentIds(messageStateRef.current, threadId);
  }

  function intentForId(intentId: string): MessageIntent | null {
    return messageStateRef.current.intentsById[intentId] || null;
  }

  function setThreadRuntimeState(
    threadId: string,
    runtimeState: ThreadRuntimeState,
    options?: {
      activeIntentId?: string;
      remoteRunId?: string;
      error?: string;
    },
  ) {
    dispatchMessageState({
      type: "thread/runtime",
      threadId,
      runtimeState,
      activeIntentId: options?.activeIntentId,
      remoteRunId: options?.remoteRunId,
      error: options?.error,
    });
  }

  function updateLiveStreamState(
    threadId: string,
    updater: (current: LiveStreamState | null) => LiveStreamState | null,
  ): LiveStreamState | null {
    const next = updater(liveStreamStateRef.current[threadId] || null);
    const updated = { ...liveStreamStateRef.current };
    if (next) {
      updated[threadId] = next;
    } else {
      delete updated[threadId];
    }
    liveStreamStateRef.current = updated;
    setLiveStreamStateByThread(updated);
    return next;
  }

  function clearLiveStreamState(threadId: string) {
    updateLiveStreamState(threadId, () => null);
  }

  function getLiveStreamState(threadId: string): LiveStreamState | null {
    return liveStreamStateRef.current[threadId] || null;
  }

  function updateMessagesByThread(
    updater: (current: MessageMap) => MessageMap,
  ): MessageMap {
    const next = updater(messagesByThreadRef.current);
    messagesByThreadRef.current = next;
    setMessagesByThread(next);
    return next;
  }

  function flushPendingClientLogEvents() {
    clientLogFlushFrameRef.current = null;
    const events = pendingClientLogEventsRef.current;
    if (!events.length) {
      return;
    }
    pendingClientLogEventsRef.current = [];

    setClientLogsByThread((current) => {
      let next = current;
      for (const event of events) {
        const key = `client-log-line-${clientLogSequenceRef.current}`;
        clientLogSequenceRef.current += 1;
        const nextEntry = buildClientStreamLogEntry(event, key);
        const existing = next[event.threadId] || [];
        const nextEntries = appendClientStreamLogEntry(
          existing,
          nextEntry,
          MAX_CLIENT_STREAM_LOG_ENTRIES,
        );
        if (nextEntries === existing) {
          continue;
        }
        next = {
          ...next,
          [event.threadId]: nextEntries,
        };
      }
      return next;
    });

    const selectedThread = selectedThreadIdRef.current;
    if (!selectedThread || !events.some((event) => event.threadId === selectedThread)) {
      return;
    }
    const shouldAutoScroll =
      threadLogsOpenRef.current &&
      threadLogsActiveTabRef.current === "client" &&
      threadLogsNearBottom();
    if (shouldAutoScroll) {
      setClientLogsHasUnread(false);
      window.requestAnimationFrame(() => {
        scrollThreadLogsToLatest("auto");
      });
    } else {
      setClientLogsHasUnread(true);
    }
  }

  function enqueueClientLogEvent(event: DesktopChatStreamEvent) {
    pendingClientLogEventsRef.current.push(event);
    if (clientLogFlushFrameRef.current !== null) {
      return;
    }
    clientLogFlushFrameRef.current = window.requestAnimationFrame(
      flushPendingClientLogEvents,
    );
  }

  function flushPendingAssistantDelta(
    threadId?: string,
  ): LiveStreamState | null {
    const pendingByThread = pendingAssistantDeltaByThreadRef.current;
    const pendingDeltas = threadId
      ? pendingByThread[threadId]
        ? [pendingByThread[threadId]]
        : []
      : Object.values(pendingByThread);
    if (!pendingDeltas.length) {
      return threadId ? getLiveStreamState(threadId) : null;
    }

    let latestState: LiveStreamState | null = null;
    for (const pending of pendingDeltas) {
      delete pendingByThread[pending.threadId];
      const assistantEntryId = applyStreamingAssistantDelta(
        pending.threadId,
        pending.intentId,
        pending.runId,
        pending.text,
        pending.metadata,
      );
      latestState = updateLiveStreamState(pending.threadId, (current) => ({
        threadId: pending.threadId,
        runId: pending.runId,
        activeIntentId: current?.activeIntentId || pending.intentId,
        assistantEntryId,
        pendingAckIntentIds: current?.pendingAckIntentIds || [],
        streamStatus: "streaming",
      }));
      setThreadRuntimeState(pending.threadId, "running_remote", {
        activeIntentId: pending.intentId,
        remoteRunId: pending.runId,
      });
    }
    return threadId ? getLiveStreamState(threadId) : latestState;
  }

  function schedulePendingAssistantDeltaFlush() {
    if (assistantDeltaFlushFrameRef.current !== null) {
      return;
    }
    assistantDeltaFlushFrameRef.current = window.requestAnimationFrame(() => {
      assistantDeltaFlushFrameRef.current = null;
      flushPendingAssistantDelta();
    });
  }

  function enqueueStreamingAssistantDelta(
    threadId: string,
    intentId: string | undefined,
    runId: string,
    delta: string,
    metadata?: Record<string, unknown> | null,
  ) {
    const pending = pendingAssistantDeltaByThreadRef.current[threadId];
    const nextSpeakerKey = speakerIdentityKey(metadata);
    const pendingSpeakerKey = speakerIdentityKey(pending?.metadata);
    if (
      pending &&
      (pending.runId !== runId ||
        pending.intentId !== intentId ||
        pendingSpeakerKey !== nextSpeakerKey)
    ) {
      flushPendingAssistantDelta(threadId);
    }

    const current = pendingAssistantDeltaByThreadRef.current[threadId];
    pendingAssistantDeltaByThreadRef.current[threadId] = current
      ? {
          ...current,
          text: `${current.text}${delta}`,
          metadata: metadata ?? current.metadata,
        }
      : {
          threadId,
          intentId,
          runId,
          text: delta,
          metadata: metadata ?? null,
        };
    schedulePendingAssistantDeltaFlush();
  }

  function requestMessagesBottomSnap(
    threadId: string | null | undefined,
    forceStick = false,
  ) {
    if (!threadId) {
      return;
    }
    pendingThreadBottomSnapRef.current = threadId;
    if (forceStick) {
      shouldStickMessagesToBottomRef.current = true;
      forceMessagesBottomSnapRef.current = true;
    }
  }

  function scheduleMessagesScrollToLatest(
    behavior: ScrollBehavior = "auto",
  ) {
    scrollMessagesToLatest(messagesRef.current, behavior);
    if (messagesStickScrollFrameRef.current !== null) {
      window.cancelAnimationFrame(messagesStickScrollFrameRef.current);
    }
    messagesStickScrollFrameRef.current = window.requestAnimationFrame(() => {
      messagesStickScrollFrameRef.current = null;
      if (shouldStickMessagesToBottomRef.current) {
        scrollMessagesToLatest(messagesRef.current, "auto");
      }
    });
  }

  function updatePendingAssistantActivity(
    threadId: string,
    intentId: string | undefined,
    assistantEntryId: string | null | undefined,
    text: string,
  ) {
    updateMessagesByThread((current) => {
      const existing = current[threadId] || [];
      let changed = false;
      const nextMessages = existing.map((entry) => {
        const matchesAssistantEntry =
          assistantEntryId && entry.id === assistantEntryId;
        const matchesIntent =
          intentId && entry.role === "assistant" && entry.intentId === intentId;
        if (
          entry.role !== "assistant" ||
          !entry.pending ||
          (!matchesAssistantEntry && !matchesIntent) ||
          entry.text === text
        ) {
          return entry;
        }
        changed = true;
        return {
          ...entry,
          text,
        };
      });

      if (!changed) {
        return current;
      }
      return {
        ...current,
        [threadId]: nextMessages,
      };
    });
  }

  function applyThreadTitleUpdate(threadId: string, title: string) {
    const nextTitle = title.trim();
    if (!threadId || !nextTitle) {
      return;
    }

    threadTitleOverridesRef.current = {
      ...threadTitleOverridesRef.current,
      [threadId]: nextTitle,
    };

    setDesktopState((current) => {
      if (!current) {
        return current;
      }
      let changed = false;
      const updateThread = (
        thread: (typeof current.threads)[number],
      ): (typeof current.threads)[number] => {
        if (thread.id !== threadId || thread.title === nextTitle) {
          return thread;
        }
        changed = true;
        return { ...thread, title: nextTitle };
      };
      const threads = current.threads.map(updateThread);
      const sessions = current.sessions.map(updateThread);
      return changed ? { ...current, threads, sessions } : current;
    });

    if (selectedThreadIdRef.current === threadId && !editingThreadTitle) {
      setTitleDraft(nextTitle);
    }
  }

  function appendSeededTurn(
    threadId: string,
    intent: MessageIntent,
    options?: {
      seedUserBubble?: boolean;
    },
  ): SeededTurn {
    const seedUserBubble = options?.seedUserBubble ?? true;
    const userMessage = seededUserBubble(intent);
    const legacyPendingAssistant =
      (messagesByThreadRef.current[threadId] || []).find(
        (entry) =>
          entry.role === "assistant" &&
          entry.pending &&
          entry.intentId === intent.intentId,
      ) || null;

    if (seedUserBubble) {
      updateMessagesByThread((current) => {
        const existing = current[threadId] || [];
        const hasUserMessage = existing.some((entry) => {
          return entry.role === "user" && entry.intentId === intent.intentId;
        });
        if (hasUserMessage) {
          return current;
        }
        return {
          ...current,
          [threadId]: [...existing, userMessage],
        };
      });
    }

    return {
      assistantEntryId: legacyPendingAssistant?.id || null,
      legacyPendingAssistantId: legacyPendingAssistant?.id || null,
    };
  }

  function promoteNewThreadDraftState(threadId: string) {
    dispatchMessageState({
      type: "thread/replace-id",
      fromThreadId: NEW_THREAD_DRAFT_THREAD_ID,
      toThreadId: threadId,
    });

    updateMessagesByThread((current) => {
      const draftMessages = current[NEW_THREAD_DRAFT_THREAD_ID] || [];
      if (!draftMessages.length) {
        if (!(NEW_THREAD_DRAFT_THREAD_ID in current)) {
          return current;
        }
        const next = { ...current };
        delete next[NEW_THREAD_DRAFT_THREAD_ID];
        return next;
      }

      const existing = current[threadId] || [];
      const draftIds = new Set(draftMessages.map((entry) => entry.id));
      const draftRoleIntentKeys = new Set(
        draftMessages
          .map((entry) =>
            entry.intentId ? `${entry.role}:${entry.intentId}` : "",
          )
          .filter(Boolean),
      );
      const merged = [
        ...draftMessages,
        ...existing.filter((entry) => {
          if (draftIds.has(entry.id)) {
            return false;
          }
          if (
            entry.intentId &&
            draftRoleIntentKeys.has(`${entry.role}:${entry.intentId}`)
          ) {
            return false;
          }
          return true;
        }),
      ];
      const next = {
        ...current,
        [threadId]: merged,
      };
      delete next[NEW_THREAD_DRAFT_THREAD_ID];
      return next;
    });

    const draftLiveStream =
      liveStreamStateRef.current[NEW_THREAD_DRAFT_THREAD_ID];
    if (draftLiveStream) {
      const updated = { ...liveStreamStateRef.current };
      delete updated[NEW_THREAD_DRAFT_THREAD_ID];
      updated[threadId] = {
        ...draftLiveStream,
        threadId,
      };
      liveStreamStateRef.current = updated;
      setLiveStreamStateByThread(updated);
    }

    requestMessagesBottomSnap(threadId, true);
  }

  function markLocalDispatchFailed(
    threadId: string,
    intentId: string,
    message: string,
  ) {
    clearLiveStreamState(threadId);
    dispatchMessageState({
      type: "intent/failed",
      intentId,
      error: message,
    });
    setThreadRuntimeState(threadId, "failed", {
      activeIntentId: intentId,
      error: message,
    });
    updateMessagesByThread((current) => {
      const existing = current[threadId] || [];
      let updated = false;
      const nextEntries = existing.map((entry) => {
        if (
          entry.role !== "assistant" ||
          entry.intentId !== intentId ||
          (!entry.pending && !entry.error)
        ) {
          return entry;
        }
        updated = true;
        return {
          ...entry,
          pending: false,
          error: true,
          localState: "error" as TranscriptEntryState,
          text: entry.pending ? message : entry.text || message,
        };
      });
      if (updated) {
        return {
          ...current,
          [threadId]: nextEntries,
        };
      }
      return {
        ...current,
        [threadId]: [
          ...nextEntries,
          {
            id: `assistant:error:${intentId}:${crypto.randomUUID()}`,
            role: "assistant",
            text: message,
            timestamp: new Date().toISOString(),
            intentId,
            localState: "error",
            error: true,
          },
        ],
      };
    });
  }

  function setRemotePendingInputs(
    threadId: string,
    pendingInputs: PendingThreadInput[],
  ) {
    setPendingRemoteInputsByThread((current) => {
      const next = { ...current };
      if (pendingInputs.length > 0) {
        next[threadId] = pendingInputs;
      } else {
        delete next[threadId];
      }
      pendingRemoteInputsRef.current = next;
      return next;
    });
  }

  function consumeRemotePendingInput(
    threadId: string,
    pendingInputId?: string,
  ): PendingThreadInput | null {
    let consumed: PendingThreadInput | null = null;
    setPendingRemoteInputsByThread((current) => {
      const existing = current[threadId] || [];
      const normalizedPendingInputId = pendingInputId?.trim() || "";
      const consumeIndex = normalizedPendingInputId
        ? existing.findIndex((input) => {
            return (
              input.status === "awaiting_ack" &&
              input.id === normalizedPendingInputId
            );
          })
        : existing.findIndex((input) => input.status === "awaiting_ack");
      if (consumeIndex < 0) {
        pendingRemoteInputsRef.current = current;
        return current;
      }
      consumed = existing[consumeIndex] || null;
      const nextInputs = existing.filter((_, index) => index !== consumeIndex);
      const next = { ...current };
      if (nextInputs.length > 0) {
        next[threadId] = nextInputs;
      } else {
        delete next[threadId];
      }
      pendingRemoteInputsRef.current = next;
      return next;
    });
    return consumed;
  }

  function setPendingAutomationRun(
    threadId: string,
    run: PendingAutomationRun | null,
  ) {
    setPendingAutomationRunsByThread((current) => {
      const next = { ...current };
      if (run) {
        next[threadId] = run;
      } else {
        delete next[threadId];
      }
      pendingAutomationRunsRef.current = next;
      return next;
    });
  }

  function reconcilePendingAutomationRun(
    threadId: string,
    run: PendingAutomationRun,
    attempt = 0,
  ) {
    const retryDelaysMs = [450, 900, 1_600, 2_600, 4_000, 6_000];
    const delayMs = retryDelaysMs[attempt];
    if (delayMs === undefined) {
      return;
    }

    window.setTimeout(() => {
      void (async () => {
        const currentPending = pendingAutomationRunsRef.current[threadId];
        if (!currentPending || currentPending.runId !== run.runId) {
          return;
        }

        try {
          const transcript =
            await window.garyxDesktop.getThreadHistory(threadId);
          if (
            transcript.messages.length > 0 ||
            transcript.pendingInputs.length > 0
          ) {
            applyRemoteTranscript(threadId, transcript);
          }
          if (transcriptHasAutomationResponse(transcript.messages)) {
            setPendingAutomationRun(threadId, null);
            return;
          }
        } catch {
          // Best-effort polling while the automation thread history lands.
        }

        if (pendingAutomationRunsRef.current[threadId]?.runId === run.runId) {
          reconcilePendingAutomationRun(threadId, run, attempt + 1);
        }
      })();
    }, delayMs);
  }

  function applyStreamingAssistantDelta(
    threadId: string,
    intentId: string | undefined,
    runId: string,
    delta: string,
    metadata?: Record<string, unknown> | null,
  ): string {
    let nextAssistantEntryId =
      getLiveStreamState(threadId)?.assistantEntryId || null;
    const nextSpeakerKey = speakerIdentityKey(metadata);

    updateMessagesByThread((current) => {
      const existing = current[threadId] || [];
      if (nextAssistantEntryId) {
        let matchedExistingEntry = false;
        let speakerChanged = false;
        const next = {
          ...current,
          [threadId]: existing.map((entry) => {
            if (entry.id !== nextAssistantEntryId) {
              return entry;
            }
            if (
              nextSpeakerKey &&
              nextSpeakerKey !== speakerIdentityKey(entry.metadata)
            ) {
              speakerChanged = true;
              return entry;
            }
            matchedExistingEntry = true;
            return {
              ...entry,
              text: entry.pending ? delta : `${entry.text}${delta}`,
              metadata: metadata ?? entry.metadata ?? null,
              pending: false,
              error: false,
              localState: "remote_partial" as const,
              remoteRunId: runId,
            };
          }),
        };
        if (matchedExistingEntry && !speakerChanged) {
          return next;
        }
        nextAssistantEntryId = null;
      }

      const pendingIndex = existing.findIndex((entry) => {
        return (
          entry.role === "assistant" &&
          entry.pending &&
          entry.intentId === intentId
        );
      });
      if (pendingIndex >= 0) {
        const next = [...existing];
        nextAssistantEntryId = next[pendingIndex]?.id || null;
        next[pendingIndex] = {
          ...next[pendingIndex],
          text: delta,
          metadata: metadata ?? next[pendingIndex]?.metadata ?? null,
          pending: false,
          error: false,
          remoteRunId: runId,
          localState: "remote_partial" as const,
        };
        return {
          ...current,
          [threadId]: next,
        };
      }

      nextAssistantEntryId = `assistant:${intentId || threadId}:${crypto.randomUUID()}`;
      return {
        ...current,
        [threadId]: [
          ...existing,
          {
            id: nextAssistantEntryId,
            role: "assistant",
            text: delta,
            metadata: metadata ?? null,
            timestamp: new Date().toISOString(),
            intentId,
            remoteRunId: runId,
            localState: "remote_partial" as const,
          },
        ],
      };
    });

    return (
      nextAssistantEntryId ||
      `assistant:${intentId || threadId}:${crypto.randomUUID()}`
    );
  }

  function applyStreamingAssistantBoundary(threadId: string): string | null {
    let nextAssistantEntryId =
      getLiveStreamState(threadId)?.assistantEntryId || null;

    updateMessagesByThread((current) => {
      const existing = current[threadId] || [];
      if (!nextAssistantEntryId) {
        return current;
      }
      return {
        ...current,
        [threadId]: existing.map((entry) => {
          if (entry.id !== nextAssistantEntryId) {
            return entry;
          }
          const currentText = entry.text || "";
          if (!currentText.trim()) {
            return entry;
          }
          const nextText = currentText.endsWith("\n\n")
            ? currentText
            : currentText.endsWith("\n")
              ? `${currentText}\n`
              : `${currentText}\n\n`;
          return nextText === currentText
            ? entry
            : {
                ...entry,
                text: nextText,
              };
        }),
      };
    });

    return nextAssistantEntryId;
  }

  function appendStreamingToolEvent(
    event: Extract<
      DesktopChatStreamEvent,
      { type: "tool_use" | "tool_result" }
    >,
    context?: {
      intentId?: string;
      assistantEntryId?: string | null;
    },
  ) {
    const threadId = event.threadId;
    updateMessagesByThread((current) => {
      const existing = current[threadId] || [];
      return {
        ...current,
        [threadId]: [
          ...existing.filter((entry) => {
            return !(
              entry.role === "assistant" &&
              entry.pending &&
              (entry.intentId === context?.intentId ||
                entry.id === context?.assistantEntryId)
            );
          }),
          buildStreamingToolBubble(event, context?.intentId),
        ],
      };
    });
  }

  function applyCanonicalTranscript(
    threadId: string,
    transcript: ThreadTranscript,
  ) {
    setThreadInfoByThread((current) => ({
      ...current,
      [threadId]: transcript.threadInfo ?? null,
    }));
    remoteTranscriptSignatureByThreadRef.current = {
      ...remoteTranscriptSignatureByThreadRef.current,
      [threadId]: threadActivitySignature(
        transcript.messages,
        transcript.pendingInputs,
        transcript.threadInfo,
      ),
    };
    setRemotePendingInputs(threadId, transcript.pendingInputs);
    startTransition(() => {
      updateMessagesByThread((current) => {
        const existing = current[threadId] || [];
        return {
          ...current,
          [threadId]: materializeRemoteTranscript(
            transcript.messages,
            existing,
          ),
        };
      });
    });
    markIntentsFromHistory(threadId, transcript.messages);
  }

  function materializeAckedUserMessage(threadId: string, intentId: string) {
    const intent = intentForId(intentId);
    if (!intent) {
      return;
    }
    const ackedUserBubble = seededAckedUserBubble(intent);
    updateMessagesByThread((current) => {
      const existing = current[threadId] || [];
      if (
        existing.some(
          (entry) => entry.role === "user" && entry.intentId === intentId,
        )
      ) {
        return current;
      }
      return {
        ...current,
        [threadId]: [...existing, ackedUserBubble],
      };
    });
  }

  function materializeAckedRemotePendingInput(
    threadId: string,
    pendingInput: PendingThreadInput,
  ) {
    const ackedUserBubble: UiTranscriptMessage = {
      id: `pending-user:${pendingInput.id}`,
      role: "user",
      text: pendingInput.text,
      content: pendingInput.content,
      timestamp: pendingInput.timestamp || new Date().toISOString(),
      metadata: {
        queued_input_id: pendingInput.id,
      },
      remoteRunId: pendingInput.runId || undefined,
      localState: "remote_partial",
      pending: false,
      error: false,
    };
    updateMessagesByThread((current) => {
      const existing = current[threadId] || [];
      if (
        existing.some((entry) => {
          return (
            queuedInputIdFromMessage(entry) === pendingInput.id ||
            transcriptMessagesSemanticallyMatch(entry, ackedUserBubble)
          );
        })
      ) {
        return current;
      }
      return {
        ...current,
        [threadId]: [...existing, ackedUserBubble],
      };
    });
  }

  function handleChatStreamEvent(event: DesktopChatStreamEvent) {
    const threadId = event.threadId;
    let currentStream = getLiveStreamState(threadId);
    if (event.type !== "assistant_delta") {
      currentStream = flushPendingAssistantDelta(threadId);
    }
    const activeIntentId = currentStream?.activeIntentId;

    switch (event.type) {
      case "accepted": {
        updateLiveStreamState(threadId, (current) => ({
          threadId: threadId,
          runId: event.runId,
          activeIntentId: current?.activeIntentId,
          assistantEntryId: current?.assistantEntryId ?? null,
          pendingAckIntentIds: current?.pendingAckIntentIds || [],
          streamStatus: "streaming",
        }));
        if (activeIntentId) {
          const intent = intentForId(activeIntentId);
          if (
            intent &&
            ![
              "remote_accepted",
              "awaiting_provider_ack",
              "awaiting_history",
              "completed",
            ].includes(intent.state)
          ) {
            dispatchMessageState({
              type: "intent/remote-accepted",
              intentId: activeIntentId,
              runId: event.runId,
              threadId: threadId,
              removeFromQueue: false,
            });
          }
        }
        setThreadRuntimeState(threadId, "running_remote", {
          activeIntentId,
          remoteRunId: event.runId,
        });
        break;
      }
      case "assistant_delta": {
        enqueueStreamingAssistantDelta(
          threadId,
          activeIntentId,
          event.runId,
          event.delta,
          event.metadata,
        );
        break;
      }
      case "assistant_boundary": {
        const assistantEntryId = applyStreamingAssistantBoundary(threadId);
        updateLiveStreamState(threadId, (current) => ({
          threadId: threadId,
          runId: event.runId,
          activeIntentId: current?.activeIntentId,
          assistantEntryId,
          pendingAckIntentIds: current?.pendingAckIntentIds || [],
          streamStatus: "streaming",
        }));
        setThreadRuntimeState(threadId, "running_remote", {
          activeIntentId: activeIntentId || undefined,
          remoteRunId: event.runId,
        });
        break;
      }
      case "tool_use":
      case "tool_result":
        if (event.type === "tool_result") {
          const generatedImageContent = extractImageGenerationImageContent(
            event.message,
          );
          if (generatedImageContent) {
            appendStreamingToolEvent(
              { ...event, threadId: threadId },
              {
                intentId: activeIntentId,
                assistantEntryId: currentStream?.assistantEntryId ?? null,
              },
            );
            updateMessagesByThread((current) => {
              const existing = current[threadId] || [];
              return {
                ...current,
                [threadId]: [
                  ...existing.filter((entry) => {
                    return !(
                      entry.role === "assistant" &&
                      entry.pending &&
                      (entry.intentId === activeIntentId ||
                        entry.id === (currentStream?.assistantEntryId ?? null))
                    );
                  }),
                  {
                    id: `assistant-generated-image:${event.message.toolUseId || activeIntentId || threadId}:${crypto.randomUUID()}`,
                    role: "assistant",
                    text: "",
                    content: generatedImageContent,
                    metadata: {
                      source: "codex_app_server",
                      item_type: "imageGeneration",
                      [GENERATED_IMAGE_TOOL_USE_METADATA_KEY]:
                        event.message.toolUseId || "",
                    },
                    timestamp: event.message.timestamp || new Date().toISOString(),
                    intentId: activeIntentId,
                    remoteRunId: event.runId,
                    localState: "remote_partial",
                    pending: false,
                    error: false,
                  },
                ],
              };
            });
            updateLiveStreamState(threadId, (current) => ({
              threadId: threadId,
              runId: event.runId,
              activeIntentId: current?.activeIntentId,
              assistantEntryId: null,
              pendingAckIntentIds: current?.pendingAckIntentIds || [],
              streamStatus: "streaming",
            }));
            setThreadRuntimeState(threadId, "running_remote", {
              activeIntentId: activeIntentId || undefined,
              remoteRunId: event.runId,
            });
            break;
          }
          const mediaContent = extractStreamingMessageToolImageContent(event);
          if (mediaContent) {
            updateMessagesByThread((current) => {
              const existing = current[threadId] || [];
              return {
                ...current,
                [threadId]: [
                  ...existing.filter((entry) => {
                    return !(
                      entry.role === "assistant" &&
                      entry.pending &&
                      (entry.intentId === activeIntentId ||
                        entry.id === (currentStream?.assistantEntryId ?? null))
                    );
                  }),
                  {
                    id: `assistant-media:${activeIntentId || threadId}:${crypto.randomUUID()}`,
                    role: "assistant",
                    text: "",
                    content: mediaContent,
                    timestamp: new Date().toISOString(),
                    intentId: activeIntentId,
                    remoteRunId: event.runId,
                    localState: "remote_partial",
                    pending: false,
                    error: false,
                  },
                ],
              };
            });
            updateLiveStreamState(threadId, (current) => ({
              threadId: threadId,
              runId: event.runId,
              activeIntentId: current?.activeIntentId,
              assistantEntryId: null,
              pendingAckIntentIds: current?.pendingAckIntentIds || [],
              streamStatus: "streaming",
            }));
            setThreadRuntimeState(threadId, "running_remote", {
              activeIntentId: activeIntentId || undefined,
              remoteRunId: event.runId,
            });
            break;
          }
        }
        if (!shouldRenderToolTraceMessage({ ...event.message, text: "" })) {
          updatePendingAssistantActivity(
            threadId,
            activeIntentId,
            currentStream?.assistantEntryId ?? null,
            event.type === "tool_use"
              ? HIDDEN_TOOL_USE_STATUS_TEXT
              : HIDDEN_TOOL_RESULT_STATUS_TEXT,
          );
          updateLiveStreamState(threadId, (current) => ({
            threadId: threadId,
            runId: event.runId,
            activeIntentId: current?.activeIntentId,
            assistantEntryId: current?.assistantEntryId ?? null,
            pendingAckIntentIds: current?.pendingAckIntentIds || [],
            streamStatus: "streaming",
          }));
          setThreadRuntimeState(threadId, "running_remote", {
            activeIntentId: activeIntentId || undefined,
            remoteRunId: event.runId,
          });
          break;
        }
        appendStreamingToolEvent(
          { ...event, threadId: threadId },
          {
            intentId: activeIntentId,
            assistantEntryId: currentStream?.assistantEntryId ?? null,
          },
        );
        updateLiveStreamState(threadId, (current) => ({
          threadId: threadId,
          runId: event.runId,
          activeIntentId: current?.activeIntentId,
          assistantEntryId: null,
          pendingAckIntentIds: current?.pendingAckIntentIds || [],
          streamStatus: "streaming",
        }));
        setThreadRuntimeState(threadId, "running_remote", {
          activeIntentId: activeIntentId || undefined,
          remoteRunId: event.runId,
        });
        break;
      case "user_ack": {
        let nextIntentId: string | undefined;
        const acknowledgedPendingInputId = event.pendingInputId?.trim() || "";
        updateLiveStreamState(threadId, (current) => {
          const pendingAckIntentIds = [...(current?.pendingAckIntentIds || [])];
          const matchedIndex = findPendingAckIntentIndex(
            pendingAckIntentIds,
            acknowledgedPendingInputId,
            messageStateRef.current.intentsById,
          );
          if (matchedIndex >= 0) {
            nextIntentId = pendingAckIntentIds[matchedIndex];
            pendingAckIntentIds.splice(matchedIndex, 1);
          } else {
            nextIntentId = undefined;
          }
          return current
            ? {
                ...current,
                runId: event.runId,
                activeIntentId: nextIntentId || current.activeIntentId,
                assistantEntryId: null,
                pendingAckIntentIds,
                streamStatus: "streaming",
              }
            : null;
        });
        if (nextIntentId) {
          materializeAckedUserMessage(threadId, nextIntentId);
          const acknowledgedIntent = intentForId(nextIntentId);
          dispatchMessageState({
            type: "intent/awaiting-history",
            intentId: nextIntentId,
            responseText: acknowledgedIntent?.responseText,
          });
          // Queued follow-ups can surface in the thread snapshot before the transcript catches up.
          scheduleHistoryRefresh(threadId, 8, 250, false);
          requestMessagesBottomSnap(threadId);
          setThreadRuntimeState(threadId, "running_remote", {
            activeIntentId: nextIntentId,
            remoteRunId: event.runId,
          });
        } else {
          const pendingInput = consumeRemotePendingInput(
            threadId,
            acknowledgedPendingInputId,
          );
          if (pendingInput) {
            materializeAckedRemotePendingInput(threadId, pendingInput);
            requestMessagesBottomSnap(threadId);
          }
          scheduleHistoryRefresh(threadId, 4, 250, false);
        }
        break;
      }
      case "thread_title_updated":
        applyThreadTitleUpdate(threadId, event.title);
        break;
      case "done":
        if (activeIntentId) {
          dispatchMessageState({
            type: "intent/awaiting-history",
            intentId: activeIntentId,
            responseText: intentForId(activeIntentId)?.responseText,
          });
        }
        updateLiveStreamState(threadId, (current) =>
          current
            ? {
                ...current,
                runId: event.runId,
                assistantEntryId: null,
                streamStatus: "reconciling",
              }
            : null,
        );
        setThreadRuntimeState(threadId, "reconciling_history", {
          activeIntentId: activeIntentId || undefined,
          remoteRunId: event.runId,
        });
        scheduleHistoryRefresh(threadId, 8, 250, true);
        break;
      case "error":
        const recoveryResult = activeIntentId
          ? reconcileAssistantEntriesForGatewayRecovery(
              messagesByThreadRef.current[threadId] || [],
              activeIntentId,
              [currentStream?.assistantEntryId],
            )
          : { entries: [] as UiTranscriptMessage[], matched: false };
        if (
          isTransientGatewayErrorMessage(event.error) ||
          recoveryResult.matched
        ) {
          const recoveryStatusLabel = "Waiting to sync with gateway…";
          recordGatewayStatusObservation(
            {
              ok: false,
              bridgeReady: false,
              gatewayUrl: connection?.gatewayUrl || settingsDraft.gatewayUrl,
              error: event.error,
            },
            recoveryStatusLabel,
          );
          let assistantEntryId: string | null | undefined = null;
          updateLiveStreamState(threadId, (current) => {
            assistantEntryId = current?.assistantEntryId ?? null;
            return current
              ? {
                  ...current,
                  runId: event.runId,
                  assistantEntryId: null,
                  streamStatus: "disconnected",
                }
              : null;
          });
          if (activeIntentId) {
            dispatchMessageState({
              type: "intent/awaiting-history",
              intentId: activeIntentId,
            });
          }
          setThreadRuntimeState(threadId, "reconciling_history", {
            activeIntentId: activeIntentId || undefined,
            remoteRunId: event.runId,
          });
          if (activeIntentId) {
            updateMessagesByThread((current) => {
              const nextEntries = reconcileAssistantEntriesForGatewayRecovery(
                current[threadId] || [],
                activeIntentId,
                [assistantEntryId],
              ).entries;
              return {
                ...current,
                [threadId]: nextEntries,
              };
            });
          }
          scheduleHistoryRefresh(threadId, 5, 1200, true);
          break;
        }
        updateLiveStreamState(threadId, (current) =>
          current
            ? {
                ...current,
                runId: event.runId,
                assistantEntryId: null,
                streamStatus: "failed",
              }
            : null,
        );
        if (activeIntentId) {
          dispatchMessageState({
            type: "intent/failed",
            intentId: activeIntentId,
            error: event.error,
          });
        }
        setThreadRuntimeState(threadId, "failed", {
          activeIntentId: activeIntentId || undefined,
          remoteRunId: event.runId,
          error: event.error,
        });
        setError(event.error);
        break;
      default:
        break;
    }
  }

  function markIntentsFromHistory(
    threadId: string,
    transcript: TranscriptMessage[],
  ) {
    const intents = Object.values(messageStateRef.current.intentsById).filter(
      (intent) => {
        return (
          intent.threadId === threadId &&
          [
            "dispatching",
            "remote_accepted",
            "awaiting_provider_ack",
            "awaiting_response",
            "awaiting_history",
          ].includes(intent.state)
        );
      },
    );

    for (const intent of intents) {
      const match = resolveIntentHistoryMatch(intent, transcript);
      if (!match.userVisible) {
        continue;
      }
      if (
        match.assistantVisible ||
        (!intent.responseText && intent.dispatchMode === "async_steer")
      ) {
        dispatchMessageState({
          type: "intent/completed",
          intentId: intent.intentId,
        });
      } else {
        dispatchMessageState({
          type: "intent/awaiting-history",
          intentId: intent.intentId,
          responseText: intent.responseText,
        });
      }
    }

    const runtime = selectThreadRuntime(messageStateRef.current, threadId);
    if (runtime && !hasPendingHistoryIntents(threadId)) {
      dispatchMessageState({
        type: "thread/clear",
        threadId,
      });
      const liveStream = getLiveStreamState(threadId);
      if (
        liveStream &&
        ["reconciling", "disconnected", "failed"].includes(
          liveStream.streamStatus,
        )
      ) {
        clearLiveStreamState(threadId);
      }
    }
  }

  function shiftQueuedIntent(threadId: string): MessageIntent | null {
    const [nextIntentId] = queueIntentIdsForThread(threadId);
    if (!nextIntentId) {
      return null;
    }
    const intent = intentForId(nextIntentId);
    if (!intent) {
      dispatchMessageState({
        type: "intent/cancelled",
        threadId,
        intentId: nextIntentId,
      });
      return null;
    }
    return intent;
  }

  function reorderQueuedIntent(
    threadId: string,
    draggedIntentId: string,
    targetIntentId: string,
    position: "before" | "after",
  ) {
    const queueIntentIds = queueIntentIdsForThread(threadId);
    const fromIndex = queueIntentIds.indexOf(draggedIntentId);
    const targetIndex = queueIntentIds.indexOf(targetIntentId);
    if (fromIndex < 0 || targetIndex < 0 || fromIndex === targetIndex) {
      return;
    }

    const toIndex =
      position === "before"
        ? targetIndex > fromIndex
          ? targetIndex - 1
          : targetIndex
        : targetIndex > fromIndex
          ? targetIndex
          : targetIndex + 1;

    dispatchMessageState({
      type: "intent/reorder",
      threadId,
      intentId: draggedIntentId,
      toIndex,
    });
  }

  function mergeRemoteTranscriptWithLocal(
    transcript: TranscriptMessage[],
    existing: UiTranscriptMessage[],
    options?: {
      activeRunSnapshot?: boolean;
      preserveRemoteBeforeIndex?: number | null;
    },
  ): UiTranscriptMessage[] {
    if (transcript.length === 0) {
      return existing.length > 0 ? existing : [];
    }

    const materializedRemote = materializeRemoteTranscript(
      transcript,
      existing,
      {
        ignoreTimestampForStableMessages: options?.activeRunSnapshot,
      },
    );
    const materializedRemoteIds = new Set(
      materializedRemote.map((entry) => entry.id),
    );
    const preservedRemoteBeforeEntries: UiTranscriptMessage[] = [];
    const preservedLocalEntries = existing.filter((entry, index, entries) => {
      if (entry.localState === "remote_final") {
        const historyIndex = transcriptEntryHistoryIndex(entry);
        if (
          typeof options?.preserveRemoteBeforeIndex === "number" &&
          historyIndex !== null &&
          historyIndex < options.preserveRemoteBeforeIndex &&
          !materializedRemoteIds.has(entry.id)
        ) {
          preservedRemoteBeforeEntries.push(entry);
        }
        return false;
      }
      if (
        entries.findIndex((candidate) => candidate.id === entry.id) !== index
      ) {
        return false;
      }
      if (!entry.intentId) {
        const queuedInputId = queuedInputIdFromMessage(entry);
        if (entry.role === "user" && queuedInputId) {
          return !materializedRemote.some((candidate) => {
            return queuedInputIdFromMessage(candidate) === queuedInputId;
          });
        }
        return (
          entry.localState === "error" || entry.localState === "interrupted"
        );
      }

      const intent = intentForId(entry.intentId);
      if (!intent) {
        return (
          entry.localState === "error" || entry.localState === "interrupted"
        );
      }

      const match = resolveIntentHistoryMatch(intent, transcript);
      if (entry.role === "user") {
        return !match.userVisible;
      }
      if (entry.role === "assistant") {
        return !match.assistantVisible;
      }
      if (isToolRole(entry.role)) {
        return !materializedRemote.some((candidate) =>
          toolMessagesEquivalent(candidate, entry),
        );
      }
      return false;
    });

    return [
      ...preservedRemoteBeforeEntries,
      ...materializedRemote,
      ...preservedLocalEntries,
    ];
  }

  function updateThreadHistoryPagination(
    threadId: string,
    updater: (
      current: ThreadHistoryPaginationState | null,
    ) => ThreadHistoryPaginationState | null,
  ) {
    const previous = historyPaginationByThreadRef.current[threadId] || null;
    const nextValue = updater(previous);
    const next = { ...historyPaginationByThreadRef.current };
    if (nextValue) {
      next[threadId] = nextValue;
    } else {
      delete next[threadId];
    }
    historyPaginationByThreadRef.current = next;
    setHistoryPaginationByThread(next);
  }

  function paginationStateFromTranscript(
    transcript: ThreadTranscript,
    loadingBefore = false,
  ): ThreadHistoryPaginationState {
    return {
      hasMoreBefore: Boolean(transcript.pageInfo?.hasMoreBefore),
      nextBeforeIndex:
        typeof transcript.pageInfo?.nextBeforeIndex === "number"
          ? transcript.pageInfo.nextBeforeIndex
          : null,
      loadingBefore,
    };
  }

  function threadSummaryFromTranscript(
    threadId: string,
    transcript: ThreadTranscript,
  ): DesktopThreadSummary {
    if (transcript.thread) {
      return {
        ...transcript.thread,
        agentId: transcript.thread.agentId ?? transcript.threadInfo?.agentId ?? null,
        workspacePath:
          transcript.thread.workspacePath ?? transcript.threadInfo?.workspacePath ?? null,
        worktree: transcript.thread.worktree ?? transcript.threadInfo?.worktree ?? null,
        team: transcript.thread.team ?? transcript.team ?? null,
      };
    }

    const timestamps = transcript.messages
      .map((message) => message.timestamp || '')
      .filter(Boolean);
    const fallbackTimestamp =
      timestamps[timestamps.length - 1] || new Date().toISOString();
    const preview =
      transcript.messages.find((message) => message.text.trim())?.text.trim() || '';

    return {
      id: threadId,
      title: transcript.threadInfo?.agentId || threadId,
      createdAt: timestamps[0] || fallbackTimestamp,
      updatedAt: fallbackTimestamp,
      lastMessagePreview: preview,
      workspacePath: transcript.threadInfo?.workspacePath ?? null,
      messageCount: transcript.pageInfo?.totalMessages ?? transcript.messages.length,
      agentId: transcript.threadInfo?.agentId ?? null,
      recentRunId: transcript.threadInfo?.activeRun?.runId ?? null,
      worktree: transcript.threadInfo?.worktree ?? null,
      team: transcript.team ?? null,
    };
  }

  function cacheOpenableTranscriptThread(
    threadId: string,
    transcript: ThreadTranscript,
  ) {
    const summary = threadSummaryFromTranscript(threadId, transcript);
    setDesktopState((current) => {
      if (!current || current.threads.some((thread) => thread.id === threadId)) {
        return current;
      }
      return {
        ...current,
        sessions: mergeThread(current.sessions, summary),
      };
    });
  }

  function applyRemoteTranscript(
    threadId: string,
    transcript: ThreadTranscript,
  ) {
    cacheOpenableTranscriptThread(threadId, transcript);
    updateThreadHistoryPagination(threadId, (current) => {
      const incoming = paginationStateFromTranscript(transcript);
      if (!current) {
        return incoming;
      }
      if (!current.hasMoreBefore) {
        const earliestLoadedIndex = earliestRemoteHistoryIndex(
          messagesByThreadRef.current[threadId] || [],
        );
        if (earliestLoadedIndex === 0 || !incoming.hasMoreBefore) {
          return { ...current, loadingBefore: false };
        }
        return incoming;
      }
      if (
        current.nextBeforeIndex !== null &&
        incoming.nextBeforeIndex !== null &&
        current.nextBeforeIndex <= incoming.nextBeforeIndex
      ) {
        return { ...current, loadingBefore: false };
      }
      return incoming;
    });
    setThreadInfoByThread((current) => ({
      ...current,
      [threadId]: transcript.threadInfo ?? null,
    }));
    remoteTranscriptSignatureByThreadRef.current = {
      ...remoteTranscriptSignatureByThreadRef.current,
      [threadId]: threadActivitySignature(
        transcript.messages,
        transcript.pendingInputs,
        transcript.threadInfo,
      ),
    };
    setRemotePendingInputs(threadId, transcript.pendingInputs);
    startTransition(() => {
      updateMessagesByThread((current) => {
        const existing = current[threadId] || [];
        const merged = mergeRemoteTranscriptWithLocal(
          transcript.messages,
          existing,
          {
            activeRunSnapshot: Boolean(transcript.threadInfo?.activeRun),
            preserveRemoteBeforeIndex: transcript.pageInfo?.startIndex ?? null,
          },
        );
        if (
          merged.length === existing.length &&
          merged.every((entry, index) => entry === existing[index])
        ) {
          return current;
        }
        return {
          ...current,
          [threadId]: merged,
        };
      });
    });
    // Propagate the transcript's `team` block into `desktopState.threads[i]`
    // so team-bound threads render the team badge + sub-agent peek tabs as
    // soon as the thread metadata endpoint has confirmed the binding. Without
    // this merge, a list summary (which may have been fetched before the
    // first turn) could shadow the richer detail payload, leaving the UI
    // stuck on the plain agent label. Only write when the block is present
    // and different from what's already cached — idempotent updates must
    // not churn React identity and re-trigger dependent effects.
    if (transcript.team !== undefined) {
      setDesktopState((current) => {
        if (!current) {
          return current;
        }
        const nextTeam = transcript.team ?? null;
        let changed = false;
        const mapThreadTeam = (
          thread: (typeof current.threads)[number],
        ): (typeof current.threads)[number] => {
          if (thread.id !== threadId) {
            return thread;
          }
          const prev = thread.team ?? null;
          if (teamBlocksEqual(prev, nextTeam)) {
            return thread;
          }
          changed = true;
          return { ...thread, team: nextTeam };
        };
        const nextThreads = current.threads.map(mapThreadTeam);
        const nextSessions = current.sessions.map(mapThreadTeam);
        if (!changed) {
          return current;
        }
        return { ...current, threads: nextThreads, sessions: nextSessions };
      });
    }
    markIntentsFromHistory(threadId, transcript.messages);
  }

  function applyOlderRemoteTranscriptPage(
    threadId: string,
    transcript: ThreadTranscript,
  ) {
    updateThreadHistoryPagination(threadId, () =>
      paginationStateFromTranscript(transcript),
    );
    if (transcript.messages.length === 0) {
      return;
    }

    updateMessagesByThread((current) => {
      const existing = current[threadId] || [];
      const existingIds = new Set(existing.map((entry) => entry.id));
      const olderEntries = materializeRemoteTranscript(
        transcript.messages,
        [],
      ).filter((entry) => !existingIds.has(entry.id));
      if (olderEntries.length === 0) {
        return current;
      }
      return {
        ...current,
        [threadId]: [...olderEntries, ...existing],
      };
    });
  }

  async function loadOlderThreadHistoryPage(threadId: string) {
    const pagination = historyPaginationByThreadRef.current[threadId] || null;
    if (
      !pagination?.hasMoreBefore ||
      pagination.loadingBefore ||
      pagination.nextBeforeIndex === null
    ) {
      return;
    }

    updateThreadHistoryPagination(threadId, (current) => ({
      hasMoreBefore: Boolean(current?.hasMoreBefore),
      nextBeforeIndex: current?.nextBeforeIndex ?? null,
      loadingBefore: true,
    }));

    try {
      const transcript = await window.garyxDesktop.getThreadHistory({
        threadId,
        beforeIndex: pagination.nextBeforeIndex,
        limit: THREAD_HISTORY_PAGE_SIZE,
        userQueryLimit: THREAD_HISTORY_USER_QUERY_LIMIT,
      });
      const node = messagesRef.current;
      if (
        transcript.messages.length > 0 &&
        node &&
        selectedThreadIdRef.current === threadId
      ) {
        pendingMessagesPrependAnchorRef.current = {
          threadId,
          scrollHeight: node.scrollHeight,
          scrollTop: node.scrollTop,
        };
      }
      applyOlderRemoteTranscriptPage(threadId, transcript);
    } catch (historyError) {
      pendingMessagesPrependAnchorRef.current = null;
      setError(
        historyError instanceof Error
          ? historyError.message
          : "Failed to load earlier thread history",
      );
    } finally {
      if (selectedThreadIdRef.current !== threadId) {
        pendingMessagesPrependAnchorRef.current = null;
      }
      updateThreadHistoryPagination(threadId, (current) =>
        current ? { ...current, loadingBefore: false } : current,
      );
    }
  }

  async function handleSelectWorkspace(
    workspacePath: string,
    threadId?: string | null,
  ) {
    await selectWorkspaceForThread({
      api: getDesktopApi(),
      workspacePath,
      threadId,
      setError,
      setContentView: () => {
        setContentView("thread");
      },
      setDesktopState,
      setSelectedThreadId,
      setNewThreadDraftActive,
      setPendingWorkspacePath,
      requestComposerFocus,
    });
  }

  async function ensureSelectedThreadId(): Promise<string | null> {
    return ensureThread({
      api: getDesktopApi(),
      selectedThreadId,
      pendingWorkspacePath,
      pendingWorkspaceMode,
      pendingAgentId,
      preferredWorkspacePath: preferredWorkspaceForNewThread?.available
        ? preferredWorkspaceForNewThread.path
        : null,
      selectableWorkspaceCount: selectableNewThreadWorkspaces.length,
      onAddWorkspace: handleAddWorkspaceForNewThread,
      setWorkspaceMutation,
      setDesktopState,
      setSelectedThreadId,
      initializeThreadMessages: (threadId) => {
        updateMessagesByThread((current) => ({
          ...current,
          [threadId]: [],
        }));
      },
      setNewThreadDraftActive,
      setPendingWorkspacePath,
      setPendingWorkspaceMode,
      setPendingBotId,
      setPendingAgentId,
      setError,
    });
  }

  async function handleResumeProviderSession(
    sessionId: string,
    providerHint?: DesktopSessionProviderHint | null,
  ): Promise<void> {
    const trimmedSessionId = sessionId.trim();
    if (!trimmedSessionId) {
      throw new Error("Paste a Claude, Codex, or Gemini session ID first.");
    }

    setError(null);
    try {
      const created = await window.garyxDesktop.createThread({
        sdkSessionId: trimmedSessionId,
        sdkSessionProviderHint: providerHint || undefined,
      });
      setDesktopState(created.state);
      setContentView("thread");
      setNewThreadDraftActive(false);
      setSelectedThreadId(created.thread.id);
      updateMessagesByThread((current) => ({
        ...current,
        [created.thread.id]: current[created.thread.id] || [],
      }));
      setPendingWorkspacePath(null);
      setPendingWorkspaceMode("local");
      setPendingBotId(null);
      setPendingAgentId(created.thread.agentId || providerHint || "claude");
      requestComposerFocus();
    } catch (resumeError) {
      const message =
        resumeError instanceof Error
          ? resumeError.message
          : "Failed to resume this session";
      setError(message);
      throw new Error(message);
    }
  }

  useEffect(() => {
    deepLinkEventHandlerRef.current = (event: DesktopDeepLinkEvent) => {
      void (async () => {
        try {
          switch (event.type) {
            case "error":
              pushToast(event.error, "error");
              return;
            case "open-thread":
              await openThreadFromDeepLink(event.threadId);
              return;
            case "new-thread":
              await applyDesktopRoute({
                kind: "new-thread",
                workspacePath: event.workspacePath || null,
                agentId: event.agentId || null,
              });
              return;
            case "resume-session":
              await waitForGatewayReadyForDeepLink();
              await handleResumeProviderSession(
                event.sessionId,
                event.providerHint,
              );
              return;
          }
        } catch (deepLinkError) {
          const message =
            deepLinkError instanceof Error
              ? deepLinkError.message
              : "Failed to handle garyx:// link.";
          pushToast(message, "error");
        }
      })();
    };
  }, [applyDesktopRoute, handleResumeProviderSession, openThreadFromDeepLink, pushToast]);

  async function syncThreadBotBinding(
    threadId: string,
    botId: string | null,
  ): Promise<void> {
    const requestSequence = botBindingRequestSequenceRef.current + 1;
    botBindingRequestSequenceRef.current = requestSequence;
    await measureUiAction("bot.bind_thread", async () => {
      const currentEndpoints = (desktopState?.endpoints || []).filter(
        (endpoint) => endpoint.threadId === threadId,
      );
      let nextDesktopState: DesktopState | null = null;

      if (!botId) {
        nextDesktopState = await window.garyxDesktop.setBotBinding({
          threadId,
          botId: null,
        });
        for (const endpoint of currentEndpoints) {
          nextDesktopState = await window.garyxDesktop.detachChannelEndpoint({
            endpointKey: endpoint.endpointKey,
          });
        }
        if (nextDesktopState) {
          const finalState = nextDesktopState;
          if (botBindingRequestSequenceRef.current !== requestSequence) {
            return;
          }
          startTransition(() => {
            setDesktopState(finalState);
          });
        }
        return;
      }

      const targetGroup = botGroups.find((group) => group.id === botId);
      const targetEndpoint =
        targetGroup?.defaultOpenEndpoint || targetGroup?.mainEndpoint || null;
      nextDesktopState = await window.garyxDesktop.setBotBinding({
        threadId,
        botId,
      });

      for (const endpoint of currentEndpoints) {
        if (endpoint.endpointKey === targetEndpoint?.endpointKey) {
          continue;
        }
        if (botGroupIdForEndpoint(endpoint) === botId) {
          continue;
        }
        nextDesktopState = await window.garyxDesktop.detachChannelEndpoint({
          endpointKey: endpoint.endpointKey,
        });
      }

      if (
        targetEndpoint?.endpointKey &&
        targetGroup?.mainThreadId !== threadId &&
        targetEndpoint.threadId !== threadId
      ) {
        nextDesktopState = await window.garyxDesktop.bindChannelEndpoint({
          endpointKey: targetEndpoint.endpointKey,
          threadId,
        });
      }

      if (nextDesktopState) {
        const finalState = nextDesktopState;
        if (botBindingRequestSequenceRef.current !== requestSequence) {
          return;
        }
        startTransition(() => {
          setDesktopState(finalState);
        });
      }
    });
  }

  async function ensureThreadBotRouting(threadId: string): Promise<boolean> {
    const desiredBotId = !selectedThreadId ? (pendingBotId ?? null) : null;
    if (!desiredBotId) {
      return true;
    }

    const alreadyBound = (desktopState?.endpoints || []).some((endpoint) => {
      return (
        endpoint.threadId === threadId &&
        botGroupIdForEndpoint(endpoint) === desiredBotId
      );
    });
    if (alreadyBound) {
      return true;
    }

    try {
      await syncThreadBotBinding(threadId, desiredBotId);
      return true;
    } catch (bindingError) {
      setError(
        bindingError instanceof Error
          ? bindingError.message
          : "Failed to update bot binding",
      );
      return false;
    }
  }

  async function handleNewThread() {
    setBotConversationGroupId(null);
    setWorkspaceConversationPath(null);
    setThreadLogsOpen(false);
    setInspectorOpen(false);
    startNewThreadDraft({
      selectableNewThreadWorkspaces,
      pendingNewThreadWorkspaceEntry,
      activeThreadNewThreadWorkspace: activeThreadNewThreadWorkspace,
      selectedNewThreadWorkspaceEntry,
      setError,
      setContentView: () => {
        setContentView("thread");
      },
      setNewThreadDraftActive,
      setSelectedThreadId,
      setPendingWorkspacePath,
      setPendingBotId,
      setPendingAgentId,
      clearComposerDraft,
      syncComposerPhase,
      requestComposerFocus,
    });
    setPendingWorkflowId(null);
  }

  function handleStartDraftForAgent(agentId: string) {
    const nextWorkspace = pickPreferredWorkspace(
      selectableNewThreadWorkspaces,
      pendingNewThreadWorkspaceEntry,
      activeThreadNewThreadWorkspace,
      selectedNewThreadWorkspaceEntry,
    );
    setError(null);
    setContentView("thread");
    setNewThreadDraftActive(true);
    setSelectedThreadId(null);
    setPendingWorkspacePath(nextWorkspace?.path || null);
    setPendingWorkspaceMode("local");
    setPendingBotId(null);
    setPendingAgentId(agentId);
    setPendingWorkflowId(null);
    clearComposerDraft();
    syncComposerPhase("");
    requestComposerFocus();
  }

  async function handleBotClick(group: DesktopBotConsoleSummary) {
    await activateBotDraftThread({
      platform: getDesktopApi(),
      desktopState,
      group,
      onState: setDesktopState,
      onOpenExistingThread: (endpoint) => {
        return handleOpenThreadFromEndpoint(endpoint, "bot-root");
      },
      onOpenThreadById: (threadId) => {
        return openExistingThread(threadId, "bot-root").then((opened) => {
          if (opened) {
            setPendingWorkspacePath(null);
            setPendingWorkspaceMode("local");
            setPendingBotId(null);
          }
          return opened;
        });
      },
      shouldKeepNewDraft: (groupId, initialWorkspacePath) =>
        newThreadDraftActiveRef.current &&
        selectedThreadIdRef.current === null &&
        pendingBotIdRef.current === groupId &&
        pendingWorkspacePathRef.current === initialWorkspacePath,
      shouldOpenResolvedThread: (groupId, initialWorkspacePath) =>
        newThreadDraftActiveRef.current &&
        selectedThreadIdRef.current === null &&
        pendingBotIdRef.current === groupId &&
        pendingWorkspacePathRef.current === initialWorkspacePath &&
        !composerHasPayloadRef.current,
      setError,
      setContentView: () => {
        setContentView("thread");
      },
      setNewThreadDraftActive,
      setSelectedThreadId: (value) => {
        setSelectedThreadId(value);
        setThreadEntrySelectionSource(value ? "bot-root" : null);
      },
      setPendingWorkspacePath,
      setPendingBotId,
      clearComposerDraft,
      syncComposerPhase,
      requestComposerFocus,
    });
  }

  function handleCreateThreadForWorkspace(workspacePath: string) {
    startNewThreadDraft({
      selectableNewThreadWorkspaces,
      pendingNewThreadWorkspaceEntry,
      activeThreadNewThreadWorkspace: activeThreadNewThreadWorkspace,
      selectedNewThreadWorkspaceEntry,
      workspacePath,
      setError,
      setContentView: () => {
        setContentView("thread");
      },
      setNewThreadDraftActive,
      setSelectedThreadId,
      setPendingWorkspacePath,
      setPendingBotId,
      setPendingAgentId,
      clearComposerDraft,
      syncComposerPhase,
      requestComposerFocus,
    });
    setPendingWorkflowId(null);
  }

  async function handleAddWorkspace() {
    setAddWorkspaceDialog({
      source: "new-thread",
      initialPath: pendingWorkspacePath || selectedWorkspaceEntry?.path || "",
    });
  }

  async function handleAddWorkspaceForNewThread(): Promise<DesktopWorkspace | null> {
    return new Promise((resolve) => {
      setAddWorkspaceDialog({
        source: "new-thread",
        initialPath: pendingWorkspacePath || selectedWorkspaceEntry?.path || "",
        resolve,
      });
    });
  }

  function closeAddWorkspaceDialog(workspace: DesktopWorkspace | null = null) {
    setAddWorkspaceDialog((current) => {
      current?.resolve?.(workspace);
      return null;
    });
  }

  async function addWorkspacePathFromPicker(path: string): Promise<DesktopWorkspace | null> {
    setError(null);
    setWorkspaceMutation("add");
    try {
      const result = await window.garyxDesktop.addWorkspaceByPath({ path });
      setDesktopState(result.state);
      return result.workspace || null;
    } catch (workspaceError) {
      setError(
        workspaceError instanceof Error
          ? workspaceError.message
          : "Failed to add workspace",
      );
      return null;
    } finally {
      setWorkspaceMutation(null);
    }
  }

  async function confirmAddWorkspace(path: string) {
    const request = addWorkspaceDialog;
    if (!request) {
      return;
    }
    const workspace = await addWorkspacePathFromPicker(path);
    if (workspace) {
      if (request.source === "new-thread") {
        setNewThreadDraftActive(true);
        setPendingWorkspacePath(workspace.path);
        setPendingWorkspaceMode("local");
        requestComposerFocus();
      }
      closeAddWorkspaceDialog(workspace);
    }
  }

  async function handleRemoveWorkspace(workspacePath: string) {
    setError(null);
    setWorkspaceMutation("remove");
    const workspaceKey = workspacePath.trim().toLowerCase();
    const previousState = desktopState;
    const removedWorkspace = previousState?.workspaces.find((workspace) => {
      return (workspace.path || "").trim().toLowerCase() === workspaceKey;
    }) || null;
    if (previousState && removedWorkspace) {
      setDesktopState({
        ...previousState,
        workspaces: previousState.workspaces.filter((workspace) => {
          return (workspace.path || "").trim().toLowerCase() !== workspaceKey;
        }),
        selectedWorkspacePath:
          (previousState.selectedWorkspacePath || "").trim().toLowerCase() === workspaceKey
            ? null
            : previousState.selectedWorkspacePath,
      });
    }
    try {
      const nextState = await window.garyxDesktop.removeWorkspace({
        workspacePath,
      });
      setDesktopState(nextState);
      if (selectedThreadId) {
        const selectedThreadStillExists = nextState.threads.some(
          (thread) => thread.id === selectedThreadId,
        );
        if (!selectedThreadStillExists) {
          setSelectedThreadId(nextState.threads[0]?.id || null);
        }
      }
    } catch (removeError) {
      if (previousState && removedWorkspace) {
        setDesktopState((current) => {
          if (!current) {
            return previousState;
          }
          if (
            current.workspaces.some((workspace) => {
              return (workspace.path || "").trim().toLowerCase() === workspaceKey;
            })
          ) {
            return current;
          }
          return {
            ...current,
            workspaces: [...current.workspaces, removedWorkspace],
            selectedWorkspacePath:
              (previousState.selectedWorkspacePath || "").trim().toLowerCase() === workspaceKey
                ? previousState.selectedWorkspacePath
                : current.selectedWorkspacePath,
          };
        });
      }
      setError(
        removeError instanceof Error
          ? removeError.message
          : "Failed to remove workspace",
      );
    } finally {
      setWorkspaceMutation(null);
    }
  }

  async function handleRequestRemoveWorkspace(workspace: DesktopWorkspace) {
    setWorkspaceMenuOpenPath(null);
    await handleRemoveWorkspace(workspace.path || "");
  }

  function beginThreadTitleEdit() {
    if (!canEditThreadTitle || !activeThread) {
      return;
    }
    setTitleDraft(activeThread.title || DEFAULT_SESSION_TITLE);
    setEditingThreadTitle(true);
  }

  async function handleSaveTitle(options?: { closeEditor?: boolean }) {
    await saveThreadTitle({
      api: getDesktopApi(),
      activeThread: activeThread,
      activeAutomationThread: Boolean(activeAutomationThread),
      titleDraft,
      closeEditor: options?.closeEditor,
      defaultTitle: DEFAULT_SESSION_TITLE,
      setError,
      setSavingTitle,
      setDesktopState,
      setTitleDraft,
      setEditingThreadTitle,
    });
  }

  function cancelThreadTitleEdit() {
    setEditingThreadTitle(false);
    setTitleDraft(activeThread?.title || DEFAULT_SESSION_TITLE);
  }

  async function handleDeleteThread(threadId?: string) {
    const targetThreadId = threadId || activeThread?.id || null;
    const targetRuntime = targetThreadId
      ? selectThreadRuntime(messageStateRef.current, targetThreadId)
      : null;
    const targetIsBusy =
      targetThreadId === activeThread?.id
        ? isRuntimeBusy(activeRuntime?.state)
        : isRuntimeBusy(targetRuntime?.state);
    await deleteThread({
      api: getDesktopApi(),
      desktopState,
      targetThreadId,
      targetIsAutomationThread: Boolean(
        targetThreadId &&
        desktopState &&
        automationForLatestThread(desktopState, targetThreadId),
      ),
      targetIsBusy,
      selectedThreadId,
      setError,
      setDeletingThreadId,
      setDesktopState,
      setSelectedThreadId,
      dispatchDelete: (nextThreadId) => {
        dispatchMessageState({
          type: "thread/delete",
          threadId: nextThreadId,
        });
      },
    });
  }

  async function handleArchiveBotConversationEndpoint(endpoint: DesktopChannelEndpoint) {
    const targetThreadId = endpoint.threadId || null;
    if (!targetThreadId || !desktopState) {
      return;
    }
    const targetRuntime = selectThreadRuntime(
      messageStateRef.current,
      targetThreadId,
    );
    const targetIsBusy =
      targetThreadId === activeThread?.id
        ? isRuntimeBusy(activeRuntime?.state)
        : isRuntimeBusy(targetRuntime?.state);
    if (targetIsBusy) {
      return;
    }
    if (automationForLatestThread(desktopState, targetThreadId)) {
      setError("Delete this automation from the Automation view.");
      return;
    }

    const endpointKeys = new Set(
      (desktopState.endpoints || [])
        .filter((candidate) => candidate.threadId === targetThreadId)
        .map((candidate) => candidate.endpointKey)
        .filter((value): value is string => Boolean(value)),
    );
    if (endpoint.endpointKey) {
      endpointKeys.add(endpoint.endpointKey);
    }

    setDeletingThreadId(targetThreadId);
    setError(null);
    try {
      const api = getDesktopApi();
      let nextState: DesktopState | null = null;
      for (const endpointKey of endpointKeys) {
        nextState = await api.detachChannelEndpoint({ endpointKey });
      }
      if (nextState) {
        setDesktopState(nextState);
      }

      const deletedState = await api.deleteThread({ threadId: targetThreadId });
      const deletingSelected = targetThreadId === selectedThreadId;
      const fallbackThread = deletingSelected
        ? deletedState.threads[0] || null
        : null;
      setDesktopState(deletedState);
      if (deletingSelected) {
        setSelectedThreadId(fallbackThread?.id || null);
      }
      dispatchMessageState({
        type: "thread/delete",
        threadId: targetThreadId,
      });
    } catch (archiveError) {
      setError(
        archiveError instanceof Error
          ? archiveError.message
          : "Failed to delete the thread",
      );
    } finally {
      setDeletingThreadId((current) =>
        current === targetThreadId ? null : current,
      );
    }
  }

  async function handleOpenThreadFromEndpoint(
    endpoint: DesktopChannelEndpoint,
    entrySource: ThreadEntrySelectionSource | null = null,
  ): Promise<boolean> {
    if (endpoint.threadId) {
      return openExistingThread(endpoint.threadId, entrySource);
    }

    openThreadFromEndpoint({
      endpoint,
      setError,
      setContentView: () => {
        setContentView("thread");
      },
      setNewThreadDraftActive,
      setSelectedThreadId: (value) => {
        setSelectedThreadId(value);
        setThreadEntrySelectionSource(value ? entrySource : null);
      },
    });
    return false;
  }

  async function handleTakeOverEndpoint(endpointKey: string) {
    await bindEndpointToThread({
      api: getDesktopApi(),
      endpointKey,
      threadId: activeThread?.id,
      setBindingMutation,
      setError,
      setDesktopState,
    });
  }

  async function handleDetachEndpoint(endpointKey: string) {
    await detachEndpointFromThread({
      api: getDesktopApi(),
      endpointKey,
      setBindingMutation,
      setError,
      setDesktopState,
    });
  }

  async function handleSetBotBinding(botId: string | null) {
    await updateThreadBotBinding({
      threadId: activeThread?.id,
      botId,
      setBindingMutation,
      setError,
      syncThreadBotBinding,
    });
  }

  function hasPendingHistoryIntents(threadId: string): boolean {
    return Object.values(messageStateRef.current.intentsById).some((intent) => {
      return (
        intent.threadId === threadId &&
        [
          "remote_accepted",
          "awaiting_provider_ack",
          "awaiting_history",
          "awaiting_response",
          "dispatching",
        ].includes(intent.state)
      );
    });
  }

  function scheduleHistoryRefresh(
    threadId: string,
    attempts = 4,
    delayMs = 1200,
    canonical = false,
  ) {
    scheduleThreadHistoryRefresh({
      api: getDesktopApi(),
      threadId,
      attempts,
      delayMs,
      canonical,
      shouldContinue: hasPendingHistoryIntents,
      onCanonicalTranscript: applyCanonicalTranscript,
      onRemoteTranscript: applyRemoteTranscript,
      onExhausted: forceReleaseThreadRuntime,
    });
  }

  function forceReleaseThreadRuntime(threadId: string) {
    const pendingStates = [
      "dispatching",
      "remote_accepted",
      "awaiting_provider_ack",
      "awaiting_response",
      "awaiting_history",
    ];
    for (const intent of Object.values(messageStateRef.current.intentsById)) {
      if (intent.threadId === threadId && pendingStates.includes(intent.state)) {
        dispatchMessageState({
          type: "intent/completed",
          intentId: intent.intentId,
        });
      }
    }
    dispatchMessageState({
      type: "thread/clear",
      threadId,
    });
    const liveStream = getLiveStreamState(threadId);
    if (
      liveStream &&
      ["reconciling", "disconnected", "failed"].includes(liveStream.streamStatus)
    ) {
      clearLiveStreamState(threadId);
    }
  }

  async function sendIntentOnce(
    threadId: string,
    intentId: string,
    options?: {
      seedUserBubble?: boolean;
      seededTurn?: SeededTurn;
    },
  ): Promise<boolean> {
    const intent = intentForId(intentId);
    if (!intent) {
      return false;
    }

    const { assistantEntryId, legacyPendingAssistantId } =
      options?.seededTurn || appendSeededTurn(threadId, intent, options);

    dispatchMessageState({
      type: "intent/dispatch-started",
      intentId: intent.intentId,
    });
    dispatchMessageState({
      type: "intent/awaiting-response",
      intentId: intent.intentId,
    });
    setThreadRuntimeState(threadId, "dispatching_sync", {
      activeIntentId: intent.intentId,
    });
    updateLiveStreamState(threadId, () => ({
      threadId,
      activeIntentId: intent.intentId,
      assistantEntryId,
      pendingAckIntentIds: [],
      streamStatus: "connecting",
    }));

    setError(null);
    requestMessagesBottomSnap(threadId, true);

    try {
      const result = await window.garyxDesktop.openChatStream({
        threadId,
        clientIntentId: intent.intentId,
        message: intent.text,
        images: intent.images,
        files: intent.files,
      });
      const resultThreadId = result.threadId || result.sessionId || threadId;
      const liveState = getLiveStreamState(resultThreadId);
      if (!liveState?.runId && result.runId) {
        updateLiveStreamState(resultThreadId, (current) =>
          current
            ? {
                ...current,
                runId: result.runId,
                streamStatus:
                  result.status === "completed"
                    ? "reconciling"
                    : "disconnected",
              }
            : null,
        );
      }
      if (result.status === "disconnected") {
        recordGatewayStatusObservation(
          {
            ok: false,
            bridgeReady: false,
            gatewayUrl: connection?.gatewayUrl || settingsDraft.gatewayUrl,
            error: "stream disconnected",
          },
          "Waiting to sync with gateway…",
        );
      }
      const latestIntent = intentForId(intent.intentId);
      if (
        latestIntent &&
        ![
          "remote_accepted",
          "awaiting_provider_ack",
          "awaiting_history",
          "completed",
        ].includes(latestIntent.state)
      ) {
        dispatchMessageState({
          type: "intent/remote-accepted",
          intentId: intent.intentId,
          runId: result.runId,
          threadId: resultThreadId,
          responseText: result.response,
          removeFromQueue: false,
        });
      }
      dispatchMessageState({
        type: "intent/awaiting-history",
        intentId: intent.intentId,
        responseText: result.response,
      });
      setThreadRuntimeState(threadId, "reconciling_history", {
        activeIntentId: intent.intentId,
        remoteRunId: result.runId,
      });

      setDesktopState((current) => {
        if (!current) {
          return current;
        }
        const titleOverride = threadTitleOverridesRef.current[resultThreadId];
        const resultThread = titleOverride
          ? { ...result.thread, title: titleOverride }
          : result.thread;
        return {
          ...current,
          threads: mergeThread(current.threads, resultThread),
          sessions: mergeThread(current.threads, resultThread),
        };
      });

      const transcript =
        await window.garyxDesktop.getThreadHistory(resultThreadId);
      const intentSnapshot = intentForId(intent.intentId) || {
        ...intent,
        responseText: result.response,
      };
      const match = resolveIntentHistoryMatch(
        intentSnapshot,
        transcript.messages,
      );

      if (
        transcript.messages.length > 0 &&
        match.userVisible &&
        (match.assistantVisible ||
          normalizeMessageText(result.response).length === 0)
      ) {
        applyCanonicalTranscript(resultThreadId, transcript);
      } else {
        if (
          legacyPendingAssistantId &&
          !result.response &&
          result.status === "completed"
        ) {
          updateMessagesByThread((current) => ({
            ...current,
            [resultThreadId]: (current[resultThreadId] || []).filter(
              (entry) => {
                return !(
                  entry.id === legacyPendingAssistantId &&
                  entry.pending
                );
              },
            ),
          }));
        }
        scheduleHistoryRefresh(resultThreadId, 4, 1200, true);
      }

      clearLiveStreamState(resultThreadId);

      return true;
    } catch (sendError) {
      const rawMessage =
        sendError instanceof Error
          ? sendError.message
          : "Garyx request failed before completion";
      const threadProviderType = inferProviderTypeForThread(
        threadId,
        threadInfoByThread,
        desktopState,
        desktopAgents,
      );
      const message = presentProviderReadyError(
        rawMessage,
        threadProviderType,
      );
      const interrupted = rawMessage === "request interrupted";
      const errorState: TranscriptEntryState = interrupted
        ? "interrupted"
        : "error";
      const liveState = getLiveStreamState(threadId);
      const failedIntentId = liveState?.activeIntentId || intent.intentId;
      const recoveryResult = reconcileAssistantEntriesForGatewayRecovery(
        messagesByThreadRef.current[threadId] || [],
        failedIntentId,
        [legacyPendingAssistantId, liveState?.assistantEntryId],
      );
      const likelyTransportDrop =
        !interrupted &&
        (isTransientGatewayErrorMessage(message) || recoveryResult.matched);

      if (likelyTransportDrop) {
        recordGatewayStatusObservation(
          {
            ok: false,
            bridgeReady: false,
            gatewayUrl: connection?.gatewayUrl || settingsDraft.gatewayUrl,
            error: rawMessage,
          },
          "Waiting to sync with gateway…",
        );
        clearLiveStreamState(threadId);
        dispatchMessageState({
          type: "intent/awaiting-history",
          intentId: failedIntentId,
          responseText: intent.responseText,
        });
        setThreadRuntimeState(threadId, "reconciling_history", {
          activeIntentId: failedIntentId,
          remoteRunId: liveState?.runId,
        });
        updateMessagesByThread((current) => ({
          ...current,
          [threadId]: reconcileAssistantEntriesForGatewayRecovery(
            current[threadId] || [],
            failedIntentId,
            [legacyPendingAssistantId, liveState?.assistantEntryId],
          ).entries,
        }));
        scheduleHistoryRefresh(threadId, 5, 1200, true);
        return true;
      }

      clearLiveStreamState(threadId);
      setError(message);
      dispatchMessageState({
        type: interrupted ? "intent/interrupted" : "intent/failed",
        intentId: failedIntentId,
        ...(interrupted ? { error: message } : { error: message }),
      });
      setThreadRuntimeState(threadId, interrupted ? "interrupting" : "failed", {
        activeIntentId: failedIntentId,
        error: message,
      });
      updateMessagesByThread((current) => ({
        ...current,
        [threadId]: (() => {
          const existing = current[threadId] || [];
          let updated = false;
          const next = existing.map((entry) => {
            const isTargetAssistant =
              entry.role === "assistant" &&
              entry.intentId === failedIntentId &&
              (entry.pending ||
                entry.id === legacyPendingAssistantId ||
                entry.id === liveState?.assistantEntryId);
            if (!isTargetAssistant) {
              return entry;
            }
            updated = true;
            return {
              ...entry,
              pending: false,
              error: true,
              localState: errorState,
              text: interrupted
                ? entry.text ||
                  "Run interrupted before Garyx produced a final answer."
                : entry.text || message,
            };
          });
          if (updated) {
            return next;
          }
          return [
            ...next,
            {
              id: `assistant:error:${failedIntentId}:${crypto.randomUUID()}`,
              role: "assistant",
              text: interrupted
                ? "Run interrupted before Garyx produced a final answer."
                : message,
              timestamp: new Date().toISOString(),
              intentId: failedIntentId,
              localState: errorState,
              error: true,
            },
          ];
        })(),
      }));
      return false;
    }
  }

  async function runQueuedBatch(threadId: string, initialIntentId?: string) {
    const firstIntentId = initialIntentId || "";
    if (!firstIntentId && queueIntentIdsForThread(threadId).length === 0) {
      return;
    }

    setError(null);

    try {
      let nextIntentId = firstIntentId;
      let dispatchedFromQueue = false;
      let seededTurn: SeededTurn | undefined;

      while (nextIntentId || queueIntentIdsForThread(threadId).length > 0) {
        seededTurn = undefined;
        if (!nextIntentId) {
          const currentQueuedIntent = shiftQueuedIntent(threadId);
          nextIntentId = currentQueuedIntent?.intentId || "";
          dispatchedFromQueue = true;
          if (!currentQueuedIntent || !nextIntentId) {
            break;
          }
          seededTurn = appendSeededTurn(threadId, currentQueuedIntent);
          dispatchMessageState({
            type: "intent/request-dispatch",
            threadId,
            intentId: nextIntentId,
            mode: "sync_send",
            source: "queue_send",
            removeFromQueue: true,
          });
        } else {
          dispatchedFromQueue = false;
        }

        const didSucceed = await sendIntentOnce(threadId, nextIntentId, {
          seededTurn,
        });
        if (!didSucceed) {
          if (dispatchedFromQueue) {
            dispatchMessageState({
              type: "intent/requeue-front",
              threadId,
              intentId: nextIntentId,
              source: "queue_send",
              error: intentForId(nextIntentId)?.error,
            });
          }
          break;
        }
        nextIntentId = "";
      }
    } finally {
      if (!hasPendingHistoryIntents(threadId)) {
        dispatchMessageState({
          type: "thread/clear",
          threadId,
        });
      }
      const status = await window.garyxDesktop.checkConnection();
      setConnection(status);
    }
  }

  useEffect(() => {
    const threadId = selectedThreadId;
    if (!threadId || contentView !== "thread") {
      return;
    }
    if (activeQueue.length === 0) {
      delete deferredQueueDrainByThreadRef.current[threadId];
      delete queueDrainInFlightByThreadRef.current[threadId];
      return;
    }
    if (
      isActiveSendingThread ||
      isDraftSendingThread ||
      !deferredQueueDrainByThreadRef.current[threadId] ||
      queueDrainInFlightByThreadRef.current[threadId]
    ) {
      return;
    }

    deferredQueueDrainByThreadRef.current[threadId] = false;
    queueDrainInFlightByThreadRef.current[threadId] = true;
    void runQueuedBatch(threadId).finally(() => {
      delete queueDrainInFlightByThreadRef.current[threadId];
    });
  }, [
    activeQueue.length,
    contentView,
    isActiveSendingThread,
    isDraftSendingThread,
    selectedThreadId,
  ]);

  async function handleQueueCurrentPrompt(options?: { steerImmediately?: boolean }) {
    if (composerAttachmentUploadPending) {
      setError("Attachments are still uploading to gateway.");
      return;
    }
    const prompt = composerDraftRef.current.trim();
    if (!prompt && !composerImages.length && !composerFiles.length) {
      return;
    }
    const threadId = await ensureSelectedThreadId();
    if (!threadId) {
      return;
    }
    if (!(await ensureThreadBotRouting(threadId))) {
      return;
    }
    const intent = buildIntent({
      threadId,
      text: prompt,
      images: composerImages,
      files: composerFiles,
      source: "composer_queue",
      state: "queued_local",
    });
    dispatchMessageState({
      type: "intent/created",
      intent,
      enqueue: true,
    });
    if (isActiveSendingThread) {
      deferredQueueDrainByThreadRef.current[threadId] = true;
    }
    clearComposerDraft();
    setError(null);
    if (options?.steerImmediately) {
      await steerQueuedIntent(intent);
    }
  }

  async function steerQueuedIntent(latestIntent: MessageIntent) {
    const threadId = latestIntent.threadId;
    if (!canSteerQueuedPrompt) {
      return;
    }
    if (latestIntent.state !== "queued_local") {
      return;
    }

    dispatchMessageState({
      type: "intent/request-dispatch",
      threadId: threadId,
      intentId: latestIntent.intentId,
      mode: "async_steer",
      source: "queue_steer",
      removeFromQueue: false,
    });
    dispatchMessageState({
      type: "intent/dispatch-started",
      intentId: latestIntent.intentId,
    });

    setError(null);
    requestMessagesBottomSnap(threadId, true);
    const optimisticRunId =
      getLiveStreamState(threadId)?.runId ||
      selectThreadRuntime(messageStateRef.current, threadId)?.remoteRunId ||
      `stream:${threadId}`;
    updateLiveStreamState(threadId, (current) => {
      const pendingAckIntentIds = current?.pendingAckIntentIds || [];
      return {
        threadId,
        runId: current?.runId || optimisticRunId,
        activeIntentId: current?.activeIntentId,
        assistantEntryId: current?.assistantEntryId ?? null,
        pendingAckIntentIds: pendingAckIntentIds.includes(latestIntent.intentId)
          ? pendingAckIntentIds
          : [...pendingAckIntentIds, latestIntent.intentId],
        streamStatus: current?.streamStatus || "streaming",
      };
    });

    try {
      const result = await window.garyxDesktop.sendStreamingInput({
        threadId,
        clientIntentId: latestIntent.intentId,
        message: latestIntent.text,
        images: latestIntent.images,
        files: latestIntent.files,
      });
      const resultThreadId = result.threadId || result.sessionId || threadId;
      if (result.status === "queued") {
        const activeRunId =
          getLiveStreamState(resultThreadId)?.runId ||
          selectThreadRuntime(messageStateRef.current, resultThreadId)
            ?.remoteRunId ||
          `stream:${resultThreadId}`;
        dispatchMessageState({
          type: "intent/remote-accepted",
          intentId: latestIntent.intentId,
          runId: activeRunId,
          threadId: resultThreadId,
          pendingInputId: result.pendingInputId,
          removeFromQueue: true,
          awaitProviderAck: true,
        });
        updateLiveStreamState(resultThreadId, (current) => ({
          threadId: resultThreadId,
          runId: current?.runId || activeRunId,
          activeIntentId: current?.activeIntentId,
          assistantEntryId: current?.assistantEntryId ?? null,
          pendingAckIntentIds: (
            current?.pendingAckIntentIds || []
          ).includes(latestIntent.intentId)
            ? current?.pendingAckIntentIds || []
            : [...(current?.pendingAckIntentIds || []), latestIntent.intentId],
          streamStatus: current?.streamStatus || "streaming",
        }));
        setThreadRuntimeState(resultThreadId, "running_remote", {
          activeIntentId:
            getLiveStreamState(resultThreadId)?.activeIntentId ||
            selectThreadRuntime(messageStateRef.current, resultThreadId)
              ?.activeIntentId,
          remoteRunId: activeRunId,
        });
        return;
      }

      updateLiveStreamState(threadId, (current) =>
        current
          ? {
              ...current,
              pendingAckIntentIds: current.pendingAckIntentIds.filter(
                (entry) => entry !== latestIntent.intentId,
              ),
            }
          : current,
      );
      dispatchMessageState({
        type: "intent/request-dispatch",
        threadId: threadId,
        intentId: latestIntent.intentId,
        mode: "sync_send",
        source: "queue_steer",
        removeFromQueue: true,
      });
      dispatchMessageState({
        type: "intent/dispatch-started",
        intentId: latestIntent.intentId,
      });
      const didSucceed = await sendIntentOnce(threadId, latestIntent.intentId, {
        seedUserBubble: true,
      });
      if (!didSucceed) {
        dispatchMessageState({
          type: "intent/requeue-front",
          threadId: threadId,
          intentId: latestIntent.intentId,
          source: "queue_steer",
          error: intentForId(latestIntent.intentId)?.error,
        });
      }
    } catch (steerError) {
      updateLiveStreamState(threadId, (current) =>
        current
          ? {
              ...current,
              pendingAckIntentIds: current.pendingAckIntentIds.filter(
                (entry) => entry !== latestIntent.intentId,
              ),
            }
          : current,
      );
      const message =
        steerError instanceof Error
          ? steerError.message
          : "Failed to steer follow-up";
      setError(message);
      dispatchMessageState({
        type: "intent/requeue-front",
        threadId: threadId,
        intentId: latestIntent.intentId,
        source: "queue_steer",
        error: message,
      });
    }
  }

  async function handleSteerQueuedPrompt(intent: MessageIntent) {
    const latestIntent = intentForId(intent.intentId);
    if (!latestIntent || latestIntent.state !== "queued_local") {
      return;
    }
    await steerQueuedIntent(latestIntent);
  }

  async function handleStartWorkflowThreadFromComposer(input: {
    prompt: string;
    promptFiles: MessageFileAttachment[];
    promptImages: MessageImageAttachment[];
    workflowId: string;
  }) {
    if (input.promptFiles.length > 0 || input.promptImages.length > 0) {
      setError("Remove attachments before starting a workflow.");
      return;
    }

    newThreadInitialDispatchLockRef.current = true;
    setWorkflowThreadStarting(true);
    try {
      const workspacePath =
        pendingWorkspacePath ||
        (await ensureWorkspaceForNewThread({
          api: getDesktopApi(),
          preferredWorkspacePath: preferredWorkspaceForNewThread?.available
            ? preferredWorkspaceForNewThread.path
            : null,
          selectableWorkspaceCount: selectableNewThreadWorkspaces.length,
          onAddWorkspace: handleAddWorkspaceForNewThread,
          setWorkspaceMutation,
          setDesktopState,
          setError,
        }));
      if (!workspacePath) {
        return;
      }

      setError(null);
      const started = await window.garyxDesktop.startWorkflowThread({
        workflowId: input.workflowId,
        input: input.prompt,
        workspacePath,
        workspaceMode: pendingWorkspaceMode,
      });
      setDesktopState(started.state);
      setSelectedThreadId(started.thread.id);
      setThreadEntrySelectionSource(null);
      updateMessagesByThread((current) => ({
        ...current,
        [started.thread.id]: current[started.thread.id] || [],
      }));
      setNewThreadDraftActive(false);
      setPendingWorkspacePath(null);
      setPendingWorkspaceMode("local");
      setPendingBotId(null);
      setPendingWorkflowId(null);
      setPendingAgentId("claude");
      clearComposerDraft();
      setContentView("thread");
      scheduleHistoryRefresh(started.thread.id, 4, 500);
    } catch (workflowError) {
      setError(
        workflowError instanceof Error
          ? workflowError.message
          : "Failed to start workflow thread",
      );
    } finally {
      setWorkflowThreadStarting(false);
      newThreadInitialDispatchLockRef.current = false;
    }
  }

  async function handleStartDispatch() {
    const startingNewThread = !selectedThreadId;
    const prompt = composerDraftRef.current.trim();
    const promptImages = [...composerImages];
    const promptFiles = [...composerFiles];
    const hasPromptPayload =
      Boolean(prompt) || promptImages.length > 0 || promptFiles.length > 0;

    if (
      isActiveSendingThread ||
      composerAttachmentUploadPending ||
      (startingNewThread && newThreadInitialDispatchLockRef.current)
    ) {
      if (composerAttachmentUploadPending) {
        setError("Attachments are still uploading to gateway.");
      }
      return;
    }

    if (startingNewThread && pendingWorkflowId) {
      if (!hasPromptPayload) {
        return;
      }
      await handleStartWorkflowThreadFromComposer({
        prompt,
        promptFiles,
        promptImages,
        workflowId: pendingWorkflowId,
      });
      return;
    }

    if (startingNewThread && hasPromptPayload) {
      newThreadInitialDispatchLockRef.current = true;
    }

    const canSeedNewThreadDraft = Boolean(
      startingNewThread &&
        hasPromptPayload &&
        (pendingWorkspacePath || preferredWorkspaceForNewThread?.available),
    );
    let seededDraftIntentId: string | undefined;

    if (canSeedNewThreadDraft) {
      const draftIntent = buildIntent({
        threadId: NEW_THREAD_DRAFT_THREAD_ID,
        text: prompt,
        images: promptImages,
        files: promptFiles,
        source: "composer_send",
        state: "dispatch_requested",
        dispatchMode: "sync_send",
      });
      const { assistantEntryId } = appendSeededTurn(
        NEW_THREAD_DRAFT_THREAD_ID,
        draftIntent,
      );
      dispatchMessageState({
        type: "intent/created",
        intent: draftIntent,
        enqueue: false,
      });
      setThreadRuntimeState(NEW_THREAD_DRAFT_THREAD_ID, "dispatching_sync", {
        activeIntentId: draftIntent.intentId,
      });
      updateLiveStreamState(NEW_THREAD_DRAFT_THREAD_ID, () => ({
        threadId: NEW_THREAD_DRAFT_THREAD_ID,
        activeIntentId: draftIntent.intentId,
        assistantEntryId,
        pendingAckIntentIds: [],
        streamStatus: "connecting",
      }));
      requestMessagesBottomSnap(NEW_THREAD_DRAFT_THREAD_ID, true);
      seededDraftIntentId = draftIntent.intentId;
      clearComposerDraft();
      setError(null);
    }

    const threadId = await ensureSelectedThreadId();
    if (!threadId) {
      if (seededDraftIntentId) {
        const message = "Failed to create a thread";
        markLocalDispatchFailed(
          NEW_THREAD_DRAFT_THREAD_ID,
          seededDraftIntentId,
          message,
        );
      }
      if (startingNewThread) {
        newThreadInitialDispatchLockRef.current = false;
      }
      return;
    }
    if (seededDraftIntentId) {
      promoteNewThreadDraftState(threadId);
    }
    if (!(await ensureThreadBotRouting(threadId))) {
      if (seededDraftIntentId) {
        markLocalDispatchFailed(
          threadId,
          seededDraftIntentId,
          "Failed to update bot binding",
        );
      }
      if (startingNewThread) {
        newThreadInitialDispatchLockRef.current = false;
      }
      return;
    }

    if (
      !hasPromptPayload &&
      queueIntentIdsForThread(threadId).length === 0
    ) {
      if (startingNewThread) {
        newThreadInitialDispatchLockRef.current = false;
      }
      return;
    }

    let initialIntentId = seededDraftIntentId;
    if (!initialIntentId && hasPromptPayload) {
      const intent = buildIntent({
        threadId,
        text: prompt,
        images: promptImages,
        files: promptFiles,
        source: "composer_send",
        state: "dispatch_requested",
        dispatchMode: "sync_send",
      });
      dispatchMessageState({
        type: "intent/created",
        intent,
        enqueue: false,
      });
      initialIntentId = intent.intentId;
      clearComposerDraft();
    }

    const batch = runQueuedBatch(threadId, initialIntentId);
    if (startingNewThread) {
      void batch.finally(() => {
        newThreadInitialDispatchLockRef.current = false;
      });
    } else {
      void batch;
    }
  }

  function markInterruptedAssistantEntries(
    threadId: string,
    intentIds: string[],
    activeAssistantEntryId?: string | null,
  ) {
    if (!intentIds.length) {
      return;
    }
    const interruptedIntentIds = new Set(intentIds);
    updateMessagesByThread((current) => ({
      ...current,
      [threadId]: (current[threadId] || []).map((entry) => {
        if (entry.role !== "assistant") {
          return entry;
        }
        if (!entry.intentId || !interruptedIntentIds.has(entry.intentId)) {
          return entry;
        }
        const isPendingEntry =
          entry.pending ||
          entry.localState === "optimistic" ||
          entry.id === activeAssistantEntryId;
        if (!isPendingEntry) {
          return entry;
        }
        return {
          ...entry,
          pending: false,
          error: true,
          localState: "interrupted",
          text:
            entry.text ||
            "Run interrupted before Garyx produced a final answer.",
        };
      }),
    }));
  }

  async function handleInterrupt() {
    const threadId = activeThreadId || selectedThreadId;
    if (!threadId) {
      return;
    }

    const runtime = selectThreadRuntime(messageStateRef.current, threadId);
    if (!runtime || !isRuntimeBusy(runtime.state)) {
      return;
    }

    const liveState = getLiveStreamState(threadId);
    const interruptedIntentIds = [
      runtime.activeIntentId,
      ...(liveState?.pendingAckIntentIds || []),
    ].filter((intentId, index, intents): intentId is string => {
      return Boolean(intentId) && intents.indexOf(intentId) === index;
    });

    setThreadRuntimeState(threadId, "interrupting", {
      activeIntentId: runtime.activeIntentId,
      remoteRunId: runtime.remoteRunId,
    });
    for (const intentId of interruptedIntentIds) {
      dispatchMessageState({
        type: "intent/interrupted",
        intentId,
        error: "request interrupted",
      });
    }
    markInterruptedAssistantEntries(
      threadId,
      interruptedIntentIds,
      liveState?.assistantEntryId ?? null,
    );

    await window.garyxDesktop.interruptThread(threadId);
    clearLiveStreamState(threadId);
    dispatchMessageState({
      type: "thread/clear",
      threadId: threadId,
    });
    scheduleHistoryRefresh(threadId, 2, 500);
    const status = await window.garyxDesktop.checkConnection();
    setConnection(status);
  }

  function markIgnoreComposerSubmitWindow(durationMs = 80) {
    ignoreComposerSubmitUntilRef.current = performance.now() + durationMs;
  }

  function handleComposerSubmit() {
    if (composerSubmitLockRef.current) {
      return;
    }
    composerSubmitLockRef.current = true;
    queueMicrotask(() => {
      composerSubmitLockRef.current = false;
    });

    if (isActiveSendingThread && composerHasPayload) {
      void handleQueueCurrentPrompt({
        steerImmediately:
          settingsDraft.followUpBehavior === "steer" && canSteerQueuedPrompt,
      });
      return;
    }
    void handleStartDispatch();
  }

  function renderWorkspaceFileNodes(
    workspacePath: string,
    directoryPath = "",
    depth = 0,
  ): ReactNode {
    const key = workspaceDirectoryKey(workspacePath, directoryPath);
    const state = workspaceDirectories[key];
    const entries = state?.entries || [];

    if (state?.loading && !entries.length) {
      return (
        <div
          className="workspace-file-empty"
          style={{ paddingLeft: `${depth * 14}px` }}
        >
          Loading…
        </div>
      );
    }

    if (state?.error && !entries.length) {
      return (
        <div
          className="workspace-file-empty workspace-file-error"
          style={{ paddingLeft: `${depth * 14}px` }}
        >
          {state.error}
        </div>
      );
    }

    if (!entries.length) {
      return null;
    }

    const nodes: ReactNode[] = [];

    nodes.push(
      ...entries.map((entry) => {
        const childKey = workspaceDirectoryKey(workspacePath, entry.path);
        const isExpanded = expandedWorkspaceDirectories[childKey] === true;
        const isSelected =
          selectedWorkspaceFile?.workspacePath === workspacePath &&
          selectedWorkspaceFile?.path === entry.path;

        return (
          <div
            className="workspace-file-node-shell"
            key={`${workspacePath}:${entry.path}`}
          >
            <button
              className={`workspace-file-node ${isSelected ? "active" : ""}`}
              onClick={() => {
                void handleWorkspaceFileEntryActivate(entry);
              }}
              style={{ paddingLeft: `${10 + depth * 16}px` }}
              title={entry.path || entry.name}
              type="button"
            >
              <WorkspaceFileIcon entry={entry} open={isExpanded} />
              <span className="workspace-file-node-copy">
                <span className="workspace-file-node-name">{entry.name}</span>
              </span>
            </button>
            {entry.entryType === "directory" && isExpanded ? (
              <div className="workspace-file-children">
                {renderWorkspaceFileNodes(workspacePath, entry.path, depth + 1)}
              </div>
            ) : null}
          </div>
        );
      }),
    );

    return nodes;
  }

  const workspaceDirectoryPanel = activeWorkspacePath ? (
    <>
      <input
        className="workspace-upload-input"
        multiple
        onChange={(event) => {
          const files = Array.from(event.target.files || []);
          if (!files.length) {
            return;
          }
          void uploadWorkspaceFilesToActiveWorkspace(files);
          event.target.value = "";
        }}
        ref={workspaceUploadInputRef}
        tabIndex={-1}
        type="file"
      />
      <div
        className="workspace-directory-tree"
        onDragOver={(event) => {
          if (event.dataTransfer.types.includes("Files")) {
            event.preventDefault();
            event.dataTransfer.dropEffect = "copy";
          }
        }}
        onDrop={(event) => {
          const files = Array.from(event.dataTransfer.files || []);
          if (!files.length) {
            return;
          }
          event.preventDefault();
          event.stopPropagation();
          void uploadWorkspaceFilesToActiveWorkspace(files);
        }}
      >
        {renderWorkspaceFileNodes(activeWorkspacePath)}
      </div>
    </>
  ) : null;

  if (loading) {
    return (
      <I18nProvider languagePreference={settingsDraft.languagePreference}>
        <div className="startup-shell" role="status" aria-live="polite">
          <div className="startup-panel">
            <div className="startup-mark" aria-hidden="true">G</div>
            <div className="startup-copy">
              <strong>{t('Starting Garyx')}</strong>
              <span>{t('Syncing workspace state and gateway status...')}</span>
            </div>
            <div className="startup-progress" aria-hidden="true" />
          </div>
        </div>
      </I18nProvider>
    );
  }

  const gatewayProfiles = desktopState?.gatewayProfiles ?? [];
  const persistedGatewayUrl = desktopState?.settings.gatewayUrl.trim() || "";
  const gatewayAuthSetupMessage = gatewaySetupMessageForAuthError(
    connection?.error,
  );
  const gatewaySetupMessage = gatewayAuthSetupMessage || localSettingsStatus;
  const requiresGatewaySetup =
    gatewaySetupForced || !persistedGatewayUrl || Boolean(gatewaySetupMessage);

  if (requiresGatewaySetup) {
    const setupMessage = gatewayAuthSetupMessage
      ? t(gatewayAuthSetupMessage)
      : localSettingsStatus
        ? t("Save failed: {message}", { message: t(localSettingsStatus) })
        : t("Set the gateway address and token, then save. Saving verifies the gateway before continuing.");
    const canCancelGatewaySetup = gatewaySetupForced && gatewaySetupCanCancel;

    return (
      <I18nProvider languagePreference={settingsDraft.languagePreference}>
        <div className="loading-shell">
          <div className="loading-panel gateway-setup-panel">
            <span className="eyebrow">{t('Gateway Setup')}</span>
            <h1>{t('Connect this Mac app to your Garyx gateway')}</h1>
            <p>
              {t(
                'Enter the gateway address and token before continuing. The default address is {address}. Create or print the token on the gateway host with {command}.',
                {
                  address: '127.0.0.1:31337',
                  command: 'garyx gateway token',
                },
              )}
            </p>

            <div className="gateway-setup-form">
              <label className="gateway-setup-field">
                <span>{t('Gateway URL')}</span>
                <div className="gateway-url-input-shell">
                  <Input
                    autoCapitalize="off"
                    autoComplete="off"
                    className="gateway-setup-input gateway-url-input-with-history"
                    placeholder="http://127.0.0.1:31337"
                    spellCheck={false}
                    type="text"
                    value={settingsDraft.gatewayUrl}
                    onChange={(event) => {
                      setLocalSettingsStatus(null);
                      setSettingsDraft((current) => ({
                        ...current,
                        gatewayUrl: event.target.value,
                      }));
                    }}
                  />
                  <GatewayProfileHistoryButton
                    profiles={gatewayProfiles}
                    onSelect={(profile) => {
                      setLocalSettingsStatus(null);
                      setSettingsDraft((current) => ({
                        ...current,
                        gatewayUrl: profile.gatewayUrl,
                        gatewayAuthToken: profile.gatewayAuthToken,
                      }));
                    }}
                  />
                </div>
              </label>

              <label className="gateway-setup-field">
                <span>{t('Gateway Token')}</span>
                <Input
                  autoCapitalize="off"
                  autoComplete="off"
                  className="gateway-setup-input"
                  placeholder={t('Run `garyx gateway token` on the gateway host')}
                  spellCheck={false}
                  type="password"
                  value={settingsDraft.gatewayAuthToken}
                  onChange={(event) => {
                    setLocalSettingsStatus(null);
                    setSettingsDraft((current) => ({
                      ...current,
                      gatewayAuthToken: event.target.value,
                    }));
                  }}
                />
              </label>
            </div>

            <p
              className={`gateway-setup-status ${gatewaySetupMessage ? "error" : ""}`}
            >
              {setupMessage}
            </p>

            <div className="gateway-setup-actions">
              {canCancelGatewaySetup ? (
                <button
                  className="gateway-setup-button secondary"
                  disabled={savingSettings}
                  onClick={handleCancelGatewaySetup}
                  type="button"
                >
                  {t("Cancel")}
                </button>
              ) : null}
              <button
                className="gateway-setup-button primary"
                disabled={savingSettings}
                onClick={() => {
                  void handleSaveLocalSettingsNow({
                    requireGatewayConnection: true,
                    reloadGatewaySettings: true,
                  }).then((saved) => {
                    if (saved) {
                      setGatewaySetupForced(false);
                      setGatewaySetupCanCancel(false);
                      gatewaySetupSavedConnectionRef.current = null;
                    }
                  });
                }}
                type="button"
              >
                {savingSettings ? t("Saving...") : t("Save and Continue")}
              </button>
            </div>
          </div>
        </div>
      </I18nProvider>
    );
  }

  return (
    <I18nProvider languagePreference={settingsDraft.languagePreference}>
    <div
      className={appShellClassName}
      style={
        {
          "--spacing-token-sidebar": `${sidebarWidth}px`,
        } as React.CSSProperties
      }
    >
      <ToastViewport onDismiss={dismissToast} toasts={toasts} />
      <AppLeftRail
        activeBotConversationGroupId={
          shouldShowConversationRail ? botConversationGroupId : null
        }
        activeWorkspaceThreadGroupPath={
          shouldShowConversationRail ? workspaceConversationPath : null
        }
        botGroups={visibleBotGroups}
        formatThreadTimestamp={formatThreadTimestamp}
        isAutomationView={isAutomationView}
        isAutoResearchView={isAutoResearchView}
        showAutoResearch={showAutoResearchLab}
        showDreams={showDreamsFeature}
        isAgentsView={isAgentsView}
        isBrowserView={isBrowserView}
        isTeamsView={isTeamsView}
        isSettingsView={isSettingsView}
        isSkillsView={isSkillsView}
        isTasksView={isTasksView || isWorkflowView}
        isDreamsView={isDreamsView}
        onBackToThreads={() => {
          setContentView("thread");
        }}
        onCreateThreadForWorkspace={(workspacePath) => {
          void handleCreateThreadForWorkspace(workspacePath);
        }}
        onNewThread={() => {
          void handleNewThread();
        }}
        onOpenBot={(group) => {
          setBotConversationGroupId((current) =>
            current === group.id ? current : null,
          );
          setWorkspaceConversationPath(null);
          void handleBotClick(group);
        }}
        onOpenPinnedThread={(threadId) => {
          setBotConversationGroupId(null);
          setWorkspaceConversationPath(null);
          void openExistingThread(threadId, "pinned");
        }}
        onUnpinThread={(threadId) => {
          togglePinnedThread(threadId);
        }}
        onArchivePinnedThread={(threadId) => {
          // `handleArchiveBotConversationEndpoint` reads only `threadId` and
          // (optionally) `endpointKey` off the argument; it then looks up every
          // matching endpoint from `desktopState.endpoints` itself. Passing a
          // minimal endpoint shape is enough to drive the archive path from a
          // pinned-row context that doesn't carry a full endpoint.
          void handleArchiveBotConversationEndpoint({
            threadId,
            endpointKey: '',
          } as DesktopChannelEndpoint);
        }}
        onToggleBotConversationGroup={(group) => {
          setWorkspaceConversationPath(null);
          setBotConversationGroupId((current) =>
            current === group.id ? null : group.id,
          );
        }}
        onToggleWorkspaceThreadGroup={(workspacePath) => {
          setBotConversationGroupId(null);
          setWorkspaceConversationPath((current) => {
            const currentKey = current?.trim().toLowerCase() || "";
            const nextKey = workspacePath.trim().toLowerCase();
            return currentKey === nextKey ? null : workspacePath;
          });
        }}
        onAddBot={() => {
          void openAddBotDialog();
        }}
        onAddWorkspace={() => {
          void handleAddWorkspace();
        }}
        onOpenSettings={() => {
          openSettingsView();
        }}
        onSidebarResizeStart={handleSidebarResizeStart}
        sidebarResizing={sidebarResizing}
        onOpenAutoResearch={() => {
          setContentView("auto_research");
        }}
        onOpenBrowser={() => {
          setContentView("browser");
        }}
        onOpenAgents={() => {
          setContentView("agents");
        }}
        onOpenSkills={() => {
          setContentView("skills");
        }}
        onOpenTasks={() => {
          setContentView("tasks");
        }}
        onOpenDreams={() => {
          setContentView("dreams");
        }}
        onRequestRemoveWorkspace={(workspace) => {
          void handleRequestRemoveWorkspace(workspace);
        }}
        onSelectAutomation={(automationId) => {
          void handleSelectAutomation(automationId);
        }}
        onSelectSettingsTab={(tabId) => {
          void handleSelectSettingsTab(tabId);
        }}
        pinnedThreadRows={pinnedThreadRows}
        selectedAutomationId={selectedAutomationId}
        selectedThreadId={botRootSelectedThreadId}
        setWorkspaceMenuOpenPath={setWorkspaceMenuOpenPath}
        settingsActiveTab={settingsActiveTab}
        workspaceMenuOpenPath={workspaceMenuOpenPath}
        workspaceMutation={workspaceMutation}
        workspaceThreadGroups={workspaceThreadGroups}
      />
      {activeBotConversationGroup ? (
        <BotConversationSidebar
          deletingThreadId={deletingThreadId}
          formatThreadTimestamp={formatThreadTimestamp}
          group={activeBotConversationGroup}
          isThreadRuntimeBusy={(threadId) => {
            return isRuntimeBusy(
              selectThreadRuntime(messageState, threadId)?.state,
            );
          }}
          onArchiveEndpoint={(endpoint) => {
            void handleArchiveBotConversationEndpoint(endpoint);
          }}
          onClose={() => {
            setBotConversationGroupId(null);
          }}
          onOpenEndpoint={(endpoint) => {
            void handleOpenThreadFromEndpoint(endpoint, "bot-conversation");
          }}
          onRailResizeStart={handleRailResizeStart}
          railResizing={railResizing}
          selectedThreadId={botConversationSelectedThreadId}
        />
      ) : activeWorkspaceThreadGroup ? (
        <WorkspaceConversationSidebar
          deletingThreadId={deletingThreadId}
          desktopState={desktopState}
          formatThreadTimestamp={formatThreadTimestamp}
          group={activeWorkspaceThreadGroup}
          isThreadRuntimeBusy={(threadId) => {
            return isRuntimeBusy(
              selectThreadRuntime(messageState, threadId)?.state,
            );
          }}
          onClose={() => {
            setWorkspaceConversationPath(null);
          }}
          onDeleteThread={(threadId) => {
            void handleDeleteThread(threadId);
          }}
          onOpenThread={(threadId) => {
            void openExistingThread(threadId, "workspace-conversation");
          }}
          onRailResizeStart={handleRailResizeStart}
          railResizing={railResizing}
          selectedThreadId={workspaceConversationSelectedThreadId}
        />
      ) : null}
      <AddBotDialog
        onClose={() => {
          setAddBotDialogOpen(false);
          setAddBotInitialValues(null);
        }}
        onCreateChannel={handleAddChannelAccount}
        onPollWeixinAuth={handlePollWeixinChannelAuth}
        onStartWeixinAuth={handleStartWeixinChannelAuth}
        onStartFeishuAuth={handleStartFeishuChannelAuth}
        onPollFeishuAuth={handlePollFeishuChannelAuth}
        open={addBotDialogOpen}
        initialValues={addBotInitialValues}
        agentTargets={addBotAgentTargets}
        workspaces={workspacePickerWorkspaces}
        onAddWorkspace={addWorkspacePathFromPicker}
      />
      <WorkspacePathPickerDialog
        open={Boolean(addWorkspaceDialog)}
        title={t("Add Workspace")}
        description={t("Choose a folder")}
        initialPath={addWorkspaceDialog?.initialPath || ""}
        saving={workspaceMutation === "add"}
        workspaces={workspacePickerWorkspaces}
        onCancel={() => closeAddWorkspaceDialog(null)}
        onConfirm={confirmAddWorkspace}
      />

      {isBrowserView ? (
        <main className="conversation browser-view">
          <Suspense
            fallback={
              <div className="view-loading-fallback">
                {t("Loading…")}
              </div>
            }
          >
            <BrowserPage />
          </Suspense>
        </main>
      ) : (
        <main className={conversationClassName}>
          {isTasksView || isWorkflowView || isDreamsView ? null : showStaticWindowToolbar ? (
            <div aria-hidden="true" className="settings-window-toolbar" />
          ) : (
            <header className="conversation-header">
              <ConversationHeaderTitle
                activeThreadBot={activeThreadBot}
                activeThreadTitle={activeThread?.title || null}
                activeWorkspaceName={activeWorkspace?.name || null}
                canEditThreadTitle={canEditThreadTitle}
                contextText={conversationContextText}
                editingThreadTitle={editingThreadTitle}
                isAutomationView={isAutomationView}
                isBotsView={isBotsView}
                isSkillsView={isSkillsView}
                isThreadPinned={selectedThreadPinned}
                archiveThreadDisabled={Boolean(
                  !selectedThreadId ||
                    activeAutomationThread ||
                    isRuntimeBusy(activeRuntime?.state),
                )}
                onBeginEdit={beginThreadTitleEdit}
                onArchiveThread={() => {
                  void handleDeleteThread();
                }}
                onCancelEdit={cancelThreadTitleEdit}
                onSaveTitle={() => {
                  void handleSaveTitle({ closeEditor: true });
                }}
                onTogglePinnedThread={() => {
                  if (selectedThreadId) {
                    togglePinnedThread(selectedThreadId);
                  }
                }}
                onTitleDraftChange={setTitleDraft}
                savingTitle={savingTitle}
                titleDraft={titleDraft}
                titleInputRef={threadTitleInputRef}
              />
              <ConversationHeaderActions
                gatewayStatusLabel={gatewayIndicator?.label || null}
                gatewayStatusTone={gatewayIndicator?.tone || null}
                hasWorkspaceDirectory={Boolean(activeWorkspacePath)}
                inspectorOpen={inspectorOpen}
                isAutomationView={isAutomationView}
                isBotsView={isBotsView}
                isSkillsView={isSkillsView}
                selectedThreadId={selectedThreadId}
                teamSummary={activeTeamSummary}
                threadInfo={activeThreadInfo}
                threadInfoLoaded={activeThreadInfoLoaded}
                threadLogsHasUnread={threadLogsHasUnread || clientLogsHasUnread}
                threadLogsOpen={threadLogsOpen}
                onCreateAutomation={() => {
                  openAutomationDialog("create");
                }}
                onOpenThread={(threadId) => {
                  void openExistingThread(threadId);
                }}
                onOpenThreads={() => {
                  setContentView("thread");
                }}
                onToggleInspector={() => {
                  setThreadLogsOpen(false);
                  setInspectorOpen((current) => !current);
                }}
                onToggleThreadLogs={() => {
                  setInspectorOpen(false);
                  setThreadLogsOpen((current) => !current);
                }}
              />
            </header>
          )}
          <section
            className={`conversation-body ${isSettingsView ? "settings-layout" : ""}`}
          >
            <Suspense
              fallback={
                <div className="view-loading-fallback">
                  {t("Loading…")}
                </div>
              }
            >
            {isSettingsView ? (
              <div className="settings-page">
                <SettingsErrorBoundary
                  activeTab={settingsActiveTab}
                  onRetry={handleRetrySettingsView}
                >
                  <GatewaySettingsPanel
                    activeTab={settingsActiveTab}
                    agents={desktopAgents}
                    teams={desktopTeams}
                    commands={commands}
                    commandsLoading={commandsLoading}
                    commandsSaving={commandsSaving}
                    connection={connection}
                    gatewayDirty={gatewaySettingsDirty}
                    gatewayDraft={gatewaySettingsDraft}
                    gatewayLoading={gatewaySettingsLoading}
                    gatewaySettingsSource={gatewaySettingsSource}
                    gatewaySaving={gatewaySettingsSaving}
                    gatewayStatusMessage={gatewaySettingsStatus}
                    localSettingsDirty={localSettingsDirty}
                    localSettings={settingsDraft}
                    workspaces={workspacePickerWorkspaces}
                    onAddWorkspace={addWorkspacePathFromPicker}
                    mcpServers={mcpServers}
                    mcpServersLoading={mcpServersLoading}
                    mcpServersSaving={mcpServersSaving}
                    onCreateMcpServer={(input) => {
                      return handleCreateMcpServer(input);
                    }}
                    onCreateSlashCommand={(input) => {
                      return handleCreateSlashCommand(input);
                    }}
                    onDeleteMcpServer={(name) => {
                      return handleDeleteMcpServer(name);
                    }}
                    onAddChannelAccount={handleAddChannelAccount}
                    onStartWeixinChannelAuth={handleStartWeixinChannelAuth}
                    onPollWeixinChannelAuth={handlePollWeixinChannelAuth}
                    onStartFeishuChannelAuth={handleStartFeishuChannelAuth}
                    onPollFeishuChannelAuth={handlePollFeishuChannelAuth}
                    onDeleteSlashCommand={(name) => {
                      return handleDeleteSlashCommand(name);
                    }}
                    onLocalSettingsChange={setSettingsDraft}
                    onMutateGatewayDraft={mutateGatewaySettingsDraft}
                    onSaveLocalSettingsNow={(options) => {
                      return handleSaveLocalSettingsNow(options);
                    }}
                    onSaveLocalSettingsDraft={(nextSettings, options) => {
                      return handleSaveLocalSettingsDraft(nextSettings, options);
                    }}
                    onSaveGatewaySettings={(options) => {
                      return handleSaveGatewaySettings(options);
                    }}
                    onSaveGatewaySettingsPatch={(patch, options) => {
                      return handleSaveGatewaySettingsPatch(patch, options);
                    }}
                    onOpenGatewaySetup={() => {
                      void handleOpenGatewaySetup();
                    }}
                    onRefreshAgentTargets={refreshAgentTargets}
                    onToggleMcpServer={handleToggleMcpServer}
                    onUpdateMcpServer={(input) => {
                      return handleUpdateMcpServer(input);
                    }}
                    onUpdateSlashCommand={(input) => {
                      return handleUpdateSlashCommand(input);
                    }}
                    savingLocalSettings={savingSettings}
                  />
                </SettingsErrorBoundary>
              </div>
            ) : isAutomationView ? (
              <AutomationListPage
                automations={automations}
                agents={desktopAgents}
                desktopState={desktopState}
                automationMutation={automationMutation}
                onRunNow={(a) => {
                  void handleRunAutomationNow(a);
                }}
                onToggleEnabled={(a, enabled) => {
                  void handleToggleAutomationEnabled(a, enabled);
                }}
                onEdit={(a) => {
                  openAutomationDialog("edit", a);
                }}
                onOpenMemory={(a) => {
                  void openMemoryDialog({
                    scope: "automation",
                    automationId: a.id,
                    title: `${a.label} memory.md`,
                  });
                }}
                onOpenThread={(a) => {
                  void handleOpenAutomationThread(a);
                }}
                onDelete={(a) => {
                  void handleDeleteAutomation(a);
                }}
                onCreateAutomation={() => {
                  openAutomationDialog("create");
                }}
              />
            ) : isAutoResearchView ? (
              <AutoResearchPanel
                currentWorkspace={selectedWorkspaceEntry}
                loading={autoResearchLoading}
                onCreateRun={async (input) => {
                  await createAutoResearchRun(input);
                }}
                onRefresh={async (runId) => {
                  await loadAutoResearchRun(runId);
                }}
                onOpenThread={(threadId) => {
                  void openExistingThread(threadId);
                }}
                onSelectRun={async (runId) => {
                  await loadAutoResearchRun(runId);
                }}
                onStop={async (runId) => {
                  await stopAutoResearchRun(runId);
                }}
                onDelete={async (runId) => {
                  await deleteAutoResearchRun(runId);
                }}
                onSelectCandidate={async (runId, candidateId) => {
                  await selectAutoResearchCandidate(runId, candidateId);
                }}
                iterations={autoResearchIterations}
                candidatesResponse={autoResearchCandidatesResponse}
                workspaces={desktopState?.workspaces || []}
                onAddWorkspace={addWorkspacePathFromPicker}
                runs={autoResearchRuns}
                runDetail={autoResearchRunDetail}
                saving={autoResearchSaving}
              />
            ) : isAgentsView ? (
              <AgentsHubPanel
                initialTab="agents"
                workspaces={workspacePickerWorkspaces}
                onAddWorkspace={addWorkspacePathFromPicker}
                onOpenMemory={(agent) => {
                  void openMemoryDialog({
                    scope: "agent",
                    agentId: agent.agentId,
                    title: `${agent.displayName || agent.agentId} memory.md`,
                  });
                }}
                onStartThread={handleStartDraftForAgent}
                onToast={pushToast}
              />
            ) : isTeamsView ? (
              <AgentsHubPanel
                initialTab="teams"
                workspaces={workspacePickerWorkspaces}
                onAddWorkspace={addWorkspacePathFromPicker}
                onOpenMemory={(agent) => {
                  void openMemoryDialog({
                    scope: "agent",
                    agentId: agent.agentId,
                    title: `${agent.displayName || agent.agentId} memory.md`,
                  });
                }}
                onStartThread={handleStartDraftForAgent}
                onToast={pushToast}
              />
            ) : isSkillsView ? (
              <SkillsPanel onToast={pushToast} />
            ) : isTasksView ? (
              <TasksPanel
                agents={desktopAgents}
                botGroups={botGroups}
                onAddWorkspace={addWorkspacePathFromPicker}
                onOpenThread={(threadId) => {
                  void openExistingThread(threadId);
                }}
                onOpenWorkflowTask={openWorkflowTask}
                onToast={pushToast}
                workspaces={workspacePickerWorkspaces}
                workspaceMutation={workspaceMutation}
              />
            ) : isWorkflowView && selectedWorkflowTaskId ? (
              <WorkflowRunsPanel
                onOpenTasks={() => {
                  setContentView("tasks");
                }}
                onOpenThread={(threadId) => {
                  void openExistingThread(threadId);
                }}
                onToast={pushToast}
                t={t}
                task={selectedWorkflowTask}
                taskId={selectedWorkflowTaskId}
              />
            ) : selectedWorkflowRunThreadId ? (
              <WorkflowRunsPanel
                onOpenThread={(threadId) => {
                  void openExistingThread(threadId);
                }}
                onToast={pushToast}
                t={t}
                workflowRunId={selectedWorkflowRunThreadId}
              />
            ) : isDreamsView ? (
              <DreamsPanel
                onOpenThread={(threadId) => {
                  void openExistingThread(threadId, "dreams");
                }}
              />
            ) : isBotsView ? (
              <BotConsolePage
                busyBotId={
                  bindingMutation === "bot-binding" ? activeThreadBotId : null
                }
                groups={botGroups}
                onCreateThread={(group) => {
                  void handleBotClick(group);
                }}
                onOpenSettings={() => {
                  openSettingsView();
                }}
                onOpenThread={(threadId) => {
                  const endpoint = botGroups
                    .flatMap((group) => group.endpoints)
                    .find((item) => item.threadId === threadId);
                  if (endpoint) {
                    void handleOpenThreadFromEndpoint(endpoint);
                  }
                }}
                totalEndpoints={desktopState?.endpoints.length || 0}
              />
            ) : (
              <ThreadPage
                agentLabel={composerAgentLabel}
                composerAgentOptions={composerAgentOptions}
                composerWorkflowOptions={composerWorkflowOptions}
                composerWorkflowOptionsLoading={workflowDefinitionsLoading}
                activeMessages={activeMessages}
                activePendingAckIntents={visiblePendingAckIntents}
                activePendingAutomationRun={activePendingAutomationRun}
                activeToolTraceLoadingKey={activeToolTraceLoadingKey}
                activeQueue={activeQueue}
                activeRenderableBlocks={activeRenderableBlocks}
                activeThreadLogsHasUnread={activeThreadLogsHasUnread}
                activeThreadLogsPath={activeThreadLogsPath}
                activeThreadSummary={activeThread || null}
                activeThreadTitle={activeThread?.title || null}
                activeThreadRunId={activeThreadRunId}
                availableWorkspaceCount={availableWorkspaceCount}
                clientThreadLogEntries={clientThreadLogEntries}
                composer={composer}
                composerAttachmentInputRef={composerAttachmentInputRef}
                composerFiles={composerFiles}
                composerHasPayload={composerHasPayload}
                composerImages={composerImages}
                composerLocked={composerLocked}
                composerPlaceholder={composerPlaceholder}
                composerProviderType={composerProviderType}
                composerResetKey={composerResetKey}
                composerWorkspaceBranch={composerWorkspaceBranch}
                composerWorkspaceMode={composerWorkspaceMode}
                activeThreadBot={activeThreadBot}
                activeThreadBotId={activeThreadBotId}
                botBindingDisabled={bindingMutation === "bot-binding"}
                botGroups={botGroups}
                slashCommands={commands}
                slashCommandsLoaded={commandsLoaded}
                slashCommandsLoading={commandsLoading}
                composerTextareaRef={composerTextareaRef}
                draggedQueueIntentId={draggedQueueIntentId}
                expandedClientLogEntries={expandedClientLogEntries}
                historyLoading={historyLoading}
                historyLoadingEarlier={Boolean(
                  activeHistoryPagination?.loadingBefore,
                )}
                ignoreComposerSubmitUntilRef={ignoreComposerSubmitUntilRef}
                inspectorOpen={inspectorOpen}
                isActiveSendingThread={isActiveSendingThread}
                canSteerQueuedPrompt={canSteerQueuedPrompt}
                isComposingRef={isComposingRef}
                messagesRef={messagesRef}
                mobileThreadLogLines={mobileThreadLogLines}
                newThreadSelectedAgentId={pendingAgentId}
                newThreadSelectedWorkflowId={pendingWorkflowId}
                newThreadWorkspaceEntry={newThreadWorkspaceEntry}
                newThreadWorkspaceMode={pendingWorkspaceMode}
                onAddWorkspace={() => {
                  void handleAddWorkspace();
                }}
                onAppendComposerAttachments={(files) => {
                  void appendComposerAttachments(files);
                }}
                onCancelIntent={(threadId, intentId) => {
                  dispatchMessageState({
                    type: "intent/cancelled",
                    threadId,
                    intentId,
                  });
                }}
                onComposerChange={(value) => {
                  composerDraftRef.current = value;
                  const nextTextPresent = value.trim().length > 0;
                  setComposerTextPresent((current) =>
                    current === nextTextPresent ? current : nextTextPresent,
                  );
                  if (
                    /^\/[a-z0-9_]*$/i.test(value) &&
                    !commandsLoaded &&
                    !commandsLoading
                  ) {
                    void loadSlashCommands();
                  }
                  syncComposerPhase(value);
                }}
                onComposerCompositionEnd={(value) => {
                  isComposingRef.current = false;
                  syncComposerPhase(value, false);
                  markIgnoreComposerSubmitWindow();
                }}
                onComposerCompositionStart={() => {
                  isComposingRef.current = true;
                  syncComposerPhase(composerDraftRef.current, true);
                }}
                onComposerInterrupt={() => {
                  void handleInterrupt();
                }}
                onComposerSubmit={handleComposerSubmit}
                onJumpToLatestThreadLogs={() => {
                  if (threadLogsActiveTab === "client") {
                    setClientLogsHasUnread(false);
                  } else {
                    setThreadLogsHasUnread(false);
                  }
                  scrollThreadLogsToLatest("smooth");
                }}
                onLocalWorkspaceFileLinkClick={handleLocalFileLinkClick}
                onMarkIgnoreComposerSubmitWindow={
                  markIgnoreComposerSubmitWindow
                }
                onMessagesScroll={() => {
                  const node = messagesRef.current;
                  shouldStickMessagesToBottomRef.current =
                    messagesNearBottom(node);
                  if (
                    selectedThreadId &&
                    node &&
                    messagesNearEarlierUserTurnBoundary(node)
                  ) {
                    void loadOlderThreadHistoryPage(selectedThreadId);
                  }
                }}
                onQueueDropTargetChange={setQueueDropTarget}
                onRemoveComposerFile={removeComposerFile}
                onRemoveComposerImage={removeComposerImage}
                onReorderQueuedIntent={reorderQueuedIntent}
                onSelectNewThreadAgent={(agentId) => {
                  setPendingAgentId(agentId);
                  setPendingWorkflowId(null);
                }}
                onSelectNewThreadWorkflow={(workflowId) => {
                  setPendingWorkflowId(workflowId);
                  setPendingAgentId("claude");
                }}
                onSelectNewThreadWorkspaceMode={setPendingWorkspaceMode}
                onResumeProviderSession={handleResumeProviderSession}
                onSelectBotBinding={(botId) => {
                  if (selectedThreadId) {
                    const threadId = selectedThreadId;
                    setOptimisticThreadBotBinding({ threadId, botId });
                    void handleSetBotBinding(botId).finally(() => {
                      setOptimisticThreadBotBinding((current) => {
                        return current?.threadId === threadId &&
                          current.botId === botId
                          ? null
                          : current;
                      });
                    });
                  } else {
                    setPendingBotId(botId);
                  }
                }}
                onSelectThreadLogsTab={setThreadLogsActiveTab}
                onOpenThreadById={(threadId) => {
                  void openExistingThread(threadId);
                }}
                onSelectWorkspace={(workspacePath) => {
                  setPendingWorkspaceMode("local");
                  void handleSelectWorkspace(workspacePath, null);
                }}
                onSetDraggedQueueIntentId={setDraggedQueueIntentId}
                onSteerQueuedPrompt={(item) => {
                  void handleSteerQueuedPrompt(item);
                }}
                onThreadLogsContentScroll={() => {
                  if (threadLogsNearBottom()) {
                    if (threadLogsActiveTab === "client") {
                      setClientLogsHasUnread(false);
                    } else {
                      setThreadLogsHasUnread(false);
                    }
                  }
                }}
                onThreadLogsResizeKeyDown={handleThreadLogsResizeKeyDown}
                onThreadLogsResizeStart={handleThreadLogsResizeStart}
                onToggleClientLogEntry={(entryKey) => {
                  setExpandedClientLogEntries((current) => ({
                    ...current,
                    [entryKey]: !current[entryKey],
                  }));
                }}
                preferredWorkspaceForNewThread={preferredWorkspaceForNewThread}
                queueDropTarget={queueDropTarget}
                selectableNewThreadWorkspaces={selectableNewThreadWorkspaces}
                selectedThreadId={selectedThreadId}
                showAutomationRunInitialPlaceholder={
                  showAutomationRunInitialPlaceholder
                }
                showDreams={showDreamsFeature}
                showAutomationRunTailLoading={showAutomationRunTailLoading}
                showHistoryLoadingPlaceholder={showHistoryLoadingPlaceholder}
                showPendingAckLoading={showPendingAckLoading}
                threadLayoutRef={threadLayoutRef}
                threadLayoutStyle={
                  threadLogsOpen
                    ? ({
                        "--thread-log-panel-width": `${threadLogsPanelWidth}px`,
                      } as React.CSSProperties)
                    : undefined
                }
                threadLogsActiveTab={threadLogsActiveTab}
                threadLogsError={threadLogsError}
                threadLogsLoading={threadLogsLoading}
                threadLogsMaxWidth={clampThreadLogsPanelWidth(
                  THREAD_LOG_PANEL_MAX_WIDTH,
                  currentThreadLayoutWidth(),
                )}
                threadLogsOpen={threadLogsOpen}
                threadLogsPanelWidth={threadLogsPanelWidth}
                threadLogsRef={threadLogsRef}
                threadLogsResizing={threadLogsResizing}
                teamAgentDisplayNamesById={teamAgentDisplayNamesById}
                visibleRemoteAwaitingAckInputs={visibleRemoteAwaitingAckInputs}
                visibleRemotePendingInputs={visibleRemotePendingInputs}
                workspaceDirectoryPanel={workspaceDirectoryPanel}
                workspaceMutation={workspaceMutation}
              />
            )}
            </Suspense>
          </section>
        </main>
      )}
      {workspacePreviewModalOpen ? (
        <Suspense fallback={null}>
          <WorkspacePreviewModal
            error={workspaceFilePreviewError}
            loading={workspaceFilePreviewLoading}
            onClose={() => {
              setWorkspacePreviewModalOpen(false);
            }}
            onLocalFileLinkClick={handleLocalFileLinkClick}
            open={workspacePreviewModalOpen}
            preview={workspaceFilePreview}
            title={workspacePreviewTitle}
          />
        </Suspense>
      ) : null}

      {automationDialog ? (
        <Suspense fallback={null}>
          <AutomationDialog
            state={automationDialog}
            agentOptions={automationAgentOptions}
            threadOptions={desktopState?.threads || []}
            workspaces={workspacePickerWorkspaces}
            onAddWorkspace={addWorkspacePathFromPicker}
            saving={
              automationMutation === "create" ||
              automationMutation === `edit:${automationDialog.automationId || ""}`
            }
            onDraftChange={updateAutomationDialogDraft}
            onSubmit={(event) => {
              void handleSubmitAutomationDialog(event);
            }}
            onClose={() => {
              setAutomationDialog(null);
            }}
          />
        </Suspense>
      ) : null}

      {memoryDialogTarget ? (
        <Suspense fallback={null}>
          <MemoryDialog
            dirty={memoryDialogDirty}
            draftContent={memoryDialogDraft}
            error={memoryDialogError}
            exists={memoryDialogDocument?.exists ?? false}
            loading={memoryDialogLoading}
            modifiedAt={memoryDialogDocument?.modifiedAt ?? null}
            onClose={closeMemoryDialog}
            onDraftChange={setMemoryDialogDraft}
            onSave={() => {
              void saveMemoryDialog();
            }}
            open={Boolean(memoryDialogTarget)}
            path={memoryDialogDocument?.path || null}
            saving={memoryDialogSaving}
            scope={memoryDialogTarget?.scope || "agent"}
            status={memoryDialogStatus}
            title={memoryDialogTarget?.title || "memory.md"}
          />
        </Suspense>
      ) : null}
    </div>
    </I18nProvider>
  );
}
