import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import test from "node:test";

const appShell = readFileSync(new URL("./AppShell.tsx", import.meta.url), "utf8");
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
  "setThreadLogsOpenLegacy",
  "setRecentThreadsRailOpenLegacy",
  "setBotConversationGroupIdLegacy",
  "setWorkspaceConversationPathLegacy",
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
    "setThreadLogsOpen",
    "setRecentThreadsRailOpen",
    "setBotConversationGroupId",
    "setWorkspaceConversationPath",
  ]) {
    assert.doesNotMatch(
      appShell,
      new RegExp(`\\b${retiredSetterName}\\b`),
      `${retiredSetterName} must not bypass the occupancy bridge`,
    );
  }
});

test("sidebar normal intent is logged while compact presentation stays legacy-only", () => {
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
    /if \(!compactSidebarViewport\) \{\s*commitLegacyLayoutIntent\("user-panel"/,
  );
  assert.match(
    appShell,
    /\}\s*toggleSidebarCollapsedLegacy\(\);/,
    "compact toggles still call only the old in-window controller",
  );
});

test("route, capsule, logs replace, and cleanup writers enter the same bridge", () => {
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
    /onToggleWorkspaceThreadGroup=\{[\s\S]*?commitLegacyLayoutIntent\("user-route"/,
  );
  assert.match(
    appShell,
    /onCloseCapsuleTab=\{[\s\S]*?commitLegacyLayoutIntent\("user-route"/,
  );
  assert.match(
    appShell,
    /onOpenCapsule=\{[\s\S]*?commitLegacyLayoutIntent\("user-route"/,
  );
  assert.match(
    appShell,
    /onToggleThreadLogs=\{[\s\S]*?inspectorOpen: false,[\s\S]*?openCapsuleTabs: \[\],[\s\S]*?threadLogsOpen: nextThreadLogsOpen/,
    "logs replacement supplies one complete target vector",
  );
  assert.equal(
    (appShell.match(/commitLegacyLayoutIntent\("system-cleanup"/g) || [])
      .length,
    9,
    "route, data, Escape, no-thread, and capsule cleanup paths are normalized",
  );
});

test("every side-tools opening writer replaces thread logs in one vector", () => {
  assert.match(
    appShell,
    /function replaceThreadLogsWithSideTools[\s\S]*?\.\.\.patch,[\s\S]*?threadLogsOpen: false/,
    "the shared opening constructor makes right-panel exclusion structural",
  );
  assert.equal(
    (appShell.match(/\breplaceThreadLogsWithSideTools\(/g) || []).length,
    5,
    "one constructor definition plus all four side-tools opening writers",
  );
  assert.match(
    appShell,
    /handleWorkspacePreviewRequested[\s\S]*?replaceThreadLogsWithSideTools\(current, \{ inspectorOpen: true \}\)/,
    "workspace preview applies one mutually-exclusive vector",
  );
  assert.match(
    appShell,
    /workspacePreviewModalOpen[\s\S]*?replaceThreadLogsWithSideTools\(current, \{ inspectorOpen: true \}\)/,
    "workspace preview effect uses the same constructor",
  );
  assert.match(
    appShell,
    /onOpenCapsule=\{[\s\S]*?replaceThreadLogsWithSideTools\(current, \{[\s\S]*?openCapsuleTabs:/,
    "transcript capsule open replaces logs before opening the side-tools union",
  );
  assert.match(
    appShell,
    /onToggleInspector=\{[\s\S]*?replaceThreadLogsWithSideTools\(current, \{[\s\S]*?inspectorOpen: nextInspectorOpen/,
    "inspector toggle keeps the right panels mutually exclusive",
  );
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
    (resizeController.match(/"pointercancel"/g) || []).length >= 8,
    "all four resize paths keep pointercancel-as-commit listeners and cleanup",
  );
});
