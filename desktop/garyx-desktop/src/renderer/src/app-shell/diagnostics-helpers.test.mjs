import test from 'node:test';
import assert from 'node:assert/strict';

import {
  defaultSideToolsPanelWidth,
} from './diagnostics-helpers.ts';

test('defaults side tools to the measured wide layout ratio when space allows', () => {
  assert.equal(defaultSideToolsPanelWidth(1800), 1080);
});

test('clamps side tools width to keep the main message column usable', () => {
  assert.equal(defaultSideToolsPanelWidth(1235), 685);
  assert.equal(defaultSideToolsPanelWidth(900), 520);
});
