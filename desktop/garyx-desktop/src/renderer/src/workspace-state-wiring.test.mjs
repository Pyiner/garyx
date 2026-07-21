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

test('thread creation folds the authoritative summary into the remembered main state', () => {
  // Main is the durable cross-process owner of hidden created threads: the
  // create response summary must enter the REMEMBERED state, not just the
  // returned value (renderer-only seeds die on the next full refresh).
  assert.match(
    storeSource,
    /rememberHydratedDesktopState\(stateWithCreatedThread\(state, thread\)\)/,
    'createDesktopThread remembers the folded state in the main process',
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
