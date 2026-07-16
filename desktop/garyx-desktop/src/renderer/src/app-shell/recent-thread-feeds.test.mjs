import assert from "node:assert/strict";
import test from "node:test";

import {
  completeRecentThreadRequest,
  consumeRecentThreadMutationFollowUp,
  createRecentThreadFeedsState,
  failRecentThreadRequest,
  ingestRecentThreadSummaries,
  noteRecentThreadFilterLocalMutation,
  removeThreadFromRecentFeeds,
  requestRecentThreadLoadMore,
  requestRecentThreadRefresh,
  resetRecentThreadFeedsScope,
  rollbackRecentThreadRemoval,
  recentThreadTasksQuery,
  selectRecentThreadFilter,
  selectedRecentThreadSummaries,
  upsertChatInRecentFeeds,
} from "./recent-thread-feeds.ts";

function summary(id, title = id, threadType = "chat") {
  return {
    id,
    title,
    threadType,
    createdAt: "2026-07-11T00:00:00Z",
    updatedAt: "2026-07-11T00:00:00Z",
    lastMessagePreview: "",
  };
}

function page(scope, ids, options = {}) {
  const hasMore = options.hasMore ?? false;
  return {
    gatewayScope: scope,
    storeIncarnationId: options.storeIncarnationId ?? "incarnation-a",
    serverBootId: options.serverBootId ?? "boot-a",
    threads: ids.map((id) => summary(id)),
    count: options.count ?? ids.length,
    total: options.total ?? ids.length,
    limit: options.limit ?? 100,
    hasMore,
    nextCursor: Object.hasOwn(options, "nextCursor")
      ? options.nextCursor
      : hasMore
        ? `cursor-${ids.at(-1) || "empty"}`
        : null,
  };
}

function refresh(state, filter, ids, options = {}) {
  const decision = requestRecentThreadRefresh(state, filter);
  assert.ok(decision.ticket);
  return completeRecentThreadRequest(
    decision.state,
    decision.ticket,
    page(state.gatewayScope, ids, options),
  );
}

test("Recent feeds default to All and map both filters to explicit wire values", () => {
  const state = createRecentThreadFeedsState("https://gateway.test");
  assert.equal(state.selectedFilter, "all");
  assert.equal(recentThreadTasksQuery("all"), "include");
  assert.equal(recentThreadTasksQuery("nonTask"), "exclude");
  assert.equal(state.feeds.all.isPrimed, false);
  assert.equal(state.feeds.nonTask.isPrimed, false);
  assert.equal(state.feeds.all.refreshAfterMutation, false);
  assert.equal(state.feeds.all.loadMoreAfterMutation, false);
});

test("filter tickets own independent rows and accept late completion into their own cache", () => {
  let state = createRecentThreadFeedsState("https://gateway.test");
  const all = requestRecentThreadRefresh(state, "all");
  state = all.state;
  state = selectRecentThreadFilter(state, "nonTask");
  const chats = requestRecentThreadRefresh(state, "nonTask");
  state = chats.state;
  state = selectRecentThreadFilter(state, "all");

  state = completeRecentThreadRequest(
    state,
    chats.ticket,
    page(state.gatewayScope, ["chat-1", "chat-2"], { hasMore: true }),
  );
  assert.deepEqual(state.feeds.nonTask.orderedThreadIds, ["chat-1", "chat-2"]);
  assert.deepEqual(state.feeds.all.orderedThreadIds, []);
  assert.equal(state.selectedFilter, "all");

  state = completeRecentThreadRequest(
    state,
    all.ticket,
    page(state.gatewayScope, ["task-1", "chat-1"]),
  );
  assert.deepEqual(state.feeds.all.orderedThreadIds, ["task-1", "chat-1"]);
  assert.deepEqual(
    selectedRecentThreadSummaries(state).map((thread) => thread.id),
    ["task-1", "chat-1"],
  );
});

test("scope reset abandons old epochs and resets selection without leaking summaries", () => {
  let state = createRecentThreadFeedsState("https://gateway-a.test");
  const decision = requestRecentThreadRefresh(state, "all");
  state = selectRecentThreadFilter(decision.state, "nonTask");
  state = resetRecentThreadFeedsScope(state, "https://gateway-b.test");
  state = completeRecentThreadRequest(
    state,
    decision.ticket,
    page("https://gateway-a.test", ["old-thread"]),
  );
  assert.equal(state.selectedFilter, "all");
  assert.deepEqual(state.feeds.all.orderedThreadIds, []);
  assert.deepEqual(state.summariesById, {});
});

test("successful empty pages are primed while first failures remain unavailable", () => {
  let empty = createRecentThreadFeedsState("https://gateway.test");
  empty = refresh(empty, "all", []);
  assert.equal(empty.feeds.all.isPrimed, true);
  assert.deepEqual(empty.feeds.all.orderedThreadIds, []);
  assert.equal(empty.feeds.all.headFailure, null);

  let failed = createRecentThreadFeedsState("https://gateway.test");
  const decision = requestRecentThreadRefresh(failed, "nonTask");
  failed = failRecentThreadRequest(
    decision.state,
    decision.ticket,
    "offline",
  );
  assert.equal(failed.feeds.nonTask.isPrimed, false);
  assert.equal(failed.feeds.nonTask.headFailure, "offline");
});

test("cursor pages replace the head and append load-more rows", () => {
  let state = createRecentThreadFeedsState("https://gateway.test");
  state = refresh(state, "all", ["a", "b", "c"], {
    count: 3,
    total: 9,
    hasMore: true,
  });

  let load = requestRecentThreadLoadMore(state, "all");
  assert.equal(load.ticket.cursor, "cursor-c");
  state = completeRecentThreadRequest(
    load.state,
    load.ticket,
    page(state.gatewayScope, ["d", "e", "f"], {
      count: 3,
      total: 9,
      hasMore: true,
    }),
  );
  assert.deepEqual(state.feeds.all.orderedThreadIds, ["a", "b", "c", "d", "e", "f"]);
  assert.equal(state.feeds.all.nextCursor, "cursor-f");

  const head = requestRecentThreadRefresh(state, "all");
  state = completeRecentThreadRequest(
    head.state,
    head.ticket,
    page(state.gatewayScope, ["new", "a", "b"], {
      count: 3,
      total: 10,
      hasMore: true,
    }),
  );
  assert.deepEqual(state.feeds.all.orderedThreadIds, ["new", "a", "b"]);
  assert.equal(state.feeds.all.nextCursor, "cursor-b");

  load = requestRecentThreadLoadMore(state, "all");
  assert.equal(load.ticket.cursor, "cursor-b");
  state = completeRecentThreadRequest(
    load.state,
    load.ticket,
    page(state.gatewayScope, ["a", "b", "c", "d", "e", "f", "g"], {
      count: 5,
      total: 8,
      hasMore: false,
    }),
  );
  assert.deepEqual(state.feeds.all.orderedThreadIds, ["new", "a", "b", "c", "d", "e", "f", "g"]);
  assert.equal(state.feeds.all.nextCursor, null);
  assert.equal(state.feeds.all.loadGate, "exhausted");
});

test("each filter owns its cursor and coalesces duplicate load-more triggers", () => {
  let state = createRecentThreadFeedsState("https://gateway.test");
  state = refresh(state, "all", ["task", "chat-a", "chat-b"], {
    count: 3,
    total: 20,
    hasMore: true,
  });
  state = refresh(state, "nonTask", ["chat-a"], {
    count: 1,
    total: 10,
    hasMore: true,
  });

  const allLoad = requestRecentThreadLoadMore(state, "all");
  assert.ok(allLoad.ticket);
  assert.equal(requestRecentThreadLoadMore(allLoad.state, "all").ticket, null);
  const chatsLoad = requestRecentThreadLoadMore(allLoad.state, "nonTask");
  assert.ok(chatsLoad.ticket);
  assert.equal(allLoad.ticket.cursor, "cursor-chat-b");
  assert.equal(chatsLoad.ticket.cursor, "cursor-chat-a");

  state = completeRecentThreadRequest(
    chatsLoad.state,
    chatsLoad.ticket,
    page(state.gatewayScope, ["chat-b", "chat-c", "chat-d"], {
      count: 3,
      total: 10,
      hasMore: true,
    }),
  );
  state = completeRecentThreadRequest(
    state,
    allLoad.ticket,
    page(state.gatewayScope, ["tail-a", "tail-b", "tail-c"], {
      count: 3,
      total: 20,
      hasMore: true,
    }),
  );
  assert.equal(state.feeds.all.nextCursor, "cursor-tail-c");
  assert.equal(state.feeds.nonTask.nextCursor, "cursor-chat-d");
});

test("load-more failures require explicit retry and do not contaminate the other feed", () => {
  let state = createRecentThreadFeedsState("https://gateway.test");
  state = refresh(state, "all", ["a"], { hasMore: true });
  const load = requestRecentThreadLoadMore(state, "all");
  state = failRecentThreadRequest(load.state, load.ticket, "offline");
  assert.equal(state.feeds.all.loadGate, "failed");
  assert.equal(requestRecentThreadLoadMore(state, "all").ticket, null);
  assert.ok(requestRecentThreadLoadMore(state, "all", true).ticket);
  assert.equal(state.feeds.nonTask.loadGate, "ready");
});

test("a failed cached head refresh preserves rows and only marks its ticket feed", () => {
  let state = createRecentThreadFeedsState("https://gateway.test");
  state = refresh(state, "all", ["task", "chat"]);
  state = refresh(state, "nonTask", ["chat"]);
  const failed = requestRecentThreadRefresh(state, "nonTask");
  state = failRecentThreadRequest(failed.state, failed.ticket, "offline");

  assert.deepEqual(state.feeds.nonTask.orderedThreadIds, ["chat"]);
  assert.equal(state.feeds.nonTask.headFailure, "offline");
  assert.equal(state.feeds.all.headFailure, null);
});

test("local archive surgery blocks stale pages and rollback restores both feed orders", () => {
  let state = createRecentThreadFeedsState("https://gateway.test");
  state = refresh(state, "all", ["task", "chat", "tail"], { hasMore: true });
  state = refresh(state, "nonTask", ["chat", "tail"], { hasMore: true });
  const stale = requestRecentThreadRefresh(state, "all");
  const removed = removeThreadFromRecentFeeds(stale.state, "chat");
  state = removed.state;
  assert.deepEqual(state.feeds.all.orderedThreadIds, ["task", "tail"]);
  assert.deepEqual(state.feeds.nonTask.orderedThreadIds, ["tail"]);

  state = completeRecentThreadRequest(
    state,
    stale.ticket,
    page(state.gatewayScope, ["task", "chat", "tail"]),
  );
  assert.deepEqual(state.feeds.all.orderedThreadIds, ["task", "tail"]);

  // A page requested after local removal can still race the server commit.
  // The session tombstone must reject that row too, not just tickets issued
  // before the mutation sequence changed.
  const postRemoval = requestRecentThreadRefresh(state, "nonTask");
  state = completeRecentThreadRequest(
    postRemoval.state,
    postRemoval.ticket,
    page(state.gatewayScope, ["chat", "tail"]),
  );
  assert.deepEqual(state.feeds.nonTask.orderedThreadIds, ["tail"]);

  state = rollbackRecentThreadRemoval(state, removed.rollback);
  assert.deepEqual(state.feeds.all.orderedThreadIds, ["task", "chat", "tail"]);
  assert.deepEqual(state.feeds.nonTask.orderedThreadIds, ["chat", "tail"]);
});

test("new chats upsert both feeds without deriving task membership from cached rows", () => {
  let state = createRecentThreadFeedsState("https://gateway.test");
  state = refresh(state, "all", ["task", "chat"]);
  state = refresh(state, "nonTask", ["chat"]);
  state = upsertChatInRecentFeeds(state, summary("new-chat", "New chat"));
  assert.deepEqual(state.feeds.all.orderedThreadIds, ["new-chat", "task", "chat"]);
  assert.deepEqual(state.feeds.nonTask.orderedThreadIds, ["new-chat", "chat"]);
});

test("new task invalidation is owned by All and leaves Chats requests current", () => {
  let state = createRecentThreadFeedsState("https://gateway.test");
  state = refresh(state, "all", ["task", "chat"]);
  state = refresh(state, "nonTask", ["chat"]);
  const chats = requestRecentThreadRefresh(state, "nonTask");
  const allSequence = chats.state.feeds.all.localMutationSequence;
  const chatsSequence = chats.state.feeds.nonTask.localMutationSequence;

  state = noteRecentThreadFilterLocalMutation(chats.state, "all");
  assert.equal(state.feeds.all.localMutationSequence, allSequence + 1);
  assert.equal(state.feeds.nonTask.localMutationSequence, chatsSequence);

  state = completeRecentThreadRequest(
    state,
    chats.ticket,
    page(state.gatewayScope, ["chat", "new-chat"]),
  );
  assert.deepEqual(state.feeds.nonTask.orderedThreadIds, ["chat", "new-chat"]);
});

test("a refresh abandoned by local mutation structurally schedules a fresh ticket", () => {
  let state = createRecentThreadFeedsState("https://gateway.test");
  const stale = requestRecentThreadRefresh(state, "all");
  state = noteRecentThreadFilterLocalMutation(stale.state, "all");
  state = completeRecentThreadRequest(
    state,
    stale.ticket,
    page(state.gatewayScope, ["stale-row"]),
  );
  assert.equal(state.feeds.all.isPrimed, false);
  assert.equal(state.feeds.all.isRefreshingHead, false);
  assert.equal(state.feeds.all.refreshAfterMutation, true);

  const followUp = consumeRecentThreadMutationFollowUp(
    state,
    "all",
    "refresh",
  );
  assert.ok(followUp.ticket);
  assert.equal(followUp.state.feeds.all.isRefreshingHead, true);
  assert.equal(followUp.state.feeds.all.refreshAfterMutation, false);
  state = completeRecentThreadRequest(
    followUp.state,
    followUp.ticket,
    page(state.gatewayScope, ["fresh-row"]),
  );
  assert.equal(state.feeds.all.isPrimed, true);
  assert.deepEqual(state.feeds.all.orderedThreadIds, ["fresh-row"]);
});

test("a load-more abandoned by local mutation reissues its owned filter window", () => {
  let state = createRecentThreadFeedsState("https://gateway.test");
  state = refresh(state, "nonTask", ["chat-a", "chat-b"], {
    count: 2,
    total: 10,
    hasMore: true,
  });
  const stale = requestRecentThreadLoadMore(state, "nonTask");
  state = noteRecentThreadFilterLocalMutation(stale.state, "nonTask");
  state = completeRecentThreadRequest(
    state,
    stale.ticket,
    page(state.gatewayScope, ["chat-a", "chat-b", "stale-chat"], {
      count: 3,
      total: 10,
      hasMore: true,
    }),
  );
  assert.equal(state.feeds.nonTask.loadMoreAfterMutation, true);
  assert.equal(state.feeds.nonTask.nextCursor, "cursor-chat-b");

  const followUp = consumeRecentThreadMutationFollowUp(
    state,
    "nonTask",
    "loadMore",
  );
  assert.ok(followUp.ticket);
  assert.equal(followUp.ticket.filter, "nonTask");
  assert.equal(followUp.ticket.cursor, "cursor-chat-b");
  assert.equal(followUp.state.feeds.nonTask.loadMoreAfterMutation, false);
});

test("the renderer adopts each server page verbatim and never post-filters thread kinds", () => {
  let state = createRecentThreadFeedsState("https://gateway.test");
  const decision = requestRecentThreadRefresh(state, "nonTask");
  state = completeRecentThreadRequest(decision.state, decision.ticket, {
    ...page(state.gatewayScope, ["server-owned-row"]),
    threads: [summary("server-owned-row", "Server owned", "task")],
  });
  assert.deepEqual(state.feeds.nonTask.orderedThreadIds, ["server-owned-row"]);
});

test("canonical Recent membership never truncates the shared DesktopState cache", () => {
  const desktopThreads = [
    summary("task", "Task", "task"),
    summary("chat", "Chat"),
    summary("generated", "Automation generated"),
    summary("hidden-side-chat", "Hidden side chat"),
  ];
  let state = createRecentThreadFeedsState("https://gateway.test");
  state = ingestRecentThreadSummaries(state, desktopThreads);
  state = refresh(state, "all", ["task", "chat"]);
  state = refresh(state, "nonTask", ["chat"]);

  assert.deepEqual(state.feeds.all.orderedThreadIds, ["task", "chat"]);
  assert.deepEqual(state.feeds.nonTask.orderedThreadIds, ["chat"]);
  assert.equal(state.summariesById.generated.title, "Automation generated");
  assert.equal(state.summariesById["hidden-side-chat"].title, "Hidden side chat");
  assert.deepEqual(
    desktopThreads.map((thread) => thread.id),
    ["task", "chat", "generated", "hidden-side-chat"],
  );
});
