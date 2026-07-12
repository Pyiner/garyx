import test from "node:test";
import assert from "node:assert/strict";

import {
  CONVERSATION_RAIL_DEFAULT_WIDTH,
  CONVERSATION_RAIL_MAX_WIDTH,
  CONVERSATION_RAIL_MIN_WIDTH,
  DUAL_RAIL_COMPACT_WIDTH,
  SIDEBAR_DEFAULT_WIDTH,
  SIDEBAR_MAX_WIDTH,
  SIDEBAR_MIN_WIDTH,
  SINGLE_RAIL_COMPACT_WIDTH,
  TASK_TREE_DOCK_MIN_WIDTH,
  clampConversationRailWidth,
  clampSidebarWidth,
  isDockedSidePanel,
  isDockedTaskTree,
  isCompactSidebarViewport,
  resolveSidebarCollapsed,
  responsiveSidebarBreakpoint,
} from "./responsive-layout-model.ts";

test("legacy panel resize clamps are table-driven and preserve fractional input", () => {
  assert.equal(SIDEBAR_DEFAULT_WIDTH, 245);
  assert.equal(SIDEBAR_MIN_WIDTH, 245);
  assert.equal(SIDEBAR_MAX_WIDTH, 520);
  assert.equal(CONVERSATION_RAIL_DEFAULT_WIDTH, 258);
  assert.equal(CONVERSATION_RAIL_MIN_WIDTH, 220);
  assert.equal(CONVERSATION_RAIL_MAX_WIDTH, 420);

  const cases = [
    ["sidebar below min", clampSidebarWidth, 244, 245],
    ["sidebar min", clampSidebarWidth, 245, 245],
    ["sidebar fractional", clampSidebarWidth, 311.5, 311.5],
    ["sidebar max", clampSidebarWidth, 520, 520],
    ["sidebar above max", clampSidebarWidth, 521, 520],
    ["rail below min", clampConversationRailWidth, 219, 220],
    ["rail min", clampConversationRailWidth, 220, 220],
    ["rail fractional", clampConversationRailWidth, 311.5, 311.5],
    ["rail max", clampConversationRailWidth, 420, 420],
    ["rail above max", clampConversationRailWidth, 421, 420],
  ];
  for (const [label, clamp, input, expected] of cases) {
    assert.equal(clamp(input), expected, label);
  }
  assert.equal(Number.isNaN(clampSidebarWidth(Number.NaN)), true);
  assert.equal(Number.isNaN(clampConversationRailWidth(Number.NaN)), true);
});

test("matches Codex's 720px automatic sidebar breakpoint", () => {
  assert.equal(responsiveSidebarBreakpoint(false), SINGLE_RAIL_COMPACT_WIDTH);
  assert.equal(
    isCompactSidebarViewport({
      secondaryRailOpen: false,
      viewportWidth: SINGLE_RAIL_COMPACT_WIDTH,
    }),
    true,
  );
  assert.equal(
    isCompactSidebarViewport({
      secondaryRailOpen: false,
      viewportWidth: SINGLE_RAIL_COMPACT_WIDTH + 1,
    }),
    false,
  );
});

test("collapses the global rail earlier when the conversation rail is open", () => {
  assert.equal(responsiveSidebarBreakpoint(true), DUAL_RAIL_COMPACT_WIDTH);
  assert.equal(
    isCompactSidebarViewport({
      secondaryRailOpen: true,
      viewportWidth: DUAL_RAIL_COMPACT_WIDTH,
    }),
    true,
  );
  assert.equal(
    isCompactSidebarViewport({
      secondaryRailOpen: true,
      viewportWidth: DUAL_RAIL_COMPACT_WIDTH + 1,
    }),
    false,
  );
});

test("compact mode can be manually opened without changing the desktop preference", () => {
  assert.equal(
    resolveSidebarCollapsed({
      compactOpen: false,
      compactViewport: true,
      userCollapsed: false,
    }),
    true,
  );
  assert.equal(
    resolveSidebarCollapsed({
      compactOpen: true,
      compactViewport: true,
      userCollapsed: true,
    }),
    false,
  );
  assert.equal(
    resolveSidebarCollapsed({
      compactOpen: false,
      compactViewport: false,
      userCollapsed: true,
    }),
    true,
  );
});

test("legacy compact presentation resolves every intent combination", () => {
  const cases = [
    { compactOpen: false, compactViewport: false, userCollapsed: false, expected: false },
    { compactOpen: true, compactViewport: false, userCollapsed: false, expected: false },
    { compactOpen: false, compactViewport: false, userCollapsed: true, expected: true },
    { compactOpen: true, compactViewport: false, userCollapsed: true, expected: true },
    { compactOpen: false, compactViewport: true, userCollapsed: false, expected: true },
    { compactOpen: true, compactViewport: true, userCollapsed: false, expected: false },
    { compactOpen: false, compactViewport: true, userCollapsed: true, expected: true },
    { compactOpen: true, compactViewport: true, userCollapsed: true, expected: false },
  ];
  for (const input of cases) {
    const { expected, ...state } = input;
    assert.equal(resolveSidebarCollapsed(state), expected, JSON.stringify(state));
  }
});

test("task tree docks only after the full 736px reading column still fits", () => {
  assert.equal(isDockedTaskTree(TASK_TREE_DOCK_MIN_WIDTH - 1), false);
  assert.equal(isDockedTaskTree(TASK_TREE_DOCK_MIN_WIDTH), true);
});

test("thread logs overlay before they can crush the primary thread", () => {
  assert.equal(
    isDockedSidePanel({
      canvasWidth: 829,
      panelWidth: 280,
    }),
    false,
  );
  assert.equal(
    isDockedSidePanel({
      canvasWidth: 830,
      panelWidth: 280,
    }),
    true,
  );
  assert.equal(
    isDockedSidePanel({
      canvasWidth: 736,
      panelWidth: 280,
    }),
    false,
  );
  assert.equal(
    isDockedSidePanel({
      canvasWidth: 1235,
      panelWidth: 685,
    }),
    true,
  );
});

test("legacy dock helper rejects invalid geometry and honors custom budgets", () => {
  const cases = [
    { canvasWidth: Number.NaN, panelWidth: 280, expected: false },
    { canvasWidth: 830, panelWidth: Number.NaN, expected: false },
    { canvasWidth: 0, panelWidth: 280, expected: false },
    { canvasWidth: 830, panelWidth: 0, expected: false },
    {
      canvasWidth: 710,
      panelWidth: 300,
      minMainWidth: 400,
      resizerWidth: 10,
      expected: true,
    },
    {
      canvasWidth: 709,
      panelWidth: 300,
      minMainWidth: 400,
      resizerWidth: 10,
      expected: false,
    },
  ];
  for (const { expected, ...input } of cases) {
    assert.equal(isDockedSidePanel(input), expected, JSON.stringify(input));
  }
});
