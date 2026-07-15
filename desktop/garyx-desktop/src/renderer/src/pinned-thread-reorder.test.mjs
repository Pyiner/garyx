import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";

import { reorderPinnedThreadIds } from "./pinned-thread-reorder.ts";

test("moves a pinned thread to either segment edge", () => {
  assert.deepEqual(
    reorderPinnedThreadIds(["a", "b", "c"], "c", "a"),
    ["c", "a", "b"],
  );
  assert.deepEqual(
    reorderPinnedThreadIds(["a", "b", "c"], "a", "c"),
    ["b", "c", "a"],
  );
});

test("cancel, same-position drop, and foreign destinations do not mutate", () => {
  const order = ["a", "b", "c"];
  assert.equal(reorderPinnedThreadIds(order, "b", null), null);
  assert.equal(reorderPinnedThreadIds(order, "b", "b"), null);
  assert.equal(reorderPinnedThreadIds(order, "b", "outside"), null);
  assert.deepEqual(order, ["a", "b", "c"]);
});

test("pinned sidebar owns the vertical sortable lifecycle contract", async () => {
  const source = await readFile(
    new URL("./PinnedThreadsSidebar.tsx", import.meta.url),
    "utf8",
  );

  assert.match(source, /<DndContext/);
  assert.match(source, /<SortableContext/);
  assert.match(source, /strategy=\{verticalListSortingStrategy\}/);
  assert.match(source, /restrictToVerticalAxis/);
  assert.match(source, /onDragCancel=\{handleDragCancel\}/);
  assert.match(source, /onDragStart=\{handleDragStart\}/);
  assert.match(source, /onReorderThreads\(nextOrder\)/);
  // The rows projection emptying mid-drag must cancel the drag: the
  // component renders null while staying mounted, so the cancel keys off
  // rows change (review #TASK-2312 P2); unmount cleanup stays as defense.
  assert.match(source, /dragActiveRef\.current\s*=\s*true/);
  assert.match(
    source,
    /shouldCancelDanglingDrag\(rows\.length, dragActiveRef\.current\)/,
  );
  assert.match(source, /\}, \[rows\.length\]\);/);
  assert.match(
    source,
    /return \(\) => \{[\s\S]*?onDragCancelRef\.current\(\)/,
  );
});

test("dangling drag predicate cancels only an active drag with emptied rows", async () => {
  const { shouldCancelDanglingDrag } = await import(
    "./pinned-thread-reorder.ts"
  );
  assert.equal(shouldCancelDanglingDrag(0, true), true);
  assert.equal(shouldCancelDanglingDrag(0, false), false);
  assert.equal(shouldCancelDanglingDrag(1, true), false);
  assert.equal(shouldCancelDanglingDrag(3, false), false);
});
