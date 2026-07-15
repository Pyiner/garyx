import test from 'node:test';
import assert from 'node:assert/strict';

import { RUN_LOADING_LABEL } from './loading-labels.ts';

test('uses the explicit run loading label', () => {
  assert.equal(RUN_LOADING_LABEL, 'Thinking');
});
