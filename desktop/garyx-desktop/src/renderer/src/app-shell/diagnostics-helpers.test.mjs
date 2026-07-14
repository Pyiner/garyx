import test from 'node:test';
import assert from 'node:assert/strict';

import {
  SIDE_TOOLS_PANEL_MAX_WIDTH,
  buildThreadLogLines,
  clampSideToolsPanelWidth,
  defaultSideToolsPanelWidth,
  sideToolsPanelMinWidth,
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

test('side-tools min width is the policy knob: legacy pinned, expand-v1 doubled', () => {
  assert.equal(sideToolsPanelMinWidth('legacy'), 320);
  assert.equal(sideToolsPanelMinWidth('expand-v1'), 640);
  assert.equal(SIDE_TOOLS_PANEL_MAX_WIDTH, 1180);
});

test('defaults side tools to the policy min width regardless of canvas', () => {
  assert.equal(defaultSideToolsPanelWidth('expand-v1', 1800), 640);
  assert.equal(defaultSideToolsPanelWidth('expand-v1', 1235), 640);
  assert.equal(defaultSideToolsPanelWidth('expand-v1', 900), 640);
  assert.equal(defaultSideToolsPanelWidth('legacy', 1800), 320);
  assert.equal(defaultSideToolsPanelWidth('legacy', 900), 320);
});

test('clamps a customized side-tools rail to the measured canvas', () => {
  assert.equal(clampSideToolsPanelWidth('expand-v1', 840, 736), 640);
  assert.equal(clampSideToolsPanelWidth('expand-v1', 685, 1235), 685);
  assert.equal(clampSideToolsPanelWidth('legacy', 520, 736), 320);
  assert.equal(clampSideToolsPanelWidth('legacy', 685, 1235), 685);
});

test('side-tools width helpers match their complete boundary table', () => {
  const cases = [
    ['expand tools invalid uses default', 'expand-v1', Number.NaN, null, 640],
    ['expand tools below min', 'expand-v1', 639, null, 640],
    ['expand tools rounds', 'expand-v1', 700.5, null, 701],
    ['expand tools above max', 'expand-v1', 1181, null, 1180],
    ['expand tools narrow canvas', 'expand-v1', 840, 736, 640],
    ['expand tools exact canvas budget', 'expand-v1', 700, 1235, 685],
    ['legacy tools invalid uses default', 'legacy', Number.NaN, null, 320],
    ['legacy tools below min', 'legacy', 319, null, 320],
    ['legacy tools rounds', 'legacy', 400.5, null, 401],
    ['legacy tools above max', 'legacy', 1181, null, 1180],
    ['legacy tools narrow canvas', 'legacy', 520, 736, 320],
    ['legacy tools exact canvas budget', 'legacy', 700, 1235, 685],
  ];
  for (const [label, policy, width, layoutWidth, expected] of cases) {
    assert.equal(
      clampSideToolsPanelWidth(policy, width, layoutWidth),
      expected,
      label,
    );
  }
});
