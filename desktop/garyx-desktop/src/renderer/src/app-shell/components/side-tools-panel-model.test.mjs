import test from "node:test";
import assert from "node:assert/strict";

import {
  capsuleIdFromTabKey,
  capsuleTabKey,
  closeTab,
  isCapsuleTabKey,
  shouldCollapseFileDirectoryForPreview,
  workspacePreviewDirectoryCollapseKey,
} from "./side-tools-panel-model.ts";

test("workspace preview has no directory collapse key when preview is absent", () => {
  assert.equal(
    workspacePreviewDirectoryCollapseKey({
      shouldShowWorkspacePreview: false,
      workspaceFilePreviewPath: "notes.md",
      workspacePreviewTitle: "/workspace/project/notes.md",
    }),
    null,
  );
});

test("workspace preview collapse key is stable from selection title", () => {
  assert.equal(
    workspacePreviewDirectoryCollapseKey({
      shouldShowWorkspacePreview: true,
      workspaceFilePreviewPath: null,
      workspacePreviewTitle: "/workspace/project/notes.md",
    }),
    "title:/workspace/project/notes.md",
  );
  assert.equal(
    workspacePreviewDirectoryCollapseKey({
      shouldShowWorkspacePreview: true,
      workspaceFilePreviewPath: "notes.md",
      workspacePreviewTitle: "/workspace/project/notes.md",
    }),
    "title:/workspace/project/notes.md",
  );
});

test("workspace preview collapse key falls back to preview path", () => {
  assert.equal(
    workspacePreviewDirectoryCollapseKey({
      shouldShowWorkspacePreview: true,
      workspaceFilePreviewPath: "src/index.ts",
      workspacePreviewTitle: "Select a file",
    }),
    "path:src/index.ts",
  );
});

test("capsule tab keys round-trip and are distinguished from built-in tools", () => {
  const key = capsuleTabKey("cap-123");
  assert.equal(key, "capsule:cap-123");
  assert.equal(isCapsuleTabKey(key), true);
  assert.equal(isCapsuleTabKey("files"), false);
  assert.equal(capsuleIdFromTabKey(key), "cap-123");
  assert.equal(capsuleIdFromTabKey("files"), null);
  // Capsule ids may themselves contain a colon; only the first prefix is stripped.
  assert.equal(capsuleIdFromTabKey("capsule:a:b"), "a:b");
});

test("closeTab removes the tab and repicks the active when it was active", () => {
  // Closing the active tab activates the last remaining tab.
  assert.deepEqual(
    closeTab(["files", "chat", "capsule:x"], "chat", "chat"),
    { openTabs: ["files", "capsule:x"], activeKey: "capsule:x" },
  );
  // Closing a non-active tab leaves the active key untouched.
  assert.deepEqual(
    closeTab(["files", "chat", "capsule:x"], "capsule:x", "files"),
    { openTabs: ["chat", "capsule:x"], activeKey: "capsule:x" },
  );
  // Closing the last tab clears the active key.
  assert.deepEqual(
    closeTab(["capsule:x"], "capsule:x", "capsule:x"),
    { openTabs: [], activeKey: null },
  );
});

test("file directory collapses only when a new preview opens", () => {
  assert.equal(
    shouldCollapseFileDirectoryForPreview({
      previousPreviewKey: null,
      nextPreviewKey: "title:/workspace/project/notes.md",
    }),
    true,
  );
  assert.equal(
    shouldCollapseFileDirectoryForPreview({
      previousPreviewKey: "title:/workspace/project/notes.md",
      nextPreviewKey: "title:/workspace/project/notes.md",
    }),
    false,
  );
  assert.equal(
    shouldCollapseFileDirectoryForPreview({
      previousPreviewKey: "title:/workspace/project/notes.md",
      nextPreviewKey: "title:/workspace/project/README.md",
    }),
    true,
  );
  assert.equal(
    shouldCollapseFileDirectoryForPreview({
      previousPreviewKey: "title:/workspace/project/notes.md",
      nextPreviewKey: null,
    }),
    false,
  );
});
