// RouteEffectBridge (endgame architecture batches 6c-1..6c-2): the one
// place that owns the two external route inputs — (a) route-store commits
// (navigate and hash/popstate), applied through applyDesktopRoute on a
// microtask, and (b) the garyx:// deep-link IPC channel, translated into
// route applications behind the gateway-readiness retry ladder. The
// state-to-hash fold is gone (6c-2c): view state is route selectors, and
// every selection/draft transition carries its own route sync.

import { useCallback, useEffect, useRef } from "react";

import type {
  ConnectionStatus,
  DesktopDeepLinkEvent,
  DesktopSessionProviderHint,
  DesktopState,
  DesktopTaskSummary,
  DesktopWorkspaceMode,
} from "@shared/contracts";

import { getDesktopApi } from "../platform/desktop-api";
import type { SettingsTabId } from "../settings-tabs";
import type { ToastTone } from "../toast";
import type { DesktopRoute } from "./desktop-route";
import type { DesktopRouteStore } from "./desktop-route-store";
import type { ContentView } from "./types";

export function isKnownThreadId(
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

const DEEP_LINK_GATEWAY_RETRY_DELAYS_MS = [0, 300, 650, 1_100, 1_700, 2_500];

export function waitForMs(ms: number): Promise<void> {
  return new Promise((resolve) => {
    window.setTimeout(resolve, ms);
  });
}

type RouteEffectBridgeArgs = {
  clearComposerDraft: () => void;
  contentView: ContentView;
  desktopState: DesktopState | null;
  desktopRouteStore: DesktopRouteStore;
  ensureThreadOpenable: (threadId: string) => Promise<boolean>;
  handleResumeProviderSession: (
    sessionId: string,
    providerHint?: DesktopSessionProviderHint | null,
  ) => Promise<void>;
  handleSelectAutomation: (automationId: string | null) => Promise<void>;
  handleSelectSettingsTab: (nextTab: SettingsTabId) => Promise<boolean>;
  loading: boolean;
  openExistingThread: (threadId: string) => Promise<boolean>;
  /**
   * The shared draft-entry command (review #TASK-1621): draft entry must
   * run its side effects even when the route equals the current one, so
   * openers call it directly and this application delegates to it for
   * route-only entries (external hash, deep link).
   */
  enterNewThreadDraft: (input: {
    workspacePath: string | null;
    agentId?: string | null;
    workflowId?: string | null;
    botId?: string | null;
  }) => void;
  /**
   * Task-summary hand-off from callers that already hold the object
   * (openWorkflowTask): the workflow-task application seeds from it
   * instead of clearing and re-fetching by id (6c-2a).
   */
  pendingWorkflowTaskHintRef: React.MutableRefObject<DesktopTaskSummary | null>;
  pushToast: (message: string, tone?: ToastTone, durationMs?: number) => void;
  requestComposerFocus: () => void;
  selectedThreadId: string | null;
  selectedWorkflowRunId: string | null;
  /**
   * The shell's thread-selection request sequence (the stale guard inside
   * selectExistingThreadInPlace). The deep-link open reads it to tell
   * whether its open actually selected or was superseded by a concurrent
   * user navigation.
   */
  selectThreadRequestSequenceRef: React.MutableRefObject<number>;
  selectedWorkflowTaskId: string | null;
  setConnection: React.Dispatch<React.SetStateAction<ConnectionStatus | null>>;
  setError: React.Dispatch<React.SetStateAction<string | null>>;
  setNewThreadDraftActive: React.Dispatch<React.SetStateAction<boolean>>;
  setPendingAgentId: React.Dispatch<React.SetStateAction<string>>;
  setPendingBotId: React.Dispatch<React.SetStateAction<string | null>>;
  setPendingWorkflowId: React.Dispatch<React.SetStateAction<string | null>>;
  setPendingWorkspaceMode: React.Dispatch<
    React.SetStateAction<DesktopWorkspaceMode>
  >;
  setPendingWorkspacePath: React.Dispatch<React.SetStateAction<string | null>>;
  setSelectedThreadId: React.Dispatch<React.SetStateAction<string | null>>;
  setSelectedWorkflowRunId: React.Dispatch<React.SetStateAction<string | null>>;
  setSelectedWorkflowTask: React.Dispatch<
    React.SetStateAction<DesktopTaskSummary | null>
  >;
};

export function useRouteEffectBridge({
  clearComposerDraft,
  contentView,
  desktopState,
  desktopRouteStore,
  ensureThreadOpenable,
  handleResumeProviderSession,
  handleSelectAutomation,
  handleSelectSettingsTab,
  loading,
  openExistingThread,
  enterNewThreadDraft,
  pendingWorkflowTaskHintRef,
  pushToast,
  requestComposerFocus,
  selectedThreadId,
  selectedWorkflowRunId,
  selectThreadRequestSequenceRef,
  selectedWorkflowTaskId,
  setConnection,
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
}: RouteEffectBridgeArgs): void {
  const deepLinkEventHandlerRef = useRef<(event: DesktopDeepLinkEvent) => void>(
    () => {},
  );

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

  /**
   * Returns true when the open actually selected the thread; false when a
   * concurrent user navigation superseded it (the request-sequence guard
   * inside selectExistingThreadInPlace resolves a stale open without
   * selecting — this reads the same sequence, so the caller can skip the
   * late route write).
   */
  async function openThreadFromDeepLink(threadId: string): Promise<boolean> {
    if (!(await ensureThreadOpenable(threadId))) {
      throw new Error(`Thread not found for garyx:// link: ${threadId}`);
    }
    const requestSequence = selectThreadRequestSequenceRef.current + 1;
    await openExistingThread(threadId);
    return selectThreadRequestSequenceRef.current === requestSequence;
  }

  const applyDesktopRoute = useCallback(
    async (
      route: DesktopRoute,
      origin: "navigate" | "external" = "navigate",
    ): Promise<void> => {
      switch (route.kind) {
        case "thread":
          await openExistingThread(route.threadId);
          return;
        case "new-thread":
          // Route-only entries (external hash, deep link) run the shared
          // draft-entry command with the route's params (no bot binding —
          // bots are not addressable in the hash).
          enterNewThreadDraft({
            workspacePath: route.workspacePath || null,
            agentId: route.agentId || null,
            workflowId: route.workflowId || null,
            botId: null,
          });
          return;
        case "automation":
          if (route.automationId) {
            await handleSelectAutomation(route.automationId);
          }
          return;
        case "settings":
          if (route.tabId) {
            await handleSelectSettingsTab(route.tabId);
          }
          return;
        case "workflow-task": {
          setError(null);
          // A caller that already holds the task summary seeds it through
          // the mailbox (openWorkflowTask); route-only entries (external
          // hash, deep link) clear and let the fetch effect load it.
          const taskHint = pendingWorkflowTaskHintRef.current;
          pendingWorkflowTaskHintRef.current = null;
          if (
            taskHint &&
            (taskHint.taskId || `#TASK-${taskHint.number}`) === route.taskId
          ) {
            setSelectedWorkflowTask(taskHint);
            setSelectedWorkflowRunId(taskHint.threadId || null);
          } else {
            setSelectedWorkflowTask(null);
            setSelectedWorkflowRunId(null);
          }
          return;
        }
        case "capsule":
          // The preview id is a route selector (6c-2c); nothing to apply.
          return;
        case "view":
          // The gallery/preview split is the route itself (6c-2c);
          // nothing to apply.
          return;
        case "thread-home": {
          setNewThreadDraftActive(false);
          setPendingWorkspacePath(null);
          setPendingWorkspaceMode("local");
          // The application runs synchronously on the pre-commit render, so
          // the closure values are current (the functional updater guarded a
          // startup race that no longer exists on this path).
          const nextSelected = isKnownThreadId(desktopState, selectedThreadId)
            ? selectedThreadId
            : desktopState?.threads[0]?.id || null;
          setSelectedThreadId(nextSelected);
          // thread-home redirect (design contract): an INTERNAL thread-home
          // navigation rests on the resolved selection's thread route. An
          // external #/thread entry keeps its hash (4b no-counter-write).
          if (origin === "navigate" && nextSelected) {
            desktopRouteStore.syncRoute({
              kind: "thread",
              threadId: nextSelected,
            });
          }
          return;
        }
      }
    },
    [
      desktopState,
      desktopRouteStore,
      handleSelectAutomation,
      handleSelectSettingsTab,
      openExistingThread,
      selectedThreadId,
    ],
  );

  useEffect(() => {
    if (
      loading ||
      contentView !== "workflow" ||
      !selectedWorkflowTaskId ||
      selectedWorkflowRunId
    ) {
      return;
    }
    let cancelled = false;
    const taskId = selectedWorkflowTaskId;
    void (async () => {
      try {
        const task = await getDesktopApi().getTask({ taskId });
        if (cancelled) {
          return;
        }
        setSelectedWorkflowTask(task);
        // Sync the route to the server-confirmed task id (normalization,
        // 6c-2c) — the id itself is a route selector now.
        desktopRouteStore.syncRoute({
          kind: "workflow-task",
          taskId: task.taskId || taskId,
        });
        if (task.executor?.type !== "workflow") {
          setError(`Task is not workflow-backed: ${task.taskId || taskId}`);
          return;
        }
        if (!task.threadId) {
          setError(`Workflow task has no thread: ${task.taskId || taskId}`);
          return;
        }
        setSelectedWorkflowRunId(task.threadId);
        setError(null);
      } catch (routeError) {
        if (!cancelled) {
          setError(
            routeError instanceof Error
              ? routeError.message
              : `Failed to load workflow task: ${taskId}`,
          );
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [
    contentView,
    loading,
    selectedWorkflowRunId,
    selectedWorkflowTaskId,
  ]);

  // Batch 6c-2a: the route application transaction. Every navigate and
  // external commit is applied through applyDesktopRoute (sync commits are
  // the state-to-hash pass itself — the state already reflects them, so
  // re-applying would re-run entry side effects against live state). While
  // any application is pending the state-to-hash effect below is
  // suppressed: the fold over mid-application state folds back a different
  // route (thread A→B folds #/thread/A mid-flight). Settle convergence is
  // version-keyed: only the application whose commit version is still
  // current requests the one convergence pass, and only for internal
  // commits — a failed external application must not counter-write the
  // entered hash (4b). A superseded application only decrements.
  useEffect(() => {
    return desktopRouteStore.subscribeCommits((event) => {
      if (event.origin === "sync") {
        // A sync commit means the state already reflects the route (the
        // command/selector paths wrote both); applying it would re-run
        // entry side effects against live state.
        return;
      }
      // Narrowed copy: TS does not carry the narrowing into the closure.
      const origin = event.origin;
      // Apply on a microtask, NOT synchronously inside commit(): navigate()
      // writes its hash AFTER committing, so a synchronous application's
      // own route sync (e.g. the thread-home redirect) would be overwritten
      // by navigate's trailing replaceHash. One microtask later navigate
      // has fully returned; the application still reads the pre-commit
      // render's closures. Late async landings inside the applications are
      // guarded by the store-version checks (6c-2a).
      void Promise.resolve().then(() => applyDesktopRoute(event.route, origin));
    });
  }, [applyDesktopRoute, desktopRouteStore]);

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
    deepLinkEventHandlerRef.current = (event: DesktopDeepLinkEvent) => {
      void (async () => {
        try {
          switch (event.type) {
            case "error":
              pushToast(event.error, "error");
              return;
            case "open-thread": {
              // Cold-start deep links race the managed gateway boot; the
              // readiness ladder covers open-thread like resume-session
              // (batch 6c-1) instead of failing the lookup immediately.
              await waitForGatewayReadyForDeepLink();
              // A user navigation during the readiness/open await supersedes
              // the deep link: the late write must not overwrite the route
              // the user moved to, so only write the hash when the open
              // actually selected. syncRoute (not navigate): the open above
              // IS the application — a navigate commit would make the route
              // effect apply the thread route a second time.
              if (await openThreadFromDeepLink(event.threadId)) {
                desktopRouteStore.syncRoute({
                  kind: "thread",
                  threadId: event.threadId,
                });
              }
              return;
            }
            case "new-thread": {
              const route: DesktopRoute = {
                kind: "new-thread",
                workspacePath: event.workspacePath || null,
                agentId: event.agentId || null,
              };
              await applyDesktopRoute(route);
              desktopRouteStore.syncRoute(route);
              return;
            }
            case "resume-session":
              await waitForGatewayReadyForDeepLink();
              await handleResumeProviderSession(
                event.sessionId,
                event.providerHint,
              );
              return;
            case "open-capsule": {
              const route: DesktopRoute = {
                kind: "capsule",
                capsuleId: event.capsuleId,
              };
              await applyDesktopRoute(route);
              desktopRouteStore.syncRoute(route);
              return;
            }
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
  }, [
    applyDesktopRoute,
    desktopRouteStore,
    handleResumeProviderSession,
    openThreadFromDeepLink,
    pushToast,
  ]);
}
