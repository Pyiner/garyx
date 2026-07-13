import assert from "node:assert/strict";
import test from "node:test";

import { resolveHorizontalLayoutPolicy } from "./contracts/window-layout.ts";

test("expand-v1 is the default and every feature-off spelling restores legacy", () => {
  assert.equal(resolveHorizontalLayoutPolicy(undefined), "expand-v1");
  assert.equal(resolveHorizontalLayoutPolicy("1"), "expand-v1");
  assert.equal(resolveHorizontalLayoutPolicy("expand-v1"), "expand-v1");
  for (const value of ["0", "false", "off", "legacy", " FALSE "]) {
    assert.equal(resolveHorizontalLayoutPolicy(value), "legacy");
  }
});
