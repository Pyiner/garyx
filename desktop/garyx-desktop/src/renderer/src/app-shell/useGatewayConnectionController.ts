import { startTransition, useEffect, useRef, useState } from "react";

import type {
  ConnectionStatus,
  DesktopCustomAgent,
  DesktopSettings,
  DesktopState,
  DesktopTeam,
  DesktopWorkflowDefinition,
} from "@shared/contracts";

import type { Translate } from "../i18n";
import {
  selectThreadRuntime,
  type MessageMachineState,
} from "../message-machine";
import type { ToastTone } from "../toast";
import { isTransientGatewayErrorMessage } from "./gateway-errors";
import type { LiveStreamState } from "./types";

const GATEWAY_HEALTHY_POLL_MS = 12000;
const SILENT_DESKTOP_STATE_REFRESH_MS = 60000;
const RUN_STATE_LIST_REFRESH_DEBOUNCE_MS = 350;
const GATEWAY_READY_STATE_REFRESH_THROTTLE_MS = 12000;
const GATEWAY_RETRY_BACKOFF_MS = [2500, 4000, 6500, 10000, 15000];

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

type UseGatewayConnectionControllerArgs = {
  connection: ConnectionStatus | null;
  desktopState: DesktopState | null;
  error: string | null;
  gatewaySettingsStatus: string | null;
  gatewaySetupMessageForAuthError: (
    message: string | null | undefined,
  ) => string | null;
  liveStreamStateRef: React.MutableRefObject<Record<string, LiveStreamState>>;
  loading: boolean;
  messageStateRef: React.MutableRefObject<MessageMachineState>;
  pushToast: (message: string, tone?: ToastTone, durationMs?: number) => void;
  scheduleHistoryRefresh: (
    threadId: string,
    attempts?: number,
    delayMs?: number,
    canonical?: boolean,
  ) => void;
  selectedThreadId: string | null;
  selectedThreadIdRef: React.MutableRefObject<string | null>;
  setConnection: React.Dispatch<React.SetStateAction<ConnectionStatus | null>>;
  setDesktopAgents: React.Dispatch<React.SetStateAction<DesktopCustomAgent[]>>;
  setDesktopState: React.Dispatch<React.SetStateAction<DesktopState | null>>;
  setDesktopTeams: React.Dispatch<React.SetStateAction<DesktopTeam[]>>;
  setDesktopWorkflows: React.Dispatch<
    React.SetStateAction<DesktopWorkflowDefinition[]>
  >;
  setError: React.Dispatch<React.SetStateAction<string | null>>;
  setGatewaySettingsStatus: React.Dispatch<
    React.SetStateAction<string | null>
  >;
  setLocalSettingsStatus: React.Dispatch<React.SetStateAction<string | null>>;
  setSettingsDraft: React.Dispatch<React.SetStateAction<DesktopSettings>>;
  settingsDraft: DesktopSettings;
  t: Translate;
};

export function useGatewayConnectionController({
  connection,
  desktopState,
  error,
  gatewaySettingsStatus,
  gatewaySetupMessageForAuthError,
  liveStreamStateRef,
  loading,
  messageStateRef,
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
}: UseGatewayConnectionControllerArgs) {
  const [gatewayStatusHint, setGatewayStatusHint] = useState<string | null>(
    "Connecting to gateway…",
  );
  const [gatewayFailureCount, setGatewayFailureCount] = useState(0);
  const [gatewaySetupForced, setGatewaySetupForced] = useState(false);
  const [gatewaySetupCanCancel, setGatewaySetupCanCancel] = useState(false);
  const gatewayRetryStepRef = useRef(0);
  const gatewaySetupSavedConnectionRef = useRef<ConnectionStatus | null>(null);
  const previousConnectionOkRef = useRef<boolean | null>(null);
  const desktopStateRefreshTimeoutRef = useRef<number | null>(null);
  const lastGatewayReadyStateRefreshAtRef = useRef(0);

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
        gatewayHeaders: savedSettings.gatewayHeaders,
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
      gatewayHeaders: savedSettings.gatewayHeaders,
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
    recordGatewayStatusObservation(connection, connection?.error);
  }, [connection]);

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

  function scheduleDesktopStateRefresh(delayMs = RUN_STATE_LIST_REFRESH_DEBOUNCE_MS) {
    if (desktopStateRefreshTimeoutRef.current !== null) {
      window.clearTimeout(desktopStateRefreshTimeoutRef.current);
    }
    desktopStateRefreshTimeoutRef.current = window.setTimeout(() => {
      desktopStateRefreshTimeoutRef.current = null;
      void refreshDesktopState().catch((refreshError) => {
        console.debug("Desktop state refresh failed.", refreshError);
      });
    }, delayMs);
  }

  function scheduleGatewayReadyStateRefresh() {
    const now = Date.now();
    if (
      now - lastGatewayReadyStateRefreshAtRef.current <
      GATEWAY_READY_STATE_REFRESH_THROTTLE_MS
    ) {
      return;
    }
    lastGatewayReadyStateRefreshAtRef.current = now;
    scheduleDesktopStateRefresh(0);
  }

  useEffect(() => {
    return () => {
      if (desktopStateRefreshTimeoutRef.current !== null) {
        window.clearTimeout(desktopStateRefreshTimeoutRef.current);
        desktopStateRefreshTimeoutRef.current = null;
      }
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
          scheduleGatewayReadyStateRefresh();
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
        // Defensive: the persistent main-process stream may have silently died
        // while hidden; re-fetch the open thread's canonical transcript so it
        // converges to the server's latest state on return, instead of relying
        // solely on the connection never stopping (#TASK-1449 symptom 2).
        const openThreadId = selectedThreadIdRef.current;
        if (openThreadId) {
          scheduleHistoryRefresh(openThreadId, 1, 0, true);
        }
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

  return {
    gatewayFailureCount,
    gatewaySetupCanCancel,
    gatewaySetupForced,
    gatewaySetupSavedConnectionRef,
    gatewayStatusHint,
    handleCancelGatewaySetup,
    handleOpenGatewaySetup,
    hasGatewayRecoveryActivity,
    recordGatewayStatusObservation,
    refreshDesktopState,
    scheduleDesktopStateRefresh,
    setGatewaySetupCanCancel,
    setGatewaySetupForced,
  };
}
