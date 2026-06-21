import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import {
  buildThreadViewBlocks,
  buildThreadViewRows,
} from './render-view-model.ts';

const renderFixture = JSON.parse(
  readFileSync(
    new URL(
      '../../../../../test-fixtures/render-layer/render-state-cases.json',
      import.meta.url,
    ),
    'utf8',
  ),
);

// Collect every message ref a render_state references, so we can synthesize a
// fully-loaded committed window keyed by the raw record seq, mirroring the real
// desktop resolution path.
function collectRefs(renderState) {
  const refs = [];
  const pushRef = (ref) => {
    if (ref) refs.push(ref);
  };
  for (const row of renderState.rows) {
    pushRef(row.user);
    for (const activity of row.activity) {
      if (activity.kind === 'assistant_reply') {
        pushRef(activity.message);
        continue;
      }
      pushRef(activity.final_message);
      for (const item of activity.steps) {
        if (item.kind === 'assistant_message') {
          pushRef(item.message);
          continue;
        }
        for (const entry of item.entries) {
          pushRef(entry.tool_use);
          pushRef(entry.tool_result);
        }
      }
    }
  }
  return refs;
}

function messagesBySeqFor(renderState, { metadataBySeq = {} } = {}) {
  const map = new Map();
  for (const ref of collectRefs(renderState)) {
    map.set(ref.seq, {
      id: ref.id,
      seq: ref.seq,
      role: ref.role,
      text: `body-${ref.seq}`,
      metadata: metadataBySeq[ref.seq] ?? null,
    });
  }
  return map;
}

function blockMessageIds(blocks) {
  const ids = [];
  for (const block of blocks) {
    if (block.kind === 'message') {
      ids.push(block.entry.message.id);
      continue;
    }
    for (const entry of block.entries) {
      if (entry.toolUse) ids.push(entry.toolUse.id);
      if (entry.toolResult) ids.push(entry.toolResult.id);
    }
  }
  return ids;
}

test('every fixture render_state maps to exactly its visible messages', () => {
  for (const fixtureCase of renderFixture.cases) {
    const renderState = fixtureCase.expected;
    const messages = messagesBySeqFor(renderState);
    const blocks = buildThreadViewBlocks(renderState, messages);
    const ids = blockMessageIds(blocks);
    // No drops, no duplicates: flattened blocks resolve exactly the visible set.
    assert.deepEqual(
      [...ids].sort(),
      [...renderState.visibleMessageIds].sort(),
      `${fixtureCase.name}: block message ids must equal visibleMessageIds`,
    );
    // Every resolved block carries a real body (no unresolved placeholder).
    for (const block of blocks) {
      if (block.kind === 'message') {
        assert.ok(block.entry.message, `${fixtureCase.name}: message body`);
      } else {
        assert.ok(block.entries.length, `${fixtureCase.name}: non-empty group`);
      }
    }
  }
});

test('user_turn rows resolve to a user_turn; orphan rows surface at top level', () => {
  for (const fixtureCase of renderFixture.cases) {
    const renderState = fixtureCase.expected;
    const messages = messagesBySeqFor(renderState);
    const rows = buildThreadViewRows(renderState, messages);
    const hasUserRow = renderState.rows.some((row) => row.user);
    const hasOrphanRow = renderState.rows.some(
      (row) => !row.user && row.activity.length,
    );
    if (hasUserRow) {
      assert.ok(
        rows.some((row) => row.kind === 'user_turn'),
        `${fixtureCase.name}: expected a user_turn row`,
      );
    }
    if (hasOrphanRow && !hasUserRow) {
      assert.ok(
        rows.every((row) => row.kind !== 'user_turn'),
        `${fixtureCase.name}: orphan rows must not wrap in user_turn`,
      );
    }
  }
});

test('tool_group active flag mirrors activeToolGroupId', () => {
  const fixtureCase = renderFixture.cases.find((c) => c.name === 'tool active');
  const renderState = fixtureCase.expected;
  const rows = buildThreadViewRows(renderState, messagesBySeqFor(renderState));
  const userTurn = rows.find((row) => row.kind === 'user_turn');
  assert.ok(userTurn, 'tool active should produce a user_turn');
  const turn = userTurn.activityRows.find((row) => row.kind === 'turn');
  assert.ok(turn, 'tool active should produce a turn row');
  assert.equal(turn.isRunning, true);
  const group = turn.steps.find((block) => block.kind === 'tool_group');
  assert.ok(group, 'tool active should contain a tool_group block');
  assert.equal(
    group.key === renderState.activeToolGroupId,
    true,
    'tool_group key must equal activeToolGroupId so the shimmer activates',
  );
});

test('final answer surfaces a finalBlock outside the collapsible', () => {
  const fixtureCase = renderFixture.cases.find(
    (c) => c.name === 'final answer completed',
  );
  const renderState = fixtureCase.expected;
  const rows = buildThreadViewRows(renderState, messagesBySeqFor(renderState));
  const userTurn = rows.find((row) => row.kind === 'user_turn');
  assert.ok(userTurn, 'expected user_turn');
  const turn = userTurn.activityRows.find((row) => row.kind === 'turn');
  assert.ok(turn, 'expected a turn activity');
  assert.equal(turn.isRunning, false);
  assert.ok(turn.finalBlock, 'completed turn surfaces the final answer block');
  assert.equal(turn.finalBlock.entry.message.role, 'assistant');
});

test('assistant_reply maps to a flat row (no Worked-for wrapper)', () => {
  const fixtureCase = renderFixture.cases.find(
    (c) => c.name === 'assistant streaming text',
  );
  const renderState = fixtureCase.expected;
  const rows = buildThreadViewRows(renderState, messagesBySeqFor(renderState));
  const userTurn = rows.find((row) => row.kind === 'user_turn');
  assert.ok(userTurn, 'expected user_turn');
  assert.ok(
    userTurn.activityRows.some((row) => row.kind === 'flat'),
    'pure assistant reply should be a flat row',
  );
});

test('render_state ref seq resolves the body keyed by that same seq', () => {
  const renderState = {
    based_on_seq: 2,
    rows: [
      {
        kind: 'user_turn',
        id: 'user_turn:seq:2',
        user: { id: 'stable-intent-id', seq: 2, role: 'user' },
        activity: [],
        started_at: null,
        finished_at: null,
      },
    ],
    tailActivity: 'none',
    activeToolGroupId: null,
    progress_locus: 'none',
    visibleMessageIds: ['seq:2'],
    filtered_placeholders: [],
  };
  // Body stored at seq 2 with a STABLE (non-seq-encoding) id, as optimistic
  // reconciliation produces. Resolution is by seq, not by parsing the id.
  const messages = new Map([
    [2, { id: 'stable-intent-id', seq: 2, role: 'user', text: 'hi' }],
  ]);
  const rows = buildThreadViewRows(renderState, messages);
  assert.equal(rows.length, 1);
  assert.equal(rows[0].kind, 'user_turn');
  assert.equal(rows[0].userBlock.entry.message.text, 'hi');
  // A map keyed at the index (seq − 1) instead of the raw seq must NOT resolve.
  const wrongKeyed = new Map([
    [1, { id: 'stable-intent-id', seq: 2, role: 'user', text: 'hi' }],
  ]);
  assert.equal(buildThreadViewRows(renderState, wrongKeyed).length, 0);
});

test('origin user ids remain stable while the body resolves by seq', () => {
  const originId = 'origin:00000000-0000-0000-0000-000000000001';
  const renderState = {
    based_on_seq: 2,
    rows: [
      {
        kind: 'user_turn',
        id: `user_turn:${originId}`,
        user: { id: originId, seq: 2, role: 'user' },
        activity: [],
        started_at: null,
        finished_at: null,
      },
    ],
    tailActivity: 'none',
    activeToolGroupId: null,
    progress_locus: 'none',
    visibleMessageIds: [originId],
    filtered_placeholders: [],
  };
  const messages = new Map([
    [2, { id: originId, seq: 2, role: 'user', text: 'hello' }],
  ]);

  const rows = buildThreadViewRows(renderState, messages);

  assert.equal(rows.length, 1);
  assert.equal(rows[0].kind, 'user_turn');
  assert.equal(rows[0].key, `user-turn:${originId}`);
  assert.equal(rows[0].userBlock.key, originId);
  assert.equal(rows[0].userBlock.entry.message.text, 'hello');
});

test('unloaded committed window: rows whose bodies are absent are skipped', () => {
  const fixtureCase = renderFixture.cases.find(
    (c) => c.name === 'final answer completed',
  );
  const renderState = fixtureCase.expected;
  const full = messagesBySeqFor(renderState);
  // Drop the user body (seq 2): the turn loses its user but keeps resolvable
  // activity, so it degrades to a top-level orphan, never a torn row.
  const partial = new Map(full);
  partial.delete(2);
  const rows = buildThreadViewRows(renderState, partial);
  assert.ok(
    rows.every((row) => row.kind !== 'user_turn'),
    'missing user body must not render a user_turn',
  );
  // Dropping everything yields no rows at all (no empty shells).
  assert.equal(buildThreadViewRows(renderState, new Map()).length, 0);
  assert.equal(buildThreadViewBlocks(renderState, new Map()).length, 0);
});

test('team flatten preserves block order and per-message metadata', () => {
  // Two user turns, each a step whose assistant carries a distinct agent_id, so
  // the team speaker headers (resolved from metadata) can group correctly.
  const renderState = {
    based_on_seq: 6,
    rows: [
      {
        kind: 'user_turn',
        id: 'user_turn:seq:1',
        user: { id: 'seq:1', seq: 1, role: 'user' },
        activity: [
          {
            kind: 'step',
            id: 'step:assistant_step:seq:2',
            steps: [
              {
                kind: 'assistant_message',
                id: 'assistant_step:seq:2',
                message: { id: 'seq:2', seq: 2, role: 'assistant' },
                streaming: false,
              },
            ],
            final_message: { id: 'seq:3', seq: 3, role: 'assistant' },
            running: false,
            started_at: null,
            finished_at: null,
          },
        ],
        started_at: null,
        finished_at: null,
      },
      {
        kind: 'user_turn',
        id: 'user_turn:seq:4',
        user: { id: 'seq:4', seq: 4, role: 'user' },
        activity: [
          {
            kind: 'assistant_reply',
            id: 'assistant_reply:seq:5',
            message: { id: 'seq:5', seq: 5, role: 'assistant' },
            streaming: false,
          },
        ],
        started_at: null,
        finished_at: null,
      },
    ],
    tailActivity: 'none',
    activeToolGroupId: null,
    progress_locus: 'none',
    visibleMessageIds: ['seq:1', 'seq:2', 'seq:3', 'seq:4', 'seq:5'],
    filtered_placeholders: [],
  };
  const messages = messagesBySeqFor(renderState, {
    metadataBySeq: {
      2: { agent_id: 'alpha' },
      3: { agent_id: 'alpha' },
      5: { agent_id: 'beta' },
    },
  });
  const blocks = buildThreadViewBlocks(renderState, messages);
  // Deterministic flatten: user, intermediate assistant, final assistant, user,
  // reply — in transcript order.
  assert.deepEqual(blockMessageIds(blocks), [
    'seq:1',
    'seq:2',
    'seq:3',
    'seq:4',
    'seq:5',
  ]);
  const agentIds = blocks.map(
    (block) => block.entry.message.metadata?.agent_id ?? null,
  );
  assert.deepEqual(agentIds, [null, 'alpha', 'alpha', null, 'beta']);
});
