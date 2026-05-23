import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { buildTurnRows } from './turn-render.ts';

const parityFixture = JSON.parse(
  readFileSync(
    new URL('../../../../../test-fixtures/turn-render-parity.json', import.meta.url),
    'utf8',
  ),
);

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

function fixtureMessageBlock(message) {
  return {
    kind: 'message',
    key: message.id,
    entry: {
      kind: 'message',
      key: message.id,
      message: {
        id: message.id,
        role: message.role,
        text: message.text ?? '',
        pending: message.isStreaming === true,
        timestamp: message.timestamp,
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

function fixtureToolBlock(message) {
  return {
    kind: 'tool_group',
    key: message.id,
    defaultExpanded: false,
    entries: [
      {
        kind: 'tool',
        key: `${message.id}:tool`,
        toolUse: {
          id: `${message.id}:tool`,
          role: 'tool_use',
          text: '',
          content: { name: 'lookup' },
          timestamp: message.timestamp,
        },
      },
    ],
  };
}

function fixtureBlocks(messages) {
  return messages.map((message) =>
    message.role === 'tool' ? fixtureToolBlock(message) : fixtureMessageBlock(message),
  );
}

function normalizeActivityRow(row) {
  if (row.kind === 'flat') {
    return { kind: 'flat', key: row.key };
  }
  return {
    kind: 'turn',
    key: row.key,
    steps: row.steps.map((step) => step.key),
    final: row.finalBlock?.key ?? null,
    running: row.isRunning,
    startedAt: row.startedAt,
    finishedAt: row.finishedAt,
  };
}

function normalizeRows(rows) {
  return rows.map((row) => {
    if (row.kind === 'flat') {
      return { kind: 'flat', key: row.key };
    }
    if (row.kind === 'turn') {
      return normalizeActivityRow(row);
    }
    return {
      kind: 'user_turn',
      key: row.key,
      user: row.userBlock.key,
      activity: row.activityRows.map(normalizeActivityRow),
    };
  });
}

test('matches the shared mobile parity fixture', () => {
  for (const testCase of parityFixture.cases) {
    assert.deepEqual(
      normalizeRows(
        buildTurnRows(fixtureBlocks(testCase.messages), {
          deferTrailingFinalAssistant: testCase.isRunningThread,
        }),
      ),
      testCase.expected,
      testCase.name,
    );
  }
});

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

test('keeps active trailing text inside a stable turn until the run finishes', () => {
  const rows = buildTurnRows(
    [
      messageBlock('u0', 'user', 'question'),
      messageBlock('a0', 'assistant', 'intermediate answer'),
    ],
    { deferTrailingFinalAssistant: true },
  );

  assert.equal(rows.length, 1);
  assert.equal(rows[0].kind, 'user_turn');
  assert.equal(rows[0].activityRows[0].kind, 'turn');
  assert.equal(rows[0].activityRows[0].key, 'turn:a0');
  assert.deepEqual(
    rows[0].activityRows[0].steps.map((step) => step.key),
    ['a0'],
  );
  assert.equal(rows[0].activityRows[0].finalBlock, null);

  const rowsAfterTool = buildTurnRows(
    [
      messageBlock('u0', 'user', 'question'),
      messageBlock('a0', 'assistant', 'intermediate answer'),
      toolBlock('t0'),
    ],
    { deferTrailingFinalAssistant: true },
  );

  assert.equal(rowsAfterTool.length, 1);
  assert.equal(rowsAfterTool[0].kind, 'user_turn');
  assert.equal(rowsAfterTool[0].activityRows[0].kind, 'turn');
  assert.equal(rowsAfterTool[0].activityRows[0].key, 'turn:a0');
  assert.deepEqual(
    rowsAfterTool[0].activityRows[0].steps.map((step) => step.key),
    ['a0', 't0'],
  );
});
