import test from 'node:test';
import assert from 'node:assert/strict';

import {
  visibleRemotePendingInputsForThread,
} from './pending-inputs.ts';

test('remote queued input is hidden after its committed user row is represented', () => {
  // Sanitized from the captured thread_render_frame window for the duplicate
  // bubble report: one committed user row with metadata.queued_input_id plus
  // one stale pending_user_inputs entry for the same queued follow-up.
  const pendingInput = {
    id: 'queued_input:test-follow-up',
    runId: 'run:test',
    text: 'Test queued follow-up',
    content: 'Test queued follow-up',
    timestamp: '2026-06-24T06:32:44.000Z',
    status: 'awaiting_ack',
    active: true,
  };
  const committedUser = {
    id: 'origin:intent:test-follow-up',
    seq: 179,
    role: 'user',
    text: 'Test queued follow-up',
    content: 'Test queued follow-up',
    timestamp: '2026-06-24T06:33:03.000Z',
    metadata: {
      origin_id: 'intent:test-follow-up',
      queued_at: '2026-06-24T06:32:44.000Z',
      queued_input_id: 'queued_input:test-follow-up',
    },
    localState: 'remote_final',
  };

  const visible = visibleRemotePendingInputsForThread({
    activeMessages: [committedUser],
    visiblePendingAckIntentCount: 0,
    remotePendingInputs: [pendingInput],
  });

  assert.deepEqual(
    visible,
    [],
    'committed user row should suppress the stale remote pending bubble',
  );
});

test('remote queued input remains visible until represented by committed user metadata', () => {
  const pendingInput = {
    id: 'queued_input:test-unmatched',
    runId: 'run:test',
    text: 'Test queued follow-up',
    content: 'Test queued follow-up',
    timestamp: '2026-06-24T06:32:44.000Z',
    status: 'awaiting_ack',
    active: true,
  };

  const visible = visibleRemotePendingInputsForThread({
    activeMessages: [
      {
        id: 'origin:intent:other',
        seq: 180,
        role: 'user',
        text: 'Other message',
        metadata: {
          queued_input_id: 'queued_input:other',
        },
      },
    ],
    visiblePendingAckIntentCount: 0,
    remotePendingInputs: [pendingInput],
  });

  assert.deepEqual(visible, [pendingInput]);
});

test('visible local pending ack intents suppress remote pending input chrome', () => {
  const visible = visibleRemotePendingInputsForThread({
    activeMessages: [],
    visiblePendingAckIntentCount: 1,
    remotePendingInputs: [
      {
        id: 'queued_input:test-follow-up',
        runId: 'run:test',
        text: 'Test queued follow-up',
        status: 'awaiting_ack',
        active: true,
      },
    ],
  });

  assert.deepEqual(visible, []);
});

test('queued follow-up pending input dedupes after committed origin row materializes', () => {
  const pendingInput = {
    id: 'queued_input:test-follow-up',
    runId: 'run:test',
    text: 'Test queued follow-up',
    content: 'Test queued follow-up',
    timestamp: '2026-06-24T06:32:44.000Z',
    status: 'awaiting_ack',
    active: true,
  };
  const committedUser = {
    id: 'origin:intent:test-follow-up',
    seq: 249,
    role: 'user',
    text: 'Test queued follow-up',
    content: 'Test queued follow-up',
    timestamp: '2026-06-24T06:33:03.000Z',
    metadata: {
      origin_id: 'intent:test-follow-up',
    },
    localState: 'remote_final',
  };
  const pendingInputOriginRefs = [
    {
      pendingInputId: 'queued_input:test-follow-up',
      originId: 'intent:test-follow-up',
    },
  ];

  const beforeCommitRemote = visibleRemotePendingInputsForThread({
    activeMessages: [],
    visiblePendingAckIntentCount: 1,
    remotePendingInputs: [pendingInput],
    pendingInputOriginRefs,
  });
  assert.deepEqual(
    beforeCommitRemote,
    [],
    'local pending ack chrome owns the queued message before commit',
  );

  const afterCommitRemote = visibleRemotePendingInputsForThread({
    activeMessages: [committedUser],
    visiblePendingAckIntentCount: 0,
    remotePendingInputs: [pendingInput],
    pendingInputOriginRefs,
  });
  const renderedUserLikeRows = 1 + afterCommitRemote.length;

  assert.equal(
    renderedUserLikeRows,
    1,
    'committed origin row should suppress the stale remote pending bubble',
  );
});

test('pending input origin refs do not hide a different queued input', () => {
  const committedUser = {
    id: 'origin:intent:committed',
    seq: 249,
    role: 'user',
    text: 'Committed queued follow-up',
    metadata: {
      origin_id: 'intent:committed',
    },
    localState: 'remote_final',
  };
  const stillQueued = {
    id: 'queued_input:still-queued',
    runId: 'run:test',
    text: 'Still queued follow-up',
    status: 'awaiting_ack',
    active: true,
  };

  const visible = visibleRemotePendingInputsForThread({
    activeMessages: [committedUser],
    visiblePendingAckIntentCount: 0,
    remotePendingInputs: [stillQueued],
    pendingInputOriginRefs: [
      {
        pendingInputId: 'queued_input:committed',
        originId: 'intent:committed',
      },
      {
        pendingInputId: 'queued_input:still-queued',
        originId: 'intent:still-queued',
      },
    ],
  });

  assert.deepEqual(visible, [stillQueued]);
});
