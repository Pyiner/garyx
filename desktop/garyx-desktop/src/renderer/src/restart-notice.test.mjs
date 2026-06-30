import test from 'node:test';
import assert from 'node:assert/strict';

import { parseRestartNoticeText } from './restart-notice.ts';

test('parses a garyx_restarted notice envelope', () => {
  const parsed = parseRestartNoticeText(
    '<garyx_restarted>Garyx has restarted. Continue your task.</garyx_restarted>',
  );
  assert.deepEqual(parsed, {
    message: 'Garyx has restarted. Continue your task.',
  });
});

test('tolerates surrounding whitespace and attributes', () => {
  const parsed = parseRestartNoticeText(`
<garyx_restarted reason="manual">
Back online — pick up where you left off.
</garyx_restarted>
`);
  assert.deepEqual(parsed, {
    message: 'Back online — pick up where you left off.',
  });
});

test('falls back to a default message for an empty body', () => {
  const parsed = parseRestartNoticeText('<garyx_restarted></garyx_restarted>');
  assert.deepEqual(parsed, {
    message: 'Garyx has restarted. Continue your task.',
  });
});

test('returns null for non-restart text', () => {
  assert.equal(parseRestartNoticeText('just a normal message'), null);
  assert.equal(
    parseRestartNoticeText('<garyx_task_notification></garyx_task_notification>'),
    null,
  );
});
