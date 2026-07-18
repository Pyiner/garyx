import assert from "node:assert/strict";
import test from "node:test";

import {
  activeToolGroupChangeAffectsRow,
  renderToolProjectionEqual,
  transcriptRenderRowPropsEqual,
  turnRenderRowPresentationEqual,
} from "./thread-render-row-equality.ts";

const userMessage = {
  id: "message-user",
  seq: 1,
  role: "user",
  text: "question",
};
const toolUseMessage = {
  id: "message-tool-use",
  seq: 2,
  role: "tool_use",
  text: "run",
};
const toolResultMessage = {
  id: "message-tool-result",
  seq: 3,
  role: "tool_result",
  text: "done",
};

const projection = {
  tool_name: "commandExecution",
  kind: "command",
  visibility: "normal",
  summary: {
    root: "content",
    path: ["description"],
    format: "text",
    label: "call",
  },
  call: {
    root: "content",
    path: ["command"],
    format: "code",
    label: "command",
  },
  diff: {
    source: "tool_use",
    segments: [
      {
        unified: {
          text: { root: "content", path: ["changes", "0", "diff"] },
        },
      },
    ],
  },
  result: {
    root: "content",
    path: ["aggregatedOutput"],
    format: "code",
    label: "output",
  },
  status: "completed",
  exit_code: 0,
  duration_ms: 4,
};

function messageBlock(message) {
  return {
    kind: "message",
    key: message.id,
    entry: { kind: "message", key: message.id, message },
  };
}

function toolRow({
  nextProjection = projection,
  running = false,
  capsuleCards = [],
} = {}) {
  return {
    kind: "user_turn",
    key: "user-turn:message-user",
    userBlock: messageBlock(userMessage),
    activityRows: [
      {
        kind: "turn",
        key: "turn:step-1",
        steps: [
          {
            kind: "tool_group",
            key: "tool-group-1",
            defaultExpanded: false,
            entries: [
              {
                kind: "tool",
                key: "tool-entry-1",
                toolUse: toolUseMessage,
                toolResult: toolResultMessage,
                projection: nextProjection,
              },
            ],
          },
        ],
        finalBlock: null,
        isRunning: running,
        startedAt: "2026-01-01T00:00:00Z",
        finishedAt: running ? null : "2026-01-01T00:00:01Z",
      },
    ],
    capsuleCards,
  };
}

function comparableProps(row, overrides = {}) {
  return {
    row,
    activeToolGroupId: null,
    actions: comparableProps.actions,
    translationIdentity: comparableProps.translation,
    imagePreviewIdentity: comparableProps.imagePreview,
    canRetryFailedMessage: true,
    canOpenCapsule: true,
    ...overrides,
  };
}
comparableProps.actions = {};
comparableProps.translation = () => "translated";
comparableProps.imagePreview = () => null;

test("equal-value fresh tool projections preserve a historical row", () => {
  const freshProjection = structuredClone(projection);
  assert.notEqual(freshProjection, projection);
  assert.equal(renderToolProjectionEqual(projection, freshProjection), true);

  const previous = toolRow();
  const next = toolRow({ nextProjection: freshProjection });
  assert.notEqual(previous, next);
  assert.equal(turnRenderRowPresentationEqual(previous, next), true);
  assert.equal(
    transcriptRenderRowPropsEqual(
      comparableProps(previous),
      comparableProps(next),
    ),
    true,
  );
});

test("projection selector and running-state changes rerender", () => {
  const changedProjection = structuredClone(projection);
  changedProjection.result.path = ["differentOutput"];
  assert.equal(renderToolProjectionEqual(projection, changedProjection), false);
  assert.equal(
    turnRenderRowPresentationEqual(
      toolRow(),
      toolRow({ nextProjection: changedProjection }),
    ),
    false,
  );
  assert.equal(
    turnRenderRowPresentationEqual(toolRow(), toolRow({ running: true })),
    false,
  );

  const changedSummaryProjection = structuredClone(projection);
  changedSummaryProjection.summary.path = ["differentDescription"];
  assert.equal(
    renderToolProjectionEqual(projection, changedSummaryProjection),
    false,
  );

  const changedDiffProjection = structuredClone(projection);
  changedDiffProjection.diff.segments[0].unified.text.path = ["changes", "1", "diff"];
  assert.equal(
    renderToolProjectionEqual(projection, changedDiffProjection),
    false,
  );
  assert.equal(
    turnRenderRowPresentationEqual(
      toolRow(),
      toolRow({ nextProjection: changedDiffProjection }),
    ),
    false,
    "a frame that changes only diff segments must rerender",
  );
});

test("Capsule cards compare by value and rerender on a changed field", () => {
  const card = {
    id: "capsule-card-1",
    capsule_id: "00000000-0000-4000-8000-000000000001",
    title: "Synthetic Capsule",
    revision: 1,
    action: "created",
  };
  assert.equal(
    turnRenderRowPresentationEqual(
      toolRow({ capsuleCards: [card] }),
      toolRow({ capsuleCards: [structuredClone(card)] }),
    ),
    true,
  );
  assert.equal(
    turnRenderRowPresentationEqual(
      toolRow({ capsuleCards: [card] }),
      toolRow({ capsuleCards: [{ ...card, revision: 2 }] }),
    ),
    false,
  );
});

test("active tool changes invalidate only rows containing old or new group", () => {
  const row = toolRow();
  assert.equal(
    activeToolGroupChangeAffectsRow(row, null, "unrelated-group"),
    false,
  );
  assert.equal(
    activeToolGroupChangeAffectsRow(row, null, "tool-group-1"),
    true,
  );
  assert.equal(
    activeToolGroupChangeAffectsRow(row, "tool-group-1", null),
    true,
  );

  assert.equal(
    transcriptRenderRowPropsEqual(
      comparableProps(row, { activeToolGroupId: null }),
      comparableProps(toolRow(), { activeToolGroupId: "unrelated-group" }),
    ),
    true,
  );
  assert.equal(
    transcriptRenderRowPropsEqual(
      comparableProps(row, { activeToolGroupId: null }),
      comparableProps(toolRow(), { activeToolGroupId: "tool-group-1" }),
    ),
    false,
  );
});

test("locale, image-loader, and action availability remain render inputs", () => {
  const row = toolRow();
  const previous = comparableProps(row);
  assert.equal(
    transcriptRenderRowPropsEqual(previous, {
      ...previous,
      translationIdentity: () => "new locale",
    }),
    false,
  );
  assert.equal(
    transcriptRenderRowPropsEqual(previous, {
      ...previous,
      imagePreviewIdentity: () => null,
    }),
    false,
  );
  assert.equal(
    transcriptRenderRowPropsEqual(previous, {
      ...previous,
      canOpenCapsule: false,
    }),
    false,
  );
});
