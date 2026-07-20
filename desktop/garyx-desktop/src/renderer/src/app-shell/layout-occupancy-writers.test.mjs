import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import test from "node:test";

const appShell = readFileSync(new URL("./AppShell.tsx", import.meta.url), "utf8");
const leftRail = readFileSync(
  new URL("./components/AppLeftRail.tsx", import.meta.url),
  "utf8",
);
const sideToolsPanel = readFileSync(
  new URL("./components/SideToolsPanel.tsx", import.meta.url),
  "utf8",
);
const conversationHeaderActions = readFileSync(
  new URL("../ConversationHeaderActions.tsx", import.meta.url),
  "utf8",
);
const resizeController = readFileSync(
  new URL("./useLayoutResizeController.ts", import.meta.url),
  "utf8",
);
const frameStore = readFileSync(
  new URL("./horizontal-layout-frame-store.ts", import.meta.url),
  "utf8",
);
const mainWindow = readFileSync(
  new URL("../../../main/index.ts", import.meta.url),
  "utf8",
);
const mainExecutor = readFileSync(
  new URL("../../../main/window-layout-executor.ts", import.meta.url),
  "utf8",
);
const effectRunner = readFileSync(
  new URL("./horizontal-layout-effect-runner.ts", import.meta.url),
  "utf8",
);

function rendererSourceFiles(directory) {
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const url = new URL(
      `${entry.name}${entry.isDirectory() ? "/" : ""}`,
      directory,
    );
    if (entry.isDirectory()) {
      files.push(...rendererSourceFiles(url));
    } else if (/\.tsx?$/.test(entry.name)) {
      files.push(url);
    }
  }
  return files;
}

const rendererSourceUrls = rendererSourceFiles(
  new URL("../", import.meta.url),
);
const rendererSources = rendererSourceUrls.map((url) =>
  readFileSync(url, "utf8"),
);
const legacySetterNames = [
  "setOpenCapsuleTabsLegacy",
  "setInspectorOpenLegacy",
  "setRecentThreadsRailOpenLegacy",
  "setBotConversationGroupIdLegacy",
];

test("all legacy occupancy setters have exactly one centralized call site", () => {
  for (const setterName of legacySetterNames) {
    const references = rendererSources.flatMap((source) =>
      source.match(new RegExp(`\\b${setterName}\\b`, "g")) || [],
    );
    const calls = rendererSources.flatMap((source) =>
      source.match(new RegExp(`\\b${setterName}\\(`, "g")) || [],
    );
    assert.equal(
      references.length,
      2,
      `${setterName}: declaration plus centralized writer only`,
    );
    assert.equal(calls.length, 1, `${setterName}: one centralized call`);
  }

  for (const retiredSetterName of [
    "setOpenCapsuleTabs",
    "setInspectorOpen",
    "setRecentThreadsRailOpen",
    "setBotConversationGroupId",
    // The workspace conversation rail itself is retired: the whole setter
    // family must never come back (inline thread subtree replaced it).
    "setWorkspaceConversationPath",
    "setWorkspaceConversationPathLegacy",
  ]) {
    assert.doesNotMatch(
      appShell,
      new RegExp(`\\b${retiredSetterName}\\b`),
      `${retiredSetterName} must not bypass the occupancy bridge`,
    );
  }
});

test("compact sidebar expansion enters the layout machine instead of opening an overlay", () => {
  assert.match(
    resizeController,
    /compactSidebarViewport: frame\.presentation\.compactViewport/,
  );
  assert.match(
    resizeController,
    /sidebarDesiredOpen: store\.getState\(\)\.desiredOccupancy\.globalSidebar/,
  );
  assert.equal(
    appShell.match(/\btoggleSidebarCollapsedLegacy\b/g)?.length,
    3,
    "controller alias, normalized wrapper call, and callback dependency",
  );
  assert.equal(
    appShell.match(/\btoggleSidebarCollapsedLegacy\(/g)?.length,
    1,
  );
  assert.match(
    appShell,
    /compactSidebarViewport &&\s*\(window\.garyxDesktop\.horizontalLayoutPolicy === "legacy" \|\|\s*\(sidebarCollapsed && sidebarDesiredOpen\)\)/,
  );
  assert.match(
    appShell,
    /commitLegacyLayoutIntent\("user-panel"/,
    "a user opening a compact sidebar must still own a layout transaction",
  );
  assert.doesNotMatch(resizeController, /"compact-overlay"/);
});

test("feature-off compact toggles retain the legacy temporary overlay path", () => {
  assert.match(
    appShell,
    /compactSidebarViewport &&\s*\(window\.garyxDesktop\.horizontalLayoutPolicy === "legacy" \|\|/,
    "legacy compact toggles must bypass the desired-state/persistence path in both overlay states",
  );
  assert.match(
    appShell,
    /toggleSidebarCollapsedLegacy\(\);\s*return;/,
  );
});

test("route, capsule, and cleanup writers enter the same bridge", () => {
  assert.match(
    appShell,
    /onOpenRecent=\{\(\) => \{[\s\S]*?commitLegacyLayoutIntent\("user-route"/,
  );
  assert.match(
    appShell,
    /onToggleBotConversationGroup=\{[\s\S]*?commitLegacyLayoutIntent\("user-route"/,
  );
  assert.match(
    appShell,
    /onCloseCapsuleTab=\{[\s\S]*?commitLegacyLayoutIntent\("user-route"/,
  );
  assert.match(
    appShell,
    /onOpenCapsule=\{[\s\S]*?commitLegacyLayoutIntent\("user-route"/,
  );
  assert.equal(
    (appShell.match(/commitLegacyLayoutIntent\("system-cleanup"/g) || [])
      .length,
    5,
    // Was 6 before the workspace conversation rail retired; its stale-path
    // cleanup writer went with it.
    "route, data, Escape, and capsule cleanup paths are normalized",
  );
});

test("thread logs replace Tasks inside side tools without entering layout occupancy", () => {
  assert.match(
    sideToolsPanel,
    /\{ id: "logs", label: t\("Logs"\)/,
    "Logs occupies the former Tasks slot in the side-tools catalog",
  );
  assert.match(
    sideToolsPanel,
    /activeTool\?\.id === "logs"[\s\S]*?<ThreadLogsTool/,
    "the Logs tab owns the polling subtree",
  );
  assert.match(sideToolsPanel, /availableThreadSideToolIds\(hasWorkspace\)/);
  assert.doesNotMatch(conversationHeaderActions, /hasWorkspace|disabled=/);
  assert.doesNotMatch(
    appShell,
    /if \(!activeWorkspacePath\) \{\s*commitLegacyLayoutIntent\("system-cleanup"/,
    "a no-workspace thread must keep side tools open for Logs",
  );
  assert.doesNotMatch(
    sideToolsPanel,
    /id: "tasks"|SideThreadTasksTool|listTasks|onOpenTaskThread/,
    "the side-tools Tasks implementation is removed",
  );
  assert.match(
    leftRail,
    /onOpenTasks|TasksIcon/,
    "the global Tasks entry remains untouched",
  );
  assert.match(appShell, /<TasksPanel/);
  assert.doesNotMatch(appShell, /threadLogs|ThreadLog/);
});

test("desired cleanup may lead legacy commits without changing their UI sequence", () => {
  assert.match(
    appShell,
    /normalizeNewThreadIntent[\s\S]*?openCapsuleTabs: \[\],[\s\S]*?openCapsuleTabs: current\.openCapsuleTabs/,
    "new-thread intent includes capsule cleanup while the old effect still applies it",
  );
  assert.match(
    appShell,
    /layoutOccupancyEventLogRef\.current = appendResult\.log/,
  );
  assert.match(
    appShell,
    /conversationRail: secondaryConversationRailRequested[\s\S]*?: \{ kind: "closed" \}/,
    "the event log and live store share the same route-gated initial rail seed",
  );
});

test("Phase 4 switches the whole stack while the feature-off branch stays legacy", () => {
  assert.match(frameStore, /\breduceHorizontalLayout\b/);
  assert.match(frameStore, /\bprojectHorizontalLayout\b/);
  assert.match(resizeController, /createLegacyHorizontalLayoutFrameStore/);
  assert.match(resizeController, /createHorizontalLayoutEffectRunner/);
  assert.match(
    resizeController,
    /if \(layoutPolicy === "expand-v1"\) \{\s*pendingEffectsRef\.current\.push\(\.\.\.effects\);/,
    "legacy mode discards effects already settled by its local checkpoint",
  );
  assert.match(resizeController, /useSyncExternalStore/);
  assert.match(appShell, /dispatchLayoutOccupancyEvent\(appendResult\.event\)/);
  assert.doesNotMatch(resizeController, /adjustWindow|setBounds/);
  assert.match(
    mainWindow,
    /minWidth: horizontalLayoutPolicy === "expand-v1" \? 480 : 1180/,
  );
  assert.match(mainWindow, /GARYX_DESKTOP_EXPAND_V1/);
  assert.match(
    resizeController,
    /if \(layoutPolicy === "legacy"\)[\s\S]*createLegacyHorizontalLayoutFrameStore/,
  );
  assert.match(effectRunner, /executeWindowLayoutCommand/);
  assert.match(mainExecutor, /#host\.setBounds\(command\.targetBounds\)/);
  assert.match(mainExecutor, /#host\.readEnvironment\(\)/);
  assert.doesNotMatch(
    appShell,
    /--spacing-token-sidebar|--side-tools-panel-width|--thread-log-panel-width/,
  );
  assert.match(frameStore, /function applyFrame/);
  assert.match(frameStore, /data-layout-revision/);
  assert.match(resizeController, /window\.requestAnimationFrame\(flush\)/);
  assert.ok(
    (resizeController.match(/"pointercancel"/g) || []).length >= 6,
    "all three resize paths keep pointercancel-as-commit listeners and cleanup",
  );
});
