import type { RenderTranscriptBlock } from './transcript-render';

/**
 * A "turn" groups consecutive non-user blocks (intermediate assistant
 * messages + tool groups) so the renderer can collapse them behind a
 * single "已处理 Xs" summary, mirroring Codex's chat layout.
 *
 * The trailing assistant message — when present and not pending — is
 * surfaced as `finalBlock` so it renders OUTSIDE the collapsible (this
 * is the "user-visible answer", which Codex never folds).
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

export type TurnRenderRow = FlatRow | TurnRow;

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
  // assistant message. While the stream is in flight or the trailing
  // block is a tool group, leave it inside the collapsible so the user
  // sees live progress.
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

/**
 * Walks the existing block list and clusters consecutive non-user blocks
 * into `TurnRow`s. User blocks (and any other top-level blocks we don't
 * group) pass through as `FlatRow`s.
 *
 * The grouping is stable and idempotent — calling it again on the same
 * input produces the same output, which keeps React keys consistent.
 */
export function buildTurnRows(
  blocks: RenderTranscriptBlock[],
): TurnRenderRow[] {
  const rows: TurnRenderRow[] = [];
  let currentSteps: RenderTranscriptBlock[] = [];
  let currentKey: string | null = null;
  let precedingUserTs: string | null = null;

  const flush = () => {
    if (!currentSteps.length || !currentKey) {
      currentSteps = [];
      currentKey = null;
      return;
    }
    const { steps, finalBlock } = pickFinalBlock(currentSteps);
    // Always emit a TurnRow so every assistant turn shows the
    // Codex-parity "Worked for X" header — even short replies with no
    // intermediate tool calls. The header is the visible end-of-run
    // marker plus the toggle to inspect the run details if any exist.
    rows.push(summarizeTurn(steps, finalBlock, currentKey, precedingUserTs));
    currentSteps = [];
    currentKey = null;
  };

  for (const block of blocks) {
    if (isUserBlock(block)) {
      flush();
      rows.push({ kind: 'flat', key: block.key, block });
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
  flush();
  return rows;
}
