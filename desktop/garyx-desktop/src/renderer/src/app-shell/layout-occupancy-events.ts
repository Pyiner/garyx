import type {
  HorizontalLayoutEvent,
  LayoutIntentCause,
  LayoutPanelOccupancy,
} from "./responsive-layout-model";

export type {
  LayoutIntentCause,
  LayoutPanelOccupancy,
} from "./responsive-layout-model";

export type LayoutOccupancySources = Readonly<{
  globalSidebar: boolean;
  conversationRailKey: string | null;
  inspectorOpen: boolean;
  openCapsuleCount: number;
}>;

export type LayoutOccupancyEvent = Extract<
  HorizontalLayoutEvent,
  { type: "LAYOUT_INTENT_CHANGED" }
>;

export type LayoutOccupancyEventLog = Readonly<{
  currentSources: LayoutOccupancySources;
  events: readonly LayoutOccupancyEvent[];
  nextTransactionSequence: number;
}>;

const MAX_RETAINED_LAYOUT_OCCUPANCY_EVENTS = 256;

function assertSources(sources: LayoutOccupancySources): void {
  if (
    !Number.isInteger(sources.openCapsuleCount) ||
    sources.openCapsuleCount < 0
  ) {
    throw new RangeError("openCapsuleCount must be a non-negative integer");
  }
  if (sources.conversationRailKey === "") {
    throw new TypeError("conversationRailKey must be null or non-empty");
  }
}

export function projectLayoutOccupancy(
  sources: LayoutOccupancySources,
): LayoutPanelOccupancy {
  assertSources(sources);
  return {
    globalSidebar: sources.globalSidebar,
    conversationRail: sources.conversationRailKey !== null,
    sideTools: sources.inspectorOpen || sources.openCapsuleCount > 0,
  };
}

export function createLayoutOccupancyEventLog(
  initialSources: LayoutOccupancySources,
): LayoutOccupancyEventLog {
  assertSources(initialSources);
  return {
    currentSources: initialSources,
    events: [],
    nextTransactionSequence: 1,
  };
}

function occupanciesEqual(
  left: LayoutPanelOccupancy,
  right: LayoutPanelOccupancy,
): boolean {
  return (
    left.globalSidebar === right.globalSidebar &&
    left.conversationRail === right.conversationRail &&
    left.sideTools === right.sideTools
  );
}

export function appendLayoutOccupancyIntent(
  log: LayoutOccupancyEventLog,
  nextSources: LayoutOccupancySources,
  cause: LayoutIntentCause,
): {
  log: LayoutOccupancyEventLog;
  event: LayoutOccupancyEvent | null;
} {
  assertSources(nextSources);
  const previousOccupancy = projectLayoutOccupancy(log.currentSources);
  const nextOccupancy = projectLayoutOccupancy(nextSources);
  const conversationRailChanged =
    log.currentSources.conversationRailKey !==
    nextSources.conversationRailKey;

  // A rail-to-rail route switch is one replace transaction even though both
  // boolean occupancy vectors say L2=open. Side-tool source changes, by
  // contrast, emit only on the inspector/capsule union's 0↔1 edge.
  if (
    occupanciesEqual(previousOccupancy, nextOccupancy) &&
    !conversationRailChanged
  ) {
    return {
      log: {
        ...log,
        currentSources: nextSources,
      },
      event: null,
    };
  }

  const event: LayoutOccupancyEvent = {
    type: "LAYOUT_INTENT_CHANGED",
    previousOccupancy,
    nextOccupancy,
    cause,
    transactionId: `layout-intent-${log.nextTransactionSequence}`,
  };
  const events = [...log.events, event].slice(
    -MAX_RETAINED_LAYOUT_OCCUPANCY_EVENTS,
  );
  return {
    log: {
      currentSources: nextSources,
      events,
      nextTransactionSequence: log.nextTransactionSequence + 1,
    },
    event,
  };
}
