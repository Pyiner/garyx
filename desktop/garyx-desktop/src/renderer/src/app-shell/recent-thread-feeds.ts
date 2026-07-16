import type {
  DesktopRecentThreadsPage,
  DesktopThreadSummary,
  RecentThreadTaskFilter,
} from "@shared/contracts";

export type RecentThreadFilter = "all" | "nonTask";
export type RecentThreadRequestKind = "refresh" | "loadMore";
export type RecentThreadLoadGate = "ready" | "exhausted" | "failed";

export const RECENT_THREAD_PAGE_LIMIT = 100;

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
}

export interface RecentThreadFeedsState {
  gatewayScope: string;
  selectedFilter: RecentThreadFilter;
  feeds: Record<RecentThreadFilter, RecentThreadFeedState>;
  summariesById: Record<string, DesktopThreadSummary>;
  /** Session tombstones for successful or still-pending local archives. */
  removedThreadIds: Record<string, true>;
  nextRequestId: number;
}

interface RecentThreadRequestTicketBase {
  gatewayScope: string;
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
}

export interface RecentThreadLoadMoreTicket
  extends RecentThreadRequestTicketBase {
  kind: "loadMore";
}

export type RecentThreadRequestTicket =
  | RecentThreadRefreshTicket
  | RecentThreadLoadMoreTicket;

export interface RecentThreadRequestDecision<T extends RecentThreadRequestTicket> {
  state: RecentThreadFeedsState;
  ticket: T | null;
}

export interface RecentThreadRemovalRollback {
  gatewayScope: string;
  threadId: string;
  positions: Record<RecentThreadFilter, number>;
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
  };
}

export function createRecentThreadFeedsState(
  gatewayScope = "",
): RecentThreadFeedsState {
  return {
    gatewayScope,
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
): RecentThreadFeedsState {
  if (state.gatewayScope === gatewayScope) {
    return state;
  }
  return {
    gatewayScope,
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
): RecentThreadRequestDecision<RecentThreadRefreshTicket> {
  const feed = state.feeds[filter];
  if (!state.gatewayScope || feed.isRefreshingHead) {
    return { state, ticket: null };
  }
  const requestId = state.nextRequestId;
  const ticket: RecentThreadRefreshTicket = {
    gatewayScope: state.gatewayScope,
    filter,
    feedEpoch: feed.epoch,
    requestId,
    observedLocalMutationSequence: feed.localMutationSequence,
    observedLoadMoreFailureRevision: feed.loadMoreFailureRevision,
    kind: "refresh",
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
    feed.nextCursor === null ||
    !gateAllowsRequest
  ) {
    return { state, ticket: null };
  }
  const requestId = state.nextRequestId;
  const ticket: RecentThreadLoadMoreTicket = {
    gatewayScope: state.gatewayScope,
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

export function completeRecentThreadRequest(
  state: RecentThreadFeedsState,
  ticket: RecentThreadRequestTicket,
  page: DesktopRecentThreadsPage,
): RecentThreadFeedsState {
  if (
    state.gatewayScope !== ticket.gatewayScope ||
    page.gatewayScope !== ticket.gatewayScope
  ) {
    return clearOwnedRequest(state, ticket);
  }
  const feed = state.feeds[ticket.filter];
  if (!requestIsOwned(feed, ticket)) {
    return state;
  }
  if (feed.localMutationSequence !== ticket.observedLocalMutationSequence) {
    return markRecentThreadMutationFollowUp(
      clearOwnedRequest(state, ticket),
      ticket,
    );
  }

  // A request issued after optimistic removal but before the archive commits
  // may still observe the old server row. Keep a session tombstone after
  // success (scope reset clears it); rollback is the only path that restores
  // membership. This is mutation protection, not task-filter fallback.
  const visiblePageThreads = page.threads.filter(
    (thread) => !state.removedThreadIds[thread.id.trim()],
  );
  const pageIds = uniqueThreadIds(
    visiblePageThreads.map((thread) => thread.id),
  );
  const withSummaries = ingestRecentThreadSummaries(
    state,
    visiblePageThreads,
  );
  const currentFeed = withSummaries.feeds[ticket.filter];
  if (ticket.kind === "refresh") {
    const forgivesLoadFailure =
      currentFeed.loadGate === "failed" &&
      currentFeed.loadMoreFailureRevision ===
        ticket.observedLoadMoreFailureRevision;
    let loadGate = currentFeed.loadGate;
    if (currentFeed.loadGate === "failed" && !forgivesLoadFailure) {
      // A load-more that failed after this head request started owns the gate.
    } else {
      loadGate = page.hasMore ? "ready" : "exhausted";
    }
    return {
      ...withSummaries,
      feeds: {
        ...withSummaries.feeds,
        [ticket.filter]: {
          ...currentFeed,
          orderedThreadIds: pageIds,
          isPrimed: true,
          isRefreshingHead: false,
          headFailure: null,
          loadGate,
          nextCursor: page.nextCursor,
          activeRefreshRequestId: null,
        },
      },
    };
  }

  return {
    ...withSummaries,
    feeds: {
      ...withSummaries.feeds,
      [ticket.filter]: {
        ...currentFeed,
        orderedThreadIds: appendPage(
          pageIds,
          currentFeed.orderedThreadIds,
        ),
        isLoadingMore: false,
        loadGate: page.hasMore ? "ready" : "exhausted",
        nextCursor: page.nextCursor,
        activeLoadMoreRequestId: null,
      },
    },
  };
}

export function failRecentThreadRequest(
  state: RecentThreadFeedsState,
  ticket: RecentThreadRequestTicket,
  message: string,
): RecentThreadFeedsState {
  if (state.gatewayScope !== ticket.gatewayScope) {
    return state;
  }
  const feed = state.feeds[ticket.filter];
  if (!requestIsOwned(feed, ticket)) {
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
  const active =
    kind === "refresh" ? feed.isRefreshingHead : feed.isLoadingMore;
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
  feed: RecentThreadFeedState,
  ticket: RecentThreadRequestTicket,
): boolean {
  if (feed.epoch !== ticket.feedEpoch) {
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
  if (!requestIsOwned(feed, ticket)) {
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

function uniqueThreadIds(ids: string[]): string[] {
  const seen = new Set<string>();
  return ids.flatMap((id) => {
    const normalized = id.trim();
    return normalized && !seen.has(normalized) && seen.add(normalized)
      ? [normalized]
      : [];
  });
}

function appendPage(pageIds: string[], existingIds: string[]): string[] {
  const seen = new Set(existingIds);
  return [
    ...existingIds,
    ...pageIds.filter((id) => !seen.has(id) && Boolean(seen.add(id))),
  ];
}
