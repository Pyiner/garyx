import test from 'node:test';
import assert from 'node:assert/strict';

import {
  RUN_LOADING_LABEL,
  isRunLoadingPlaceholderMessage,
  isRunLoadingPlaceholderText,
} from './loading-labels.ts';

test('recognizes run loading placeholder text variants', () => {
  assert.equal(isRunLoadingPlaceholderText(RUN_LOADING_LABEL), true);
  assert.equal(isRunLoadingPlaceholderText('Garyx is working through the run...'), true);
  assert.equal(isRunLoadingPlaceholderText('A real assistant response'), false);
});

test('only assistant messages can be run loading placeholders', () => {
  assert.equal(
    isRunLoadingPlaceholderMessage({
      role: 'assistant',
      text: 'Garyx is working through the run...',
    }),
    true,
  );
  assert.equal(
    isRunLoadingPlaceholderMessage({
      role: 'user',
      text: 'Garyx is working through the run...',
    }),
    false,
  );
});
