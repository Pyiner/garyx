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

export interface SideChatSessionsSnapshot {
  readonly version: number;
  readonly threadBySource: Readonly<Record<string, string>>;
  readonly composerBySource: Readonly<Record<string, SideComposerDraft>>;
  readonly creatingBySource: Readonly<Record<string, boolean>>;
  readonly errorBySource: Readonly<Record<string, string>>;
  readonly attachmentUploadCount: number;
  readonly historyLoading: boolean;
}

export class SideChatSessions {
  /** Normalized gateway URL partitioning the persisted bindings. */
  gatewayScope = "";
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

  /** Pure: the per-thread committed-stream consumer identity. */
  streamConsumerId(threadId: string): string {
    return `side-chat:${threadId}`;
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

  /** Bind a side thread to a source thread (+ sessionStorage write). */
  rememberThread(sourceThreadId: string, sideThreadId: string): void {
    if (this.threadBySource[sourceThreadId] === sideThreadId) {
      return;
    }
    this.threadBySource = {
      ...this.threadBySource,
      [sourceThreadId]: sideThreadId,
    };
    persistSideChatThreadId(this.gatewayScope, sourceThreadId, sideThreadId);
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
      this.gatewayScope,
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

  beginAttachmentUpload(): void {
    this.attachmentUploadCount += 1;
    this.commit();
  }

  endAttachmentUpload(): void {
    this.attachmentUploadCount = Math.max(0, this.attachmentUploadCount - 1);
    this.commit();
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
