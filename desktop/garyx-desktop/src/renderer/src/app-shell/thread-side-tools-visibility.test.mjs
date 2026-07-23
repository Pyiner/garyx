import assert from "node:assert/strict";
import { test } from "node:test";

const { ThreadSideToolsVisibility } = await import(
  "./thread-side-tools-visibility.ts"
);

test("side-tools visibility is isolated by source thread", () => {
  const sourceA = "thread::source-a";
  const sourceB = "thread::source-b";
  const visibility = new ThreadSideToolsVisibility(sourceA, false);

  visibility.setOpen(sourceA, true);
  visibility.updatePanel(sourceA, (current) => ({
    ...current,
    openTools: ["chat"],
    activeTabKey: "chat",
  }));
  assert.equal(visibility.isOpen(sourceA), true, "A remembers its open rail");
  assert.equal(
    visibility.isOpen(sourceB),
    false,
    "a new source thread defaults to closed",
  );

  visibility.setOpen(sourceB, true);
  visibility.setOpen(sourceB, false);
  assert.equal(visibility.isOpen(sourceB), false, "B owns its closed state");
  assert.equal(
    visibility.isOpen(sourceA),
    true,
    "returning to A restores A's open state",
  );
  assert.deepEqual(
    visibility.panelFor(sourceA),
    {
      open: true,
      openTools: ["chat"],
      activeTabKey: "chat",
    },
    "A restores the side-chat tab, not an empty rail",
  );
});

test("restored visibility binds once when the initial route hydrates", () => {
  const sourceA = "thread::source-a";
  const sourceB = "thread::source-b";
  const visibility = new ThreadSideToolsVisibility(null, true);

  assert.equal(
    visibility.isOpen(null),
    true,
    "restored native occupancy stays projected before route hydration",
  );
  visibility.adoptInitialSource(sourceA);
  assert.equal(visibility.isOpen(sourceA), true);
  assert.equal(
    visibility.isOpen(sourceB),
    false,
    "the restored state does not leak past its first committed source",
  );
});
