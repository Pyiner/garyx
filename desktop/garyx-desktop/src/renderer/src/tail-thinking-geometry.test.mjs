import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (relativePath) =>
  readFileSync(new URL(relativePath, import.meta.url), "utf8");

const threadPage = read("./app-shell/components/ThreadPage.tsx");
const appShell = read("./app-shell/AppShell.tsx");
const conversationCss = read("./styles/conversation.css");
const composerCss = read("./styles/composer.css");
const sideToolsCss = read("./styles/side-tools.css");
const settingsShellCss = read("./styles/settings-shell.css");

test("tail thinking is transient scroller chrome outside scroll content", () => {
  const contentStart = threadPage.indexOf("<MessageScrollerContent");
  const contentEnd = threadPage.indexOf("</MessageScrollerContent>", contentStart);
  const viewportEnd = threadPage.indexOf("</MessageScrollerViewport>", contentEnd);
  const anchor = threadPage.indexOf('className="messages-tail-anchor"', contentStart);
  const chrome = threadPage.indexOf('data-tail-thinking-chrome="true"', viewportEnd);

  assert.ok(contentStart >= 0, "message content must exist");
  assert.ok(anchor > contentStart && anchor < contentEnd, "the zero-geometry anchor stays in content");
  assert.ok(contentEnd < viewportEnd, "content closes before the viewport");
  assert.ok(chrome > viewportEnd, "tail chrome must be a viewport sibling, not scroll content");
  assert.doesNotMatch(
    threadPage.slice(contentStart, contentEnd),
    /showTailThinking/,
    "tail visibility must not add or remove a scroll-content child",
  );
  assert.match(
    appShell,
    /activeRenderState\?\.tailActivity === "thinking" \|\| showPendingAckLoading/,
    "server render_state remains the tail-thinking truth source",
  );
});

test("tail anchor and chrome have zero scroll-content lifecycle delta", () => {
  assert.match(
    settingsShellCss,
    /\.messages-scroller\s*\{[^}]*anchor-scope:\s*--garyx-transcript-tail;/s,
  );
  assert.match(
    settingsShellCss,
    /\.messages-content,\s*\.messages-item\s*\{[^}]*--messages-row-gap:\s*14px;[^}]*gap:\s*var\(--messages-row-gap\);/s,
  );
  assert.match(
    settingsShellCss,
    /\.messages-tail-anchor\s*\{[^}]*anchor-name:\s*--garyx-transcript-tail;[^}]*block-size:\s*0;[^}]*margin-block-start:\s*calc\(-1 \* var\(--messages-row-gap\)\);/s,
    "the permanent anchor must cancel its flex gap and contribute zero extent",
  );
  assert.match(
    settingsShellCss,
    /\.messages-tail-thinking\s*\{[^}]*position:\s*absolute;[^}]*position-anchor:\s*--garyx-transcript-tail;[^}]*position-area:\s*block-end span-inline-start;[^}]*width:\s*anchor-size\(width\);[^}]*pointer-events:\s*none;/s,
    "visible chrome must follow the anchor without entering layout",
  );
});

test("tail thinking keeps a natural-height clearance above every composer", () => {
  assert.match(
    conversationCss,
    /\.thread-main\s*\{[^}]*--composer-bottom-inset:\s*16px;[^}]*--composer-tail-thinking-clearance:\s*16px;/s,
    "the thread geometry must name the composer inset and tail clearance",
  );
  assert.match(
    composerCss,
    /\.composer-shell-wrap\s*\{[^}]*bottom:\s*var\(--composer-bottom-inset\);/s,
    "the composer and tail cap must share one bottom-inset source of truth",
  );
  assert.match(
    sideToolsCss,
    /\.side-tool-chat-thread \.thread-main\s*\{[^}]*--composer-bottom-inset:\s*calc\(30px \+ var\(--padding-panel\)\);/s,
    "side chat must publish its higher composer inset to the shared geometry",
  );
  assert.match(
    settingsShellCss,
    /@position-try --garyx-tail-thinking-clearance\s*\{[^}]*align-self:\s*end;/s,
    "overflowing tail chrome must align its natural-height margin box to the cap",
  );

  const tailRule = settingsShellCss.match(/\.messages-tail-thinking\s*\{([^}]*)\}/s)?.[1] ?? "";
  assert.match(tailRule, /position-area:\s*block-end span-inline-start;/);
  assert.match(
    tailRule,
    /margin-block:\s*8px\s+calc\(\s*var\(--composer-overlay-height\)\s*\+\s*var\(--composer-bottom-inset\)\s*\+\s*var\(--composer-tail-thinking-clearance\)\s*-\s*var\(--composer-scroll-clip-height\)\s*\);/s,
    "the end margin must stop at composer top minus the named clearance",
  );
  assert.match(tailRule, /align-self:\s*start;/);
  assert.match(tailRule, /justify-self:\s*end;/);
  assert.match(
    tailRule,
    /position-try-fallbacks:\s*--garyx-tail-thinking-clearance;/,
  );
  assert.doesNotMatch(
    tailRule,
    /(?:block-size|height):/,
    "the cap must use the rendered chrome height instead of duplicating it",
  );
});
