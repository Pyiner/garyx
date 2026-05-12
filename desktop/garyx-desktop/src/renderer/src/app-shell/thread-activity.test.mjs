import test from 'node:test';
import assert from 'node:assert/strict';

import {
  activeRunAwaitsAssistant,
  deriveThreadActivityModel,
  latestUserMessageAwaitsAssistant,
  threadActivitySignature,
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

function activeRun(overrides = {}) {
  return {
    runId: 'run-1',
    providerType: 'codex',
    providerLabel: 'Codex',
    assistantResponse: null,
    updatedAt: '2026-05-09T00:00:01.000Z',
    pendingUserInputCount: 0,
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

test('active run shows loading for external channel user turn', () => {
  assert.equal(
    activeRunAwaitsAssistant({
      threadInfo: { activeRun: activeRun() },
      messages: [
        message({
          id: 'telegram-user-1',
          text: 'message from external channel',
        }),
      ],
    }),
    true,
  );
});

test('active run loading stops when assistant or tool follows user', () => {
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
  assert.equal(
    activeRunAwaitsAssistant({
      threadInfo: { activeRun: activeRun() },
      messages,
    }),
    false,
  );
});

test('pending ack loading suppresses duplicate active run loading', () => {
  assert.equal(
    activeRunAwaitsAssistant({
      threadInfo: { activeRun: activeRun() },
      messages: [message({ id: 'telegram-user-1' })],
      suppressForPendingAck: true,
    }),
    false,
  );
});

test('thread activity signature changes when only active run changes', () => {
  const messages = [message({ id: 'telegram-user-1' })];
  const withoutRun = threadActivitySignature(messages, [], {
    activeRun: null,
  });
  const withRun = threadActivitySignature(messages, [], {
    activeRun: activeRun(),
  });
  const clearedRun = threadActivitySignature(messages, [], {
    activeRun: null,
  });

  assert.notEqual(withoutRun, withRun);
  assert.equal(withoutRun, clearedRun);
});

test('thread activity signature tracks active run response updates', () => {
  const messages = [message({ id: 'telegram-user-1' })];
  const first = threadActivitySignature(messages, [], {
    activeRun: activeRun({ assistantResponse: 'partial one' }),
  });
  const second = threadActivitySignature(messages, [], {
    activeRun: activeRun({ assistantResponse: 'partial two' }),
  });

  assert.notEqual(first, second);
});

test('thread activity model treats snapshot active run as source-independent loading', () => {
  const model = deriveThreadActivityModel({
    messages: [message({ id: 'telegram-user-1' })],
    threadInfo: { activeRun: activeRun() },
    liveStream: null,
    runtimeBusy: false,
    pendingAckIntentCount: 0,
    remoteAwaitingAckInputCount: 0,
    pendingHistoryIntent: false,
  });

  assert.equal(model.runActive, true);
  assert.equal(model.showRunLoading, true);
  assert.equal(model.canSteerQueuedPrompt, false);
});

test('thread activity model keeps bottom thinking visible after streamed text', () => {
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
    threadInfo: { activeRun: activeRun() },
    liveStream: null,
    runtimeBusy: false,
    pendingAckIntentCount: 0,
    remoteAwaitingAckInputCount: 0,
    pendingHistoryIntent: false,
  });

  assert.equal(model.runActive, true);
  assert.equal(model.showRunLoading, true);
});

test('thread activity model avoids duplicate thinking for pending assistant rows', () => {
  const model = deriveThreadActivityModel({
    messages: [
      message({ id: 'user-1' }),
      message({
        id: 'assistant-1',
        role: 'assistant',
        text: 'Thinking',
        kind: 'assistant_reply',
        pending: true,
      }),
    ],
    threadInfo: { activeRun: activeRun() },
    liveStream: null,
    runtimeBusy: false,
    pendingAckIntentCount: 0,
    remoteAwaitingAckInputCount: 0,
    pendingHistoryIntent: false,
  });

  assert.equal(model.runActive, true);
  assert.equal(model.showRunLoading, false);
});

test('thread activity model allows steering only for local live streams', () => {
  const model = deriveThreadActivityModel({
    messages: [message({ id: 'local-user-1' })],
    threadInfo: { activeRun: null },
    liveStream: {
      threadId: 'thread-1',
      runId: 'run-1',
      pendingAckIntentIds: [],
      streamStatus: 'streaming',
    },
    runtimeBusy: false,
    pendingAckIntentCount: 0,
    remoteAwaitingAckInputCount: 0,
    pendingHistoryIntent: false,
  });

  assert.equal(model.runActive, true);
  assert.equal(model.canSteerQueuedPrompt, true);
});
