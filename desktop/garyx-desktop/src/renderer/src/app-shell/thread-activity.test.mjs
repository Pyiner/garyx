import test from 'node:test';
import assert from 'node:assert/strict';

import {
  deriveThreadActivityModel,
  latestUserMessageAwaitsAssistant,
} from './thread-activity.ts';

function message(overrides) {
  return {
    id: 'msg-1',
    role: 'user',
    text: 'hello',
    timestamp: '2026-05-09T00:00:00.000Z',
    kind: 'user_input',
    internalKind: null,
    internal: false,
    ...overrides,
  };
}

test('external user message without local intent waits for assistant', () => {
  assert.equal(
    latestUserMessageAwaitsAssistant([
      message({
        id: 'telegram-user-1',
        text: 'message from external channel',
      }),
    ]),
    true,
  );
});

test('latest user wait stops when assistant or tool follows user', () => {
  const messages = [
    message({ id: 'telegram-user-1' }),
    message({
      id: 'assistant-1',
      role: 'assistant',
      text: 'done',
      kind: 'assistant_reply',
    }),
  ];

  assert.equal(latestUserMessageAwaitsAssistant(messages), false);
});

test('thread activity model uses runtime busy as the remote business gate', () => {
  const model = deriveThreadActivityModel({
    messages: [message({ id: 'telegram-user-1' })],
    runtimeBusy: true,
    pendingAckIntentCount: 0,
    remoteAwaitingAckInputCount: 0,
    pendingHistoryIntent: false,
  });

  assert.equal(model.runActive, true);
  assert.equal(model.canSteerQueuedPrompt, true);
});

test('server render thinking keeps the composer interruptible when local runtime is idle', () => {
  const model = deriveThreadActivityModel({
    messages: [message({ id: 'telegram-user-1' })],
    runtimeBusy: false,
    renderTailActivity: 'thinking',
    renderActiveToolGroupId: null,
    pendingAckIntentCount: 0,
    remoteAwaitingAckInputCount: 0,
    pendingHistoryIntent: false,
  });

  assert.equal(model.runActive, true);
  assert.equal(model.canSteerQueuedPrompt, true);
});

test('thread activity model does not derive rendered loading from messages', () => {
  const model = deriveThreadActivityModel({
    messages: [
      message({ id: 'user-1' }),
      message({
        id: 'assistant-1',
        role: 'assistant',
        text: 'partial answer',
        kind: 'assistant_reply',
      }),
    ],
    runtimeBusy: false,
    pendingAckIntentCount: 0,
    remoteAwaitingAckInputCount: 0,
    pendingHistoryIntent: false,
  });

  assert.equal(model.runActive, false);
  assert.equal(model.canSteerQueuedPrompt, false);
});

test('thread activity model allows steering while waiting for remote ack', () => {
  const model = deriveThreadActivityModel({
    messages: [message({ id: 'local-user-1' })],
    runtimeBusy: false,
    pendingAckIntentCount: 1,
    remoteAwaitingAckInputCount: 0,
    pendingHistoryIntent: false,
  });

  assert.equal(model.runActive, false);
  assert.equal(model.canSteerQueuedPrompt, true);
});
