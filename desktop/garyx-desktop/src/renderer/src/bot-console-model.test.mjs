import test from 'node:test';
import assert from 'node:assert/strict';

import {
  botRootBoundThreadId,
  buildBotGroups,
  buildBotSidebarThreadEntries,
} from './bot-console-model.ts';

function makeEndpoint(overrides = {}) {
  return {
    endpointKey: 'weixin::zhao::default',
    channel: 'weixin',
    accountId: 'zhao',
    peerId: 'peer-1',
    chatId: 'chat-1',
    deliveryTargetType: 'chat_id',
    deliveryTargetId: 'chat-1',
    displayLabel: 'Smoke test endpoint',
    threadId: 'thread::smoke',
    ...overrides,
  };
}

function makeBotGroup(overrides = {}) {
  const defaultOpenEndpoint = makeEndpoint();
  return {
    id: 'weixin::zhao',
    channel: 'weixin',
    accountId: 'zhao',
    title: 'Test User',
    subtitle: 'Weixin Bot · zhao',
    rootBehavior: 'open_default',
    status: 'connected',
    latestActivity: null,
    endpointCount: 1,
    boundEndpointCount: 1,
    workspaceDir: null,
    mainEndpointStatus: 'unresolved',
    mainEndpoint: null,
    mainThreadId: null,
    defaultOpenEndpoint,
    defaultOpenThreadId: defaultOpenEndpoint.threadId,
    conversationNodes: [],
    endpoints: [defaultOpenEndpoint],
    ...overrides,
  };
}

test('bot root bound thread ignores default-open conversation threads', () => {
  const group = makeBotGroup();

  assert.equal(botRootBoundThreadId(group), null);
});

test('bot root bound thread prefers explicit main binding over default-open thread', () => {
  const group = makeBotGroup({
    mainThreadId: 'thread::main',
  });

  assert.equal(botRootBoundThreadId(group), 'thread::main');
});

test('bot sidebar conversations use stable label order instead of activity order', () => {
  const group = makeBotGroup({
    endpoints: [
      makeEndpoint({
        endpointKey: 'weixin::test-account::z-room',
        chatId: 'z-room',
        deliveryTargetId: 'z-room',
        displayLabel: 'Z Room',
        conversationKind: 'group',
        lastInboundAt: '2026-03-16T03:00:00Z',
        threadId: 'thread::z-room',
      }),
      makeEndpoint({
        endpointKey: 'weixin::test-account::a-room',
        chatId: 'a-room',
        deliveryTargetId: 'a-room',
        displayLabel: 'A Room',
        conversationKind: 'group',
        lastInboundAt: '2026-03-16T01:00:00Z',
        threadId: 'thread::a-room',
      }),
    ],
  });

  assert.deepEqual(
    buildBotSidebarThreadEntries(group).map((entry) => entry.title),
    ['A Room', 'Z Room'],
  );
});

test('bot group endpoints use stable label order instead of activity order', () => {
  const group = makeBotGroup({
    endpoints: [
      makeEndpoint({
        endpointKey: 'weixin::test-account::z-room',
        displayLabel: 'Z Room',
        lastInboundAt: '2026-03-16T03:00:00Z',
      }),
      makeEndpoint({
        endpointKey: 'weixin::test-account::a-room',
        displayLabel: 'A Room',
        lastInboundAt: '2026-03-16T01:00:00Z',
      }),
    ],
  });

  const [built] = buildBotGroups([], [], {}, [group]);

  assert.deepEqual(
    built.endpoints.map((endpoint) => endpoint.displayLabel),
    ['A Room', 'Z Room'],
  );
});
