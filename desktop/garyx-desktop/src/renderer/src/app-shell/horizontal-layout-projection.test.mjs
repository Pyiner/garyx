import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  EXPAND_V1_SIDE_TOOLS_MIN_WIDTH,
  EXPAND_V1_WINDOW_MIN_WIDTH,
  LEGACY_SIDE_TOOLS_MIN_WIDTH,
  LEGACY_WINDOW_MIN_WIDTH,
  MIN_PRIMARY_THREAD_WIDTH,
  createHorizontalLayoutState,
  horizontalLayoutColumnSum,
  horizontalLayoutPolicy,
  projectHorizontalLayout,
  reduceHorizontalLayout,
} from "./responsive-layout-model.ts";

const legacyOracle = JSON.parse(
  readFileSync(
    new URL(
      "./fixtures/legacy-horizontal-layout-oracle.json",
      import.meta.url,
    ),
    "utf8",
  ),
);
const expandGolden = JSON.parse(
  readFileSync(
    new URL(
      "./fixtures/expand-v1-horizontal-layout-golden.json",
      import.meta.url,
    ),
    "utf8",
  ),
);
const modelSource = readFileSync(
  new URL("./responsive-layout-model.ts", import.meta.url),
  "utf8",
);
const protocolSource = readFileSync(
  new URL("./window-layout-protocol.ts", import.meta.url),
  "utf8",
);

function snapshot({
  width,
  mode = "normal",
  origin = "hydrate",
  revision = 1,
}) {
  const bounds = { x: 100, y: 80, width, height: 800 };
  return {
    windowRevision: revision,
    bounds,
    contentBounds: { x: 0, y: 0, width, height: 800 },
    normalBounds: bounds,
    workArea: { x: 0, y: 0, width: 4000, height: 1400 },
    mode,
    displayId: "synthetic-display",
    scaleFactor: 2,
    origin,
  };
}

function stableProjection({
  policy,
  width,
  desiredOccupancy,
  widths,
  mode,
}) {
  const frame = projectHorizontalLayout(
    createHorizontalLayoutState({
      policy,
      rendererEpoch: "projection-test",
      snapshot: snapshot({ width, mode }),
      desiredOccupancy,
      widths,
    }),
  );
  assert.equal(frame.kind, "stable");
  return frame;
}

function pixelTracks(value) {
  return [...value.matchAll(/(-?\d+(?:\.\d+)?)px/g)].map((match) =>
    Number(match[1]),
  );
}

function columnVector(frame) {
  return [
    frame.columns.globalSidebar,
    frame.columns.conversationRail,
    frame.columns.conversationDivider,
    frame.columns.primaryThread,
    frame.columns.sideToolsResizer,
    frame.columns.sideTools,
  ];
}

test("legacy projection shadows every Phase 0 packaged structure scenario", () => {
  assert.equal(legacyOracle.policy, "legacy");
  for (const scenario of legacyOracle.scenarios) {
    const frame = stableProjection({
      policy: "legacy",
      width: scenario.viewport.width,
      desiredOccupancy: scenario.desiredOccupancy,
    });

    const expectedShellTracks = frame.columns.conversationRail
      ? [
          frame.nestedColumns.shell.globalSidebar,
          frame.nestedColumns.shell.conversationRail,
          frame.nestedColumns.shell.main,
        ]
      : [
          frame.nestedColumns.shell.globalSidebar,
          frame.nestedColumns.shell.main,
        ];
    const expectedConversationTracks = frame.columns.sideTools
      ? [
          frame.nestedColumns.conversation.threadLayout,
          frame.nestedColumns.conversation.sideToolsResizer,
          frame.nestedColumns.conversation.sideTools,
        ]
      : [frame.nestedColumns.conversation.threadLayout];
    const expectedThreadTracks = [frame.nestedColumns.thread.main];

    assert.deepEqual(
      pixelTracks(scenario.elements.appShell.computed.gridTemplateColumns),
      expectedShellTracks,
      `${scenario.name}: shell tracks`,
    );
    assert.deepEqual(
      pixelTracks(scenario.elements.conversation.computed.gridTemplateColumns),
      expectedConversationTracks,
      `${scenario.name}: conversation tracks`,
    );
    assert.deepEqual(
      pixelTracks(scenario.elements.threadLayout.computed.gridTemplateColumns),
      expectedThreadTracks,
      `${scenario.name}: thread tracks`,
    );
    assert.equal(
      frame.presentation.globalSidebar,
      scenario.presentation.globalSidebar,
      `${scenario.name}: sidebar presentation`,
    );
    assert.equal(
      frame.presentation.conversationRail,
      scenario.presentation.conversationRail === "closed" ? "closed" : "open",
      `${scenario.name}: rail presentation`,
    );
    assert.equal(
      frame.presentation.sideTools,
      scenario.presentation.sideTools,
      `${scenario.name}: side-tools presentation`,
    );
    assert.equal(
      frame.presentation.taskTree,
      scenario.presentation.taskTree,
      `${scenario.name}: task-tree presentation`,
    );
  }
});

test("expand-v1 intentional differences are isolated in a golden matrix", () => {
  assert.equal(expandGolden.policy, "expand-v1");
  for (const scenario of expandGolden.scenarios) {
    const frame = stableProjection({
      policy: "expand-v1",
      width: scenario.width,
      desiredOccupancy: scenario.desiredOccupancy,
    });
    const { reasons, ...presentation } = frame.presentation;
    assert.deepEqual(presentation, scenario.presentation, scenario.name);
    assert.deepEqual(
      reasons,
      scenario.reasons,
      `${scenario.name}: presentation reasons`,
    );
    assert.deepEqual(columnVector(frame), scenario.columns, scenario.name);
  }
  assert.deepEqual(horizontalLayoutPolicy("legacy"), {
    name: "legacy",
    windowMinWidth: LEGACY_WINDOW_MIN_WIDTH,
    windowExpansionEnabled: false,
    conversationRailAutoHide: false,
    sideToolsAutoHide: false,
    sideToolsMinWidth: LEGACY_SIDE_TOOLS_MIN_WIDTH,
  });
  assert.deepEqual(horizontalLayoutPolicy("expand-v1"), {
    name: "expand-v1",
    windowMinWidth: EXPAND_V1_WINDOW_MIN_WIDTH,
    windowExpansionEnabled: true,
    conversationRailAutoHide: true,
    sideToolsAutoHide: true,
    sideToolsMinWidth: EXPAND_V1_SIDE_TOOLS_MIN_WIDTH,
  });
  const legacyAt960 = stableProjection({
    policy: "legacy",
    width: 960,
    desiredOccupancy: {
      globalSidebar: true,
      conversationRail: false,
      sideTools: true,
    },
  });
  assert.equal(legacyAt960.presentation.sideTools, "docked");
});

test("both policies satisfy stable geometry invariants across the full matrix", () => {
  const widths = [
    480, 720, 721, 960, 961, 980, 981, 1116, 1280, 1480, 1920,
  ];
  const modes = ["normal", "maximized", "fullscreen"];
  for (const policy of ["legacy", "expand-v1"]) {
    for (const width of widths) {
      for (const mode of modes) {
        for (const globalSidebar of [false, true]) {
          for (const conversationRail of [false, true]) {
            for (const sideTools of [false, true]) {
              const requested = {
                globalSidebar,
                conversationRail,
                sideTools,
              };
              const frame = stableProjection({
                policy,
                width,
                mode,
                desiredOccupancy: requested,
              });
              const label = JSON.stringify({ policy, width, mode, requested });
              assert.equal(horizontalLayoutColumnSum(frame), width, label);
              assert.ok(
                frame.primaryThreadWidth >= MIN_PRIMARY_THREAD_WIDTH,
                label,
              );
              assert.deepEqual(frame.requestedOccupancy, requested, label);
              if (frame.presentation.sideTools === "hidden") {
                assert.equal(
                  frame.effectiveOccupancy.sideTools,
                  false,
                  label,
                );
              }
            }
          }
        }
      }
    }
  }
});

test("explicit compact sidebar presentation stays in flow", () => {
  const state = createHorizontalLayoutState({
    policy: "expand-v1",
    rendererEpoch: "compact-test",
    snapshot: snapshot({ width: 720 }),
    desiredOccupancy: {
      globalSidebar: true,
      conversationRail: false,
      sideTools: false,
    },
  });
  const compactOpen = { ...state, compactSidebarOpen: true };
  const frame = projectHorizontalLayout(compactOpen);
  assert.equal(frame.kind, "stable");
  assert.equal(frame.presentation.globalSidebar, "expanded");
  assert.equal(frame.columns.globalSidebar, 245);
  assert.equal(frame.requestedOccupancy.globalSidebar, true);
});

test("legacy compact sidebar remains a temporary overlay without changing intent", () => {
  const state = createHorizontalLayoutState({
    policy: "legacy",
    rendererEpoch: "legacy-compact-test",
    snapshot: snapshot({ width: 720 }),
    desiredOccupancy: {
      globalSidebar: false,
      conversationRail: false,
      sideTools: false,
    },
  });
  const toggled = reduceHorizontalLayout(state, {
    type: "COMPACT_SIDEBAR_TOGGLED",
  });
  const frame = projectHorizontalLayout(toggled.state);

  assert.equal(frame.kind, "stable");
  assert.equal(frame.presentation.globalSidebar, "compact-overlay");
  assert.equal(frame.columns.globalSidebar, 0);
  assert.equal(frame.requestedOccupancy.globalSidebar, false);
});

test("invalid viewport is rejected instead of manufacturing a stable frame", () => {
  const frame = projectHorizontalLayout(
    createHorizontalLayoutState({
      policy: "expand-v1",
      rendererEpoch: "invalid-viewport",
      snapshot: snapshot({ width: MIN_PRIMARY_THREAD_WIDTH - 1 }),
    }),
  );
  assert.deepEqual(frame, {
    kind: "rejected",
    policy: "expand-v1",
    revision: 0,
    reason: "invalid-viewport",
    triggerPanel: null,
    frame: null,
  });
});

test("layout policy module stays headless and side-effect free", () => {
  for (const source of [modelSource, protocolSource]) {
    assert.doesNotMatch(source, /\b(?:document|localStorage|ResizeObserver)\b/);
    assert.doesNotMatch(source, /\b(?:window|globalThis)\s*\./);
    assert.doesNotMatch(source, /from\s+["'](?:react|electron)["']/);
    assert.doesNotMatch(source, /\bDate\.now\b|\bsetTimeout\b/);
  }
});
