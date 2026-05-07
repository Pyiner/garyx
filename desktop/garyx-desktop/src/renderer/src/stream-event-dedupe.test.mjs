import test from 'node:test';
import assert from 'node:assert/strict';

import {
  createChatStreamEventDedupeState,
  shouldAcceptChatStreamEvent,
} from './stream-event-dedupe.ts';

function deltaEvent(eventSeq, delta = 'hello') {
  return {
    type: 'assistant_delta',
    runId: 'run-test-1',
    threadId: 'thread::test-stream',
    eventSeq,
    delta,
  };
}

test('deduplicates repeated stream events by thread, run, and sequence', () => {
  const state = createChatStreamEventDedupeState();

  assert.equal(shouldAcceptChatStreamEvent(state, deltaEvent(1)), true);
  assert.equal(shouldAcceptChatStreamEvent(state, deltaEvent(1)), false);
  assert.equal(shouldAcceptChatStreamEvent(state, deltaEvent(2)), true);
});

test('does not guess duplicates when stream sequence is absent', () => {
  const state = createChatStreamEventDedupeState();
  const event = deltaEvent(undefined, 'repeated');

  assert.equal(shouldAcceptChatStreamEvent(state, event), true);
  assert.equal(shouldAcceptChatStreamEvent(state, event), true);
});
