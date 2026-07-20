import test from 'node:test';
import assert from 'node:assert/strict';

import {
  buildWorkspaceThreadGroups,
  selectedThread,
  threadSummariesEquivalent,
} from './thread-model.ts';

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

test('workspace sidebar groups use only user-saved workspace rows', () => {
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
      makeThread('thread-manual', manualWithThread.path, {
        rootWorkspacePath: manualWithThread.path,
      }),
      makeThread('thread-inferred', inferredWithThread.path, {
        rootWorkspacePath: inferredWithThread.path,
      }),
      // A worktree thread's membership comes from the server-derived root;
      // its runtime path never groups it anywhere.
      makeThread('thread-worktree', worktreeWorkspace.path, {
        rootWorkspacePath: manualWithThread.path,
        worktree: {
          worktreeDir: worktreeWorkspace.path,
        },
      }),
      // An implicit thread (server root null) belongs to no workspace even
      // though its runtime path exists.
      makeThread('thread-implicit', '/Users/test/data/thread-workspaces/thread--implicit', {
        rootWorkspacePath: null,
        workspaceOrigin: 'implicit',
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
    [manualEmpty.path, manualWithThread.path],
  );
  assert.equal(groups.find((group) => group.workspace.path === manualEmpty.path)?.threads.length, 0);
  assert.equal(groups.find((group) => group.workspace.path === inferredWithThread.path), undefined);
  assert.deepEqual(
    groups.find((group) => group.workspace.path === manualWithThread.path)?.threads.map((thread) => thread.id),
    ['thread-manual', 'thread-worktree'],
  );
  assert.equal(groups[0].canManageWorkspace, true);
  assert.equal(groups[1].canManageWorkspace, true);
});

test('selectedThread can resolve cached hidden session threads', () => {
  const hiddenChild = makeThread('thread-hidden-child', '/Users/test/project');
  const state = makeState({
    sessions: [hiddenChild],
  });

  assert.equal(selectedThread(state, hiddenChild.id)?.id, hiddenChild.id);
});

test('threadSummariesEquivalent treats re-fetched identical summaries as equal', () => {
  const makeSummary = () =>
    makeThread('thread-side-chat', '/Users/test/project', {
      messageCount: 4,
      agentId: 'claude',
      recentRunId: null,
      worktree: { branch: 'main', path: '/Users/test/project' },
    });

  assert.equal(threadSummariesEquivalent(makeSummary(), makeSummary()), true);
});

test('threadSummariesEquivalent normalizes missing optional fields to null', () => {
  const left = makeThread('thread-side-chat', '/Users/test/project');
  const right = makeThread('thread-side-chat', '/Users/test/project', {
    agentId: null,
    recentRunId: null,
    worktree: null,
  });

  assert.equal(threadSummariesEquivalent(left, right), true);
});

test('threadSummariesEquivalent detects meaningful changes', () => {
  const base = makeThread('thread-side-chat', '/Users/test/project', {
    messageCount: 4,
  });

  assert.equal(
    threadSummariesEquivalent(base, { ...base, updatedAt: '2026-01-02T00:00:00.000Z' }),
    false,
  );
  assert.equal(
    threadSummariesEquivalent(base, { ...base, messageCount: 5 }),
    false,
  );
  assert.equal(
    threadSummariesEquivalent(base, { ...base, recentRunId: 'run-1' }),
    false,
  );
  assert.equal(
    threadSummariesEquivalent(base, { ...base, worktree: { branch: 'main' } }),
    false,
  );
});
