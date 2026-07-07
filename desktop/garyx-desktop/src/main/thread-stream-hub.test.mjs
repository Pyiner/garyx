import test from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { once } from "node:events";

import { createThreadStreamHub } from "./thread-stream-hub.ts";

// Regression harness for #TASK-1840: during a gateway crash loop the desktop
// stacked up zombie SSE connections until Chromium's 6-per-host pool was
// exhausted and every gateway request timed out until app restart. These
// tests drive the real streamThreadEvents reader against a real local HTTP
// server and assert the invariant that fixes the incident class:
//
//   the number of live gateway stream sockets never exceeds the number of
//   live forwarders, across gap errors, silent streams, and crash loops.

function sseFrame(threadId, seqs, basedOnSeq) {
  const payload = {
    type: "thread_render_frame",
    thread_id: threadId,
    events: seqs.map((seq) => ({
      seq,
      thread_id: threadId,
      message: { role: "assistant", text: `m${seq}` },
    })),
    render_state: {
      based_on_seq: basedOnSeq ?? (seqs.length ? seqs[seqs.length - 1] : 0),
      rows: [],
    },
  };
  return `data: ${JSON.stringify(payload)}\n\n`;
}

function afterSeqOf(req) {
  const url = new URL(req.url, "http://localhost");
  return Math.max(0, Number(url.searchParams.get("after_seq") || "0"));
}

/**
 * Local SSE gateway stand-in. `handler(ctx)` scripts each stream request;
 * the harness tracks live sockets so tests can assert connection bounds —
 * the exact thing lsof measured against the real gateway in the incident.
 */
async function startSseServer(handler) {
  const sockets = new Set();
  const timers = new Set();
  const state = {
    requestsByThread: new Map(),
    highWater: 0,
    activeStreams: 0,
  };
  const server = createServer((req, res) => {
    const match = /^\/api\/threads\/([^/]+)\/stream/.exec(req.url || "");
    if (!match) {
      res.statusCode = 404;
      res.end("not found");
      return;
    }
    const threadId = decodeURIComponent(match[1]);
    const requestIndex = state.requestsByThread.get(threadId) ?? 0;
    state.requestsByThread.set(threadId, requestIndex + 1);
    req.socket.garyxThreadId = threadId;
    // A live stream is a response the client has not torn down yet — the
    // exact resource that leaked in the incident. (Raw socket counts also
    // see undici's idle keep-alive pool connection, which is reusable and
    // expires on its own; it is not a zombie.)
    state.activeStreams += 1;
    res.on("close", () => {
      state.activeStreams -= 1;
    });
    const ping = () => {
      const timer = setInterval(() => {
        res.write(": ping\n\n");
      }, 25);
      timers.add(timer);
      res.on("close", () => {
        clearInterval(timer);
        timers.delete(timer);
      });
    };
    const wait = (ms) =>
      new Promise((resolve) => {
        const timer = setTimeout(resolve, ms);
        timers.add(timer);
        res.on("close", () => clearTimeout(timer));
      });
    res.writeHead(200, {
      "content-type": "text/event-stream",
      "cache-control": "no-cache",
    });
    handler({
      threadId,
      requestIndex,
      afterSeq: afterSeqOf(req),
      res,
      socket: req.socket,
      ping,
      wait,
      frame: (seqs, basedOnSeq) => res.write(sseFrame(threadId, seqs, basedOnSeq)),
    });
  });
  server.on("connection", (socket) => {
    sockets.add(socket);
    state.highWater = Math.max(state.highWater, sockets.size);
    socket.on("close", () => sockets.delete(socket));
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const port = server.address().port;
  return {
    url: `http://127.0.0.1:${port}`,
    liveSockets: (threadId) =>
      Array.from(sockets).filter(
        (socket) => !threadId || socket.garyxThreadId === threadId,
      ).length,
    activeStreams: () => state.activeStreams,
    requests: (threadId) => state.requestsByThread.get(threadId) ?? 0,
    highWater: () => state.highWater,
    close: async () => {
      for (const timer of timers) {
        clearTimeout(timer);
        clearInterval(timer);
      }
      for (const socket of sockets) {
        socket.destroy();
      }
      await new Promise((resolve) => server.close(resolve));
    },
  };
}

function makeHub(serverUrl, overrides = {}) {
  const events = [];
  const hub = createThreadStreamHub({
    resolveSettings: async () => ({
      gatewayUrl: serverUrl,
      gatewayAuthToken: "",
      gatewayHeaders: "",
    }),
    isSinkAlive: () => true,
    sendEvent: (payload) => {
      events.push(payload);
      overrides.onEvent?.(payload);
    },
    retryInitialDelayMs: 20,
    retryMaxDelayMs: 50,
    idleTimeoutMs: overrides.idleTimeoutMs,
  });
  return { hub, events };
}

async function until(check, timeoutMs, label) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (check()) {
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  if (!check()) {
    assert.fail(`timed out waiting for: ${label}`);
  }
}

const settle = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

test("gap-error reconnect cycles never leak stream sockets", async () => {
  // Incident shape: every stream teardown that is not owner-initiated (here:
  // seq gap -> renderer refetch -> restart) must abort the old connection.
  // Pre-fix, the forwarder breaks out of its loop on ThreadStreamGapError
  // without aborting, deletes its map entry, and the restarted forwarder can
  // no longer reach the old controller: one zombie ESTABLISHED socket per gap.
  const server = await startSseServer((ctx) => {
    const base = ctx.afterSeq;
    ctx.frame([base + 1, base + 2]);
    if (ctx.requestIndex < 3) {
      void ctx.wait(40).then(() => {
        if (!ctx.res.writableEnded && !ctx.res.destroyed) {
          // Skips ahead of the connection cursor -> client-side gap error.
          ctx.frame([base + 10]);
        }
      });
      ctx.ping();
    } else {
      ctx.ping();
    }
  });
  try {
    let tail = 0;
    let gapErrors = 0;
    const { hub } = makeHub(server.url, {
      onEvent: (payload) => {
        if (payload.type === "thread_render_frame") {
          for (const event of payload.events) {
            tail = Math.max(tail, event.seq);
          }
          return;
        }
        if (payload.type === "error" && payload.runId === "thread-stream-gap") {
          gapErrors += 1;
          // The renderer reacts to a gap by refetching and restarting the
          // stream; emulate that restart.
          setTimeout(() => {
            hub.start({ threadId: "t-gap", afterSeq: tail });
          }, 10);
        }
      },
    });

    hub.start({ threadId: "t-gap", afterSeq: 0 });
    await until(() => gapErrors >= 3, 5000, "3 gap errors observed");
    await until(
      () => server.requests("t-gap") >= 4,
      5000,
      "post-gap stream reconnected",
    );
    // Give closes a moment to land, then hold the bound for a while: the
    // zombie sockets of the pre-fix code do NOT close on their own.
    await settle(300);
    assert.equal(
      server.liveSockets("t-gap"),
      1,
      `expected exactly the live stream socket, got ${server.liveSockets("t-gap")} (zombies leaked)`,
    );

    hub.stop();
    await until(() => server.activeStreams() === 0, 2000, "stop() aborts all live streams");
  } finally {
    await server.close();
  }
});

test("a silent stream is aborted and re-dialed by the idle watchdog", async () => {
  // Incident shape: a stream whose peer stops sending (half-open / wedged
  // gateway) must not hold its pool slot forever. The gateway sends a
  // keep-alive ping every 30s; a client that sees nothing for the idle
  // timeout must abort and reconnect.
  const server = await startSseServer((ctx) => {
    ctx.frame([ctx.afterSeq + 1]);
    if (ctx.requestIndex === 0) {
      // First stream goes silent: no pings, socket stays open.
      return;
    }
    ctx.ping();
  });
  try {
    const { hub } = makeHub(server.url, { idleTimeoutMs: 250 });
    hub.start({ threadId: "t-idle", afterSeq: 0 });

    await until(
      () => server.requests("t-idle") >= 2,
      5000,
      "watchdog re-dialed the silent stream",
    );
    await until(
      () => server.liveSockets("t-idle") === 1,
      2000,
      "silent socket was closed",
    );

    hub.stop();
    await until(() => server.activeStreams() === 0, 2000, "stop() aborts all live streams");
  } finally {
    await server.close();
  }
});

test("a crash-looping gateway keeps connections bounded and self-heals", async () => {
  // Incident verification requirement: gateway dies repeatedly (kill -9 /
  // launchd throttle); connection count must stay bounded and the stream must
  // recover without an app restart once the gateway stays up.
  const server = await startSseServer((ctx) => {
    if (ctx.requestIndex < 3) {
      void ctx.wait(30).then(() => ctx.socket.destroy());
      return;
    }
    ctx.frame([ctx.afterSeq + 1]);
    ctx.ping();
  });
  try {
    let sawFrame = false;
    const { hub } = makeHub(server.url, {
      onEvent: (payload) => {
        if (payload.type === "thread_render_frame" && payload.events.length) {
          sawFrame = true;
        }
      },
    });
    hub.start({ threadId: "t-flap", afterSeq: 0 });

    await until(() => sawFrame, 5000, "stream recovered after crash loop");
    await settle(200);
    assert.equal(server.liveSockets("t-flap"), 1);
    assert.ok(
      server.highWater() <= 2,
      `crash-loop reconnects must not stack sockets (high water ${server.highWater()})`,
    );

    hub.stop();
    await until(() => server.activeStreams() === 0, 2000, "stop() aborts all live streams");
  } finally {
    await server.close();
  }
});
