import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const appShellSource = readFileSync(
  new URL("./AppShell.tsx", import.meta.url),
  "utf8",
);
const sideChatSource = readFileSync(
  new URL("./components/SideChatPanel.tsx", import.meta.url),
  "utf8",
);

test("AppShell transcript rendering subscribes only to active and side threads", () => {
  assert.doesNotMatch(appShellSource, /subscribeTranscriptMaps\s*\(/);
  assert.doesNotMatch(appShellSource, /subscribeLiveStreams\s*\(/);
  assert.match(
    appShellSource,
    /useGatewayThreadMirror\(\s*gatewayMirror,\s*activeThreadMessageKey,?\s*\)/,
  );
  assert.match(
    appShellSource,
    /useGatewayThreadMirror\(\s*gatewayMirror,\s*sideChatThreadId,?\s*\)/,
  );
});

test("SideChatPanel subscribes directly to its bound thread", () => {
  assert.doesNotMatch(sideChatSource, /subscribeTranscriptMaps\s*\(/);
  assert.doesNotMatch(sideChatSource, /subscribeLiveStreams\s*\(/);
  assert.match(
    sideChatSource,
    /useThreadMirror\(sideChatThreadId\)/,
  );
});
