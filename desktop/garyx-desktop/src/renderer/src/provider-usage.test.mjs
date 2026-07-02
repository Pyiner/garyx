import test from 'node:test';
import assert from 'node:assert/strict';

import {
  clampUsagePercent,
  formatUsageAge,
  formatUsageDuration,
  formatUsagePercent,
  usageLevelForRemainingPercent,
  usageProviderIdForModelProviderKey,
  usageResetText,
} from './provider-usage.ts';

test('model provider rows use explicit gateway usage provider ids', () => {
  assert.equal(usageProviderIdForModelProviderKey('claude_code'), 'claude_code');
  assert.equal(usageProviderIdForModelProviderKey('codex_app_server'), 'codex');
  assert.equal(usageProviderIdForModelProviderKey('antigravity'), 'antigravity');
});

test('unknown model providers are not inferred from provider type', () => {
  assert.equal(usageProviderIdForModelProviderKey('gemini_cli'), undefined);
});

test('usage severity follows the shared remaining-percent thresholds', () => {
  assert.equal(usageLevelForRemainingPercent(50), 'healthy');
  assert.equal(usageLevelForRemainingPercent(20), 'warning');
  assert.equal(usageLevelForRemainingPercent(19.99), 'critical');
  assert.equal(usageLevelForRemainingPercent(99, false), 'unavailable');
});

test('usage percent formatting clamps invalid and out-of-range values', () => {
  assert.equal(clampUsagePercent(112), 100);
  assert.equal(clampUsagePercent(-7), 0);
  assert.equal(formatUsagePercent(49.6), '50%');
  assert.equal(formatUsagePercent(Number.NaN), '0%');
});

test('usage duration keeps the two largest useful units', () => {
  assert.equal(formatUsageDuration(183_600), '2d 3h');
  assert.equal(formatUsageDuration(4_320), '1h 12m');
  assert.equal(formatUsageDuration(59), '<1m');
});

test('usage reset text prefers the shorter conservative reset estimate', () => {
  const now = Date.parse('2026-07-03T00:00:00.000Z');
  assert.equal(
    usageResetText('2026-07-03T03:00:00.000Z', 7_200, 'unknown', now),
    'resets in 2h',
  );
  assert.equal(
    usageResetText('2026-07-03T01:00:00.000Z', 7_200, 'unknown', now),
    'resets in 1h',
  );
  assert.equal(usageResetText(null, null, 'unknown', now), 'unknown');
});

test('usage freshness reports elapsed time from refreshed_at', () => {
  const now = Date.parse('2026-07-03T00:30:00.000Z');
  assert.equal(formatUsageAge('2026-07-03T00:00:00.000Z', now), 'updated 30m ago');
  assert.equal(formatUsageAge('not-a-date', now), null);
});
