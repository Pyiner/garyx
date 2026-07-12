import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const appShell = readFileSync(new URL("./AppShell.tsx", import.meta.url), "utf8");
const resizeController = readFileSync(
  new URL("./useLayoutResizeController.ts", import.meta.url),
  "utf8",
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
    const references =
      appShell.match(new RegExp(`\\b${setterName}\\b`, "g")) || [];
    const calls = appShell.match(new RegExp(`\\b${setterName}\\(`, "g")) || [];
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
  assert.match(resizeController, /compactSidebarViewport,/);
  assert.match(resizeController, /sidebarDesiredOpen: !sidebarCollapsedByUser/);
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
  assert.ok(
    (appShell.match(/commitLegacyLayoutIntent\("system-cleanup"/g) || [])
      .length >= 8,
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
