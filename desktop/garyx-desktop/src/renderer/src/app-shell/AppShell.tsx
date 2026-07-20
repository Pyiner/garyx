import {
  Suspense,
  lazy,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import { startTransition } from "react";
import { PanelLeft } from "lucide-react";

import {
  type DesktopApiProviderType,
  type DesktopAgentCatalog,
  type DesktopBotConsoleSummary,
  type DesktopCustomAgent,
  type ConnectionStatus,
  type DesktopChatStreamEvent,
  type DesktopChannelEndpoint,
  type DesktopProviderModels,
  type DesktopSessionProviderHint,
  type DesktopState,
  type DesktopThreadSummary,
  type DesktopThreadPinOrderSnapshot,
  type DesktopWorkspace,
  type DraftWorkspaceSelection,
  type DesktopWorkspaceMode,
  type ThreadRuntimeInfo,
  type ThreadTranscript,
  type WindowLayoutBootstrap,
} from "@shared/contracts";

import { desktopStateWithoutThread } from "@shared/desktop-state";
import { isToolRole } from "@shared/transcript-sync";

import {
  isRuntimeBusy,
  selectGlobalActiveThreadId,
  selectQueueIntentIds,
  selectThreadRuntime,
  type MessageMachineAction,
  type MessageMachineState,
  type MessageIntent,
  type ThreadRuntimeState,
} from "../message-machine";
import type { SettingsTabId } from "../settings-tabs";
import { GatewayProfileHistoryButton } from "../GatewayProfileHistoryButton";
import { GatewayHeadersEditor } from "../GatewayHeadersEditor";
import { GatewayIdentityBar } from "../GatewaySwitcher";
import { SettingsErrorBoundary } from "../SettingsErrorBoundary";
import { Input } from "../components/ui/input";
import { WorkspacePathPickerDialog } from "../components/WorkspacePathPicker";
import { WorkspaceRenameDialog } from "../components/WorkspaceRenameDialog";
import { workspaceGitStatusCache } from "../workspace-git-status-cache";
import { WorkspaceEpochContext } from "../components/workspace-data-adapter";
// Side-effect import: wires cross-store capsule cache invalidation (a `/serve`
// 404 in either the HTML or thumbnail store tombstones the other for that id).
import "./capsule-cache";
import {
  AddBotDialogRoot,
  type AddBotDialogHandle,
} from "./components/AddBotDialogRoot";
import { WorkspaceFileTree } from "./components/WorkspaceFileTree";
import {
  ThreadSideToolsPanel,
  type SideCapsuleTab,
  type SideToolWorkspaceFile,
} from "./components/SideToolsPanel";
import { BotConversationSidebar } from "../BotConversationSidebar";
import { RecentConversationSidebar } from "../RecentConversationSidebar";
import { ConversationHeaderActions } from "../ConversationHeaderActions";
import {
  ConversationTitleRoot,
  type ConversationTitleHandle,
} from "./components/ConversationTitleRoot";
import { ToastViewportHost, useToastActions } from "../toast-provider";
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
} from "../bot-console-model";
import {
  automationForLatestThread,
  buildWorkspaceThreadGroups,
  isSelectableNewThreadWorkspace,
  newThreadWorkspaceOptions,
  pickPreferredWorkspace,
  selectedThread,
  selectedWorkspace,
  resolveDefaultDraftWorkspace,
  visibleWorkspaceList,
  workspaceForThread,
  workspaceSuggestionFromPath,
} from "../thread-model";
import {
  buildThreadAvatarCatalog,
  resolveThreadAvatarIdentity,
} from "../thread-avatar";
import {
  ensureThread,
  scheduleThreadHistoryRefresh,
  startNewThreadDraft,
  updateThreadBotBinding,
} from "../thread-controller";
import {
  RecentIcon,
} from "./icons";
import type {
  BoundBot,
  LiveStreamState,
  MessageMap,
  PendingAutomationRun,
  UiTranscriptMessage,
} from "./types";
import { AppLeftRail } from "./components/AppLeftRail";
import { ThreadPage } from "./components/ThreadPage";
import { useAutomationController } from "./useAutomationController";
import {
  SIDE_TOOLS_PANEL_MAX_WIDTH,
  clampSideToolsPanelWidth,
  computeGatewayIndicator,
  sideToolsPanelMinWidth,
} from "./diagnostics-helpers";
import {
  isKnownThreadId,
  useRouteEffectBridge,
  waitForMs,
} from "./route-effect-bridge";
import { useGatewayConnectionController } from "./useGatewayConnectionController";
import { useLayoutResizeController } from "./useLayoutResizeController";
import {
  appendLayoutOccupancyIntent,
  createLayoutOccupancyEventLog,
  type LayoutIntentCause,
  type LayoutOccupancySources,
} from "./layout-occupancy-events";
import { resolveMemoryDialogTargetFromPath } from "./useMemoryDialogController";
import {
  MemoryDialogRoot,
  type MemoryDialogHandle,
} from "./components/MemoryDialogRoot";
import {
  NEW_THREAD_DRAFT_THREAD_ID,
  prepareAttachmentUploads,
  useMessageDispatchController,
  type SeededTurn,
} from "./useMessageDispatchController";
import { GatewayMirror } from "../gateway-mirror/mirror";
import type { DispatchOrchestratorDeps } from "../gateway-mirror/dispatch-orchestrator";
import {
  GatewayMirrorContext,
  useGatewayThreadMirror,
} from "../gateway-mirror/react";
import { useSettingsController } from "./useSettingsController";
import {
  scrollMessagesToLatest,
  type TranscriptScrollIntent,
} from "./components/thread-transcript-scroll";
import { SideChatSessions } from "./side-chat-sessions";
import {
  beginPinnedOrderGatewaySwitch,
  PinnedOrderIngress,
  installPinnedOrderIngress,
  normalizeGatewayIdentity,
  requestDesktopState,
  requestDesktopStateResult,
  restorePinnedOrderGatewayDomain,
} from "../pinned-order-ingress";
import {
  ensureSideChatThread as ensureSideChatThreadOp,
  type SideChatOpsContext,
} from "./side-chat-ops";
import { SideChatPanel } from "./components/SideChatPanel";
import { loadThreadHistory } from "../thread-controller";
import {
  transcriptHasAutomationResponse,
} from "../gateway-mirror/transcript-materialize";
import {
  pendingAckIntentsNotRepresented,
  representedUserIntentIds,
} from "./pending-ack-intents";
import { useWorkspaceController } from "./useWorkspaceController";
import {
  workspaceDirectoryKey,
  workspaceFileAbsolutePath,
} from "./workspace-helpers";
import {
  summarizeRemoteStateErrors,
} from "./gateway-errors";
import { buildAgentOptions, buildAgentTargetOptions } from "./agent-options";
import {
  canUseAgentForNewBinding,
  isNewDraftBindingBlocked,
  suggestedAgentId,
} from "./agent-availability-model";
import {
  I18nProvider,
  createTranslator,
  useResolvedLocale,
} from "../i18n";
import garyxIconUrl from "../assets/garyx-icon.png";
import {
  contentViewForDesktopRoute,
  draftSelectionFromRouteWorkspace,
  routeWorkspaceFromDraftSelection,
  type DesktopRoute,
} from "./desktop-route";
import {
  DesktopRouteStore,
  createBrowserRouteHost,
} from "./desktop-route-store";
import { useRecentThreadFeeds } from "./useRecentThreadFeeds";
import { useThreadFavorites } from "./useThreadFavorites";
import { presentedFavoriteRows } from "./favorites-ingress";
import {
  lifecycleUiSettlement,
  resolveLifecycleStoreIncarnation,
  runLifecycleMutation,
} from "./lifecycle-ingress";
import type { RecentThreadFeedState } from "./recent-thread-feeds";
import { recordTranscriptRender } from "./transcript-render-probe";
import {
  deferConversationRailUnmount,
  settleDeferredConversationRailUnmount,
  type ConversationRailIntent,
} from "./conversation-rail-lifecycle";


type ThreadEntrySelectionSource =
  | "pinned"
  | "recent"
  | "bot-root"
  | "bot-conversation";

function draftSelectionWorkspacePath(
  selection: DraftWorkspaceSelection | null,
): string | null {
  return selection?.kind === "path" ? selection.path : null;
}

type LegacyLayoutIntentState = {
  globalSidebarOpen: boolean;
  conversationRail: ConversationRailIntent;
  inspectorOpen: boolean;
  openCapsuleTabs: SideCapsuleTab[];
};

type LegacyLayoutIntentUpdate = (
  current: LegacyLayoutIntentState,
) => LegacyLayoutIntentState;

function conversationRailIntentFromLegacyState(input: {
  botConversationGroupId: string | null;
  recentThreadsRailOpen: boolean;
}): ConversationRailIntent {
  if (input.botConversationGroupId) {
    return { kind: "bot", groupId: input.botConversationGroupId };
  }
  return input.recentThreadsRailOpen ? { kind: "recent" } : { kind: "closed" };
}

function conversationRailKey(intent: ConversationRailIntent): string | null {
  switch (intent.kind) {
    case "recent":
      return "recent";
    case "bot":
      return `bot:${intent.groupId}`;
    case "closed":
      return null;
  }
}

function layoutOccupancySources(
  state: LegacyLayoutIntentState,
): LayoutOccupancySources {
  return {
    globalSidebar: state.globalSidebarOpen,
    conversationRailKey: conversationRailKey(state.conversationRail),
    inspectorOpen: state.inspectorOpen,
    openCapsuleCount: state.openCapsuleTabs.length,
  };
}

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
const EMPTY_UI_TRANSCRIPT_MESSAGES: UiTranscriptMessage[] = [];
const EMPTY_DESKTOP_AGENT_CATALOG: DesktopAgentCatalog = {
  agents: [],
  defaultAgentId: null,
  effectiveDefaultAgentId: null,
};
const EMPTY_DESKTOP_THREAD_SUMMARIES: DesktopThreadSummary[] = [];


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

const STARTUP_HYDRATION_RETRY_DELAYS_MS = [0, 300, 650, 1_100, 1_700];

function threadRunStateIsRunning(thread: DesktopThreadSummary): boolean {
  return (thread.runState || "").trim().toLowerCase() === "running";
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
  threadInfo: ThreadRuntimeInfo | null,
  desktopState: DesktopState | null,
  desktopAgents: DesktopCustomAgent[],
): DesktopApiProviderType | null {
  const runtimeProvider = threadInfo?.providerType;
  if (
    runtimeProvider === "claude_code" ||
    runtimeProvider === "codex_app_server" ||
    runtimeProvider === "antigravity" ||
    runtimeProvider === "traex"
  ) {
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
  if (agentId === "claude") {
    return "claude_code";
  }
  return (
    desktopAgents.find((agent) => agent.agentId === agentId)?.providerType ||
    null
  );
}

export function AppShell() {
  recordTranscriptRender("AppShell");
  // Batch 4b: the DesktopRouteStore owns the URL hash. It seeds from the
  // initial location, navigate() is the only hash writer (the legacy
  // state-to-hash replace effect routes through it), and external
  // hash/popstate edits reach applyDesktopRoute through subscribeExternal.
  const [desktopRouteStore] = useState(
    () => new DesktopRouteStore(createBrowserRouteHost()),
  );
  const initialRouteRef = useRef<DesktopRoute | null>(null);
  if (!initialRouteRef.current) {
    initialRouteRef.current = desktopRouteStore.getSnapshot().route;
  }
  const initialRouteValue = initialRouteRef.current;
  const initialWindowLayoutRef = useRef<Readonly<{
    bootstrap: WindowLayoutBootstrap;
    rendererEpoch: string;
  }> | null | undefined>(undefined);
  if (initialWindowLayoutRef.current === undefined) {
    if (window.garyxDesktop.horizontalLayoutPolicy === "expand-v1") {
      const rendererEpoch =
        typeof crypto.randomUUID === "function"
          ? crypto.randomUUID()
          : `renderer-${Date.now()}-${Math.random().toString(36).slice(2)}`;
      initialWindowLayoutRef.current = {
        rendererEpoch,
        bootstrap: window.garyxDesktop.getWindowLayoutBootstrap({
          rendererEpoch,
        }),
      };
    } else {
      initialWindowLayoutRef.current = null;
    }
  }
  const initialWindowLayout = initialWindowLayoutRef.current;
  const restoredLayoutOccupancy =
    initialWindowLayout && !initialWindowLayout.bootstrap.freshSession
      ? initialWindowLayout.bootstrap.acknowledgedSession.desiredOccupancy
      : null;
  const [pinnedOrderIngress] = useState(() => {
    const rendererSessionId =
      initialWindowLayout?.rendererEpoch ||
      (typeof crypto.randomUUID === "function"
        ? crypto.randomUUID()
        : `pins-renderer-${Date.now()}-${Math.random().toString(36).slice(2)}`);
    const ingress = new PinnedOrderIngress(rendererSessionId);
    installPinnedOrderIngress(ingress);
    return ingress;
  });
  const [pinnedOrderMainSnapshot, setPinnedOrderMainSnapshot] =
    useState<DesktopThreadPinOrderSnapshot | null>(null);
  // Endgame architecture (docs/design/appshell-endgame-architecture.md):
  // the mirror instance is created once and provided via context. During
  // the migration it runs alongside the legacy React state; batches move
  // ownership over domain by domain.
  const [gatewayMirror] = useState(
    () =>
      new GatewayMirror({
        getState: async () => {
          const state = await requestDesktopState(() => window.garyxDesktop.getState());
          void window.garyxDesktop
            .getThreadPinOrderSnapshot()
            .then(setPinnedOrderMainSnapshot)
            .catch(() => undefined);
          return state;
        },
        listCustomAgents: () => window.garyxDesktop.listCustomAgents(),
        getThreadHistory: (input) => window.garyxDesktop.getThreadHistory(input),
        getThreadHistoryFull: (threadId) =>
          window.garyxDesktop.getThreadHistory(threadId),
        saveThreadTranscriptCache: (transcript, renderState) =>
          window.garyxDesktop.saveThreadTranscriptCache(transcript, renderState),
        loadThreadTranscriptCache: (threadId) =>
          window.garyxDesktop.loadThreadTranscriptCache(threadId),
        clearThreadTranscriptCache: (threadId) =>
          window.garyxDesktop.clearThreadTranscriptCache(threadId),
        startThreadStream: (input) =>
          window.garyxDesktop.startThreadStream(input),
        stopThreadStream: (input) => window.garyxDesktop.stopThreadStream(input),
        // Temporary batch-3 seam: the message machine's intent lookup, read
        // through the machine-state getter. The closure runs post-mount, so
        // the later declaration it captures is initialized by the time it
        // is read.
        intentForId: (intentId) =>
          messageStateRef.current.intentsById[intentId] || null,
      }),
  );
  // 5b-7a: shell-owned side-chat session store (bindings/drafts/transients
  // outlive the inspector dock; its shadow refs feed the orchestration deps).
  const [sideChatSessions] = useState(() => new SideChatSessions());
  const [desktopState, setDesktopStateRaw] = useState<DesktopState | null>(null);
  const setDesktopState = useCallback<
    React.Dispatch<React.SetStateAction<DesktopState | null>>
  >(
    (action) => {
      setDesktopStateRaw((current) =>
        pinnedOrderIngress.commitState(current, action),
      );
    },
    [pinnedOrderIngress],
  );
  const [desktopAgentCatalog, setDesktopAgentCatalog] =
    useState<DesktopAgentCatalog>(EMPTY_DESKTOP_AGENT_CATALOG);
  const desktopAgents = desktopAgentCatalog.agents;
  const [connection, setConnection] = useState<ConnectionStatus | null>(null);
  const [selectedThreadId, setSelectedThreadId] = useState<string | null>(() =>
    initialRouteValue.kind === "thread" ? initialRouteValue.threadId : null,
  );
  // Capsule preview selection lives here (single source of truth) so the route,
  // deep links, and gallery clicks all flow through one path.
  // Batch 6c-2c: the capsule preview id is a selector over the committed
  // route (declared below the route snapshot).
  // Capsules opened as tabs in the right side-tools dock (#TASK-1470). AppShell
  // owns the list so the dock can show without a workspace; the panel renders
  // these and owns active-tab selection. `pendingActiveCapsuleId` is a one-shot
  // request to activate a capsule's tab (consumed by the panel).
  const [openCapsuleTabs, setOpenCapsuleTabsLegacy] = useState<
    SideCapsuleTab[]
  >([]);
  const [pendingActiveCapsuleId, setPendingActiveCapsuleId] = useState<
    string | null
  >(null);
  const [threadEntrySelectionSource, setThreadEntrySelectionSource] =
    useState<ThreadEntrySelectionSource | null>(null);
  const [newThreadDraftActive, setNewThreadDraftActive] = useState(
    initialRouteValue.kind === "new-thread",
  );
  const [pendingWorkspaceSelection, setPendingWorkspaceSelection] =
    useState<DraftWorkspaceSelection | null>(
      initialRouteValue.kind === "new-thread"
        ? draftSelectionFromRouteWorkspace(initialRouteValue.workspacePath)
        : null,
    );
  const [pendingWorkspaceMode, setPendingWorkspaceMode] =
    useState<DesktopWorkspaceMode>("local");
  const [pendingBotId, setPendingBotId] = useState<string | null>(null);
  const [optimisticThreadBotBinding, setOptimisticThreadBotBinding] = useState<{
    botId: string | null;
    threadId: string;
  } | null>(null);
  const [pendingAgentId, setPendingAgentId] = useState<string | null>(
    initialRouteValue.kind === "new-thread" && initialRouteValue.agentId
      ? initialRouteValue.agentId
      : null,
  );
  const [pendingModel, setPendingModel] = useState<string | null>(null);
  const [pendingModelReasoningEffort, setPendingModelReasoningEffort] =
    useState<string | null>(null);
  const [pendingModelServiceTier, setPendingModelServiceTier] =
    useState<string | null>(null);
  const [providerModelsByType, setProviderModelsByType] = useState<
    Record<string, DesktopProviderModels | null>
  >({});
  const hasNewThreadDraft = newThreadDraftActive && !selectedThreadId;
  const activeThreadMessageKey =
    selectedThreadId ||
    (hasNewThreadDraft ? NEW_THREAD_DRAFT_THREAD_ID : null);
  // AppShell renders the GatewayMirror provider, so it uses the explicit-
  // instance binding. Only the selected/draft thread can invalidate this
  // snapshot; background transcript commits never enter the shell.
  const activeThreadMirror = useGatewayThreadMirror(
    gatewayMirror,
    activeThreadMessageKey,
  );
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
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [historyLoading, setHistoryLoading] = useState(false);
  // Batch 5b: title edit state lives in ConversationTitleRoot; the shell
  // keeps a handle for the transcript controller's remote title sync.
  const conversationTitleRef = useRef<ConversationTitleHandle | null>(null);
  const [deletingThreadId, setDeletingThreadId] = useState<string | null>(null);
  const [bindingMutation, setBindingMutation] = useState<string | null>(null);
  const [inspectorOpen, setInspectorOpenLegacy] = useState(
    restoredLayoutOccupancy?.sideTools ?? false,
  );
  const [botConversationGroupId, setBotConversationGroupIdLegacy] = useState<
    string | null
  >(null);
  const [recentThreadsRailOpen, setRecentThreadsRailOpenLegacy] =
    useState(restoredLayoutOccupancy?.conversationRail ?? false);
  // Batch 6c-2b: contentView is a SELECTOR over the committed route — the
  // route store is the only view state (AppShell subscribes on its local
  // store instance, not through context: AppShell renders the Provider).
  // Writers are gone; view changes navigate, and the open-thread /
  // draft-entry commands sync the route so direct callers flip the view
  // through the same commit.
  const routeSnapshot = useSyncExternalStore(
    useCallback(
      (onChange) => desktopRouteStore.subscribe(onChange),
      [desktopRouteStore],
    ),
    () => desktopRouteStore.getSnapshot(),
  );
  const contentView = contentViewForDesktopRoute(routeSnapshot.route);
  // 6c-2c id selectors: routed ids read the committed route directly.
  const capsulePreviewId =
    routeSnapshot.route.kind === "capsule"
      ? routeSnapshot.route.capsuleId
      : null;
  // Settings tab: route-selected with last-active stickiness — plain
  // #/settings shows the previously active tab (design contract:
  // route.tabId ?? last ?? 'labs'); selecting a tab navigates.
  const lastSettingsTabRef = useRef<SettingsTabId | null>(null);
  const settingsActiveTab: SettingsTabId =
    routeSnapshot.route.kind === "settings"
      ? (routeSnapshot.route.tabId ?? lastSettingsTabRef.current ?? "labs")
      : (lastSettingsTabRef.current ?? "labs");
  useEffect(() => {
    if (routeSnapshot.route.kind === "settings") {
      lastSettingsTabRef.current = settingsActiveTab;
    }
  }, [routeSnapshot, settingsActiveTab]);
  useEffect(() => {
    if (contentView !== "thread" || !selectedThreadId) {
      setThreadEntrySelectionSource(null);
    }
  }, [contentView, selectedThreadId]);
  // Batch 5b: the add-bot dialog is a colocated feature root; the shell
  // keeps a handle (the legacy addBotInitialValues state was
  // write-only-null dead state and is dropped).
  const addBotDialogRef = useRef<AddBotDialogHandle | null>(null);
  const [workspaceMutation, setWorkspaceMutation] = useState<
    "add" | "assign" | "relink" | "remove" | null
  >(null);
  const [workspaceRenameTarget, setWorkspaceRenameTarget] =
    useState<DesktopWorkspace | null>(null);
  const [workspaceRenameSaving, setWorkspaceRenameSaving] = useState(false);
  const [, setPinnedThreadsVersion] = useState(0);
  const [addWorkspaceDialog, setAddWorkspaceDialog] = useState<{
    source: "new-thread" | "task";
    initialPath?: string;
    resolve?: (workspace: DesktopWorkspace | null) => void;
  } | null>(null);
  const [workspaceMenuOpenPath, setWorkspaceMenuOpenPath] = useState<string | null>(
    null,
  );
  // The aggregate live-stream map remains an imperative event-path shadow;
  // React reads selected/side transport state from their thread snapshots.
  const [pendingAutomationRunsByThread, setPendingAutomationRunsByThread] =
    useState<Record<string, PendingAutomationRun>>({});
  const selectedThreadIdRef = useRef<string | null>(null);
  const selectedThreadGenerationRef = useRef(0);
  const selectThreadRequestSequenceRef = useRef(0);
  const newThreadDraftActiveRef = useRef(false);
  const pendingWorkspaceSelectionRef = useRef<DraftWorkspaceSelection | null>(null);
  const pendingWorkspaceModeRef = useRef<DesktopWorkspaceMode>("local");
  const pendingBotIdRef = useRef<string | null>(null);
  const newThreadInitialDispatchLockRef = useRef(false);
  // #TASK-1633: a stable getter over the mirror's machine state. The
  // transcript lifecycle (batch 6b-2a) dispatches machine actions inside
  // the mirror, bypassing the old warming proxy — a plain ref shadow
  // would go stale between a lifecycle dispatch and the next React
  // commit. The getter always reads the mirror's live state (the 6a
  // reader pattern), so every event-path reader stays warm.
  const [messageStateRef] = useState(() => ({
    get current(): MessageMachineState {
      return gatewayMirror.getMachineState();
    },
  }));
  // 6b-2d: a stable getter over the mirror's live-stream map (the 1633
  // messageStateRef pattern) — lifecycle/orchestrator writes land in the
  // mirror, so a fed shadow would go stale between commits.
  const [liveStreamStateRef] = useState(() => ({
    get current(): Record<string, LiveStreamState> {
      return gatewayMirror.getLiveStreamMap();
    },
  }));
  const deferredQueueDrainByThreadRef = useRef<Record<string, boolean>>({});
  const queueDrainInFlightByThreadRef = useRef<Record<string, boolean>>({});
  const pendingAutomationRunsRef = useRef<Record<string, PendingAutomationRun>>(
    {},
  );
  const botBindingRequestSequenceRef = useRef(0);
  const lastRemoteStateWarningKeyRef = useRef<string | null>(null);
  const pendingThreadBottomSnapRef = useRef<string | null>(null);

  // Batch 5a: the memory dialog is a colocated feature root; the shell
  // only holds an imperative handle to open it.
  const memoryDialogRef = useRef<MemoryDialogHandle | null>(null);
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
    effectiveDefaultAgentId: desktopAgentCatalog.effectiveDefaultAgentId,
    getRouteVersion: () => desktopRouteStore.getSnapshot().version,
    navigateRoute: (route) => {
      desktopRouteStore.navigate(route, { replace: true });
    },
    syncAutomationRoute: (automationId) => {
      desktopRouteStore.syncRoute({ kind: "automation", automationId });
    },
    pendingThreadBottomSnapRef,
    selectedThreadId,
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
    savingSettings,
    setGatewaySettingsStatus,
    setLocalSettingsStatus,
    setSettingsDraft,
    settingsDraft,
  } = useSettingsController({
    contentView,
    desktopState,
    setConnection,
    setDesktopState,
    setError,
    settingsActiveTab,
  });
  const locale = useResolvedLocale(settingsDraft.languagePreference);
  const t = useMemo(() => createTranslator(locale), [locale]);

  // Batch 5a: toast ownership lives in ToastProvider (App.tsx); the
  // stable actions context makes pushToast identity-constant here.
  const { pushToast } = useToastActions();

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
    setDesktopAgentCatalog,
    setDesktopState,
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
    // mirror; the mirror is the only writer. Event-path readers reach the
    // committed state through the messageStateRef getter (#TASK-1633) —
    // no shadow write needed.
    gatewayMirror.dispatchMachineAction(action);
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
  const desktopAgentMap = new Map(
    desktopAgents.map((agent) => [agent.agentId, agent] as const),
  );
  const threadAvatarCatalog = useMemo(
    () => buildThreadAvatarCatalog(desktopAgents),
    [desktopAgents],
  );
  const activeAgentId = activeThread?.agentId || null;
  const activeThreadInfo = selectedThreadId
    ? activeThreadMirror?.threadInfo || null
    : null;
  const activeThreadInfoLoaded = selectedThreadId
    ? Boolean(activeThreadMirror?.transcriptLoaded)
    : false;
  const activeThreadProviderType = selectedThreadId
    ? inferProviderTypeForThread(
        selectedThreadId,
        activeThreadInfo,
        desktopState,
        desktopAgents,
      )
    : null;
  const activeThreadProviderModels = activeThreadProviderType
    ? providerModelsByType[activeThreadProviderType] || null
    : null;
  const pendingAgent = pendingAgentId
    ? desktopAgentMap.get(pendingAgentId) || null
    : null;
  const pendingAgentProviderType = pendingAgent?.providerType || null;
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

  const composerAgentOptions = useMemo(
    () => buildAgentOptions(desktopAgents),
    [desktopAgents],
  );
  const addBotAgentTargets = useMemo(() => {
    return buildAgentTargetOptions(desktopAgents);
  }, [desktopAgents]);
  const pendingAgentAvailable = canUseAgentForNewBinding(pendingAgent);
  const newDraftHasNoEnabledAgent = isNewDraftBindingBlocked(
    hasNewThreadDraft,
    pendingAgent,
  );
  useEffect(() => {
    if (!hasNewThreadDraft || pendingAgentId !== null) {
      return;
    }
    const effectiveAgentId = suggestedAgentId(desktopAgentCatalog);
    if (!effectiveAgentId) {
      return;
    }
    setPendingAgentId(effectiveAgentId);
    syncDraftRoute({ agentId: effectiveAgentId });
  }, [
    desktopAgentCatalog.effectiveDefaultAgentId,
    hasNewThreadDraft,
    pendingAgentId,
  ]);
  const pendingAgentLabel =
    pendingAgent?.displayName?.trim() ||
    pendingAgentId ||
    null;
  const activeAgentLabel =
    activeAgent?.displayName ||
    activeThread?.agentId ||
    null;
  const composerProviderType: DesktopApiProviderType = selectedThreadId
    ? activeAgent?.providerType || "claude_code"
    : pendingAgent?.providerType || "claude_code";
  const composerAgentLabel = selectedThreadId
    ? activeAgentLabel
    : pendingAgentAvailable
      ? pendingAgentLabel
      : t("No enabled agents");
  const gatewayIndicator = computeGatewayIndicator({
    status: connection,
    failureCount: gatewayFailureCount,
    recovering: hasGatewayRecoveryActivity(),
    reason: gatewayStatusHint || connection?.error || null,
  });
  const selectedWorkspaceEntry = selectedWorkspace(
    desktopState,
    desktopState?.selectedWorkspacePath || null,
  );
  const activeThreadWorkspace = workspaceForThread(
    desktopState,
    selectedThreadId,
  );
  const pendingWorkspacePath =
    pendingWorkspaceSelection?.kind === "path"
      ? pendingWorkspaceSelection.path
      : null;
  const pendingWorkspaceEntry = selectedWorkspace(
    desktopState,
    pendingWorkspacePath,
  );
  const rawActiveMessages = activeThreadMirror?.messages ||
    EMPTY_UI_TRANSCRIPT_MESSAGES;
  const activeMessages = useMemo(
    () => [...rawActiveMessages],
    [rawActiveMessages],
  );
  const activeHistoryPagination = activeThreadMirror?.historyPagination || null;
  const secondaryConversationRailRequested =
    contentView === "thread" &&
    Boolean(botConversationGroupId || recentThreadsRailOpen);
  const {
    compactSidebarViewport,
    currentConversationWidth,
    conversationRailPresented,
    dispatchLayoutOccupancyEvent,
    handleRailResizeStart,
    handleSidebarResizeStart,
    handleSideToolsResizeKeyDown,
    handleSideToolsResizeStart,
    layoutRootRef,
    railResizing,
    persistSidebarDesiredOpen,
    sidebarCollapsed,
    sidebarDesiredOpen,
    sidebarResizing,
    sideToolsPanelWidth,
    sideToolsEffectiveVisible,
    sideToolsPresented,
    sideToolsResizing,
    taskTreeDocked,
    toggleSidebarCollapsed: toggleSidebarCollapsedLegacy,
  } = useLayoutResizeController({
    contentView,
    inspectorOpen,
    openCapsuleTabs,
    secondaryRailOpen: secondaryConversationRailRequested,
    windowLayoutBootstrap: initialWindowLayout,
  });
  const initialLegacyLayoutIntent: LegacyLayoutIntentState = {
    globalSidebarOpen: sidebarDesiredOpen,
    conversationRail: secondaryConversationRailRequested
      ? conversationRailIntentFromLegacyState({
          botConversationGroupId,
          recentThreadsRailOpen,
        })
      : { kind: "closed" },
    inspectorOpen,
    openCapsuleTabs,
  };
  const desiredLayoutIntentRef = useRef(initialLegacyLayoutIntent);
  const appliedLayoutIntentRef = useRef(initialLegacyLayoutIntent);
  const layoutOccupancyEventLogRef = useRef(
    createLayoutOccupancyEventLog(
      layoutOccupancySources(initialLegacyLayoutIntent),
    ),
  );
  const applyLegacyLayoutIntentState = useCallback(
    (nextApplied: LegacyLayoutIntentState) => {
      appliedLayoutIntentRef.current = nextApplied;
      setOpenCapsuleTabsLegacy(nextApplied.openCapsuleTabs);
      setInspectorOpenLegacy(nextApplied.inspectorOpen);
      setRecentThreadsRailOpenLegacy(
        nextApplied.conversationRail.kind === "recent",
      );
      setBotConversationGroupIdLegacy(
        nextApplied.conversationRail.kind === "bot"
          ? nextApplied.conversationRail.groupId
          : null,
      );
    },
    [],
  );

  // Every horizontal panel writer enters here. The desired update is logged
  // synchronously as one full vector and fed into the active policy store.
  // updateApplied preserves the component state used to render panel content.
  const commitLegacyLayoutIntent = useCallback(
    (
      cause: LayoutIntentCause,
      updateDesired: LegacyLayoutIntentUpdate,
      updateApplied: LegacyLayoutIntentUpdate = updateDesired,
    ) => {
      const nextDesired = updateDesired(desiredLayoutIntentRef.current);
      desiredLayoutIntentRef.current = nextDesired;
      const appendResult = appendLayoutOccupancyIntent(
        layoutOccupancyEventLogRef.current,
        layoutOccupancySources(nextDesired),
        cause,
      );
      layoutOccupancyEventLogRef.current = appendResult.log;
      if (appendResult.event) {
        dispatchLayoutOccupancyEvent(appendResult.event);
      }

      const currentApplied = appliedLayoutIntentRef.current;
      const appliedCandidate = updateApplied(currentApplied);
      const nextApplied = {
        ...appliedCandidate,
        conversationRail: deferConversationRailUnmount(
          currentApplied.conversationRail,
          appliedCandidate.conversationRail,
        ),
      };
      applyLegacyLayoutIntentState(nextApplied);
    },
    [applyLegacyLayoutIntentState, dispatchLayoutOccupancyEvent],
  );
  useEffect(() => {
    const currentApplied = appliedLayoutIntentRef.current;
    const settledRail = settleDeferredConversationRailUnmount(
      currentApplied.conversationRail,
      desiredLayoutIntentRef.current.conversationRail,
      conversationRailPresented,
    );
    if (settledRail === currentApplied.conversationRail) {
      return;
    }
    applyLegacyLayoutIntentState({
      ...currentApplied,
      conversationRail: settledRail,
    });
  }, [applyLegacyLayoutIntentState, conversationRailPresented]);
  const toggleSidebarCollapsed = useCallback(() => {
    if (
      compactSidebarViewport &&
      (window.garyxDesktop.horizontalLayoutPolicy === "legacy" ||
        (sidebarCollapsed && sidebarDesiredOpen))
    ) {
      toggleSidebarCollapsedLegacy();
      return;
    }
    const nextOpen = !sidebarDesiredOpen;
    commitLegacyLayoutIntent("user-panel", (current) => ({
      ...current,
      globalSidebarOpen: nextOpen,
    }));
    persistSidebarDesiredOpen(nextOpen);
  }, [
    commitLegacyLayoutIntent,
    compactSidebarViewport,
    persistSidebarDesiredOpen,
    sidebarCollapsed,
    sidebarDesiredOpen,
    toggleSidebarCollapsedLegacy,
  ]);
  // Batch 5b scroll colocation: the DOM-bound effects/scheduler live in
  // ThreadPage's useThreadTranscriptScroll; the shell keeps the scroll
  // INTENT bundle (it must survive viewport unmounts — automations pre-arm
  // snaps from other views, and the dispatch/lifecycle orchestration
  // requests snaps regardless of the active view) plus the snap API those
  // writers call.
  const messagesRef = useRef<HTMLDivElement | null>(null);
  const pendingMessagesPrependAnchorRef = useRef<{
    threadId: string;
    scrollHeight: number;
    scrollTop: number;
  } | null>(null);
  const forceMessagesBottomSnapRef = useRef(false);
  const shouldStickMessagesToBottomRef = useRef(true);
  const lastRenderedMessageThreadRef = useRef<string | null>(null);
  const lastRenderedMessageCountRef = useRef(0);
  const lastRenderedMessageTailSignatureRef = useRef("0");
  const [transcriptScrollIntent] = useState<TranscriptScrollIntent>(() => ({
    pendingThreadBottomSnapRef,
    forceMessagesBottomSnapRef,
    shouldStickMessagesToBottomRef,
    pendingMessagesPrependAnchorRef,
    lastRenderedMessageThreadRef,
    lastRenderedMessageCountRef,
    lastRenderedMessageTailSignatureRef,
    selectedThreadIdRef,
  }));

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

  function requestSelectedThreadMessagesBottomSnap(
    threadId: string | null | undefined,
    forceStick = false,
  ) {
    if (!threadId || threadId !== selectedThreadIdRef.current) {
      return;
    }
    requestMessagesBottomSnap(threadId, forceStick);
  }

  useEffect(() => {
    if (activeThreadMessageKey == null) {
      pendingThreadBottomSnapRef.current = null;
      forceMessagesBottomSnapRef.current = false;
      return;
    }
    requestMessagesBottomSnap(activeThreadMessageKey, true);
  }, [activeThreadMessageKey]);
  useEffect(() => {
    selectedThreadIdRef.current = selectedThreadId;
    selectedThreadGenerationRef.current += 1;
  }, [selectedThreadId]);
  // Batch 6b-2d: useTranscriptController dissolved into the mirror. What
  // remains here is wiring — mirror-backed readers, thin delegates the
  // side-chat/dispatch controllers still take as args (until their own
  // colocation cuts), and the three transport React effects.
  function intentForId(intentId: string): MessageIntent | null {
    return messageStateRef.current.intentsById[intentId] || null;
  }
  function setThreadRuntimeState(
    threadId: string,
    runtimeState: ThreadRuntimeState,
    options?: { activeIntentId?: string; remoteRunId?: string; error?: string },
  ) {
    gatewayMirror.setThreadRuntimeState(threadId, runtimeState, options);
  }
  function hasPendingHistoryIntents(threadId: string): boolean {
    return gatewayMirror.hasPendingHistoryIntents(threadId);
  }
  function updateLiveStreamState(
    threadId: string,
    updater: (current: LiveStreamState | null) => LiveStreamState | null,
  ): LiveStreamState | null {
    return gatewayMirror.updateThreadLiveStream(threadId, updater);
  }
  function replaceLiveStreamThreadId(fromThreadId: string, toThreadId: string) {
    gatewayMirror.replaceLiveStreamThreadId(fromThreadId, toThreadId);
  }
  function clearLiveStreamState(threadId: string) {
    updateLiveStreamState(threadId, () => null);
  }
  function updateMessagesByThread(
    updater: (current: MessageMap) => MessageMap,
  ): MessageMap {
    return gatewayMirror.updateMessagesByThread(updater);
  }
  function applyCanonicalTranscript(
    threadId: string,
    transcript: ThreadTranscript,
    options?: { syncRunState?: boolean },
  ) {
    gatewayMirror.acceptAuthoritativeTranscript(threadId, transcript, options);
  }
  function applyRemoteTranscript(
    threadId: string,
    transcript: ThreadTranscript,
    options?: {
      persist?: boolean;
      syncRunState?: boolean;
      mirrorAlreadyApplied?: boolean;
    },
  ) {
    gatewayMirror.acceptRemoteTranscript(threadId, transcript, options);
  }
  function forceReleaseThreadRuntime(threadId: string) {
    gatewayMirror.forceReleaseThreadRuntime(threadId);
  }
  async function startCommittedThreadStream(
    threadId: string,
    transcript: ThreadTranscript,
    consumerId: string,
  ): Promise<void> {
    await gatewayMirror.startCommittedThreadStream(
      threadId,
      transcript,
      consumerId,
    );
  }

  useEffect(() => {
    const listener = (event: DesktopChatStreamEvent) => {
      // The lifecycle owns the whole pass — mirror ingest first (one
      // atomic commit), then the machine/run-state/error side effects.
      gatewayMirror.notifyStreamEvent(event);
    };
    window.garyxDesktop.subscribeChatStream(listener);
    return () => {
      window.garyxDesktop.unsubscribeChatStream(listener);
    };
  }, []);

  useEffect(() => {
    const flushTranscriptCaches = () => {
      gatewayMirror.flushAllTranscriptPersistence();
    };
    window.addEventListener("pagehide", flushTranscriptCaches);
    return () => {
      window.removeEventListener("pagehide", flushTranscriptCaches);
    };
  }, []);

  useEffect(() => {
    if (!selectedThreadId || !desktopState) {
      return;
    }

    void gatewayMirror.loadSelectedThreadTranscript(selectedThreadId);

    return () => {
      gatewayMirror.cancelSelectedThreadLoad(selectedThreadId);
    };
  }, [Boolean(desktopState), selectedThreadId]);

  // Dev-only mirror handle for CDP walkthroughs (the batch-2b parity probe
  // was deleted with the legacy dual-write in batch 6a).
  useEffect(() => {
    if (!import.meta.env.DEV) {
      return undefined;
    }
    const probeWindow = window as typeof window & {
      __garyxGatewayMirror?: GatewayMirror;
    };
    probeWindow.__garyxGatewayMirror = gatewayMirror;
    return () => {
      delete probeWindow.__garyxGatewayMirror;
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
  const activeRenderState = activeThreadMirror?.renderState || null;
  const activeQueue = useMemo(
    () =>
      selectQueueIntentIds(messageState, activeThreadMessageKey)
        .map((intentId) => messageState.intentsById[intentId])
        .filter((intent): intent is MessageIntent => Boolean(intent)),
    [messageState, activeThreadMessageKey],
  );
  const activeRuntime = selectThreadRuntime(
    messageState,
    activeThreadMessageKey,
  );
  const activeLiveStream = activeThreadMirror?.liveStream || null;
  const activePendingAckIntents = useMemo(
    () =>
      (activeLiveStream?.pendingAckIntentIds || [])
        .filter((intentId, index, intentIds) => {
          return intentIds.indexOf(intentId) === index;
        })
        .map((intentId) => messageState.intentsById[intentId])
        .filter((intent): intent is MessageIntent => {
          return Boolean(intent) && intent.state === "awaiting_provider_ack";
        }),
    [activeLiveStream?.pendingAckIntentIds, messageState.intentsById],
  );
  const representedActiveUserIntentIds = useMemo(
    () => representedUserIntentIds(activeMessages),
    [activeMessages],
  );
  const visiblePendingAckIntents = useMemo(
    () =>
      pendingAckIntentsNotRepresented(
        activePendingAckIntents,
        representedActiveUserIntentIds,
      ),
    [activePendingAckIntents, representedActiveUserIntentIds],
  );
  const activeThreadRunId =
    activeLiveStream?.runId || activeThread?.recentRunId || null;
  const activeRemotePendingInputs = selectedThreadId
    ? activeThreadMirror?.pendingRemoteInputs || []
    : [];
  const activePendingInputOriginRefs = useMemo(
    () =>
      pendingInputOriginRefsForThread(
        messageState.intentsById,
        activeThreadMessageKey,
      ),
    [messageState.intentsById, activeThreadMessageKey],
  );
  const visibleRemotePendingInputs = useMemo(
    () =>
      visibleRemotePendingInputsForThread({
        activeMessages,
        visiblePendingAckIntentCount: visiblePendingAckIntents.length,
        remotePendingInputs: activeRemotePendingInputs,
        pendingInputOriginRefs: activePendingInputOriginRefs,
      }),
    [
      activeMessages,
      visiblePendingAckIntents.length,
      activeRemotePendingInputs,
      activePendingInputOriginRefs,
    ],
  );
  const visibleRemoteAwaitingAckInputs = visibleRemotePendingInputs;
  const activePendingHistoryIntent = useMemo(
    () =>
      activeThreadMessageKey
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
        : false,
    [messageState.intentsById, activeThreadMessageKey],
  );
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
    commitLegacyLayoutIntent("user-route", (current) => ({
      ...current,
      inspectorOpen: true,
    }));
  }, [commitLegacyLayoutIntent]);
  const {
    activeWorkspaceDirectoryState,
    expandedWorkspaceDirectories,
    handleLocalWorkspaceFileLinkClick,
    handleWorkspaceFileEntryActivate,
    loadWorkspaceDirectory,
    selectedWorkspaceFile,
    selectedWorkspaceFileEntry,
    closeWorkspacePreview,
    setExpandedWorkspaceDirectories,
    uploadWorkspaceFilesToActiveWorkspace,
    workspaceDirectories,
    workspaceFilePreview,
    workspaceFilePreviewError,
    workspaceFilePreviewLoading,
    workspacePreviewModalOpen,
    workspacePreviewTitle,
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
  const composerEditingLocked = isDraftSendingThread;
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
  const activeThreadEndpoints = useMemo(
    () =>
      activeThread && !activeAutomationThread
        ? (desktopState?.endpoints || []).filter(
            (endpoint) => endpoint.threadId === activeThread.id,
          )
        : [],
    [activeThread, activeAutomationThread, desktopState?.endpoints],
  );
  const activeThreadBots = useMemo(
    () => boundBotsForThread(activeThreadEndpoints),
    [activeThreadEndpoints],
  );
  const mappedThreadBotId = useMemo(
    () =>
      activeThread
        ? (Object.entries(desktopState?.botMainThreads || {}).find(
            ([, threadId]) => threadId === activeThread.id,
          )?.[0] ?? null)
        : null,
    [activeThread, desktopState?.botMainThreads],
  );
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
  const activeThreadBot = useMemo(
    () =>
      activeThreadBotId
        ? (botGroups.find((group) => group.id === activeThreadBotId) ?? null)
        : null,
    [activeThreadBotId, botGroups],
  );

  const isSettingsView = contentView === "settings";
  const isBrowserView = contentView === "browser";
  const isBotsView = contentView === "bots";
  const isAutomationView = contentView === "automation";
  const isCapsulesView = contentView === "capsules";
  const isAgentsView = contentView === "agents";
  const isSkillsView = contentView === "skills";
  const isTasksView = contentView === "tasks";
  const shouldShowConversationRail = contentView === "thread";
  const visibleSelectedThreadId = shouldShowConversationRail ? selectedThreadId : null;
  const visibleThreadEntrySelectionSource = shouldShowConversationRail
    ? threadEntrySelectionSource
    : null;

  const botRootSelectedThreadId =
    visibleThreadEntrySelectionSource === "bot-root" ? visibleSelectedThreadId : null;
  const botConversationSelectedThreadId =
    visibleThreadEntrySelectionSource === "bot-conversation"
      ? visibleSelectedThreadId
      : null;
  const threadFavorites = useThreadFavorites({
    enabled: Boolean(desktopState?.entitiesGatewayUrl),
    gatewayScope: desktopState?.entitiesGatewayUrl || "",
    onError: setError,
  });
  const recentThreadFeeds = useRecentThreadFeeds({
    enabled: shouldShowConversationRail && recentThreadsRailOpen,
    // Main owns Gateway URL normalization and stamps every entity slice with
    // that canonical scope. Do not grow a second renderer normalizer or fall
    // back to the raw settings string (trailing-slash mismatch would make an
    // otherwise valid page look cross-scope).
    gatewayScope: desktopState?.entitiesGatewayUrl || "",
    runtimeEpoch: threadFavorites.state.runtimeEpoch,
    observeStoreResponse: threadFavorites.observeStoreResponse,
    sharedSummaries:
      desktopState?.threads || EMPTY_DESKTOP_THREAD_SUMMARIES,
  });
  const lifecycleStoreIncarnation = resolveLifecycleStoreIncarnation([
    threadFavorites.state.storeIncarnationId,
    recentThreadFeeds.state.feeds.all.storeIncarnationId,
    recentThreadFeeds.state.feeds.nonTask.storeIncarnationId,
  ]);
  const lifecycleRuntimeEpoch =
    threadFavorites.state.runtimeEpoch + recentThreadFeeds.state.runtimeEpoch;
  const lifecycleIdentityRef = useRef({
    gatewayScope: desktopState?.entitiesGatewayUrl || "",
    runtimeEpoch: lifecycleRuntimeEpoch,
  });
  lifecycleIdentityRef.current = {
    gatewayScope: desktopState?.entitiesGatewayUrl || "",
    runtimeEpoch: lifecycleRuntimeEpoch,
  };
  const favoriteThreads = useMemo(() => {
    return presentedFavoriteRows(
      threadFavorites.state,
      desktopState?.threads || EMPTY_DESKTOP_THREAD_SUMMARIES,
    ).filter(
      (thread) => !recentThreadFeeds.state.removedThreadIds[thread.id],
    );
  }, [
    desktopState?.threads,
    recentThreadFeeds.state.removedThreadIds,
    threadFavorites.state,
  ]);
  const favoritesFeed = useMemo<RecentThreadFeedState>(
    () => ({
      orderedThreadIds: favoriteThreads.map((thread) => thread.id),
      isPrimed: threadFavorites.state.rawRevision !== null,
      isRefreshingHead: Boolean(threadFavorites.state.activeSnapshotTicket),
      isLoadingMore: false,
      headFailure: threadFavorites.state.snapshotFailure,
      loadGate: "exhausted",
      nextCursor: null,
      epoch: threadFavorites.state.runtimeEpoch,
      localMutationSequence: 0,
      loadMoreFailureRevision: 0,
      activeRefreshRequestId: null,
      activeLoadMoreRequestId: null,
      refreshAfterMutation: false,
      loadMoreAfterMutation: false,
      storeIncarnationId: threadFavorites.state.storeIncarnationId,
      serverBootId: threadFavorites.state.favoritesServerBootId,
      refreshCycle: 0,
      forceReplacementPending: false,
      forceReplacementGeneration: 0,
      trailingDirty: threadFavorites.state.snapshotTrailingDirty,
    }),
    [favoriteThreads, threadFavorites.state],
  );
  const showingFavoriteThreads =
    recentThreadFeeds.state.selectedFilter === "favorites";
  const visibleRecentThreads = showingFavoriteThreads
    ? favoriteThreads
    : recentThreadFeeds.selectedThreads;
  const recentThreadRows = useMemo(
    () =>
      // Ordering and membership come from the selected server-owned unit:
      // keyset Recent for All/Chats, atomic snapshot for Favorites.
      visibleRecentThreads.map((thread) => ({
        thread,
        isActive:
          visibleThreadEntrySelectionSource === "recent" &&
          visibleSelectedThreadId === thread.id,
        isBusy: threadRunStateIsRunning(thread),
      })),
    [
      visibleRecentThreads,
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
  async function setThreadPinned(threadId: string, pinned: boolean) {
    const normalizedId = threadId.trim();
    if (!normalizedId) {
      return;
    }
    recentThreadFeeds.noteLocalMutation();
    const rollbackOrder = pinnedOrderIngress.presentedOrder;
    const optimisticOrder = pinnedOrderIngress.commitLocalMembership(
      normalizedId,
      pinned,
    );
    const membershipEpoch = pinnedOrderIngress.currentEpoch;
    setDesktopState((current) =>
      current
        ? {
            ...current,
            pinnedThreadIds: optimisticOrder,
          }
        : current,
    );
    try {
      const nextState = await requestDesktopState(() =>
        window.garyxDesktop.setThreadPinned({
          threadId: normalizedId,
          pinned,
        }),
      );
      setDesktopState(nextState);
      setPinnedThreadsVersion((version) => version + 1);
      void window.garyxDesktop
        .getThreadPinOrderSnapshot()
        .then(setPinnedOrderMainSnapshot)
        .catch(() => undefined);
    } catch (error) {
      if (pinnedOrderIngress.currentEpoch === membershipEpoch) {
        const restored = pinnedOrderIngress.rollbackLocalMembership(rollbackOrder);
        setDesktopState((current) =>
          current ? { ...current, pinnedThreadIds: restored } : current,
        );
      }
      setError(
        error instanceof Error
          ? error.message
          : pinned
            ? t("Failed to pin thread")
            : t("Failed to unpin thread"),
      );
      void refreshDesktopState().catch(() => null);
      void window.garyxDesktop
        .getThreadPinOrderSnapshot()
        .then(setPinnedOrderMainSnapshot)
        .catch(() => undefined);
    }
  }

  function beginPinnedThreadDrag() {
    pinnedOrderIngress.beginDrag();
  }

  function cancelPinnedThreadDrag() {
    const acceptedOrder = pinnedOrderIngress.cancelDrag();
    setDesktopState((current) =>
      current
        ? {
            ...current,
            pinnedThreadIds: acceptedOrder,
          }
        : current,
    );
  }

  function reorderPinnedThreads(threadIds: string[]) {
    const previousEpoch = pinnedOrderIngress.currentEpoch;
    const optimisticOrder = pinnedOrderIngress.commitDragOrder(threadIds);
    setDesktopState((current) =>
      current
        ? {
            ...current,
            pinnedThreadIds: optimisticOrder,
          }
        : current,
    );
    if (pinnedOrderIngress.currentEpoch === previousEpoch) {
      return;
    }

    recentThreadFeeds.noteLocalMutation();
    void requestDesktopState(() =>
      window.garyxDesktop.setThreadPinOrder({ threadIds: optimisticOrder }),
    )
      .then((nextState) => {
        setDesktopState(nextState);
        setPinnedThreadsVersion((version) => version + 1);
        void window.garyxDesktop
          .getThreadPinOrderSnapshot()
          .then(setPinnedOrderMainSnapshot)
          .catch(() => undefined);
      })
      .catch(() => {
        // R5: keep the optimistic order. Main owns retry classification and
        // the durable outbox; a refresh can only merge membership beneath it.
        void refreshDesktopState().catch(() => null);
        void window.garyxDesktop
          .getThreadPinOrderSnapshot()
          .then(setPinnedOrderMainSnapshot)
          .catch(() => undefined);
      });
  }

  function togglePinnedThread(threadId: string) {
    const pinned = (desktopState?.pinnedThreadIds || []).includes(threadId);
    void setThreadPinned(threadId, !pinned);
  }
  useEffect(() => {
    if (shouldShowConversationRail) {
      return;
    }
    commitLegacyLayoutIntent("system-cleanup", (current) => ({
      ...current,
      conversationRail: { kind: "closed" },
    }));
  }, [commitLegacyLayoutIntent, shouldShowConversationRail]);
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
      commitLegacyLayoutIntent("system-cleanup", (current) => ({
        ...current,
        conversationRail: { kind: "closed" },
      }));
    }
  }, [botConversationGroupId, commitLegacyLayoutIntent, visibleBotGroups]);
  const activeBotConversationGroup = useMemo(() => {
    if (!botConversationGroupId) {
      return null;
    }
    return (
      visibleBotGroups.find(
        (group) =>
          group.id === botConversationGroupId &&
          (group.conversationNodes || []).length > 0,
      ) || null
    );
  }, [botConversationGroupId, visibleBotGroups]);
  const appShellClassName = [
    "app-shell",
    sidebarCollapsed ? "sidebar-collapsed" : null,
    conversationRailPresented
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
    isSkillsView;
  const canEditThreadTitle = Boolean(
    activeThread &&
    !activeAutomationThread &&
    !isAutomationView &&
    !isCapsulesView &&
    !isSkillsView &&
    !isTasksView &&
    !isBotsView &&
    !isAgentsView,
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
  // Batch 5b-7b: the side-chat panel colocates derivations/behavior/JSX
  // (components/SideChatPanel.tsx). The shell keeps what must outlive the
  // dock: the session store (7a), the sessionStorage restore + active-
  // source effects, the always-on transcript-load effect, the deferred
  // queue-drain effect, and the two out-of-panel commands (chat auto-open
  // and the Tasks-tab open, per the design's command ruling).
  const sideChatSessionsSnapshot = useSyncExternalStore(
    sideChatSessions.subscribe,
    sideChatSessions.getSnapshot,
  );
  const sideChatSourceThreadId = activeThread?.id?.trim() || null;
  const sideChatThreadId = sideChatSourceThreadId
    ? sideChatSessionsSnapshot.threadBySource[sideChatSourceThreadId] || null
    : null;
  // Side-chat stream/queue orchestration remains shell-owned while the dock
  // is hidden, so this is the shell's second and only other thread listener.
  const shellSideChatMirror = useGatewayThreadMirror(
    gatewayMirror,
    sideChatThreadId,
  );
  const sideChatMessagesRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    // sessionStorage read-through for a source with no in-memory binding.
    if (sideChatSourceThreadId) {
      sideChatSessions.restorePersisted(sideChatSourceThreadId);
    }
  }, [sideChatSourceThreadId]);

  useEffect(() => {
    // The active-source flip re-derives the sideChatThreadIdRef shadow
    // inside the store.
    sideChatSessions.setActiveSource(sideChatSourceThreadId);
  }, [sideChatSourceThreadId]);

  // Load the side thread transcript once per side thread (after state
  // hydration). Depending on `desktopState` identity here is unsafe: applying
  // a transcript can rewrite `desktopState.sessions`, which would re-fire
  // this effect in a fetch loop. Steady-state sync comes from the per-thread
  // committed stream started after the initial committed cursor is known.
  // Always-on by design: a bound side thread keeps its committed stream
  // alive even while the dock is hidden (review-confirmed premise).
  const desktopStateHydrated = Boolean(desktopState);
  useEffect(() => {
    if (!sideChatThreadId || !desktopStateHydrated) {
      return;
    }

    let cancelled = false;
    let latestTranscript: ThreadTranscript | null = null;
    const consumerId = sideChatSessions.streamConsumerId(sideChatThreadId);
    void loadThreadHistory({
      api: getDesktopApi(),
      threadId: sideChatThreadId,
      onBeforeLoad: (threadId) => {
        if (!gatewayMirror.getThreadSnapshot(threadId).messages.length) {
          scrollMessagesToLatest(sideChatMessagesRef.current);
        }
      },
      onTranscript: (threadId, transcript) => {
        if (cancelled) {
          return;
        }
        latestTranscript = transcript;
        applyRemoteTranscript(threadId, transcript);
      },
      onAutomationResponseDetected: (threadId) => {
        setPendingAutomationRun(threadId, null);
      },
      hasAutomationResponse: transcriptHasAutomationResponse,
      setHistoryLoading: (loading) => sideChatSessions.setHistoryLoading(loading),
      setError,
    }).then(() => {
      if (cancelled || !latestTranscript) {
        return;
      }
      void startCommittedThreadStream(
        sideChatThreadId,
        latestTranscript,
        consumerId,
      );
    });

    return () => {
      cancelled = true;
      void gatewayMirror.stopCommittedThreadStream({
        threadId: sideChatThreadId,
        consumerId,
      });
    };
  }, [desktopStateHydrated, sideChatThreadId]);

  // Deferred queue drain for the side thread (always-on: legacy drained
  // with the dock hidden because the controller stayed mounted). The
  // busy inputs are rebuilt here verbatim from the same mirror-backed
  // sources the panel derives from, so both read one truth.
  const shellSideChatMessages =
    (shellSideChatMirror?.messages as UiTranscriptMessage[] | undefined) ||
    EMPTY_UI_TRANSCRIPT_MESSAGES;
  const shellSideChatRenderState = shellSideChatMirror?.renderState || null;
  const shellSideChatQueueLength = sideChatThreadId
    ? selectQueueIntentIds(messageState, sideChatThreadId).length
    : 0;
  const shellSideChatRuntime = selectThreadRuntime(
    messageState,
    sideChatThreadId,
  );
  const shellSideChatLiveStream = shellSideChatMirror?.liveStream || null;
  const shellSideChatPendingAckIntents = useMemo(
    () =>
      (shellSideChatLiveStream?.pendingAckIntentIds || [])
        .map((intentId) => messageState.intentsById[intentId])
        .filter((intent): intent is MessageIntent => Boolean(intent)),
    [shellSideChatLiveStream?.pendingAckIntentIds, messageState.intentsById],
  );
  const representedShellSideChatIntentIds = useMemo(
    () => representedUserIntentIds(shellSideChatMessages),
    [shellSideChatMessages],
  );
  const shellSideChatVisiblePendingAckIntents = useMemo(
    () =>
      pendingAckIntentsNotRepresented(
        shellSideChatPendingAckIntents,
        representedShellSideChatIntentIds,
      ),
    [shellSideChatPendingAckIntents, representedShellSideChatIntentIds],
  );
  const shellSideChatRemotePendingInputs = sideChatThreadId
    ? shellSideChatMirror?.pendingRemoteInputs || []
    : [];
  const shellSideChatPendingInputOriginRefs = useMemo(
    () =>
      pendingInputOriginRefsForThread(
        messageState.intentsById,
        sideChatThreadId,
      ),
    [messageState.intentsById, sideChatThreadId],
  );
  const shellSideChatVisibleRemotePendingInputs = useMemo(
    () =>
      visibleRemotePendingInputsForThread({
        activeMessages: shellSideChatMessages,
        visiblePendingAckIntentCount:
          shellSideChatVisiblePendingAckIntents.length,
        remotePendingInputs: shellSideChatRemotePendingInputs,
        pendingInputOriginRefs: shellSideChatPendingInputOriginRefs,
      }),
    [
      shellSideChatMessages,
      shellSideChatVisiblePendingAckIntents.length,
      shellSideChatRemotePendingInputs,
      shellSideChatPendingInputOriginRefs,
    ],
  );
  const shellSideChatPendingHistoryIntent = useMemo(
    () =>
      sideChatThreadId
        ? Object.values(messageState.intentsById).some((intent) => {
            return (
              intent.threadId === sideChatThreadId &&
              [
                "dispatching",
                "remote_accepted",
                "awaiting_provider_ack",
                "awaiting_response",
                "awaiting_history",
              ].includes(intent.state)
            );
          })
        : false,
    [messageState.intentsById, sideChatThreadId],
  );
  const shellSideChatRuntimeBusy = Boolean(
    shellSideChatRuntime && isRuntimeBusy(shellSideChatRuntime.state),
  );
  const shellSideChatActivity = deriveThreadActivityModel({
    messages: shellSideChatMessages,
    runtimeBusy: shellSideChatRuntimeBusy,
    pendingAckIntentCount: shellSideChatPendingAckIntents.length,
    remoteAwaitingAckInputCount:
      shellSideChatVisibleRemotePendingInputs.length,
    pendingHistoryIntent: shellSideChatPendingHistoryIntent,
    renderTailActivity: shellSideChatRenderState?.tailActivity ?? null,
    renderActiveToolGroupId:
      shellSideChatRenderState?.activeToolGroupId ?? null,
  });
  const { isActiveSendingThread: shellSideChatIsSendingThread } =
    deriveThreadComposerControlModel({
      hasThread: Boolean(sideChatThreadId),
      runtimeBusy: shellSideChatRuntimeBusy,
      showPendingAckLoading: shellSideChatActivity.showPendingAckLoading,
      renderTailActivity: shellSideChatRenderState?.tailActivity ?? null,
      renderActiveToolGroupId:
        shellSideChatRenderState?.activeToolGroupId ?? null,
    });
  useEffect(() => {
    if (!sideChatThreadId) {
      return;
    }
    if (shellSideChatQueueLength === 0) {
      delete deferredQueueDrainByThreadRef.current[sideChatThreadId];
      delete queueDrainInFlightByThreadRef.current[sideChatThreadId];
      return;
    }
    if (
      shellSideChatIsSendingThread ||
      !deferredQueueDrainByThreadRef.current[sideChatThreadId] ||
      queueDrainInFlightByThreadRef.current[sideChatThreadId]
    ) {
      return;
    }

    deferredQueueDrainByThreadRef.current[sideChatThreadId] = false;
    queueDrainInFlightByThreadRef.current[sideChatThreadId] = true;
    void runQueuedBatch(sideChatThreadId).finally(() => {
      delete queueDrainInFlightByThreadRef.current[sideChatThreadId];
    });
  }, [shellSideChatIsSendingThread, shellSideChatQueueLength, sideChatThreadId]);

  // Out-of-panel side-chat command (design ruling, #TASK-1658) may run while
  // the panel is not mounted.
  function sideChatOpsContext(): SideChatOpsContext {
    return {
      sessions: sideChatSessions,
      mirror: gatewayMirror,
      sourceThreadId: sideChatSourceThreadId,
      activeThread,
      threadSummaryById,
      setDesktopState,
      setError,
    };
  }
  // Batch 3c-2: the dispatch orchestration (send/steer/interrupt/queue
  // drain) lives in the mirror; its deps are refreshed on every commit
  // (the streamEventHandlerRef pattern) so orchestration entry points
  // destructure this render's values — the legacy closure capture.
  const dispatchOrchestratorDeps: DispatchOrchestratorDeps = {
    canSteerQueuedPrompt,
    checkConnection: () => window.garyxDesktop.checkConnection(),
    connection,
    desktopAgents,
    desktopState,
    getThreadHistory: (threadId) =>
      window.garyxDesktop.getThreadHistory(threadId),
    inferProviderTypeForThread,
    interruptThread: (threadId) =>
      window.garyxDesktop.interruptThread(threadId),
    openChatStream: (input) => window.garyxDesktop.openChatStream(input),
    recordGatewayStatusObservation,
    requestMessagesBottomSnap,
    scheduleHistoryRefresh,
    sendStreamingInput: (input) =>
      window.garyxDesktop.sendStreamingInput(input),
    setConnection,
    setDesktopState,
    setError,
    settingsDraft,
    sideChatThreadIdsRef: sideChatSessions.sideChatThreadIdsRef,
  };
  // Boot instrumentation (perf round 2026-07): cheap performance.marks so
  // packaged boots decompose without an attached profiler. Read them via
  // performance.getEntriesByType('mark').
  useEffect(() => {
    performance.mark("garyx:shell-mounted");
  }, []);
  const bootHydratedMarkedRef = useRef(false);
  useEffect(() => {
    if (bootHydratedMarkedRef.current || !desktopState) {
      return;
    }
    bootHydratedMarkedRef.current = true;
    performance.mark("garyx:state-hydrated");
    requestAnimationFrame(() => {
      performance.mark("garyx:first-interactive-frame");
    });
  }, [desktopState]);
  useEffect(() => {
    gatewayMirror.setDispatchDeps(dispatchOrchestratorDeps);
  });
  // Batch 6b-2c: the transcript lifecycle's React seams, refreshed every
  // commit (the setDispatchDeps pattern). Fed here rather than inside
  // useTranscriptController because the side-chat stream identity comes
  // from the later useSideChatController call.
  useEffect(() => {
    gatewayMirror.setTranscriptLifecycleDeps({
      setDesktopState,
      syncThreadTitleDraft: (nextTitle: string) => {
        conversationTitleRef.current?.syncTitle(nextTitle);
      },
      requestSelectedThreadMessagesBottomSnap,
      selectedThreadIdRef,
      setError,
      setHistoryLoading,
      setPendingAutomationRun,
      recordGatewayStatusObservation,
      scheduleDesktopStateRefresh,
      scheduleHistoryRefresh,
      connection,
      settingsDraft,
      desktopState,
      refreshDesktopState,
      selectedThreadGenerationRef,
      lastRenderedMessageThreadRef,
      messagesRef,
      pendingMessagesPrependAnchorRef,
      sideChatThreadIdRef: sideChatSessions.sideChatThreadIdRef,
      sideChatStreamConsumerId: (threadId: string) =>
        sideChatSessions.streamConsumerId(threadId),
    });
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
    composerPendingUploads,
    composerResetKey,
    composerTextareaRef,
    handleAddBrowserAnnotationComment,
    handleComposerSubmit,
    handleInterrupt,
    handleRetryFailedMessage,
    handleSendPromptText,
    handleSteerQueuedPrompt,
    ignoreComposerSubmitUntilRef,
    isComposingRef,
    markIgnoreComposerSubmitWindow,
    removeComposerBrowserAnnotation,
    removeComposerFile,
    removeComposerImage,
    removeComposerPendingUpload,
    reorderQueuedIntent,
    requestComposerFocus,
    setComposerTextPresent,
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
    intentForId,
    interruptThread,
    isActiveSendingThread,
    isDraftSendingThread,
    messageStateRef,
    newThreadInitialDispatchLockRef,
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
  });
  const conversationContextText = isAutomationView
    ? `${desktopState?.automations.length || 0} scheduled runs`
    : isCapsulesView
      ? "Self-contained HTML capsules"
    : isSkillsView
      ? "Local and project skill registry"
    : isTasksView
        ? "Global task board"
      : isAgentsView
        ? "Reusable agents"
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
      memoryDialogRef.current?.open(memoryTarget);
      return;
    }
    handleLocalWorkspaceFileLinkClick(absolutePath);
  }, [
    automations,
    desktopAgents,
    handleLocalWorkspaceFileLinkClick,
  ]);

  function openSettingsView() {
    // The settings application branch runs handleSelectSettingsTab, whose
    // same-tab path refreshes non-local tab resources (superset of the
    // old inline refresh, adding the gateway auto-save flush).
    desktopRouteStore.navigate(
      { kind: "settings", tabId: settingsActiveTab },
      { replace: true },
    );
  }

  async function refreshAgentTargets() {
    const nextCatalog = await window.garyxDesktop
      .listCustomAgents()
      .catch(() => EMPTY_DESKTOP_AGENT_CATALOG);
    startTransition(() => {
      setDesktopAgentCatalog(nextCatalog);
    });
  }

  async function openAddBotDialog() {
    addBotDialogRef.current?.open();
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
    const nextState = await requestDesktopState(() =>
      window.garyxDesktop.addChannelAccount(input),
    );
    startTransition(() => {
      setDesktopState(nextState);
    });
    await loadGatewaySettings({ clearStatus: true });
    pushToast(t("Bot added."), "success");
  }

  // Batch 6b-2d: the openability gate lives in the mirror's transcript
  // lifecycle (with loadSelectedThreadTranscript it fulfills the parent
  // design's mirror.openThread contract; route semantics stay here).
  async function ensureThreadOpenable(threadId: string): Promise<boolean> {
    return gatewayMirror.ensureThreadOpenable(threadId);
  }

  async function openExistingThread(
    threadId: string,
    entrySource: ThreadEntrySelectionSource | null = null,
  ): Promise<boolean> {
    setError(null);
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
    // Opening a thread is a command like the draft entry (6c-2b): direct
    // callers (rows, panels, bot flows) land here without a route commit,
    // so sync the route now — the contentView selector flips through this
    // same commit. Equal-route no-op when the navigate application called
    // us; origin 'sync' means the route effect never re-applies.
    desktopRouteStore.syncRoute({ kind: "thread", threadId });
    return true;
  }

  // Thread openers with a selection source (bot-root, endpoint, recent…)
  // hand it through this mailbox; the bridge's openExistingThread wrapper
  // consumes it so the thread-route application tags the selection.
  const pendingThreadEntrySourceHintRef =
    useRef<ThreadEntrySelectionSource | null>(null);

  useRouteEffectBridge({
    clearComposerDraft,
    desktopState,
    desktopRouteStore,
    ensureThreadOpenable,
    handleResumeProviderSession,
    handleSelectAutomation,
    handleSelectSettingsTab,
    // The thread-route application consumes the entry-source mailbox so
    // navigations from bot roots / endpoints / recents tag the selection.
    openExistingThread: (threadId: string) => {
      const entrySource = pendingThreadEntrySourceHintRef.current;
      pendingThreadEntrySourceHintRef.current = null;
      return openExistingThread(threadId, entrySource);
    },
    enterNewThreadDraft,
    pushToast,
    requestComposerFocus,
    selectedThreadId,
    selectThreadRequestSequenceRef,
    setConnection,
    setNewThreadDraftActive,
    setPendingAgentId,
    setPendingBotId,
    setPendingWorkspaceMode,
    setPendingWorkspaceSelection,
    setSelectedThreadId,
  });

  useEffect(() => {
    newThreadDraftActiveRef.current = newThreadDraftActive;
  }, [newThreadDraftActive]);

  useEffect(() => {
    pendingWorkspaceSelectionRef.current = pendingWorkspaceSelection;
  }, [pendingWorkspaceSelection]);

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
      // The selected workspace was removed: the one sanctioned re-resolution
      // of a live draft. The route sync carries the new selection
      // (review #TASK-1627: raw pending writes must carry their sync).
      const fallback = resolveDefaultDraftWorkspace(
        visibleWorkspaceList(desktopState),
      );
      setPendingWorkspaceSelection(fallback);
      syncDraftRoute({
        workspacePath: routeWorkspaceFromDraftSelection(fallback),
      });
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
    if (selectedThreadId && pendingWorkspaceSelection) {
      setPendingWorkspaceSelection(null);
    }
  }, [pendingWorkspaceSelection, selectedThreadId]);

  // A draft that entered before the workspace catalog loaded (cold start,
  // startup route without a workspace param) resolves its default here —
  // still exactly once: the effect only fires while the selection is
  // unresolved, and a resolved draft never re-enters it.
  useEffect(() => {
    // Any visible draft composer resolves here — the explicit new-thread
    // draft AND the thread-home empty state (its composer is a draft too).
    const draftComposerVisible =
      contentView === "thread" && !selectedThreadId;
    // An empty catalog is a valid answer (→ explicit none), not a reason
    // to stay unresolved — but a FAILED catalog fetch is not an answer at
    // all: its fallback is an empty list plus a workspaces remote error,
    // and resolving on it would permanently pin `none` before the retry
    // brings the real list. Hydration success is what gates this.
    const workspacesSliceFailed = Boolean(
      desktopState?.remoteErrors?.some((error) => error.source === "workspaces"),
    );
    if (
      !draftComposerVisible ||
      pendingWorkspaceSelection !== null ||
      !desktopState ||
      workspacesSliceFailed
    ) {
      return;
    }
    const resolved = resolveDefaultDraftWorkspace(
      visibleWorkspaceList(desktopState),
    );
    setPendingWorkspaceSelection(resolved);
    syncDraftRoute({
      workspacePath: routeWorkspaceFromDraftSelection(resolved),
    });
  }, [
    contentView,
    desktopState,
    pendingWorkspaceSelection,
    selectedThreadId,
  ]);

  // Gateway switch = a new workspace universe (single-gateway scoping,
  // design §4.4). Close every transient workspace surface so nothing keeps
  // rendering rows or issuing requests against the previous gateway; late
  // responses from the old epoch are rejected by the DesktopState ingress
  // and per-request cancellation.
  // The epoch is a connection GENERATION, not the gateway URL: switching
  // A -> B -> A produces three distinct epochs, so a late result from the
  // first A connection can never be accepted after returning to A. Every
  // switch closes the transient workspace surfaces and resets mutation
  // busy state (a stale mutation's own cleanup is epoch-guarded and must
  // not touch the new epoch's state).
  const workspaceGatewayKey = desktopState?.entitiesGatewayUrl || "";
  const workspaceGatewayKeyRef = useRef(workspaceGatewayKey);
  const workspaceEpochCounterRef = useRef(0);
  const [workspaceEpoch, setWorkspaceEpoch] = useState("g0");
  const workspaceEpochRef = useRef(workspaceEpoch);
  useEffect(() => {
    if (workspaceGatewayKeyRef.current === workspaceGatewayKey) {
      return;
    }
    workspaceGatewayKeyRef.current = workspaceGatewayKey;
    workspaceEpochCounterRef.current += 1;
    const nextEpoch = `g${workspaceEpochCounterRef.current}`;
    workspaceEpochRef.current = nextEpoch;
    setWorkspaceEpoch(nextEpoch);
    setWorkspaceMenuOpenPath(null);
    setWorkspaceRenameTarget(null);
    setWorkspaceRenameSaving(false);
    setWorkspaceMutation(null);
    setAddWorkspaceDialog((current) => {
      current?.resolve?.(null);
      return null;
    });
    // Cached git statuses are keyed by path only; a different gateway's
    // filesystem must not answer for them.
    workspaceGitStatusCache.clear();
  }, [workspaceGatewayKey]);

  /** Late workspace-mutation results from a previous gateway epoch are
   *  complete no-ops: don't touch drafts, routes, or dialogs with them. */
  const isCurrentWorkspaceEpoch = useCallback(
    (epoch: string) => workspaceEpochRef.current === epoch,
    [],
  );

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

          // Fast hydration: the threads slice is a recent page (pinned ids
          // repaired by id). The full set follows below, off the paint path.
          const [nextState, nextStatus, nextAgentCatalog] =
            await Promise.all([
              requestDesktopState(() => window.garyxDesktop.getStateFast()),
              window.garyxDesktop.checkConnection(),
              window.garyxDesktop
                .listCustomAgents()
                .catch(() => EMPTY_DESKTOP_AGENT_CATALOG),
            ]);
          if (cancelled) {
            return;
          }

          state = nextState;
          pinnedOrderIngress.initializeFromState(nextState);
          const pinOrderSnapshot = await window.garyxDesktop
            .getThreadPinOrderSnapshot()
            .catch(() => null);
          if (cancelled) {
            return;
          }

          startTransition(() => {
            setDesktopState(nextState);
            if (pinOrderSnapshot) {
              setPinnedOrderMainSnapshot(pinOrderSnapshot);
            }
            setDesktopAgentCatalog(nextAgentCatalog);
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
          const automationId = startupRoute.automationId;
          try {
            hydratedState = await requestDesktopState(() =>
              window.garyxDesktop.selectAutomation({
                automationId,
              }),
            );
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
            // Batch 4b (intentional change #2): an unknown #/thread/<id>
            // stays selected and addressable — no silent fallback selection
            // that would rewrite the entered hash. The selected-thread
            // loader is the single error surface (its missing-thread gate
            // raises "Thread not found"); setting the error here too would
            // double the toast.
            setSelectedThreadId(startupRoute.threadId);
          }
        } else if (startupRoute.kind === "new-thread") {
          setNewThreadDraftActive(true);
          setSelectedThreadId(null);
          setPendingWorkspaceSelection(
            draftSelectionFromRouteWorkspace(startupRoute.workspacePath) ??
              resolveDefaultDraftWorkspace(
                visibleWorkspaceList(hydratedState),
              ),
          );
          setPendingWorkspaceMode("local");
          setPendingAgentId(
            startupRoute.agentId || null,
          );
        } else {
          setSelectedThreadId((current) =>
            isKnownThreadId(hydratedState, current)
              ? current
              : hydratedState.threads[0]?.id || null,
          );
        }
        // Follow-up full state: restores full-set semantics (workspace
        // groups, worktree exclusions, bot gates) shortly after first
        // paint. Failures are non-fatal — any later refreshDesktopState
        // delivers the full set too.
        void requestDesktopState(() => window.garyxDesktop.getState())
          .then((fullState) => {
            if (!cancelled) {
              startTransition(() => {
                setDesktopState(fullState);
              });
            }
          })
          .catch((fullStateError) => {
            console.debug("Full desktop state follow-up failed.", fullStateError);
          });
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

    // Batch 4b (intentional changes #1/#2): only converge when NOTHING is
    // selected (the thread-home default). A selected-but-unknown thread —
    // an externally entered #/thread/<id> or a side-chat/hidden thread —
    // stays selected and addressable instead of being silently rewritten
    // to threads[0] (the legacy quirk where a manual hash edit bounced
    // back to the previously selected thread).
    if (selectedThreadId) {
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
    // Capsule tabs are scoped to the current thread; drop them when the thread
    // or content view changes so a different thread's capsules never linger in
    // the dock (#TASK-1470).
    commitLegacyLayoutIntent("system-cleanup", (current) => ({
      ...current,
      openCapsuleTabs: [],
    }));
    setPendingActiveCapsuleId(null);
  }, [commitLegacyLayoutIntent, contentView, selectedThreadId]);

  useEffect(() => {
    if (!inspectorOpen) {
      return;
    }

    function handleKeydown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        commitLegacyLayoutIntent("system-cleanup", (current) => ({
          ...current,
          inspectorOpen: false,
        }));
      }
    }

    window.addEventListener("keydown", handleKeydown);
    return () => {
      window.removeEventListener("keydown", handleKeydown);
    };
  }, [commitLegacyLayoutIntent, inspectorOpen]);

  useEffect(() => {
    if (contentView !== "thread") {
      commitLegacyLayoutIntent("system-cleanup", (current) => ({
        ...current,
        inspectorOpen: false,
      }));
    }
  }, [commitLegacyLayoutIntent, contentView]);

  useEffect(() => {
    // Logs keep side tools valid for threads without a workspace. Only the
    // workspace-directory expansion waits for DesktopState and a real path.
    if (loading || !desktopState || !activeWorkspacePath) {
      return;
    }

    setExpandedWorkspaceDirectories((current) => ({
      ...current,
      [workspaceDirectoryKey(activeWorkspacePath, "")]: true,
    }));
  }, [activeWorkspacePath, desktopState, loading]);

  useEffect(() => {
    if (!workspacePreviewModalOpen || contentView !== "thread") {
      return;
    }
    commitLegacyLayoutIntent("user-route", (current) => ({
      ...current,
      inspectorOpen: true,
    }));
  }, [commitLegacyLayoutIntent, contentView, workspacePreviewModalOpen]);

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

  async function ensureSelectedThreadId(): Promise<string | null> {
    return ensureThread({
      api: getDesktopApi(),
      selectedThreadId,
      pendingWorkspaceSelection,
      pendingWorkspaceMode,
      pendingAgentId,
      pendingModel,
      pendingModelReasoningEffort,
      pendingModelServiceTier,
      setWorkspaceMutation,
      setDesktopState,
      // Draft promotion selects the created thread synchronously; sync its
      // route in the same step (6c-2c) so the hash follows once the fold
      // dies. sync origin — the route effect never re-opens it.
      setSelectedThreadId: (threadId) => {
        setSelectedThreadId(threadId);
        if (threadId) {
          desktopRouteStore.syncRoute({ kind: "thread", threadId });
        }
      },
      initializeThreadMessages: (threadId) => {
        updateMessagesByThread((current) => ({
          ...current,
          [threadId]: [],
        }));
      },
      setNewThreadDraftActive,
      setPendingWorkspaceSelection,
      setPendingWorkspaceMode,
      setPendingBotId,
      setPendingAgentId,
      setPendingModel,
      setPendingModelReasoningEffort,
      setPendingModelServiceTier,
      setError,
      onThreadCreated: recentThreadFeeds.upsertChat,
    });
  }

  async function handleResumeProviderSession(
    sessionId: string,
    providerHint?: DesktopSessionProviderHint | null,
  ): Promise<void> {
    const trimmedSessionId = sessionId.trim();
    if (!trimmedSessionId) {
      throw new Error("Paste a Claude or Codex session ID first.");
    }

    setError(null);
    try {
      const created = await requestDesktopStateResult(
        () => window.garyxDesktop.createThread({
          sdkSessionId: trimmedSessionId,
          sdkSessionProviderHint: providerHint || undefined,
        }),
        (response) => response.state,
      );
      setDesktopState(created.state);
      recentThreadFeeds.upsertChat(created.thread);
      // Selection + view flip is the thread-route application (6c-2a).
      desktopRouteStore.navigate(
        { kind: "thread", threadId: created.thread.id },
        { replace: true },
      );
      updateMessagesByThread((current) => ({
        ...current,
        [created.thread.id]: current[created.thread.id] || [],
      }));
      setPendingWorkspaceSelection(null);
      setPendingWorkspaceMode("local");
      setPendingBotId(null);
      setPendingAgentId(created.thread.agentId || providerHint || null);
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
      nextDesktopState = await requestDesktopState(() =>
        window.garyxDesktop.setBotBinding({
          threadId,
          botId: null,
        }),
      );
      for (const endpoint of currentEndpoints) {
        nextDesktopState = await requestDesktopState(() =>
          window.garyxDesktop.detachChannelEndpoint({
            endpointKey: endpoint.endpointKey,
          }),
        );
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
    nextDesktopState = await requestDesktopState(() =>
      window.garyxDesktop.setBotBinding({
        threadId,
        botId,
      }),
    );

    for (const endpoint of currentEndpoints) {
      if (endpoint.endpointKey === targetEndpoint?.endpointKey) {
        continue;
      }
      if (botGroupIdForEndpoint(endpoint) === botId) {
        continue;
      }
      nextDesktopState = await requestDesktopState(() =>
        window.garyxDesktop.detachChannelEndpoint({
          endpointKey: endpoint.endpointKey,
        }),
      );
    }

    if (
      targetEndpoint?.endpointKey &&
      targetGroup?.mainThreadId !== threadId &&
      targetEndpoint.threadId !== threadId
    ) {
      nextDesktopState = await requestDesktopState(() =>
        window.garyxDesktop.bindChannelEndpoint({
          endpointKey: targetEndpoint.endpointKey,
          threadId,
        }),
      );
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
    const normalizeNewThreadIntent: LegacyLayoutIntentUpdate = (current) => ({
      ...current,
      conversationRail:
        current.conversationRail.kind === "recent"
          ? current.conversationRail
          : { kind: "closed" },
      inspectorOpen: false,
      openCapsuleTabs: [],
    });
    commitLegacyLayoutIntent(
      "user-route",
      normalizeNewThreadIntent,
      (current) => ({
        ...normalizeNewThreadIntent(current),
        openCapsuleTabs: current.openCapsuleTabs,
      }),
    );
    startNewThreadDraft({
      selectableNewThreadWorkspaces,
      pendingNewThreadWorkspaceEntry,
      activeThreadNewThreadWorkspace: activeThreadNewThreadWorkspace,
      selectedNewThreadWorkspaceEntry,
      setError,
      enterDraft: (selection) => {
        enterNewThreadDraft({ workspaceSelection: selection, agentId: null });
      },
      syncComposerPhase,
    });
  }

  /**
   * Draft entry is a COMMAND: re-entering the same draft route must still
   * reset pendings, clear the composer, and (re)bind the bot, so openers
   * call this directly — an equal new-thread route through navigate would
   * no-op and swallow those side effects (review #TASK-1621). The hash
   * syncs from the state fold; external #/new entries reach this through
   * the bridge's new-thread application. `agentId` undefined keeps the
   * user's current pick (bot drafts, workspace drafts).
   */
  function enterNewThreadDraft(input: {
    workspaceSelection: DraftWorkspaceSelection | null;
    agentId?: string | null;
    botId?: string | null;
  }) {
    setError(null);
    setNewThreadDraftActive(true);
    setSelectedThreadId(null);
    // The draft's workspace tri-state resolves exactly once, here at draft
    // creation: an explicit selection (route, sidebar entry point, chip)
    // wins; otherwise the then-current default. It never re-resolves on
    // catalog refresh.
    const selection =
      input.workspaceSelection ??
      resolveDefaultDraftWorkspace(visibleWorkspaceList(desktopState));
    setPendingWorkspaceSelection(selection);
    setPendingWorkspaceMode("local");
    setPendingBotId(input.botId ?? null);
    if (input.agentId !== undefined) {
      const nextAgentId = input.agentId
        || desktopAgentCatalog.effectiveDefaultAgentId;
      setPendingAgentId(nextAgentId);
    }
    clearComposerDraft();
    requestComposerFocus();
    // The contentView selector flips through this commit (6c-2b). Kept
    // values fold from the current synchronous closure — this command is
    // sync, so the closure IS the latest value.
    desktopRouteStore.syncRoute({
      kind: "new-thread",
      workspacePath: routeWorkspaceFromDraftSelection(selection),
      agentId: input.agentId !== undefined
        ? input.agentId || desktopAgentCatalog.effectiveDefaultAgentId
        : pendingAgentId,
    });
  }

  /**
   * Selection-fallback route sync (6c-2c): deletion/removal fallbacks in
   * the thread view converge the route the way the fold used to — a
   * surviving selection syncs its thread route, an emptied one rests at
   * thread-home. Callers gate on the thread view (the fold only followed
   * the selection there). sync origin — never re-applied.
   */
  function syncSelectedThreadRoute(threadId: string | null) {
    desktopRouteStore.syncRoute(
      threadId ? { kind: "thread", threadId } : { kind: "thread-home" },
    );
  }

  /**
   * Draft-route sync for in-draft mutations (6c-2c): agent and workspace
   * picks change the folded new-thread route, so sync it
   * directly (overrides carry the just-picked values; the rest folds from
   * the current render). Guarded by the same predicate the fold uses for
   * the new-thread shape — outside the draft these pendings do not drive
   * the hash. Equal-route syncs are no-ops while the fold still runs.
   */
  function syncDraftRoute(overrides: {
    workspacePath?: string | null;
    agentId?: string | null;
  }) {
    // Base on the committed route, not render closures: the guard (route
    // IS the draft) and the untouched params stay correct even when the
    // caller is an async background correction (review #TASK-1627).
    const route = desktopRouteStore.getSnapshot().route;
    if (route.kind !== "new-thread") {
      return;
    }
    desktopRouteStore.syncRoute({
      kind: "new-thread",
      workspacePath:
        overrides.workspacePath !== undefined
          ? overrides.workspacePath
          : (route.workspacePath ?? null),
      agentId:
        overrides.agentId !== undefined
          ? overrides.agentId
          : (route.agentId ?? null),
    });
  }

  function handleStartDraftForAgent(agentId: string) {
    const nextWorkspace = pickPreferredWorkspace(
      selectableNewThreadWorkspaces,
      pendingNewThreadWorkspaceEntry,
      activeThreadNewThreadWorkspace,
      selectedNewThreadWorkspaceEntry,
    );
    enterNewThreadDraft({
      workspaceSelection: nextWorkspace?.path
        ? { kind: "path", path: nextWorkspace.path }
        : null,
      agentId,
    });
    syncComposerPhase("");
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
            setPendingWorkspaceSelection(null);
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
        draftSelectionWorkspacePath(pendingWorkspaceSelectionRef.current) ===
          initialWorkspacePath,
      shouldOpenResolvedThread: (groupId, initialWorkspacePath) =>
        newThreadDraftActiveRef.current &&
        selectedThreadIdRef.current === null &&
        pendingBotIdRef.current === groupId &&
        draftSelectionWorkspacePath(pendingWorkspaceSelectionRef.current) ===
          initialWorkspacePath &&
        !composerHasPayloadRef.current,
      setError,
      enterBotDraft: (workspacePath, botId) => {
        // agentId stays undefined: the legacy bot draft left the user's pick
        // untouched, and an async fallback must not write a stale closure
        // value back (review #TASK-1621).
        enterNewThreadDraft({
          workspaceSelection: workspacePath
            ? { kind: "path", path: workspacePath }
            : null,
          botId,
        });
      },
      // Background workspace correction for an already-open bot draft: the
      // sync helper's route-based guard makes this async-safe (review
      // #TASK-1627) — a draft the user already left is a no-op.
      setPendingWorkspacePath: (value) => {
        const selection = value ? { kind: "path" as const, path: value } : null;
        setPendingWorkspaceSelection(selection);
        syncDraftRoute({
          workspacePath: routeWorkspaceFromDraftSelection(selection),
        });
      },
      syncComposerPhase,
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
      enterDraft: (selection) => {
        enterNewThreadDraft({
          workspaceSelection: selection,
          agentId: null,
        });
      },
      syncComposerPhase,
    });
  }

  async function handleAddWorkspace() {
    setAddWorkspaceDialog({
      source: "new-thread",
      initialPath: pendingWorkspacePath || selectedWorkspaceEntry?.path || "",
    });
  }

  function closeAddWorkspaceDialog(workspace: DesktopWorkspace | null = null) {
    setAddWorkspaceDialog((current) => {
      current?.resolve?.(workspace);
      return null;
    });
  }

  async function addWorkspacePathFromPicker(
    path: string,
    name?: string | null,
  ): Promise<DesktopWorkspace | null> {
    setError(null);
    setWorkspaceMutation("add");
    const epoch = workspaceEpochRef.current;
    try {
      const result = await requestDesktopStateResult(
        () => window.garyxDesktop.addWorkspaceByPath({ path, name }),
        (response) => response.state,
      );
      if (!isCurrentWorkspaceEpoch(epoch)) {
        return null;
      }
      setDesktopState(result.state);
      return result.workspace || null;
    } catch (workspaceError) {
      if (!isCurrentWorkspaceEpoch(epoch)) {
        return null;
      }
      setError(
        workspaceError instanceof Error
          ? workspaceError.message
          : "Failed to add workspace",
      );
      return null;
    } finally {
      if (isCurrentWorkspaceEpoch(epoch)) {
        setWorkspaceMutation(null);
      }
    }
  }

  async function confirmAddWorkspace(path: string, name: string | null) {
    const request = addWorkspaceDialog;
    if (!request) {
      return;
    }
    const workspace = await addWorkspacePathFromPicker(path, name);
    if (workspace) {
      if (request.source === "new-thread") {
        setNewThreadDraftActive(true);
        setPendingWorkspaceSelection(
          workspace.path ? { kind: "path", path: workspace.path } : null,
        );
        setPendingWorkspaceMode("local");
        requestComposerFocus();
        desktopRouteStore.syncRoute({
          kind: "new-thread",
          workspacePath: workspace.path,
          agentId: pendingAgentId,
        });
      }
      closeAddWorkspaceDialog(workspace);
    }
  }

  async function handleRemoveWorkspace(workspacePath: string) {
    setError(null);
    setWorkspaceMutation("remove");
    const epoch = workspaceEpochRef.current;
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
      const nextState = await requestDesktopState(() =>
        window.garyxDesktop.removeWorkspace({ workspacePath }),
      );
      if (!isCurrentWorkspaceEpoch(epoch)) {
        return;
      }
      setDesktopState(nextState);
      if (selectedThreadId) {
        const selectedThreadStillExists = nextState.threads.some(
          (thread) => thread.id === selectedThreadId,
        );
        if (!selectedThreadStillExists) {
          const fallbackId = nextState.threads[0]?.id || null;
          setSelectedThreadId(fallbackId);
          if (contentView === "thread") {
            syncSelectedThreadRoute(fallbackId);
          }
        }
      }
    } catch (removeError) {
      if (!isCurrentWorkspaceEpoch(epoch)) {
        return;
      }
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
      if (isCurrentWorkspaceEpoch(epoch)) {
        setWorkspaceMutation(null);
      }
    }
  }

  async function handleRequestRemoveWorkspace(workspace: DesktopWorkspace) {
    setWorkspaceMenuOpenPath(null);
    await handleRemoveWorkspace(workspace.path || "");
  }

  async function handlePinWorkspace(
    workspace: DesktopWorkspace,
    pinned: boolean,
  ) {
    if (!workspace.path) {
      return;
    }
    setError(null);
    setWorkspaceMenuOpenPath(null);
    const epoch = workspaceEpochRef.current;
    try {
      const nextState = await requestDesktopState(() =>
        window.garyxDesktop.pinWorkspace({
          workspacePath: workspace.path || "",
          pinned,
        }),
      );
      if (!isCurrentWorkspaceEpoch(epoch)) {
        return;
      }
      setDesktopState(nextState);
    } catch (pinError) {
      if (!isCurrentWorkspaceEpoch(epoch)) {
        return;
      }
      setError(
        pinError instanceof Error ? pinError.message : "Failed to pin workspace",
      );
    }
  }

  async function handleRenameWorkspaceSubmit(
    workspace: DesktopWorkspace,
    name: string,
  ) {
    if (!workspace.path) {
      setWorkspaceRenameTarget(null);
      return;
    }
    setError(null);
    setWorkspaceRenameSaving(true);
    const epoch = workspaceEpochRef.current;
    try {
      const nextState = await requestDesktopState(() =>
        window.garyxDesktop.renameWorkspace({
          workspacePath: workspace.path || "",
          name,
        }),
      );
      if (!isCurrentWorkspaceEpoch(epoch)) {
        return;
      }
      setDesktopState(nextState);
      setWorkspaceRenameTarget(null);
    } catch (renameError) {
      if (!isCurrentWorkspaceEpoch(epoch)) {
        return;
      }
      setError(
        renameError instanceof Error
          ? renameError.message
          : "Failed to rename workspace",
      );
    } finally {
      if (isCurrentWorkspaceEpoch(epoch)) {
        setWorkspaceRenameSaving(false);
      }
    }
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
    if (!lifecycleStoreIncarnation) {
      setError("Thread storage identity is unavailable. Refresh and try again.");
      recentThreadFeeds.forceReplacement();
      threadFavorites.refreshSnapshot();
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
    const recentRollback = recentThreadFeeds.removeThread(targetThreadId);
    setDesktopState((current) =>
      current ? desktopStateWithoutThread(current, targetThreadId) : current,
    );
    if (deletingSelected) {
      setSelectedThreadId(fallbackThread?.id || null);
      setThreadEntrySelectionSource(null);
      if (contentView === "thread") {
        syncSelectedThreadRoute(fallbackThread?.id || null);
      }
    }
    dispatchMessageState({
      type: "thread/delete",
      threadId: targetThreadId,
    });

    try {
      const api = getDesktopApi();
      const operationId = globalThis.crypto.randomUUID();
      const gatewayScope = desktopState.entitiesGatewayUrl || "";
      const runtimeEpoch = lifecycleRuntimeEpoch;
      const archivedResult = await runLifecycleMutation(
        {
          gatewayScope,
          runtimeEpoch,
          operationId,
          expectedStoreIncarnation: lifecycleStoreIncarnation,
          threadId: targetThreadId,
        },
        ({ operationId: stableOperationId, expectedStoreIncarnation }) =>
          api.archiveThread({
            threadId: targetThreadId,
            operationId: stableOperationId,
            expectedStoreIncarnation,
            endpointKeys: Array.from(endpointKeys).sort(),
          }),
        {
          isCurrent: (identity) =>
            lifecycleIdentityRef.current.gatewayScope === identity.gatewayScope &&
            lifecycleIdentityRef.current.runtimeEpoch === identity.runtimeEpoch,
        },
      );
      const settlement = lifecycleUiSettlement(archivedResult);
      if (archivedResult.kind !== "applied") {
        if (settlement.rollbackOptimistic) {
          recentThreadFeeds.rollbackRemoval(recentRollback);
        }
        if (archivedResult.kind === "cancelled") {
          return;
        }
        setError(settlement.errorMessage);
        if (settlement.operationIdConflict) {
          console.error("Thread lifecycle operation_id conflict", {
            operationId,
            threadId: targetThreadId,
          });
        }
        void refreshDesktopState().catch(() => null);
        if (settlement.requireFullReplacement) {
          recentThreadFeeds.forceReplacement();
          threadFavorites.refreshSnapshot();
        }
        return;
      }
      setDesktopState(
        desktopStateWithoutThread(archivedResult.value, targetThreadId),
      );
      if (settlement.requireFullReplacement) {
        recentThreadFeeds.forceReplacement();
        threadFavorites.refreshSnapshot();
      }
    } catch (archiveError) {
      recentThreadFeeds.rollbackRemoval(recentRollback);
      setError(
        archiveError instanceof Error
          ? archiveError.message
          : "Failed to delete the thread",
      );
      void refreshDesktopState().catch(() => null);
      // IPC failure occurs after the renderer handed the operation to Main;
      // whether the Gateway committed is unknowable, so reconstruct all
      // affected feeds after preserving today's rollback/error UX.
      recentThreadFeeds.forceReplacement();
      threadFavorites.refreshSnapshot();
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
      navigateThread: (threadId) => {
        pendingThreadEntrySourceHintRef.current = entrySource;
        desktopRouteStore.navigate(
          { kind: "thread", threadId },
          { replace: true },
        );
      },
    });
    return false;
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

  const sideChatPanel = (
    <SideChatPanel
      sessions={sideChatSessions}
      activeThread={activeThread}
      composerAgentOptions={composerAgentOptions}
      availableWorkspaceCount={availableWorkspaceCount}
      threadAvatarCatalog={threadAvatarCatalog}
      botGroups={botGroups}
      botBindingDisabled={bindingMutation === "bot-binding"}
      slashCommands={commands}
      slashCommandsLoaded={commandsLoaded}
      slashCommandsLoading={commandsLoading}
      loadSlashCommands={loadSlashCommands}
      composerProviderType={composerProviderType}
      pendingAgentId={pendingAgentId}
      settingsDraft={settingsDraft}
      desktopAgents={desktopAgents}
      desktopAgentMap={desktopAgentMap}
      threadSummaryById={threadSummaryById}
      boundBotsForThread={boundBotsForThread}
      inferProviderTypeForThread={inferProviderTypeForThread}
      pendingInputOriginRefsForThread={pendingInputOriginRefsForThread}
      prepareAttachmentUploads={prepareAttachmentUploads}
      setDesktopState={setDesktopState}
      setError={setError}
      sideChatMessagesRef={sideChatMessagesRef}
      deferredQueueDrainByThreadRef={deferredQueueDrainByThreadRef}
      onLocalWorkspaceFileLinkClick={handleLocalFileLinkClick}
      onResumeProviderSession={handleResumeProviderSession}
      onRetryFailedMessage={(message) => {
        void handleRetryFailedMessage(message);
      }}
      onOpenThreadById={(threadId) => {
        void openExistingThread(threadId);
      }}
      onOpenCapsule={(card) => {
        desktopRouteStore.navigate(
          { kind: "capsule", capsuleId: card.capsule_id },
          { replace: true },
        );
      }}
      onReorderQueuedIntent={reorderQueuedIntent}
      syncThreadBotBinding={syncThreadBotBinding}
      t={t}
    />
  );

  // The dock is built for any thread (not only workspace threads) so it can host
  // capsule tabs opened from the transcript even when no workspace is attached
  // (#TASK-1470). Built-in workspace tools stay gated by `hasWorkspace` inside.
  const sideToolsPanel = contentView === "thread" ? (
    <WorkspaceFileTree
      activeWorkspacePath={activeWorkspacePath}
      expandedWorkspaceDirectories={expandedWorkspaceDirectories}
      onActivateEntry={(entry) => {
        void handleWorkspaceFileEntryActivate(entry);
      }}
      onUploadFiles={(files) => {
        void uploadWorkspaceFilesToActiveWorkspace(files);
      }}
      selectedWorkspaceFile={selectedWorkspaceFile}
      workspaceDirectories={workspaceDirectories}
      workspaceUploadInputRef={workspaceUploadInputRef}
    >
      {(workspaceDirectoryPanel, workspaceFilter) => (
    <ThreadSideToolsPanel
      activeThreadTitle={activeThread?.title || null}
      activeWorkspaceName={activeWorkspace?.name || null}
      activeWorkspacePath={activeWorkspacePath}
      activeThreadId={selectedThreadId}
      selectedWorkspaceFile={selectedSideToolWorkspaceFile}
      sideChatPanel={sideChatPanel}
      workspaceBranch={composerWorkspaceBranch}
      workspaceDirectoryPanel={workspaceDirectoryPanel}
      workspaceFileFilter={workspaceFilter.value}
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
        commitLegacyLayoutIntent("user-route", (current) => ({
          ...current,
          openCapsuleTabs: current.openCapsuleTabs.filter(
            (tab) => tab.capsuleId !== capsuleId,
          ),
        }));
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
        commitLegacyLayoutIntent("user-panel", (current) => ({
          ...current,
          inspectorOpen: false,
          openCapsuleTabs: [],
        }));
        setPendingActiveCapsuleId(null);
      }}
      onOpenSideChat={() => {
        void ensureSideChatThreadOp(sideChatOpsContext());
      }}
      onWorkspaceFileFilterChange={workspaceFilter.onChange}
    />
      )}
    </WorkspaceFileTree>
  ) : null;

  // The dock shows when the inspector is open (workspace tools) or any capsule
  // tab is open. Capsule visibility is independent of `inspectorOpen` so it is
  // not force-closed for no-workspace threads (#TASK-1470).
  const showConversationSideTools = Boolean(
    sideToolsPanel && (inspectorOpen || openCapsuleTabs.length > 0),
  );
  const conversationClassName = [
    "conversation",
    contentView === "thread" ? "thread-view" : null,
    isSettingsView ? "settings-view" : null,
    isCapsulesView ? "capsules-view" : null,
    isAutomationView ? "automation-view" : null,
    isAgentsView ? "agents-view" : null,
    isSkillsView ? "skills-view" : null,
    isTasksView ? "tasks-view" : null,
    sideToolsPresented ? "with-side-tools" : null,
    sideToolsResizing ? "side-tools-resizing" : null,
  ]
    .filter(Boolean)
    .join(" ");
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
        activeMessages={activeMessages}
        activePendingAckIntents={visiblePendingAckIntents}
        activePendingAutomationRun={activePendingAutomationRun}
        activeToolGroupId={activeToolGroupId}
        activeQueue={activeQueue}
        renderState={activeRenderState}
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
        composerPendingUploads={composerPendingUploads}
        composerEditingLocked={composerEditingLocked || newDraftHasNoEnabledAgent}
        composerLocked={composerLocked || newDraftHasNoEnabledAgent}
        composerPlaceholder={newDraftHasNoEnabledAgent
          ? t("Enable an agent to start a new thread")
          : composerPlaceholder}
        composerProviderType={composerProviderType}
        composerResetKey={composerResetKey}
        composerWorkspaceBranch={composerWorkspaceBranch}
        composerWorkspaceMode={composerWorkspaceMode}
        draftWorkspaceSelection={pendingWorkspaceSelection}
        draftWorkspaceMode={pendingWorkspaceMode}
        draftWorkspaces={workspacePickerWorkspaces}
        gatewayHome={desktopState?.gatewayHome ?? null}
        workspaceAddBusy={workspaceMutation === "add"}
        onDraftWorkspaceSelectionChange={(selection) => {
          setPendingWorkspaceSelection(selection);
          if (selection.kind !== "path") {
            setPendingWorkspaceMode("local");
          }
          syncDraftRoute({
            workspacePath: routeWorkspaceFromDraftSelection(selection),
          });
        }}
        onDraftWorkspaceModeChange={setPendingWorkspaceMode}
        onDraftAddWorkspace={() => {
          void handleAddWorkspace();
        }}
        activeThreadBot={activeThreadBot}
        activeThreadBotId={activeThreadBotId}
        botBindingDisabled={bindingMutation === "bot-binding"}
        botGroups={botGroups}
        slashCommands={commands}
        slashCommandsLoaded={commandsLoaded}
        slashCommandsLoading={commandsLoading}
        composerTextareaRef={composerTextareaRef}
        historyLoading={historyLoading}
        historyLoadingEarlier={Boolean(activeHistoryPagination?.loadingBefore)}
        ignoreComposerSubmitUntilRef={ignoreComposerSubmitUntilRef}
        inspectorOpen={embedded ? false : sideToolsEffectiveVisible}
        isActiveSendingThread={isActiveSendingThread}
        canSteerQueuedPrompt={canSteerQueuedPrompt}
        isComposingRef={isComposingRef}
        messagesRef={messagesRef}
        newThreadSelectedAgentId={pendingAgentId}
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
        onLocalWorkspaceFileLinkClick={handleLocalFileLinkClick}
        onMarkIgnoreComposerSubmitWindow={markIgnoreComposerSubmitWindow}
        scrollIntent={transcriptScrollIntent}
        activeHistoryPagination={activeHistoryPagination}
        activeThreadMessageKey={activeThreadMessageKey}
        onRemoveComposerFile={removeComposerFile}
        onRemoveComposerImage={removeComposerImage}
        onRemoveComposerPendingUpload={removeComposerPendingUpload}
        onRemoveComposerBrowserAnnotation={removeComposerBrowserAnnotation}
        onReorderQueuedIntent={reorderQueuedIntent}
        onSelectNewThreadAgent={(agentId) => {
          setPendingAgentId(agentId);
          syncDraftRoute({ agentId });
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
            void selectExistingThreadInPlace(threadId);
          } else {
            void openExistingThread(threadId);
          }
        }}
        onOpenCapsule={(card) => {
          if (!selectedThreadId) {
            return;
          }
          // Open/activate this capsule as a tab in the right dock (#TASK-1470).
          // Dedup by id; refresh title/revision if it is already open. The
          // capsule path drives the dock without inspector state.
          const capsuleId = card.capsule_id;
          const title = card.title?.trim() || "";
          commitLegacyLayoutIntent("user-route", (current) => ({
            ...current,
            openCapsuleTabs: current.openCapsuleTabs.some(
              (tab) => tab.capsuleId === capsuleId,
            )
              ? current.openCapsuleTabs.map((tab) =>
                  tab.capsuleId === capsuleId
                    ? {
                        ...tab,
                        revision: card.revision,
                        title: title || tab.title,
                      }
                    : tab,
                )
              : [
                  ...current.openCapsuleTabs,
                  { capsuleId, revision: card.revision, title },
                ],
          }));
          setPendingActiveCapsuleId(capsuleId);
        }}
        onSteerQueuedPrompt={(item) => {
          void handleSteerQueuedPrompt(item);
        }}
        selectedThreadId={selectedThreadId}
        showAutomationRunInitialPlaceholder={showAutomationRunInitialPlaceholder}
        showHistoryLoadingPlaceholder={showHistoryLoadingPlaceholder}
        showTailThinking={showTailThinking}
        rateLimit={activeRateLimit}
        onRateLimitContinue={() => handleSendPromptText("continue")}
        taskTreeDocked={embedded ? false : taskTreeDocked}
        threadLayoutStyle={undefined}
        threadAvatarCatalog={threadAvatarCatalog}
        visibleRemoteAwaitingAckInputs={visibleRemoteAwaitingAckInputs}
        visibleRemotePendingInputs={visibleRemotePendingInputs}
      />
    );
  }

  // Batch 5a review fix: every return branch renders through this chrome
  // so the colocated MemoryDialogRoot (and its unsaved-draft state) never
  // unmounts when the shell flips into the loading/gateway-setup branches
  // mid-session — the legacy controller lived at the hook layer and
  // survived those flips; the root must too.
  const appShellChrome = (content: ReactNode) => (
    <GatewayMirrorContext.Provider value={gatewayMirror}>
      <WorkspaceEpochContext.Provider value={workspaceEpoch}>
        <I18nProvider languagePreference={settingsDraft.languagePreference}>
          {content}
          <MemoryDialogRoot ref={memoryDialogRef} />
        </I18nProvider>
      </WorkspaceEpochContext.Provider>
    </GatewayMirrorContext.Provider>
  );

  if (loading) {
    return appShellChrome(
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
        </div>,
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

    return appShellChrome(
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
        </div>,
    );
  }

  return appShellChrome(
    <div
      className={appShellClassName}
      ref={layoutRootRef}
    >
      <ToastViewportHost />
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
        gatewayHome={desktopState?.gatewayHome ?? null}
        botGroups={visibleBotGroups}
        formatThreadTimestamp={formatThreadTimestamp}
        isAutomationView={isAutomationView}
        isCapsulesView={isCapsulesView}
        isAgentsView={isAgentsView}
        isBrowserView={isBrowserView}
        isSettingsView={isSettingsView}
        isSkillsView={isSkillsView}
        isTasksView={isTasksView}
        recentRailOpen={shouldShowConversationRail && recentThreadsRailOpen}
        onBackToThreads={() => {
          desktopRouteStore.navigate({ kind: "thread-home" }, { replace: true });
        }}
        onCreateThreadForWorkspace={(workspacePath) => {
          handleCreateThreadForWorkspace(workspacePath);
        }}
        onNewThread={() => {
          void handleNewThread();
        }}
        onOpenRecent={() => {
          commitLegacyLayoutIntent("user-route", (current) => ({
            ...current,
            conversationRail:
              shouldShowConversationRail &&
              current.conversationRail.kind === "recent"
                ? { kind: "closed" }
                : { kind: "recent" },
          }));
          if (!shouldShowConversationRail) {
            desktopRouteStore.navigate({ kind: "thread-home" }, { replace: true });
          }
        }}
        onOpenBot={(group) => {
          void (async () => {
            commitLegacyLayoutIntent("user-route", (current) => ({
              ...current,
              conversationRail:
                current.conversationRail.kind === "bot" &&
                current.conversationRail.groupId === group.id
                  ? current.conversationRail
                  : { kind: "closed" },
            }));
            await handleBotClick(group);
          })();
        }}
        onOpenPinnedThread={(threadId) => {
          void (async () => {
            commitLegacyLayoutIntent("user-route", (current) => ({
              ...current,
              conversationRail: { kind: "closed" },
            }));
            await openExistingThread(threadId, "pinned");
          })();
        }}
        onUnpinThread={(threadId) => {
          togglePinnedThread(threadId);
        }}
        onArchivePinnedThread={(threadId) => {
          void handleDeleteThread(threadId);
        }}
        onPinnedThreadDragCancel={cancelPinnedThreadDrag}
        onPinnedThreadDragStart={beginPinnedThreadDrag}
        onReorderPinnedThreads={reorderPinnedThreads}
        onToggleBotConversationGroup={(group) => {
          commitLegacyLayoutIntent("user-route", (current) => ({
            ...current,
            conversationRail:
              current.conversationRail.kind === "bot" &&
              current.conversationRail.groupId === group.id
                ? { kind: "closed" }
                : { kind: "bot", groupId: group.id },
          }));
        }}
        onPinWorkspace={(workspace, pinned) => {
          void handlePinWorkspace(workspace, pinned);
        }}
        onRequestRenameWorkspace={(workspace) => {
          setWorkspaceRenameTarget(workspace);
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
          desktopRouteStore.navigate({ kind: "view", view: "agents" }, { replace: true });
        }}
        onOpenSkills={() => {
          desktopRouteStore.navigate({ kind: "view", view: "skills" }, { replace: true });
        }}
        onOpenCapsules={() => {
          desktopRouteStore.navigate({ kind: "view", view: "capsules" }, { replace: true });
        }}
        onOpenTasks={() => {
          desktopRouteStore.navigate({ kind: "view", view: "tasks" }, { replace: true });
        }}
        onRequestRemoveWorkspace={(workspace) => {
          void handleRequestRemoveWorkspace(workspace);
        }}
        onSelectAutomation={(automationId) => {
          desktopRouteStore.navigate(
            { kind: "automation", automationId },
            { replace: true },
          );
        }}
        onSelectSettingsTab={(tabId) => {
          desktopRouteStore.navigate(
            { kind: "settings", tabId },
            { replace: true },
          );
        }}
        pinnedThreadRows={pinnedThreadRows}
        pinnedThreadSyncPending={
          Boolean(pinnedOrderIngress.desiredOrder) ||
          Boolean(
            pinnedOrderMainSnapshot?.unsettled &&
              // Normalize BOTH sides: the main snapshot identity preserves
              // case while the state URL was lowercased, which hid the badge
              // for uppercase gateway URLs.
              normalizeGatewayIdentity(pinnedOrderMainSnapshot.gatewayIdentity) ===
                normalizeGatewayIdentity(
                  desktopState?.entitiesGatewayUrl ||
                    desktopState?.settings.gatewayUrl,
                ),
          )
        }
        selectedAutomationId={selectedAutomationId}
        selectedThreadId={botRootSelectedThreadId}
        setWorkspaceMenuOpenPath={setWorkspaceMenuOpenPath}
        settingsActiveTab={settingsActiveTab}
        workspaceMenuOpenPath={workspaceMenuOpenPath}
        workspaceMutation={workspaceMutation}
        workspaceThreadGroups={workspaceThreadGroups}
      />
      {conversationRailPresented && activeBotConversationGroup ? (
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
            commitLegacyLayoutIntent("user-route", (current) => ({
              ...current,
              conversationRail: { kind: "closed" },
            }));
          }}
          onOpenEndpoint={(endpoint) => {
            void handleOpenThreadFromEndpoint(endpoint, "bot-conversation");
          }}
          onRailResizeStart={handleRailResizeStart}
          railResizing={railResizing}
          selectedThreadId={botConversationSelectedThreadId}
        />

      ) : conversationRailPresented && recentThreadsRailOpen ? (
        <RecentConversationSidebar
          collapseLabel={t("Collapse recent threads")}
          feed={showingFavoriteThreads ? favoritesFeed : recentThreadFeeds.selectedFeed!}
          formatThreadTimestamp={formatThreadTimestamp}
          logo={
            <span className="recent-conversation-logo">
              <RecentIcon />
            </span>
          }
          onClose={() => {
            commitLegacyLayoutIntent("user-route", (current) => ({
              ...current,
              conversationRail: { kind: "closed" },
            }));
          }}
          onLoadMore={showingFavoriteThreads ? undefined : recentThreadFeeds.loadMore}
          onRailResizeStart={handleRailResizeStart}
          onRetry={
            showingFavoriteThreads
              ? threadFavorites.refreshSnapshot
              : recentThreadFeeds.retry
          }
          onSelectFilter={(filter) => {
            recentThreadFeeds.selectFilter(filter);
            if (filter === "favorites") {
              threadFavorites.refreshSnapshot();
            }
          }}
          railResizing={railResizing}
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
            onUnfavorite: showingFavoriteThreads
              ? () => {
                  threadFavorites.setFavorite(row.thread.id, false);
                }
              : undefined,
            onArchive: row.isBusy
              ? undefined
              : () => {
                  void handleDeleteThread(row.thread.id);
                },
          }))}
          selectedFilter={recentThreadFeeds.state.selectedFilter}
        />
      ) : null}
      <AddBotDialogRoot
        agentTargets={addBotAgentTargets}
        effectiveDefaultAgentId={desktopAgentCatalog.effectiveDefaultAgentId}
        onAddWorkspace={addWorkspacePathFromPicker}
        onCreateChannel={handleAddChannelAccount}
        ref={addBotDialogRef}
        workspaces={workspacePickerWorkspaces}
      />
      <WorkspaceRenameDialog
        onCancel={() => setWorkspaceRenameTarget(null)}
        onSubmit={(workspace, name) => {
          void handleRenameWorkspaceSubmit(workspace, name);
        }}
        saving={workspaceRenameSaving}
        workspace={workspaceRenameTarget}
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
        >
          {isCapsulesView || isTasksView ? null : showStaticWindowToolbar ? (
            <div aria-hidden="true" className="settings-window-toolbar" />
          ) : (
            <header className="conversation-header">
              <ConversationTitleRoot
                activeAutomationThread={Boolean(activeAutomationThread)}
                activeThread={activeThread}
                activeThreadBot={activeThreadBot}
                activeWorkspaceName={activeWorkspace?.name || null}
                archiveThreadDisabled={Boolean(
                  !selectedThreadId ||
                    activeAutomationThread ||
                    isRuntimeBusy(activeRuntime?.state),
                )}
                canEditThreadTitle={canEditThreadTitle}
                contextText={conversationContextText}
                isAutomationView={isAutomationView}
                isBotsView={isBotsView}
                isSkillsView={isSkillsView}
                isThreadFavorite={Boolean(
                  selectedThreadId && threadFavorites.isFavorite(selectedThreadId),
                )}
                isThreadPinned={selectedThreadPinned}
                onArchiveThread={() => {
                  void handleDeleteThread();
                }}
                onToggleFavoriteThread={() => {
                  if (selectedThreadId) {
                    threadFavorites.toggleFavorite(selectedThreadId);
                  }
                }}
                onTogglePinnedThread={() => {
                  if (selectedThreadId) {
                    togglePinnedThread(selectedThreadId);
                  }
                }}
                ref={conversationTitleRef}
                setDesktopState={setDesktopState}
                setError={setError}
              />
              <ConversationHeaderActions
                gatewayStatusLabel={gatewayIndicator?.label || null}
                gatewayStatusTone={gatewayIndicator?.tone || null}
                inspectorOpen={inspectorOpen}
                isAutomationView={isAutomationView}
                isBotsView={isBotsView}
                isSkillsView={isSkillsView}
                selectedThreadId={selectedThreadId}
                threadInfo={activeThreadInfo}
                threadInfoLoaded={activeThreadInfoLoaded}
                onCreateAutomation={() => {
                  openAutomationDialog("create");
                }}
                onOpenThreads={() => {
                  desktopRouteStore.navigate({ kind: "thread-home" }, { replace: true });
                }}
                onToggleInspector={() => {
                  const nextInspectorOpen =
                    !appliedLayoutIntentRef.current.inspectorOpen;
                  commitLegacyLayoutIntent("user-panel", (current) => ({
                    ...current,
                    inspectorOpen: nextInspectorOpen,
                  }));
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
                    effectiveDefaultAgentId={desktopAgentCatalog.effectiveDefaultAgentId}
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
                      const nextState = await requestDesktopState(() =>
                        window.garyxDesktop.addGatewayProfile(input),
                      );
                      setDesktopState(nextState);
                    }}
                    onUpdateGatewayProfile={async (input) => {
                      const normalizeIdentity = (value: string) =>
                        value.trim().replace(/\/+$/, "").toLowerCase();
                      const previousIdentity = normalizeIdentity(
                        desktopState?.entitiesGatewayUrl ||
                          desktopState?.settings.gatewayUrl ||
                          "",
                      );
                      const editedProfile = desktopState?.gatewayProfiles.find(
                        (profile) => profile.id === input.profileId,
                      );
                      const switchesCurrentGateway = Boolean(
                        editedProfile &&
                          normalizeIdentity(editedProfile.gatewayUrl) ===
                            previousIdentity &&
                          normalizeIdentity(input.gatewayUrl) !== previousIdentity,
                      );
                      const targetIdentity = switchesCurrentGateway
                        ? normalizeIdentity(input.gatewayUrl)
                        : previousIdentity;
                      const gatewayRollback = switchesCurrentGateway
                        ? beginPinnedOrderGatewaySwitch(targetIdentity)
                        : null;
                      let nextState: DesktopState;
                      try {
                        nextState = await requestDesktopState(
                          () => window.garyxDesktop.updateGatewayProfile(input),
                          targetIdentity,
                        );
                      } catch (error) {
                        if (gatewayRollback) {
                          restorePinnedOrderGatewayDomain(gatewayRollback);
                        }
                        throw error;
                      }
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
                      const nextState = await requestDesktopState(() =>
                        window.garyxDesktop.deleteGatewayProfile({ profileId }),
                      );
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
                    onDeleteSlashCommand={(name) => {
                      return handleDeleteSlashCommand(name);
                    }}
                    onLocalSettingsChange={setSettingsDraft}
                    onMutateGatewayDraft={mutateGatewaySettingsDraft}
                    onSaveLocalSettingsNow={(options) => {
                      return handleSaveLocalSettingsNow(options);
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
                  memoryDialogRef.current?.open({
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
                gatewayScope={desktopState ? [
                  desktopState.settings.gatewayUrl,
                  desktopState.settings.gatewayAuthToken,
                  desktopState.settings.gatewayHeaders,
                ].join('\u001f') : ''}
                workspaces={workspacePickerWorkspaces}
                onAddWorkspace={addWorkspacePathFromPicker}
                onOpenMemory={(agent) => {
                  memoryDialogRef.current?.open({
                    scope: "agent",
                    agentId: agent.agentId,
                    title: `${agent.displayName || agent.agentId} memory.md`,
                  });
                }}
                onStartThread={handleStartDraftForAgent}
                onRefreshAgentTargets={refreshAgentTargets}
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
                  desktopRouteStore.navigate(
                    { kind: "capsule", capsuleId },
                    { replace: true },
                  );
                }}
                onCloseCapsulePreview={() => {
                  desktopRouteStore.navigate(
                    { kind: "view", view: "capsules" },
                    { replace: true },
                  );
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
                onTaskCreated={() => {
                  recentThreadFeeds.noteAllLocalMutation();
                  recentThreadFeeds.refreshAll();
                }}
                onToast={pushToast}
                workspaces={workspacePickerWorkspaces}
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
          {sideToolsPresented ? (
            <>
              <div
                aria-label={t("Resize side tools")}
                aria-orientation="vertical"
                aria-valuemax={clampSideToolsPanelWidth(
                  window.garyxDesktop.horizontalLayoutPolicy,
                  SIDE_TOOLS_PANEL_MAX_WIDTH,
                  currentConversationWidth(),
                )}
                aria-valuemin={sideToolsPanelMinWidth(
                  window.garyxDesktop.horizontalLayoutPolicy,
                )}
                aria-valuenow={sideToolsPanelWidth}
                className="side-tools-resizer"
                onKeyDown={handleSideToolsResizeKeyDown}
                onPointerDown={handleSideToolsResizeStart}
                role="separator"
                tabIndex={0}
              />
              {showConversationSideTools ? (
                sideToolsPanel
              ) : (
                <aside
                  aria-hidden="true"
                  className="thread-side-tools-panel"
                />
              )}
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

    </div>,
  );
}
