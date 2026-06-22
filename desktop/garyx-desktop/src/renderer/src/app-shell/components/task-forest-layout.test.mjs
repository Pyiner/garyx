import test from "node:test";
import assert from "node:assert/strict";

import {
  buildTaskForestLayout,
  taskForestParentNumberForTest,
  visibleTaskForestNodeNumbers,
} from "./task-forest-layout.ts";

function task(overrides) {
  const number = overrides.number;
  return {
    threadId: `thread::${number}`,
    taskId: `#TASK-${number}`,
    number,
    title: `Task ${number}`,
    status: "todo",
    creator: { kind: "agent", agentId: "claude" },
    assignee: null,
    source: null,
    executor: null,
    updatedAt: `2026-01-01T00:00:${String(number).padStart(2, "0")}.000Z`,
    updatedBy: { kind: "agent", agentId: "claude" },
    runtimeAgentId: "claude",
    replyCount: 0,
    runState: "idle",
    activeRunId: null,
    lastActiveAt: null,
    ...overrides,
  };
}

test("lays out parent and child in right-growing columns", () => {
  const layout = buildTaskForestLayout([
    task({ number: 1 }),
    task({ number: 2, parentTaskNumber: 1 }),
  ]);

  const parent = layout.nodes.find((node) => node.task.number === 1);
  const child = layout.nodes.find((node) => node.task.number === 2);
  assert.ok(parent);
  assert.ok(child);
  assert.equal(parent.depth, 0);
  assert.equal(child.depth, 1);
  assert.ok(child.x > parent.x);
  assert.deepEqual(layout.edges.map((edge) => [edge.from, edge.to]), [[1, 2]]);
  assert.match(layout.edges[0].path, /^M /);
});

test("uses rounded orthogonal edge paths when nodes are vertically offset", () => {
  const layout = buildTaskForestLayout([
    task({ number: 1 }),
    task({ number: 2, parentTaskNumber: 1 }),
    task({ number: 3, parentTaskNumber: 1 }),
  ]);

  assert.ok(
    layout.edges.some((edge) => /^M .* L .* Q .* L .* Q .* L .*/.test(edge.path)),
  );
});

test("treats filtered children with missing parents as roots", () => {
  const layout = buildTaskForestLayout([
    task({ number: 7, parentTaskNumber: 1 }),
    task({ number: 8, source: { taskId: "#TASK-2" } }),
  ]);

  assert.deepEqual(
    layout.nodes.map((node) => node.depth),
    [0, 0],
  );
  assert.equal(layout.edges.length, 0);
});

test("parses legacy source task id when explicit parent field is absent", () => {
  const parent = task({ number: 11 });
  const child = task({
    number: 12,
    source: { taskId: "#TASK-11" },
  });
  const layout = buildTaskForestLayout([parent, child]);

  assert.equal(taskForestParentNumberForTest(child), 11);
  assert.deepEqual(layout.edges.map((edge) => [edge.from, edge.to]), [[11, 12]]);
});

test("summarizes hidden descendants at the depth cap", () => {
  const layout = buildTaskForestLayout(
    [
      task({ number: 1 }),
      task({ number: 2, parentTaskNumber: 1 }),
      task({ number: 3, parentTaskNumber: 2 }),
      task({ number: 4, parentTaskNumber: 3, status: "done" }),
    ],
    { maxDepth: 3 },
  );
  const capped = layout.nodes.find((node) => node.task.number === 3);

  assert.ok(capped);
  assert.equal(capped.hiddenDescendantCount, 1);
  assert.equal(capped.descendantStatusCounts.done, 1);
});

test("selects visible nodes with viewport overscan", () => {
  const layout = buildTaskForestLayout(
    [
      task({ number: 1 }),
      task({ number: 2 }),
      task({ number: 3 }),
    ],
    { rootGap: 260 },
  );
  const [first, second, third] = layout.nodes.slice().sort((left, right) => left.y - right.y);

  assert.ok(first);
  assert.ok(second);
  assert.ok(third);
  const visible = visibleTaskForestNodeNumbers(layout.nodes, {
    minX: first.x - 10,
    minY: first.y - 10,
    maxX: first.x + first.width + 10,
    maxY: first.y + first.height + 10,
  });
  assert.deepEqual([...visible], [first.task.number]);

  const overscanned = visibleTaskForestNodeNumbers(
    layout.nodes,
    {
      minX: first.x - 10,
      minY: first.y - 10,
      maxX: first.x + first.width + 10,
      maxY: first.y + first.height + 10,
    },
    second.y - first.y,
  );
  assert.ok(overscanned.has(second.task.number));
  assert.equal(overscanned.has(third.task.number), false);
});
