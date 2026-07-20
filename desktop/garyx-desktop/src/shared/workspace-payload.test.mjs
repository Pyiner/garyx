import assert from 'node:assert/strict';
import { test } from 'node:test';

import {
  parseDirectoryListingPayload,
  parseWorkspaceCatalogPayload,
} from './workspace-payload.ts';

function workspacePayload(overrides = {}) {
  return {
    name: 'garyx',
    path: '/Users/test/repos/garyx',
    pinned: false,
    thread_count: 0,
    last_activity_at: null,
    git_repo: false,
    ...overrides,
  };
}

test('catalog parsing preserves gateway order and names verbatim', () => {
  // Deliberately not alphabetical and not activity-ordered: the gateway owns
  // the total order (pinned, activity, name, path) and the client must not
  // reorder or rewrite what it received.
  const catalog = parseWorkspaceCatalogPayload({
    workspace_state_initialized: true,
    gateway_home: '/Users/test',
    workspaces: [
      workspacePayload({ name: 'zeta pinned', path: '/Users/test/z', pinned: true }),
      workspacePayload({
        name: 'Custom Display Name',
        path: '/Users/test/repos/garyx',
        thread_count: 42,
        last_activity_at: '2026-07-20T00:00:00Z',
        git_repo: true,
      }),
      workspacePayload({ name: 'alpha', path: '/Users/test/a' }),
    ],
  });

  assert.equal(catalog.gatewayHome, '/Users/test');
  assert.equal(catalog.workspaceStateInitialized, true);
  assert.deepEqual(
    catalog.workspaces.map((workspace) => workspace.name),
    ['zeta pinned', 'Custom Display Name', 'alpha'],
  );
  // A custom display name never falls back to the path basename.
  assert.equal(catalog.workspaces[1].name, 'Custom Display Name');
  assert.equal(catalog.workspaces[1].threadCount, 42);
  assert.equal(catalog.workspaces[1].lastActivityAt, '2026-07-20T00:00:00Z');
  assert.equal(catalog.workspaces[1].gitRepo, true);
  assert.equal(catalog.workspaces[0].pinned, true);
});

test('catalog parsing rejects payloads missing the server-owned fields', () => {
  for (const missing of ['pinned', 'thread_count', 'git_repo']) {
    const payload = workspacePayload();
    delete payload[missing];
    assert.throws(
      () =>
        parseWorkspaceCatalogPayload({
          workspace_state_initialized: true,
          gateway_home: null,
          workspaces: [payload],
        }),
      new RegExp(missing),
    );
  }
});

test('directory listing parsing carries the git badge per entry', () => {
  const listing = parseDirectoryListingPayload({
    path: '/Users/test/repos',
    parentPath: '/Users/test',
    entries: [
      { name: 'garyx', path: '/Users/test/repos/garyx', gitRepo: true },
      { name: 'notes', path: '/Users/test/repos/notes', gitRepo: false },
    ],
  });
  assert.deepEqual(
    listing.entries.map((entry) => [entry.name, entry.gitRepo]),
    [
      ['garyx', true],
      ['notes', false],
    ],
  );
  assert.equal(listing.parentPath, '/Users/test');
});
