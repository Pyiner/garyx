import test from 'node:test';
import assert from 'node:assert/strict';

import {
  isTransientGatewayErrorMessage,
  summarizeRemoteStateErrors,
} from './gateway-errors.ts';

test('treats abort timeout messages as transient gateway errors', () => {
  assert.equal(
    isTransientGatewayErrorMessage('The operation was aborted due to timeout'),
    true,
  );
  assert.equal(isTransientGatewayErrorMessage('TimeoutError: timeout'), true);
});

test('does not toast transient remote state sync failures', () => {
  assert.equal(
    summarizeRemoteStateErrors([
      {
        source: 'threads',
        label: 'threads',
        message: 'The operation was aborted due to timeout',
      },
    ]),
    null,
  );
});

test('still summarizes non-transient remote state failures', () => {
  assert.deepEqual(
    summarizeRemoteStateErrors([
      {
        source: 'threads',
        label: 'threads',
        message: '500 internal server error',
      },
    ]),
    {
      key: 'threads:500 internal server error',
      message: 'Gateway sync incomplete: threads failed. 500 internal server error',
    },
  );
});
