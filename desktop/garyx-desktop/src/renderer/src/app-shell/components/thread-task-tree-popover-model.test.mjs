import test from "node:test";
import assert from "node:assert/strict";

import {
  buildTaskRows,
  isCurrentTaskTreeNode,
  taskStatusLabel,
  taskStatusTone,
  taskTreeBadgeCount,
  visibleTaskTreeTasks,
} from "./thread-task-tree-popover-model.ts";

function task(overrides) {
  const number = overrides.number;
  return {
    kind: "task",
    nodeId: `task:thread::${number}`,
    parentNodeId: null,
    threadId: `thread::${number}`,
    taskId: `#TASK-${number}`,
    number,
    title: `Task ${number}`,
    status: "todo",
    creator: { kind: "agent", agentId: "test-agent" },
    assignee: null,
    source: null,
    executor: null,
    updatedAt: "2026-01-01T00:00:00.000Z",
    updatedBy: { kind: "agent", agentId: "test-agent" },
    runtimeAgentId: "test-agent",
    replyCount: 0,
    parentTaskNumber: null,
    parentThreadId: null,
    activeRunId: null,
    runState: "idle",
    lastActiveAt: null,
    ...overrides,
  };
}

test("keeps task nodes regardless of status for server-pruned tree", () => {
  const threadRoot = {
    kind: "thread",
    nodeId: "thread-root:thread::root",
    threadId: "thread::root",
    title: "Pinned root",
    threadType: "chat",
    providerType: "codex",
    agentId: "codex",
    messageCount: 1,
    lastMessagePreview: "",
    activeRunId: null,
    runState: "idle",
    updatedAt: null,
    lastActiveAt: null,
  };
  const doneAncestor = task({ number: 1, status: "done" });
  const activeChild = task({
    number: 2,
    status: "in_review",
    parentNodeId: doneAncestor.nodeId,
    parentTaskNumber: 1,
    parentThreadId: doneAncestor.threadId,
  });

  assert.deepEqual(
    visibleTaskTreeTasks([threadRoot, doneAncestor, activeChild]),
    [doneAncestor, activeChild],
  );
});

test("badge counts only active tasks", () => {
  assert.equal(
    taskTreeBadgeCount([
      task({ number: 1, status: "done" }),
      task({ number: 2, status: "in_progress" }),
      task({ number: 3, status: "in_review" }),
      task({ number: 4, status: "todo" }),
    ]),
    2,
  );
});

test("current node is local to selected thread", () => {
  const current = task({ number: 2, threadId: "thread::current" });
  assert.equal(isCurrentTaskTreeNode(current, "thread::current"), true);
  assert.equal(isCurrentTaskTreeNode(current, "thread::other"), false);
});

test("rows preserve original parent edges for done ancestors", () => {
  const doneAncestor = task({ number: 1, status: "done" });
  const activeChild = task({
    number: 2,
    status: "in_progress",
    parentNodeId: doneAncestor.nodeId,
    parentTaskNumber: 1,
    parentThreadId: doneAncestor.threadId,
  });

  assert.deepEqual(
    buildTaskRows([doneAncestor, activeChild]).map(({ task, depth }) => [
      task.number,
      depth,
    ]),
    [
      [1, 0],
      [2, 1],
    ],
  );
});

test("status labels and tones include inactive ancestors", () => {
  assert.equal(taskStatusLabel("todo"), "Todo");
  assert.equal(taskStatusLabel("done"), "Done");
  assert.equal(taskStatusTone("todo"), "todo");
  assert.equal(taskStatusTone("done"), "done");
});
