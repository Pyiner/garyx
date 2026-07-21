import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

// Production-wiring contracts for the workspace state machine. The pure
// helpers have their own behavior tests; these assertions make the PRODUCTION
// call sites load-bearing — reverting a write point to the old behavior
// fails here even though the helper tests stay green.

const storeSource = readFileSync(
  new URL('../../main/store.ts', import.meta.url),
  'utf8',
);
const appShellSource = readFileSync(
  new URL('./app-shell/AppShell.tsx', import.meta.url),
  'utf8',
);

test('hidden-session retention is wired at every sessions write point', () => {
  // Both production writers of `sessions` go through the retaining merge;
  // the old blind mirror (`sessions: threads`) must not come back.
  assert.equal(
    (storeSource.match(/sessions: mergeRetainedHiddenSessions\(/g) || []).length,
    2,
    'withSortedEntities and normalizeState both retain hidden sessions',
  );
  assert.doesNotMatch(
    storeSource,
    /sessions: threads\b/,
    'no sessions write point may blindly mirror threads',
  );
});

test('thread creation folds the authoritative summary into the persisted main state', () => {
  // Hydration starts from the PERSISTED local state, so the durable
  // cross-process owner is the persisted state — the fold must be written
  // through writeState, not merely remembered or returned (renderer-only
  // seeds and remembered-only folds both die on the next full refresh).
  assert.match(
    storeSource,
    /await mutatePersistedState\(\(local\) => \{[\s\S]{0,400}?stateWithCreatedThread\(local, thread\)/,
    'createDesktopThread persists the folded state through the mutation owner',
  );
  assert.match(
    storeSource,
    /rememberHydratedDesktopState\(stateWithCreatedThread\(state, thread\)\)/,
    'the returned snapshot carries the fold for immediate rendering',
  );
});

test('persisted-state mutations are serialized and lifecycle removals clear the owner', () => {
  // All persisted read-modify-writes flow through the single mutation
  // owner (parallel creations must serialize, not race rename()).
  assert.match(
    storeSource,
    /let persistedStateMutationChain: Promise<void> = Promise\.resolve\(\);/,
    'the persisted-state mutation queue exists',
  );
  // The create fold validates the creating gateway scope inside the
  // critical section (a late response from a previous gateway is a no-op).
  assert.match(
    storeSource,
    /if \(localScope !== creatingGatewayScope\) \{\s*return null;/,
    'a stale-gateway create never pollutes the new scope',
  );
  // Successful delete AND archive both drop the thread from the persisted
  // owner, or a retained hidden session would resurrect on refresh.
  assert.equal(
    (storeSource.match(
      /await mutatePersistedState\(\(local\) =>\s*withSortedEntities\(desktopStateWithoutThread\(local, input\.threadId\)\),\s*\);/g,
    ) || []).length,
    2,
    'delete and archive clear the persisted owner',
  );
  // Unique temp names keep concurrent atomic writes from colliding.
  assert.match(
    storeSource,
    /tmp-\$\{process\.pid\}-\$\{Date\.now\(\)\.toString\(36\)\}-\$\{randomUUID\(\)/,
    'atomic writes use collision-free temp paths',
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
