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

test("live resize leaves mechanical remainder tracks to CSS instead of rewriting viewport pixels", () => {
  const ownerCss = read("styles/app-shell.css");
  const frameStore = read("app-shell/horizontal-layout-frame-store.ts");

  assert.ok(ownerCss.includes("minmax(0, 1fr)"));
  assert.ok(!ownerCss.includes("var(--gx-shell-main-width)"));
  assert.ok(!ownerCss.includes("var(--gx-conversation-width)"));
  assert.ok(!ownerCss.includes("var(--gx-thread-main-width)"));
  assert.ok(frameStore.includes("HORIZONTAL_LAYOUT_PAINT_VARIABLES"));
  assert.ok(frameStore.includes("getRenderRevision"));
});

test("transcript and composer share Codex's 736px reading edge", () => {
  const conversationCss = read("styles/conversation.css");
  const composerCss = read("styles/composer.css");
  const appShellSource = read("app-shell/AppShell.tsx");
  const threadPageSource = read("app-shell/components/ThreadPage.tsx");

  assert.ok(
    conversationCss.includes(
      ".conversation.thread-view .conversation-body {\n  grid-template-rows: minmax(0, 1fr);\n  gap: 0;\n  padding: 0;",
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
  assert.ok(
    composerCss.includes(".composer-shell-wrap {\n  position: absolute;\n  right: 0;\n  bottom: 16px;"),
  );
  assert.ok(threadPageSource.includes("composerContext={composerContext}"));
  assert.ok(!threadPageSource.includes("<div\n                aria-label={t(\"Workspace mode\")}\n                className=\"thread-composer-status\""));
});

test("task tree only reserves a rail when the full reading column fits", () => {
  const css = read("styles/gateway-panels.css");
  const ownerCss = read("styles/app-shell.css");
  const source = read("app-shell/components/ThreadTaskTreePopover.tsx");
  const controller = read("app-shell/useLayoutResizeController.ts");
  const headerSource = read("ConversationHeaderActions.tsx");

  assert.ok(ownerCss.includes('[data-task-tree-presentation="docked"]'));
  assert.ok(!css.includes("@container thread-task-tree"));
  assert.ok(ownerCss.includes("var(--thread-reading-width)"));
  assert.ok(css.includes(".thread-subtask-pop.is-compact-open"));
  assert.ok(css.includes(".thread-subtask-toggle.has-active::after"));
  assert.ok(source.includes('aria-controls={popoverId}'));
  assert.ok(source.includes('aria-expanded={compactOpen}'));
  assert.ok(source.includes('className={`thread-subtask-toggle'));
  assert.ok(source.includes("createPortal("));
  assert.ok(source.includes("triggerHost && !taskTreeDocked"));
  assert.ok(!source.includes("ResizeObserver"));
  assert.ok(!source.includes("getBoundingClientRect"));
  assert.ok(controller.includes("frame.presentation.taskTreeDocked"));
  assert.ok(headerSource.includes("data-thread-task-tree-trigger-host"));
  assert.ok(css.includes("max-height: calc(100% - 24px)"));
});

test("side tools use one right-docked presentation at every width", () => {
  const appShellSource = read("app-shell/AppShell.tsx");
  const controllerSource = read("app-shell/useLayoutResizeController.ts");
  const ownerCss = read("styles/app-shell.css");
  const browserCss = read("styles/browser.css");

  assert.ok(!appShellSource.includes("sideToolsDocked"));
  assert.ok(!controllerSource.includes("sideToolsDocked"));
  assert.ok(ownerCss.includes(".conversation.with-side-tools {"));
  assert.ok(ownerCss.includes("var(--gx-right-panel-width)"));
  assert.ok(!ownerCss.includes("side-tools-docked"));
  assert.ok(!ownerCss.includes("side-tools-overlay"));
  assert.ok(!browserCss.includes(".conversation.with-side-tools {"));
  assert.ok(!browserCss.includes(".thread-side-tools-panel {"));
});

test("thread logs are side-tools content and have no independent placement policy", () => {
  const appShellSource = read("app-shell/AppShell.tsx");
  const leftRailSource = read("app-shell/components/AppLeftRail.tsx");
  const sideToolsSource = read("app-shell/components/SideToolsPanel.tsx");
  const sideToolsModel = read("app-shell/components/side-tools-panel-model.ts");
  const threadPageSource = read("app-shell/components/ThreadPage.tsx");
  const controllerSource = read("app-shell/useLayoutResizeController.ts");
  const frameStoreSource = read("app-shell/horizontal-layout-frame-store.ts");
  const ownerCss = read("styles/app-shell.css");
  const headerSource = read("ConversationHeaderActions.tsx");

  assert.ok(sideToolsSource.includes('{ id: "logs"'));
  assert.ok(sideToolsSource.includes('<ThreadLogsTool'));
  assert.ok(sideToolsSource.includes('activeTool?.id === "logs"'));
  assert.ok(sideToolsSource.includes("availableThreadSideToolIds(hasWorkspace)"));
  assert.ok(!sideToolsSource.includes('{ id: "tasks"'));
  assert.ok(!sideToolsSource.includes('SideThreadTasksTool'));
  assert.ok(sideToolsModel.includes('"files" | "logs" | "chat"'));
  assert.ok(!sideToolsModel.includes('"tasks"'));
  assert.ok(leftRailSource.includes("onOpenTasks"));
  assert.ok(leftRailSource.includes("TasksIcon"));
  assert.ok(appShellSource.includes("<TasksPanel"));
  assert.ok(!threadPageSource.includes("ThreadLog"));
  assert.ok(!controllerSource.includes("threadLogs"));
  assert.ok(!headerSource.includes("threadLogs"));
  assert.ok(frameStoreSource.includes("projectHorizontalLayout"));
  assert.ok(!frameStoreSource.includes("threadLogs"));
  assert.ok(!ownerCss.includes("thread-log-resizer"));
  assert.ok(!ownerCss.includes("data-thread-logs-presentation"));
});
