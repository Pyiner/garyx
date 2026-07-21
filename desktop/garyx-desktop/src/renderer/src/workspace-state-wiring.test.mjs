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
  // 1. The scope transition effect owns the post-transition restore: the
  //    persisted binding is re-adopted AFTER setGatewayScope clears the
  //    domain (cold-start scope landing and gateway switches alike).
  const transitionIdx = appShellSource.indexOf(
    'sideChatSessions.setGatewayScope(workspaceGatewayKey);',
  );
  assert.ok(transitionIdx > 0, 'the scope transition call exists');
  const transitionWindow = appShellSource.slice(
    transitionIdx,
    transitionIdx + 900,
  );
  assert.match(
    transitionWindow,
    /restorePersisted\(sideChatSourceThreadId\)/,
    'the transition effect restores the persisted binding after clearing',
  );

  // 2. Render derivation requires the snapshot's owning scope to match the
  //    current gateway key: a mismatched frame derives an EMPTY side-chat
  //    universe instead of the previous gateway's binding.
  assert.match(
    appShellSource,
    /sideChatSessionsSnapshot\.gatewayScope === workspaceGatewayKey/,
    'the derivation gates on snapshot scope identity',
  );
  assert.match(
    appShellSource,
    /sideChatSourceThreadId && sideChatScopeCurrent\s*\?/,
    'sideChatThreadId derives only from a scope-current snapshot',
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
