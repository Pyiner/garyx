import type { RenderTranscriptBlock } from './transcript-render';

/**
 * A "turn" groups consecutive non-user blocks (intermediate assistant
 * messages + tool groups) so the renderer can collapse them behind a
 * single elapsed-time summary, mirroring Codex's chat layout.
 *
 * Once the run is complete, the trailing assistant message — when present
 * and not pending — is surfaced as `finalBlock` so it renders OUTSIDE the
 * collapsible (this is the "user-visible answer", which Codex never folds).
 */
export interface TurnRow {
  kind: 'turn';
  key: string;
  /** Blocks rendered inside the collapsible body (oldest → newest). */
  steps: RenderTranscriptBlock[];
  /** Final assistant message rendered outside the collapsible. */
  finalBlock: RenderTranscriptBlock | null;
  /** True while any block in the turn is still streaming. */
  isRunning: boolean;
  /** Earliest timestamp on any block in the turn (ISO string). */
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

export interface BuildTurnRowsOptions {
  /** Only true once the current run is known to be finished. */
  surfaceFinalAssistant?: boolean;
  /** Keep the trailing assistant message inside the active turn until the run is done. */
  deferTrailingFinalAssistant?: boolean;
}

function isUserBlock(block: RenderTranscriptBlock): boolean {
  return block.kind === 'message' && block.entry.message.role === 'user';
}

function blockTimestamp(block: RenderTranscriptBlock): string | null {
  if (block.kind === 'message') {
    return block.entry.message.timestamp ?? null;
  }
  // Tool groups don't carry their own timestamp; fall back to the first
  // tool message's timestamp when available.
  const first = block.entries[0];
  if (!first) return null;
  return (first.toolUse?.timestamp ?? first.toolResult?.timestamp) ?? null;
}

function blockIsPending(block: RenderTranscriptBlock): boolean {
  if (block.kind !== 'message') return false;
  return block.entry.message.pending === true;
}

function pickFinalBlock(
  steps: RenderTranscriptBlock[],
): { steps: RenderTranscriptBlock[]; finalBlock: RenderTranscriptBlock | null } {
  // Only surface the final block when the LAST block is a non-pending
  // assistant message after the caller has confirmed the run is done.
  // While the stream is in flight or the trailing block is a tool group,
  // leave it inside the collapsible so the user sees live progress.
  if (steps.length === 0) {
    return { steps, finalBlock: null };
  }
  const last = steps[steps.length - 1]!;
  if (
    last.kind === 'message' &&
    last.entry.message.role === 'assistant' &&
    !last.entry.message.pending
  ) {
    return { steps: steps.slice(0, -1), finalBlock: last };
  }
  return { steps, finalBlock: null };
}

function summarizeTurn(
  steps: RenderTranscriptBlock[],
  finalBlock: RenderTranscriptBlock | null,
  key: string,
  precedingUserTs: string | null,
): TurnRow {
  const allBlocks = finalBlock ? [...steps, finalBlock] : steps;
  const isRunning = allBlocks.some(blockIsPending);
  const timestamps = allBlocks
    .map(blockTimestamp)
    .filter((value): value is string => Boolean(value));
  // Prefer the preceding user message's timestamp so the "Worked for X"
  // counter starts ticking from the submit moment (matching Codex), even
  // when the turn ends up with a single short assistant reply that
  // doesn't carry meaningful intra-turn timestamps of its own.
  const startedAt =
    precedingUserTs ?? (timestamps.length ? (timestamps[0] ?? null) : null);
  const finishedAt = isRunning
    ? null
    : timestamps.length
      ? (timestamps[timestamps.length - 1] ?? null)
      : null;
  return {
    kind: 'turn',
    key: `turn:${key}`,
    steps,
    finalBlock,
    isRunning,
    startedAt,
    finishedAt,
  };
}

function buildUserTurnActivityRows(
  steps: RenderTranscriptBlock[],
  key: string | null,
  precedingUserTs: string | null,
  options: BuildTurnRowsOptions,
  isTrailingTurn: boolean,
): UserTurnActivityRow[] {
  const surfaceFinalAssistant = options.surfaceFinalAssistant !== false;
  const deferTrailingFinalAssistant =
    options.deferTrailingFinalAssistant === true;
  if (!steps.length || !key) {
    return [];
  }
  // Codex emits its `worked-for` divider only as part of a turn that
  // had real agent activity (reasoning + tool calls). A pure-text
  // reply ("1 + 1 = 2") should sit flat under the user message with
  // no Worked-for header. Detect that case here so we mirror the
  // same UX.
  const isTrailingDeferredTurn = deferTrailingFinalAssistant && isTrailingTurn;
  const isPureTextReply =
    surfaceFinalAssistant &&
    !isTrailingDeferredTurn &&
    steps.length === 1 &&
    steps[0]!.kind === 'message' &&
    steps[0]!.entry.message.role === 'assistant' &&
    steps[0]!.entry.message.pending !== true;
  if (isPureTextReply) {
    const only = steps[0]!;
    return [{ kind: 'flat', key: only.key, block: only }];
  }

  const shouldSurfaceFinalAssistant =
    surfaceFinalAssistant && !(deferTrailingFinalAssistant && isTrailingTurn);
  const { steps: summarySteps, finalBlock } = shouldSurfaceFinalAssistant
    ? pickFinalBlock(steps)
    : { steps, finalBlock: null };
  return [summarizeTurn(summarySteps, finalBlock, key, precedingUserTs)];
}

/**
 * Walks the existing block list and builds explicit user turns. A user turn
 * is the user message plus all following agent activity until the next user
 * message. This is the shared unit for rendering, folding, and scroll
 * pagination heuristics.
 *
 * The grouping is stable and idempotent — calling it again on the same
 * input produces the same output, which keeps React keys consistent.
 */
export function buildTurnRows(
  blocks: RenderTranscriptBlock[],
  options: BuildTurnRowsOptions = {},
): TurnRenderRow[] {
  const rows: TurnRenderRow[] = [];
  let currentUserBlock: RenderTranscriptBlock | null = null;
  let currentSteps: RenderTranscriptBlock[] = [];
  let currentKey: string | null = null;
  let precedingUserTs: string | null = null;

  const flush = (isTrailingTurn = false) => {
    if (!currentUserBlock) {
      rows.push(
        ...buildUserTurnActivityRows(
          currentSteps,
          currentKey,
          precedingUserTs,
          options,
          isTrailingTurn,
        ),
      );
    } else {
      rows.push({
        kind: 'user_turn',
        key: `user-turn:${currentUserBlock.key}`,
        userBlock: currentUserBlock,
        activityRows: buildUserTurnActivityRows(
          currentSteps,
          currentKey,
          precedingUserTs,
          options,
          isTrailingTurn,
        ),
      });
    }
    currentUserBlock = null;
    currentSteps = [];
    currentKey = null;
    precedingUserTs = null;
  };

  for (const block of blocks) {
    if (isUserBlock(block)) {
      flush(false);
      currentUserBlock = block;
      if (block.kind === 'message' && block.entry.message.timestamp) {
        precedingUserTs = block.entry.message.timestamp;
      }
      continue;
    }
    if (currentKey === null) {
      currentKey = block.key;
    }
    currentSteps.push(block);
  }
  flush(true);
  return rows;
}
