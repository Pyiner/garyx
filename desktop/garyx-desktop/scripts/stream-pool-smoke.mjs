// Reproduces the #TASK-1840 starvation class end-to-end on the real Chromium
// network stack: per-thread SSE streams and control-plane requests sharing one
// HTTP/1.1 socket pool (6 connections per host) means six live streams starve
// every other gateway request into its AbortSignal timeout ("The operation was
// aborted due to timeout"). Run under the Electron binary:
//
//   npx electron scripts/stream-pool-smoke.mjs            # exercises app wiring
//   npx electron scripts/stream-pool-smoke.mjs --legacy   # single-pool wiring (red)
//
// Exits 0 when the control request completes while all six streams stay
// connected; exits 1 with the observed error when it starves.
import { createServer } from "node:http";

import electron from "electron";
const { app, net } = electron;

const STREAM_COUNT = 6;
const CONTROL_TIMEOUT_MS = 3_000;
const legacySinglePool = process.argv.includes("--legacy");

const streamSockets = new Set();
let controlServed = 0;

const stubGateway = createServer((req, res) => {
  if (req.url.startsWith("/api/threads/")) {
    res.writeHead(200, {
      "content-type": "text/event-stream",
      "cache-control": "no-cache",
    });
    res.write(": connected\n\n");
    streamSockets.add(res);
    const ping = setInterval(() => res.write(": ping\n\n"), 200);
    req.on("close", () => {
      clearInterval(ping);
      streamSockets.delete(res);
    });
    return;
  }
  controlServed += 1;
  res.writeHead(200, { "content-type": "application/json" });
  res.end(JSON.stringify({ ok: true }));
});

function listen() {
  return new Promise((resolve) => {
    stubGateway.listen(0, "127.0.0.1", () =>
      resolve(`http://127.0.0.1:${stubGateway.address().port}`),
    );
  });
}

async function openStream(streamFetch, baseUrl, index) {
  const response = await streamFetch(
    `${baseUrl}/api/threads/thread%3A%3Asmoke-${index}/stream?after_seq=0`,
    { headers: { Accept: "text/event-stream" } },
  );
  if (!response.ok || !response.body) {
    throw new Error(`stream ${index} failed: ${response.status}`);
  }
  const reader = response.body.getReader();
  void (async () => {
    try {
      while (true) {
        const { done } = await reader.read();
        if (done) break;
      }
    } catch {
      // Stream teardown at process exit is expected.
    }
  })();
}

async function main() {
  const baseUrl = await listen();

  let controlFetch;
  let streamFetch;
  if (legacySinglePool) {
    // The pre-fix composition: every gateway request through the default
    // session's pool (what setGatewayFetch(net.fetch) alone produced).
    controlFetch = (input, init) => net.fetch(input, init);
    streamFetch = controlFetch;
  } else {
    // The app's actual transport wiring.
    const transport = await import("../src/main/gateway-transport.ts");
    transport.wireGatewayTransport();
    const http = await import("../src/main/gary-client/http.ts");
    controlFetch = http.gatewayFetch;
    streamFetch = http.gatewayStreamFetch;
  }

  for (let index = 0; index < STREAM_COUNT; index += 1) {
    await openStream(streamFetch, baseUrl, index);
  }
  const streamsConnected = await waitFor(
    () => streamSockets.size === STREAM_COUNT,
    5_000,
  );
  if (!streamsConnected) {
    console.error(
      `FAIL: only ${streamSockets.size}/${STREAM_COUNT} streams connected`,
    );
    process.exit(1);
  }

  const startedAt = Date.now();
  try {
    const response = await controlFetch(`${baseUrl}/api/health`, {
      signal: AbortSignal.timeout(CONTROL_TIMEOUT_MS),
    });
    const elapsedMs = Date.now() - startedAt;
    if (!response.ok) {
      console.error(`FAIL: control request status ${response.status}`);
      process.exit(1);
    }
    await response.text();
    console.log(
      `PASS: control request completed in ${elapsedMs}ms with ` +
        `${streamSockets.size} live streams (served=${controlServed})`,
    );
    process.exit(0);
  } catch (error) {
    const elapsedMs = Date.now() - startedAt;
    console.error(
      `FAIL: control request did not complete (${elapsedMs}ms, ` +
        `${streamSockets.size} live streams): ${error}`,
    );
    process.exit(1);
  }
}

function waitFor(predicate, timeoutMs) {
  return new Promise((resolve) => {
    const startedAt = Date.now();
    const timer = setInterval(() => {
      if (predicate()) {
        clearInterval(timer);
        resolve(true);
      } else if (Date.now() - startedAt > timeoutMs) {
        clearInterval(timer);
        resolve(false);
      }
    }, 50);
  });
}

app.whenReady().then(() => {
  main().catch((error) => {
    console.error(`FAIL: ${error}`);
    process.exit(1);
  });
});
