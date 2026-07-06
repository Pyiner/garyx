// TranscriptLifecycle: the transcript transport orchestration that used to
// live in useTranscriptController (endgame architecture batch 6b-2,
// docs/design/appshell-transcript-dissolve.md). Slice 2a moves the machine
// bookkeeping and the run-state chain; slice 2b moves the apply chain
// (persist, session cache, title/team propagation, intent marking) behind
// the accept* high-level entries; the fetch/stream lifecycle follows in 2c.
//
// Pattern: dispatch-orchestrator's — a class whose React seams arrive
// through setDeps (refreshed every React commit); each entry point
// destructures the deps it needs once (the legacy closure capture).
// Mirror-internal state (machine, live streams, transport snapshot) is
// reached through the narrow MirrorPort the GatewayMirror implements —
// this module never touches React or the route store.

import type {
  CachedThreadTranscript,
  ConnectionStatus,
  DesktopSettings,
  DesktopState,
  DesktopThreadSummary,
  GetThreadHistoryInput,
  StartThreadStreamInput,
  StopThreadStreamInput,
  ThreadTranscript,
} from "@shared/contracts";
import {
  applyTranscriptRunStateRecord,
  decideTranscriptFetchPageAction,
  isThreadStreamGapError,
  mergeForwardTranscriptPage,
  reduceTranscriptRunState,
  shouldRefetchAuthoritativeAfterForwardPageLimit,
  shouldRestartSelectedThreadStreamAfterRefetch,
  streamResumeCursor,
  transcriptCommittedAfterCursor,
  transcriptControlKind,
  transcriptForCommittedCache,
  transcriptRewriteAction,
  transcriptWithResolvedActiveRun,
  type TranscriptRunState,
} from "../../../shared/transcript-sync.ts";

import {
  findPendingAckIntentIndex,
  selectThreadRuntime,
  type MessageIntent,
  type MessageMachineAction,
  type MessageMachineState,
  type ThreadRuntimeState,
} from "../message-machine.ts";
import type {
  LiveStreamState,
  MessageMap,
  UiTranscriptMessage,
} from "../app-shell/types";
import type {
  CommittedMessageEvent,
  DesktopChatStreamEvent,
  RenderState,
} from "@shared/contracts";
import {
  isKnownThreadId,
  mergeThread,
  teamBlocksEqual,
  threadSummariesEquivalent,
} from "../thread-model.ts";
import {
  chatStreamEventHasRunLifecycle,
  isMissingThreadTranscript,
  reconcileAssistantEntriesForGatewayRecovery,
  resolveIntentHistoryMatch,
  transcriptHasAutomationResponse,
  visibleTranscriptMessages,
  THREAD_HISTORY_PAGE_SIZE,
  THREAD_HISTORY_USER_QUERY_LIMIT,
} from "./transcript-materialize.ts";
import { isTransientGatewayErrorMessage } from "../app-shell/gateway-errors.ts";
import type { TranscriptMessage } from "@shared/contracts";
import type { PendingAutomationRun } from "../app-shell/types";
import type { TranscriptMapsSnapshot } from "./mirror.ts";

export const SELECTED_THREAD_STREAM_CONSUMER_ID = "selected-thread";

const THREAD_HISTORY_FORWARD_PAGE_LIMIT = 50;

/**
 * The mirror-internal surface the lifecycle orchestrates over. The
 * GatewayMirror implements it; keeping it narrow documents exactly which
 * mirror domains the lifecycle touches (machine, live streams, transport
 * snapshot) and keeps the module node-testable against stubs.
 */
export interface TranscriptLifecycleMirrorPort {
  dispatchMachineAction(action: MessageMachineAction): MessageMachineState;
  getMachineState(): MessageMachineState;
  updateThreadLiveStream(
    threadId: string,
    updater: (current: LiveStreamState | null) => LiveStreamState | null,
  ): LiveStreamState | null;
  getLiveStreamMap(): Record<string, LiveStreamState>;
  getThreadSnapshotTranscript(threadId: string): ThreadTranscript | null;
  // Slice 2b: the pure cache-only commits the accept* entries wrap, the
  // aggregate maps the local-write bridge and the persist ride-along read,
  // and the transcript-cache persistence IPC (mirror-injected service).
  applyAuthoritativeTranscript(
    threadId: string,
    transcript: ThreadTranscript,
  ): void;
  applyRemoteTranscript(threadId: string, transcript: ThreadTranscript): void;
  getTranscriptMapsSnapshot(): TranscriptMapsSnapshot;
  syncThreadUiMessages(
    threadId: string,
    messages: readonly UiTranscriptMessage[],
  ): void;
  persistTranscriptCache(
    transcript: ThreadTranscript,
    renderState: RenderState | null,
  ): void;
  // Slice 2c: the fetch/stream lifecycle's transport surface — commit-side
  // mirror entries plus the injected IPC services (all resolved by the
  // GatewayMirror; the lifecycle stays window-free).
  ingest(event: DesktopChatStreamEvent): void;
  clearThreadTranscript(threadId: string): void;
  fetchOlderThreadHistoryPage(
    threadId: string,
    options?: { onPageFetched?: (transcript: ThreadTranscript) => void },
  ): Promise<void>;
  startThreadStream(input: StartThreadStreamInput): Promise<void>;
  stopThreadStream(input: StopThreadStreamInput): Promise<void>;
  loadThreadTranscriptCache(
    threadId: string,
  ): Promise<CachedThreadTranscript | null>;
  clearThreadTranscriptCache(threadId: string): Promise<void>;
  getThreadHistoryFull(threadId: string): Promise<ThreadTranscript>;
  getThreadHistoryPage(input: GetThreadHistoryInput): Promise<ThreadTranscript>;
}

/**
 * The React seams slice 2a needs (the full contract grows with 2b/2c per
 * the design). Refreshed every React commit by the wiring layer.
 */
export interface TranscriptLifecycleDeps {
  setDesktopState: (
    updater: (current: DesktopState | null) => DesktopState | null,
  ) => void;
  /** Colocated title root handle (not-editing guard lives in the root). */
  syncThreadTitleDraft: (nextTitle: string) => void;
  requestSelectedThreadMessagesBottomSnap: (
    threadId: string | null | undefined,
    forceStick?: boolean,
  ) => void;
  selectedThreadIdRef: { readonly current: string | null };
  // Slice 2c: the fetch/stream lifecycle's remaining React seams.
  setError: (error: string | null) => void;
  setHistoryLoading: (loading: boolean) => void;
  setPendingAutomationRun: (
    threadId: string,
    run: PendingAutomationRun | null,
  ) => void;
  recordGatewayStatusObservation: (
    status: ConnectionStatus | null,
    reason?: string | null,
  ) => void;
  scheduleDesktopStateRefresh: (delayMs?: number) => void;
  scheduleHistoryRefresh: (
    threadId: string,
    attempts?: number,
    delayMs?: number,
    canonical?: boolean,
  ) => void;
  /** Per-commit snapshots (stream recovery's gatewayUrl fallback). */
  connection: ConnectionStatus | null;
  settingsDraft: DesktopSettings;
  desktopState: DesktopState | null;
  /**
   * The AppShell refresh wrapper (mirror IPC round + legacy React-state
   * synchronization) — ensureThreadOpenable's known-thread re-check.
   */
  refreshDesktopState: () => Promise<DesktopState>;
  /** Selection/scroll shadows the lifecycle reads (UI-owned by design). */
  selectedThreadGenerationRef: { readonly current: number };
  lastRenderedMessageThreadRef: { readonly current: string | null };
  messagesRef: { readonly current: HTMLDivElement | null };
  pendingMessagesPrependAnchorRef: {
    current: {
      threadId: string;
      scrollHeight: number;
      scrollTop: number;
    } | null;
  };
  /**
   * Side-chat stream identity (the refetch restart must also re-arm an
   * open side chat on the same thread; side-chat keeps its own colocation
   * cut, these two seams only mirror its stream identity).
   */
  sideChatThreadIdRef: { readonly current: string | null };
  sideChatStreamConsumerId: (threadId: string) => string;
}

export class TranscriptLifecycle {
  private port: TranscriptLifecycleMirrorPort;
  private deps: TranscriptLifecycleDeps | null = null;
  // Module-internal orchestration state (plain maps, not React refs).
  private runStateByThread = new Map<string, TranscriptRunState>();
  private titleOverridesByThread: Record<string, string> = {};
  // Slice 2c: per-thread operation tokens for the selected-thread load
  // (generation counters; cancel invalidates by bumping) and the in-flight
  // de-dupe for the single-owner rewrite refetch.
  private selectedLoadGenerationByThread = new Map<string, number>();
  private refetchInFlightByThread = new Map<string, Promise<void>>();

  constructor(port: TranscriptLifecycleMirrorPort) {
    this.port = port;
  }

  setDeps(deps: TranscriptLifecycleDeps): void {
    this.deps = deps;
  }

  private requireDeps(): TranscriptLifecycleDeps {
    if (!this.deps) {
      throw new Error("TranscriptLifecycle deps not attached");
    }
    return this.deps;
  }

  /**
   * Remote-title overrides the dispatch orchestrator consults to avoid
   * clobbering server titles (read facade per the design; the map itself
   * is module-internal).
   */
  getThreadTitleOverrides(): Record<string, string> {
    return this.titleOverridesByThread;
  }

  intentForId(intentId: string): MessageIntent | null {
    return this.port.getMachineState().intentsById[intentId] || null;
  }

  setThreadRuntimeState(
    threadId: string,
    runtimeState: ThreadRuntimeState,
    options?: {
      activeIntentId?: string;
      remoteRunId?: string;
      error?: string;
    },
  ): void {
    this.port.dispatchMachineAction({
      type: "thread/runtime",
      threadId,
      runtimeState,
      activeIntentId: options?.activeIntentId,
      remoteRunId: options?.remoteRunId,
      error: options?.error,
    });
  }

  hasPendingHistoryIntents(threadId: string): boolean {
    return Object.values(this.port.getMachineState().intentsById).some(
      (intent) => {
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
      },
    );
  }

  // 6b-2d: the mirror's live-stream map is read directly (the AppShell
  // shadow ref became a getter over getLiveStreamMap, so there is nothing
  // left to feed).
  private updateLiveStreamState(
    threadId: string,
    updater: (current: LiveStreamState | null) => LiveStreamState | null,
  ): LiveStreamState | null {
    return this.port.updateThreadLiveStream(threadId, updater);
  }

  private clearLiveStreamState(threadId: string): void {
    this.updateLiveStreamState(threadId, () => null);
  }

  private getLiveStreamState(threadId: string): LiveStreamState | null {
    return this.port.getLiveStreamMap()[threadId] || null;
  }

  private applyThreadTitleUpdate(threadId: string, title: string): void {
    const { setDesktopState, syncThreadTitleDraft, selectedThreadIdRef } =
      this.requireDeps();
    const nextTitle = title.trim();
    if (!threadId || !nextTitle) {
      return;
    }

    this.titleOverridesByThread = {
      ...this.titleOverridesByThread,
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

    if (selectedThreadIdRef.current === threadId) {
      syncThreadTitleDraft(nextTitle);
    }
  }

  private publishTranscriptRunState(
    threadId: string,
    state: TranscriptRunState,
  ): TranscriptRunState {
    this.runStateByThread.set(threadId, state);
    if (state.title) {
      this.applyThreadTitleUpdate(threadId, state.title);
    }
    const remoteRunId = state.activeRunId || undefined;
    if (state.busy) {
      const runtimeState: ThreadRuntimeState =
        state.activity === "reconciling"
          ? "reconciling_history"
          : "running_remote";
      this.updateLiveStreamState(threadId, (current) => ({
        threadId,
        runId: remoteRunId || current?.runId,
        activeIntentId: current?.activeIntentId,
        assistantEntryId: current?.assistantEntryId ?? null,
        pendingAckIntentIds: current?.pendingAckIntentIds || [],
        streamStatus:
          state.activity === "reconciling" ? "reconciling" : "streaming",
      }));
      this.setThreadRuntimeState(threadId, runtimeState, {
        activeIntentId: this.getLiveStreamState(threadId)?.activeIntentId,
        remoteRunId,
      });
      return state;
    }
    if (state.terminalStatus) {
      this.updateLiveStreamState(threadId, (current) =>
        current
          ? {
              ...current,
              runId: current.runId || remoteRunId,
              assistantEntryId: null,
              streamStatus:
                state.terminalStatus === "interrupted"
                  ? "interrupted"
                  : "reconciling",
            }
          : null,
      );
      if (!this.hasPendingHistoryIntents(threadId)) {
        this.port.dispatchMachineAction({
          type: "thread/clear",
          threadId,
        });
        this.clearLiveStreamState(threadId);
      }
    }
    return state;
  }

  syncTranscriptRunState(
    threadId: string,
    transcript: ThreadTranscript,
  ): TranscriptRunState {
    return this.publishTranscriptRunState(
      threadId,
      reduceTranscriptRunState(transcript.messages),
    );
  }

  applyCommittedTranscriptRunState(event: {
    threadId: string;
    seq: number;
    message: CommittedMessageEvent["message"];
  }): TranscriptRunState {
    // The reduce fallback is initialization-only: the committed stream
    // always starts after the first transcript apply (which seeds the
    // run-state map through syncTranscriptRunState). The mirror snapshot
    // read keeps a sane base if that ever changes.
    const current =
      this.runStateByThread.get(event.threadId) ||
      reduceTranscriptRunState(
        this.port.getThreadSnapshotTranscript(event.threadId)?.messages || [],
      );
    return this.publishTranscriptRunState(
      event.threadId,
      applyTranscriptRunStateRecord(current, event.message, {
        seq: event.seq,
      }),
    );
  }

  markIntentsFromHistory(
    threadId: string,
    transcript: TranscriptMessage[],
  ): void {
    const visibleTranscript = visibleTranscriptMessages(transcript);
    const intents = Object.values(
      this.port.getMachineState().intentsById,
    ).filter((intent) => {
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
    });

    for (const intent of intents) {
      const match = resolveIntentHistoryMatch(intent, visibleTranscript);
      if (!match.userVisible) {
        continue;
      }
      if (
        match.assistantVisible ||
        (!intent.responseText && intent.dispatchMode === "async_steer")
      ) {
        this.port.dispatchMachineAction({
          type: "intent/completed",
          intentId: intent.intentId,
        });
      } else {
        this.port.dispatchMachineAction({
          type: "intent/awaiting-history",
          intentId: intent.intentId,
          responseText: intent.responseText,
        });
      }
    }

    const runtime = selectThreadRuntime(this.port.getMachineState(), threadId);
    if (runtime && !this.hasPendingHistoryIntents(threadId)) {
      this.port.dispatchMachineAction({
        type: "thread/clear",
        threadId,
      });
      const liveStream = this.getLiveStreamState(threadId);
      if (
        liveStream &&
        ["reconciling", "disconnected", "failed"].includes(
          liveStream.streamStatus,
        )
      ) {
        this.clearLiveStreamState(threadId);
      }
    }
  }

  applyUserAck(
    threadId: string,
    runId: string,
    pendingInputId?: string,
  ): void {
    const { requestSelectedThreadMessagesBottomSnap } = this.requireDeps();
    let nextIntentId: string | undefined;
    const acknowledgedPendingInputId = pendingInputId?.trim() || "";
    this.updateLiveStreamState(threadId, (current) => {
      const pendingAckIntentIds = [...(current?.pendingAckIntentIds || [])];
      const matchedIndex = findPendingAckIntentIndex(
        pendingAckIntentIds,
        acknowledgedPendingInputId,
        this.port.getMachineState().intentsById,
      );
      if (matchedIndex >= 0) {
        nextIntentId = pendingAckIntentIds[matchedIndex];
        pendingAckIntentIds.splice(matchedIndex, 1);
      } else {
        nextIntentId = undefined;
      }
      const nextPendingAckIntentIds = nextIntentId
        ? pendingAckIntentIds.filter((intentId) => intentId !== nextIntentId)
        : pendingAckIntentIds;
      return current
        ? {
            ...current,
            runId,
            activeIntentId: nextIntentId || current.activeIntentId,
            assistantEntryId: null,
            pendingAckIntentIds: nextPendingAckIntentIds,
            streamStatus: "streaming",
          }
        : null;
    });
    if (nextIntentId) {
      const acknowledgedIntent = this.intentForId(nextIntentId);
      this.port.dispatchMachineAction({
        type: "intent/awaiting-history",
        intentId: nextIntentId,
        responseText: acknowledgedIntent?.responseText,
      });
      // Queued-intent pickup is not a fresh user action: snap only while
      // the viewport is already following the bottom, never yank a reader
      // who scrolled up (they have the scroll-to-latest button instead).
      requestSelectedThreadMessagesBottomSnap(threadId, false);
      this.setThreadRuntimeState(threadId, "running_remote", {
        activeIntentId: nextIntentId,
        remoteRunId: runId,
      });
    }
  }

  forceReleaseThreadRuntime(threadId: string): void {
    const pendingStates = [
      "dispatching",
      "remote_accepted",
      "awaiting_provider_ack",
      "awaiting_response",
      "awaiting_history",
    ];
    for (const intent of Object.values(
      this.port.getMachineState().intentsById,
    )) {
      if (intent.threadId === threadId && pendingStates.includes(intent.state)) {
        this.port.dispatchMachineAction({
          type: "intent/completed",
          intentId: intent.intentId,
        });
      }
    }
    this.port.dispatchMachineAction({
      type: "thread/clear",
      threadId,
    });
    const liveStream = this.getLiveStreamState(threadId);
    if (
      liveStream &&
      ["reconciling", "disconnected", "failed"].includes(
        liveStream.streamStatus,
      )
    ) {
      this.clearLiveStreamState(threadId);
    }
  }

  // ---- Slice 2b: the apply chain -----------------------------------------

  /**
   * Local-write bridge: optimistic and recovery writes still run through
   * the legacy-shaped updater; per-thread diffs commit into the mirror,
   * which notifies the read side. Remote applies never come through here —
   * the mirror computes those itself (applyRemote/applyAuthoritative/
   * applyOlderPage).
   */
  updateMessagesByThread(
    updater: (current: MessageMap) => MessageMap,
  ): MessageMap {
    const previous = this.port.getTranscriptMapsSnapshot()
      .messagesByThread as MessageMap;
    const next = updater(previous);
    if (next !== previous) {
      for (const threadId of Object.keys(next)) {
        if (next[threadId] !== previous[threadId]) {
          this.port.syncThreadUiMessages(threadId, next[threadId]);
        }
      }
      // Deleted keys (e.g. the new-thread draft promoted to a real thread)
      // sync as an empty array so the mirror drops the stale rows too.
      for (const threadId of Object.keys(previous)) {
        if (!(threadId in next)) {
          this.port.syncThreadUiMessages(threadId, []);
        }
      }
    }
    return next;
  }

  // The snapshot itself lives in the mirror's transcript cache (batch
  // 6b-1, getThreadSnapshotTranscript); this keeps the run-state sync and
  // the disk-cache persistence that ride along with every apply.
  private rememberTranscriptSnapshot(
    threadId: string,
    transcript: ThreadTranscript,
    persist = true,
    syncRunState = true,
  ): void {
    if (syncRunState) {
      this.syncTranscriptRunState(threadId, transcript);
    }
    if (persist) {
      const cacheTranscript = transcriptForCommittedCache(transcript);
      if (
        cacheTranscript.messages.length > 0 ||
        !transcript.threadInfo?.activeRun
      ) {
        // Persist the last render snapshot alongside committed messages so
        // the next cold/offline open can render folded history before a
        // live frame.
        this.port.persistTranscriptCache(
          cacheTranscript,
          this.port.getTranscriptMapsSnapshot().renderStateByThread[threadId] ??
            null,
        );
      }
    }
  }

  /**
   * Authoritative-apply high-level entry: runs the pure cache commit plus
   * the ride-alongs (run-state sync, cache persistence, intent marking).
   */
  acceptAuthoritativeTranscript(
    threadId: string,
    transcript: ThreadTranscript,
    options?: { syncRunState?: boolean },
  ): void {
    this.port.applyAuthoritativeTranscript(threadId, transcript);
    const resolvedTranscript = transcriptWithResolvedActiveRun(transcript);
    this.rememberTranscriptSnapshot(
      threadId,
      resolvedTranscript,
      true,
      options?.syncRunState ?? true,
    );
    this.markIntentsFromHistory(
      threadId,
      visibleTranscriptMessages(resolvedTranscript.messages),
    );
  }

  private threadSummaryFromTranscript(
    threadId: string,
    transcript: ThreadTranscript,
  ): DesktopThreadSummary {
    if (transcript.thread) {
      return {
        ...transcript.thread,
        agentId:
          transcript.thread.agentId ?? transcript.threadInfo?.agentId ?? null,
        workspacePath:
          transcript.thread.workspacePath ??
          transcript.threadInfo?.workspacePath ??
          null,
        worktree:
          transcript.thread.worktree ?? transcript.threadInfo?.worktree ?? null,
        team: transcript.thread.team ?? transcript.team ?? null,
      };
    }

    const timestamps = transcript.messages
      .map((message) => message.timestamp || "")
      .filter(Boolean);
    const fallbackTimestamp =
      timestamps[timestamps.length - 1] || new Date().toISOString();
    const preview =
      transcript.messages.find((message) => message.text.trim())?.text.trim() ||
      "";

    return {
      id: threadId,
      title: transcript.threadInfo?.agentId || threadId,
      createdAt: timestamps[0] || fallbackTimestamp,
      updatedAt: fallbackTimestamp,
      lastMessagePreview: preview,
      workspacePath: transcript.threadInfo?.workspacePath ?? null,
      messageCount:
        transcript.pageInfo?.totalMessages ?? transcript.messages.length,
      agentId: transcript.threadInfo?.agentId ?? null,
      recentRunId: transcript.threadInfo?.activeRun?.runId ?? null,
      worktree: transcript.threadInfo?.worktree ?? null,
      team: transcript.team ?? null,
    };
  }

  private cacheOpenableTranscriptThread(
    threadId: string,
    transcript: ThreadTranscript,
  ): void {
    const { setDesktopState } = this.requireDeps();
    const summary = this.threadSummaryFromTranscript(threadId, transcript);
    setDesktopState((current) => {
      if (!current || current.threads.some((thread) => thread.id === threadId)) {
        return current;
      }
      // Hidden threads (side chats, child threads) live only in `sessions`,
      // so this cache write runs on every transcript application. Re-writing
      // an equivalent summary must keep `desktopState` identity stable, or
      // history-loading effects keyed on it re-fire and loop.
      const existing = current.sessions.find(
        (session) => session.id === threadId,
      );
      if (existing && threadSummariesEquivalent(existing, summary)) {
        return current;
      }
      return {
        ...current,
        sessions: mergeThread(current.sessions, summary),
      };
    });
  }

  /**
   * Remote-apply high-level entry: runs the pure cache commit plus the
   * ride-alongs (run-state sync, cache persistence, desktopState session
   * cache, team propagation, intent marking).
   */
  acceptRemoteTranscript(
    threadId: string,
    transcript: ThreadTranscript,
    options?: {
      persist?: boolean;
      syncRunState?: boolean;
      /**
       * Set by the committed-stream path, whose events already reached the
       * mirror through ingest — applying the folded transcript again would
       * apply the same data twice per event.
       */
      mirrorAlreadyApplied?: boolean;
    },
  ): void {
    const { setDesktopState } = this.requireDeps();
    if (!options?.mirrorAlreadyApplied) {
      this.port.applyRemoteTranscript(threadId, transcript);
    }
    const resolvedTranscript = transcriptWithResolvedActiveRun(transcript);
    this.rememberTranscriptSnapshot(
      threadId,
      resolvedTranscript,
      options?.persist !== false,
      options?.syncRunState ?? true,
    );
    this.cacheOpenableTranscriptThread(threadId, resolvedTranscript);
    // Propagate the transcript's `team` block into `desktopState.threads[i]`
    // so team-bound threads render the team badge + sub-agent peek tabs as
    // soon as the thread metadata endpoint has confirmed the binding. Without
    // this merge, a list summary (which may have been fetched before the
    // first turn) could shadow the richer detail payload, leaving the UI
    // stuck on the plain agent label. Only write when the block is present
    // and different from what's already cached — idempotent updates must
    // not churn React identity and re-trigger dependent effects.
    if (resolvedTranscript.team !== undefined) {
      setDesktopState((current) => {
        if (!current) {
          return current;
        }
        const nextTeam = resolvedTranscript.team ?? null;
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
    this.markIntentsFromHistory(
      threadId,
      visibleTranscriptMessages(resolvedTranscript.messages),
    );
  }

  /**
   * Committed side-effect step: the per-event machine/run-state/ack pass
   * that follows the mirror's ingest fold of a committed message.
   */
  applyCommittedThreadMessage(
    event: Extract<DesktopChatStreamEvent, { type: "committed_message" }>,
  ): void {
    const { requestSelectedThreadMessagesBottomSnap, selectedThreadIdRef } =
      this.requireDeps();
    const threadId = event.threadId;
    if (transcriptRewriteAction(event.message) === "refetch_authoritative") {
      void this.refetchAuthoritativeTranscriptAfterRewrite(threadId);
      return;
    }
    this.applyCommittedTranscriptRunState(event);
    // The mirror's ingest (stream-listener top) already folded this event
    // into its snapshot transcript — that fold is the legacy
    // committedMessageForwardPage result (batch 6b-1 single snapshot).
    const merged = this.port.getThreadSnapshotTranscript(threadId);
    if (!merged) {
      return;
    }
    if (selectedThreadIdRef.current === threadId) {
      // Passive committed events must not yank a reader who scrolled up;
      // a non-forced snap still keeps a bottom-following viewport pinned.
      requestSelectedThreadMessagesBottomSnap(threadId, false);
    }
    this.acceptRemoteTranscript(threadId, merged, {
      syncRunState: false,
      mirrorAlreadyApplied: true,
    });
    const controlKind = transcriptControlKind(event.message);
    if (controlKind === "user_ack") {
      const control =
        event.message.content &&
        typeof event.message.content === "object" &&
        !Array.isArray(event.message.content)
          ? (event.message.content as { control?: Record<string, unknown> })
              .control
          : null;
      this.applyUserAck(
        threadId,
        event.runId,
        typeof control?.pending_input_id === "string"
          ? control.pending_input_id
          : typeof control?.pendingInputId === "string"
            ? control.pendingInputId
            : undefined,
      );
    }
  }

  // ---- Slice 2c: the fetch/stream lifecycle -------------------------------

  /**
   * Stream-listener entry: ingest the event into the mirror first (one
   * atomic commit), then run the machine/run-state/error side effects the
   * legacy handler owned.
   */
  notifyStreamEvent(event: DesktopChatStreamEvent): void {
    const { scheduleDesktopStateRefresh } = this.requireDeps();
    this.port.ingest(event);
    if (chatStreamEventHasRunLifecycle(event)) {
      scheduleDesktopStateRefresh();
    }
    this.handleChatStreamEvent(event);
  }

  private handleChatStreamEvent(event: DesktopChatStreamEvent): void {
    const {
      recordGatewayStatusObservation,
      scheduleHistoryRefresh,
      setError,
      connection,
      settingsDraft,
    } = this.requireDeps();
    const threadId = event.threadId;
    if (event.type === "thread_render_frame") {
      // The ingest above already committed the frame (events + render
      // snapshot, monotonic guard included). This pass keeps the per-event
      // machine/run-state/ack side effects.
      for (const committed of event.events) {
        this.applyCommittedThreadMessage(committed);
      }
      return;
    }
    if (event.type !== "error") {
      return;
    }
    const currentStream = this.getLiveStreamState(threadId);
    const activeIntentId = currentStream?.activeIntentId;

    if (isThreadStreamGapError(event)) {
      if (activeIntentId) {
        this.port.dispatchMachineAction({
          type: "intent/awaiting-history",
          intentId: activeIntentId,
        });
      }
      this.updateLiveStreamState(threadId, (current) =>
        current
          ? {
              ...current,
              runId: event.runId,
              assistantEntryId: null,
              streamStatus: "reconciling",
            }
          : null,
      );
      this.setThreadRuntimeState(threadId, "reconciling_history", {
        activeIntentId: activeIntentId || undefined,
        remoteRunId: event.runId,
      });
      void this.refetchAuthoritativeTranscriptAfterRewrite(threadId);
      return;
    }
    const recoveryResult = activeIntentId
      ? reconcileAssistantEntriesForGatewayRecovery(
          (this.port.getTranscriptMapsSnapshot().messagesByThread as MessageMap)[
            threadId
          ] || [],
          activeIntentId,
          [currentStream?.assistantEntryId],
        )
      : { entries: [] as UiTranscriptMessage[], matched: false };
    const isTerminalRunError = event.terminal === true;
    if (
      !isTerminalRunError &&
      (isTransientGatewayErrorMessage(event.error) || recoveryResult.matched)
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
      this.updateLiveStreamState(threadId, (current) => {
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
        this.port.dispatchMachineAction({
          type: "intent/awaiting-history",
          intentId: activeIntentId,
        });
      }
      this.setThreadRuntimeState(threadId, "reconciling_history", {
        activeIntentId: activeIntentId || undefined,
        remoteRunId: event.runId,
      });
      if (activeIntentId) {
        this.updateMessagesByThread((current) => {
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
      return;
    }
    this.updateLiveStreamState(threadId, (current) =>
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
      this.port.dispatchMachineAction({
        type: "intent/failed",
        intentId: activeIntentId,
        error: event.error,
      });
    }
    this.setThreadRuntimeState(threadId, "failed", {
      activeIntentId: activeIntentId || undefined,
      remoteRunId: event.runId,
      error: event.error,
    });
    setError(event.error);
  }

  /**
   * Single-owner authoritative refetch after a committed rewrite/reset (or
   * a stream gap). Concurrent triggers for the same thread coalesce into
   * one in-flight fetch + stream restart.
   */
  refetchAuthoritativeTranscriptAfterRewrite(threadId: string): Promise<void> {
    const inFlight = this.refetchInFlightByThread.get(threadId);
    if (inFlight) {
      return inFlight;
    }
    const run = this.runAuthoritativeRefetch(threadId).finally(() => {
      this.refetchInFlightByThread.delete(threadId);
    });
    this.refetchInFlightByThread.set(threadId, run);
    return run;
  }

  private async runAuthoritativeRefetch(threadId: string): Promise<void> {
    // Ingest can trigger this before the wiring layer attaches deps (e.g.
    // deps-less contract-test mirrors replaying rewrite controls). With no
    // React seams there is nothing to refresh or retry — skip, never throw.
    if (!this.deps) {
      return;
    }
    const {
      requestSelectedThreadMessagesBottomSnap,
      scheduleHistoryRefresh,
      selectedThreadGenerationRef,
      selectedThreadIdRef,
      sideChatThreadIdRef,
      sideChatStreamConsumerId,
    } = this.requireDeps();
    const startSelectionGeneration = selectedThreadGenerationRef.current;
    try {
      await this.port.clearThreadTranscriptCache(threadId);
      const transcript = await this.port.getThreadHistoryFull(threadId);
      if (selectedThreadIdRef.current === threadId) {
        requestSelectedThreadMessagesBottomSnap(threadId, true);
      }
      this.acceptRemoteTranscript(threadId, transcript);
      const shouldRestartSelectedStream =
        shouldRestartSelectedThreadStreamAfterRefetch({
          threadId,
          selectedThreadId: selectedThreadIdRef.current,
          startSelectionGeneration,
          currentSelectionGeneration: selectedThreadGenerationRef.current,
        });
      if (shouldRestartSelectedStream) {
        await this.startCommittedThreadStream(
          threadId,
          transcript,
          SELECTED_THREAD_STREAM_CONSUMER_ID,
        );
      }
      if (sideChatThreadIdRef.current === threadId) {
        await this.startCommittedThreadStream(
          threadId,
          transcript,
          sideChatStreamConsumerId(threadId),
        );
      }
    } catch {
      scheduleHistoryRefresh(threadId, 3, 500, true);
    }
  }

  async startCommittedThreadStream(
    threadId: string,
    transcript: ThreadTranscript,
    consumerId: string,
  ): Promise<void> {
    await this.port.startThreadStream({
      threadId,
      consumerId,
      afterSeq: streamResumeCursor({
        afterCursor: transcriptCommittedAfterCursor(transcript),
        fallbackMaxIndex: null,
      }),
    });
  }

  /**
   * Older-page load with the UI-owned scroll-anchor capture between fetch
   * and apply, and error surfacing (the pure fetch/guard/apply lives on
   * the mirror as fetchOlderThreadHistoryPage).
   */
  async loadOlderThreadHistoryPage(threadId: string): Promise<void> {
    const {
      messagesRef,
      pendingMessagesPrependAnchorRef,
      selectedThreadIdRef,
      setError,
    } = this.requireDeps();
    try {
      await this.port.fetchOlderThreadHistoryPage(threadId, {
        onPageFetched: (transcript) => {
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
        },
      });
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
    }
  }

  /**
   * Openability gate for an existing thread id (moved verbatim from
   * AppShell, 6b-2d): known in the current desktop state, or known after
   * one refresh, or fetchable as a non-missing transcript (which is then
   * accepted so the thread renders immediately once selected).
   */
  async ensureThreadOpenable(threadId: string): Promise<boolean> {
    const { desktopState, refreshDesktopState } = this.requireDeps();
    if (isKnownThreadId(desktopState, threadId)) {
      return true;
    }

    const refreshedState = await refreshDesktopState();
    if (isKnownThreadId(refreshedState, threadId)) {
      return true;
    }

    const transcript = await this.port.getThreadHistoryFull(threadId);
    if (isMissingThreadTranscript(transcript)) {
      return false;
    }

    this.acceptRemoteTranscript(threadId, transcript);
    return true;
  }

  /**
   * Selected-thread transcript load behind a per-thread operation token:
   * a newer load (or cancelSelectedThreadLoad) invalidates every pending
   * state landing of the superseded run.
   */
  loadSelectedThreadTranscript(threadId: string): Promise<void> {
    const generation =
      (this.selectedLoadGenerationByThread.get(threadId) ?? 0) + 1;
    this.selectedLoadGenerationByThread.set(threadId, generation);
    const isCancelled = () =>
      this.selectedLoadGenerationByThread.get(threadId) !== generation;
    return this.loadSelectedThreadTranscriptFromSingleSource(
      threadId,
      isCancelled,
    );
  }

  /**
   * The React selected-thread effect's cleanup: invalidate the operation
   * token and stop the selected-thread stream consumer (exactly the legacy
   * effect-local `cancelled` + cleanup semantics).
   */
  cancelSelectedThreadLoad(threadId: string): void {
    this.selectedLoadGenerationByThread.set(
      threadId,
      (this.selectedLoadGenerationByThread.get(threadId) ?? 0) + 1,
    );
    void this.port.stopThreadStream({
      threadId,
      consumerId: SELECTED_THREAD_STREAM_CONSUMER_ID,
    });
  }

  /// Incremental forward fetch for the selected thread. `authoritative: true`
  /// marks a full server refetch (no cache / reset / shrink / page-limit
  /// overflow) whose transcript must replace local state verbatim;
  /// `authoritative: false` marks an incremental aggregate that the caller
  /// must forward-merge onto the live snapshot, because the committed stream
  /// may have advanced it past this fetch's tail while pages were in flight.
  private async fetchSelectedThreadIncrementalTranscript(
    threadId: string,
    cached: ThreadTranscript | null,
    isCancelled: () => boolean,
  ): Promise<{ transcript: ThreadTranscript; authoritative: boolean }> {
    let current = cached;
    let cursor = transcriptCommittedAfterCursor(current);
    if (!current || cursor === null) {
      return {
        transcript: await this.port.getThreadHistoryFull(threadId),
        authoritative: true,
      };
    }

    let pagesFetched = 0;
    let latestHasMoreAfter = false;
    for (
      let pageCount = 0;
      pageCount < THREAD_HISTORY_FORWARD_PAGE_LIMIT;
      pageCount += 1
    ) {
      const page = await this.port.getThreadHistoryPage({
        threadId,
        afterIndex: cursor,
        limit: THREAD_HISTORY_PAGE_SIZE,
        userQueryLimit: THREAD_HISTORY_USER_QUERY_LIMIT,
      });
      if (isCancelled()) {
        return { transcript: current, authoritative: false };
      }
      pagesFetched = pageCount + 1;
      const action = decideTranscriptFetchPageAction({
        cursor,
        reset: page.pageInfo?.reset,
        hasMoreAfter: page.pageInfo?.hasMoreAfter,
        totalMessagesInThread: page.pageInfo?.totalMessages,
      });
      if (action.type === "reset" || action.type === "shrink_refetch") {
        await this.port.clearThreadTranscriptCache(threadId);
        return {
          transcript: await this.port.getThreadHistoryFull(threadId),
          authoritative: true,
        };
      }

      current = mergeForwardTranscriptPage(current, page);
      latestHasMoreAfter = action.continuePaging;
      if (!action.continuePaging) {
        return { transcript: current, authoritative: false };
      }
      const nextCursor =
        page.pageInfo?.nextAfterIndex ?? transcriptCommittedAfterCursor(current);
      if (nextCursor === null || nextCursor <= cursor) {
        return { transcript: current, authoritative: false };
      }
      cursor = nextCursor;
    }
    if (isCancelled()) {
      return { transcript: current, authoritative: false };
    }
    if (
      shouldRefetchAuthoritativeAfterForwardPageLimit({
        pagesFetched,
        maxPages: THREAD_HISTORY_FORWARD_PAGE_LIMIT,
        hasMoreAfter: latestHasMoreAfter,
      })
    ) {
      await this.port.clearThreadTranscriptCache(threadId);
      return {
        transcript: await this.port.getThreadHistoryFull(threadId),
        authoritative: true,
      };
    }
    return { transcript: current, authoritative: false };
  }

  private async loadSelectedThreadTranscriptFromSingleSource(
    threadId: string,
    isCancelled: () => boolean,
  ): Promise<void> {
    const {
      lastRenderedMessageThreadRef,
      requestSelectedThreadMessagesBottomSnap,
      setError,
      setHistoryLoading,
      setPendingAutomationRun,
    } = this.requireDeps();
    const hasRenderedThread = lastRenderedMessageThreadRef.current === threadId;
    const hasCachedMessages =
      (
        (this.port.getTranscriptMapsSnapshot().messagesByThread as MessageMap)[
          threadId
        ] || []
      ).length > 0;
    requestSelectedThreadMessagesBottomSnap(
      threadId,
      !hasRenderedThread || !hasCachedMessages,
    );

    setHistoryLoading(true);
    setError(null);
    let latestTranscript: ThreadTranscript | null =
      this.port.getThreadSnapshotTranscript(threadId);
    let streamReady = false;
    let streamStarted = false;
    try {
      const cached = await this.port.loadThreadTranscriptCache(threadId);
      if (isCancelled()) {
        return;
      }
      if (cached) {
        latestTranscript = cached.transcript;
        this.acceptRemoteTranscript(threadId, cached.transcript, {
          persist: false,
        });
        // Restore the offline render snapshot so folded history renders before
        // the live stream's first frame arrives. The mirror's render snapshot
        // only advances through ingested frames, so replay the cached snapshot
        // as a synthesized snapshot-only frame (same wire semantics; the
        // monotonic guard applies).
        if (cached.renderState) {
          this.port.ingest({
            type: "thread_render_frame",
            threadId,
            events: [],
            renderState: cached.renderState,
          });
        }
        // Start the committed stream from the cached cursor right away: its
        // replay plus first render frame is what shows turns committed while
        // this client wasn't subscribed. Waiting for the incremental HTTP
        // fetch below kept the restored (possibly stale) render snapshot on
        // screen for the whole fetch, hiding those turns.
        await this.startCommittedThreadStream(
          threadId,
          cached.transcript,
          SELECTED_THREAD_STREAM_CONSUMER_ID,
        );
        streamStarted = true;
      }

      const fetched = await this.fetchSelectedThreadIncrementalTranscript(
        threadId,
        latestTranscript,
        isCancelled,
      );
      if (isCancelled()) {
        return;
      }
      // Batch 4b: a selected-but-missing thread (externally entered
      // #/thread/<id> that stays addressable) must not be applied or
      // streamed — the gateway history responds remoteFound:false with an
      // empty transcript, and the stream endpoint would 404-retry forever.
      // The predicate is shared with ensureThreadOpenable. It gates on the
      // AUTHORITATIVE fetch result, so a stale persisted cache for a
      // deleted thread lands here too: the cached fast path above already
      // applied it and started the stream, so roll both back — stop the
      // stream, drop the persisted cache, and clear the applied state
      // (mirror.clearThreadTranscript resets all five transcript maps plus
      // the committed records/frontier in one commit).
      if (
        fetched.authoritative &&
        isMissingThreadTranscript(fetched.transcript)
      ) {
        if (streamStarted) {
          await this.port.stopThreadStream({
            threadId,
            consumerId: SELECTED_THREAD_STREAM_CONSUMER_ID,
          });
          streamStarted = false;
        }
        if (latestTranscript) {
          void this.port.clearThreadTranscriptCache(threadId);
          this.port.clearThreadTranscript(threadId);
          latestTranscript = null;
        }
        setError(`Thread not found: ${threadId}`);
        return;
      }
      requestSelectedThreadMessagesBottomSnap(threadId, true);
      // The stream may have advanced the live snapshot past this fetch's tail
      // while pages were in flight; forward-merge keeps that progress. An
      // authoritative refetch (reset/shrink) intentionally replaces state.
      latestTranscript = fetched.authoritative
        ? fetched.transcript
        : mergeForwardTranscriptPage(
            this.port.getThreadSnapshotTranscript(threadId),
            fetched.transcript,
          );
      this.acceptRemoteTranscript(threadId, latestTranscript);
      if (transcriptHasAutomationResponse(latestTranscript.messages)) {
        setPendingAutomationRun(threadId, null);
      }
      streamReady = true;
    } catch (historyError) {
      if (!latestTranscript) {
        setError(
          historyError instanceof Error
            ? historyError.message
            : "Failed to load thread history",
        );
      } else {
        setError(
          historyError instanceof Error
            ? `Failed to sync latest thread history: ${historyError.message}`
            : "Failed to sync latest thread history",
        );
      }
    } finally {
      if (!isCancelled()) {
        setHistoryLoading(false);
        if (!(streamStarted || !streamReady || !latestTranscript)) {
          await this.startCommittedThreadStream(
            threadId,
            latestTranscript,
            SELECTED_THREAD_STREAM_CONSUMER_ID,
          );
        }
      }
    }
  }
}
