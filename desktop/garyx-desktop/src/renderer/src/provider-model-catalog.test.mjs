import test from 'node:test';
import assert from 'node:assert/strict';

import { shouldRequestProviderModelCatalog } from './provider-model-catalog.ts';

function state(overrides = {}) {
  return {
    catalogs: {},
    requests: {},
    attempted: {},
    ...overrides,
  };
}

test('provider model catalog loads before a provider is attempted', () => {
  assert.equal(shouldRequestProviderModelCatalog(state(), 'claude_code'), true);
});

test('provider model catalog does not reload when a catalog is already present', () => {
  assert.equal(
    shouldRequestProviderModelCatalog(
      state({ catalogs: { claude_code: { models: [] } } }),
      'claude_code',
      { retry: true },
    ),
    false,
  );
});

test('provider model catalog does not duplicate an in-flight request', () => {
  assert.equal(
    shouldRequestProviderModelCatalog(
      state({ requests: { codex_app_server: Promise.resolve() } }),
      'codex_app_server',
      { retry: true },
    ),
    false,
  );
});

test('provider model catalog prefetch does not loop after a failed attempt', () => {
  assert.equal(
    shouldRequestProviderModelCatalog(
      state({ attempted: { antigravity: true } }),
      'antigravity',
    ),
    false,
  );
});

test('provider model catalog can retry when a configure dialog is opened', () => {
  assert.equal(
    shouldRequestProviderModelCatalog(
      state({ attempted: { traex: true } }),
      'traex',
      { retry: true },
    ),
    true,
  );
});
