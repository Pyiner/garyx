import test from 'node:test';
import assert from 'node:assert/strict';

import { buildWorkspaceThreadGroups } from './thread-model.ts';

function makeWorkspace(path, overrides = {}) {
  return {
    name: path.split('/').pop() || 'workspace',
    path,
    kind: 'local',
    createdAt: '2026-01-01T00:00:00.000Z',
    updatedAt: '2026-01-01T00:00:00.000Z',
    available: true,
    ...overrides,
  };
}

function makeThread(id, workspacePath, overrides = {}) {
  return {
    id,
    title: `Thread ${id}`,
    createdAt: '2026-01-01T00:00:00.000Z',
    updatedAt: '2026-01-01T00:00:00.000Z',
    lastMessagePreview: '',
    workspacePath,
    ...overrides,
  };
}

function makeState(overrides = {}) {
  return {
    settings: {},
    gatewayProfiles: [],
    workspaces: [],
    hiddenWorkspacePaths: [],
    selectedWorkspacePath: null,
    pinnedThreadIds: [],
    threads: [],
    sessions: [],
    endpoints: [],
    configuredBots: [],
    botConsoles: [],
    automations: [],
    selectedAutomationId: null,
    lastSeenRunAtByAutomation: {},
    botMainThreads: {},
    remoteErrors: [],
    ...overrides,
  };
}

test('workspace sidebar groups exclude only worktree directories', () => {
  const manualEmpty = makeWorkspace('/Users/test/manual-empty');
  const manualWithThread = makeWorkspace('/Users/test/manual-with-thread');
  const inferredWithThread = makeWorkspace('/Users/test/inferred-with-thread', {
    managed: true,
  });
  const worktreeWorkspace = makeWorkspace('/Users/test/thread-worktree', {
    managed: true,
  });
  const garyxWorktreeStorage = makeWorkspace('/Users/test/.garyx/worktrees/abc123/garyx', {
    managed: true,
  });
  const codexWorktreeStorage = makeWorkspace('/Users/test/.codex/worktrees/def456/garyx', {
    managed: true,
  });
  const state = makeState({
    workspaces: [
      manualEmpty,
      inferredWithThread,
      manualWithThread,
      worktreeWorkspace,
      garyxWorktreeStorage,
      codexWorktreeStorage,
    ],
    threads: [
      makeThread('thread-manual', manualWithThread.path),
      makeThread('thread-inferred', inferredWithThread.path),
      makeThread('thread-worktree', worktreeWorkspace.path, {
        worktree: {
          worktreeDir: worktreeWorkspace.path,
        },
      }),
    ],
  });

  const groups = buildWorkspaceThreadGroups({
    state,
    activeThread: null,
    selectedThreadId: null,
    workspaceSelectionEntry: null,
  });

  assert.deepEqual(
    groups.map((group) => group.workspace.path),
    [manualEmpty.path, inferredWithThread.path, manualWithThread.path],
  );
  assert.equal(groups.find((group) => group.workspace.path === manualEmpty.path)?.threads.length, 0);
  assert.deepEqual(
    groups.find((group) => group.workspace.path === inferredWithThread.path)?.threads.map((thread) => thread.id),
    ['thread-inferred'],
  );
  assert.deepEqual(
    groups.find((group) => group.workspace.path === manualWithThread.path)?.threads.map((thread) => thread.id),
    ['thread-manual'],
  );
  assert.equal(groups[0].canManageWorkspace, true);
  assert.equal(groups[1].canManageWorkspace, true);
});
