export const TRANSCRIPT_PERSIST_DEBOUNCE_MS = 1_000;
export const TRANSCRIPT_PERSIST_MAX_WAIT_MS = 5_000;

export interface TranscriptPersistClock {
  setTimeout(callback: () => void, delayMs: number): unknown;
  clearTimeout(handle: unknown): void;
}

const systemClock: TranscriptPersistClock = {
  setTimeout(callback, delayMs) {
    return globalThis.setTimeout(callback, delayMs);
  },
  clearTimeout(handle) {
    globalThis.clearTimeout(handle as ReturnType<typeof setTimeout>);
  },
};

interface PendingTranscriptPersist {
  trailingTimer: unknown;
  maxWaitTimer: unknown;
}

function unrefTimer(handle: unknown): void {
  if (
    handle &&
    typeof handle === "object" &&
    "unref" in handle &&
    typeof (handle as { unref?: unknown }).unref === "function"
  ) {
    (handle as { unref: () => void }).unref();
  }
}

/**
 * Per-thread trailing debounce with a non-resetting max-wait deadline. The
 * scheduler stores only dirty thread ids; its flush callback reads the latest
 * transcript lazily, so committed events do no cache projection or IPC clone.
 */
export class TranscriptPersistScheduler {
  private readonly pending = new Map<string, PendingTranscriptPersist>();
  private readonly onFlush: (threadId: string) => void;
  private readonly clock: TranscriptPersistClock;
  private readonly debounceMs: number;
  private readonly maxWaitMs: number;

  constructor(
    onFlush: (threadId: string) => void,
    clock: TranscriptPersistClock = systemClock,
    debounceMs = TRANSCRIPT_PERSIST_DEBOUNCE_MS,
    maxWaitMs = TRANSCRIPT_PERSIST_MAX_WAIT_MS,
  ) {
    this.onFlush = onFlush;
    this.clock = clock;
    this.debounceMs = debounceMs;
    this.maxWaitMs = maxWaitMs;
  }

  schedule(threadId: string): void {
    const normalizedThreadId = threadId.trim();
    if (!normalizedThreadId) {
      return;
    }
    const current = this.pending.get(normalizedThreadId);
    if (current) {
      this.clock.clearTimeout(current.trailingTimer);
      current.trailingTimer = this.scheduleTimer(
        normalizedThreadId,
        this.debounceMs,
      );
      return;
    }

    const entry: PendingTranscriptPersist = {
      trailingTimer: this.scheduleTimer(normalizedThreadId, this.debounceMs),
      maxWaitTimer: this.scheduleTimer(normalizedThreadId, this.maxWaitMs),
    };
    this.pending.set(normalizedThreadId, entry);
  }

  flush(threadId: string): boolean {
    const normalizedThreadId = threadId.trim();
    const entry = this.pending.get(normalizedThreadId);
    if (!entry) {
      return false;
    }
    this.pending.delete(normalizedThreadId);
    this.clock.clearTimeout(entry.trailingTimer);
    this.clock.clearTimeout(entry.maxWaitTimer);
    this.onFlush(normalizedThreadId);
    return true;
  }

  cancel(threadId: string): boolean {
    const normalizedThreadId = threadId.trim();
    const entry = this.pending.get(normalizedThreadId);
    if (!entry) {
      return false;
    }
    this.pending.delete(normalizedThreadId);
    this.clock.clearTimeout(entry.trailingTimer);
    this.clock.clearTimeout(entry.maxWaitTimer);
    return true;
  }

  flushAll(): number {
    const threadIds = [...this.pending.keys()];
    for (const threadId of threadIds) {
      this.flush(threadId);
    }
    return threadIds.length;
  }

  pendingThreadCount(): number {
    return this.pending.size;
  }

  private scheduleTimer(threadId: string, delayMs: number): unknown {
    const handle = this.clock.setTimeout(() => {
      this.flush(threadId);
    }, delayMs);
    unrefTimer(handle);
    return handle;
  }
}
