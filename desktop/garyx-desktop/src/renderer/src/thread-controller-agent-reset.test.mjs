import assert from 'node:assert/strict';
import test from 'node:test';

import { suggestedAgentId } from './app-shell/agent-availability-model.ts';
import { ensureThread } from './thread-controller.ts';

test('creating a thread clears its one-time agent override so the next draft uses effective default', async () => {
  const createRequests = [];
  const pendingAgentWrites = [];
  let pendingAgentId = 'codex';

  const threadId = await ensureThread({
    api: {
      createThread: async (input) => {
        createRequests.push(input);
        return {
          state: { threads: [] },
          thread: { id: 'thread::created' },
        };
      },
    },
    selectedThreadId: null,
    pendingWorkspacePath: '/Users/test/project',
    pendingWorkspaceMode: 'local',
    pendingAgentId,
    selectableWorkspaceCount: 1,
    setWorkspaceMutation: () => {},
    setDesktopState: () => {},
    setSelectedThreadId: () => {},
    initializeThreadMessages: () => {},
    setNewThreadDraftActive: () => {},
    setPendingWorkspaceSelection: () => {},
    setPendingWorkspaceMode: () => {},
    setPendingBotId: () => {},
    setPendingAgentId: (value) => {
      pendingAgentId = value;
      pendingAgentWrites.push(value);
    },
    setError: (error) => assert.equal(error, null),
  });

  assert.equal(threadId, 'thread::created');
  assert.equal(createRequests[0].agentId, 'codex');
  assert.deepEqual(pendingAgentWrites, [null]);

  if (pendingAgentId === null) {
    pendingAgentId = suggestedAgentId({ effectiveDefaultAgentId: 'codex' });
  }
  assert.equal(pendingAgentId, 'codex');
});
