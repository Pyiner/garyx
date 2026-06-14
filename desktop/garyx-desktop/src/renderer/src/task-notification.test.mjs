import test from 'node:test';
import assert from 'node:assert/strict';

import { parseTaskNotificationText } from './task-notification.ts';

test('parses ready-for-review task notification envelope', () => {
  const parsed = parseTaskNotificationText(`
<garyx_task_notification event="ready_for_review" task_id="#TASK-42" status="in_review">
Task #TASK-42 is ready for review: Ship task notifications

Done.

View details:
garyx task get #TASK-42

Review next:
If changes are needed, move the task back to in progress and send feedback to the task thread:
garyx task update #TASK-42 --status in_progress --note "needs changes: summary"

If approved, mark it done:
garyx task update #TASK-42 --status done --note "approved by reviewer"
</garyx_task_notification>
`);

  assert.deepEqual(parsed, {
    event: 'ready_for_review',
    status: 'in_review',
    taskId: '#TASK-42',
    title: 'Ship task notifications',
    finalMessage: 'Done.',
    detailCommand: 'garyx task get #TASK-42',
    reviewCommands: [
      'garyx task update #TASK-42 --status in_progress --note "needs changes: summary"',
      'garyx task update #TASK-42 --status done --note "approved by reviewer"',
    ],
  });
});

test('keeps markdown-like final text without requiring strict XML content', () => {
  const parsed = parseTaskNotificationText(`
<garyx_task_notification event="ready_for_review" task_id="#TASK-7" status="in_review">
Task #TASK-7 is ready for review: Review renderer output

527 skill 审查 + 验证充分:

- <review> should not become a visible wrapper.
- command stayed safe & readable.

View details:
garyx task get #TASK-7

Review next:
garyx task update #TASK-7 --status done --note "approved by reviewer"
</garyx_task_notification>
`);

  assert.equal(parsed?.taskId, '#TASK-7');
  assert.equal(parsed?.title, 'Review renderer output');
  assert.match(parsed?.finalMessage || '', /527 skill/);
  assert.match(parsed?.finalMessage || '', /<review>/);
  assert.deepEqual(parsed?.reviewCommands, [
    'garyx task update #TASK-7 --status done --note "approved by reviewer"',
  ]);
});

test('ignores ordinary XML snippets', () => {
  assert.equal(parseTaskNotificationText('<review>done</review>'), null);
});
