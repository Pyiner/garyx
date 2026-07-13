import test from 'node:test';
import assert from 'node:assert/strict';

import {
  findPendingAckIntentIndex,
  initialMessageMachineState,
  messageMachineReducer,
  shouldTrackProviderAckAfterStreamInputResponse,
} from './message-machine.ts';
import { GatewayMirror } from './gateway-mirror/mirror.ts';

function intent(overrides) {
  return {
    intentId: 'intent-1',
    threadId: 'thread-1',
    text: 'hello',
    images: [],
    files: [],
    createdAt: '2026-05-09T00:00:00.000Z',
    updatedAt: '2026-05-09T00:00:00.000Z',
    state: 'queued_local',
    source: 'composer_queue',
    ...overrides,
  };
}

test('moves draft thread state to the created thread id', () => {
  const state = {
    ...initialMessageMachineState,
    intentsById: {
      'intent-1': {
        intentId: 'intent-1',
        threadId: '__garyx_new_thread_draft__',
        text: 'hello',
        images: [],
        files: [],
        createdAt: '2026-05-03T00:00:00.000Z',
        updatedAt: '2026-05-03T00:00:00.000Z',
        state: 'dispatch_requested',
        source: 'composer_send',
        dispatchMode: 'sync_send',
      },
    },
    queueByThread: {
      thread2: ['intent-2'],
      __garyx_new_thread_draft__: ['intent-1'],
    },
    threadRuntimeByThread: {
      __garyx_new_thread_draft__: {
        threadId: '__garyx_new_thread_draft__',
        state: 'dispatching_sync',
        activeIntentId: 'intent-1',
        updatedAt: '2026-05-03T00:00:00.000Z',
      },
    },
  };

  const next = messageMachineReducer(state, {
    type: 'thread/replace-id',
    fromThreadId: '__garyx_new_thread_draft__',
    toThreadId: 'thread2',
  });

  assert.equal(next.intentsById['intent-1'].threadId, 'thread2');
  assert.deepEqual(next.queueByThread.thread2, ['intent-2', 'intent-1']);
  assert.equal(next.queueByThread.__garyx_new_thread_draft__, undefined);
  assert.equal(
    next.threadRuntimeByThread.__garyx_new_thread_draft__,
    undefined,
  );
  assert.equal(next.threadRuntimeByThread.thread2.threadId, 'thread2');
  assert.equal(next.threadRuntimeByThread.thread2.activeIntentId, 'intent-1');
});

test('tracks queued downstream input until provider ack', () => {
  const created = messageMachineReducer(initialMessageMachineState, {
    type: 'intent/created',
    enqueue: true,
    intent: intent({
      intentId: 'intent-follow-up-1',
      threadId: 'thread-1',
      text: 'follow up',
    }),
  });

  const dispatched = messageMachineReducer(created, {
    type: 'intent/request-dispatch',
    threadId: 'thread-1',
    intentId: 'intent-follow-up-1',
    mode: 'async_steer',
    source: 'queue_steer',
    removeFromQueue: false,
  });

  const queued = messageMachineReducer(dispatched, {
    type: 'intent/remote-accepted',
    intentId: 'intent-follow-up-1',
    runId: 'run-1',
    threadId: 'thread-1',
    pendingInputId: 'queued_input:1',
    removeFromQueue: true,
    awaitProviderAck: true,
  });

  assert.equal(
    queued.intentsById['intent-follow-up-1'].state,
    'awaiting_provider_ack',
  );
  assert.equal(
    queued.intentsById['intent-follow-up-1'].pendingInputId,
    'queued_input:1',
  );
  assert.deepEqual(queued.queueByThread['thread-1'], []);

  const acked = messageMachineReducer(queued, {
    type: 'intent/awaiting-history',
    intentId: 'intent-follow-up-1',
  });

  assert.equal(
    acked.intentsById['intent-follow-up-1'].state,
    'awaiting_history',
  );
});

test('keeps awaiting provider ack stable across duplicate remote accepted events', () => {
  const state = {
    ...initialMessageMachineState,
    intentsById: {
      'intent-follow-up-1': intent({
        intentId: 'intent-follow-up-1',
        state: 'awaiting_provider_ack',
        remoteRunId: 'run-1',
        remoteThreadKey: 'thread-1',
        pendingInputId: 'queued_input:1',
      }),
    },
  };

  const next = messageMachineReducer(state, {
    type: 'intent/remote-accepted',
    intentId: 'intent-follow-up-1',
    runId: 'run-1',
    threadId: 'thread-1',
    removeFromQueue: false,
  });

  assert.equal(
    next.intentsById['intent-follow-up-1'].state,
    'awaiting_provider_ack',
  );
  assert.equal(
    next.intentsById['intent-follow-up-1'].pendingInputId,
    'queued_input:1',
  );
});

test('requeue clears stale downstream ack identity', () => {
  const state = {
    ...initialMessageMachineState,
    intentsById: {
      'intent-follow-up-1': intent({
        intentId: 'intent-follow-up-1',
        state: 'awaiting_provider_ack',
        remoteRunId: 'run-1',
        remoteThreadKey: 'thread-1',
        pendingInputId: 'queued_input:1',
      }),
    },
    queueByThread: {
      'thread-1': [],
    },
  };

  const next = messageMachineReducer(state, {
    type: 'intent/requeue-front',
    threadId: 'thread-1',
    intentId: 'intent-follow-up-1',
    error: 'temporary failure',
  });

  assert.equal(next.intentsById['intent-follow-up-1'].state, 'queued_local');
  assert.equal(next.intentsById['intent-follow-up-1'].pendingInputId, undefined);
  assert.deepEqual(next.queueByThread['thread-1'], ['intent-follow-up-1']);
});

test('matches provider ack by exact pending input id', () => {
  const pendingAckIntentIds = ['intent-1', 'intent-2'];
  const intentsById = {
    'intent-1': intent({
      intentId: 'intent-1',
      pendingInputId: 'queued_input:1',
    }),
    'intent-2': intent({
      intentId: 'intent-2',
      pendingInputId: 'queued_input:2',
    }),
  };

  assert.equal(
    findPendingAckIntentIndex(
      pendingAckIntentIds,
      'queued_input:2',
      intentsById,
    ),
    1,
  );
});

test('matches provider ack to the only unresolved downstream intent', () => {
  const pendingAckIntentIds = ['intent-1', 'intent-2'];
  const intentsById = {
    'intent-1': intent({
      intentId: 'intent-1',
      pendingInputId: 'queued_input:1',
    }),
    'intent-2': intent({
      intentId: 'intent-2',
      pendingInputId: undefined,
    }),
  };

  assert.equal(
    findPendingAckIntentIndex(
      pendingAckIntentIds,
      'queued_input:2',
      intentsById,
    ),
    1,
  );
});

test('does not match unknown provider ack when all pending input ids are resolved', () => {
  const pendingAckIntentIds = ['intent-1', 'intent-2'];
  const intentsById = {
    'intent-1': intent({
      intentId: 'intent-1',
      pendingInputId: 'queued_input:1',
    }),
    'intent-2': intent({
      intentId: 'intent-2',
      pendingInputId: 'queued_input:2',
    }),
  };

  assert.equal(
    findPendingAckIntentIndex(
      pendingAckIntentIds,
      'queued_input:missing',
      intentsById,
    ),
    -1,
  );
});

test('tracks provider ack only while the streamed input is not already acknowledged', () => {
  assert.equal(
    shouldTrackProviderAckAfterStreamInputResponse(intent({
      state: 'dispatching',
    })),
    true,
  );
  assert.equal(
    shouldTrackProviderAckAfterStreamInputResponse(intent({
      state: 'awaiting_provider_ack',
    })),
    true,
  );
  assert.equal(
    shouldTrackProviderAckAfterStreamInputResponse(intent({
      state: 'awaiting_history',
    })),
    false,
  );
  assert.equal(
    shouldTrackProviderAckAfterStreamInputResponse(intent({
      state: 'completed',
    })),
    false,
  );
});

test('thread clear releases completed attachment intents instead of retaining the session history', () => {
  const threadId = 'thread-attachment-gc';
  const intentCount = 64;
  const attachmentData = 'A'.repeat(16 * 1024);
  const mirror = new GatewayMirror();

  for (let index = 0; index < intentCount; index += 1) {
    const intentId = `intent-attachment-${index}`;
    mirror.dispatchMachineAction({
      type: 'intent/created',
      enqueue: false,
      intent: intent({
        intentId,
        threadId,
        text: `message ${index}`,
        images: [
          {
            id: `image-${index}`,
            name: `image-${index}.png`,
            mediaType: 'image/png',
            data: attachmentData,
          },
        ],
        state: 'awaiting_history',
        source: 'composer_send',
      }),
    });
    mirror.dispatchMachineAction({
      type: 'intent/completed',
      intentId,
    });
  }

  let state = mirror.getMachineState();
  assert.equal(Object.keys(state.intentsById).length, intentCount);
  let notifications = 0;
  const unsubscribe = mirror.subscribeMachine(() => {
    notifications += 1;
    const committed = mirror.getMachineState();
    assert.equal(Object.keys(committed.intentsById).length, 0);
  });
  state = mirror.dispatchMachineAction({ type: 'thread/clear', threadId });
  unsubscribe();

  assert.equal(Object.keys(state.intentsById).length, 0);
  assert.equal(state.queueByThread[threadId], undefined);
  assert.equal(notifications, 1, 'runtime clear and compaction publish atomically');
});

test('desktop thread release retains terminal intents while transcript reconciliation references them', () => {
  const threadId = 'thread-reference-aware-gc';
  const localIntentId = 'intent-local-reference';
  const pendingIntentId = 'intent-pending-reference';
  const mirror = new GatewayMirror();

  for (const createdIntent of [
    intent({
      intentId: localIntentId,
      threadId,
      state: 'completed',
    }),
    intent({
      intentId: pendingIntentId,
      threadId,
      pendingInputId: 'pending-input-1',
      state: 'failed',
    }),
  ]) {
    mirror.dispatchMachineAction({
      type: 'intent/created',
      enqueue: false,
      intent: createdIntent,
    });
  }
  mirror.syncThreadUiMessages(threadId, [{
    id: `origin:${localIntentId}`,
    role: 'user',
    text: 'optimistic message',
    intentId: localIntentId,
    localState: 'optimistic',
  }]);
  mirror.applyRemoteTranscript(threadId, {
    threadId,
    remoteFound: true,
    messages: [],
    pendingInputs: [{
      id: 'pending-input-1',
      text: 'queued input',
      status: 'awaiting_ack',
      active: true,
    }],
    threadInfo: null,
  });

  let state = mirror.dispatchMachineAction({ type: 'thread/clear', threadId });
  assert.ok(state.intentsById[localIntentId]);
  assert.ok(state.intentsById[pendingIntentId]);

  mirror.syncThreadUiMessages(threadId, [{
    id: `origin:${localIntentId}`,
    role: 'user',
    text: 'optimistic message',
    intentId: localIntentId,
    localState: 'remote_final',
  }]);
  mirror.applyRemoteTranscript(threadId, {
    threadId,
    remoteFound: true,
    messages: [],
    pendingInputs: [],
    threadInfo: null,
  });
  state = mirror.dispatchMachineAction({ type: 'thread/clear', threadId });

  assert.equal(state.intentsById[localIntentId], undefined);
  assert.equal(state.intentsById[pendingIntentId], undefined);
});

test('canonical thread clear remains runtime-only for shared state-machine callers', () => {
  const threadId = 'thread-shared-clear-contract';
  const created = messageMachineReducer(initialMessageMachineState, {
    type: 'intent/created',
    enqueue: false,
    intent: intent({
      intentId: 'intent-shared-clear-contract',
      threadId,
      state: 'completed',
    }),
  });
  const next = messageMachineReducer(created, { type: 'thread/clear', threadId });

  assert.ok(next.intentsById['intent-shared-clear-contract']);
});
