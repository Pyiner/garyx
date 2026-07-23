import assert from "node:assert/strict";
import test from "node:test";

import { shouldDismissWorkspaceMenuOnPointerDown } from "./workspace-menu-pointer.ts";

function pointerTargetInside(selectors) {
  return {
    closest(selector) {
      return selectors.has(selector) ? this : null;
    },
  };
}

test("pointerdown inside portaled workspace menu content stays open for item selection", () => {
  const portaledMenuItem = pointerTargetInside(
    new Set(["[data-workspace-menu-content]"]),
  );

  assert.equal(
    shouldDismissWorkspaceMenuOnPointerDown(portaledMenuItem),
    false,
  );
});

test("pointerdown outside workspace actions and menu content dismisses the menu", () => {
  assert.equal(
    shouldDismissWorkspaceMenuOnPointerDown(pointerTargetInside(new Set())),
    true,
  );
});
