// Reproduces the #TASK-1840 starvation class end-to-end on the real Chromium
// network stack: per-thread SSE streams and control-plane requests sharing one
// HTTP/1.1 socket pool (6 connections per host) means six live streams starve
// every other gateway request into its AbortSignal timeout ("The operation was
// aborted due to timeout"). Both modes drive the REAL stream reader
// (streamThreadEvents) and the real transport seams; they differ only in
// wiring. Run under the Electron binary:
//
//   NODE_OPTIONS=--experimental-strip-types electron scripts/stream-pool-smoke.mjs            # app wiring (green)
//   NODE_OPTIONS=--experimental-strip-types electron scripts/stream-pool-smoke.mjs --legacy   # single-pool wiring (red)
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
  if (req.url.includes("/stream")) {
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

async function main() {
  const baseUrl = await listen();

  const http = await import("../src/main/garyx-client/http.ts");
  const { streamThreadEvents } = await import(
    "../src/main/garyx-client/stream.ts"
  );
  if (legacySinglePool) {
    // The pre-fix composition: one pool for everything (what a bare
    // setGatewayFetch(net.fetch) produced — streams fall back onto the
    // control transport).
    http.setGatewayFetch((input, init) => net.fetch(input, init));
    http.setGatewayStreamFetch(null);
  } else {
    // The app's actual transport wiring.
    const transport = await import("../src/main/gateway-transport.ts");
    transport.wireGatewayTransport();
  }

  // Open the streams through the real per-thread SSE reader so the smoke
  // covers stream.ts's transport seam consumption, not just the wiring.
  const settings = { gatewayUrl: baseUrl, gatewayAuthToken: "" };
  for (let index = 0; index < STREAM_COUNT; index += 1) {
    const connected = new Promise((resolve) => {
      void streamThreadEvents(
        settings,
        `thread::smoke-${index}`,
        () => {},
        undefined,
        { onConnected: () => resolve(true) },
      ).catch(() => resolve(false));
    });
    if (!(await connected)) {
      console.error(`FAIL: stream ${index} did not connect`);
      process.exit(1);
    }
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
    const response = await http.gatewayFetch(`${baseUrl}/api/health`, {
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

app.whenReady().then(() => {
  main().catch((error) => {
    console.error(`FAIL: ${error}`);
    process.exit(1);
  });
});
