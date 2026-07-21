import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
import path from "node:path";
import test from "node:test";

import * as esbuild from "esbuild";

import { buildThreadViewRows } from "./render-view-model.ts";

// Redacted deterministic projection of the captured seq-227 evidence. The
// original committed text is 8,339 characters. History must preserve that
// complete body, including the task-notification close tag. Identifiers and
// body text are synthetic.
const CAPTURED_TEXT_CHARS = 8_339;

function capturedNotificationText() {
  const prefix = [
    '<garyx_task_notification event="ready_for_review" task_id="#TASK-42" status="in_review">',
    "Task #TASK-42 reached review: Synthetic renderer review",
    "",
    "# Review conclusion: FAIL",
    "",
  ].join("\n");
  const suffix = [
    "",
    "View details:",
    "garyx task get #TASK-42",
    "</garyx_task_notification>",
  ].join("\n");
  const paddingLength = CAPTURED_TEXT_CHARS - prefix.length - suffix.length;
  assert.ok(paddingLength > 0);
  const text = `${prefix}${"x".repeat(paddingLength)}${suffix}`;
  assert.equal(text.length, CAPTURED_TEXT_CHARS);
  return text;
}

function capturedHistoryMessage() {
  const text = capturedNotificationText();
  return {
    id: "thread::00000000-0000-4000-8000-000000000001:226",
    seq: 227,
    role: "user",
    text,
    content: text,
    timestamp: "2026-01-01T00:00:00.000Z",
    kind: "user_input",
    internal: true,
    internalKind: null,
    metadata: {
      internal_dispatch: true,
      task_notification: true,
      task_notification_event: "ready_for_review",
      task_id: "#TASK-42",
    },
    localState: "remote_final",
  };
}

const capturedRenderState = {
  based_on_seq: 227,
  rows: [
    {
      activity: [],
      finished_at: null,
      id: "user_turn:seq:227",
      kind: "user_turn",
      started_at: null,
      user: {
        id: "seq:227",
        role: "user",
        seq: 227,
        presentation: {
          kind: "task_notification",
          event: "ready_for_review",
          status: "in_review",
          task_id: "#TASK-42",
          title: "Synthetic renderer review",
        },
      },
    },
  ],
  tailActivity: "none",
  activeToolGroupId: null,
  progress_locus: "none",
  filtered_placeholders: [],
};

async function buildRichMessageRenderer() {
  const result = await esbuild.build({
    stdin: {
      contents: [
        'import React from "react";',
        'import { renderToStaticMarkup } from "react-dom/server";',
        'import { RichMessageContent } from "./src/renderer/src/message-rich-content.tsx";',
        "export function render(props) {",
        "  return renderToStaticMarkup(React.createElement(RichMessageContent, props));",
        "}",
      ].join("\n"),
      resolveDir: process.cwd(),
      sourcefile: "task-notification-repro-ssr.mjs",
    },
    alias: {
      "@": path.resolve("src/renderer/src"),
      "@renderer": path.resolve("src/renderer/src"),
      "@shared": path.resolve("src/shared"),
    },
    banner: {
      js: [
        'import { createRequire } from "node:module";',
        'const require = createRequire(process.cwd() + "/package.json");',
      ].join("\n"),
    },
    bundle: true,
    format: "esm",
    jsx: "automatic",
    platform: "node",
    write: false,
  });
  return import(
    `data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString("base64")}`
  );
}

function visibleText(html) {
  return html.replace(/<[^>]*>/g, " ").replace(/\s+/g, " ").trim();
}

async function assertCapturedNotificationRenders(message) {
  const rows = buildThreadViewRows(
    capturedRenderState,
    new Map([[message.seq, message]]),
  );
  const userTurn = rows.find((row) => row.kind === "user_turn");
  assert.ok(userTurn, "captured user_turn row should resolve from the body cache");
  assert.deepEqual(
    userTurn.userBlock.entry.presentation,
    {
      kind: "task_notification",
      event: "ready_for_review",
      status: "in_review",
      task_id: "#TASK-42",
      title: "Synthetic renderer review",
    },
    "the server render ref selects the task-notification presentation",
  );

  const renderer = await buildRichMessageRenderer();
  const html = renderer.render({
    altPrefix: message.role,
    content: message.content,
    presentation: userTurn.userBlock.entry.presentation,
    text: message.text,
  });

  assert.match(
    html,
    /class="task-notification-card"/,
    "the complete text should select the task-notification card renderer",
  );
  assert.match(
    visibleText(html),
    /Review conclusion: FAIL/,
    "the captured notification body should remain visible",
  );

  const unclassified = renderer.render({
    altPrefix: message.role,
    content: message.content,
    text: message.text,
  });
  assert.doesNotMatch(
    unclassified,
    /class="task-notification-card"/,
    "message text alone must not select the task-notification presentation",
  );
}

test("captured long history content stays complete and renders a visible task-notification card", async () => {
  const message = capturedHistoryMessage();
  assert.equal(message.content.length, CAPTURED_TEXT_CHARS);
  assert.equal(message.content, message.text);
  assert.match(message.content, /<\/garyx_task_notification>$/);
  assert.doesNotMatch(message.content, /\[truncated:/);

  await assertCapturedNotificationRenders(message);
});
