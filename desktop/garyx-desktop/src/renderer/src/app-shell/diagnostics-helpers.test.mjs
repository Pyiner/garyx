import test from 'node:test';
import assert from 'node:assert/strict';

import {
  buildThreadLogLines,
  clampSideToolsPanelWidth,
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

test('defaults side tools to the single measured Codex-style right rail', () => {
  assert.equal(defaultSideToolsPanelWidth(1800), 320);
  assert.equal(defaultSideToolsPanelWidth(1235), 320);
  assert.equal(defaultSideToolsPanelWidth(900), 320);
});

test('clamps a customized side-tools rail to the measured canvas', () => {
  assert.equal(clampSideToolsPanelWidth(520, 736), 320);
  assert.equal(clampSideToolsPanelWidth(685, 1235), 685);
});
