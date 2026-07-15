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
  const mergeStep = source.indexOf(
    "const effectivePinnedThreadIds = await applyRemotePinsMergeStep(",
    requests,
  );
  const commit = source.indexOf("pinnedThreadIds,", mergeStep);

  assert.ok(mergeStart >= 0);
  assert.ok(stamp > mergeStart && stamp < requests, "stamp must be captured before requests await");
  assert.ok(mergeStep > requests, "merge step (acceptance + projection) must run after the fetch");
  assert.match(
    source.slice(mergeStep, mergeStep + 400),
    /pinsRequestStamp/,
    "merge step must consume the pre-await stamp",
  );
  assert.ok(commit > mergeStep, "remote overwrite commits the projected order");
  assert.match(source.slice(commit, commit + 180), /pinsRevision: pinOrder\.state\.highestObservedRevision/);
});

test("preload and main expose reorder plus non-persisted sync snapshot contracts", async () => {
  const [apiSource, preloadSource, mainSource, storeSource] = await Promise.all([
    readFile(new URL("../shared/contracts/desktop-api.ts", import.meta.url), "utf8"),
    readFile(new URL("../preload/index.ts", import.meta.url), "utf8"),
    readFile(new URL("./index.ts", import.meta.url), "utf8"),
    readFile(new URL("./store.ts", import.meta.url), "utf8"),
  ]);

  assert.match(apiSource, /setThreadPinOrder: \(input: SetThreadPinOrderInput\) => Promise<DesktopState>/);
  assert.match(preloadSource, /setThreadPinOrder:[\s\S]*garyx:set-thread-pin-order/);
  assert.match(mainSource, /garyx:set-thread-pin-order[\s\S]*setDesktopThreadPinOrder/);
  assert.match(apiSource, /getThreadPinOrderSnapshot: \(\) => Promise<DesktopThreadPinOrderSnapshot>/);
  assert.match(preloadSource, /getThreadPinOrderSnapshot:[\s\S]*garyx:get-thread-pin-order-snapshot/);
  assert.match(mainSource, /garyx:get-thread-pin-order-snapshot[\s\S]*getDesktopThreadPinOrderSnapshot/);
  assert.match(storeSource, /export async function resumeDesktopPinnedOrderSync/);
  assert.match(mainSource, /window\.on\("focus"[\s\S]*resumeDesktopPinnedOrderSync/);
});
