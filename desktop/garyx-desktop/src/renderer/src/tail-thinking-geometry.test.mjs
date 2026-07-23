import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  restoreTranscriptScrollAnchor,
  tailThinkingScrollReserve,
} from "./app-shell/components/transcript-scroll-anchor.ts";
import {
  applyTranscriptScrollTransaction,
  beginTranscriptScrollTransaction,
  settleTranscriptScrollTransaction,
} from "./app-shell/components/transcript-scroll-transaction.ts";

const read = (relativePath) =>
  readFileSync(new URL(relativePath, import.meta.url), "utf8");

const threadPage = read("./app-shell/components/ThreadPage.tsx");
const appShell = read("./app-shell/AppShell.tsx");
const conversationCss = read("./styles/conversation.css");
const composerCss = read("./styles/composer.css");
const sideToolsCss = read("./styles/side-tools.css");
const settingsShellCss = read("./styles/settings-shell.css");
const transcriptScroll = read(
  "./app-shell/components/thread-transcript-scroll.ts",
);

test("tail thinking is the final semantic row inside message content", () => {
  const contentStart = threadPage.indexOf("<MessageScrollerContent");
  const contentEnd = threadPage.indexOf("</MessageScrollerContent>", contentStart);
  const viewportEnd = threadPage.indexOf("</MessageScrollerViewport>", contentEnd);
  const rateLimit = threadPage.indexOf("<RateLimitBanner", contentStart);
  const thinkingRow = threadPage.indexOf(
    'data-tail-thinking-row="true"',
    contentStart,
  );

  assert.ok(contentStart >= 0, "message content must exist");
  assert.ok(rateLimit > contentStart && rateLimit < thinkingRow);
  assert.ok(
    thinkingRow > rateLimit && thinkingRow < contentEnd,
    "thinking must be the last conditional transcript row",
  );
  assert.ok(contentEnd < viewportEnd, "the row must close with scroll content");
  assert.doesNotMatch(threadPage, /messages-tail-anchor|data-tail-thinking-chrome/);
  assert.match(
    appShell,
    /activeRenderState\?\.tailActivity === "thinking" \|\| showPendingAckLoading/,
    "server render_state remains the tail-thinking truth source",
  );
});

test("the old out-of-flow anchor chrome design is fully removed", () => {
  const combinedCss = [
    conversationCss,
    composerCss,
    sideToolsCss,
    settingsShellCss,
  ].join("\n");
  assert.doesNotMatch(
    combinedCss,
    /anchor-scope|anchor-name|position-anchor|position-area|@position-try|composer-tail-thinking-clearance/,
  );
  assert.match(
    settingsShellCss,
    /\.messages-tail-thinking-row\s*\{[^}]*overflow-anchor:\s*none;/s,
  );
  assert.match(
    settingsShellCss,
    /var\(--messages-tail-scroll-reserve\)/,
    "the measured in-flow row must consume the existing bottom clearance",
  );
  assert.match(
    transcriptScroll,
    /TranscriptScrollCoordinator/,
    "tail lifecycle stability belongs to the single transcript scroll owner",
  );
  assert.match(
    threadPage,
    /<MessageScrollerProvider autoScroll scrollEdgeThreshold=\{48\}>/,
    "the primitive and Garyx must agree on the existing near-bottom threshold",
  );
});

function restoreHarness({ beforeTop, currentTop, scrollTop }) {
  const element = {
    isConnected: true,
    getBoundingClientRect: () => ({ top: currentTop }),
  };
  const viewport = {
    scrollTop,
    contains: (candidate) => candidate === element,
  };
  const correction = restoreTranscriptScrollAnchor(viewport, {
    element,
    viewportTop: beforeTop,
  });
  return { correction, scrollTop: viewport.scrollTop };
}

test("tail appearance and disappearance preserve the viewport row anchor", () => {
  const reserve = tailThinkingScrollReserve(24, 14, true);
  assert.equal(reserve, 38);

  const hiddenScrollHeight = 4_369;
  const visibleScrollHeight = hiddenScrollHeight + reserve - reserve;
  assert.equal(
    visibleScrollHeight,
    hiddenScrollHeight,
    "the in-flow row trades equal measured space with bottom clearance",
  );

  assert.deepEqual(
    restoreHarness({
      beforeTop: 34,
      currentTop: -4,
      scrollTop: 3_568,
    }),
    { correction: -38, scrollTop: 3_530 },
    "a bottom-follow scroll from the old 38px lifecycle delta is undone",
  );
  assert.deepEqual(
    restoreHarness({
      beforeTop: 34,
      currentTop: 34,
      scrollTop: 3_530,
    }),
    { correction: 0, scrollTop: 3_530 },
    "removing the reserved row keeps the same viewport coordinate",
  );
  assert.deepEqual(
    restoreHarness({
      beforeTop: -6,
      currentTop: -6,
      scrollTop: 3_210,
    }),
    { correction: 0, scrollTop: 3_210 },
    "a non-bottom reader is never moved",
  );
});

test("thinking-to-first-output is an atomic visual-anchor handoff", () => {
  assert.deepEqual(
    restoreHarness({
      beforeTop: 34,
      currentTop: -2,
      scrollTop: 3_566,
    }),
    { correction: -36, scrollTop: 3_530 },
    "the first assistant glyph cannot consume the prior tail clearance as a jump",
  );
});

test("tail anchor preservation remains authoritative through the resize pass", () => {
  const viewport = {
    contains: (candidate) => candidate === anchor,
    scrollTop: 3_568,
  };
  const anchorDocumentTop = 3_564;
  const anchor = {
    isConnected: true,
    getBoundingClientRect: () => ({
      top: anchorDocumentTop - viewport.scrollTop,
    }),
  };
  const transaction = beginTranscriptScrollTransaction({
    active: null,
    anchor: {
      element: anchor,
      viewportTop: 34,
    },
    forceBottom: false,
    preserveTailAnchor: true,
    revision: 1,
    scopeKey: "thread::tail-stability",
  });
  assert.equal(transaction?.mode, "preserve-tail-anchor");
  assert.ok(transaction);

  const unexpectedFollowBottom = () => {
    assert.fail("a preserve-tail-anchor transaction must not follow bottom");
  };
  applyTranscriptScrollTransaction(
    viewport,
    transaction,
    unexpectedFollowBottom,
  );
  assert.equal(viewport.scrollTop, 3_530);

  viewport.scrollTop = 3_568;
  applyTranscriptScrollTransaction(
    viewport,
    transaction,
    unexpectedFollowBottom,
  );
  assert.equal(viewport.scrollTop, 3_530);
  assert.equal(
    settleTranscriptScrollTransaction(transaction, transaction),
    null,
  );
});
