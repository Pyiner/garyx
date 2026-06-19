import test from 'node:test';
import assert from 'node:assert/strict';

import {
  appendClientStreamLogEntry,
  buildClientStreamLogEntry,
  defaultSideToolsPanelWidth,
} from './diagnostics-helpers.ts';

function committedEvent(seq, kind = 'assistant_reply') {
  return {
    type: 'committed_message',
    threadId: 'thread-1',
    runId: 'run-1',
    seq,
    message: {
      id: `thread-1:${seq - 1}`,
      role: kind === 'control' ? 'system' : 'assistant',
      text: kind === 'control' ? '' : 'hello',
      kind,
    },
  };
}

test('builds committed-message client log entries', () => {
  const first = buildClientStreamLogEntry(
    committedEvent(1),
    'client-log-line-1',
  );

  assert.equal(first.eventType, 'committed_message');
  assert.equal(first.runId, 'run-1');
  assert.equal(first.summary, 'seq=1 · assistant_reply');
});

test('trims committed client logs after append', () => {
  const entries = [
    buildClientStreamLogEntry(
      committedEvent(1),
      'client-log-line-1',
    ),
    buildClientStreamLogEntry(
      committedEvent(2, 'control'),
      'client-log-line-2',
    ),
  ];
  const next = buildClientStreamLogEntry(
    committedEvent(3),
    'client-log-line-3',
  );

  const trimmed = appendClientStreamLogEntry(entries, next, 2);

  assert.equal(trimmed.length, 2);
  assert.equal(trimmed[0].key, 'client-log-line-2');
  assert.equal(trimmed[1].key, 'client-log-line-3');
});

test('defaults side tools to the measured wide layout ratio when space allows', () => {
  assert.equal(defaultSideToolsPanelWidth(1800), 1080);
});

test('clamps side tools width to keep the main message column usable', () => {
  assert.equal(defaultSideToolsPanelWidth(1235), 685);
  assert.equal(defaultSideToolsPanelWidth(900), 520);
});
