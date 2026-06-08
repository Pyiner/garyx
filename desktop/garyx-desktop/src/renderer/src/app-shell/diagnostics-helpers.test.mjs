import test from 'node:test';
import assert from 'node:assert/strict';

import {
  appendClientStreamLogEntry,
  buildClientStreamLogEntry,
  defaultSideToolsPanelWidth,
} from './diagnostics-helpers.ts';

function assistantDeltaEvent(runId, delta) {
  return {
    type: 'assistant_delta',
    threadId: 'thread-1',
    runId,
    delta,
  };
}

test('coalesces consecutive assistant delta client logs for the same run', () => {
  const first = buildClientStreamLogEntry(
    assistantDeltaEvent('run-1', 'hello'),
    'client-log-line-1',
  );
  const second = buildClientStreamLogEntry(
    assistantDeltaEvent('run-1', ' world'),
    'client-log-line-2',
  );

  const entries = appendClientStreamLogEntry(
    appendClientStreamLogEntry([], first),
    second,
  );

  assert.equal(entries.length, 1);
  assert.equal(entries[0].key, 'client-log-line-1');
  assert.equal(entries[0].count, 2);
  assert.equal(entries[0].totalChars, 11);
  assert.equal(entries[0].summary, '2 chunks · 11 chars');
});

test('keeps assistant delta client logs separate across runs', () => {
  const first = buildClientStreamLogEntry(
    assistantDeltaEvent('run-1', 'hello'),
    'client-log-line-1',
  );
  const second = buildClientStreamLogEntry(
    assistantDeltaEvent('run-2', 'world'),
    'client-log-line-2',
  );

  const entries = appendClientStreamLogEntry(
    appendClientStreamLogEntry([], first),
    second,
  );

  assert.equal(entries.length, 2);
  assert.equal(entries[0].runId, 'run-1');
  assert.equal(entries[1].runId, 'run-2');
});

test('trims client logs after append or coalesce', () => {
  const entries = [
    buildClientStreamLogEntry(
      { type: 'accepted', threadId: 'thread-1', runId: 'run-1' },
      'client-log-line-1',
    ),
    buildClientStreamLogEntry(
      { type: 'accepted', threadId: 'thread-1', runId: 'run-2' },
      'client-log-line-2',
    ),
  ];
  const next = buildClientStreamLogEntry(
    assistantDeltaEvent('run-3', 'x'),
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
