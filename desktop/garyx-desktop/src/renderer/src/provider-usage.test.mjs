import test from 'node:test';
import assert from 'node:assert/strict';

import { usageProviderIdForModelProviderKey } from './provider-usage.ts';

test('model provider rows use explicit gateway usage provider ids', () => {
  assert.equal(usageProviderIdForModelProviderKey('claude_code'), 'claude_code');
  assert.equal(usageProviderIdForModelProviderKey('codex_app_server'), 'codex');
  assert.equal(usageProviderIdForModelProviderKey('antigravity'), 'antigravity');
});

test('unknown model providers are not inferred from provider type', () => {
  assert.equal(usageProviderIdForModelProviderKey('gemini_cli'), undefined);
});
