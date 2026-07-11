import type {
  DesktopRecentThreadsPage,
  DesktopThreadSummary,
  RecentThreadTaskFilter,
} from "@shared/contracts";

export type RecentThreadFilter = "all" | "nonTask";
export type RecentThreadRequestKind = "refresh" | "loadMore";
export type RecentThreadLoadGate = "ready" | "exhausted" | "failed";

export const RECENT_THREAD_PAGE_LIMIT = 100;
export const RECENT_THREAD_PAGE_OVERLAP = 5;

export interface RecentThreadFeedState {
  orderedThreadIds: string[];
  isPrimed: boolean;
  isRefreshingHead: boolean;
  isLoadingMore: boolean;
  headFailure: string | null;
  loadGate: RecentThreadLoadGate;
  nextOffset: number;
  epoch: number;
  localMutationSequence: number;
  loadMoreFailureRevision: number;
  activeRefreshRequestId: number | null;
  activeLoadMoreRequestId: number | null;
}

export interface RecentThreadFeedsState {
  gatewayScope: string;
  selectedFilter: RecentThreadFilter;
  feeds: Record<RecentThreadFilter, RecentThreadFeedState>;
  summariesById: Record<string, DesktopThreadSummary>;
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
  offset: number;
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
    nextOffset: 0,
    epoch,
    localMutationSequence: 0,
    loadMoreFailureRevision: 0,
    activeRefreshRequestId: null,
    activeLoadMoreRequestId: null,
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
    offset: 0,
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
    feed.nextOffset <= 0 ||
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
    offset: Math.max(0, feed.nextOffset - RECENT_THREAD_PAGE_OVERLAP),
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
    return clearOwnedRequest(state, ticket);
  }

  const pageIds = uniqueThreadIds(page.threads.map((thread) => thread.id));
  const withSummaries = ingestRecentThreadSummaries(state, page.threads);
  const currentFeed = withSummaries.feeds[ticket.filter];
  const returnedEnd = page.offset + page.count;

  if (ticket.kind === "refresh") {
    const beyondHead = currentFeed.nextOffset > returnedEnd;
    const orderedThreadIds = beyondHead
      ? mergeHead(pageIds, currentFeed.orderedThreadIds)
      : pageIds;
    const forgivesLoadFailure =
      currentFeed.loadGate === "failed" &&
      currentFeed.loadMoreFailureRevision ===
        ticket.observedLoadMoreFailureRevision;
    let loadGate = currentFeed.loadGate;
    let nextOffset = currentFeed.nextOffset;
    if (currentFeed.loadGate === "failed" && !forgivesLoadFailure) {
      // A load-more that failed after this head request started owns the gate.
    } else if (beyondHead) {
      if (forgivesLoadFailure) {
        loadGate = "ready";
      }
    } else {
      loadGate = page.hasMore ? "ready" : "exhausted";
      nextOffset = returnedEnd;
    }
    return {
      ...withSummaries,
      feeds: {
        ...withSummaries.feeds,
        [ticket.filter]: {
          ...currentFeed,
          orderedThreadIds,
          isPrimed: true,
          isRefreshingHead: false,
          headFailure: null,
          loadGate,
          nextOffset,
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
        nextOffset: returnedEnd,
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
    state: { ...state, feeds },
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
  return { ...state, feeds };
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

function uniqueThreadIds(ids: string[]): string[] {
  const seen = new Set<string>();
  return ids.flatMap((id) => {
    const normalized = id.trim();
    return normalized && !seen.has(normalized) && seen.add(normalized)
      ? [normalized]
      : [];
  });
}

function mergeHead(pageIds: string[], existingIds: string[]): string[] {
  const pageIdSet = new Set(pageIds);
  return [
    ...pageIds,
    ...existingIds.filter((id) => !pageIdSet.has(id)),
  ];
}

function appendPage(pageIds: string[], existingIds: string[]): string[] {
  const seen = new Set(existingIds);
  return [
    ...existingIds,
    ...pageIds.filter((id) => !seen.has(id) && Boolean(seen.add(id))),
  ];
}
