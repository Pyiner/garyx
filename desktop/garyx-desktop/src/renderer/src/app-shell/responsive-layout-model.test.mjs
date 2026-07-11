import test from "node:test";
import assert from "node:assert/strict";

import {
  DUAL_RAIL_COMPACT_WIDTH,
  SINGLE_RAIL_COMPACT_WIDTH,
  TASK_TREE_DOCK_MIN_WIDTH,
  isDockedSidePanel,
  isDockedTaskTree,
  isCompactSidebarViewport,
  resolveSidebarCollapsed,
  responsiveSidebarBreakpoint,
} from "./responsive-layout-model.ts";

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
