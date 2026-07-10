import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const rendererDir = path.dirname(fileURLToPath(import.meta.url));
const read = (relativePath) =>
  readFileSync(path.join(rendererDir, relativePath), "utf8");

test("narrow windows keep the app shell horizontal instead of stacking rails", () => {
  const css = read("styles/browser.css");
  assert.ok(!css.includes("grid-template-columns: 1fr;\n    height: auto;"));
  assert.ok(!css.includes(".left-rail {\n    padding-top: calc(var(--inset-toolbar-sm) + 8px);"));
  assert.ok(!css.includes(".conversation {\n    min-height: 70vh;"));
  assert.ok(!css.includes(".messages {\n    padding-inline: 0;"));
});

test("transcript and composer share Codex's 736px reading edge", () => {
  const conversationCss = read("styles/conversation.css");
  const composerCss = read("styles/composer.css");
  const appShellSource = read("app-shell/AppShell.tsx");

  assert.ok(
    conversationCss.includes(
      ".conversation.thread-view .conversation-body {\n  padding-inline: 0;",
    ),
  );
  assert.equal(
    composerCss.match(/width: min\(var\(--thread-reading-width\)/g)?.length,
    2,
  );
  assert.ok(
    appShellSource.includes(
      'contentView === "thread" ? "thread-view" : null',
    ),
  );
});

test("task tree only reserves a rail when the full reading column fits", () => {
  const css = read("styles/gateway-panels.css");
  const source = read("app-shell/components/ThreadTaskTreePopover.tsx");
  const headerSource = read("ConversationHeaderActions.tsx");

  assert.ok(css.includes("@container thread-task-tree (min-width: 1088px)"));
  assert.ok(css.includes("var(--thread-reading-width)"));
  assert.ok(css.includes(".thread-subtask-pop.is-compact-open"));
  assert.ok(css.includes(".thread-subtask-toggle.has-active::after"));
  assert.ok(source.includes('aria-controls={popoverId}'));
  assert.ok(source.includes('aria-expanded={compactOpen}'));
  assert.ok(source.includes('className={`thread-subtask-toggle'));
  assert.ok(source.includes("createPortal("));
  assert.ok(headerSource.includes("data-thread-task-tree-trigger-host"));
});
