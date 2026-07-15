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
  assert.match(source, /onDragCancel=\{onDragCancel\}/);
  assert.match(source, /onDragStart=\{onDragStart\}/);
  assert.match(source, /onReorderThreads\(nextOrder\)/);
});
