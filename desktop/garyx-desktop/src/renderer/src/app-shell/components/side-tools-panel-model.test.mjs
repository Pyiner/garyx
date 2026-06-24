import test from "node:test";
import assert from "node:assert/strict";

import {
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
