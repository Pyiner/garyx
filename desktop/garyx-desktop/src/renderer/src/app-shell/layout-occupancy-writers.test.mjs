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
const liveNativeEffectSources = rendererSourceUrls
  .filter(
    (url) =>
      !url.pathname.endsWith("/responsive-layout-model.ts") &&
      !url.pathname.endsWith("/window-layout-protocol.ts"),
  )
  .map((url) => readFileSync(url, "utf8"));

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

test("desired logging may lead legacy commits without changing their UI sequence", () => {
  assert.match(
    appShell,
    /handleWorkspacePreviewRequested[\s\S]*?commitLegacyLayoutIntent\(\s*"user-route",[\s\S]*?inspectorOpen: true,[\s\S]*?threadLogsOpen: false,[\s\S]*?\(current\) => \(\{ \.\.\.current, inspectorOpen: true \}\)/,
    "workspace preview logs its final replace vector but preserves the first legacy write",
  );
  assert.match(
    appShell,
    /normalizeNewThreadIntent[\s\S]*?openCapsuleTabs: \[\],[\s\S]*?openCapsuleTabs: current\.openCapsuleTabs/,
    "new-thread intent includes capsule cleanup while the old effect still applies it",
  );
  assert.match(
    appShell,
    /layoutOccupancyEventLogRef\.current = appendResult\.log/,
  );
});

test("Phase 2 consumes legacy frames while native window effects remain disconnected", () => {
  assert.match(frameStore, /\breduceHorizontalLayout\b/);
  assert.match(frameStore, /\bprojectHorizontalLayout\b/);
  assert.match(resizeController, /createLegacyHorizontalLayoutFrameStore/);
  assert.match(resizeController, /useSyncExternalStore/);
  assert.match(appShell, /dispatchLayoutOccupancyEvent\(appendResult\.event\)/);
  for (const source of liveNativeEffectSources) {
    assert.doesNotMatch(source, /\bAPPLY_WINDOW_BOUNDS\b/);
    assert.doesNotMatch(source, /\bCLAIM_INITIAL_LAYOUT\b/);
  }
  assert.doesNotMatch(resizeController, /adjustWindow|setBounds/);
  assert.match(mainWindow, /minWidth: 1180/);
  assert.doesNotMatch(mainWindow, /minWidth: 480/);
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
