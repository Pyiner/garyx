import test from 'node:test';
import assert from 'node:assert/strict';

import { buildTurnRows } from './turn-render.ts';

function messageBlock(id, role, text) {
  return {
    kind: 'message',
    key: id,
    entry: {
      kind: 'message',
      key: id,
      message: {
        id,
        role,
        text,
      },
    },
  };
}

function toolBlock(id) {
  return {
    kind: 'tool_group',
    key: id,
    defaultExpanded: false,
    entries: [
      {
        kind: 'tool',
        key: `${id}:tool`,
        toolUse: {
          id: `${id}:tool`,
          role: 'tool_use',
          text: '',
          content: { name: 'lookup' },
        },
      },
    ],
  };
}

test('surfaces final assistant text for every completed user turn', () => {
  const rows = buildTurnRows([
    messageBlock('u0', 'user', 'first question'),
    toolBlock('t0'),
    messageBlock('a0', 'assistant', 'first answer'),
    messageBlock('u1', 'user', 'second question'),
    toolBlock('t1'),
    messageBlock('a1', 'assistant', 'second answer'),
  ]);

  assert.equal(rows.length, 2);
  assert.equal(rows[0].kind, 'user_turn');
  assert.equal(rows[0].userBlock.key, 'u0');
  assert.equal(rows[0].activityRows[0].kind, 'turn');
  assert.equal(rows[0].activityRows[0].steps.length, 1);
  assert.equal(rows[0].activityRows[0].finalBlock?.key, 'a0');
  assert.equal(rows[1].kind, 'user_turn');
  assert.equal(rows[1].userBlock.key, 'u1');
  assert.equal(rows[1].activityRows[0].kind, 'turn');
  assert.equal(rows[1].activityRows[0].steps.length, 1);
  assert.equal(rows[1].activityRows[0].finalBlock?.key, 'a1');
});

test('defers only the trailing active turn final text while keeping older answers visible', () => {
  const rows = buildTurnRows(
    [
      messageBlock('u0', 'user', 'first question'),
      toolBlock('t0'),
      messageBlock('a0', 'assistant', 'first answer'),
      messageBlock('u1', 'user', 'second question'),
      toolBlock('t1'),
      messageBlock('a1', 'assistant', 'second answer still in flight'),
    ],
    { deferTrailingFinalAssistant: true },
  );

  assert.equal(rows.length, 2);
  assert.equal(rows[0].kind, 'user_turn');
  assert.equal(rows[0].activityRows[0].kind, 'turn');
  assert.equal(rows[0].activityRows[0].steps.length, 1);
  assert.equal(rows[0].activityRows[0].finalBlock?.key, 'a0');
  assert.equal(rows[1].kind, 'user_turn');
  assert.equal(rows[1].activityRows[0].kind, 'turn');
  assert.equal(rows[1].activityRows[0].finalBlock, null);
  assert.deepEqual(
    rows[1].activityRows[0].steps.map((step) => step.key),
    ['t1', 'a1'],
  );
});

test('keeps pure text replies inside their user turn without a summary row', () => {
  const rows = buildTurnRows([
    messageBlock('u0', 'user', 'simple question'),
    messageBlock('a0', 'assistant', 'simple answer'),
  ]);

  assert.equal(rows.length, 1);
  assert.equal(rows[0].kind, 'user_turn');
  assert.equal(rows[0].userBlock.key, 'u0');
  assert.equal(rows[0].activityRows[0].kind, 'flat');
  assert.equal(rows[0].activityRows[0].block.key, 'a0');
});
