import assert from "node:assert/strict";
import test from "node:test";

import { deriveThreadActivityModel } from "./app-shell/thread-activity.ts";
import { tailThinkingScrollReserve } from "./app-shell/components/transcript-scroll-anchor.ts";
import {
  applyTranscriptScrollTransaction,
  beginTranscriptScrollTransaction,
  decideTranscriptBottomScroll,
  messageTailSignature,
  settleTranscriptScrollTransaction,
} from "./app-shell/components/transcript-scroll-transaction.ts";
import { buildThreadViewRowsWithLocalUsers } from "./render-view-model.ts";

function renderStateFixture() {
  return {
    based_on_seq: 2,
    rows: [
      {
        kind: "user_turn",
        id: "user_turn:seq:1",
        user: { id: "seq:1", seq: 1, role: "user" },
        activity: [
          {
            kind: "assistant_reply",
            message: { id: "seq:2", seq: 2, role: "assistant" },
          },
        ],
        capsule_cards: [],
        started_at: null,
        finished_at: null,
      },
    ],
    tailActivity: "thinking",
    activeToolGroupId: null,
    progress_locus: "tail",
    filtered_placeholders: [],
  };
}

test("composer send force-bottom transaction keeps the optimistic row and thinking tail visible", () => {
  const threadId = "thread::scroll-repro";
  const committedMessages = [
    {
      id: "seq:1",
      seq: 1,
      role: "user",
      text: "Earlier question",
      localState: "remote_final",
    },
    {
      id: "seq:2",
      seq: 2,
      role: "assistant",
      text: "Earlier answer",
      localState: "remote_final",
    },
  ];
  const optimisticUser = {
    id: "origin:intent-repro",
    role: "user",
    text: "Run the requested follow-up.",
    timestamp: "2026-07-23T00:00:00.000Z",
    intentId: "intent-repro",
    localState: "optimistic",
  };
  const runningRenderState = renderStateFixture();
  const activeMessages = [...committedMessages, optimisticUser];
  const rows = buildThreadViewRowsWithLocalUsers(
    runningRenderState,
    new Map(committedMessages.map((message) => [message.seq, message])),
    activeMessages,
  );
  const activity = deriveThreadActivityModel({
    messages: activeMessages,
    runtimeBusy: true,
    pendingAckIntentCount: 0,
    remoteAwaitingAckInputCount: 0,
    pendingHistoryIntent: true,
    renderTailActivity: runningRenderState.tailActivity,
    renderActiveToolGroupId: runningRenderState.activeToolGroupId,
  });

  assert.equal(rows.at(-1)?.key, `user-turn:${optimisticUser.id}`);
  assert.equal(runningRenderState.tailActivity, "thinking");
  assert.equal(activity.showPendingAckLoading, true);

  const clientHeight = 600;
  const beforeScrollTop = 3_000;
  const composerScrollClip = 72;
  const composerMessageClearance = 56;
  const rowGap = 14;
  const thinkingHeight = 24;
  const optimisticBubbleHeight = 80;
  const tailReserve = tailThinkingScrollReserve(
    thinkingHeight,
    rowGap,
    true,
  );
  const runningBottomPadding =
    composerScrollClip + composerMessageClearance - tailReserve;
  const optimisticRowExtent = optimisticBubbleHeight + rowGap;
  const latestScrollTop = beforeScrollTop + optimisticRowExtent;

  const viewport = {
    clientHeight,
    contains: () => true,
    scrollHeight: latestScrollTop + clientHeight,
    scrollTop: beforeScrollTop,
  };
  const anchorDocumentTop = beforeScrollTop + 34;
  const anchorElement = {
    isConnected: true,
    getBoundingClientRect: () => ({
      top: anchorDocumentTop - viewport.scrollTop,
    }),
  };
  const priorTailSignature = messageTailSignature(committedMessages);
  const bottomDecision = decideTranscriptBottomScroll({
    activeMessages,
    currentThreadId: threadId,
    forceBottomSnap: true,
    historyLoading: false,
    pendingThreadBottomSnap: threadId,
    previousTailSignature: priorTailSignature,
    previousThreadId: threadId,
    shouldStickToBottom: true,
  });

  assert.equal(bottomDecision.forceSnap, true);
  assert.equal(bottomDecision.shouldSnapToBottom, true);
  assert.equal(bottomDecision.messageTailChanged, true);

  const transaction = beginTranscriptScrollTransaction({
    active: null,
    anchor: {
      element: anchorElement,
      viewportTop: 34,
    },
    forceBottom:
      bottomDecision.forceSnap && bottomDecision.shouldSnapToBottom,
    preserveTailAnchor: true,
    revision: 1,
    scopeKey: threadId,
  });
  assert.equal(transaction?.mode, "force-bottom");
  assert.ok(transaction);

  let followBottomCalls = 0;
  const followBottom = () => {
    followBottomCalls += 1;
    viewport.scrollTop = viewport.scrollHeight - viewport.clientHeight;
  };
  applyTranscriptScrollTransaction(viewport, transaction, followBottom);
  assert.equal(viewport.scrollTop, latestScrollTop);

  const resizeTransaction = beginTranscriptScrollTransaction({
    active: transaction,
    anchor: {
      element: anchorElement,
      viewportTop: 34,
    },
    forceBottom: false,
    preserveTailAnchor: true,
    revision: 2,
    scopeKey: threadId,
  });
  assert.strictEqual(
    resizeTransaction,
    transaction,
    "a ResizeObserver anchor pass cannot downgrade force-bottom",
  );

  // Model the competing observer correction that used to restore the prior
  // row anchor, then re-apply the coordinator's authoritative transaction.
  viewport.scrollTop = beforeScrollTop;
  applyTranscriptScrollTransaction(
    viewport,
    resizeTransaction,
    followBottom,
  );
  assert.equal(followBottomCalls, 2);
  assert.equal(
    settleTranscriptScrollTransaction(resizeTransaction, resizeTransaction),
    null,
  );

  const bottomDistance =
    viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight;
  const tailBottomClearance = runningBottomPadding - bottomDistance;
  const userBubbleBottomClearance = tailBottomClearance + tailReserve;
  assert.deepEqual(
    {
      scrollTop: viewport.scrollTop,
      bottomDistance,
      tailBottomClearance,
      userBubbleBottomClearance,
      tailVisibleAboveComposer: tailBottomClearance >= composerScrollClip,
      userBubbleClearsComposer:
        userBubbleBottomClearance >= composerScrollClip,
    },
    {
      scrollTop: latestScrollTop,
      bottomDistance: 0,
      tailBottomClearance: runningBottomPadding,
      userBubbleBottomClearance: runningBottomPadding + tailReserve,
      tailVisibleAboveComposer: true,
      userBubbleClearsComposer: true,
    },
  );
});
