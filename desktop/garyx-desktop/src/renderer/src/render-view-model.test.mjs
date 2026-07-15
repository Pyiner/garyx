import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import * as renderViewModel from './render-view-model.ts';
import { materializeRemoteTranscript } from './gateway-mirror/transcript-materialize.ts';

const {
  buildThreadViewRows,
  buildThreadViewRowsWithLocalUsers,
} = renderViewModel;

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
    if (row.kind !== 'user_turn') {
      continue;
    }
    pushRef(row.user);
    for (const activity of row.activity) {
      if (activity.kind === 'assistant_reply') {
        pushRef(activity.message);
        continue;
      }
      if (activity.kind !== 'step') {
        continue;
      }
      pushRef(activity.final_message);
      for (const item of activity.steps) {
        if (item.kind === 'assistant_message') {
          pushRef(item.message);
          continue;
        }
        if (item.kind !== 'tool_group') {
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

function blocksForRows(rows) {
  const blocks = [];
  const appendActivity = (row) => {
    if (row.kind === 'flat') {
      blocks.push(row.block);
      return;
    }
    blocks.push(...row.steps);
    if (row.finalBlock) blocks.push(row.finalBlock);
  };
  for (const row of rows) {
    if (row.kind === 'flat' || row.kind === 'turn') {
      appendActivity(row);
      continue;
    }
    if (row.kind === 'user_turn') {
      blocks.push(row.userBlock);
      row.activityRows.forEach(appendActivity);
    }
  }
  return blocks;
}

test('every fixture render_state maps to exactly its visible messages', () => {
  for (const fixtureCase of renderFixture.cases) {
    const renderState = fixtureCase.expected;
    const messages = messagesBySeqFor(renderState);
    const blocks = blocksForRows(buildThreadViewRows(renderState, messages));
    const ids = blockMessageIds(blocks);
    // No drops, no duplicates: flattened blocks resolve exactly the message
    // refs the row tree references.
    assert.deepEqual(
      [...ids].sort(),
      collectRefs(renderState)
        .map((ref) => ref.id)
        .sort(),
      `${fixtureCase.name}: block message ids must equal the row-tree refs`,
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

test('tool entries pass the server field projection through unchanged', () => {
  const fixtureCase = renderFixture.cases.find(
    (c) => c.name === 'tool lull after completed tool',
  );
  const renderState = fixtureCase.expected;
  const rows = buildThreadViewRows(renderState, messagesBySeqFor(renderState));
  const userTurn = rows.find((row) => row.kind === 'user_turn');
  const turn = userTurn.activityRows.find((row) => row.kind === 'turn');
  const group = turn.steps.find((block) => block.kind === 'tool_group');
  const wireEntry = renderState.rows[0].activity[0].steps.find(
    (step) => step.kind === 'tool_group',
  ).entries[0];

  assert.deepEqual(group.entries[0].projection, wireEntry.projection);
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

test('server-visible assistant text is not inferred to be a loading placeholder', () => {
  const assistant = {
    id: 'assistant:legacy-loading-copy',
    seq: 1,
    role: 'assistant',
    text: 'Garyx is working through the run…',
    kind: 'assistant_reply',
  };
  const renderState = {
    based_on_seq: 1,
    rows: [
      {
        kind: 'user_turn',
        id: 'orphan-visible-assistant',
        user: null,
        activity: [
          {
            kind: 'assistant_reply',
            message: { id: assistant.id, seq: assistant.seq, role: assistant.role },
          },
        ],
        capsule_cards: [],
        started_at: null,
        finished_at: null,
      },
    ],
    tailActivity: 'none',
    activeToolGroupId: null,
    progress_locus: 'none',
    filtered_placeholders: [],
  };
  const materialized = materializeRemoteTranscript([assistant], []);
  const messages = new Map(materialized.map((message) => [message.seq, message]));

  const rows = buildThreadViewRows(renderState, messages);

  assert.equal(rows.length, 1);
  assert.equal(rows[0].kind, 'flat');
  assert.equal(rows[0].block.entry.message.text, assistant.text);
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

test('local optimistic user row renders before committed render_state includes it', () => {
  const localUser = {
    id: 'origin:intent-optimistic-1',
    role: 'user',
    text: 'Please run the desktop smoke test.',
    timestamp: '2026-06-23T00:00:00.000Z',
    intentId: 'intent-optimistic-1',
    localState: 'optimistic',
  };
  const renderState = {
    based_on_seq: 0,
    rows: [],
    tailActivity: 'none',
    activeToolGroupId: null,
    progress_locus: 'none',
    filtered_placeholders: [],
  };

  const rows = buildThreadViewRowsWithLocalUsers(renderState, new Map(), [
    localUser,
  ]);

  assert.equal(rows.length, 1);
  assert.equal(rows[0].kind, 'user_turn');
  assert.equal(rows[0].key, `user-turn:${localUser.id}`);
  assert.equal(rows[0].userBlock.entry.message.text, localUser.text);
});

test('local optimistic user row dedupes once render_state represents its origin id', () => {
  const originId = 'origin:intent-optimistic-2';
  const renderState = {
    based_on_seq: 3,
    rows: [
      {
        kind: 'user_turn',
        id: `user_turn:${originId}`,
        user: { id: originId, seq: 3, role: 'user' },
        activity: [],
        started_at: null,
        finished_at: null,
      },
    ],
    tailActivity: 'none',
    activeToolGroupId: null,
    progress_locus: 'none',
    filtered_placeholders: [],
  };
  const committed = {
    id: originId,
    seq: 3,
    role: 'user',
    text: 'Run the smoke test.',
    localState: 'remote_final',
  };
  const optimistic = {
    ...committed,
    seq: undefined,
    localState: 'optimistic',
    intentId: 'intent-optimistic-2',
  };

  const rows = buildThreadViewRowsWithLocalUsers(
    renderState,
    new Map([[3, committed]]),
    [optimistic],
  );

  assert.equal(rows.length, 1);
  assert.equal(rows[0].kind, 'user_turn');
  assert.equal(rows[0].userBlock.entry.message.text, committed.text);
});

test('local failed user row remains visible for retry chrome', () => {
  const failedUser = {
    id: 'origin:intent-failed-1',
    role: 'user',
    text: 'Deploy the staging build.',
    timestamp: '2026-06-23T00:00:00.000Z',
    intentId: 'intent-failed-1',
    localState: 'error',
    error: true,
  };

  const rows = buildThreadViewRowsWithLocalUsers(null, new Map(), [failedUser]);

  assert.equal(rows.length, 1);
  assert.equal(rows[0].kind, 'user_turn');
  assert.equal(rows[0].userBlock.entry.message.error, true);
  assert.equal(rows[0].userBlock.entry.message.localState, 'error');
});

test('local assistant error row does not render through the user overlay', () => {
  const optimisticUser = {
    id: 'origin:intent-failed-2',
    role: 'user',
    text: 'Deploy the staging build.',
    timestamp: '2026-06-23T00:00:00.000Z',
    intentId: 'intent-failed-2',
    localState: 'optimistic',
  };
  const assistantError = {
    id: 'assistant:error:intent-failed-2:synthetic',
    role: 'assistant',
    text: 'Gateway rejected the request.',
    timestamp: '2026-06-23T00:00:01.000Z',
    intentId: 'intent-failed-2',
    localState: 'error',
    error: true,
  };

  const rows = buildThreadViewRowsWithLocalUsers(null, new Map(), [
    optimisticUser,
    assistantError,
  ]);

  assert.equal(rows.length, 1);
  assert.equal(rows[0].kind, 'user_turn');
  assert.equal(rows[0].userBlock.entry.message.id, optimisticUser.id);
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
});

const capsuleCardFixture = {
  id: 'capsule_card:01900000-0000-7000-8000-000000000903',
  capsule_id: '01900000-0000-7000-8000-000000000903',
  title: 'Desktop Contract Capsule',
  revision: 2,
  action: 'updated',
};

test('capsule_cards pass through the turn without entering visible ids/blocks', () => {
  const renderState = {
    based_on_seq: 2,
    rows: [
      {
        kind: 'user_turn',
        id: 'user_turn:seq:1',
        user: { id: 'seq:1', seq: 1, role: 'user' },
        activity: [
          {
            kind: 'assistant_reply',
            id: 'assistant_reply:seq:2',
            message: { id: 'seq:2', seq: 2, role: 'assistant' },
            streaming: false,
          },
        ],
        started_at: null,
        finished_at: null,
        capsule_cards: [capsuleCardFixture],
      },
    ],
    tailActivity: 'none',
    activeToolGroupId: null,
    progress_locus: 'none',
    filtered_placeholders: [],
  };
  const messages = messagesBySeqFor(renderState);

  const rows = buildThreadViewRows(renderState, messages);

  assert.equal(rows.length, 1);
  assert.equal(rows[0].kind, 'user_turn');
  assert.equal(rows[0].activityRows.length, 1);
  // The cards pass through on the turn...
  assert.deepEqual(rows[0].capsuleCards, [capsuleCardFixture]);
  // ...but are never transcript messages.
  assert.deepEqual(blockMessageIds(blocksForRows(rows)), ['seq:1', 'seq:2']);
});

test('orphan turn with capsule_cards surfaces a top-level capsule_only row', () => {
  const renderState = {
    based_on_seq: 3,
    rows: [
      {
        kind: 'user_turn',
        id: 'user_turn:seq:1',
        user: { id: 'seq:1', seq: 1, role: 'user' },
        activity: [
          {
            kind: 'assistant_reply',
            id: 'assistant_reply:seq:2',
            message: { id: 'seq:2', seq: 2, role: 'assistant' },
            streaming: false,
          },
        ],
        started_at: null,
        finished_at: null,
        capsule_cards: [capsuleCardFixture],
      },
    ],
    tailActivity: 'none',
    activeToolGroupId: null,
    progress_locus: 'none',
    filtered_placeholders: [],
  };
  // Drop the user body (seq 1): the turn becomes an orphan, but its cards must
  // still surface via a dedicated capsule_only row (no fake user bubble).
  const messages = messagesBySeqFor(renderState);
  messages.delete(1);

  const rows = buildThreadViewRows(renderState, messages);
  assert.ok(rows.every((row) => row.kind !== 'user_turn'));
  const capsuleOnly = rows.find((row) => row.kind === 'capsule_only');
  assert.ok(capsuleOnly, 'orphan cards should produce a capsule_only row');
  assert.deepEqual(capsuleOnly.capsuleCards, [capsuleCardFixture]);
});

test('unknown render activity and step item kinds are skipped', () => {
  const renderState = {
    based_on_seq: 4,
    rows: [
      {
        kind: 'future_top_level_row',
        id: 'future:top-level',
      },
      {
        kind: 'user_turn',
        id: 'user_turn:seq:1',
        user: { id: 'seq:1', seq: 1, role: 'user' },
        activity: [
          {
            kind: 'future_activity',
            id: 'future:activity',
            message: { id: 'seq:999', seq: 999, role: 'assistant' },
          },
          {
            kind: 'step',
            id: 'step:seq:2',
            steps: [
              {
                kind: 'future_step_item',
                id: 'future:step',
                message: { id: 'seq:998', seq: 998, role: 'assistant' },
              },
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
    ],
    tailActivity: 'none',
    activeToolGroupId: null,
    progress_locus: 'none',
    filtered_placeholders: [],
  };
  const messages = messagesBySeqFor(renderState);

  const rows = buildThreadViewRows(renderState, messages);
  const blocks = blocksForRows(rows);

  assert.equal(rows.length, 1);
  assert.equal(rows[0].kind, 'user_turn');
  const turn = rows[0].activityRows.find((row) => row.kind === 'turn');
  assert.ok(turn);
  assert.ok(turn.finalBlock);
  assert.deepEqual(
    [...turn.steps.map((block) => block.key), turn.finalBlock.key],
    ['seq:2', 'seq:3'],
  );
  assert.deepEqual(blockMessageIds(blocks), ['seq:1', 'seq:2', 'seq:3']);
});
