import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const recentSidebar = readFileSync(
  new URL("./RecentConversationSidebar.tsx", import.meta.url),
  "utf8",
);
const appShell = readFileSync(
  new URL("./app-shell/AppShell.tsx", import.meta.url),
  "utf8",
);
const hook = readFileSync(
  new URL("./app-shell/useRecentThreadFeeds.ts", import.meta.url),
  "utf8",
);
const preload = readFileSync(
  new URL("../../preload/index.ts", import.meta.url),
  "utf8",
);
const main = readFileSync(
  new URL("../../main/index.ts", import.meta.url),
  "utf8",
);

test("Recent tabs expose the required accessible segmented semantics", () => {
  assert.match(recentSidebar, /aria-label=\{t\("Recent filter"\)\}/);
  assert.match(recentSidebar, /role="tablist"/);
  assert.match(recentSidebar, /role="tab"/);
  assert.match(recentSidebar, /aria-selected=\{selected\}/);
  assert.match(recentSidebar, /event\.key !== "ArrowLeft"/);
  assert.match(recentSidebar, /event\.key !== "ArrowRight"/);
});

test("AppShell owns the feed hook outside the conditional rail", () => {
  const hookOwner = appShell.indexOf("const recentThreadFeeds = useRecentThreadFeeds");
  const recentRowsStart = appShell.indexOf("const recentThreadRows = useMemo");
  const pinnedRowsStart = appShell.indexOf("const pinnedThreadRows = useMemo");
  const conditionalRail = appShell.indexOf("<RecentConversationSidebar");
  assert.ok(hookOwner >= 0);
  assert.ok(recentRowsStart > hookOwner);
  assert.ok(pinnedRowsStart > recentRowsStart);
  assert.ok(conditionalRail > hookOwner);
  const recentRowsOwner = appShell.slice(recentRowsStart, pinnedRowsStart);
  assert.match(
    recentRowsOwner,
    /recentThreadFeeds\.selectedThreads\.map\(\(thread\) => \(\{/,
  );
  assert.doesNotMatch(recentRowsOwner, /desktopState\?\.threads/);
  assert.match(hook, /resetRecentThreadFeedsScope/);
  assert.match(appShell, /gatewayScope: desktopState\?\.entitiesGatewayUrl \|\| ""/);
  assert.doesNotMatch(
    appShell,
    /gatewayScope:[\s\S]{0,160}desktopState\?\.settings\.gatewayUrl/,
  );
  assert.match(
    appShell,
    /onTaskCreated=\{\(\) => \{\s*recentThreadFeeds\.noteAllLocalMutation\(\);\s*recentThreadFeeds\.refreshAll\(\);/,
  );
  assert.match(hook, /queuedRefreshesRef\.current\.add\("all"\)/);
});

test("closing Recent retains its content until the layout frame releases the rail", () => {
  assert.match(appShell, /deferConversationRailUnmount/);
  assert.match(appShell, /settleDeferredConversationRailUnmount/);
  assert.doesNotMatch(
    appShell,
    /<div aria-hidden="true" className="bot-conversation-rail" \/>/,
  );
});

test("preload forwards a narrow input while only main owns the Recent URL", () => {
  assert.match(
    preload,
    /ipcRenderer\.invoke\("garyx:list-recent-threads", input\)/,
  );
  assert.doesNotMatch(preload, /api\/recent-threads/);
  assert.match(main, /validateListRecentThreadsInput\(rawInput\)/);
  assert.match(main, /assertRecentThreadGatewayScope\(settings, input\.gatewayScope\)/);
  assert.doesNotMatch(appShell, /api\/recent-threads/);
  assert.doesNotMatch(hook, /api\/recent-threads/);
});
