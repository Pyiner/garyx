import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";

test("DesktopState persists only main-owned pin page fields", async () => {
  const [stateSource, storeSource] = await Promise.all([
    readFile(new URL("../shared/contracts/state.ts", import.meta.url), "utf8"),
    readFile(new URL("./store.ts", import.meta.url), "utf8"),
  ]);

  assert.match(stateSource, /pinnedThreadIds: string\[\];\s*\/\*\*[\s\S]*?pinsRevision: number;/);
  assert.doesNotMatch(stateSource, /capturedEpoch|rendererSessionId/);
  assert.match(storeSource, /type PersistedDesktopState[\s\S]*pinnedOrderOutbox/);
  assert.match(storeSource, /pinnedOrderOutbox\?: PinnedOrderOutbox \| null/);
  assert.doesNotMatch(stateSource, /pinnedOrderOutbox/);
});

test("main-store pin merge captures before await and projects through the reducer", async () => {
  const source = await readFile(new URL("./store.ts", import.meta.url), "utf8");
  const mergeStart = source.indexOf("async function mergeRemoteDesktopState");
  const stamp = source.indexOf("const pinsRequestStamp = pinOrder.requestStamp()", mergeStart);
  const requests = source.indexOf("await Promise.all", mergeStart);
  const acceptance = source.indexOf("await pinOrder.receivePage", requests);
  const projection = source.indexOf(
    "const effectivePinnedThreadIds = pinOrder.state.presentedOrder",
    acceptance,
  );
  const commit = source.indexOf("pinnedThreadIds,", projection);

  assert.ok(mergeStart >= 0);
  assert.ok(stamp > mergeStart && stamp < requests, "stamp must be captured before requests await");
  assert.ok(acceptance > requests, "raw page must pass revision acceptance");
  assert.ok(projection > acceptance, "accepted reducer projection owns visible order");
  assert.ok(commit > projection, "remote overwrite commits the projected order");
  assert.match(source.slice(commit, commit + 180), /pinsRevision: pinOrder\.state\.highestObservedRevision/);
});

test("preload and main expose one setThreadPinOrder IPC contract", async () => {
  const [apiSource, preloadSource, mainSource] = await Promise.all([
    readFile(new URL("../shared/contracts/desktop-api.ts", import.meta.url), "utf8"),
    readFile(new URL("../preload/index.ts", import.meta.url), "utf8"),
    readFile(new URL("./index.ts", import.meta.url), "utf8"),
  ]);

  assert.match(apiSource, /setThreadPinOrder: \(input: SetThreadPinOrderInput\) => Promise<DesktopState>/);
  assert.match(preloadSource, /setThreadPinOrder:[\s\S]*garyx:set-thread-pin-order/);
  assert.match(mainSource, /garyx:set-thread-pin-order[\s\S]*setDesktopThreadPinOrder/);
});
