import test from 'node:test';
import assert from 'node:assert/strict';

import { botRootBoundThreadId } from './bot-console-model.ts';

function makeEndpoint(overrides = {}) {
  return {
    endpointKey: 'weixin::zhao::default',
    channel: 'weixin',
    accountId: 'zhao',
    peerId: 'peer-1',
    chatId: 'chat-1',
    deliveryTargetType: 'chat_id',
    deliveryTargetId: 'chat-1',
    displayLabel: '真实派活 smoke test',
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
    title: '赵婉潇',
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
