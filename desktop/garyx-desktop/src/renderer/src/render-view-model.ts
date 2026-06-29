import type {
  RenderActivityRow,
  RenderMessageRef,
  RenderState,
  RenderStepRow,
  RenderToolEntry,
  RenderToolGroup,
  TranscriptMessage,
} from '@shared/contracts';

/**
 * Block 4 dumb-render mapping. The server (`garyx-models`
 * `transcript_render_state.rs`) already computes the full semantic structure —
 * tool grouping, turn splitting, final-answer surfacing, empty-placeholder
 * filtering, running/active state. This module performs ONLY a pure structural
 * translation of `render_state.rows` into the presentation view-model the
 * existing React components consume, resolving each `seq` ref against the
 * locally-cached committed messages. No folding/grouping/pairing logic lives
 * here.
 */

// View-model types consumed by the presentation layer (TurnSummary,
// ToolTraceGroup, ThreadPage) while the semantic structure remains server-owned.
export type RenderTranscriptEntry =
  | {
      kind: 'message';
      key: string;
      message: TranscriptMessage;
    }
  | {
      kind: 'tool';
      key: string;
      toolUse?: TranscriptMessage;
      toolResult?: TranscriptMessage;
    };

export type RenderTranscriptBlock =
  | {
      kind: 'message';
      key: string;
      entry: Extract<RenderTranscriptEntry, { kind: 'message' }>;
    }
  | {
      kind: 'tool_group';
      key: string;
      defaultExpanded: boolean;
      entries: Array<Extract<RenderTranscriptEntry, { kind: 'tool' }>>;
    };

export interface TurnRow {
  kind: 'turn';
  key: string;
  /** Blocks rendered inside the collapsible body (oldest → newest). */
  steps: RenderTranscriptBlock[];
  /** Final assistant message rendered outside the collapsible. */
  finalBlock: RenderTranscriptBlock | null;
  /** True while the server reports this turn's run is still in flight. */
  isRunning: boolean;
  /** Earliest timestamp on the turn (ISO string). */
  startedAt: string | null;
  /** Latest non-pending timestamp in the turn (ISO string). */
  finishedAt: string | null;
}

export interface FlatRow {
  kind: 'flat';
  key: string;
  block: RenderTranscriptBlock;
}

export type UserTurnActivityRow = FlatRow | TurnRow;

export interface UserTurnRow {
  kind: 'user_turn';
  key: string;
  userBlock: RenderTranscriptBlock;
  activityRows: UserTurnActivityRow[];
}

export type TurnRenderRow = FlatRow | TurnRow | UserTurnRow;

type LocalTranscriptMessage = TranscriptMessage & {
  localState?: string;
};

/**
 * Committed messages keyed by their raw transcript record `seq` (1-based),
 * stamped at the wire boundary (`TranscriptMessage.seq`). The message id is NOT
 * used for resolution: optimistic reconciliation rewrites ids to stable values
 * that no longer encode the seq. `render_state` refs carry the same raw seq, so
 * resolution is `messagesBySeq.get(ref.seq)`.
 */
export type MessagesBySeq = Map<number, TranscriptMessage>;

function lookup(
  messages: MessagesBySeq,
  ref: RenderMessageRef | null | undefined,
): TranscriptMessage | null {
  if (!ref) {
    return null;
  }
  return messages.get(ref.seq) ?? null;
}

function messageBlock(message: TranscriptMessage): RenderTranscriptBlock {
  return {
    kind: 'message',
    key: message.id,
    entry: { kind: 'message', key: message.id, message },
  };
}

function collectBlockMessageIds(
  block: RenderTranscriptBlock | null | undefined,
  ids: Set<string>,
) {
  if (!block) {
    return;
  }
  if (block.kind === 'message') {
    ids.add(block.entry.message.id);
    return;
  }
  for (const entry of block.entries) {
    if (entry.toolUse) {
      ids.add(entry.toolUse.id);
    }
    if (entry.toolResult) {
      ids.add(entry.toolResult.id);
    }
  }
}

function collectActivityRowMessageIds(
  row: UserTurnActivityRow,
  ids: Set<string>,
) {
  if (row.kind === 'flat') {
    collectBlockMessageIds(row.block, ids);
    return;
  }
  for (const block of row.steps) {
    collectBlockMessageIds(block, ids);
  }
  collectBlockMessageIds(row.finalBlock, ids);
}

function representedMessageIdsForRows(rows: TurnRenderRow[]): Set<string> {
  const ids = new Set<string>();
  for (const row of rows) {
    if (row.kind === 'flat') {
      collectBlockMessageIds(row.block, ids);
      continue;
    }
    if (row.kind === 'turn') {
      collectActivityRowMessageIds(row, ids);
      continue;
    }
    collectBlockMessageIds(row.userBlock, ids);
    for (const activityRow of row.activityRows) {
      collectActivityRowMessageIds(activityRow, ids);
    }
  }
  return ids;
}

function isLocalUserMessage(message: LocalTranscriptMessage): boolean {
  return (
    message.role === 'user' &&
    Boolean(message.localState) &&
    message.localState !== 'remote_final' &&
    !(Boolean(message.internal) && message.internalKind === 'loop_continuation')
  );
}

function toolEntry(
  entry: RenderToolEntry,
  messages: MessagesBySeq,
): Extract<RenderTranscriptEntry, { kind: 'tool' }> | null {
  const toolUse = lookup(messages, entry.tool_use) ?? undefined;
  const toolResult = lookup(messages, entry.tool_result) ?? undefined;
  // Drop entries whose bodies aren't in the loaded committed window yet.
  if (!toolUse && !toolResult) {
    return null;
  }
  return { kind: 'tool', key: entry.id, toolUse, toolResult };
}

function toolGroupBlock(
  group: RenderToolGroup,
  messages: MessagesBySeq,
): RenderTranscriptBlock | null {
  const entries = group.entries
    .map((entry) => toolEntry(entry, messages))
    .filter(
      (entry): entry is Extract<RenderTranscriptEntry, { kind: 'tool' }> =>
        entry !== null,
    );
  if (!entries.length) {
    return null;
  }
  return { kind: 'tool_group', key: group.id, defaultExpanded: false, entries };
}

function stepBlocks(
  step: RenderStepRow,
  messages: MessagesBySeq,
): RenderTranscriptBlock[] {
  const blocks: RenderTranscriptBlock[] = [];
  for (const item of step.steps) {
    if (item.kind === 'assistant_message') {
      const message = lookup(messages, item.message);
      if (message) {
        blocks.push(messageBlock(message));
      }
      continue;
    }
    if (item.kind === 'tool_group') {
      const block = toolGroupBlock(item, messages);
      if (block) {
        blocks.push(block);
      }
    }
  }
  return blocks;
}

function stepToTurnRow(
  step: RenderStepRow,
  messages: MessagesBySeq,
): TurnRow | null {
  const steps = stepBlocks(step, messages);
  const finalMessage = lookup(messages, step.final_message);
  const finalBlock = finalMessage ? messageBlock(finalMessage) : null;
  if (!steps.length && !finalBlock) {
    return null;
  }
  return {
    kind: 'turn',
    key: `turn:${step.id}`,
    steps,
    finalBlock,
    isRunning: step.running,
    startedAt: step.started_at,
    finishedAt: step.finished_at,
  };
}

function activityToRow(
  activity: RenderActivityRow,
  messages: MessagesBySeq,
): UserTurnActivityRow | null {
  if (activity.kind === 'assistant_reply') {
    const message = lookup(messages, activity.message);
    if (!message) {
      return null;
    }
    const block = messageBlock(message);
    return { kind: 'flat', key: block.key, block };
  }
  if (activity.kind === 'step') {
    return stepToTurnRow(activity, messages);
  }
  return null;
}

/**
 * Map `render_state.rows` → top-level rows for solo threads. Rows/blocks whose
 * referenced messages aren't in the loaded committed window are skipped, which
 * naturally renders the loaded suffix while older history pages in.
 */
export function buildThreadViewRows(
  renderState: RenderState | null | undefined,
  messages: MessagesBySeq,
): TurnRenderRow[] {
  if (!renderState) {
    return [];
  }
  const rows: TurnRenderRow[] = [];
  for (const row of renderState.rows) {
    if (row.kind !== 'user_turn') {
      continue;
    }
    const activityRows = row.activity
      .map((activity) => activityToRow(activity, messages))
      .filter((activity): activity is UserTurnActivityRow => activity !== null);
    const user = lookup(messages, row.user);
    if (user) {
      rows.push({
        kind: 'user_turn',
        key: `user-turn:${user.id}`,
        userBlock: messageBlock(user),
        activityRows,
      });
      continue;
    }
    // Orphan turn (server `user=null`, or the user body isn't loaded yet):
    // surface its activity at the top level, mirroring the old orphan handling.
    rows.push(...activityRows);
  }
  return rows;
}

export function buildThreadViewRowsWithLocalUsers(
  renderState: RenderState | null | undefined,
  messages: MessagesBySeq,
  activeMessages: readonly LocalTranscriptMessage[],
): TurnRenderRow[] {
  const rows = buildThreadViewRows(renderState, messages);
  const representedMessageIds = representedMessageIdsForRows(rows);
  const committedMessageIds = new Set(
    [...messages.values()].map((message) => message.id),
  );
  const localRows: UserTurnRow[] = [];
  for (const message of activeMessages) {
    if (!isLocalUserMessage(message)) {
      continue;
    }
    if (
      representedMessageIds.has(message.id) ||
      committedMessageIds.has(message.id)
    ) {
      continue;
    }
    localRows.push({
      kind: 'user_turn',
      key: `user-turn:${message.id}`,
      userBlock: messageBlock(message),
      activityRows: [],
    });
  }
  return localRows.length ? [...rows, ...localRows] : rows;
}

/**
 * Deterministic flatten of `render_state.rows` → ordered blocks for team mode,
 * which renders blocks linearly with per-agent speaker headers instead of
 * collapsing turns. Block order matches the old `buildRenderTranscriptBlocks`.
 */
export function buildThreadViewBlocks(
  renderState: RenderState | null | undefined,
  messages: MessagesBySeq,
): RenderTranscriptBlock[] {
  if (!renderState) {
    return [];
  }
  const blocks: RenderTranscriptBlock[] = [];
  for (const row of renderState.rows) {
    if (row.kind !== 'user_turn') {
      continue;
    }
    const user = lookup(messages, row.user);
    if (user) {
      blocks.push(messageBlock(user));
    }
    for (const activity of row.activity) {
      if (activity.kind === 'assistant_reply') {
        const message = lookup(messages, activity.message);
        if (message) {
          blocks.push(messageBlock(message));
        }
        continue;
      }
      if (activity.kind === 'step') {
        blocks.push(...stepBlocks(activity, messages));
        const finalMessage = lookup(messages, activity.final_message);
        if (finalMessage) {
          blocks.push(messageBlock(finalMessage));
        }
      }
    }
  }
  return blocks;
}
