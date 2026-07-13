import test from 'node:test';
import assert from 'node:assert/strict';

import {
  SIDE_TOOLS_PANEL_MAX_WIDTH,
  SIDE_TOOLS_PANEL_MIN_WIDTH,
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

test('side-tools width helpers match their complete boundary table', () => {
  assert.equal(SIDE_TOOLS_PANEL_MIN_WIDTH, 320);
  assert.equal(SIDE_TOOLS_PANEL_MAX_WIDTH, 1180);

  const cases = [
    ['tools invalid uses default', clampSideToolsPanelWidth, Number.NaN, null, 320],
    ['tools below min', clampSideToolsPanelWidth, 319, null, 320],
    ['tools rounds', clampSideToolsPanelWidth, 400.5, null, 401],
    ['tools above max', clampSideToolsPanelWidth, 1181, null, 1180],
    ['tools narrow canvas', clampSideToolsPanelWidth, 520, 736, 320],
    ['tools exact canvas budget', clampSideToolsPanelWidth, 700, 1235, 685],
  ];
  for (const [label, clamp, width, layoutWidth, expected] of cases) {
    assert.equal(clamp(width, layoutWidth), expected, label);
  }
});
