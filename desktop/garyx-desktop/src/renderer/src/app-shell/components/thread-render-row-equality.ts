import type {
  RenderCapsuleCard,
  RenderToolDiffRecipe,
  RenderToolDiffSegment,
  RenderToolFieldProjection,
  RenderToolFieldSelector,
  RenderToolValueSelector,
} from "@shared/contracts";

import type {
  RenderTranscriptBlock,
  TurnRenderRow,
  TurnRow,
  UserTurnActivityRow,
} from "../../render-view-model";

function stringArrayEqual(
  left: readonly string[] | undefined,
  right: readonly string[] | undefined,
): boolean {
  if (left === right) {
    return true;
  }
  if (!left || !right || left.length !== right.length) {
    return false;
  }
  return left.every((value, index) => value === right[index]);
}

function valueSelectorEqual(
  left: RenderToolValueSelector | null | undefined,
  right: RenderToolValueSelector | null | undefined,
): boolean {
  if (left === right) {
    return true;
  }
  if (!left || !right) {
    return false;
  }
  return (
    left.root === right.root &&
    stringArrayEqual(left.path, right.path)
  );
}

function selectorEqual(
  left: RenderToolFieldSelector | undefined,
  right: RenderToolFieldSelector | undefined,
): boolean {
  return (
    valueSelectorEqual(left, right) &&
    left?.format === right?.format &&
    left?.label === right?.label
  );
}

function diffSegmentEqual(
  left: RenderToolDiffSegment,
  right: RenderToolDiffSegment,
): boolean {
  if ("unified" in left && "unified" in right) {
    return valueSelectorEqual(left.unified.text, right.unified.text);
  }
  if ("pair" in left && "pair" in right) {
    return (
      valueSelectorEqual(left.pair.old, right.pair.old) &&
      valueSelectorEqual(left.pair.new, right.pair.new)
    );
  }
  return false;
}

function diffRecipeEqual(
  left: RenderToolDiffRecipe | undefined,
  right: RenderToolDiffRecipe | undefined,
): boolean {
  if (left === right) {
    return true;
  }
  return Boolean(
    left &&
    right &&
    left.source === right.source &&
    left.segments.length === right.segments.length &&
    left.segments.every((segment, index) =>
      diffSegmentEqual(segment, right.segments[index]),
    ),
  );
}

export function renderToolProjectionEqual(
  left: RenderToolFieldProjection | undefined,
  right: RenderToolFieldProjection | undefined,
): boolean {
  if (left === right) {
    return true;
  }
  if (!left || !right) {
    return false;
  }
  return (
    left.tool_name === right.tool_name &&
    left.kind === right.kind &&
    left.visibility === right.visibility &&
    left.status === right.status &&
    left.exit_code === right.exit_code &&
    left.duration_ms === right.duration_ms &&
    selectorEqual(left.summary, right.summary) &&
    selectorEqual(left.call, right.call) &&
    diffRecipeEqual(left.diff, right.diff) &&
    selectorEqual(left.result, right.result)
  );
}

function capsuleCardEqual(
  left: RenderCapsuleCard,
  right: RenderCapsuleCard,
): boolean {
  return (
    left.id === right.id &&
    left.capsule_id === right.capsule_id &&
    left.title === right.title &&
    left.revision === right.revision &&
    left.action === right.action
  );
}

function capsuleCardsEqual(
  left: readonly RenderCapsuleCard[],
  right: readonly RenderCapsuleCard[],
): boolean {
  if (left === right) {
    return true;
  }
  return (
    left.length === right.length &&
    left.every((card, index) => capsuleCardEqual(card, right[index]))
  );
}

function renderBlockEqual(
  left: RenderTranscriptBlock | null,
  right: RenderTranscriptBlock | null,
): boolean {
  if (left === right) {
    return true;
  }
  if (!left || !right || left.kind !== right.kind || left.key !== right.key) {
    return false;
  }
  if (left.kind === "message") {
    return (
      right.kind === "message" &&
      left.entry.key === right.entry.key &&
      left.entry.message === right.entry.message &&
      renderMessagePresentationEqual(
        left.entry.presentation,
        right.entry.presentation,
      )
    );
  }
  if (right.kind !== "tool_group") {
    return false;
  }
  return (
    left.defaultExpanded === right.defaultExpanded &&
    left.entries.length === right.entries.length &&
    left.entries.every((entry, index) => {
      const other = right.entries[index];
      return (
        entry.key === other.key &&
        entry.toolUse === other.toolUse &&
        entry.toolResult === other.toolResult &&
        renderToolProjectionEqual(entry.projection, other.projection)
      );
    })
  );
}

function renderMessagePresentationEqual(
  left: Extract<RenderTranscriptBlock, { kind: "message" }>["entry"]["presentation"],
  right: Extract<RenderTranscriptBlock, { kind: "message" }>["entry"]["presentation"],
): boolean {
  if (left === right) {
    return true;
  }
  if (!left || !right || left.kind !== right.kind) {
    return false;
  }
  return (
    left.event === right.event &&
    left.status === right.status &&
    left.task_id === right.task_id &&
    left.title === right.title
  );
}

function turnRowEqual(left: TurnRow, right: TurnRow): boolean {
  return (
    left.key === right.key &&
    left.isRunning === right.isRunning &&
    left.startedAt === right.startedAt &&
    left.finishedAt === right.finishedAt &&
    left.steps.length === right.steps.length &&
    left.steps.every((block, index) =>
      renderBlockEqual(block, right.steps[index]),
    ) &&
    renderBlockEqual(left.finalBlock, right.finalBlock)
  );
}

function activityRowEqual(
  left: UserTurnActivityRow,
  right: UserTurnActivityRow,
): boolean {
  if (left === right) {
    return true;
  }
  if (left.kind !== right.kind || left.key !== right.key) {
    return false;
  }
  if (left.kind === "flat") {
    return right.kind === "flat" && renderBlockEqual(left.block, right.block);
  }
  return right.kind === "turn" && turnRowEqual(left, right);
}

export function turnRenderRowPresentationEqual(
  left: TurnRenderRow,
  right: TurnRenderRow,
): boolean {
  if (left === right) {
    return true;
  }
  if (left.kind !== right.kind || left.key !== right.key) {
    return false;
  }
  if (left.kind === "flat") {
    return right.kind === "flat" && renderBlockEqual(left.block, right.block);
  }
  if (left.kind === "turn") {
    return right.kind === "turn" && turnRowEqual(left, right);
  }
  if (left.kind === "capsule_only") {
    return (
      right.kind === "capsule_only" &&
      capsuleCardsEqual(left.capsuleCards, right.capsuleCards)
    );
  }
  return (
    right.kind === "user_turn" &&
    renderBlockEqual(left.userBlock, right.userBlock) &&
    left.activityRows.length === right.activityRows.length &&
    left.activityRows.every((activity, index) =>
      activityRowEqual(activity, right.activityRows[index]),
    ) &&
    capsuleCardsEqual(left.capsuleCards, right.capsuleCards)
  );
}

function blockContainsToolGroup(
  block: RenderTranscriptBlock | null,
  groupId: string,
): boolean {
  return block?.kind === "tool_group" && block.key === groupId;
}

function activityContainsToolGroup(
  activity: UserTurnActivityRow,
  groupId: string,
): boolean {
  if (activity.kind === "flat") {
    return blockContainsToolGroup(activity.block, groupId);
  }
  return (
    activity.steps.some((block) => blockContainsToolGroup(block, groupId)) ||
    blockContainsToolGroup(activity.finalBlock, groupId)
  );
}

export function turnRenderRowContainsToolGroup(
  row: TurnRenderRow,
  groupId: string | null,
): boolean {
  if (!groupId || row.kind === "capsule_only") {
    return false;
  }
  if (row.kind === "flat") {
    return blockContainsToolGroup(row.block, groupId);
  }
  if (row.kind === "turn") {
    return activityContainsToolGroup(row, groupId);
  }
  return (
    blockContainsToolGroup(row.userBlock, groupId) ||
    row.activityRows.some((activity) =>
      activityContainsToolGroup(activity, groupId),
    )
  );
}

export function activeToolGroupChangeAffectsRow(
  row: TurnRenderRow,
  previousGroupId: string | null,
  nextGroupId: string | null,
): boolean {
  if (previousGroupId === nextGroupId) {
    return false;
  }
  return (
    turnRenderRowContainsToolGroup(row, previousGroupId) ||
    turnRenderRowContainsToolGroup(row, nextGroupId)
  );
}

export type TranscriptRenderRowComparableProps = {
  row: TurnRenderRow;
  activeToolGroupId: string | null;
  actions: unknown;
  translationIdentity: unknown;
  imagePreviewIdentity: unknown;
  canRetryFailedMessage: boolean;
  canOpenCapsule: boolean;
};

export function transcriptRenderRowPropsEqual(
  previous: TranscriptRenderRowComparableProps,
  next: TranscriptRenderRowComparableProps,
): boolean {
  if (
    previous.actions !== next.actions ||
    previous.translationIdentity !== next.translationIdentity ||
    previous.imagePreviewIdentity !== next.imagePreviewIdentity ||
    previous.canRetryFailedMessage !== next.canRetryFailedMessage ||
    previous.canOpenCapsule !== next.canOpenCapsule ||
    !turnRenderRowPresentationEqual(previous.row, next.row)
  ) {
    return false;
  }
  return !activeToolGroupChangeAffectsRow(
    previous.row,
    previous.activeToolGroupId,
    next.activeToolGroupId,
  );
}
