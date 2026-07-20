import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (relativePath) =>
  readFileSync(new URL(relativePath, import.meta.url), "utf8");

const threadPage = read("./app-shell/components/ThreadPage.tsx");
const appShell = read("./app-shell/AppShell.tsx");
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
    /\.messages-tail-thinking\s*\{[^}]*position:\s*absolute;[^}]*position-anchor:\s*--garyx-transcript-tail;[^}]*top:\s*calc\(anchor\(top\) \+ 8px\);[^}]*left:\s*anchor\(left\);[^}]*pointer-events:\s*none;/s,
    "visible chrome must follow the anchor without entering layout",
  );
});
