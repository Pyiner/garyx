import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

// Production-wiring contracts for the workspace state machine. The pure
// helpers have their own behavior tests; these assertions make the PRODUCTION
// call sites load-bearing -- reverting a write point to the old behavior
// fails here even though the helper tests stay green.

const storeSource = readFileSync(
  new URL('../../main/store.ts', import.meta.url),
  'utf8',
);
const appShellSource = readFileSync(
  new URL('./app-shell/AppShell.tsx', import.meta.url),
  'utf8',
);
const hiddenStoreSource = readFileSync(
  new URL('../../main/hidden-session-store-core.ts', import.meta.url),
  'utf8',
);
const hiddenStoreBindingSource = readFileSync(
  new URL('../../main/hidden-session-store.ts', import.meta.url),
  'utf8',
);
const sideChatOpsSource = readFileSync(
  new URL('./app-shell/side-chat-ops.ts', import.meta.url),
  'utf8',
);
const sideChatPanelSource = readFileSync(
  new URL('./app-shell/components/SideChatPanel.tsx', import.meta.url),
  'utf8',
);

test('hidden-session retention is wired at every sessions write point', () => {
  // Production writers of `sessions` go through the retaining merge; the
  // old blind mirror (`sessions: threads`) must not come back.
  assert.ok(
    (storeSource.match(/sessions: mergeRetainedHiddenSessions\(/g) || []).length >= 2,
    'sessions write points retain hidden sessions',
  );
  assert.doesNotMatch(
    storeSource,
    /sessions: threads\b/,
    'no sessions write point may blindly mirror threads',
  );
});

test('hidden sessions have one dedicated scoped owner', () => {
  // Its own file + serialized mutations: the main state file's many
  // independent writers can never race this domain.
  assert.match(hiddenStoreBindingSource, /garyx-hidden-sessions\.json/);
  // Single-flight initialization: concurrent loads share one read and the
  // cache installs exactly once (a late duplicate read cannot clobber a
  // cache that mutations have already advanced).
  assert.match(hiddenStoreSource, /cachedPartitions \?\?= partitions;/);
  assert.match(
    hiddenStoreSource,
    /let mutationChain: Promise<void> = Promise\.resolve\(\);/,
  );
  assert.match(
    hiddenStoreSource,
    /const run = mutationChain\.then\(/,
    'mutations chain on the shared owner, not detached promises',
  );
  assert.match(
    hiddenStoreSource,
    /tmp-\$\{process\.pid\}-\$\{Date\.now\(\)\.toString\(36\)\}-\$\{randomUUID\(\)/,
    'atomic writes use collision-free temp paths',
  );

  // Creation folds into the CREATING gateway partition; the returned
  // snapshot carries every retained hidden session for that scope.
  assert.match(
    storeSource,
    /await rememberHiddenSession\(creatingGatewayScope, thread\)/,
    'createDesktopThread persists through the scoped owner',
  );
  assert.match(
    storeSource,
    /listHiddenSessions\(creatingGatewayScope\)/,
    'the returned snapshot merges the whole partition',
  );
  // Hydration reads the same owner.
  assert.match(
    storeSource,
    /listHiddenSessions\(hydrationScope\)/,
    'hydration merges retained hidden sessions from the owner',
  );
  // Lifecycle removals target the OPERATION gateway partition only --
  // equal thread ids on another gateway stay untouched.
  assert.equal(
    (storeSource.match(
      /await forgetHiddenSession\(\s*normalizeGatewayUrl\(current\.settings\.gatewayUrl \|\| ''\),\s*input\.threadId,\s*\);/g,
    ) || []).length,
    2,
    'delete and archive clear their own gateway partition',
  );
});

test('the renderer commits the created state without stripping its envelope', () => {
  assert.match(
    sideChatOpsSource,
    /setDesktopState\(created\.state\);/,
    'the main snapshot is committed as-is',
  );
  assert.doesNotMatch(
    sideChatOpsSource,
    /setDesktopState\(\{\s*\.\.\.created\.state/,
    'no spread re-wrap that would strip the ingress envelope',
  );
});

test('side-chat scope ownership is wired at every production boundary', () => {
  // 1. The connection-scope transition lives in a LAYOUT effect: it runs
  //    only for COMMITTED renders (a functional updater can be replayed or
  //    abandoned without committing), synchronously before paint, and
  //    before every passive effect — so no painted frame and no loader can
  //    observe the previous gateway's universe. The committed state is
  //    adopted as the mirror's new root in the same call.
  const transitionIdx = appShellSource.indexOf(
    'gatewayMirror.beginConnectionScope(committedGatewayKey, {',
  );
  assert.ok(transitionIdx > 0, 'the mirror transition call exists');
  const layoutIdx = appShellSource.lastIndexOf(
    'useLayoutEffect(() => {',
    transitionIdx,
  );
  assert.ok(
    layoutIdx > 0 && transitionIdx - layoutIdx < 600,
    'the transition runs inside a layout effect (real commit boundary)',
  );
  const domainTransitionIdx = appShellSource.indexOf(
    'sideChatSessions.setGatewayScope(committedGatewayKey);',
  );
  assert.ok(
    domainTransitionIdx > transitionIdx,
    'the mirror universe resets before the side-chat domain republishes',
  );
  // The agent catalog has ONE owner: the mirror fetches, fences, and
  // publishes; AppShell's refresh delegates and never fetches directly, and
  // no second write path bypasses the subscription mirror.
  assert.match(
    appShellSource,
    /async function refreshAgentTargets\(\) \{[\s\S]{0,300}?await gatewayMirror\.refreshAgentCatalog\(\);/,
    'the catalog refresh delegates to the single mirror owner',
  );
  assert.doesNotMatch(
    appShellSource,
    /listCustomAgents\(\)\s*\n?\s*\.catch\(\(\) => EMPTY_DESKTOP_AGENT_CATALOG\)[\s\S]{0,400}?setDesktopAgentCatalog/,
    'no direct fetch-and-write catalog path remains in AppShell',
  );
  assert.match(
    appShellSource,
    /gatewayMirror\.refreshAgentCatalog\(\),/,
    'boot hydration requests its catalog through the mirror owner',
  );
  assert.equal(
    (appShellSource.match(/window\.garyxDesktop\.listCustomAgents/g) || [])
      .length,
    1,
    'the ONLY direct catalog fetch is the mirror services injection',
  );
  // The Agents hub is a SUBSCRIBER of the mirror catalog, never a second
  // fetch/owner path.
  const agentsHubSource = readFileSync(
    new URL('./app-shell/components/AgentsHubPanel.tsx', import.meta.url),
    'utf8',
  );
  assert.match(
    agentsHubSource,
    /const catalogSnapshot = useCatalog\(\);/,
    'the agents hub consumes the mirror catalog snapshot',
  );
  assert.doesNotMatch(
    agentsHubSource,
    /window\.garyxDesktop\.listCustomAgents/,
    'the agents hub never fetches the catalog directly',
  );
  // Every automation async handler settles through one operation owner.
  const automationSource = readFileSync(
    new URL('./app-shell/useAutomationController.ts', import.meta.url),
    'utf8',
  );
  assert.ok(
    (automationSource.match(/const operation = openAutomationOperation\(\);/g) || [])
      .length >= 5,
    'select, submit, toggle, delete, and run-now all open an operation owner',
  );
  assert.ok(
    (automationSource.match(/operation\.isCurrent\(\)/g) || []).length >= 12,
    'resolve/catch/finally/timer writes settle through the owner',
  );
  // The module ingress singleton is installed only for the COMMITTED
  // component instance (a useState initializer can run twice and keep the
  // first instance while the singleton points at the discarded second).
  assert.match(
    appShellSource,
    /useLayoutEffect\(\(\) => \{\s*\n\s*installPinnedOrderIngress\(pinnedOrderIngress\);\s*\n\s*\}, \[pinnedOrderIngress\]\);/,
    'the ingress singleton installs from a layout effect, not the initializer',
  );
  assert.doesNotMatch(
    appShellSource,
    /const ingress = new PinnedOrderIngress\(rendererSessionId\);\s*\n\s*installPinnedOrderIngress/,
    'no render-time singleton installation remains',
  );
  // Lifecycle mutations stamp their NESTED authoritative state and commit
  // the applied value as-is (a rebuild would strip delivery identity).
  assert.match(
    appShellSource,
    /setDesktopState\(archivedResult\.value\);/,
    'archive commits the authoritative value as-is',
  );
  // No side effects hide inside the state updater anymore.
  assert.doesNotMatch(
    appShellSource,
    /commitState\(current, action\);[\s\S]{0,400}beginConnectionScope/,
    'the functional updater stays pure',
  );
  // The panel derives its scope from the shell-truth prop, never the
  // (possibly lagging) mirror root.
  assert.match(
    sideChatPanelSource,
    /const scopedView = scopedSideChatView\(\s*sessionsSnapshot,\s*gatewayKey,/,
    'the panel scope key is the shell-truth prop',
  );
  // The persisted binding is re-adopted in the scope-keyed effect AFTER the
  // commit-boundary transition cleared the domain.
  const restoreIdx = appShellSource.lastIndexOf(
    'sideChatSessions.restorePersisted(sideChatSourceThreadId);',
  );
  assert.ok(restoreIdx > 0, 'the post-transition restore exists');
  assert.match(
    appShellSource.slice(restoreIdx, restoreIdx + 300),
    /\}, \[sideChatSessions, workspaceGatewayKey\]\);/,
    'the restore effect is keyed on the gateway scope',
  );

  // 2. EVERY consumer of the sessions snapshot derives through the shared
  //    scope-current projection: a mismatched frame is the EMPTY side-chat
  //    universe for AppShell's shell-owned effects AND the panel's own
  //    subscription alike.
  assert.match(
    appShellSource,
    /const sideChatThreadId = scopedSideChatView\(/,
    'AppShell derives the side thread through the shared scoped view',
  );
  assert.match(
    sideChatPanelSource,
    /const scopedView = scopedSideChatView\(/,
    'the panel derives through the shared scoped view',
  );
  assert.doesNotMatch(
    sideChatPanelSource,
    /sessionsSnapshot\.threadBySource/,
    'the panel never reads raw bindings past the scope identity',
  );

  // 2b. Universe-scoped bookkeeping resets with the scope, and every
  //     same-thread-id consumer re-keys on the gateway universe: the
  //     selected loader carries the gateway key in its deps, and the legacy
  //     history reconcile loop is epoch-owned end to end.
  assert.match(
    appShellSource,
    /deferredQueueDrainByThreadRef\.current = \{\};/,
    'the deferred drain bookkeeping resets on the transition',
  );
  assert.match(
    appShellSource,
    /\}, \[Boolean\(desktopState\), selectedThreadId, desktopState\?\.entitiesGatewayUrl\]\);/,
    'the selected-thread loader re-keys on the gateway universe',
  );
  assert.match(
    appShellSource,
    /const epoch = gatewayMirror\.currentConnectionEpoch;\s*\n\s*const scopeCurrent = \(\) =>\s*gatewayMirror\.isCurrentConnectionEpoch\(epoch\);/,
    'scheduleHistoryRefresh captures its owning epoch at schedule time',
  );

  // 3. The history/stream effect is keyed on the scope generation, so a
  //    same-thread-id gateway switch still tears down and re-keys.
  assert.match(
    appShellSource,
    /\}, \[desktopStateHydrated, sideChatThreadId, sideChatScopeGeneration\]\);/,
    'the history effect identity includes the scope generation',
  );

  // 4. ensureSideChatThread captures its owning generation BEFORE the first
  //    await: the existing-binding adoption path is inside the same fence.
  const generationIdx = sideChatOpsSource.indexOf(
    'const opGeneration = sessions.scopeGeneration;',
  );
  const openableIdx = sideChatOpsSource.indexOf('ensureThreadOpenable');
  assert.ok(
    generationIdx > 0 && openableIdx > generationIdx,
    'the ops generation fence precedes the openability await',
  );

  // 5. The composer upload lock is released only through the
  //    generation-bound release closure (no raw decrement API exists).
  assert.match(
    sideChatPanelSource,
    /const releaseUpload = sessions\.beginAttachmentUpload\(\);/,
    'the panel acquires the upload lock through the release-closure API',
  );
  assert.doesNotMatch(
    sideChatPanelSource,
    /endAttachmentUpload/,
    'no raw upload decrement call remains',
  );
});

test('cold-start and one-shot draft resolution share the failed-catalog gate', () => {
  assert.match(
    appShellSource,
    /resolveStartupDraftSelection\(\s*draftSelectionFromRouteWorkspace\(/,
    'the startup new-thread branch resolves through the shared pure function',
  );
  assert.match(
    appShellSource,
    /workspaceCatalogUnavailable\(desktopState\)/,
    'the one-shot resolution effect uses the shared availability predicate',
  );
  // The old inline empty-catalog check must not return to either branch.
  assert.doesNotMatch(
    appShellSource,
    /workspaces\.length === 0\s*\)\s*\{\s*return;\s*\}\s*const resolved = resolveDefaultDraftWorkspace/,
    'no inline empty-catalog early-return ahead of default resolution',
  );
});
