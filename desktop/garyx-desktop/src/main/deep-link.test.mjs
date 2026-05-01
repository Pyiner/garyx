import test from "node:test";
import assert from "node:assert/strict";

import { parseDesktopDeepLink } from "./deep-link.ts";

test("parses canonical thread deep link", () => {
  assert.deepEqual(parseDesktopDeepLink("garyx://thread/thread::abc123"), {
    type: "open-thread",
    url: "garyx://thread/thread::abc123",
    threadId: "thread::abc123",
  });
});

test("parses canonical resume deep link", () => {
  assert.deepEqual(
    parseDesktopDeepLink("garyx://resume/04b3eff5-fea5-4339-a682-afd3774b7cc8"),
    {
      type: "resume-session",
      url: "garyx://resume/04b3eff5-fea5-4339-a682-afd3774b7cc8",
      sessionId: "04b3eff5-fea5-4339-a682-afd3774b7cc8",
    },
  );
});

test("parses canonical provider-specific resume deep link", () => {
  assert.deepEqual(parseDesktopDeepLink("garyx://resume/CodeX/session-123"), {
    type: "resume-session",
    url: "garyx://resume/CodeX/session-123",
    sessionId: "session-123",
    providerHint: "codex",
  });
});

test("parses new thread deep link with workspace query", () => {
  assert.deepEqual(
    parseDesktopDeepLink("garyx://new?workspace=%2FUsers%2Fgary%2Frepo&agent=codex"),
    {
      type: "new-thread",
      url: "garyx://new?workspace=%2FUsers%2Fgary%2Frepo&agent=codex",
      workspacePath: "/Users/gary/repo",
      agentId: "codex",
    },
  );
});

test("parses new thread deep link with encoded workspace path segment", () => {
  assert.deepEqual(
    parseDesktopDeepLink("garyx://new/%2FUsers%2Fgary%2Frepo"),
    {
      type: "new-thread",
      url: "garyx://new/%2FUsers%2Fgary%2Frepo",
      workspacePath: "/Users/gary/repo",
      agentId: null,
    },
  );
});

test("rejects legacy query-based thread links", () => {
  assert.deepEqual(parseDesktopDeepLink("garyx://open?thread=thread::abc123"), {
    type: "error",
    url: "garyx://open?thread=thread::abc123",
    error:
      "Unsupported garyx:// target. Use garyx://thread/<thread-id>, garyx://new?workspace=<path>, garyx://resume/<session-id>, or garyx://resume/<provider>/<session-id>.",
  });
});

test("rejects unknown resume providers", () => {
  assert.deepEqual(parseDesktopDeepLink("garyx://resume/unknown/session-123"), {
    type: "error",
    url: "garyx://resume/unknown/session-123",
    error: "Unsupported resume provider. Use claude, codex, or gemini.",
  });
});

test("rejects extra path segments", () => {
  assert.deepEqual(
    parseDesktopDeepLink("garyx://resume/codex/session-123/extra"),
    {
      type: "error",
      url: "garyx://resume/codex/session-123/extra",
      error:
        "Unsupported garyx:// format. Use garyx://thread/<thread-id>, garyx://new?workspace=<path>, garyx://resume/<session-id>, or garyx://resume/<provider>/<session-id>.",
    },
  );
});
