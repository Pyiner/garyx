import type {
  DesktopChatStreamEvent,
  DesktopSettings,
  StartThreadStreamInput,
  StopThreadStreamInput,
} from "@shared/contracts";
import { ThreadStreamGapError, streamThreadEvents } from "./gary-client.ts";

/**
 * Owns every per-thread gateway SSE connection in the main process.
 *
 * All gateway HTTP goes through Chromium's network stack (`net.fetch`), whose
 * HTTP/1.1 pool allows only 6 concurrent connections per host and is shared
 * with every other gateway request the app makes. A single leaked stream
 * therefore permanently burns 1/6 of the whole app's connectivity to the
 * gateway, and 6 leaked streams starve everything (all API calls queue until
 * their AbortSignal timeouts fire). The hub's job is to make that state
 * unrepresentable: no stream connection may outlive the attempt that opened
 * it, and no attempt may outlive its forwarder entry.
 */
export interface ThreadStreamHubDeps {
  resolveSettings(): Promise<DesktopSettings>;
  /** Deliver a stream payload to the renderer sink. */
  sendEvent(payload: DesktopChatStreamEvent): void;
  /** Whether the renderer sink can still receive events. */
  isSinkAlive(): boolean;
  /** Test seam; production uses the real gateway SSE reader. */
  streamThreadEventsImpl?: typeof streamThreadEvents;
  retryInitialDelayMs?: number;
  retryMaxDelayMs?: number;
  /** Watchdog overrides forwarded to the stream reader (tests use small
   * values); production omits them for the 20s/90s defaults. */
  headerTimeoutMs?: number;
  idleTimeoutMs?: number;
}

export interface ThreadStreamHub {
  start(input: StartThreadStreamInput, ownerIds?: Iterable<string>): void;
  stop(input?: StopThreadStreamInput | null): void;
  restartAll(): void;
  activeThreadIds(): string[];
}

interface ThreadStreamForwarder {
  controller: AbortController;
  owners: Set<string>;
  lastSeq: number;
  /** Render window floor this stream is rendering with; pinned across
   * reconnects so a caught-up resume keeps the server's windowed derivation
   * instead of falling back to the full-transcript path. */
  lastFloor: number;
}

function threadStreamConsumerId(input?: {
  consumerId?: string | null;
} | null): string {
  return input?.consumerId?.trim() || "default";
}

function sleepWithAbort(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal.aborted) {
      resolve();
      return;
    }
    const timer = setTimeout(resolve, ms);
    signal.addEventListener(
      "abort",
      () => {
        clearTimeout(timer);
        resolve();
      },
      { once: true },
    );
  });
}

export function createThreadStreamHub(deps: ThreadStreamHubDeps): ThreadStreamHub {
  const streamImpl = deps.streamThreadEventsImpl ?? streamThreadEvents;
  const retryInitialDelayMs = deps.retryInitialDelayMs ?? 500;
  const retryMaxDelayMs = deps.retryMaxDelayMs ?? 10_000;
  const forwarders = new Map<string, ThreadStreamForwarder>();

  function stop(input?: StopThreadStreamInput | null): void {
    const normalizedThreadId = input?.threadId?.trim() || null;
    const consumerId = input?.consumerId?.trim() || null;
    if (!normalizedThreadId) {
      for (const forwarder of forwarders.values()) {
        forwarder.controller.abort();
      }
      forwarders.clear();
      return;
    }
    const forwarder = forwarders.get(normalizedThreadId);
    if (!forwarder) {
      return;
    }
    if (consumerId) {
      forwarder.owners.delete(consumerId);
      if (forwarder.owners.size > 0) {
        return;
      }
    }
    forwarder.controller.abort();
    forwarders.delete(normalizedThreadId);
  }

  function start(
    input: StartThreadStreamInput,
    ownerIds?: Iterable<string>,
  ): void {
    const threadId = input.threadId.trim();
    if (!threadId || !deps.isSinkAlive()) {
      return;
    }
    const owners = new Set(ownerIds ?? [threadStreamConsumerId(input)]);
    const existing = forwarders.get(threadId);
    if (existing) {
      for (const owner of existing.owners) {
        owners.add(owner);
      }
      existing.controller.abort();
    }
    const controller = new AbortController();
    const afterSeq = Math.max(
      Math.max(0, Math.trunc(input.afterSeq ?? 0)),
      existing?.lastSeq ?? 0,
    );
    const forwarder: ThreadStreamForwarder = {
      controller,
      owners,
      lastSeq: afterSeq,
      lastFloor: Math.max(
        Math.max(0, Math.trunc(input.renderFloor ?? 0)),
        existing?.lastFloor ?? 0,
      ),
    };
    forwarders.set(threadId, forwarder);
    void (async () => {
      try {
        let retryDelayMs = retryInitialDelayMs;
        let resumeAfterSeq = forwarder.lastSeq;
        while (!controller.signal.aborted && deps.isSinkAlive()) {
          try {
            const settings = await deps.resolveSettings();
            await streamImpl(
              settings,
              threadId,
              (payload) => {
                if (deps.isSinkAlive()) {
                  deps.sendEvent(payload);
                }
              },
              controller.signal,
              {
                afterSeq: resumeAfterSeq,
                renderFloor: forwarder.lastFloor,
                headerTimeoutMs: deps.headerTimeoutMs,
                idleTimeoutMs: deps.idleTimeoutMs,
                onCommittedSeq: (seq) => {
                  resumeAfterSeq = seq;
                  forwarder.lastSeq = seq;
                },
                onWindowFloor: (floorSeq) => {
                  forwarder.lastFloor = floorSeq;
                },
              },
            );
            retryDelayMs = retryInitialDelayMs;
          } catch (error) {
            if (controller.signal.aborted || !deps.isSinkAlive()) {
              break;
            }
            if (error instanceof ThreadStreamGapError) {
              deps.sendEvent({
                type: "error",
                runId: "thread-stream-gap",
                threadId,
                sessionId: threadId,
                error: `Thread stream seq gap after ${error.resumeAfterSeq}; authoritative refetch required`,
              });
              break;
            }
          }
          await sleepWithAbort(retryDelayMs, controller.signal);
          retryDelayMs = Math.min(
            Math.max(retryDelayMs * 2, retryInitialDelayMs),
            retryMaxDelayMs,
          );
        }
      } finally {
        // The loop's exit is the attempt's end of life: abort so no fetch
        // (or its checked-out pool connection) can outlive the forwarder,
        // whatever path led here (gap break, dead sink, external stop).
        controller.abort();
        if (forwarders.get(threadId)?.controller === controller) {
          forwarders.delete(threadId);
        }
      }
    })();
  }

  function restartAll(): void {
    const active = Array.from(forwarders.entries()).map(
      ([threadId, forwarder]) => ({
        threadId,
        afterSeq: forwarder.lastSeq,
        renderFloor: forwarder.lastFloor,
        owners: new Set(forwarder.owners),
      }),
    );
    for (const forwarder of forwarders.values()) {
      forwarder.controller.abort();
    }
    forwarders.clear();
    for (const item of active) {
      start(
        {
          threadId: item.threadId,
          afterSeq: item.afterSeq,
          renderFloor: item.renderFloor,
        },
        item.owners,
      );
    }
  }

  return {
    start,
    stop,
    restartAll,
    activeThreadIds: () => Array.from(forwarders.keys()),
  };
}

/**
 * The renderer sink's true lifetime is the document, not the window. A
 * main-frame cross-document navigation (Cmd+R reload, dev-server restart)
 * replaces the document without running React effect cleanup, so the stop
 * calls that normally balance every start never arrive — and `isSinkAlive`
 * stays true because the window survives. Each forwarder orphaned that way
 * holds a healthy gateway SSE socket indefinitely (the idle watchdog never
 * fires on a live stream), so cross-reload thread switching accumulates one
 * zombie socket per cycle: the TASK-1840 pool-starvation path. Dropping every
 * forwarder when a cross-document navigation starts restores the invariant;
 * the new document re-subscribes to exactly what it renders.
 */
export function bindThreadStreamSinkNavigation(
  hub: ThreadStreamHub,
  contents: {
    on(
      event: "did-start-navigation",
      listener: (details: {
        isMainFrame: boolean;
        isSameDocument: boolean;
      }) => void,
    ): unknown;
  },
): void {
  contents.on("did-start-navigation", (details) => {
    if (details.isMainFrame && !details.isSameDocument) {
      hub.stop();
    }
  });
}
