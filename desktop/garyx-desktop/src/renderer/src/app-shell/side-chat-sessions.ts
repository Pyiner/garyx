// SideChatSessions: the shell-owned side-chat session store (endgame
// batch 5b-7a, docs/design/appshell-sidechat-colocation.md). Sessions
// must outlive the inspector dock: per-source side-thread bindings
// (sessionStorage-persisted, session scope on purpose), composer drafts,
// the attachment-upload composer lock, creating/error transients, and
// the in-flight creation de-dupe all live here. The dispatch-orchestrator
// and transcript-lifecycle deps read the store's shadow refs, so side
// threads keep their orchestration semantics while the panel is hidden.
//
// uSES contract (the mirror's snapshot rules): getSnapshot() returns a
// cached object reference, rebuilt only after a write bumps the version.

import type {
  BrowserAnnotationCommentRequest,
  MessageFileAttachment,
  MessageImageAttachment,
} from "@shared/contracts";

export type SideComposerDraft = {
  text: string;
  textPresent: boolean;
  images: MessageImageAttachment[];
  files: MessageFileAttachment[];
  browserAnnotations: BrowserAnnotationCommentRequest[];
  resetKey: number;
};

export function emptySideComposerDraft(): SideComposerDraft {
  return {
    text: "",
    textPresent: false,
    images: [],
    files: [],
    browserAnnotations: [],
    resetKey: 0,
  };
}

function sideChatThreadStorageKey(
  gatewayScope: string,
  sourceThreadId: string,
): string {
  // Partitioned by gateway scope: thread ids are only unique per gateway,
  // so a binding persisted under gateway A must never be adopted on B.
  return `garyx.side-tools.side-chat-thread.${gatewayScope}.${sourceThreadId}`;
}

function readPersistedSideChatThreadId(
  gatewayScope: string,
  sourceThreadId: string,
): string | null {
  if (typeof window === "undefined") {
    return null;
  }
  try {
    return (
      window.sessionStorage.getItem(
        sideChatThreadStorageKey(gatewayScope, sourceThreadId),
      ) || null
    );
  } catch {
    return null;
  }
}

function persistSideChatThreadId(
  gatewayScope: string,
  sourceThreadId: string,
  sideThreadId: string,
) {
  if (typeof window === "undefined") {
    return;
  }
  try {
    window.sessionStorage.setItem(
      sideChatThreadStorageKey(gatewayScope, sourceThreadId),
      sideThreadId,
    );
  } catch {
    // Session storage may be unavailable (private modes); the in-memory
    // binding still works for this app session.
  }
}

/**
 * The scope-current projection of one source thread's side-chat state.
 * EVERY consumer (AppShell's shell-owned effects AND the panel's own
 * subscription) must derive through {@link scopedSideChatView}: when the
 * snapshot's owning scope does not match the current render's gateway key
 * (a switch frame before the store transition effect), the view is the
 * EMPTY universe — no binding, no draft, no transient — never the previous
 * gateway's state (thread ids are only unique per gateway).
 */
export interface ScopedSideChatView {
  readonly scopeCurrent: boolean;
  readonly threadId: string | null;
  readonly draft: SideComposerDraft;
  readonly creating: boolean;
  readonly error: string | null;
}

export function scopedSideChatView(
  snapshot: SideChatSessionsSnapshot,
  gatewayKey: string,
  sourceThreadId: string | null,
): ScopedSideChatView {
  const scopeCurrent = snapshot.gatewayScope === gatewayKey;
  if (!scopeCurrent || !sourceThreadId) {
    return {
      scopeCurrent,
      threadId: null,
      draft: emptySideComposerDraft(),
      creating: false,
      error: null,
    };
  }
  return {
    scopeCurrent,
    threadId: snapshot.threadBySource[sourceThreadId] || null,
    draft: snapshot.composerBySource[sourceThreadId] || emptySideComposerDraft(),
    creating: Boolean(snapshot.creatingBySource[sourceThreadId]),
    error: snapshot.errorBySource[sourceThreadId] || null,
  };
}

export interface SideChatSessionsSnapshot {
  readonly version: number;
  /** The gateway scope that OWNS every binding in this snapshot. Render
   *  derivations must require it to match the current gateway key: a
   *  mismatched frame (scope transition not yet applied) renders an empty
   *  side-chat universe instead of the previous gateway's bindings. */
  readonly gatewayScope: string;
  /** Connection generation of {@link gatewayScope}; effect/subscription
   *  identities include it so A -> B -> A re-keys everything. */
  readonly scopeGeneration: number;
  readonly threadBySource: Readonly<Record<string, string>>;
  readonly composerBySource: Readonly<Record<string, SideComposerDraft>>;
  readonly creatingBySource: Readonly<Record<string, boolean>>;
  readonly errorBySource: Readonly<Record<string, string>>;
  readonly attachmentUploadCount: number;
  readonly historyLoading: boolean;
}

export class SideChatSessions {
  /** Normalized gateway URL partitioning the persisted bindings. Written
   *  only through {@link setGatewayScope}. */
  private gatewayScopeValue = "";
  /** Connection generation: increments on EVERY scope transition, so
   *  A -> B -> A yields three distinct values — a late async completion
   *  from the first A can never pass a generation gate after returning
   *  to A. */
  private scopeGenerationValue = 0;
  private threadBySource: Record<string, string> = {};
  private composerBySource: Record<string, SideComposerDraft> = {};
  private creatingBySource: Record<string, boolean> = {};
  private errorBySource: Record<string, string> = {};
  private creationPromiseBySource: Record<string, Promise<string | null>> = {};
  private attachmentUploadCount = 0;
  private historyLoading = false;

  // Orchestration shadows (dispatch-orchestrator ignores side threads in
  // queue routing; the transcript lifecycle re-arms the ACTIVE side
  // thread's stream after a refetch). Kept in sync on every write.
  readonly sideChatThreadIdRef: { current: string | null } = { current: null };
  readonly sideChatThreadIdsRef: { current: Set<string> } = {
    current: new Set(),
  };

  private activeSource: string | null = null;
  private version = 0;
  private snapshot: SideChatSessionsSnapshot | null = null;
  private listeners = new Set<() => void>();

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  getSnapshot = (): SideChatSessionsSnapshot => {
    if (!this.snapshot) {
      this.snapshot = {
        version: this.version,
        gatewayScope: this.gatewayScopeValue,
        scopeGeneration: this.scopeGenerationValue,
        threadBySource: this.threadBySource,
        composerBySource: this.composerBySource,
        creatingBySource: this.creatingBySource,
        errorBySource: this.errorBySource,
        attachmentUploadCount: this.attachmentUploadCount,
        historyLoading: this.historyLoading,
      };
    }
    return this.snapshot;
  };

  private commit(): void {
    this.version += 1;
    this.snapshot = null;
    for (const listener of this.listeners) {
      listener();
    }
  }

  private syncShadows(): void {
    this.sideChatThreadIdsRef.current = new Set(
      Object.values(this.threadBySource),
    );
    this.sideChatThreadIdRef.current = this.activeSource
      ? this.threadBySource[this.activeSource] || null
      : null;
  }

  /** The per-thread committed-stream consumer identity. Includes the
   *  CURRENT scope generation: a stream started on generation N is stopped
   *  by the exact same id, and a post-transition start can never collide
   *  with (or be torn down by) a previous generation's lifecycle. */
  streamConsumerId(threadId: string): string {
    return `side-chat:g${this.scopeGenerationValue}:${threadId}`;
  }

  setActiveSource(sourceThreadId: string | null): void {
    if (this.activeSource === sourceThreadId) {
      return;
    }
    this.activeSource = sourceThreadId;
    this.syncShadows();
    // Shadow-only change: the active-source flip re-derives through the
    // consumers' own render inputs, so no snapshot bump is needed — but
    // restoring a persisted binding below may bump.
  }

  threadFor(sourceThreadId: string | null): string | null {
    return sourceThreadId
      ? this.threadBySource[sourceThreadId] || null
      : null;
  }

  get gatewayScope(): string {
    return this.gatewayScopeValue;
  }

  get scopeGeneration(): number {
    return this.scopeGenerationValue;
  }

  /**
   * Formal scope transition. In-memory bindings, drafts, in-flight
   * creation promises, errors, and the shadow refs are all owned by ONE
   * gateway scope — thread ids are only unique per gateway, so switching
   * gateways clears the whole domain and publishes a fresh snapshot.
   * Late async completions from the previous scope compare against this
   * value and become no-ops.
   */
  setGatewayScope(scope: string): void {
    if (this.gatewayScopeValue === scope) {
      return;
    }
    this.gatewayScopeValue = scope;
    this.scopeGenerationValue += 1;
    this.threadBySource = {};
    this.composerBySource = {};
    this.creatingBySource = {};
    this.errorBySource = {};
    this.creationPromiseBySource = {};
    this.attachmentUploadCount = 0;
    this.historyLoading = false;
    this.sideChatThreadIdRef.current = null;
    this.sideChatThreadIdsRef.current = new Set();
    this.commit();
  }

  /** Bind a side thread to a source thread (+ sessionStorage write). */
  rememberThread(sourceThreadId: string, sideThreadId: string): void {
    if (this.threadBySource[sourceThreadId] === sideThreadId) {
      return;
    }
    this.threadBySource = {
      ...this.threadBySource,
      [sourceThreadId]: sideThreadId,
    };
    persistSideChatThreadId(this.gatewayScopeValue, sourceThreadId, sideThreadId);
    this.syncShadows();
    this.commit();
  }

  /**
   * sessionStorage read-through: adopt a persisted binding for a source
   * that has none in memory. Returns the effective binding.
   */
  restorePersisted(sourceThreadId: string): string | null {
    const existing = this.threadBySource[sourceThreadId];
    if (existing) {
      return existing;
    }
    const persisted = readPersistedSideChatThreadId(
      this.gatewayScopeValue,
      sourceThreadId,
    );
    if (!persisted) {
      return null;
    }
    this.threadBySource = {
      ...this.threadBySource,
      [sourceThreadId]: persisted,
    };
    this.syncShadows();
    this.commit();
    return persisted;
  }

  /**
   * Drop a binding that failed its openability check (the side thread was
   * deleted server-side). Guarded by the expected id so a concurrent
   * rebind is not clobbered. The sessionStorage entry stays — matching
   * the legacy behavior, whose catch also only cleared the in-memory map.
   */
  forgetThread(sourceThreadId: string, expectedThreadId: string): void {
    if (this.threadBySource[sourceThreadId] !== expectedThreadId) {
      return;
    }
    const next = { ...this.threadBySource };
    delete next[sourceThreadId];
    this.threadBySource = next;
    this.syncShadows();
    this.commit();
  }

  /** Track an out-of-band side thread id (queue-routing shadow only). */
  rememberSideThreadId(threadId: string): void {
    this.sideChatThreadIdsRef.current = new Set([
      ...this.sideChatThreadIdsRef.current,
      threadId,
    ]);
  }

  draftFor(sourceThreadId: string | null): SideComposerDraft {
    return (
      (sourceThreadId && this.composerBySource[sourceThreadId]) ||
      emptySideComposerDraft()
    );
  }

  updateDraft(
    sourceThreadId: string,
    updater: (current: SideComposerDraft) => SideComposerDraft,
  ): void {
    const current = this.draftFor(sourceThreadId);
    const next = updater(current);
    if (next === current) {
      return;
    }
    this.composerBySource = {
      ...this.composerBySource,
      [sourceThreadId]: next,
    };
    this.commit();
  }

  clearDraft(sourceThreadId: string): void {
    const current = this.composerBySource[sourceThreadId];
    this.composerBySource = {
      ...this.composerBySource,
      [sourceThreadId]: {
        ...emptySideComposerDraft(),
        resetKey: (current?.resetKey ?? 0) + 1,
      },
    };
    this.commit();
  }

  /**
   * Acquire the composer upload lock and get back its ONLY release: an
   * idempotent closure bound to the acquiring scope generation. There is
   * deliberately no public decrement — a late release from a previous
   * generation is structurally a no-op (the transition already zeroed the
   * counter), so stale uploads can never unlock a newer scope's composer.
   */
  beginAttachmentUpload(): () => void {
    const generation = this.scopeGenerationValue;
    this.attachmentUploadCount += 1;
    this.commit();
    let released = false;
    return () => {
      if (released) {
        return;
      }
      released = true;
      if (this.scopeGenerationValue !== generation) {
        return;
      }
      this.attachmentUploadCount = Math.max(0, this.attachmentUploadCount - 1);
      this.commit();
    };
  }

  setCreating(sourceThreadId: string, creating: boolean): void {
    if (Boolean(this.creatingBySource[sourceThreadId]) === creating) {
      return;
    }
    this.creatingBySource = {
      ...this.creatingBySource,
      [sourceThreadId]: creating,
    };
    this.commit();
  }

  setError(sourceThreadId: string, error: string | null): void {
    if ((this.errorBySource[sourceThreadId] || null) === error) {
      return;
    }
    const next = { ...this.errorBySource };
    if (error) {
      next[sourceThreadId] = error;
    } else {
      delete next[sourceThreadId];
    }
    this.errorBySource = next;
    this.commit();
  }

  setHistoryLoading(loading: boolean): void {
    if (this.historyLoading === loading) {
      return;
    }
    this.historyLoading = loading;
    this.commit();
  }

  creationPromiseFor(
    sourceThreadId: string,
  ): Promise<string | null> | undefined {
    return this.creationPromiseBySource[sourceThreadId];
  }

  setCreationPromise(
    sourceThreadId: string,
    promise: Promise<string | null> | null,
  ): void {
    if (promise) {
      this.creationPromiseBySource[sourceThreadId] = promise;
    } else {
      delete this.creationPromiseBySource[sourceThreadId];
    }
  }
}
