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
  DesktopState,
  DesktopThreadSummary,
  ThreadTranscript,
} from "@shared/contracts";
import {
  applyTranscriptRunStateRecord,
  reduceTranscriptRunState,
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
  mergeThread,
  teamBlocksEqual,
  threadSummariesEquivalent,
} from "../thread-model.ts";
import {
  resolveIntentHistoryMatch,
  visibleTranscriptMessages,
} from "./transcript-materialize.ts";
import type { TranscriptMessage } from "@shared/contracts";
import type { TranscriptMapsSnapshot } from "./mirror.ts";

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
  /**
   * The synchronous live-stream shadow for event-path readers (the 3c-1
   * proxy semantics): every live-stream write refreshes it right after
   * the mirror commit.
   */
  liveStreamStateRef: { current: Record<string, LiveStreamState> };
  /**
   * TRANSITIONAL (2b only): the committed side-effect step still triggers
   * the hook-owned rewrite refetch. Slice 2c moves the refetch inside the
   * lifecycle as the single de-duplicated owner and deletes this seam —
   * it is deliberately NOT part of the end-state deps contract.
   */
  refetchAuthoritativeTranscriptAfterRewrite: (
    threadId: string,
  ) => Promise<void>;
}

export class TranscriptLifecycle {
  private port: TranscriptLifecycleMirrorPort;
  private deps: TranscriptLifecycleDeps | null = null;
  // Module-internal orchestration state (plain maps, not React refs).
  private runStateByThread = new Map<string, TranscriptRunState>();
  private titleOverridesByThread: Record<string, string> = {};

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

  // Live-stream writes keep the 3c-1 proxy semantics: mirror commit, then
  // refresh the synchronous shadow.
  private updateLiveStreamState(
    threadId: string,
    updater: (current: LiveStreamState | null) => LiveStreamState | null,
  ): LiveStreamState | null {
    const deps = this.requireDeps();
    const next = this.port.updateThreadLiveStream(threadId, updater);
    deps.liveStreamStateRef.current = this.port.getLiveStreamMap();
    return next;
  }

  private clearLiveStreamState(threadId: string): void {
    this.updateLiveStreamState(threadId, () => null);
  }

  private getLiveStreamState(threadId: string): LiveStreamState | null {
    return this.requireDeps().liveStreamStateRef.current[threadId] || null;
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
      requestSelectedThreadMessagesBottomSnap(threadId, true);
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
    const {
      refetchAuthoritativeTranscriptAfterRewrite,
      requestSelectedThreadMessagesBottomSnap,
      selectedThreadIdRef,
    } = this.requireDeps();
    const threadId = event.threadId;
    if (transcriptRewriteAction(event.message) === "refetch_authoritative") {
      void refetchAuthoritativeTranscriptAfterRewrite(threadId);
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
      requestSelectedThreadMessagesBottomSnap(threadId, true);
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
}
