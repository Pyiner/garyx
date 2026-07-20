import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  draftSelectionFromRouteWorkspace,
  routeWorkspaceFromDraftSelection,
} from './app-shell/desktop-route.ts';
import { resolveDefaultDraftWorkspace } from './thread-model.ts';

function workspace(overrides = {}) {
  return {
    name: 'garyx',
    path: '/Users/test/repos/garyx',
    kind: 'local',
    createdAt: '2026-01-01T00:00:00.000Z',
    updatedAt: '2026-01-01T00:00:00.000Z',
    available: true,
    pinned: false,
    threadCount: 0,
    lastActivityAt: null,
    gitRepo: false,
    ...overrides,
  };
}

test('route round-trip preserves the draft tri-state', () => {
  const pathSelection = { kind: 'path', path: '/Users/test/repos/garyx' };
  assert.deepEqual(
    draftSelectionFromRouteWorkspace(
      routeWorkspaceFromDraftSelection(pathSelection),
    ),
    pathSelection,
  );
  const noneSelection = { kind: 'none' };
  assert.deepEqual(
    draftSelectionFromRouteWorkspace(
      routeWorkspaceFromDraftSelection(noneSelection),
    ),
    noneSelection,
  );
  // Unpinned routes stay unresolved: the draft-entry command resolves them.
  assert.equal(draftSelectionFromRouteWorkspace(null), null);
  assert.equal(routeWorkspaceFromDraftSelection(null), null);
  // The explicit-none literal cannot collide with absolute paths.
  assert.deepEqual(draftSelectionFromRouteWorkspace('none'), { kind: 'none' });
});

test('default resolution prefers latest activity, then server order', () => {
  const byActivity = resolveDefaultDraftWorkspace([
    workspace({ path: '/Users/test/a', lastActivityAt: '2026-07-01T00:00:00Z' }),
    workspace({ path: '/Users/test/b', lastActivityAt: '2026-07-20T00:00:00Z' }),
    workspace({ path: '/Users/test/c', lastActivityAt: null }),
  ]);
  assert.deepEqual(byActivity, { kind: 'path', path: '/Users/test/b' });

  // No activity anywhere → the first row of the server total order.
  const byOrder = resolveDefaultDraftWorkspace([
    workspace({ path: '/Users/test/first' }),
    workspace({ path: '/Users/test/second' }),
  ]);
  assert.deepEqual(byOrder, { kind: 'path', path: '/Users/test/first' });

  // Unavailable rows never become the default.
  const skipsUnavailable = resolveDefaultDraftWorkspace([
    workspace({ path: '/Users/test/broken', available: false }),
    workspace({ path: '/Users/test/ok' }),
  ]);
  assert.deepEqual(skipsUnavailable, { kind: 'path', path: '/Users/test/ok' });

  // An empty catalog means an implicit No-workspace draft, never an error.
  assert.deepEqual(resolveDefaultDraftWorkspace([]), { kind: 'none' });
});

test('resolution is a pure function of its input — refreshes cannot drift it', () => {
  const catalog = [
    workspace({ path: '/Users/test/a', lastActivityAt: '2026-07-10T00:00:00Z' }),
  ];
  const first = resolveDefaultDraftWorkspace(catalog);
  // A later refresh with different activity produces a different value, but
  // callers only invoke this at draft creation (and on removal of the
  // selected workspace) — asserting purity here pins that the function
  // itself holds no state to drift.
  const refreshed = resolveDefaultDraftWorkspace([
    workspace({ path: '/Users/test/b', lastActivityAt: '2026-07-21T00:00:00Z' }),
    ...catalog,
  ]);
  assert.deepEqual(first, { kind: 'path', path: '/Users/test/a' });
  assert.deepEqual(refreshed, { kind: 'path', path: '/Users/test/b' });
});
