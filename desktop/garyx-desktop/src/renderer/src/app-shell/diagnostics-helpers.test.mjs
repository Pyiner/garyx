import test from 'node:test';
import assert from 'node:assert/strict';

import {
  buildThreadLogLines,
  defaultSideToolsPanelWidth,
} from './diagnostics-helpers.ts';

test('parses the unified space-separated local thread-log stamp', () => {
  const [line] = buildThreadLogLines('2026-07-07 17:06:37.123 INFO [run] hello');
  assert.equal(line.timestamp, '2026-07-07 17:06:37');
  assert.equal(line.text, 'INFO [run] hello');
});

test('still parses legacy RFC3339 thread-log stamps without a timezone suffix', () => {
  const [line] = buildThreadLogLines('2026-07-07T09:06:37.123456+00:00 WARN [run] legacy');
  assert.match(line.timestamp, /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/);
  assert.equal(line.text, 'WARN [run] legacy');
});

test('defaults side tools to the measured wide layout ratio when space allows', () => {
  assert.equal(defaultSideToolsPanelWidth(1800), 1080);
});

test('clamps side tools width to keep the main message column usable', () => {
  assert.equal(defaultSideToolsPanelWidth(1235), 685);
  assert.equal(defaultSideToolsPanelWidth(900), 520);
});
