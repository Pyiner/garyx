import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import {
  stripTaskNotificationEnvelope,
  taskNotificationOverflows,
} from './message-rich-content-core.ts';

test('structurally strips only the task notification envelope body', () => {
  const text = [
    '<garyx_task_notification event="ready_for_review" task_id="#TASK-42" status="in_review" title="Changed wording">',
    'Pure handoff body.',
    '',
    '- Markdown remains intact.',
    '</garyx_task_notification>',
    '',
    'View details: garyx task get #TASK-42',
    'Review next: tutorial stays outside.',
  ].join('\n');

  assert.equal(
    stripTaskNotificationEnvelope(text),
    'Pure handoff body.\n\n- Markdown remains intact.',
  );
});

test('uses the last close tag so legacy envelope bodies stay readable', () => {
  const text = [
    '<garyx_task_notification event="ready_for_review">',
    'Body with neutralized </garyx_task_notification > text.',
    'Legacy tutorial wording can be anything.',
    '</garyx_task_notification>',
  ].join('\n');

  assert.equal(
    stripTaskNotificationEnvelope(text),
    [
      'Body with neutralized </garyx_task_notification > text.',
      'Legacy tutorial wording can be anything.',
    ].join('\n'),
  );
  assert.equal(stripTaskNotificationEnvelope('<review>done</review>'), null);
});

test('overflow decision honors the injected epsilon at the boundary', () => {
  assert.equal(taskNotificationOverflows(200, 200, 0.5), false);
  assert.equal(taskNotificationOverflows(200.5, 200, 0.5), false);
  assert.equal(taskNotificationOverflows(200.5001, 200, 0.5), true);
});

test('task notification inherits the ordinary trailing user bubble width owner', () => {
  const turnCss = readFileSync(
    new URL('./styles/turn-summary.css', import.meta.url),
    'utf8',
  );
  const userRule = turnCss.match(/\.message-bubble\.user\s*\{([^}]*)\}/)?.[1];
  const taskRule = turnCss.match(
    /\.message-bubble\.task-notification-message\s*\{([^}]*)\}/,
  )?.[1];
  const taskFillRule = turnCss.match(
    /\.message-bubble\.user\.task-notification-message\s*\{([^}]*)\}/,
  )?.[1];

  assert.ok(userRule, 'ordinary user bubble must remain the width/alignment owner');
  assert.match(userRule, /max-width:\s*77%/);
  assert.match(userRule, /align-self:\s*flex-end/);
  assert.equal(taskRule, undefined, 'task rows must not override the shared owner');
  assert.match(taskFillRule ?? '', /width:\s*100%/);
  assert.doesNotMatch(taskFillRule ?? '', /max-width|align-self/);
  assert.doesNotMatch(turnCss, /task-notification-message[^{}]*\{[^}]*736px/s);
});
