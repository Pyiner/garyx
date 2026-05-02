import test from 'node:test';
import assert from 'node:assert/strict';

import {
  initialMessageMachineState,
  messageMachineReducer,
} from './message-machine.ts';

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
