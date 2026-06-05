import test from 'node:test';
import assert from 'node:assert/strict';

import { desktopStateWithoutThread } from './desktop-state.ts';

function thread(id) {
  return {
    id,
    title: `Thread ${id}`,
    createdAt: '2026-01-01T00:00:00.000Z',
    updatedAt: '2026-01-01T00:00:00.000Z',
    lastMessagePreview: '',
    workspacePath: '/Users/test/project',
  };
}

function endpoint(endpointKey, threadId) {
  return {
    endpointKey,
    channel: 'test-channel',
    accountId: 'test-account',
    peerId: 'peer',
    chatId: 'chat',
    deliveryTargetType: 'chat_id',
    deliveryTargetId: 'chat',
    displayLabel: endpointKey,
    threadId,
    threadLabel: threadId,
    workspacePath: '/Users/test/project',
    threadUpdatedAt: '2026-01-01T00:00:00.000Z',
    lastInboundAt: null,
    lastDeliveryAt: null,
    conversationKind: 'group',
    conversationLabel: endpointKey,
  };
}

function state(overrides = {}) {
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

test('desktopStateWithoutThread removes archived thread and visible associations', () => {
  const archivedThread = thread('thread-archive');
  const keptThread = thread('thread-keep');
  const archivedEndpoint = endpoint('endpoint-archive', archivedThread.id);
  const keptEndpoint = endpoint('endpoint-keep', keptThread.id);
  const next = desktopStateWithoutThread(
    state({
      threads: [archivedThread, keptThread],
      sessions: [archivedThread, keptThread],
      pinnedThreadIds: [archivedThread.id, keptThread.id],
      endpoints: [archivedEndpoint, keptEndpoint],
      configuredBots: [{
        channel: 'test-channel',
        accountId: 'test-account',
        displayName: 'Test Bot',
        enabled: true,
        workspaceDir: '/Users/test/project',
        rootBehavior: 'open_default',
        mainEndpointStatus: 'resolved',
        mainEndpoint: archivedEndpoint,
        mainEndpointThreadId: archivedThread.id,
        defaultOpenEndpoint: archivedEndpoint,
        defaultOpenThreadId: archivedThread.id,
      }],
      botConsoles: [{
        id: 'test-channel::test-account',
        channel: 'test-channel',
        accountId: 'test-account',
        title: 'Test Bot',
        subtitle: 'Test Bot',
        rootBehavior: 'open_default',
        status: 'connected',
        latestActivity: null,
        endpointCount: 2,
        boundEndpointCount: 2,
        workspaceDir: '/Users/test/project',
        mainEndpointStatus: 'resolved',
        mainEndpoint: archivedEndpoint,
        mainThreadId: archivedThread.id,
        defaultOpenEndpoint: archivedEndpoint,
        defaultOpenThreadId: archivedThread.id,
        conversationNodes: [{
          id: 'archived-node',
          endpoint: archivedEndpoint,
          kind: 'group',
          title: 'Archived',
          badge: null,
          latestActivity: null,
          openable: true,
        }],
        endpoints: [archivedEndpoint, keptEndpoint],
      }],
      botMainThreads: {
        'test-channel::test-account': archivedThread.id,
        'other-channel::test-account': keptThread.id,
      },
    }),
    archivedThread.id,
  );

  assert.deepEqual(next.threads.map((entry) => entry.id), [keptThread.id]);
  assert.deepEqual(next.sessions.map((entry) => entry.id), [keptThread.id]);
  assert.deepEqual(next.pinnedThreadIds, [keptThread.id]);
  assert.deepEqual(next.endpoints.map((entry) => entry.endpointKey), [keptEndpoint.endpointKey]);
  assert.equal(next.configuredBots[0].mainEndpoint, null);
  assert.equal(next.configuredBots[0].mainEndpointThreadId, null);
  assert.equal(next.configuredBots[0].defaultOpenEndpoint, null);
  assert.equal(next.configuredBots[0].defaultOpenThreadId, null);
  assert.equal(next.botConsoles[0].mainEndpoint, null);
  assert.equal(next.botConsoles[0].mainThreadId, null);
  assert.equal(next.botConsoles[0].defaultOpenEndpoint, null);
  assert.equal(next.botConsoles[0].defaultOpenThreadId, null);
  assert.deepEqual(next.botConsoles[0].conversationNodes, []);
  assert.deepEqual(next.botConsoles[0].endpoints.map((entry) => entry.endpointKey), [keptEndpoint.endpointKey]);
  assert.deepEqual(next.botMainThreads, {
    'other-channel::test-account': keptThread.id,
  });
});
