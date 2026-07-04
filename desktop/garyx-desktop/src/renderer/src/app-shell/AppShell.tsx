import {
  Suspense,
  lazy,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type CSSProperties,
  type ReactNode,
} from "react";
import { startTransition } from "react";
import { PanelLeft } from "lucide-react";

import {
  DEFAULT_SESSION_TITLE,
  type CreateAutomationInput,
  type DesktopApiProviderType,
  type DesktopAutomationActivityEntry,
  type DesktopAutomationActivityFeed,
  type DesktopMcpServer,
  type DesktopAutomationSchedule,
  type DesktopBotConsoleSummary,
  type DesktopCustomAgent,
  type DesktopTeam,
  type GatewaySettingsPayload,
  type GatewaySettingsSource,
  type ConfiguredBot,
  type ConnectionStatus,
  type DesktopChannelEndpoint,
  type DesktopProviderModels,
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
  type RenderState,
  type SlashCommand,
  type ThreadRuntimeInfo,
  type TranscriptMessage,
  type UpdateMcpServerInput,
  type UpdateSlashCommandInput,
  type UpsertMcpServerInput,
  type UpsertSlashCommandInput,
} from "@shared/contracts";
import { desktopStateWithoutThread } from "@shared/desktop-state";
import {
  extractToolUseId,
  isToolRole,
  shouldRestartSelectedThreadStreamAfterRefetch,
} from "@shared/transcript-sync";

import {
  initialMessageMachineState,
  isRuntimeBusy,
  selectGlobalActiveThreadId,
  selectQueueIntentIds,
  selectThreadRuntime,
  type MessageMachineAction,
  type MessageIntent,
} from "../message-machine";
import type { SettingsTabId } from "../settings-tabs";
import { GatewayProfileHistoryButton } from "../GatewayProfileHistoryButton";
import { GatewayHeadersEditor } from "../GatewayHeadersEditor";
import { GatewayIdentityBar } from "../GatewaySwitcher";
import { SettingsErrorBoundary } from "../SettingsErrorBoundary";
import { Input } from "../components/ui/input";
import { WorkspacePathPickerDialog } from "../components/WorkspacePathPicker";
// Side-effect import: wires cross-store capsule cache invalidation (a `/serve`
// 404 in either the HTML or thumbnail store tombstones the other for that id).
import "./capsule-cache";
import { AddBotDialog } from "./components/AddBotDialog";
import { DreamsPanel } from "./components/DreamsPanel";
import {
  ThreadSideToolsPanel,
  type SideCapsuleTab,
  type SideToolWorkspaceFile,
} from "./components/SideToolsPanel";
import { BotConversationSidebar } from "../BotConversationSidebar";
import { WorkspaceConversationSidebar } from "../WorkspaceConversationSidebar";
import { ThreadConversationSidebar } from "../ThreadConversationSidebar";
import { buildComposerWorkflowOptions } from "../ComposerForm";
import { ComposerQueue } from "../ComposerQueue";
import { ConversationHeaderActions } from "../ConversationHeaderActions";
import { ConversationHeaderTitle } from "../ConversationHeaderTitle";
import { NewThreadEmptyState } from "../NewThreadEmptyState";
import { ToastViewport, type ToastItem, type ToastTone } from "../toast";
import { ToolTraceGroup } from "../tool-trace";
import {
  RichMessageContent,
} from "../message-rich-content";
import {
  deriveThreadComposerControlModel,
  deriveThreadActivityModel,
} from "./thread-activity";
import {
  visibleRemotePendingInputsForThread,
  type PendingInputOriginRef,
} from "./pending-inputs";
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
  visibleWorkspaceList,
  workspaceForThread,
  workspaceSuggestionFromPath,
} from "../thread-model";
import {
  buildThreadAvatarCatalog,
  resolveThreadAvatarIdentity,
} from "../thread-avatar";
import {
  bindEndpointToThread,
  detachEndpointFromThread,
  ensureWorkspaceForNewThread,
  ensureThread,
  saveThreadTitle,
  scheduleThreadHistoryRefresh,
  selectWorkspaceForThread,
  startNewThreadDraft,
  updateThreadBotBinding,
} from "../thread-controller";
import {
  AutomationIcon,
  BackIcon,
  NewThreadIcon,
  RecentIcon,
  SettingsIcon,
  SkillsIcon,
  WorkspaceFileIcon,
  isLocalSettingsTab,
} from "./icons";
import type {
  AutomationDraft,
  AutomationDialogState,
  BoundBot,
  ContentView,
  GatewayIndicatorTone,
  LiveStreamState,
  LiveStreamStatus,
  MessageMap,
  PendingAutomationRun,
  PendingThreadInputMap,
  ThreadLogLine,
  UiTranscriptMessage,
  WorkspaceDirectoryState,
} from "./types";
import { AppLeftRail } from "./components/AppLeftRail";
import { ThreadPage } from "./components/ThreadPage";
import { useAutomationController } from "./useAutomationController";
import {
  SIDE_TOOLS_PANEL_MAX_WIDTH,
  SIDE_TOOLS_PANEL_MIN_WIDTH,
  THREAD_LOG_PANEL_MAX_WIDTH,
  buildThreadLogLines,
  clampSideToolsPanelWidth,
  clampThreadLogsPanelWidth,
  computeGatewayIndicator,
  keepRecentThreadLogLines,
} from "./diagnostics-helpers";
import {
  isKnownThreadId,
  useDeepLinkRouteController,
  waitForMs,
} from "./useDeepLinkRouteController";
import { useGatewayConnectionController } from "./useGatewayConnectionController";
import { useLayoutResizeController } from "./useLayoutResizeController";
import {
  resolveMemoryDialogTargetFromPath,
  useMemoryDialogController,
} from "./useMemoryDialogController";
import {
  NEW_THREAD_DRAFT_THREAD_ID,
  browserAnnotationScreenshotImages,
  composePromptWithBrowserAnnotations,
  prepareAttachmentUploads,
  useMessageDispatchController,
  type SeededTurn,
} from "./useMessageDispatchController";
import { useMessagesScrollController } from "./useMessagesScrollController";
import { GatewayMirror } from "../gateway-mirror/mirror";
import type { DispatchOrchestratorDeps } from "../gateway-mirror/dispatch-orchestrator";
import { GatewayMirrorContext } from "../gateway-mirror/react";
import { useSettingsController } from "./useSettingsController";
import { useSideChatController } from "./useSideChatController";
import {
  SELECTED_THREAD_STREAM_CONSUMER_ID,
  messagesNearEarlierUserTurnBoundary,
  normalizeMessageText,
  transcriptHasAutomationResponse,
  transcriptMessageMatchesIntent,
  useTranscriptController,
  type ThreadHistoryPaginationState,
} from "./useTranscriptController";
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
  summarizeRemoteStateErrors,
} from "./gateway-errors";
import { buildAgentOptions, buildAgentTargetOptions } from "./agent-options";
import {
  I18nProvider,
  createTranslator,
  useResolvedLocale,
} from "../i18n";
import { isRunLoadingPlaceholderMessage } from "./loading-labels";
import garyxIconUrl from "../assets/garyx-icon.png";
import {
  contentViewForDesktopRoute,
  parseDesktopRoute,
  type DesktopRoute,
} from "./desktop-route";

const MESSAGES_BOTTOM_THRESHOLD_PX = 48;

type ThreadEntrySelectionSource =
  | "pinned"
  | "recent"
  | "bot-root"
  | "bot-conversation"
  | "workspace-conversation"
  | "dreams"
  | "tasks";

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
const AgentsHubPanel = lazy(() =>
  import("./components/AgentsHubPanel").then((module) => ({
    default: module.AgentsHubPanel,
  })),
);
const TasksPanel = lazy(() =>
  import("./components/TasksPanel").then((module) => ({
    default: module.TasksPanel,
  })),
);
const CapsulesPanel = lazy(() =>
  import("./components/CapsulesPanel").then((module) => ({
    default: module.CapsulesPanel,
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

function pendingInputOriginRefsForThread(
  intentsById: Record<string, MessageIntent>,
  threadId: string | null,
): PendingInputOriginRef[] {
  if (!threadId) {
    return [];
  }
  return Object.values(intentsById).flatMap((intent) => {
    if (intent.threadId !== threadId) {
      return [];
    }
    const pendingInputId = intent.pendingInputId?.trim() || "";
    const originId = intent.intentId.trim();
    return pendingInputId && originId
      ? [
          {
            pendingInputId,
            originId,
          },
        ]
      : [];
  });
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

const STARTUP_HYDRATION_RETRY_DELAYS_MS = [0, 300, 650, 1_100, 1_700];
const TRANSIENT_STATUS_MS = 3200;
const ERROR_TOAST_MS = 4400;

function threadRunStateIsRunning(thread: DesktopThreadSummary): boolean {
  return (thread.runState || "").trim().toLowerCase() === "running";
}

function savedContentView(): ContentView {
  const saved = sessionStorage.getItem("gary-content-view");
  const valid: ContentView[] = [
    "thread",
    "browser",
    "bots",
    "automation",
    "capsules",
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
    runtimeProvider === "antigravity" ||
    runtimeProvider === "traex" ||
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
  if (agentId === "antigravity") {
    return "antigravity";
  }
  if (agentId === "traex") {
    return "traex";
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
  // Endgame architecture (docs/design/appshell-endgame-architecture.md):
  // the mirror instance is created once and provided via context. During
  // the migration it runs alongside the legacy React state; batches move
  // ownership over domain by domain.
  const [gatewayMirror] = useState(
    () =>
      new GatewayMirror({
        getState: () => window.garyxDesktop.getState(),
        listCustomAgents: () => window.garyxDesktop.listCustomAgents(),
        listTeams: () => window.garyxDesktop.listTeams(),
        listWorkflowDefinitions: () =>
          window.garyxDesktop.listWorkflowDefinitions(),
        getThreadHistory: (input) => window.garyxDesktop.getThreadHistory(input),
        // Temporary batch-2/3 seams: the message machine and the
        // authoritative-refetch flow stay with their legacy owners; the
        // mirror reaches them through these injected lookups. The closures
        // run post-mount, so the later declarations they capture are
        // initialized by the time they are read.
        intentForId: (intentId) =>
          messageStateRef.current.intentsById[intentId] || null,
        requestAuthoritativeRefetch: () => {
          // Batch 2b dual-run: the legacy stream handler is still the sole
          // rewrite-refetch trigger (applyCommittedThreadMessage), and its
          // result flows back into the mirror through the
          // applyRemoteTranscript dual-write. Triggering here as well would
          // double-run the refetch (two concurrent history fetches + stream
          // restarts). Ownership flips to the mirror when the legacy path
          // is deleted (batch 6).
        },
      }),
  );
  const [desktopState, setDesktopState] = useState<DesktopState | null>(null);
  const [desktopAgents, setDesktopAgents] = useState<DesktopCustomAgent[]>([]);
  const [desktopTeams, setDesktopTeams] = useState<DesktopTeam[]>([]);
  const [desktopWorkflows, setDesktopWorkflows] = useState<
    DesktopWorkflowDefinition[]
  >([]);
  const [workflowDefinitionsLoading, setWorkflowDefinitionsLoading] =
    useState(false);
  const [connection, setConnection] = useState<ConnectionStatus | null>(null);
  const [selectedThreadId, setSelectedThreadId] = useState<string | null>(() =>
    initialRouteValue.kind === "thread" ? initialRouteValue.threadId : null,
  );
  // Capsule preview selection lives here (single source of truth) so the route,
  // deep links, and gallery clicks all flow through one path.
  // Seed it from the initial route so a cold-started #/capsules/<id> lands on
  // the preview instead of being rewritten to #/capsules by the replace effect.
  const [capsulePreviewId, setCapsulePreviewId] = useState<string | null>(() =>
    initialRouteValue.kind === "capsule" ? initialRouteValue.capsuleId : null,
  );
  // Capsules opened as tabs in the right side-tools dock (#TASK-1470). AppShell
  // owns the list so the dock can show without a workspace; the panel renders
  // these and owns active-tab selection. `pendingActiveCapsuleId` is a one-shot
  // request to activate a capsule's tab (consumed by the panel).
  const [openCapsuleTabs, setOpenCapsuleTabs] = useState<SideCapsuleTab[]>([]);
  const [pendingActiveCapsuleId, setPendingActiveCapsuleId] = useState<
    string | null
  >(null);
  const [selectedWorkflowTask, setSelectedWorkflowTask] =
    useState<DesktopTaskSummary | null>(null);
  const [selectedWorkflowTaskId, setSelectedWorkflowTaskId] = useState<
    string | null
  >(() =>
    initialRouteValue.kind === "workflow-task" ? initialRouteValue.taskId : null,
  );
  const [selectedWorkflowRunId, setSelectedWorkflowRunId] = useState<
    string | null
  >(null);
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
  const [pendingModel, setPendingModel] = useState<string | null>(null);
  const [pendingModelReasoningEffort, setPendingModelReasoningEffort] =
    useState<string | null>(null);
  const [pendingModelServiceTier, setPendingModelServiceTier] =
    useState<string | null>(null);
  const [providerModelsByType, setProviderModelsByType] = useState<
    Record<string, DesktopProviderModels | null>
  >({});
  const [pendingWorkflowId, setPendingWorkflowId] = useState<string | null>(
    initialRouteValue.kind === "new-thread" && initialRouteValue.workflowId
      ? initialRouteValue.workflowId
      : null,
  );
  const [workflowThreadStarting, setWorkflowThreadStarting] = useState(false);
  const [messagesByThread, setMessagesByThread] = useState<MessageMap>({});
  // Server-derived render snapshot per thread (block 4). The presentation layer
  // maps `renderState.rows` straight to React; bodies are resolved from
  // `messagesByThread`. Replaced atomically per render frame.
  const [renderStateByThread, setRenderStateByThread] = useState<
    Record<string, RenderState>
  >({});
  const [threadInfoByThread, setThreadInfoByThread] = useState<
    Record<string, ThreadRuntimeInfo | null>
  >({});
  // Batch 3a: the mirror's dispatch-machine module owns machine-state
  // storage; React reads it through useSyncExternalStore (same bail-out
  // semantics as the previous useReducer — an identical reference neither
  // commits nor re-renders).
  const messageState = useSyncExternalStore(
    useCallback(
      (onChange) => gatewayMirror.subscribeMachine(onChange),
      [gatewayMirror],
    ),
    () => gatewayMirror.getMachineState(),
  );
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
  const [workspaceFileFilter, setWorkspaceFileFilter] = useState("");
  const [threadLogsOpen, setThreadLogsOpen] = useState(false);
  const [threadLogsText, setThreadLogsText] = useState("");
  const [threadLogsPath, setThreadLogsPath] = useState("");
  const [threadLogsCursor, setThreadLogsCursor] = useState(0);
  const [threadLogsLoading, setThreadLogsLoading] = useState(false);
  const [threadLogsError, setThreadLogsError] = useState<string | null>(null);
  const [threadLogsHasUnread, setThreadLogsHasUnread] = useState(false);
  const [botConversationGroupId, setBotConversationGroupId] = useState<string | null>(null);
  const [workspaceConversationPath, setWorkspaceConversationPath] =
    useState<string | null>(null);
  const [recentThreadsRailOpen, setRecentThreadsRailOpen] = useState(false);
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
  const [pinnedThreadsVersion, setPinnedThreadsVersion] = useState(0);
  const [addWorkspaceDialog, setAddWorkspaceDialog] = useState<{
    source: "new-thread" | "task";
    initialPath?: string;
    resolve?: (workspace: DesktopWorkspace | null) => void;
  } | null>(null);
  const [workspaceMenuOpenPath, setWorkspaceMenuOpenPath] = useState<string | null>(
    null,
  );
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  // Batch 3c-1: the mirror's live-stream domain owns transport-state
  // storage; React reads the aggregate map through useSyncExternalStore.
  // `liveStreamStateRef` (below) stays as the synchronous shadow for
  // event-path readers, fed by the transcript-controller proxies.
  const liveStreamStateByThread = useSyncExternalStore(
    useCallback(
      (onChange) => gatewayMirror.subscribeLiveStreams(onChange),
      [gatewayMirror],
    ),
    () => gatewayMirror.getLiveStreamMap(),
  );
  const [pendingRemoteInputsByThread, setPendingRemoteInputsByThread] =
    useState<PendingThreadInputMap>({});
  const [pendingAutomationRunsByThread, setPendingAutomationRunsByThread] =
    useState<Record<string, PendingAutomationRun>>({});
  const threadTitleInputRef = useRef<HTMLInputElement | null>(null);
  const threadLogsRef = useRef<HTMLDivElement | null>(null);
  const selectedThreadIdRef = useRef<string | null>(null);
  const selectedThreadGenerationRef = useRef(0);
  const selectThreadRequestSequenceRef = useRef(0);
  const newThreadDraftActiveRef = useRef(false);
  const pendingWorkspacePathRef = useRef<string | null>(null);
  const pendingWorkspaceModeRef = useRef<DesktopWorkspaceMode>("local");
  const pendingBotIdRef = useRef<string | null>(null);
  const newThreadInitialDispatchLockRef = useRef(false);
  const messageStateRef = useRef(initialMessageMachineState);
  const liveStreamStateRef = useRef<Record<string, LiveStreamState>>({});
  const deferredQueueDrainByThreadRef = useRef<Record<string, boolean>>({});
  const queueDrainInFlightByThreadRef = useRef<Record<string, boolean>>({});
  const pendingAutomationRunsRef = useRef<Record<string, PendingAutomationRun>>(
    {},
  );
  const threadLogsCursorRef = useRef(0);
  const toastSequenceRef = useRef(1);
  const toastTimeoutsRef = useRef<Record<number, number>>({});
  const botBindingRequestSequenceRef = useRef(0);
  const lastRemoteStateWarningKeyRef = useRef<string | null>(null);
  const pendingThreadBottomSnapRef = useRef<string | null>(null);

  const {
    closeMemoryDialog,
    memoryDialogDirty,
    memoryDialogDocument,
    memoryDialogDraft,
    memoryDialogError,
    memoryDialogLoading,
    memoryDialogSaving,
    memoryDialogStatus,
    memoryDialogTarget,
    openMemoryDialog,
    saveMemoryDialog,
    setMemoryDialogDraft,
  } = useMemoryDialogController();
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

  const {
    gatewayFailureCount,
    gatewaySetupCanCancel,
    gatewaySetupForced,
    gatewaySetupSavedConnectionRef,
    gatewayStatusHint,
    handleCancelGatewaySetup,
    hasGatewayRecoveryActivity,
    recordGatewayStatusObservation,
    refreshDesktopState,
    scheduleDesktopStateRefresh,
    setGatewaySetupCanCancel,
    setGatewaySetupForced,
  } = useGatewayConnectionController({
    connection,
    desktopState,
    error,
    gatewaySettingsStatus,
    gatewaySetupMessageForAuthError,
    liveStreamStateRef,
    loading,
    messageStateRef,
    mirror: gatewayMirror,
    pushToast,
    scheduleHistoryRefresh,
    selectedThreadId,
    selectedThreadIdRef,
    setConnection,
    setDesktopAgents,
    setDesktopState,
    setDesktopTeams,
    setDesktopWorkflows,
    setError,
    setGatewaySettingsStatus,
    setLocalSettingsStatus,
    setSettingsDraft,
    settingsDraft,
    t,
  });

  useEffect(() => {
    if (!automationStatus) {
      return undefined;
    }
    pushToast(automationStatus, "success");
    setAutomationStatus(null);
    return undefined;
  }, [automationStatus, pushToast]);

  function dispatchMessageState(action: MessageMachineAction) {
    // Batch 3a: one reducer application per action, committed in the
    // mirror (the previous shape ran the reducer twice — once for the ref
    // shadow, once inside React's useReducer). The ref stays as a
    // synchronous shadow for event-path readers until the machine's
    // orchestration migrates in batch 3c; the mirror is the only writer.
    messageStateRef.current = gatewayMirror.dispatchMachineAction(action);
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

  function handleSideToolsResizeStart(
    event: React.PointerEvent<HTMLDivElement>,
  ) {
    // Resize works whenever the dock is shown, including the no-workspace
    // capsule-only dock (#TASK-1470); it no longer gates on a workspace.
    if (!showConversationSideTools) {
      return;
    }
    sideToolsPanelWidthCustomizedRef.current = true;
    sideToolsResizeStateRef.current = {
      startX: event.clientX,
      startWidth: sideToolsPanelWidthRef.current,
    };
    setSideToolsResizing(true);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    event.preventDefault();
  }

  function handleSideToolsResizeKeyDown(
    event: React.KeyboardEvent<HTMLDivElement>,
  ) {
    if (!showConversationSideTools) {
      return;
    }
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) {
      return;
    }

    event.preventDefault();
    sideToolsPanelWidthCustomizedRef.current = true;
    const step = event.shiftKey ? 56 : 28;
    const layoutWidth = currentConversationWidth();
    const nextWidth =
      event.key === "Home"
        ? SIDE_TOOLS_PANEL_MIN_WIDTH
        : event.key === "End"
          ? clampSideToolsPanelWidth(
              SIDE_TOOLS_PANEL_MAX_WIDTH,
              layoutWidth,
            )
          : event.key === "ArrowLeft"
            ? clampSideToolsPanelWidth(
                sideToolsPanelWidthRef.current + step,
                layoutWidth,
              )
            : clampSideToolsPanelWidth(
                sideToolsPanelWidthRef.current - step,
                layoutWidth,
              );
    setSideToolsPanelWidth(nextWidth);
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
  const threadAvatarCatalog = useMemo(
    () => buildThreadAvatarCatalog(desktopAgents, desktopTeams),
    [desktopAgents, desktopTeams],
  );
  const activeAgentId = activeThread?.agentId || null;
  const activeThreadInfo = selectedThreadId
    ? threadInfoByThread[selectedThreadId] || null
    : null;
  const activeThreadInfoLoaded = selectedThreadId
    ? Object.prototype.hasOwnProperty.call(threadInfoByThread, selectedThreadId)
    : false;
  const activeThreadProviderType = selectedThreadId
    ? inferProviderTypeForThread(
        selectedThreadId,
        threadInfoByThread,
        desktopState,
        desktopAgents,
      )
    : null;
  const activeThreadProviderModels = activeThreadProviderType
    ? providerModelsByType[activeThreadProviderType] || null
    : null;
  const pendingAgent = desktopAgentMap.get(pendingAgentId) || null;
  const pendingTeam = desktopTeamMap.get(pendingAgentId) || null;
  const pendingAgentProviderType = pendingTeam
    ? null
    : pendingAgent?.providerType || null;
  const pendingProviderModels = pendingAgentProviderType
    ? providerModelsByType[pendingAgentProviderType] || null
    : null;

  useEffect(() => {
    // A model/thinking/tier override only makes sense for the agent it was picked for.
    setPendingModel(null);
    setPendingModelReasoningEffort(null);
    setPendingModelServiceTier(null);
  }, [pendingAgentId]);

  useEffect(() => {
    if (!pendingAgentProviderType) {
      return;
    }
    if (pendingAgentProviderType in providerModelsByType) {
      return;
    }
    let cancelled = false;
    void window.garyxDesktop.listProviderModels(pendingAgentProviderType).then(
      (models) => {
        if (!cancelled) {
          setProviderModelsByType((current) => ({
            ...current,
            [pendingAgentProviderType]: models,
          }));
        }
      },
      () => {
        if (!cancelled) {
          setProviderModelsByType((current) => ({
            ...current,
            [pendingAgentProviderType]: null,
          }));
        }
      },
    );
    return () => {
      cancelled = true;
    };
  }, [pendingAgentProviderType, providerModelsByType]);
  useEffect(() => {
    if (!activeThreadProviderType) {
      return;
    }
    if (activeThreadProviderType in providerModelsByType) {
      return;
    }
    let cancelled = false;
    void window.garyxDesktop.listProviderModels(activeThreadProviderType).then(
      (models) => {
        if (!cancelled) {
          setProviderModelsByType((current) => ({
            ...current,
            [activeThreadProviderType]: models,
          }));
        }
      },
      () => {
        if (!cancelled) {
          setProviderModelsByType((current) => ({
            ...current,
            [activeThreadProviderType]: null,
          }));
        }
      },
    );
    return () => {
      cancelled = true;
    };
  }, [activeThreadProviderType, providerModelsByType]);
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
  const threadLogLines = useMemo(
    () => buildThreadLogLines(threadLogsText),
    [threadLogsText],
  );
  const activeThreadLogsPath = threadLogsPath || "Waiting for log file";
  const activeThreadLogsHasUnread = threadLogsHasUnread;
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
  const {
    conversationRef,
    currentConversationWidth,
    currentThreadLayoutWidth,
    handleRailResizeStart,
    handleSidebarResizeStart,
    handleThreadLogsResizeKeyDown,
    handleThreadLogsResizeStart,
    railResizing,
    setSideToolsPanelWidth,
    setSideToolsResizing,
    sidebarCollapsed,
    sidebarResizing,
    sidebarWidth,
    sideToolsPanelWidth,
    sideToolsPanelWidthCustomizedRef,
    sideToolsPanelWidthRef,
    sideToolsResizeStateRef,
    sideToolsResizing,
    threadLayoutRef,
    threadLogsPanelWidth,
    threadLogsResizing,
    toggleSidebarCollapsed,
  } = useLayoutResizeController({
    contentView,
    desktopState,
    inspectorOpen,
    openCapsuleTabs,
    setDesktopState,
    setSettingsDraft,
    threadLogsOpen,
  });
  const {
    cancelMessagesForceScrollBudget,
    lastRenderedMessageThreadRef,
    messagesRef,
    pendingMessagesPrependAnchorRef,
    requestMessagesBottomSnap,
    requestSelectedThreadMessagesBottomSnap,
    shouldStickMessagesToBottomRef,
  } = useMessagesScrollController({
    activeMessages,
    activeThreadMessageKey,
    historyLoading,
    messageTailSignature,
    pendingThreadBottomSnapRef,
    scrollMessagesToLatest,
    selectedThreadIdRef,
  });
  useEffect(() => {
    selectedThreadIdRef.current = selectedThreadId;
    selectedThreadGenerationRef.current += 1;
  }, [selectedThreadId]);
  const {
    applyCanonicalTranscript,
    applyRemoteTranscript,
    clearLiveStreamState,
    forceReleaseThreadRuntime,
    getLiveStreamState,
    hasPendingHistoryIntents,
    intentForId,
    loadOlderThreadHistoryPage,
    messagesByThreadRef,
    replaceLiveStreamThreadId,
    setThreadRuntimeState,
    startCommittedThreadStream,
    threadTitleOverridesRef,
    updateLiveStreamState,
    updateMessagesByThread,
  } = useTranscriptController({
    activeHistoryPagination,
    activeMessages,
    activeThreadMessageKey,
    connection,
    desktopState,
    dispatchMessageState,
    editingThreadTitle,
    historyLoading,
    lastRenderedMessageThreadRef,
    liveStreamStateRef,
    messageStateRef,
    messagesRef,
    mirror: gatewayMirror,
    pendingMessagesPrependAnchorRef,
    recordGatewayStatusObservation,
    refetchAuthoritativeTranscriptAfterRewrite,
    requestSelectedThreadMessagesBottomSnap,
    scheduleDesktopStateRefresh,
    scheduleHistoryRefresh,
    selectedThreadId,
    selectedThreadIdRef,
    setDesktopState,
    setError,
    setHistoryLoading,
    setHistoryPaginationByThread,
    setMessagesByThread,
    setPendingAutomationRun,
    setPendingRemoteInputsByThread,
    setRenderStateByThread,
    setThreadInfoByThread,
    setTitleDraft,
    settingsDraft,
  });
  // Batch 2b dev-only parity probe (removed with the dual-write scaffolding
  // in batch 6): `__garyxMirrorParity(threadId)` in the DevTools console
  // compares the mirror's thread snapshot against the legacy React state.
  // Since batch 3b bridges local optimistic/recovery writes into the
  // mirror, messages compare as FULL sequences; `loadingBefore` is a
  // legacy-transient flag (the mirror does not run the legacy older-page
  // fetch) and is excluded from the pagination comparison.
  const mirrorParityStateRef = useRef({
    messagesByThread,
    renderStateByThread,
    historyPaginationByThread,
    threadInfoByThread,
    pendingRemoteInputsByThread,
  });
  mirrorParityStateRef.current = {
    messagesByThread,
    renderStateByThread,
    historyPaginationByThread,
    threadInfoByThread,
    pendingRemoteInputsByThread,
  };
  useEffect(() => {
    if (!import.meta.env.DEV) {
      return undefined;
    }
    const probeWindow = window as typeof window & {
      __garyxGatewayMirror?: GatewayMirror;
      __garyxMirrorParity?: (threadId: string) => unknown;
    };
    probeWindow.__garyxGatewayMirror = gatewayMirror;
    probeWindow.__garyxMirrorParity = (threadId: string) => {
      const legacy = mirrorParityStateRef.current;
      const snapshot = gatewayMirror.getThreadSnapshot(threadId);
      const json = (value: unknown) => JSON.stringify(value ?? null);
      const legacyMessages: readonly UiTranscriptMessage[] =
        legacy.messagesByThread[threadId] || [];
      const mirrorMessages = snapshot.messages;
      const stripLoading = (
        state: ThreadHistoryPaginationState | null | undefined,
      ) => (state ? { ...state, loadingBefore: false } : null);
      const equal = {
        messages: json(legacyMessages) === json(mirrorMessages),
        renderState:
          json(legacy.renderStateByThread[threadId] ?? null) ===
          json(snapshot.renderState),
        pagination:
          json(stripLoading(legacy.historyPaginationByThread[threadId])) ===
          json(stripLoading(snapshot.historyPagination)),
        threadInfo:
          json(legacy.threadInfoByThread[threadId] ?? null) ===
          json(snapshot.threadInfo),
        pendingInputs:
          json(legacy.pendingRemoteInputsByThread[threadId] ?? []) ===
          json(snapshot.pendingRemoteInputs),
      };
      return {
        threadId,
        parity: Object.values(equal).every(Boolean),
        equal,
        counts: {
          legacyMessages: legacyMessages.length,
          mirrorMessages: mirrorMessages.length,
          localRows: legacyMessages.filter(
            (entry) => entry.localState !== "remote_final",
          ).length,
        },
      };
    };
    return () => {
      delete probeWindow.__garyxGatewayMirror;
      delete probeWindow.__garyxMirrorParity;
    };
  }, [gatewayMirror]);
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
  const activeRenderState = activeThreadMessageKey
    ? renderStateByThread[activeThreadMessageKey] || null
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
    .filter((intentId, index, intentIds) => {
      return intentIds.indexOf(intentId) === index;
    })
    .map((intentId) => messageState.intentsById[intentId])
    .filter((intent): intent is MessageIntent => {
      return Boolean(intent) && intent.state === "awaiting_provider_ack";
    });
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
  const activePendingInputOriginRefs = useMemo(
    () =>
      pendingInputOriginRefsForThread(
        messageState.intentsById,
        activeThreadMessageKey,
      ),
    [messageState.intentsById, activeThreadMessageKey],
  );
  const visibleRemotePendingInputs = visibleRemotePendingInputsForThread({
    activeMessages,
    visiblePendingAckIntentCount: visiblePendingAckIntents.length,
    remotePendingInputs: activeRemotePendingInputs,
    pendingInputOriginRefs: activePendingInputOriginRefs,
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
  const activeRuntimeBusy = Boolean(
    activeRuntime && isRuntimeBusy(activeRuntime.state),
  );
  const threadActivity = deriveThreadActivityModel({
    messages: activeMessages,
    runtimeBusy: activeRuntimeBusy,
    pendingAckIntentCount: visiblePendingAckIntents.length,
    remoteAwaitingAckInputCount: visibleRemoteAwaitingAckInputs.length,
    pendingHistoryIntent: activePendingHistoryIntent,
    renderTailActivity: activeRenderState?.tailActivity ?? null,
    renderActiveToolGroupId: activeRenderState?.activeToolGroupId ?? null,
  });
  const showPendingAckLoading = threadActivity.showPendingAckLoading;
  const canSteerQueuedPrompt = threadActivity.canSteerQueuedPrompt;
  // Rendered tail indicators come from the server snapshot (charter §6): the
  // thinking bubble is `tailActivity==="thinking"` (or the optimistic pre-ack
  // window); the tool shimmer keys off `activeToolGroupId`. assistant_streaming
  // / tool_active are carried by the rows themselves, not a separate bubble.
  const activeToolGroupId = activeRenderState?.activeToolGroupId ?? null;
  const activeRateLimit = activeRenderState?.rateLimit ?? null;
  const showTailThinking = Boolean(
    activeRenderState?.tailActivity === "thinking" || showPendingAckLoading,
  );
  const isActiveStreamingThread = Boolean(
    activeLiveStream &&
    ["connecting", "streaming", "reconciling"].includes(
      activeLiveStream.streamStatus,
    ),
  );
  const isDraftSendingThread = Boolean(
    !selectedThreadId &&
      (activeRuntimeBusy || isActiveStreamingThread),
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
  const handleWorkspacePreviewRequested = useCallback(() => {
    setInspectorOpen(true);
  }, []);
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
    closeWorkspacePreview,
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
    onWorkspacePreviewRequested: handleWorkspacePreviewRequested,
    pushToast,
    setError,
    workspaces: desktopState?.workspaces || [],
  });
  const { isActiveSendingThread } = deriveThreadComposerControlModel({
    hasThread: Boolean(selectedThreadId),
    runtimeBusy: activeRuntimeBusy,
    showPendingAckLoading,
    renderTailActivity: activeRenderState?.tailActivity ?? null,
    renderActiveToolGroupId: activeToolGroupId,
  });
  const composerEditingLocked = isDraftSendingThread || workflowThreadStarting;
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

  const activeThreadHasMessages = Boolean(
    (activeThread?.messageCount ?? 0) > 0 || activeMessages.length > 0,
  );
  const isSettingsView = contentView === "settings";
  const isBrowserView = contentView === "browser";
  const isBotsView = contentView === "bots";
  const isAutomationView = contentView === "automation";
  const isCapsulesView = contentView === "capsules";
  const showDreamsFeature = Boolean(gatewaySettingsDraft?.dreams?.enabled);
  const isAgentsView = contentView === "agents";
  const isTeamsView = contentView === "teams";
  const isSkillsView = contentView === "skills";
  const isTasksView = contentView === "tasks";
  const isWorkflowView = contentView === "workflow";
  const activeWorkflowRunThreadId =
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
  const recentThreadRows = useMemo(
    () =>
      // `desktopState.threads` is the full recency-sorted set the app loads
      // (gateway caps it at 1000). Show all of it; the rail list scrolls.
      (desktopState?.threads || []).map((thread) => ({
        thread,
        isActive:
          visibleThreadEntrySelectionSource === "recent" &&
          visibleSelectedThreadId === thread.id,
        isBusy: threadRunStateIsRunning(thread),
      })),
    [
      desktopState?.threads,
      visibleSelectedThreadId,
      visibleThreadEntrySelectionSource,
    ],
  );
  const pinnedThreadRows = useMemo(
    () =>
      pinnedThreadIds
        .map((threadId) => threadSummaryById.get(threadId) || null)
        .filter((thread): thread is DesktopThreadSummary => Boolean(thread))
        .map((thread) => ({
          thread,
          avatar: resolveThreadAvatarIdentity(thread, threadAvatarCatalog),
          isActive:
            visibleThreadEntrySelectionSource === "pinned" &&
            visibleSelectedThreadId === thread.id,
          isBusy: threadRunStateIsRunning(thread),
        })),
    [
      pinnedThreadIds,
      threadAvatarCatalog,
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
      setPinnedThreadsVersion((version) => version + 1);
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
    setRecentThreadsRailOpen((current) => (current ? false : current));
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
    sidebarCollapsed ? "sidebar-collapsed" : null,
    activeBotConversationGroup ||
    activeWorkspaceThreadGroup ||
    (shouldShowConversationRail && recentThreadsRailOpen)
      ? "with-bot-conversation-rail"
      : null,
  ]
    .filter(Boolean)
    .join(" ");
  const showStaticWindowToolbar =
    isSettingsView ||
    isAutomationView ||
    isCapsulesView ||
    isAgentsView ||
    isTeamsView ||
    isSkillsView;
  const canEditThreadTitle = Boolean(
    activeThread &&
    !activeAutomationThread &&
    !isAutomationView &&
    !isCapsulesView &&
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
  const showHistoryLoadingPlaceholder = Boolean(
    historyLoading &&
    !activeMessages.length &&
    !showAutomationRunInitialPlaceholder,
  );
  const {
    appendSideComposerAttachments,
    ensureSideChatThread,
    handleSideComposerSubmit,
    openTaskThreadInSidePanel,
    removeSideComposerBrowserAnnotation,
    removeSideComposerFile,
    removeSideComposerImage,
    sideChatActiveToolGroupId,
    sideChatAgentLabel,
    sideChatCanSteerQueuedPrompt,
    sideChatComposerEditingLocked,
    sideChatComposerHasPayload,
    sideChatComposerLocked,
    sideChatComposerPlaceholder,
    sideChatComposerProviderType,
    sideChatComposerWorkspaceBranch,
    sideChatComposerWorkspaceMode,
    sideChatCreating,
    sideChatError,
    sideChatHistoryLoading,
    sideChatHistoryPagination,
    sideChatIsSendingThread,
    sideChatLiveStream,
    sideChatMessages,
    sideChatMessagesRef,
    sideChatQueue,
    sideChatRenderState,
    sideChatShowTailThinking,
    sideChatSourceThreadId,
    sideChatStreamConsumerId,
    sideChatThreadBot,
    sideChatThreadBotId,
    sideChatThreadId,
    sideChatThreadIdRef,
    sideChatThreadIdsRef,
    sideChatThreadLayoutRef,
    sideChatThreadSummary,
    sideChatVisiblePendingAckIntents,
    sideChatVisibleRemotePendingInputs,
    sideComposerAttachmentInputRef,
    sideComposerDraft,
    sideComposerTextareaRef,
    sideIgnoreComposerSubmitUntilRef,
    sideIsComposingRef,
    updateSideComposerDraft,
  } = useSideChatController({
    activeThread,
    applyRemoteTranscript,
    botGroups,
    boundBotsForThread,
    browserAnnotationScreenshotImages,
    composePromptWithBrowserAnnotations,
    composerProviderType,
    deferredQueueDrainByThreadRef,
    desktopAgentMap,
    desktopAgents,
    desktopState,
    dispatchMessageState,
    ensureThreadOpenable,
    getLiveStreamState,
    historyPaginationByThread,
    inferProviderTypeForThread,
    liveStreamStateByThread,
    messageState,
    messageStateRef,
    messageTailSignature,
    messagesByThread,
    messagesByThreadRef,
    pendingAgentId,
    pendingInputOriginRefsForThread,
    pendingRemoteInputsByThread,
    prepareAttachmentUploads,
    queueDrainInFlightByThreadRef,
    renderStateByThread,
    runQueuedBatch,
    scrollMessagesToLatest,
    setDesktopState,
    setError,
    setPendingAutomationRun,
    settingsDraft,
    startCommittedThreadStream,
    steerQueuedIntent,
    t,
    threadInfoByThread,
    threadSummaryById,
    transcriptHasAutomationResponse,
    transcriptMessageMatchesIntent,
    updateMessagesByThread,
  });
  // Batch 3c-2: the dispatch orchestration (send/steer/interrupt/queue
  // drain) lives in the mirror; its deps are refreshed on every commit
  // (the streamEventHandlerRef pattern) so orchestration entry points
  // destructure this render's values — the legacy closure capture.
  const dispatchOrchestratorDeps: DispatchOrchestratorDeps = {
    applyCanonicalTranscript,
    canSteerQueuedPrompt,
    checkConnection: () => window.garyxDesktop.checkConnection(),
    clearLiveStreamState,
    connection,
    desktopAgents,
    desktopState,
    dispatchMessageState,
    getLiveStreamState,
    getThreadHistory: (threadId) =>
      window.garyxDesktop.getThreadHistory(threadId),
    hasPendingHistoryIntents,
    inferProviderTypeForThread,
    intentForId,
    interruptThread: (threadId) =>
      window.garyxDesktop.interruptThread(threadId),
    messageStateRef,
    messagesByThreadRef,
    openChatStream: (input) => window.garyxDesktop.openChatStream(input),
    recordGatewayStatusObservation,
    requestMessagesBottomSnap,
    scheduleHistoryRefresh,
    sendStreamingInput: (input) =>
      window.garyxDesktop.sendStreamingInput(input),
    setConnection,
    setDesktopState,
    setError,
    setThreadRuntimeState,
    settingsDraft,
    sideChatThreadIdsRef,
    threadInfoByThread,
    threadTitleOverridesRef,
    updateLiveStreamState,
    updateMessagesByThread,
  };
  useEffect(() => {
    gatewayMirror.setDispatchDeps(dispatchOrchestratorDeps);
  });
  function appendSeededTurn(
    threadId: string,
    intent: MessageIntent,
    options?: { seedUserBubble?: boolean },
  ): SeededTurn {
    return gatewayMirror.appendSeededTurn(threadId, intent, options);
  }
  function sendIntentOnce(
    threadId: string,
    intentId: string,
    options?: { seedUserBubble?: boolean; seededTurn?: SeededTurn },
  ): Promise<boolean> {
    return gatewayMirror.sendIntentOnce(threadId, intentId, options);
  }
  function interruptThread(threadId: string | null | undefined): Promise<void> {
    return gatewayMirror.interruptThread(threadId);
  }
  const {
    appendComposerAttachments,
    clearComposerDraft,
    composer,
    composerAttachmentInputRef,
    composerBrowserAnnotations,
    composerDraftRef,
    composerFiles,
    composerHasPayload,
    composerHasPayloadRef,
    composerImages,
    composerLocked,
    composerResetKey,
    composerTextareaRef,
    draggedQueueIntentId,
    handleAddBrowserAnnotationComment,
    handleComposerSubmit,
    handleInterrupt,
    handleRetryFailedMessage,
    handleSteerQueuedPrompt,
    ignoreComposerSubmitUntilRef,
    isComposingRef,
    markIgnoreComposerSubmitWindow,
    queueDropTarget,
    removeComposerBrowserAnnotation,
    removeComposerFile,
    removeComposerImage,
    reorderQueuedIntent,
    requestComposerFocus,
    setComposerTextPresent,
    setDraggedQueueIntentId,
    setQueueDropTarget,
    syncComposerPhase,
  } = useMessageDispatchController({
    activeQueue,
    activeThreadId,
    appendSeededTurn,
    canSteerQueuedPrompt,
    clearLiveStreamState,
    contentView,
    deferredQueueDrainByThreadRef,
    dispatchMessageState,
    ensureSelectedThreadId,
    ensureThreadBotRouting,
    handleStartWorkflowThreadFromComposer,
    intentForId,
    interruptThread,
    isActiveSendingThread,
    isDraftSendingThread,
    messageStateRef,
    newThreadInitialDispatchLockRef,
    pendingWorkflowId,
    pendingWorkspacePath,
    preferredWorkspaceForNewThread,
    queueDrainInFlightByThreadRef,
    replaceLiveStreamThreadId,
    requestMessagesBottomSnap,
    runQueuedBatch,
    selectedThreadId,
    sendIntentOnce,
    setError,
    setThreadRuntimeState,
    settingsDraft,
    steerQueuedIntent,
    t,
    updateLiveStreamState,
    updateMessagesByThread,
    workflowThreadStarting,
  });
  const providerSelectorLocked = Boolean(
    composerLocked ||
    isActiveSendingThread ||
    activeThreadHasMessages ||
    (historyLoading && Boolean(activeThread?.messageCount)),
  );
  const conversationContextText = isAutomationView
    ? `${desktopState?.automations.length || 0} scheduled runs`
    : isCapsulesView
      ? "Self-contained HTML capsules"
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
    void refreshAgentTargets();
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
    const nextState = await window.garyxDesktop.addChannelAccount(input);
    startTransition(() => {
      setDesktopState(nextState);
    });
    await loadGatewaySettings({ clearStatus: true });
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

  async function openExistingThread(
    threadId: string,
    entrySource: ThreadEntrySelectionSource | null = null,
  ): Promise<boolean> {
    setError(null);
    setContentView("thread");
    setNewThreadDraftActive(false);

    return selectExistingThreadInPlace(threadId, entrySource);
  }

  async function selectExistingThreadInPlace(
    threadId: string,
    entrySource: ThreadEntrySelectionSource | null = null,
  ): Promise<boolean> {
    const requestSequence = ++selectThreadRequestSequenceRef.current;
    setError(null);
    setNewThreadDraftActive(false);

    try {
      if (!(await ensureThreadOpenable(threadId))) {
        if (requestSequence !== selectThreadRequestSequenceRef.current) {
          return true;
        }
        setError(`Thread not found: ${threadId}`);
        return false;
      }
    } catch (error) {
      if (requestSequence !== selectThreadRequestSequenceRef.current) {
        return true;
      }
      setError(
        error instanceof Error
          ? error.message
          : `Failed to open thread: ${threadId}`,
      );
      return false;
    }

    if (requestSequence !== selectThreadRequestSequenceRef.current) {
      return true;
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
    setSelectedWorkflowRunId(task.threadId || null);
    setContentView("workflow");
  }

  useDeepLinkRouteController({
    capsulePreviewId,
    clearComposerDraft,
    contentView,
    desktopState,
    ensureThreadOpenable,
    handleResumeProviderSession,
    handleSelectAutomation,
    handleSelectSettingsTab,
    loading,
    newThreadDraftActive,
    openExistingThread,
    pendingAgentId,
    pendingWorkflowId,
    pendingWorkspacePath,
    pushToast,
    requestComposerFocus,
    selectedAutomationId,
    selectedThreadId,
    selectedWorkflowRunId,
    selectedWorkflowTaskId,
    setCapsulePreviewId,
    setConnection,
    setContentView,
    setError,
    setNewThreadDraftActive,
    setPendingAgentId,
    setPendingBotId,
    setPendingWorkflowId,
    setPendingWorkspaceMode,
    setPendingWorkspacePath,
    setSelectedThreadId,
    setSelectedWorkflowRunId,
    setSelectedWorkflowTask,
    setSelectedWorkflowTaskId,
    settingsActiveTab,
  });

  useEffect(() => {
    messageStateRef.current = messageState;
  }, [messageState]);

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
    // Capsule tabs are scoped to the current thread; drop them when the thread
    // or content view changes so a different thread's capsules never linger in
    // the dock (#TASK-1470).
    setOpenCapsuleTabs([]);
    setPendingActiveCapsuleId(null);
  }, [contentView, selectedThreadId]);

  useEffect(() => {
    if (!canEditThreadTitle && editingThreadTitle) {
      setEditingThreadTitle(false);
    }
  }, [canEditThreadTitle, editingThreadTitle]);

  useEffect(() => {
    if (!editingThreadTitle) {
      setTitleDraft(activeThread?.title || DEFAULT_SESSION_TITLE);
    }
  }, [editingThreadTitle, activeThread?.title]);

  useEffect(() => {
    threadLogsCursorRef.current = threadLogsCursor;
  }, [threadLogsCursor]);

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
      setWorkspaceFileFilter("");
      return;
    }

    setWorkspaceFileFilter("");
    setExpandedWorkspaceDirectories((current) => ({
      ...current,
      [workspaceDirectoryKey(activeWorkspacePath, "")]: true,
    }));
  }, [activeWorkspacePath]);

  useEffect(() => {
    if (!workspacePreviewModalOpen || contentView !== "thread") {
      return;
    }
    setThreadLogsOpen(false);
    setInspectorOpen(true);
  }, [contentView, workspacePreviewModalOpen]);

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
    setThreadLogsHasUnread(false);
    window.requestAnimationFrame(() => {
      scrollThreadLogsToLatest("auto");
    });
  }, [selectedThreadId, threadLogsOpen]);

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
      pendingModel,
      pendingModelReasoningEffort,
      pendingModelServiceTier,
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
      setPendingModel,
      setPendingModelReasoningEffort,
      setPendingModelServiceTier,
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

  async function syncThreadBotBinding(
    threadId: string,
    botId: string | null,
  ): Promise<void> {
    const requestSequence = botBindingRequestSequenceRef.current + 1;
    botBindingRequestSequenceRef.current = requestSequence;
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

  function isArchiveAlreadyApplied(error: unknown): boolean {
    if (!(error instanceof Error)) {
      return false;
    }
    return error.message.toLowerCase().includes("thread not found");
  }

  async function archiveThreadOptimistically(input?: {
    threadId?: string | null;
    endpointKey?: string | null;
  }) {
    const targetThreadId = input?.threadId || activeThread?.id || null;
    if (!targetThreadId || !desktopState) {
      return;
    }
    const targetRuntime = targetThreadId
      ? selectThreadRuntime(messageStateRef.current, targetThreadId)
      : null;
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
    if (input?.endpointKey) {
      endpointKeys.add(input.endpointKey);
    }

    const deletingSelected = targetThreadId === selectedThreadId;
    const optimisticState = desktopStateWithoutThread(
      desktopState,
      targetThreadId,
    );
    const fallbackThread = deletingSelected
      ? optimisticState.threads[0] || optimisticState.sessions[0] || null
      : null;

    setDeletingThreadId(targetThreadId);
    setError(null);
    setDesktopState((current) =>
      current ? desktopStateWithoutThread(current, targetThreadId) : current,
    );
    if (deletingSelected) {
      setSelectedThreadId(fallbackThread?.id || null);
      setThreadEntrySelectionSource(null);
    }
    dispatchMessageState({
      type: "thread/delete",
      threadId: targetThreadId,
    });

    try {
      const api = getDesktopApi();
      const archivedState = await api.archiveThread({
        threadId: targetThreadId,
        endpointKeys: Array.from(endpointKeys).sort(),
      });
      setDesktopState(desktopStateWithoutThread(archivedState, targetThreadId));
    } catch (archiveError) {
      if (isArchiveAlreadyApplied(archiveError)) {
        return;
      }
      setError(
        archiveError instanceof Error
          ? archiveError.message
          : "Failed to delete the thread",
      );
      void refreshDesktopState().catch(() => null);
    } finally {
      setDeletingThreadId((current) =>
        current === targetThreadId ? null : current,
      );
    }
  }

  async function handleDeleteThread(threadId?: string) {
    await archiveThreadOptimistically({ threadId: threadId || null });
  }

  async function handleArchiveBotConversationEndpoint(endpoint: DesktopChannelEndpoint) {
    await archiveThreadOptimistically({
      threadId: endpoint.threadId || null,
      endpointKey: endpoint.endpointKey || null,
    });
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

  async function handleUpdateActiveThreadRuntimeSettings(input: {
    model?: string | null;
    modelReasoningEffort?: string | null;
    modelServiceTier?: string | null;
  }) {
    const threadId = selectedThreadId;
    if (!threadId) {
      return;
    }
    setError(null);
    try {
      const transcript = await window.garyxDesktop.updateThreadRuntimeSettings({
        threadId,
        ...input,
      });
      applyRemoteTranscript(threadId, transcript);
    } catch (runtimeSettingsError) {
      setError(
        runtimeSettingsError instanceof Error
          ? runtimeSettingsError.message
          : "Failed to update thread model settings",
      );
    }
  }

  async function refetchAuthoritativeTranscriptAfterRewrite(threadId: string) {
    const startSelectionGeneration = selectedThreadGenerationRef.current;
    try {
      await window.garyxDesktop.clearThreadTranscriptCache(threadId);
      const transcript = await window.garyxDesktop.getThreadHistory(threadId);
      if (selectedThreadIdRef.current === threadId) {
        requestSelectedThreadMessagesBottomSnap(threadId, true);
      }
      applyRemoteTranscript(threadId, transcript);
      const shouldRestartSelectedStream =
        shouldRestartSelectedThreadStreamAfterRefetch({
          threadId,
          selectedThreadId: selectedThreadIdRef.current,
          startSelectionGeneration,
          currentSelectionGeneration: selectedThreadGenerationRef.current,
        });
      if (shouldRestartSelectedStream) {
        await startCommittedThreadStream(
          threadId,
          transcript,
          SELECTED_THREAD_STREAM_CONSUMER_ID,
        );
      }
      if (sideChatThreadIdRef.current === threadId) {
        await startCommittedThreadStream(
          threadId,
          transcript,
          sideChatStreamConsumerId(threadId),
        );
      }
    } catch {
      scheduleHistoryRefresh(threadId, 3, 500, true);
    }
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
      onCanonicalTranscript: (threadId, transcript) => {
        requestSelectedThreadMessagesBottomSnap(threadId, true);
        applyCanonicalTranscript(threadId, transcript);
      },
      onRemoteTranscript: (threadId, transcript) => {
        requestSelectedThreadMessagesBottomSnap(threadId, true);
        applyRemoteTranscript(threadId, transcript);
      },
      onExhausted: forceReleaseThreadRuntime,
    });
  }

  // Batch 3c-2: the queued-batch drain and steer orchestration live in the
  // mirror's dispatch orchestrator (gateway-mirror/dispatch-orchestrator.ts,
  // verbatim moves of the former T13 TDZ stay-behinds). These delegates keep
  // the controller arg wiring unchanged.
  function runQueuedBatch(
    threadId: string,
    initialIntentId?: string,
  ): Promise<void> {
    return gatewayMirror.runQueuedBatch(threadId, initialIntentId);
  }

  function steerQueuedIntent(
    latestIntent: MessageIntent,
    options?: { canSteer?: boolean },
  ): Promise<void> {
    return gatewayMirror.steerQueuedIntent(latestIntent, options);
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
      setDesktopState((current) => {
        const baseState = current || started.state;
        return {
          ...baseState,
          threads: mergeThread(baseState.threads, started.thread),
          sessions: mergeThread(baseState.sessions, started.thread),
        };
      });
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

  const workspaceFileFilterQuery = workspaceFileFilter.trim().toLowerCase();

  function workspaceEntryMatchesFilter(
    workspacePath: string,
    entry: DesktopWorkspaceFileEntry,
  ): boolean {
    if (!workspaceFileFilterQuery) {
      return true;
    }
    const haystack = `${entry.name}\n${entry.path}`.toLowerCase();
    if (haystack.includes(workspaceFileFilterQuery)) {
      return true;
    }
    if (entry.entryType !== "directory") {
      return false;
    }
    const childKey = workspaceDirectoryKey(workspacePath, entry.path);
    const childEntries = workspaceDirectories[childKey]?.entries || [];
    return childEntries.some((child) =>
      workspaceEntryMatchesFilter(workspacePath, child),
    );
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
        if (!workspaceEntryMatchesFilter(workspacePath, entry)) {
          return null;
        }
        const childKey = workspaceDirectoryKey(workspacePath, entry.path);
        const isExpanded = expandedWorkspaceDirectories[childKey] === true;
        const shouldShowChildren =
          entry.entryType === "directory" &&
          (isExpanded || Boolean(workspaceFileFilterQuery));
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
            {shouldShowChildren ? (
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
  const selectedSideToolWorkspaceFile: SideToolWorkspaceFile | null =
    selectedWorkspaceFile &&
    selectedWorkspaceFileEntry?.entryType === "file" &&
    selectedWorkspaceFile.workspacePath === activeWorkspacePath
      ? {
          name: selectedWorkspaceFileEntry.name,
          relativePath: selectedWorkspaceFile.path,
          absolutePath: workspaceFileAbsolutePath(
            selectedWorkspaceFile.workspacePath,
            selectedWorkspaceFile.path,
          ),
          mediaType:
            selectedWorkspaceFileEntry.mediaType ||
            (workspaceFilePreview?.workspacePath === selectedWorkspaceFile.workspacePath &&
            workspaceFilePreview.path === selectedWorkspaceFile.path
              ? workspaceFilePreview.mediaType
              : null),
        }
      : null;

  async function handleRevealSelectedWorkspaceFile() {
    if (!selectedWorkspaceFile) {
      return;
    }
    await window.garyxDesktop.revealWorkspaceFile({
      workspacePath: selectedWorkspaceFile.workspacePath,
      filePath: selectedWorkspaceFile.path,
    });
  }

  const sideChatPanel = !sideChatSourceThreadId ? (
    <div className="side-tool-empty">
      {t("Open a thread before starting side chat.")}
    </div>
  ) : sideChatThreadId ? (
    <ThreadPage
      surfaceVariant="side-chat"
      agentLabel={sideChatAgentLabel}
      composerAgentOptions={composerAgentOptions}
      composerWorkflowOptions={composerWorkflowOptions}
      composerWorkflowOptionsLoading={workflowDefinitionsLoading}
      activeMessages={sideChatMessages}
      activePendingAckIntents={sideChatVisiblePendingAckIntents}
      activePendingAutomationRun={null}
      activeToolGroupId={sideChatActiveToolGroupId}
      activeQueue={sideChatQueue}
      renderState={sideChatRenderState}
      activeThreadLogsHasUnread={false}
      activeThreadLogsPath=""
      activeThreadSummary={sideChatThreadSummary}
      activeThreadTitle={sideChatThreadSummary?.title || null}
      activeThreadRunId={
        sideChatLiveStream?.runId || sideChatThreadSummary?.recentRunId || null
      }
      availableWorkspaceCount={availableWorkspaceCount}
      composer={sideComposerDraft.text}
      composerAttachmentInputRef={sideComposerAttachmentInputRef}
      composerBrowserAnnotations={sideComposerDraft.browserAnnotations}
      composerFiles={sideComposerDraft.files}
      composerHasPayload={sideChatComposerHasPayload}
      composerImages={sideComposerDraft.images}
      composerEditingLocked={sideChatComposerEditingLocked}
      composerLocked={sideChatComposerLocked}
      composerPlaceholder={sideChatComposerPlaceholder}
      composerProviderType={sideChatComposerProviderType}
      composerResetKey={sideComposerDraft.resetKey}
      composerWorkspaceBranch={sideChatComposerWorkspaceBranch}
      composerWorkspaceMode={sideChatComposerWorkspaceMode}
      activeThreadBot={sideChatThreadBot}
      activeThreadBotId={sideChatThreadBotId}
      botBindingDisabled={bindingMutation === "bot-binding"}
      botGroups={botGroups}
      slashCommands={commands}
      slashCommandsLoaded={commandsLoaded}
      slashCommandsLoading={commandsLoading}
      composerTextareaRef={sideComposerTextareaRef}
      draggedQueueIntentId={draggedQueueIntentId}
      historyLoading={sideChatHistoryLoading}
      historyLoadingEarlier={Boolean(sideChatHistoryPagination?.loadingBefore)}
      ignoreComposerSubmitUntilRef={sideIgnoreComposerSubmitUntilRef}
      inspectorOpen={false}
      isActiveSendingThread={sideChatIsSendingThread}
      canSteerQueuedPrompt={sideChatCanSteerQueuedPrompt}
      isComposingRef={sideIsComposingRef}
      messagesRef={sideChatMessagesRef}
      threadLogLines={[]}
      newThreadSelectedAgentId={sideChatThreadSummary?.agentId || pendingAgentId}
      newThreadSelectedWorkflowId={null}
      newThreadWorkspaceEntry={newThreadWorkspaceEntry}
      newThreadWorkspaceMode={pendingWorkspaceMode}
      onAddWorkspace={() => {
        void handleAddWorkspace();
      }}
      onAppendComposerAttachments={(files) => {
        void appendSideComposerAttachments(sideChatSourceThreadId, files);
      }}
      onCancelIntent={(threadId, intentId) => {
        dispatchMessageState({
          type: "intent/cancelled",
          threadId,
          intentId,
        });
      }}
      onComposerChange={(value) => {
        updateSideComposerDraft(sideChatSourceThreadId, (current) => ({
          ...current,
          text: value,
          textPresent: value.trim().length > 0,
        }));
        if (
          /^\/[a-z0-9_]*$/i.test(value) &&
          !commandsLoaded &&
          !commandsLoading
        ) {
          void loadSlashCommands();
        }
      }}
      onComposerCompositionEnd={(value) => {
        sideIsComposingRef.current = false;
        updateSideComposerDraft(sideChatSourceThreadId, (current) => ({
          ...current,
          text: value,
          textPresent: value.trim().length > 0,
        }));
        sideIgnoreComposerSubmitUntilRef.current = performance.now() + 80;
      }}
      onComposerCompositionStart={() => {
        sideIsComposingRef.current = true;
      }}
      onComposerInterrupt={() => {
        void interruptThread(sideChatThreadIdRef.current);
      }}
      onComposerSubmit={handleSideComposerSubmit}
      onJumpToLatestThreadLogs={() => {}}
      onLocalWorkspaceFileLinkClick={handleLocalFileLinkClick}
      onMarkIgnoreComposerSubmitWindow={() => {
        sideIgnoreComposerSubmitUntilRef.current = performance.now() + 80;
      }}
      onMessagesScroll={() => {
        const node = sideChatMessagesRef.current;
        if (
          sideChatThreadId &&
          node &&
          messagesNearEarlierUserTurnBoundary(node)
        ) {
          void loadOlderThreadHistoryPage(sideChatThreadId);
        }
      }}
      onMessagesUserScrollIntent={() => {}}
      onQueueDropTargetChange={setQueueDropTarget}
      onRemoveComposerFile={(fileId) => {
        removeSideComposerFile(sideChatSourceThreadId, fileId);
      }}
      onRemoveComposerImage={(imageId) => {
        removeSideComposerImage(sideChatSourceThreadId, imageId);
      }}
      onRemoveComposerBrowserAnnotation={(annotationId) => {
        removeSideComposerBrowserAnnotation(sideChatSourceThreadId, annotationId);
      }}
      onReorderQueuedIntent={reorderQueuedIntent}
      onSelectNewThreadAgent={() => {}}
      onSelectNewThreadWorkflow={() => {}}
      onSelectNewThreadWorkspaceMode={() => {}}
      onResumeProviderSession={handleResumeProviderSession}
      onRetryFailedMessage={(message) => {
        void handleRetryFailedMessage(message);
      }}
      onSelectBotBinding={(botId) => {
        if (sideChatThreadId) {
          void syncThreadBotBinding(sideChatThreadId, botId);
        }
      }}
      onOpenThreadById={(threadId) => {
        void openExistingThread(threadId);
      }}
      onOpenCapsule={(card) => {
        setContentView("capsules");
        setCapsulePreviewId(card.capsule_id);
      }}
      onSelectWorkspace={() => {}}
      onSetDraggedQueueIntentId={setDraggedQueueIntentId}
      onSteerQueuedPrompt={(item) => {
        void steerQueuedIntent(item, { canSteer: sideChatCanSteerQueuedPrompt });
      }}
      onThreadLogsContentScroll={() => {}}
      onThreadLogsResizeKeyDown={() => {}}
      onThreadLogsResizeStart={() => {}}
      preferredWorkspaceForNewThread={preferredWorkspaceForNewThread}
      queueDropTarget={queueDropTarget}
      selectableNewThreadWorkspaces={selectableNewThreadWorkspaces}
      selectedThreadId={sideChatThreadId}
      showAutomationRunInitialPlaceholder={false}
      showDreams={false}
      // Side chats fork the provider session without importing visible
      // history, so there is never parent history to wait for — the panel
      // opens as an empty thread instead of a loading placeholder.
      showHistoryLoadingPlaceholder={false}
      showTailThinking={sideChatShowTailThinking}
      threadLayoutRef={sideChatThreadLayoutRef}
      threadLogsError={null}
      threadLogsLoading={false}
      threadLogsMaxWidth={0}
      threadLogsOpen={false}
      threadLogsPanelWidth={0}
      threadLogsRef={threadLogsRef}
      threadLogsResizing={false}
      threadAvatarCatalog={threadAvatarCatalog}
      teamAgentDisplayNamesById={teamAgentDisplayNamesById}
      visibleRemoteAwaitingAckInputs={sideChatVisibleRemotePendingInputs}
      visibleRemotePendingInputs={sideChatVisibleRemotePendingInputs}
      workflowRunContent={null}
      workspaceMutation={workspaceMutation}
    />
  ) : (
    <div className="side-tool-empty">
      {sideChatCreating
        ? t("Starting…")
        : sideChatError || t("Start a focused side thread.")}
    </div>
  );

  // The dock is built for any thread (not only workspace threads) so it can host
  // capsule tabs opened from the transcript even when no workspace is attached
  // (#TASK-1470). Built-in workspace tools stay gated by `hasWorkspace` inside.
  const sideToolsPanel = contentView === "thread" ? (
    <ThreadSideToolsPanel
      activeWorkspaceName={activeWorkspace?.name || null}
      activeWorkspacePath={activeWorkspacePath}
      activeThreadId={selectedThreadId}
      selectedWorkspaceFile={selectedSideToolWorkspaceFile}
      sideChatPanel={sideChatPanel}
      workspaceBranch={composerWorkspaceBranch}
      workspaceDirectoryPanel={workspaceDirectoryPanel}
      workspaceFileFilter={workspaceFileFilter}
      workspaceFilePreview={workspaceFilePreview}
      workspaceFilePreviewError={workspaceFilePreviewError}
      workspaceFilePreviewLoading={workspaceFilePreviewLoading}
      workspaceMode={composerWorkspaceMode || "local"}
      workspacePreviewOpen={workspacePreviewModalOpen}
      workspacePreviewTitle={workspacePreviewTitle}
      hasWorkspace={Boolean(activeWorkspacePath)}
      openCapsuleTabs={openCapsuleTabs}
      pendingActiveCapsuleId={pendingActiveCapsuleId}
      onActivatePendingCapsuleHandled={() => setPendingActiveCapsuleId(null)}
      onCloseCapsuleTab={(capsuleId) => {
        setOpenCapsuleTabs((tabs) =>
          tabs.filter((tab) => tab.capsuleId !== capsuleId),
        );
      }}
      onCloseWorkspacePreview={closeWorkspacePreview}
      onLocalFileLinkClick={handleLocalFileLinkClick}
      onRevealSelectedWorkspaceFile={handleRevealSelectedWorkspaceFile}
      onAddBrowserAnnotationComment={handleAddBrowserAnnotationComment}
      onCloseSideTools={() => {
        // "Hide side tools" closes the whole dock. Since capsule tabs keep the
        // dock visible independently of inspectorOpen, clear them too so the
        // button works in a capsule-only dock and in Files+capsule docks
        // (#TASK-1470).
        setInspectorOpen(false);
        setOpenCapsuleTabs([]);
        setPendingActiveCapsuleId(null);
      }}
      onOpenTaskThread={(task) => {
        void openTaskThreadInSidePanel(task.threadId);
      }}
      onOpenSideChat={() => {
        void ensureSideChatThread();
      }}
      onWorkspaceFileFilterChange={setWorkspaceFileFilter}
    />
  ) : null;

  // The dock shows when the inspector is open (workspace tools) or any capsule
  // tab is open. Capsule visibility is independent of `inspectorOpen` so it is
  // not force-closed for no-workspace threads (#TASK-1470).
  const showConversationSideTools = Boolean(
    sideToolsPanel && (inspectorOpen || openCapsuleTabs.length > 0),
  );
  const conversationClassName = [
    "conversation",
    isSettingsView ? "settings-view" : null,
    isCapsulesView ? "capsules-view" : null,
    isAutomationView ? "automation-view" : null,
    isAgentsView || isTeamsView ? "agents-view" : null,
    isSkillsView ? "skills-view" : null,
    isTasksView ? "tasks-view" : null,
    isWorkflowView ? "workflow-view" : null,
    isDreamsView ? "dreams-view" : null,
    showConversationSideTools ? "with-side-tools" : null,
    sideToolsResizing ? "side-tools-resizing" : null,
  ]
    .filter(Boolean)
    .join(" ");
  const conversationStyle = showConversationSideTools
    ? ({
        "--side-tools-panel-width": `${sideToolsPanelWidth}px`,
      } as CSSProperties)
    : undefined;

  function renderPrimaryThreadPage(
    options: {
      embedded?: boolean;
      surfaceVariant?: "default" | "side-chat";
    } = {},
  ) {
    const embedded = options.embedded === true;
    return (
      <ThreadPage
        surfaceVariant={options.surfaceVariant}
        agentLabel={composerAgentLabel}
        composerAgentOptions={composerAgentOptions}
        composerWorkflowOptions={composerWorkflowOptions}
        composerWorkflowOptionsLoading={workflowDefinitionsLoading}
        activeMessages={activeMessages}
        activePendingAckIntents={visiblePendingAckIntents}
        activePendingAutomationRun={activePendingAutomationRun}
        activeToolGroupId={activeToolGroupId}
        activeQueue={activeQueue}
        renderState={activeRenderState}
        activeThreadLogsHasUnread={embedded ? false : activeThreadLogsHasUnread}
        activeThreadLogsPath={activeThreadLogsPath}
        activeThreadSummary={activeThread || null}
        activeThreadTitle={activeThread?.title || null}
        activeThreadRunId={activeThreadRunId}
        availableWorkspaceCount={availableWorkspaceCount}
        composer={composer}
        composerAttachmentInputRef={composerAttachmentInputRef}
        composerBrowserAnnotations={composerBrowserAnnotations}
        composerFiles={composerFiles}
        composerHasPayload={composerHasPayload}
        composerImages={composerImages}
        composerEditingLocked={composerEditingLocked}
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
        historyLoading={historyLoading}
        historyLoadingEarlier={Boolean(activeHistoryPagination?.loadingBefore)}
        ignoreComposerSubmitUntilRef={ignoreComposerSubmitUntilRef}
        inspectorOpen={embedded ? false : showConversationSideTools}
        isActiveSendingThread={isActiveSendingThread}
        canSteerQueuedPrompt={canSteerQueuedPrompt}
        isComposingRef={isComposingRef}
        messagesRef={messagesRef}
        threadLogLines={threadLogLines}
        newThreadSelectedAgentId={pendingAgentId}
        newThreadSelectedWorkflowId={pendingWorkflowId}
        newThreadProviderModels={pendingProviderModels}
        newThreadAgentConfiguredModel={pendingAgent?.model || null}
        newThreadSelectedModel={pendingModel}
        newThreadSelectedReasoningEffort={pendingModelReasoningEffort}
        newThreadSelectedServiceTier={pendingModelServiceTier}
        threadProviderModels={activeThreadProviderModels}
        threadEffectiveModel={activeThreadInfo?.model || null}
        threadEffectiveReasoningEffort={activeThreadInfo?.modelReasoningEffort || null}
        threadEffectiveServiceTier={activeThreadInfo?.modelServiceTier || null}
        threadSelectedModel={activeThreadInfo?.modelOverride || null}
        threadSelectedReasoningEffort={
          activeThreadInfo?.modelReasoningEffortOverride || null
        }
        threadSelectedServiceTier={activeThreadInfo?.modelServiceTierOverride || null}
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
          if (/^\/[a-z0-9_]*$/i.test(value) && !commandsLoaded && !commandsLoading) {
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
          setThreadLogsHasUnread(false);
          scrollThreadLogsToLatest("smooth");
        }}
        onLocalWorkspaceFileLinkClick={handleLocalFileLinkClick}
        onMarkIgnoreComposerSubmitWindow={markIgnoreComposerSubmitWindow}
        onMessagesScroll={() => {
          const node = messagesRef.current;
          shouldStickMessagesToBottomRef.current = messagesNearBottom(node);
          if (selectedThreadId && node && messagesNearEarlierUserTurnBoundary(node)) {
            void loadOlderThreadHistoryPage(selectedThreadId);
          }
        }}
        onMessagesUserScrollIntent={cancelMessagesForceScrollBudget}
        onQueueDropTargetChange={setQueueDropTarget}
        onRemoveComposerFile={removeComposerFile}
        onRemoveComposerImage={removeComposerImage}
        onRemoveComposerBrowserAnnotation={removeComposerBrowserAnnotation}
        onReorderQueuedIntent={reorderQueuedIntent}
        onSelectNewThreadAgent={(agentId) => {
          setPendingAgentId(agentId);
          setPendingWorkflowId(null);
        }}
        onSelectNewThreadModel={setPendingModel}
        onSelectNewThreadReasoningEffort={setPendingModelReasoningEffort}
        onSelectNewThreadServiceTier={setPendingModelServiceTier}
        onSelectThreadModel={(model) => {
          void handleUpdateActiveThreadRuntimeSettings({ model });
        }}
        onSelectThreadReasoningEffort={(modelReasoningEffort) => {
          void handleUpdateActiveThreadRuntimeSettings({
            modelReasoningEffort,
          });
        }}
        onSelectThreadServiceTier={(modelServiceTier) => {
          void handleUpdateActiveThreadRuntimeSettings({
            modelServiceTier,
          });
        }}
        onSelectNewThreadWorkflow={(workflowId) => {
          setPendingWorkflowId(workflowId);
          setPendingAgentId("claude");
        }}
        onSelectNewThreadWorkspaceMode={setPendingWorkspaceMode}
        onResumeProviderSession={handleResumeProviderSession}
        onRetryFailedMessage={(message) => {
          void handleRetryFailedMessage(message);
        }}
        onSelectBotBinding={(botId) => {
          if (selectedThreadId) {
            const threadId = selectedThreadId;
            setOptimisticThreadBotBinding({ threadId, botId });
            void handleSetBotBinding(botId).finally(() => {
              setOptimisticThreadBotBinding((current) => {
                return current?.threadId === threadId && current.botId === botId
                  ? null
                  : current;
              });
            });
          } else {
            setPendingBotId(botId);
          }
        }}
        onOpenThreadById={(threadId) => {
          if (embedded) {
            void selectExistingThreadInPlace(threadId, "tasks");
          } else {
            void openExistingThread(threadId);
          }
        }}
        onOpenCapsule={(card) => {
          if (!selectedThreadId) {
            return;
          }
          // Open/activate this capsule as a tab in the right dock (#TASK-1470).
          // Dedup by id; refresh title/revision if it is already open. Does not
          // touch inspectorOpen — the capsule path drives the dock on its own.
          const capsuleId = card.capsule_id;
          const title = card.title?.trim() || "";
          setOpenCapsuleTabs((tabs) =>
            tabs.some((tab) => tab.capsuleId === capsuleId)
              ? tabs.map((tab) =>
                  tab.capsuleId === capsuleId
                    ? { ...tab, revision: card.revision, title: title || tab.title }
                    : tab,
                )
              : [
                  ...tabs,
                  { capsuleId, revision: card.revision, title },
                ],
          );
          setPendingActiveCapsuleId(capsuleId);
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
            setThreadLogsHasUnread(false);
          }
        }}
        onThreadLogsResizeKeyDown={embedded ? () => {} : handleThreadLogsResizeKeyDown}
        onThreadLogsResizeStart={embedded ? () => {} : handleThreadLogsResizeStart}
        preferredWorkspaceForNewThread={preferredWorkspaceForNewThread}
        queueDropTarget={queueDropTarget}
        selectableNewThreadWorkspaces={selectableNewThreadWorkspaces}
        selectedThreadId={selectedThreadId}
        showAutomationRunInitialPlaceholder={showAutomationRunInitialPlaceholder}
        showDreams={showDreamsFeature}
        showHistoryLoadingPlaceholder={showHistoryLoadingPlaceholder}
        showTailThinking={showTailThinking}
        rateLimit={activeRateLimit}
        threadLayoutRef={threadLayoutRef}
        threadLayoutStyle={
          !embedded && threadLogsOpen
            ? ({
                "--thread-log-panel-width": `${threadLogsPanelWidth}px`,
              } as CSSProperties)
            : undefined
        }
        threadLogsError={embedded ? null : threadLogsError}
        threadLogsLoading={embedded ? false : threadLogsLoading}
        threadLogsMaxWidth={
          embedded
            ? 0
            : clampThreadLogsPanelWidth(
                THREAD_LOG_PANEL_MAX_WIDTH,
                currentThreadLayoutWidth(),
              )
        }
        threadLogsOpen={embedded ? false : threadLogsOpen}
        threadLogsPanelWidth={embedded ? 0 : threadLogsPanelWidth}
        threadLogsRef={threadLogsRef}
        threadLogsResizing={embedded ? false : threadLogsResizing}
        threadAvatarCatalog={threadAvatarCatalog}
        teamAgentDisplayNamesById={teamAgentDisplayNamesById}
        visibleRemoteAwaitingAckInputs={visibleRemoteAwaitingAckInputs}
        visibleRemotePendingInputs={visibleRemotePendingInputs}
        workflowRunContent={
          !embedded && activeWorkflowRunThreadId ? (
            <WorkflowRunsPanel
              onOpenThread={(threadId) => {
                void openExistingThread(threadId);
              }}
              onToast={pushToast}
              t={t}
              workflowRunId={activeWorkflowRunThreadId}
            />
          ) : null
        }
        workspaceMutation={workspaceMutation}
      />
    );
  }

  if (loading) {
    return (
      <GatewayMirrorContext.Provider value={gatewayMirror}>
      <I18nProvider languagePreference={settingsDraft.languagePreference}>
        <div className="startup-shell" role="status" aria-live="polite">
          <div className="startup-panel">
            <img
              alt=""
              aria-hidden="true"
              className="startup-mark"
              draggable={false}
              src={garyxIconUrl}
            />
            <div className="startup-copy">
              <strong>{t('Starting Garyx')}</strong>
              <span>{t('Syncing workspace state and gateway status...')}</span>
            </div>
            <div className="startup-progress" aria-hidden="true" />
          </div>
        </div>
      </I18nProvider>
      </GatewayMirrorContext.Provider>
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
      <GatewayMirrorContext.Provider value={gatewayMirror}>
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
                        gatewayHeaders: profile.gatewayHeaders,
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

              <div className="gateway-setup-field">
                <GatewayHeadersEditor
                  value={settingsDraft.gatewayHeaders}
                  onChange={(value) => {
                    setLocalSettingsStatus(null);
                    setSettingsDraft((current) => ({
                      ...current,
                      gatewayHeaders: value,
                    }));
                  }}
                />
              </div>
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
      </GatewayMirrorContext.Provider>
    );
  }

  return (
    <GatewayMirrorContext.Provider value={gatewayMirror}>
    <I18nProvider languagePreference={settingsDraft.languagePreference}>
    <div
      className={appShellClassName}
      style={
        {
          "--spacing-token-sidebar": sidebarCollapsed ? "0px" : `${sidebarWidth}px`,
        } as React.CSSProperties
      }
    >
      <ToastViewport onDismiss={dismissToast} toasts={toasts} />
      <button
        aria-label={t("Toggle Sidebar")}
        aria-pressed={sidebarCollapsed}
        className="sidebar-collapse-toggle"
        onClick={toggleSidebarCollapsed}
        title={t("Toggle Sidebar")}
        type="button"
      >
        <PanelLeft aria-hidden size={15} strokeWidth={1.8} />
      </button>
      <AppLeftRail
        gatewayIdentitySlot={
          <GatewayIdentityBar
            connection={connection}
            currentGatewayUrl={persistedGatewayUrl}
            indicatorTone={gatewayIndicator?.tone || null}
            profiles={gatewayProfiles}
            onOpenSettings={() => {
              void openSettingsView();
            }}
            onSwitch={async (profile) => {
              return handleSaveLocalSettingsDraft(
                {
                  ...settingsDraft,
                  gatewayUrl: profile.gatewayUrl,
                  gatewayAuthToken: profile.gatewayAuthToken,
                  gatewayHeaders: profile.gatewayHeaders,
                },
                { requireGatewayConnection: true },
              );
            }}
          />
        }
        activeBotConversationGroupId={
          shouldShowConversationRail ? botConversationGroupId : null
        }
        activeWorkspaceThreadGroupPath={
          shouldShowConversationRail ? workspaceConversationPath : null
        }
        botGroups={visibleBotGroups}
        formatThreadTimestamp={formatThreadTimestamp}
        isAutomationView={isAutomationView}
        isCapsulesView={isCapsulesView}
        showDreams={showDreamsFeature}
        isAgentsView={isAgentsView}
        isBrowserView={isBrowserView}
        isTeamsView={isTeamsView}
        isSettingsView={isSettingsView}
        isSkillsView={isSkillsView}
        isTasksView={isTasksView || isWorkflowView}
        isDreamsView={isDreamsView}
        recentRailOpen={shouldShowConversationRail && recentThreadsRailOpen}
        onBackToThreads={() => {
          setContentView("thread");
        }}
        onCreateThreadForWorkspace={(workspacePath) => {
          handleCreateThreadForWorkspace(workspacePath);
        }}
        onNewThread={() => {
          void handleNewThread();
        }}
        onOpenRecent={() => {
          setBotConversationGroupId(null);
          setWorkspaceConversationPath(null);
          if (!shouldShowConversationRail) {
            setContentView("thread");
            setRecentThreadsRailOpen(true);
            return;
          }
          setRecentThreadsRailOpen((current) => !current);
        }}
        onOpenBot={(group) => {
          void (async () => {
            setRecentThreadsRailOpen(false);
            setBotConversationGroupId((current) =>
              current === group.id ? current : null,
            );
            setWorkspaceConversationPath(null);
            await handleBotClick(group);
          })();
        }}
        onOpenPinnedThread={(threadId) => {
          void (async () => {
            setRecentThreadsRailOpen(false);
            setBotConversationGroupId(null);
            setWorkspaceConversationPath(null);
            await openExistingThread(threadId, "pinned");
          })();
        }}
        onUnpinThread={(threadId) => {
          togglePinnedThread(threadId);
        }}
        onArchivePinnedThread={(threadId) => {
          void handleDeleteThread(threadId);
        }}
        onToggleBotConversationGroup={(group) => {
          setRecentThreadsRailOpen(false);
          setWorkspaceConversationPath(null);
          setBotConversationGroupId((current) =>
            current === group.id ? null : group.id,
          );
        }}
        onToggleWorkspaceThreadGroup={(workspacePath) => {
          setRecentThreadsRailOpen(false);
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
          void openSettingsView();
        }}
        onSidebarResizeStart={handleSidebarResizeStart}
        sidebarResizing={sidebarResizing}
        onOpenAgents={() => {
          setContentView("agents");
        }}
        onOpenSkills={() => {
          setContentView("skills");
        }}
        onOpenCapsules={() => {
          setContentView("capsules");
          setCapsulePreviewId(null);
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
          onArchiveThread={(threadId) => {
            void handleDeleteThread(threadId);
          }}
          onOpenThread={(threadId) => {
            void openExistingThread(threadId, "workspace-conversation");
          }}
          onRailResizeStart={handleRailResizeStart}
          railResizing={railResizing}
          selectedThreadId={workspaceConversationSelectedThreadId}
          threadAvatarCatalog={threadAvatarCatalog}
        />
      ) : shouldShowConversationRail && recentThreadsRailOpen ? (
        <ThreadConversationSidebar
          ariaLabel={t("Recent threads")}
          className="recent-conversation-rail"
          collapseLabel={t("Collapse recent threads")}
          emptyLabel={t("No recent threads")}
          formatThreadTimestamp={formatThreadTimestamp}
          logo={
            <span className="recent-conversation-logo">
              <RecentIcon />
            </span>
          }
          onClose={() => {
            setRecentThreadsRailOpen(false);
          }}
          onRailResizeStart={handleRailResizeStart}
          railResizing={railResizing}
          rowClassName="recent-conversation-row-shell"
          rows={recentThreadRows.map((row) => ({
            key: row.thread.id,
            title: row.thread.title,
            time: row.thread.updatedAt,
            avatar: resolveThreadAvatarIdentity(row.thread, threadAvatarCatalog),
            isActive: row.isActive,
            isBusy: row.isBusy,
            onOpen: () => {
              void openExistingThread(row.thread.id, "recent");
            },
            onArchive: row.isBusy
              ? undefined
              : () => {
                  void handleDeleteThread(row.thread.id);
                },
          }))}
          title={t("Recent")}
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
        <main
          className={conversationClassName}
          ref={conversationRef}
          style={conversationStyle}
        >
          {isCapsulesView || isTasksView || isWorkflowView || isDreamsView ? null : showStaticWindowToolbar ? (
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
                threadLogsHasUnread={threadLogsHasUnread}
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
                  // Logs and the side-tools dock are mutually exclusive right
                  // panels; opening logs closes the dock, capsule tabs included.
                  setOpenCapsuleTabs([]);
                  setPendingActiveCapsuleId(null);
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
                    gatewayProfiles={gatewayProfiles}
                    localSettingsDirty={localSettingsDirty}
                    localSettings={settingsDraft}
                    onAddGatewayProfile={async (input) => {
                      const nextState = await window.garyxDesktop.addGatewayProfile(input);
                      setDesktopState(nextState);
                    }}
                    onUpdateGatewayProfile={async (input) => {
                      const nextState = await window.garyxDesktop.updateGatewayProfile(input);
                      setDesktopState(nextState);
                      setSettingsDraft((current) => ({
                        ...current,
                        gatewayUrl: nextState.settings.gatewayUrl,
                        gatewayAuthToken: nextState.settings.gatewayAuthToken,
                        gatewayHeaders: nextState.settings.gatewayHeaders,
                      }));
                      const status = await window.garyxDesktop.checkConnection();
                      setConnection(status);
                    }}
                    onDeleteGatewayProfile={async (profileId) => {
                      const nextState = await window.garyxDesktop.deleteGatewayProfile({
                        profileId,
                      });
                      setDesktopState(nextState);
                    }}
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
            ) : isCapsulesView ? (
              <CapsulesPanel
                agents={desktopAgents}
                onToast={pushToast}
                selectedCapsuleIdFromRoute={
                  isCapsulesView ? capsulePreviewId : null
                }
                onOpenCapsulePreview={(capsuleId) => {
                  setCapsulePreviewId(capsuleId);
                }}
                onCloseCapsulePreview={() => {
                  setCapsulePreviewId(null);
                }}
                onOpenThread={(threadId) => {
                  void openExistingThread(threadId);
                }}
              />
            ) : isTasksView ? (
              <TasksPanel
                agents={desktopAgents}
                botGroups={botGroups}
                onAddWorkspace={addWorkspacePathFromPicker}
                onOpenThread={(threadId) => {
                  void openExistingThread(threadId);
                }}
                onOpenWorkflowTask={(task) => {
                  openWorkflowTask(task);
                }}
                onToast={pushToast}
                workspaces={workspacePickerWorkspaces}
                workspaceMutation={workspaceMutation}
              />
            ) : isWorkflowView && selectedWorkflowTaskId ? (
              selectedWorkflowRunId ? (
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
                  workflowRunId={selectedWorkflowRunId}
                />
              ) : (
                <div className="workflow-runs-page">
                  <section
                    aria-label={t("Workflow runs")}
                    className="workflow-runs-panel"
                  >
                    <div className="workflow-runs-body">
                      <div
                        className={
                          error
                            ? "workflow-runs-state workflow-runs-state-error"
                            : "workflow-runs-state"
                        }
                      >
                        {error || t("Loading workflow runs…")}
                      </div>
                    </div>
                  </section>
                </div>
              )
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
              renderPrimaryThreadPage()
            )}
            </Suspense>
          </section>
          {showConversationSideTools ? (
            <>
              <div
                aria-label={t("Resize side tools")}
                aria-orientation="vertical"
                aria-valuemax={clampSideToolsPanelWidth(
                  SIDE_TOOLS_PANEL_MAX_WIDTH,
                  currentConversationWidth(),
                )}
                aria-valuemin={SIDE_TOOLS_PANEL_MIN_WIDTH}
                aria-valuenow={sideToolsPanelWidth}
                className="side-tools-resizer"
                onKeyDown={handleSideToolsResizeKeyDown}
                onPointerDown={handleSideToolsResizeStart}
                role="separator"
                tabIndex={0}
              />
              {sideToolsPanel}
            </>
          ) : null}
        </main>
      )}
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

      {/* Electron composes window drag regions in document order (union for
          drag, difference for no-drag), and only at load time — runtime
          style/DOM edits never re-report them. So the no-drag hole must be
          re-punched by this last app-shell child. It cannot be an empty box
          (the collector skips boxes with no painted content — hence the icon)
          and must not be a button (mouse clicks land here because it stacks
          on top, and a focusable twin would steal focus and get force-exposed
          in the AX tree). The early sibling button owns keyboard focus order
          and screen-reader semantics. */}
      <div
        aria-hidden="true"
        className="sidebar-collapse-toggle sidebar-collapse-toggle-carveout"
        onClick={toggleSidebarCollapsed}
      >
        <PanelLeft aria-hidden size={15} strokeWidth={1.8} />
      </div>

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
    </GatewayMirrorContext.Provider>
  );
}
