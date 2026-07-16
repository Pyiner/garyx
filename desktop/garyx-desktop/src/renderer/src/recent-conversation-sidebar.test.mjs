import assert from "node:assert/strict";
import test from "node:test";

import {
  recentConversationPresentation,
  recentFilterForArrowKey,
} from "./recent-conversation-sidebar-model.ts";
import { threadRailIsNearListEnd } from "./thread-conversation-sidebar-model.ts";

test("Recent segmented tabs switch with both arrow keys", () => {
  assert.equal(recentFilterForArrowKey("all", "ArrowRight"), "nonTask");
  assert.equal(recentFilterForArrowKey("nonTask", "ArrowLeft"), "all");
  assert.equal(recentFilterForArrowKey("all", "ArrowLeft"), "nonTask");
  assert.equal(recentFilterForArrowKey("nonTask", "ArrowRight"), "all");
});

test("shared rail near-tail seam triggers only inside the threshold", () => {
  assert.equal(
    threadRailIsNearListEnd({
      clientHeight: 400,
      scrollHeight: 1_000,
      scrollTop: 439,
    }),
    false,
  );
  assert.equal(
    threadRailIsNearListEnd({
      clientHeight: 400,
      scrollHeight: 1_000,
      scrollTop: 440,
    }),
    true,
  );
  assert.equal(
    threadRailIsNearListEnd({
      clientHeight: 500,
      scrollHeight: 320,
      scrollTop: 0,
    }),
    true,
  );
});

function feed(overrides = {}) {
  return {
    orderedThreadIds: [],
    isPrimed: false,
    isRefreshingHead: false,
    isLoadingMore: false,
    headFailure: null,
    loadGate: "ready",
    nextCursor: null,
    epoch: 0,
    localMutationSequence: 0,
    loadMoreFailureRevision: 0,
    activeRefreshRequestId: null,
    activeLoadMoreRequestId: null,
    refreshAfterMutation: false,
    loadMoreAfterMutation: false,
    ...overrides,
  };
}

test("Recent presentation distinguishes initial, empty, and cached refresh states", () => {
  assert.deepEqual(recentConversationPresentation(feed(), 0, "all"), {
    emptyLabelKey: null,
    footerKind: "initialLoading",
  });
  assert.deepEqual(
    recentConversationPresentation(feed({ isPrimed: true }), 0, "all"),
    { emptyLabelKey: "No recent threads", footerKind: "hidden" },
  );
  assert.deepEqual(
    recentConversationPresentation(feed({ isPrimed: true }), 0, "nonTask"),
    { emptyLabelKey: "No recent chats", footerKind: "hidden" },
  );
  assert.deepEqual(
    recentConversationPresentation(
      feed({ isPrimed: true, isRefreshingHead: true }),
      3,
      "all",
    ),
    { emptyLabelKey: null, footerKind: "hidden" },
  );
  assert.equal(
    recentConversationPresentation(
      feed({ headFailure: "offline" }),
      0,
      "all",
    ).footerKind,
    "initialFailure",
  );
  assert.equal(
    recentConversationPresentation(
      feed({ isPrimed: true, headFailure: "offline" }),
      3,
      "all",
    ).footerKind,
    "cachedRefreshFailure",
  );
});

test("Recent presentation maps every load-more footer gate", () => {
  assert.equal(
    recentConversationPresentation(
      feed({ isPrimed: true, isLoadingMore: true }),
      3,
      "all",
    ).footerKind,
    "loadingMore",
  );
  assert.equal(
    recentConversationPresentation(
      feed({ isPrimed: true, loadGate: "failed" }),
      3,
      "all",
    ).footerKind,
    "loadMoreFailure",
  );
  assert.equal(
    recentConversationPresentation(
      feed({ isPrimed: true, nextCursor: "cursor-next" }),
      3,
      "all",
    ).footerKind,
    "idle",
  );
  assert.equal(
    recentConversationPresentation(
      feed({
        isPrimed: true,
        loadGate: "exhausted",
        nextCursor: "cursor-next",
      }),
      3,
      "all",
    ).footerKind,
    "hidden",
  );
});
