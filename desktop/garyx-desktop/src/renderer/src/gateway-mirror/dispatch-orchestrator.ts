// Dispatch orchestration domain of the GatewayMirror (endgame architecture
// batch 3c-2): send/steer/interrupt and the queued-batch drain, moved
// verbatim from useMessageDispatchController.ts (sendIntentOnce,
// appendSeededTurn, shiftQueuedIntent, interruptThread,
// markInterruptedAssistantEntries, seededUserBubble,
// presentProviderReadyError) and AppShell.tsx (runQueuedBatch,
// steerQueuedIntent — the T13 TDZ stay-behinds).
//
// Mechanical substitutions against the legacy bodies (review key):
//   1. `window.garyxDesktop.X(...)` → `deps.X(...)` for the five IPC
//      calls (openChatStream, sendStreamingInput, getThreadHistory,
//      interruptThread, checkConnection).
//   2. Every other captured closure name (dispatchMessageState,
//      intentForId, updateLiveStreamState, refs, setters, ...) is
//      destructured once from `deps` at method entry, so the statement
//      bodies stay byte-identical to the legacy functions.
//   3. Per-render captured VALUES (connection, settingsDraft,
//      desktopState, desktopAgents, threadInfoByThread,
//      canSteerQueuedPrompt) are also destructured at entry: the deps
//      object is refreshed every React commit, so an entry-time
//      destructure reproduces the legacy call-time closure capture for
//      the async lifetime of one orchestration call. (A drain loop that
//      spans re-renders reads each send's values at that send's entry —
//      legacy read them from the render that started the drain; the
//      fresher read only affects error-message composition and provider
//      inference, and is an accepted micro-deviation.)
//
// The deps are attached by AppShell through mirror.setDispatchDeps on
// every commit (the streamEventHandlerRef pattern), and dissolve as the
// remaining domains migrate (composer colocation in batch 5, legacy
// deletion in batch 6).

import type {
  ConnectionStatus,
  DesktopApiProviderType,
  DesktopCustomAgent,
  DesktopSettings,
  DesktopState,
  InterruptResult,
  OpenChatStreamResult,
  SendMessageInput,
  SendStreamingInputResult,
  ThreadRuntimeInfo,
  ThreadTranscript,
} from "@shared/contracts";

import {
  isRuntimeBusy,
  selectQueueIntentIds,
  selectThreadRuntime,
  shouldTrackProviderAckAfterStreamInputResponse,
} from "../message-machine.ts";
import type {
  MessageIntent,
  MessageMachineAction,
  MessageMachineState,
  ThreadRuntimeState,
} from "../message-machine.ts";
import { buildOptimisticTranscriptContent } from "../message-rich-content-core.ts";
import { mergeThread } from "../thread-model.ts";
import { isTransientGatewayErrorMessage } from "../app-shell/gateway-errors.ts";
import {
  normalizeMessageText,
  reconcileAssistantEntriesForGatewayRecovery,
  resolveIntentHistoryMatch,
  userMessageIdForOrigin,
} from "./transcript-materialize.ts";
import type {
  LiveStreamState,
  MessageMap,
  TranscriptEntryState,
  UiTranscriptMessage,
} from "../app-shell/types";

export type SeededTurn = {
  assistantEntryId: string | null;
  legacyPendingAssistantId: string | null;
};

/**
 * Everything the orchestration bodies used to reach through React
 * closures. Function members are stable-composition (they read refs or
 * call setState); value members are per-commit snapshots.
 */
/**
 * The mirror-internal surface the dispatch orchestration reaches directly
 * (batch 6b-2d: the machine, live-stream, message, and transcript-accept
 * entries that used to arrive as React-fed deps). The GatewayMirror
 * implements it structurally; keeping it narrow keeps the module
 * node-testable against stubs.
 */
export interface DispatchOrchestratorMirrorPort {
  dispatchMachineAction(action: MessageMachineAction): MessageMachineState;
  getMachineState(): MessageMachineState;
  setThreadRuntimeState(
    threadId: string,
    runtimeState: ThreadRuntimeState,
    options?: {
      activeIntentId?: string;
      remoteRunId?: string;
      error?: string;
    },
  ): void;
  hasPendingHistoryIntents(threadId: string): boolean;
  updateThreadLiveStream(
    threadId: string,
    updater: (current: LiveStreamState | null) => LiveStreamState | null,
  ): LiveStreamState | null;
  getLiveStreamMap(): Record<string, LiveStreamState>;
  updateMessagesByThread(
    updater: (current: MessageMap) => MessageMap,
  ): MessageMap;
  getTranscriptMapsSnapshot(): { messagesByThread: Record<string, readonly UiTranscriptMessage[]> };
  acceptAuthoritativeTranscript(
    threadId: string,
    transcript: ThreadTranscript,
  ): void;
  getThreadTitleOverrides(): Record<string, string>;
}

export interface DispatchOrchestratorDeps {
  // Transcript conveyance.
  scheduleHistoryRefresh: (
    threadId: string,
    attempts?: number,
    delayMs?: number,
    canonical?: boolean,
  ) => void;

  // Root state + UI surfaces.
  setDesktopState: (
    updater: (current: DesktopState | null) => DesktopState | null,
  ) => void;
  setConnection: (status: ConnectionStatus | null) => void;
  setError: (error: string | null) => void;
  recordGatewayStatusObservation: (
    status: ConnectionStatus | null,
    reason?: string | null,
  ) => void;
  requestMessagesBottomSnap: (
    threadId: string | null | undefined,
    forceStick?: boolean,
  ) => void;
  sideChatThreadIdsRef: { current: Set<string> };

  // Per-commit value snapshots.
  connection: ConnectionStatus | null;
  settingsDraft: DesktopSettings;
  desktopState: DesktopState | null;
  desktopAgents: DesktopCustomAgent[];
  threadInfoByThread: Record<string, ThreadRuntimeInfo | null>;
  canSteerQueuedPrompt: boolean;
  inferProviderTypeForThread: (
    threadId: string,
    threadInfoByThread: Record<string, ThreadRuntimeInfo | null>,
    desktopState: DesktopState | null,
    desktopAgents: DesktopCustomAgent[],
  ) => DesktopApiProviderType | null;

  // IPC.
  openChatStream: (input: SendMessageInput) => Promise<OpenChatStreamResult>;
  sendStreamingInput: (
    input: SendMessageInput,
  ) => Promise<SendStreamingInputResult>;
  getThreadHistory: (threadId: string) => Promise<ThreadTranscript>;
  interruptThread: (threadId: string) => Promise<InterruptResult>;
  checkConnection: () => Promise<ConnectionStatus>;
}

function seededUserBubble(intent: MessageIntent): UiTranscriptMessage {
  return {
    id: userMessageIdForOrigin(intent.intentId),
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

export function presentProviderReadyError(
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
  if (providerType === "antigravity") {
    return "Antigravity is not ready on this Mac. Check that the agy CLI is installed, logged in, and available on the Garyx gateway PATH.";
  }
  if (providerType === "traex") {
    return "Traex is not ready on this Mac. Check that the traex CLI is installed, logged in, and available on the Garyx gateway PATH.";
  }
  if (providerType === "claude_code") {
    return "Claude Code is not ready on this Mac. Check the local Claude CLI auth and environment settings.";
  }
  return "The selected provider is not ready on this Mac. Open Status and verify the provider shows Ready.";
}

export class DispatchOrchestrator {
  private port: DispatchOrchestratorMirrorPort;
  private deps: DispatchOrchestratorDeps | null = null;

  constructor(port: DispatchOrchestratorMirrorPort) {
    this.port = port;
  }

  setDeps(deps: DispatchOrchestratorDeps): void {
    this.deps = deps;
  }

  // Batch 6b-2d verbatim shims: the legacy deps-fed helpers, now reading
  // the mirror directly (the AppShell proxies carried these exact bodies).
  private dispatchMessageState(action: MessageMachineAction): void {
    this.port.dispatchMachineAction(action);
  }

  private intentForId(intentId: string): MessageIntent | null {
    return this.port.getMachineState().intentsById[intentId] || null;
  }

  private getLiveStreamState(threadId: string): LiveStreamState | null {
    return this.port.getLiveStreamMap()[threadId] || null;
  }

  private clearLiveStreamState(threadId: string): void {
    this.port.updateThreadLiveStream(threadId, () => null);
  }

  private requireDeps(): DispatchOrchestratorDeps {
    if (!this.deps) {
      throw new Error(
        "DispatchOrchestrator used before dispatch deps were attached",
      );
    }
    return this.deps;
  }

  queueIntentIdsForThread(threadId: string): string[] {
    return selectQueueIntentIds(this.port.getMachineState(), threadId);
  }

  appendSeededTurn(
    threadId: string,
    intent: MessageIntent,
    options?: {
      seedUserBubble?: boolean;
    },
  ): SeededTurn {
    const seedUserBubble = options?.seedUserBubble ?? true;
    const userMessage = seededUserBubble(intent);
    const legacyPendingAssistant =
      ((this.port.getTranscriptMapsSnapshot().messagesByThread as MessageMap)[threadId] || []).find(
        (entry) =>
          entry.role === "assistant" &&
          entry.pending &&
          entry.intentId === intent.intentId,
      ) || null;

    if (seedUserBubble) {
      this.port.updateMessagesByThread((current) => {
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

  shiftQueuedIntent(threadId: string): MessageIntent | null {
    const [nextIntentId] = this.queueIntentIdsForThread(threadId);
    if (!nextIntentId) {
      return null;
    }
    const intent = this.intentForId(nextIntentId);
    if (!intent) {
      this.dispatchMessageState({
        type: "intent/cancelled",
        threadId,
        intentId: nextIntentId,
      });
      return null;
    }
    return intent;
  }

  async sendIntentOnce(
    threadId: string,
    intentId: string,
    options?: {
      seedUserBubble?: boolean;
      seededTurn?: SeededTurn;
    },
  ): Promise<boolean> {
    const deps = this.requireDeps();
    const {
      connection,
      desktopAgents,
      desktopState,
      inferProviderTypeForThread,
      recordGatewayStatusObservation,
      requestMessagesBottomSnap,
      scheduleHistoryRefresh,
      setDesktopState,
      setError,
      settingsDraft,
      sideChatThreadIdsRef,
      threadInfoByThread,
    } = deps;
    const intent = this.intentForId(intentId);
    if (!intent) {
      return false;
    }

    const { assistantEntryId, legacyPendingAssistantId } =
      options?.seededTurn || this.appendSeededTurn(threadId, intent, options);

    this.dispatchMessageState({
      type: "intent/dispatch-started",
      intentId: intent.intentId,
    });
    this.dispatchMessageState({
      type: "intent/awaiting-response",
      intentId: intent.intentId,
    });
    this.port.setThreadRuntimeState(threadId, "dispatching_sync", {
      activeIntentId: intent.intentId,
    });
    this.port.updateThreadLiveStream(threadId, () => ({
      threadId,
      activeIntentId: intent.intentId,
      assistantEntryId,
      pendingAckIntentIds: [],
      streamStatus: "connecting",
    }));

    setError(null);
    requestMessagesBottomSnap(threadId, true);

    try {
      const result = await deps.openChatStream({
        threadId,
        clientIntentId: intent.intentId,
        message: intent.text,
        images: intent.images,
        files: intent.files,
      });
      const resultThreadId = result.threadId || result.sessionId || threadId;
      if (result.status === "accepted") {
        this.port.updateThreadLiveStream(resultThreadId, (current) =>
          current
            ? {
                ...current,
                runId: result.runId,
                streamStatus: current.streamStatus,
              }
            : {
                threadId: resultThreadId,
                runId: result.runId,
                activeIntentId: intent.intentId,
                assistantEntryId,
                pendingAckIntentIds: [],
                streamStatus: "connecting",
              },
        );
        const latestIntent = this.intentForId(intent.intentId);
        if (
          latestIntent &&
          ![
            "remote_accepted",
            "awaiting_provider_ack",
            "awaiting_history",
            "completed",
          ].includes(latestIntent.state)
        ) {
          this.dispatchMessageState({
            type: "intent/remote-accepted",
            intentId: intent.intentId,
            runId: result.runId,
            threadId: resultThreadId,
            removeFromQueue: false,
          });
        }
        setDesktopState((current) => {
          if (!current) {
            return current;
          }
          const titleOverride = this.port.getThreadTitleOverrides()[resultThreadId];
          const resultThread = titleOverride
            ? { ...result.thread, title: titleOverride }
            : result.thread;
          return {
            ...current,
            threads: mergeThread(current.threads, resultThread),
            sessions: mergeThread(current.threads, resultThread),
          };
        });
        scheduleHistoryRefresh(resultThreadId, 2, 1200, false);
        return true;
      }
      const liveState = this.getLiveStreamState(resultThreadId);
      if (!liveState?.runId && result.runId) {
        this.port.updateThreadLiveStream(resultThreadId, (current) =>
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
      const latestIntent = this.intentForId(intent.intentId);
      if (
        latestIntent &&
        ![
          "remote_accepted",
          "awaiting_provider_ack",
          "awaiting_history",
          "completed",
        ].includes(latestIntent.state)
      ) {
        this.dispatchMessageState({
          type: "intent/remote-accepted",
          intentId: intent.intentId,
          runId: result.runId,
          threadId: resultThreadId,
          responseText: result.response,
          removeFromQueue: false,
        });
      }
      this.dispatchMessageState({
        type: "intent/awaiting-history",
        intentId: intent.intentId,
        responseText: result.response,
      });
      this.port.setThreadRuntimeState(threadId, "reconciling_history", {
        activeIntentId: intent.intentId,
        remoteRunId: result.runId,
      });

      setDesktopState((current) => {
        if (!current) {
          return current;
        }
        const titleOverride = this.port.getThreadTitleOverrides()[resultThreadId];
        const resultThread = titleOverride
          ? { ...result.thread, title: titleOverride }
          : result.thread;
        if (sideChatThreadIdsRef.current.has(resultThread.id)) {
          return {
            ...current,
            threads: current.threads.filter(
              (thread) => thread.id !== resultThread.id,
            ),
            sessions: current.sessions.filter(
              (session) => session.id !== resultThread.id,
            ),
          };
        }
        return {
          ...current,
          threads: mergeThread(current.threads, resultThread),
          sessions: mergeThread(current.threads, resultThread),
        };
      });

      const transcript =
        await deps.getThreadHistory(resultThreadId);
      const intentSnapshot = this.intentForId(intent.intentId) || {
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
        this.port.acceptAuthoritativeTranscript(resultThreadId, transcript);
      } else {
        if (
          legacyPendingAssistantId &&
          !result.response &&
          result.status === "completed"
        ) {
          this.port.updateMessagesByThread((current) => ({
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

      this.clearLiveStreamState(resultThreadId);

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
      const liveState = this.getLiveStreamState(threadId);
      const failedIntentId = liveState?.activeIntentId || intent.intentId;
      const recoveryResult = reconcileAssistantEntriesForGatewayRecovery(
        (this.port.getTranscriptMapsSnapshot().messagesByThread as MessageMap)[threadId] || [],
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
        this.clearLiveStreamState(threadId);
        this.dispatchMessageState({
          type: "intent/awaiting-history",
          intentId: failedIntentId,
          responseText: intent.responseText,
        });
        this.port.setThreadRuntimeState(threadId, "reconciling_history", {
          activeIntentId: failedIntentId,
          remoteRunId: liveState?.runId,
        });
        this.port.updateMessagesByThread((current) => ({
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

      this.clearLiveStreamState(threadId);
      setError(message);
      this.dispatchMessageState({
        type: interrupted ? "intent/interrupted" : "intent/failed",
        intentId: failedIntentId,
        ...(interrupted ? { error: message } : { error: message }),
      });
      this.port.setThreadRuntimeState(threadId, interrupted ? "interrupting" : "failed", {
        activeIntentId: failedIntentId,
        error: message,
      });
      this.port.updateMessagesByThread((current) => ({
        ...current,
        [threadId]: (() => {
          const existing = current[threadId] || [];
          let assistantUpdated = false;
          const next = existing.map((entry) => {
            if (
              entry.role === "user" &&
              entry.intentId === failedIntentId &&
              entry.localState !== "remote_final"
            ) {
              return {
                ...entry,
                error: true,
                localState: errorState,
              };
            }
            const isTargetAssistant =
              entry.role === "assistant" &&
              entry.intentId === failedIntentId &&
              (entry.pending ||
                entry.id === legacyPendingAssistantId ||
                entry.id === liveState?.assistantEntryId);
            if (!isTargetAssistant) {
              return entry;
            }
            assistantUpdated = true;
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
          if (assistantUpdated) {
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

  async runQueuedBatch(threadId: string, initialIntentId?: string): Promise<void> {
    const deps = this.requireDeps();
    const {
      setConnection,
      setError,
    } = deps;
    const firstIntentId = initialIntentId || "";
    if (!firstIntentId && this.queueIntentIdsForThread(threadId).length === 0) {
      return;
    }

    setError(null);

    try {
      let nextIntentId = firstIntentId;
      let dispatchedFromQueue = false;
      let seededTurn: SeededTurn | undefined;

      while (nextIntentId || this.queueIntentIdsForThread(threadId).length > 0) {
        seededTurn = undefined;
        if (!nextIntentId) {
          const currentQueuedIntent = this.shiftQueuedIntent(threadId);
          nextIntentId = currentQueuedIntent?.intentId || "";
          dispatchedFromQueue = true;
          if (!currentQueuedIntent || !nextIntentId) {
            break;
          }
          seededTurn = this.appendSeededTurn(threadId, currentQueuedIntent);
          this.dispatchMessageState({
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

        const didSucceed = await this.sendIntentOnce(threadId, nextIntentId, {
          seededTurn,
        });
        if (!didSucceed) {
          if (dispatchedFromQueue) {
            this.dispatchMessageState({
              type: "intent/requeue-front",
              threadId,
              intentId: nextIntentId,
              source: "queue_send",
              error: this.intentForId(nextIntentId)?.error,
            });
          }
          break;
        }
        const runtime = selectThreadRuntime(this.port.getMachineState(), threadId);
        if (runtime && isRuntimeBusy(runtime.state)) {
          break;
        }
        nextIntentId = "";
      }
    } finally {
      if (!this.port.hasPendingHistoryIntents(threadId)) {
        this.dispatchMessageState({
          type: "thread/clear",
          threadId,
        });
      }
      const status = await deps.checkConnection();
      setConnection(status);
    }
  }

  async steerQueuedIntent(
    latestIntent: MessageIntent,
    options?: { canSteer?: boolean },
  ): Promise<void> {
    const deps = this.requireDeps();
    const {
      canSteerQueuedPrompt,
      requestMessagesBottomSnap,
      setError,
    } = deps;
    const threadId = latestIntent.threadId;
    if (!(options?.canSteer ?? canSteerQueuedPrompt)) {
      return;
    }
    if (latestIntent.state !== "queued_local") {
      return;
    }

    this.dispatchMessageState({
      type: "intent/request-dispatch",
      threadId: threadId,
      intentId: latestIntent.intentId,
      mode: "async_steer",
      source: "queue_steer",
      removeFromQueue: false,
    });
    this.dispatchMessageState({
      type: "intent/dispatch-started",
      intentId: latestIntent.intentId,
    });

    setError(null);
    requestMessagesBottomSnap(threadId, true);
    const optimisticRunId =
      this.getLiveStreamState(threadId)?.runId ||
      selectThreadRuntime(this.port.getMachineState(), threadId)?.remoteRunId ||
      `stream:${threadId}`;
    this.port.updateThreadLiveStream(threadId, (current) => {
      const pendingAckIntentIds = current?.pendingAckIntentIds || [];
      return {
        threadId,
        runId: current?.runId || optimisticRunId,
        activeIntentId: current?.activeIntentId,
        assistantEntryId: current?.assistantEntryId ?? null,
        pendingAckIntentIds: pendingAckIntentIds.includes(latestIntent.intentId)
          ? pendingAckIntentIds
          : [...pendingAckIntentIds, latestIntent.intentId],
        streamStatus: current?.streamStatus || "connecting",
      };
    });

    try {
      const result = await deps.sendStreamingInput({
        threadId,
        clientIntentId: latestIntent.intentId,
        message: latestIntent.text,
        images: latestIntent.images,
        files: latestIntent.files,
      });
      const resultThreadId = result.threadId || result.sessionId || threadId;
      if (result.status === "queued") {
        const activeRunId =
          this.getLiveStreamState(resultThreadId)?.runId ||
          selectThreadRuntime(this.port.getMachineState(), resultThreadId)
            ?.remoteRunId ||
          `stream:${resultThreadId}`;
        const intentBeforeAccept = this.intentForId(latestIntent.intentId);
        const shouldTrackProviderAck =
          shouldTrackProviderAckAfterStreamInputResponse(intentBeforeAccept);
        this.dispatchMessageState({
          type: "intent/remote-accepted",
          intentId: latestIntent.intentId,
          runId: activeRunId,
          threadId: resultThreadId,
          pendingInputId: result.pendingInputId,
          removeFromQueue: true,
          awaitProviderAck: true,
        });
        this.port.updateThreadLiveStream(resultThreadId, (current) => ({
          threadId: resultThreadId,
          runId: current?.runId || activeRunId,
          activeIntentId: current?.activeIntentId,
          assistantEntryId: current?.assistantEntryId ?? null,
          pendingAckIntentIds: (
            current?.pendingAckIntentIds || []
          ).includes(latestIntent.intentId)
            ? current?.pendingAckIntentIds || []
            : shouldTrackProviderAck
              ? [...(current?.pendingAckIntentIds || []), latestIntent.intentId]
              : current?.pendingAckIntentIds || [],
          streamStatus: current?.streamStatus || "connecting",
        }));
        return;
      }

      this.port.updateThreadLiveStream(threadId, (current) =>
        current
          ? {
              ...current,
              pendingAckIntentIds: current.pendingAckIntentIds.filter(
                (entry) => entry !== latestIntent.intentId,
              ),
            }
          : current,
      );
      this.dispatchMessageState({
        type: "intent/request-dispatch",
        threadId: threadId,
        intentId: latestIntent.intentId,
        mode: "sync_send",
        source: "queue_steer",
        removeFromQueue: true,
      });
      this.dispatchMessageState({
        type: "intent/dispatch-started",
        intentId: latestIntent.intentId,
      });
      const didSucceed = await this.sendIntentOnce(threadId, latestIntent.intentId, {
        seedUserBubble: true,
      });
      if (!didSucceed) {
        this.dispatchMessageState({
          type: "intent/requeue-front",
          threadId: threadId,
          intentId: latestIntent.intentId,
          source: "queue_steer",
          error: this.intentForId(latestIntent.intentId)?.error,
        });
      }
    } catch (steerError) {
      this.port.updateThreadLiveStream(threadId, (current) =>
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
      this.dispatchMessageState({
        type: "intent/requeue-front",
        threadId: threadId,
        intentId: latestIntent.intentId,
        source: "queue_steer",
        error: message,
      });
    }
  }

  private markInterruptedAssistantEntries(
    threadId: string,
    intentIds: string[],
    activeAssistantEntryId?: string | null,
  ): void {
    if (!intentIds.length) {
      return;
    }
    const interruptedIntentIds = new Set(intentIds);
    this.port.updateMessagesByThread((current) => ({
      ...current,
      [threadId]: (current[threadId] || []).map((entry) => {
        if (
          entry.role === "user" &&
          entry.intentId &&
          interruptedIntentIds.has(entry.intentId) &&
          entry.localState !== "remote_final"
        ) {
          return {
            ...entry,
            error: true,
            localState: "interrupted",
          };
        }
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

  async interruptThread(threadId: string | null | undefined): Promise<void> {
    const deps = this.requireDeps();
    const {
      scheduleHistoryRefresh,
      setConnection,
    } = deps;
    if (!threadId) {
      return;
    }

    const runtime = selectThreadRuntime(this.port.getMachineState(), threadId);
    const hasLocalBusyRuntime = Boolean(
      runtime && isRuntimeBusy(runtime.state),
    );
    if (runtime && hasLocalBusyRuntime) {
      const liveState = this.getLiveStreamState(threadId);
      const interruptedIntentIds = [
        runtime.activeIntentId,
        ...(liveState?.pendingAckIntentIds || []),
      ].filter((intentId, index, intents): intentId is string => {
        return Boolean(intentId) && intents.indexOf(intentId) === index;
      });

      this.port.setThreadRuntimeState(threadId, "interrupting", {
        activeIntentId: runtime.activeIntentId,
        remoteRunId: runtime.remoteRunId,
      });
      for (const intentId of interruptedIntentIds) {
        this.dispatchMessageState({
          type: "intent/interrupted",
          intentId,
          error: "request interrupted",
        });
      }
      this.markInterruptedAssistantEntries(
        threadId,
        interruptedIntentIds,
        liveState?.assistantEntryId ?? null,
      );
    }

    await deps.interruptThread(threadId);
    if (hasLocalBusyRuntime) {
      this.clearLiveStreamState(threadId);
      this.dispatchMessageState({
        type: "thread/clear",
        threadId: threadId,
      });
    }
    scheduleHistoryRefresh(threadId, 2, 500);
    const status = await deps.checkConnection();
    setConnection(status);
  }
}
