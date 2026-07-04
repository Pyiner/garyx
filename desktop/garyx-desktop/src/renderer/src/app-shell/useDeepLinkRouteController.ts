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
import { currentDesktopRoute, type DesktopRoute } from "./desktop-route";
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

type UseDeepLinkRouteControllerArgs = {
  capsulePreviewId: string | null;
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
  newThreadDraftActive: boolean;
  openExistingThread: (threadId: string) => Promise<boolean>;
  pendingAgentId: string;
  pendingWorkflowId: string | null;
  pendingWorkspacePath: string | null;
  pushToast: (message: string, tone?: ToastTone, durationMs?: number) => void;
  requestComposerFocus: () => void;
  selectedAutomationId: string | null;
  selectedThreadId: string | null;
  selectedWorkflowRunId: string | null;
  selectedWorkflowTaskId: string | null;
  setCapsulePreviewId: React.Dispatch<React.SetStateAction<string | null>>;
  setConnection: React.Dispatch<React.SetStateAction<ConnectionStatus | null>>;
  setContentView: React.Dispatch<React.SetStateAction<ContentView>>;
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
  setSelectedWorkflowTaskId: React.Dispatch<React.SetStateAction<string | null>>;
  settingsActiveTab: SettingsTabId;
};

export function useDeepLinkRouteController({
  capsulePreviewId,
  clearComposerDraft,
  contentView,
  desktopState,
  desktopRouteStore,
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
}: UseDeepLinkRouteControllerArgs): void {
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

  async function openThreadFromDeepLink(threadId: string): Promise<void> {
    if (!(await ensureThreadOpenable(threadId))) {
      throw new Error(`Thread not found for garyx:// link: ${threadId}`);
    }
    await openExistingThread(threadId);
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
          setSelectedWorkflowRunId(null);
          setContentView("workflow");
          return;
        case "capsule":
          setContentView("capsules");
          setCapsulePreviewId(route.capsuleId);
          return;
        case "view":
          setContentView(route.view);
          // Entering the Capsules gallery from the rail/route clears any open
          // preview so #/capsules shows the gallery, not a stale preview.
          if (route.view === "capsules") {
            setCapsulePreviewId(null);
          }
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
        setSelectedWorkflowTaskId(task.taskId || taskId);
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

  // Batch 4b: the route store is the only hash surface. External edits
  // (manual hash, back/forward) commit in the store first and reach
  // applyDesktopRoute here. While an external route application is in
  // flight, the state-sync effect below is suppressed: its deps change
  // mid-application (e.g. contentView flips) while selectedThreadId still
  // holds the previous thread, and navigating then would counter-write
  // the externally entered hash — the legacy quirk this batch removes.
  // After the application settles, a successful route has converged the
  // state (navigate becomes a no-op) and a failed one changed no synced
  // dep (only the error surface), so the entered hash stays addressable.
  const externalRouteApplicationRef = useRef(false);
  useEffect(() => {
    return desktopRouteStore.subscribeExternal(() => {
      externalRouteApplicationRef.current = true;
      void applyDesktopRoute(desktopRouteStore.getSnapshot().route).finally(
        () => {
          externalRouteApplicationRef.current = false;
        },
      );
    });
  }, [applyDesktopRoute, desktopRouteStore]);

  useEffect(() => {
    if (loading || externalRouteApplicationRef.current) {
      return;
    }
    desktopRouteStore.navigate(
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
        capsulePreviewId,
      }),
      { replace: true },
    );
  }, [
    contentView,
    desktopRouteStore,
    loading,
    newThreadDraftActive,
    pendingAgentId,
    pendingWorkflowId,
    pendingWorkspacePath,
    selectedAutomationId,
    selectedWorkflowTaskId,
    selectedThreadId,
    settingsActiveTab,
    capsulePreviewId,
  ]);

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
            case "open-capsule":
              await applyDesktopRoute({
                kind: "capsule",
                capsuleId: event.capsuleId,
              });
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
}
