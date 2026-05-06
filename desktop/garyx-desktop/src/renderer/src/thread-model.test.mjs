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

function makeThread(id, workspacePath) {
  return {
    id,
    title: `Thread ${id}`,
    createdAt: '2026-01-01T00:00:00.000Z',
    updatedAt: '2026-01-01T00:00:00.000Z',
    lastMessagePreview: '',
    workspacePath,
  };
}

function makeState(overrides = {}) {
  return {
    settings: {},
    gatewayProfiles: [],
    workspaces: [],
    hiddenWorkspacePaths: [],
    selectedWorkspacePath: null,
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

test('workspace sidebar groups only include manually added workspaces', () => {
  const manualEmpty = makeWorkspace('/Users/test/manual-empty');
  const manualWithThread = makeWorkspace('/Users/test/manual-with-thread');
  const inferredWithThread = makeWorkspace('/Users/test/inferred-with-thread', {
    managed: true,
  });
  const state = makeState({
    workspaces: [manualEmpty, inferredWithThread, manualWithThread],
    threads: [
      makeThread('thread-manual', manualWithThread.path),
      makeThread('thread-inferred', inferredWithThread.path),
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
    [manualEmpty.path, manualWithThread.path],
  );
  assert.equal(groups[0].threads.length, 0);
  assert.deepEqual(groups[1].threads.map((thread) => thread.id), ['thread-manual']);
  assert.equal(groups[0].canManageWorkspace, true);
  assert.equal(groups[1].canManageWorkspace, true);
});
