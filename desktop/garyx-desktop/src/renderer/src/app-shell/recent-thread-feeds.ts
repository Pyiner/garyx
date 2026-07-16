import type {
  DesktopRecentThreadsPage,
  DesktopThreadSummary,
  RecentThreadTaskFilter,
} from "@shared/contracts";

export type RecentThreadFilter = "all" | "nonTask";
export type RecentThreadRequestKind = "refresh" | "loadMore";
export type RecentThreadLoadGate = "ready" | "exhausted" | "failed";
export type RecentThreadRefreshMode = "rangeFill" | "replacement";

export const RECENT_THREAD_PAGE_LIMIT = 100;
export const RECENT_THREAD_MAX_CHAIN_PAGES = 5;
export const RECENT_THREAD_REPLACEMENT_CYCLE_INTERVAL = 30;

export interface RecentThreadFeedState {
  orderedThreadIds: string[];
  isPrimed: boolean;
  isRefreshingHead: boolean;
  isLoadingMore: boolean;
  headFailure: string | null;
  loadGate: RecentThreadLoadGate;
  nextCursor: string | null;
  epoch: number;
  localMutationSequence: number;
  loadMoreFailureRevision: number;
  activeRefreshRequestId: number | null;
  activeLoadMoreRequestId: number | null;
  refreshAfterMutation: boolean;
  loadMoreAfterMutation: boolean;
  storeIncarnationId: string | null;
  serverBootId: string | null;
  refreshCycle: number;
  forceReplacementPending: boolean;
  forceReplacementGeneration: number;
  /** A second head verification still moved. It is serviced next cycle. */
  trailingDirty: boolean;
}

export interface RecentThreadFeedsState {
  gatewayScope: string;
  runtimeEpoch: number;
  selectedFilter: RecentThreadFilter;
  feeds: Record<RecentThreadFilter, RecentThreadFeedState>;
  summariesById: Record<string, DesktopThreadSummary>;
  /** Session tombstones for successful or still-pending local archives. */
  removedThreadIds: Record<string, true>;
  nextRequestId: number;
}

interface RecentThreadRequestTicketBase {
  gatewayScope: string;
  runtimeEpoch: number;
  filter: RecentThreadFilter;
  feedEpoch: number;
  requestId: number;
  observedLocalMutationSequence: number;
  kind: RecentThreadRequestKind;
  limit: number;
  cursor: string | null;
}

export interface RecentThreadRefreshTicket
  extends RecentThreadRequestTicketBase {
  kind: "refresh";
  observedLoadMoreFailureRevision: number;
  mode: RecentThreadRefreshMode;
  oldHeadActivitySeq: number | null;
  observedForceReplacementGeneration: number;
}

export interface RecentThreadLoadMoreTicket
  extends RecentThreadRequestTicketBase {
  kind: "loadMore";
}

export type RecentThreadRequestTicket =
  | RecentThreadRefreshTicket
  | RecentThreadLoadMoreTicket;

export interface RecentThreadRefreshBundle {
  primaryPages: DesktopRecentThreadsPage[];
  verificationPage: DesktopRecentThreadsPage;
  /** At most one immediate fill round after the first head verification. */
  immediatePages?: DesktopRecentThreadsPage[];
  immediateVerificationPage?: DesktopRecentThreadsPage;
}

export interface RecentThreadRequestDecision<T extends RecentThreadRequestTicket> {
  state: RecentThreadFeedsState;
  ticket: T | null;
}

export interface RecentThreadRemovalRollback {
  gatewayScope: string;
  threadId: string;
  positions: Record<RecentThreadFilter, number>;
}

export type RecentThreadCompletionAction =
  | "applied"
  | "dropped"
  | "forceReplacement";

export interface RecentThreadCompletion {
  state: RecentThreadFeedsState;
  action: RecentThreadCompletionAction;
}

const FILTERS: RecentThreadFilter[] = ["all", "nonTask"];

function createFeed(epoch = 0): RecentThreadFeedState {
  return {
    orderedThreadIds: [],
    isPrimed: false,
    isRefreshingHead: false,
    isLoadingMore: false,
    headFailure: null,
    loadGate: "ready",
    nextCursor: null,
    epoch,
    localMutationSequence: 0,
    loadMoreFailureRevision: 0,
    activeRefreshRequestId: null,
    activeLoadMoreRequestId: null,
    refreshAfterMutation: false,
    loadMoreAfterMutation: false,
    storeIncarnationId: null,
    serverBootId: null,
    refreshCycle: 0,
    forceReplacementPending: false,
    forceReplacementGeneration: 0,
    trailingDirty: false,
  };
}

export function createRecentThreadFeedsState(
  gatewayScope = "",
  runtimeEpoch = 0,
): RecentThreadFeedsState {
  return {
    gatewayScope,
    runtimeEpoch,
    selectedFilter: "all",
    feeds: {
      all: createFeed(),
      nonTask: createFeed(),
    },
    summariesById: {},
    removedThreadIds: {},
    nextRequestId: 1,
  };
}

export function recentThreadTasksQuery(
  filter: RecentThreadFilter,
): RecentThreadTaskFilter {
  return filter === "all" ? "include" : "exclude";
}

export function recentThreadFilterLabel(filter: RecentThreadFilter): string {
  return filter === "all" ? "All" : "Chats";
}

export function resetRecentThreadFeedsScope(
  state: RecentThreadFeedsState,
  gatewayScope: string,
  runtimeEpoch = state.runtimeEpoch + (state.gatewayScope === gatewayScope ? 0 : 1),
): RecentThreadFeedsState {
  if (
    state.gatewayScope === gatewayScope &&
    state.runtimeEpoch === runtimeEpoch
  ) {
    return state;
  }
  return {
    gatewayScope,
    runtimeEpoch,
    selectedFilter: "all",
    feeds: {
      all: createFeed(state.feeds.all.epoch + 1),
      nonTask: createFeed(state.feeds.nonTask.epoch + 1),
    },
    summariesById: {},
    removedThreadIds: {},
    nextRequestId: state.nextRequestId,
  };
}

export function selectRecentThreadFilter(
  state: RecentThreadFeedsState,
  filter: RecentThreadFilter,
): RecentThreadFeedsState {
  return state.selectedFilter === filter
    ? state
    : { ...state, selectedFilter: filter };
}

export function ingestRecentThreadSummaries(
  state: RecentThreadFeedsState,
  summaries: DesktopThreadSummary[],
): RecentThreadFeedsState {
  if (!summaries.length) {
    return state;
  }
  const summariesById = { ...state.summariesById };
  let changed = false;
  for (const summary of summaries) {
    const id = summary.id.trim();
    if (!id) {
      continue;
    }
    const previous = summariesById[id];
    const next = previous
      ? mergeRecentThreadSummary(previous, summary)
      : summary;
    if (next !== previous) {
      summariesById[id] = next;
      changed = true;
    }
  }
  return changed ? { ...state, summariesById } : state;
}

function mergeRecentThreadSummary(
  previous: DesktopThreadSummary,
  incoming: DesktopThreadSummary,
): DesktopThreadSummary {
  return {
    ...previous,
    ...incoming,
    agentId: incoming.agentId ?? previous.agentId,
    workspacePath: incoming.workspacePath ?? previous.workspacePath,
    worktree: incoming.worktree ?? previous.worktree,
  };
}

export function requestRecentThreadRefresh(
  state: RecentThreadFeedsState,
  filter: RecentThreadFilter,
  options: { forceReplacement?: boolean } = {},
): RecentThreadRequestDecision<RecentThreadRefreshTicket> {
  const feed = state.feeds[filter];
  if (!state.gatewayScope || feed.isRefreshingHead || feed.isLoadingMore) {
    return { state, ticket: null };
  }
  const requestId = state.nextRequestId;
  const periodicReplacement =
    (feed.refreshCycle + 1) % RECENT_THREAD_REPLACEMENT_CYCLE_INTERVAL === 0;
  const mode: RecentThreadRefreshMode =
    options.forceReplacement ||
    feed.forceReplacementPending ||
    !feed.isPrimed ||
    periodicReplacement
      ? "replacement"
      : "rangeFill";
  const ticket: RecentThreadRefreshTicket = {
    gatewayScope: state.gatewayScope,
    runtimeEpoch: state.runtimeEpoch,
    filter,
    feedEpoch: feed.epoch,
    requestId,
    observedLocalMutationSequence: feed.localMutationSequence,
    observedLoadMoreFailureRevision: feed.loadMoreFailureRevision,
    kind: "refresh",
    mode,
    oldHeadActivitySeq: activitySeqForId(state, feed.orderedThreadIds[0]),
    observedForceReplacementGeneration: feed.forceReplacementGeneration,
    limit: RECENT_THREAD_PAGE_LIMIT,
    cursor: null,
  };
  return {
    state: {
      ...state,
      nextRequestId: requestId + 1,
      feeds: {
        ...state.feeds,
        [filter]: {
          ...feed,
          isRefreshingHead: true,
          headFailure: null,
          activeRefreshRequestId: requestId,
          refreshAfterMutation: false,
        },
      },
    },
    ticket,
  };
}

export function requestRecentThreadLoadMore(
  state: RecentThreadFeedsState,
  filter: RecentThreadFilter,
  retry = false,
): RecentThreadRequestDecision<RecentThreadLoadMoreTicket> {
  const feed = state.feeds[filter];
  const gateAllowsRequest = retry
    ? feed.loadGate === "failed"
    : feed.loadGate === "ready";
  if (
    !state.gatewayScope ||
    feed.isLoadingMore ||
    feed.isRefreshingHead ||
    feed.nextCursor === null ||
    !gateAllowsRequest ||
    feed.forceReplacementPending
  ) {
    return { state, ticket: null };
  }
  const requestId = state.nextRequestId;
  const ticket: RecentThreadLoadMoreTicket = {
    gatewayScope: state.gatewayScope,
    runtimeEpoch: state.runtimeEpoch,
    filter,
    feedEpoch: feed.epoch,
    requestId,
    observedLocalMutationSequence: feed.localMutationSequence,
    kind: "loadMore",
    limit: RECENT_THREAD_PAGE_LIMIT,
    cursor: feed.nextCursor,
  };
  return {
    state: {
      ...state,
      nextRequestId: requestId + 1,
      feeds: {
        ...state.feeds,
        [filter]: {
          ...feed,
          isLoadingMore: true,
          activeLoadMoreRequestId: requestId,
          loadMoreAfterMutation: false,
        },
      },
    },
    ticket,
  };
}

/** Whether an IO orchestrator must follow the last page's cursor. */
export function recentRefreshChainNeedsNextPage(
  mode: RecentThreadRefreshMode,
  oldHeadActivitySeq: number | null,
  pages: DesktopRecentThreadsPage[],
): boolean {
  const last = pages.at(-1);
  if (!last || !last.hasMore || pages.length >= RECENT_THREAD_MAX_CHAIN_PAGES) {
    return false;
  }
  if (mode === "replacement" || oldHeadActivitySeq === null) {
    return true;
  }
  const tail = last.threads.at(-1)?.activitySeq;
  return typeof tail === "number" && tail > oldHeadActivitySeq;
}

export function recentPageHeadActivitySeq(
  page: DesktopRecentThreadsPage,
): number | null {
  const value = page.threads[0]?.activitySeq;
  return typeof value === "number" ? value : null;
}

export function verificationObservedNewerHead(
  chainFirstHead: number | null,
  verificationPage: DesktopRecentThreadsPage,
): boolean {
  const verificationHead = recentPageHeadActivitySeq(verificationPage);
  return (
    verificationHead !== null &&
    (chainFirstHead === null || verificationHead > chainFirstHead)
  );
}

export function completeRecentThreadRefresh(
  state: RecentThreadFeedsState,
  ticket: RecentThreadRefreshTicket,
  bundle: RecentThreadRefreshBundle,
): RecentThreadCompletion {
  const feed = state.feeds[ticket.filter];
  if (!requestIsOwned(state, feed, ticket)) {
    return { state, action: "dropped" };
  }
  if (feed.localMutationSequence !== ticket.observedLocalMutationSequence) {
    return {
      state: markRecentThreadMutationFollowUp(
        clearOwnedRequest(state, ticket),
        ticket,
      ),
      action: "dropped",
    };
  }
  if (!bundle.primaryPages.length) {
    return {
      state: failRecentThreadRequest(state, ticket, "Recent threads are unavailable"),
      action: "dropped",
    };
  }

  const allPages = [
    ...bundle.primaryPages,
    bundle.verificationPage,
    ...(bundle.immediatePages ?? []),
    ...(bundle.immediateVerificationPage
      ? [bundle.immediateVerificationPage]
      : []),
  ];
  const identity = consistentPageIdentity(allPages);
  if (
    !identity ||
    allPages.some((page) => page.gatewayScope !== ticket.gatewayScope) ||
    (feed.storeIncarnationId !== null &&
      feed.storeIncarnationId !== identity.storeIncarnationId) ||
    (feed.serverBootId !== null &&
      feed.serverBootId !== identity.serverBootId &&
      ticket.mode !== "replacement")
  ) {
    return {
      state: markRecentThreadForceReplacement(
        clearOwnedRequest(state, ticket),
        [ticket.filter],
      ),
      action: "forceReplacement",
    };
  }

  const primary = applyRefreshChain(
    state,
    ticket,
    bundle.primaryPages,
    feed.orderedThreadIds,
    feed.nextCursor,
    feed.loadGate,
  );
  let orderedThreadIds = primary.orderedThreadIds;
  let nextCursor = primary.nextCursor;
  let loadGate = primary.loadGate;
  let replacementCommitted = primary.replacementCommitted;
  const primaryHead = recentPageHeadActivitySeq(bundle.primaryPages[0]);
  const needsImmediate = verificationObservedNewerHead(
    primaryHead,
    bundle.verificationPage,
  );
  let trailingDirty = false;

  if (needsImmediate && bundle.immediatePages?.length) {
    const immediateTicket: RecentThreadRefreshTicket = {
      ...ticket,
      mode: "rangeFill",
      oldHeadActivitySeq: primaryHead,
    };
    const immediate = applyRefreshChain(
      state,
      immediateTicket,
      bundle.immediatePages,
      orderedThreadIds,
      nextCursor,
      loadGate,
    );
    orderedThreadIds = immediate.orderedThreadIds;
    nextCursor = immediate.nextCursor;
    loadGate = immediate.loadGate;
    replacementCommitted =
      replacementCommitted || immediate.replacementCommitted;
    const immediateHead = recentPageHeadActivitySeq(bundle.immediatePages[0]);
    trailingDirty = bundle.immediateVerificationPage
      ? verificationObservedNewerHead(
          immediateHead,
          bundle.immediateVerificationPage,
        )
      : true;
  } else if (needsImmediate) {
    trailingDirty = true;
  }

  const visiblePages = [
    ...bundle.primaryPages,
    ...(bundle.immediatePages ?? []),
  ];
  const visibleThreads = visiblePages.flatMap((page) =>
    page.threads.filter(
      (thread) => !state.removedThreadIds[thread.id.trim()],
    ),
  );
  const withSummaries = ingestRecentThreadSummaries(state, visibleThreads);
  const current = withSummaries.feeds[ticket.filter];
  const nextEpoch = replacementCommitted ? current.epoch + 1 : current.epoch;
  const replacementRequestedAfterDispatch =
    current.forceReplacementPending &&
    current.forceReplacementGeneration !==
      ticket.observedForceReplacementGeneration;
  return {
    state: {
      ...withSummaries,
      feeds: {
        ...withSummaries.feeds,
        [ticket.filter]: {
          ...current,
          orderedThreadIds: orderedThreadIds.filter(
            (id) => !state.removedThreadIds[id],
          ),
          isPrimed: true,
          isRefreshingHead: false,
          headFailure: null,
          loadGate,
          nextCursor,
          epoch: nextEpoch,
          activeRefreshRequestId: null,
          storeIncarnationId: identity.storeIncarnationId,
          serverBootId: identity.serverBootId,
          refreshCycle: current.refreshCycle + 1,
          forceReplacementPending: replacementRequestedAfterDispatch,
          trailingDirty,
        },
      },
    },
    action: replacementRequestedAfterDispatch
      ? "forceReplacement"
      : "applied",
  };
}

export function completeRecentThreadLoadMore(
  state: RecentThreadFeedsState,
  ticket: RecentThreadLoadMoreTicket,
  page: DesktopRecentThreadsPage,
): RecentThreadCompletion {
  const feed = state.feeds[ticket.filter];
  if (!requestIsOwned(state, feed, ticket)) {
    return { state, action: "dropped" };
  }
  if (feed.localMutationSequence !== ticket.observedLocalMutationSequence) {
    return {
      state: markRecentThreadMutationFollowUp(
        clearOwnedRequest(state, ticket),
        ticket,
      ),
      action: "dropped",
    };
  }
  if (
    page.gatewayScope !== ticket.gatewayScope ||
    (feed.storeIncarnationId !== null &&
      feed.storeIncarnationId !== page.storeIncarnationId) ||
    (feed.serverBootId !== null && feed.serverBootId !== page.serverBootId)
  ) {
    return {
      state: markRecentThreadForceReplacement(
        clearOwnedRequest(state, ticket),
        [ticket.filter],
      ),
      action: "forceReplacement",
    };
  }
  const visibleThreads = page.threads.filter(
    (thread) => !state.removedThreadIds[thread.id.trim()],
  );
  const withSummaries = ingestRecentThreadSummaries(state, visibleThreads);
  const current = withSummaries.feeds[ticket.filter];
  const pageIds = uniqueThreadIds(visibleThreads.map((thread) => thread.id));
  return {
    state: {
      ...withSummaries,
      feeds: {
        ...withSummaries.feeds,
        [ticket.filter]: {
          ...current,
          orderedThreadIds: appendPage(
            pageIds,
            current.orderedThreadIds,
          ),
          isLoadingMore: false,
          loadGate: page.hasMore ? "ready" : "exhausted",
          nextCursor: page.nextCursor,
          activeLoadMoreRequestId: null,
          storeIncarnationId: page.storeIncarnationId,
          serverBootId: page.serverBootId,
        },
      },
    },
    action: "applied",
  };
}

export function failRecentThreadRequest(
  state: RecentThreadFeedsState,
  ticket: RecentThreadRequestTicket,
  message: string,
): RecentThreadFeedsState {
  if (
    state.gatewayScope !== ticket.gatewayScope ||
    state.runtimeEpoch !== ticket.runtimeEpoch
  ) {
    return state;
  }
  const feed = state.feeds[ticket.filter];
  if (!requestIsOwned(state, feed, ticket)) {
    return state;
  }
  const failedFeed: RecentThreadFeedState =
    ticket.kind === "refresh"
      ? {
          ...feed,
          isRefreshingHead: false,
          headFailure: message || "Recent threads are unavailable",
          activeRefreshRequestId: null,
        }
      : {
          ...feed,
          isLoadingMore: false,
          loadGate: "failed",
          loadMoreFailureRevision: feed.loadMoreFailureRevision + 1,
          activeLoadMoreRequestId: null,
        };
  return {
    ...state,
    feeds: { ...state.feeds, [ticket.filter]: failedFeed },
  };
}

export function markRecentThreadForceReplacement(
  state: RecentThreadFeedsState,
  filters: RecentThreadFilter[] = FILTERS,
): RecentThreadFeedsState {
  const feeds = { ...state.feeds };
  for (const filter of filters) {
    feeds[filter] = {
      ...feeds[filter],
      forceReplacementPending: true,
      forceReplacementGeneration:
        feeds[filter].forceReplacementGeneration + 1,
      trailingDirty: false,
    };
  }
  return { ...state, feeds };
}

export function removeThreadFromRecentFeeds(
  state: RecentThreadFeedsState,
  threadId: string,
): { state: RecentThreadFeedsState; rollback: RecentThreadRemovalRollback } {
  const normalizedId = threadId.trim();
  const positions = {
    all: state.feeds.all.orderedThreadIds.indexOf(normalizedId),
    nonTask: state.feeds.nonTask.orderedThreadIds.indexOf(normalizedId),
  };
  const feeds = { ...state.feeds };
  for (const filter of FILTERS) {
    const feed = state.feeds[filter];
    feeds[filter] = {
      ...feed,
      orderedThreadIds: feed.orderedThreadIds.filter(
        (candidate) => candidate !== normalizedId,
      ),
      localMutationSequence: feed.localMutationSequence + 1,
    };
  }
  return {
    state: {
      ...state,
      feeds,
      removedThreadIds: normalizedId
        ? { ...state.removedThreadIds, [normalizedId]: true }
        : state.removedThreadIds,
    },
    rollback: {
      gatewayScope: state.gatewayScope,
      threadId: normalizedId,
      positions,
    },
  };
}

export function rollbackRecentThreadRemoval(
  state: RecentThreadFeedsState,
  rollback: RecentThreadRemovalRollback,
): RecentThreadFeedsState {
  if (
    state.gatewayScope !== rollback.gatewayScope ||
    !rollback.threadId
  ) {
    return state;
  }
  const feeds = { ...state.feeds };
  for (const filter of FILTERS) {
    const position = rollback.positions[filter];
    const feed = state.feeds[filter];
    const withoutThread = feed.orderedThreadIds.filter(
      (candidate) => candidate !== rollback.threadId,
    );
    if (position >= 0) {
      withoutThread.splice(
        Math.min(position, withoutThread.length),
        0,
        rollback.threadId,
      );
    }
    feeds[filter] = {
      ...feed,
      orderedThreadIds: withoutThread,
      localMutationSequence: feed.localMutationSequence + 1,
    };
  }
  const removedThreadIds = { ...state.removedThreadIds };
  delete removedThreadIds[rollback.threadId];
  return { ...state, feeds, removedThreadIds };
}

export function upsertChatInRecentFeeds(
  state: RecentThreadFeedsState,
  summary: DesktopThreadSummary,
): RecentThreadFeedsState {
  const id = summary.id.trim();
  if (!id) {
    return state;
  }
  const withSummary = ingestRecentThreadSummaries(state, [summary]);
  const feeds = { ...withSummary.feeds };
  for (const filter of FILTERS) {
    const feed = withSummary.feeds[filter];
    feeds[filter] = {
      ...feed,
      orderedThreadIds: [
        id,
        ...feed.orderedThreadIds.filter((candidate) => candidate !== id),
      ],
      localMutationSequence: feed.localMutationSequence + 1,
    };
  }
  return { ...withSummary, feeds };
}

export function noteRecentThreadLocalMutation(
  state: RecentThreadFeedsState,
): RecentThreadFeedsState {
  return FILTERS.reduce(
    (current, filter) =>
      noteRecentThreadFilterLocalMutation(current, filter),
    state,
  );
}

export function noteRecentThreadFilterLocalMutation(
  state: RecentThreadFeedsState,
  filter: RecentThreadFilter,
): RecentThreadFeedsState {
  return {
    ...state,
    feeds: {
      ...state.feeds,
      [filter]: {
        ...state.feeds[filter],
        localMutationSequence:
          state.feeds[filter].localMutationSequence + 1,
      },
    },
  };
}

export function consumeRecentThreadMutationFollowUp(
  state: RecentThreadFeedsState,
  filter: RecentThreadFilter,
  kind: RecentThreadRequestKind,
): RecentThreadRequestDecision<RecentThreadRequestTicket> {
  const feed = state.feeds[filter];
  const requested =
    kind === "refresh"
      ? feed.refreshAfterMutation
      : feed.loadMoreAfterMutation;
  const active = feed.isRefreshingHead || feed.isLoadingMore;
  if (!requested || active) {
    return { state, ticket: null };
  }

  const cleared: RecentThreadFeedsState = {
    ...state,
    feeds: {
      ...state.feeds,
      [filter]: {
        ...feed,
        ...(kind === "refresh"
          ? { refreshAfterMutation: false }
          : { loadMoreAfterMutation: false }),
      },
    },
  };
  if (kind === "refresh") {
    return requestRecentThreadRefresh(cleared, filter);
  }
  return requestRecentThreadLoadMore(
    cleared,
    filter,
    feed.loadGate === "failed",
  );
}

export function selectedRecentThreadFeed(
  state: RecentThreadFeedsState,
): RecentThreadFeedState {
  return state.feeds[state.selectedFilter];
}

export function selectedRecentThreadSummaries(
  state: RecentThreadFeedsState,
): DesktopThreadSummary[] {
  return selectedRecentThreadFeed(state).orderedThreadIds.flatMap((id) => {
    const summary = state.summariesById[id];
    return summary ? [summary] : [];
  });
}

function requestIsOwned(
  state: RecentThreadFeedsState,
  feed: RecentThreadFeedState,
  ticket: RecentThreadRequestTicket,
): boolean {
  if (
    state.gatewayScope !== ticket.gatewayScope ||
    state.runtimeEpoch !== ticket.runtimeEpoch ||
    feed.epoch !== ticket.feedEpoch
  ) {
    return false;
  }
  return ticket.kind === "refresh"
    ? feed.activeRefreshRequestId === ticket.requestId
    : feed.activeLoadMoreRequestId === ticket.requestId;
}

function clearOwnedRequest(
  state: RecentThreadFeedsState,
  ticket: RecentThreadRequestTicket,
): RecentThreadFeedsState {
  const feed = state.feeds[ticket.filter];
  if (!requestIsOwned(state, feed, ticket)) {
    return state;
  }
  const nextFeed =
    ticket.kind === "refresh"
      ? {
          ...feed,
          isRefreshingHead: false,
          activeRefreshRequestId: null,
        }
      : {
          ...feed,
          isLoadingMore: false,
          activeLoadMoreRequestId: null,
        };
  return {
    ...state,
    feeds: { ...state.feeds, [ticket.filter]: nextFeed },
  };
}

function markRecentThreadMutationFollowUp(
  state: RecentThreadFeedsState,
  ticket: RecentThreadRequestTicket,
): RecentThreadFeedsState {
  const feed = state.feeds[ticket.filter];
  return {
    ...state,
    feeds: {
      ...state.feeds,
      [ticket.filter]: {
        ...feed,
        ...(ticket.kind === "refresh"
          ? { refreshAfterMutation: true }
          : { loadMoreAfterMutation: true }),
      },
    },
  };
}

function applyRefreshChain(
  state: RecentThreadFeedsState,
  ticket: RecentThreadRefreshTicket,
  pages: DesktopRecentThreadsPage[],
  existingIds: string[],
  existingCursor: string | null,
  existingLoadGate: RecentThreadLoadGate,
): {
  orderedThreadIds: string[];
  nextCursor: string | null;
  loadGate: RecentThreadLoadGate;
  replacementCommitted: boolean;
} {
  const pageIds = uniqueThreadIds(
    pages.flatMap((page) =>
      page.threads
        .filter((thread) => !state.removedThreadIds[thread.id.trim()])
        .map((thread) => thread.id),
    ),
  );
  const last = pages.at(-1);
  const tailSeq = last?.threads.at(-1)?.activitySeq;
  const reachedAnchor =
    ticket.oldHeadActivitySeq !== null &&
    typeof tailSeq === "number" &&
    tailSeq <= ticket.oldHeadActivitySeq;
  const exhaustedBeforeAnchor = Boolean(last && !last.hasMore && !reachedAnchor);
  const exceededWindow =
    pages.length >= RECENT_THREAD_MAX_CHAIN_PAGES && !reachedAnchor;
  const replacementCommitted =
    ticket.mode === "replacement" ||
    ticket.oldHeadActivitySeq === null ||
    exhaustedBeforeAnchor ||
    exceededWindow;
  if (replacementCommitted) {
    return {
      orderedThreadIds: pageIds,
      nextCursor: last?.nextCursor ?? null,
      loadGate: last?.hasMore ? "ready" : "exhausted",
      replacementCommitted: true,
    };
  }

  let loadGate = existingLoadGate;
  if (
    loadGate === "failed" &&
    ticket.observedLoadMoreFailureRevision ===
      state.feeds[ticket.filter].loadMoreFailureRevision
  ) {
    loadGate = existingCursor === null ? "exhausted" : "ready";
  }
  return {
    orderedThreadIds: appendHead(pageIds, existingIds),
    nextCursor: existingCursor,
    loadGate,
    replacementCommitted: false,
  };
}

function consistentPageIdentity(
  pages: DesktopRecentThreadsPage[],
): { storeIncarnationId: string; serverBootId: string } | null {
  const first = pages[0];
  if (!first) {
    return null;
  }
  return pages.every(
    (page) =>
      page.gatewayScope === first.gatewayScope &&
      page.storeIncarnationId === first.storeIncarnationId &&
      page.serverBootId === first.serverBootId,
  )
    ? {
        storeIncarnationId: first.storeIncarnationId,
        serverBootId: first.serverBootId,
      }
    : null;
}

function activitySeqForId(
  state: RecentThreadFeedsState,
  id: string | undefined,
): number | null {
  const value = id ? state.summariesById[id]?.activitySeq : null;
  return typeof value === "number" ? value : null;
}

function uniqueThreadIds(ids: string[]): string[] {
  const seen = new Set<string>();
  return ids.flatMap((id) => {
    const normalized = id.trim();
    return normalized && !seen.has(normalized) && seen.add(normalized)
      ? [normalized]
      : [];
  });
}

function appendHead(pageIds: string[], existingIds: string[]): string[] {
  const pageSet = new Set(pageIds);
  return [...pageIds, ...existingIds.filter((id) => !pageSet.has(id))];
}

function appendPage(pageIds: string[], existingIds: string[]): string[] {
  const seen = new Set(existingIds);
  return [
    ...existingIds,
    ...pageIds.filter((id) => !seen.has(id) && Boolean(seen.add(id))),
  ];
}
