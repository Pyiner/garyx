import { useCallback, useEffect, useRef, useState } from "react";

import type {
  DesktopRecentThreadsPage,
  DesktopThreadSummary,
} from "@shared/contracts";

import {
  completeRecentThreadLoadMore,
  completeRecentThreadRefresh,
  consumeRecentThreadMutationFollowUp,
  createRecentThreadFeedsState,
  failRecentThreadRequest,
  ingestRecentThreadSummaries,
  isPaginatedRecentThreadFilter,
  markRecentThreadForceReplacement,
  noteRecentThreadFilterLocalMutation,
  noteRecentThreadLocalMutation,
  recentPageHeadActivitySeq,
  recentRefreshChainNeedsNextPage,
  recentThreadTasksQuery,
  removeThreadFromRecentFeeds,
  requestRecentThreadLoadMore,
  requestRecentThreadRefresh,
  resetRecentThreadFeedsScope,
  rollbackRecentThreadRemoval,
  selectRecentThreadFilter,
  selectedRecentThreadFeed,
  selectedRecentThreadSummaries,
  upsertChatInRecentFeeds,
  verificationObservedNewerHead,
  PAGINATED_RECENT_THREAD_FILTERS,
  type PaginatedRecentThreadFilter,
  type RecentThreadFilter,
  type RecentThreadFeedsState,
  type RecentThreadLoadMoreTicket,
  type RecentThreadRefreshMode,
  type RecentThreadRefreshTicket,
  type RecentThreadRemovalRollback,
  type RecentThreadRequestTicket,
} from "./recent-thread-feeds";
import type {
  StoreIdentityDecision,
  StoreResponseStamp,
} from "./favorites-ingress";

type RecentThreadFeedsControllerOptions = {
  enabled: boolean;
  gatewayScope: string;
  runtimeEpoch: number;
  sharedSummaries: DesktopThreadSummary[];
  observeStoreResponse: (
    stamp: StoreResponseStamp,
    storeIncarnationId: string,
  ) => StoreIdentityDecision;
};

export type RecentThreadFeedsController = {
  state: RecentThreadFeedsState;
  selectedFeed: ReturnType<typeof selectedRecentThreadFeed>;
  selectedThreads: DesktopThreadSummary[];
  selectFilter: (filter: RecentThreadFilter) => void;
  refreshSelected: () => void;
  refreshAll: () => void;
  forceReplacement: () => void;
  loadMore: () => void;
  retry: () => void;
  removeThread: (threadId: string) => RecentThreadRemovalRollback;
  rollbackRemoval: (rollback: RecentThreadRemovalRollback) => void;
  upsertChat: (summary: DesktopThreadSummary) => void;
  noteAllLocalMutation: () => void;
  noteLocalMutation: () => void;
};

class RecentIdentityInterrupted extends Error {
  constructor(readonly decision: StoreIdentityDecision) {
    super(`Recent request interrupted by ${decision}`);
  }
}

export function useRecentThreadFeeds({
  enabled,
  gatewayScope,
  runtimeEpoch,
  sharedSummaries,
  observeStoreResponse,
}: RecentThreadFeedsControllerOptions): RecentThreadFeedsController {
  const [state, setState] = useState(() =>
    createRecentThreadFeedsState(gatewayScope, runtimeEpoch),
  );
  const stateRef = useRef(state);
  const runtimeEpochRef = useRef(runtimeEpoch);
  runtimeEpochRef.current = runtimeEpoch;
  const queuedRefreshesRef = useRef(new Set<RecentThreadFilter>());

  const commit = useCallback(
    (update: (current: RecentThreadFeedsState) => RecentThreadFeedsState) => {
      const next = update(stateRef.current);
      stateRef.current = next;
      setState(next);
      return next;
    },
    [],
  );

  const ticketIsOwned = useCallback((ticket: RecentThreadRequestTicket) => {
    const current = stateRef.current;
    const feed = current.feeds[ticket.filter];
    return (
      current.gatewayScope === ticket.gatewayScope &&
      current.runtimeEpoch === ticket.runtimeEpoch &&
      feed.epoch === ticket.feedEpoch &&
      (ticket.kind === "refresh"
        ? feed.activeRefreshRequestId === ticket.requestId
        : feed.activeLoadMoreRequestId === ticket.requestId)
    );
  }, []);

  const acceptIdentity = useCallback(
    (ticket: RecentThreadRequestTicket, page: DesktopRecentThreadsPage) => {
      const decision = observeStoreResponse(
        {
          gatewayScope: ticket.gatewayScope,
          runtimeEpoch: ticket.runtimeEpoch,
          owned: ticketIsOwned(ticket),
        },
        page.storeIncarnationId,
      );
      if (decision !== "accept") {
        if (decision === "scopeClear") {
          commit((current) =>
            resetRecentThreadFeedsScope(
              current,
              current.gatewayScope,
              ticket.runtimeEpoch + 1,
            ),
          );
        }
        throw new RecentIdentityInterrupted(decision);
      }
    },
    [commit, observeStoreResponse, ticketIsOwned],
  );

  const fetchPage = useCallback(
    async (
      ticket: RecentThreadRequestTicket,
      cursor: string | null,
    ): Promise<DesktopRecentThreadsPage> => {
      const page = await window.garyxDesktop.listRecentThreads({
        gatewayScope: ticket.gatewayScope,
        tasks: recentThreadTasksQuery(ticket.filter),
        limit: ticket.limit,
        cursor,
      });
      acceptIdentity(ticket, page);
      return page;
    },
    [acceptIdentity],
  );

  const fetchChain = useCallback(
    async (
      ticket: RecentThreadRefreshTicket,
      mode: RecentThreadRefreshMode,
      oldHeadActivitySeq: number | null,
    ) => {
      const pages: DesktopRecentThreadsPage[] = [];
      let cursor: string | null = null;
      do {
        const page = await fetchPage(ticket, cursor);
        pages.push(page);
        cursor = page.nextCursor;
      } while (
        recentRefreshChainNeedsNextPage(mode, oldHeadActivitySeq, pages)
      );
      return pages;
    },
    [fetchPage],
  );

  const executeRefresh = useCallback(
    async (ticket: RecentThreadRefreshTicket) => {
      try {
        const primaryPages = await fetchChain(
          ticket,
          ticket.mode,
          ticket.oldHeadActivitySeq,
        );
        const verificationPage = await fetchPage(ticket, null);
        const primaryHead = recentPageHeadActivitySeq(primaryPages[0]);
        let immediatePages: DesktopRecentThreadsPage[] | undefined;
        let immediateVerificationPage: DesktopRecentThreadsPage | undefined;
        if (verificationObservedNewerHead(primaryHead, verificationPage)) {
          immediatePages = await fetchChain(
            ticket,
            "rangeFill",
            primaryHead,
          );
          immediateVerificationPage = await fetchPage(ticket, null);
        }
        const completion = completeRecentThreadRefresh(
          stateRef.current,
          ticket,
          {
            primaryPages,
            verificationPage,
            immediatePages,
            immediateVerificationPage,
          },
        );
        commit(() => completion.state);
        if (completion.action === "forceReplacement") {
          queuedRefreshesRef.current.add(ticket.filter);
        }
      } catch (error) {
        if (error instanceof RecentIdentityInterrupted) {
          return;
        }
        commit((current) =>
          failRecentThreadRequest(
            current,
            ticket,
            error instanceof Error
              ? error.message
              : "Recent threads are unavailable",
          ),
        );
      }
    },
    [commit, fetchChain, fetchPage],
  );

  const executeLoadMore = useCallback(
    async (ticket: RecentThreadLoadMoreTicket) => {
      try {
        const page = await fetchPage(ticket, ticket.cursor);
        const completion = completeRecentThreadLoadMore(
          stateRef.current,
          ticket,
          page,
        );
        commit(() => completion.state);
        if (completion.action === "forceReplacement") {
          queuedRefreshesRef.current.add(ticket.filter);
        }
      } catch (error) {
        if (error instanceof RecentIdentityInterrupted) {
          return;
        }
        commit((current) =>
          failRecentThreadRequest(current, ticket, "Recent threads are unavailable"),
        );
      }
    },
    [commit, fetchPage],
  );

  const execute = useCallback(
    (ticket: RecentThreadRequestTicket) => {
      if (ticket.kind === "refresh") {
        void executeRefresh(ticket);
      } else {
        void executeLoadMore(ticket);
      }
    },
    [executeLoadMore, executeRefresh],
  );

  const issueRefresh = useCallback(
    (filter: PaginatedRecentThreadFilter, forceReplacement = false) => {
      const decision = requestRecentThreadRefresh(
        stateRef.current,
        filter,
        { forceReplacement },
      );
      if (!decision.ticket) {
        return;
      }
      stateRef.current = decision.state;
      setState(decision.state);
      execute(decision.ticket);
    },
    [execute],
  );

  const issueLoadMore = useCallback(
    (retry: boolean) => {
      const current = stateRef.current;
      if (!isPaginatedRecentThreadFilter(current.selectedFilter)) {
        return;
      }
      const decision = requestRecentThreadLoadMore(
        current,
        current.selectedFilter,
        retry,
      );
      if (!decision.ticket) {
        return;
      }
      stateRef.current = decision.state;
      setState(decision.state);
      execute(decision.ticket);
    },
    [execute],
  );

  useEffect(() => {
    queuedRefreshesRef.current.clear();
    commit((current) =>
      resetRecentThreadFeedsScope(current, gatewayScope, runtimeEpoch),
    );
  }, [commit, gatewayScope, runtimeEpoch]);

  useEffect(() => {
    commit((current) =>
      ingestRecentThreadSummaries(current, sharedSummaries),
    );
  }, [commit, sharedSummaries]);

  useEffect(() => {
    if (!enabled || !gatewayScope) {
      return;
    }
    const filter = stateRef.current.selectedFilter;
    if (isPaginatedRecentThreadFilter(filter)) {
      issueRefresh(filter);
    }
  }, [enabled, gatewayScope, issueRefresh, runtimeEpoch]);

  useEffect(() => {
    if (!enabled || !gatewayScope) {
      return;
    }
    const interval = window.setInterval(() => {
      const filter = stateRef.current.selectedFilter;
      if (isPaginatedRecentThreadFilter(filter)) {
        issueRefresh(filter);
      }
    }, 10_000);
    return () => window.clearInterval(interval);
  }, [enabled, gatewayScope, issueRefresh]);

  useEffect(() => {
    const onVisibilityChange = () => {
      if (document.visibilityState === "visible" && enabled && gatewayScope) {
        const filter = stateRef.current.selectedFilter;
        if (isPaginatedRecentThreadFilter(filter)) {
          issueRefresh(filter, true);
        }
      }
    };
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => document.removeEventListener("visibilitychange", onVisibilityChange);
  }, [enabled, gatewayScope, issueRefresh]);

  useEffect(() => {
    for (const filter of PAGINATED_RECENT_THREAD_FILTERS) {
      if (
        queuedRefreshesRef.current.has(filter) &&
        !state.feeds[filter].isRefreshingHead &&
        !state.feeds[filter].isLoadingMore
      ) {
        queuedRefreshesRef.current.delete(filter);
        issueRefresh(filter);
      }
    }
  }, [issueRefresh, state]);

  useEffect(() => {
    let next = stateRef.current;
    const tickets: RecentThreadRequestTicket[] = [];
    for (const filter of PAGINATED_RECENT_THREAD_FILTERS) {
      for (const kind of ["refresh", "loadMore"] as const) {
        const decision = consumeRecentThreadMutationFollowUp(
          next,
          filter,
          kind,
        );
        next = decision.state;
        if (decision.ticket) {
          tickets.push(decision.ticket);
        }
      }
    }
    if (next !== stateRef.current) {
      stateRef.current = next;
      setState(next);
    }
    for (const ticket of tickets) {
      execute(ticket);
    }
  }, [execute, state]);

  const selectFilter = useCallback(
    (filter: RecentThreadFilter) => {
      commit((current) => selectRecentThreadFilter(current, filter));
      if (isPaginatedRecentThreadFilter(filter)) {
        issueRefresh(filter);
      }
    },
    [commit, issueRefresh],
  );

  const refreshSelected = useCallback(() => {
    const filter = stateRef.current.selectedFilter;
    if (isPaginatedRecentThreadFilter(filter)) {
      issueRefresh(filter, true);
    }
  }, [issueRefresh]);

  const refreshAll = useCallback(() => {
    if (
      stateRef.current.feeds.all.isRefreshingHead ||
      stateRef.current.feeds.all.isLoadingMore
    ) {
      queuedRefreshesRef.current.add("all");
      return;
    }
    issueRefresh("all");
  }, [issueRefresh]);

  const forceReplacement = useCallback(() => {
    commit(markRecentThreadForceReplacement);
    for (const filter of PAGINATED_RECENT_THREAD_FILTERS) {
      if (
        stateRef.current.feeds[filter].isRefreshingHead ||
        stateRef.current.feeds[filter].isLoadingMore
      ) {
        queuedRefreshesRef.current.add(filter);
      } else {
        issueRefresh(filter, true);
      }
    }
  }, [commit, issueRefresh]);

  const loadMore = useCallback(() => issueLoadMore(false), [issueLoadMore]);

  const retry = useCallback(() => {
    const current = stateRef.current;
    const feed = selectedRecentThreadFeed(current);
    if (!feed || !isPaginatedRecentThreadFilter(current.selectedFilter)) {
      return;
    }
    if (feed.headFailure || feed.forceReplacementPending) {
      issueRefresh(current.selectedFilter, feed.forceReplacementPending);
      return;
    }
    issueLoadMore(true);
  }, [issueLoadMore, issueRefresh]);

  const removeThread = useCallback((threadId: string) => {
    const result = removeThreadFromRecentFeeds(stateRef.current, threadId);
    stateRef.current = result.state;
    setState(result.state);
    return result.rollback;
  }, []);

  const rollbackRemoval = useCallback(
    (rollback: RecentThreadRemovalRollback) => {
      commit((current) => rollbackRecentThreadRemoval(current, rollback));
    },
    [commit],
  );

  const upsertChat = useCallback(
    (summary: DesktopThreadSummary) => {
      commit((current) => upsertChatInRecentFeeds(current, summary));
    },
    [commit],
  );

  const noteLocalMutation = useCallback(() => {
    commit(noteRecentThreadLocalMutation);
  }, [commit]);

  const noteAllLocalMutation = useCallback(() => {
    commit((current) =>
      noteRecentThreadFilterLocalMutation(current, "all"),
    );
  }, [commit]);

  const visibleState =
    state.gatewayScope === gatewayScope && state.runtimeEpoch === runtimeEpoch
      ? state
      : resetRecentThreadFeedsScope(state, gatewayScope, runtimeEpoch);
  if (
    stateRef.current.gatewayScope !== gatewayScope ||
    stateRef.current.runtimeEpoch !== runtimeEpoch
  ) {
    stateRef.current = visibleState;
  }

  return {
    state: visibleState,
    selectedFeed: selectedRecentThreadFeed(visibleState),
    selectedThreads: selectedRecentThreadSummaries(visibleState),
    selectFilter,
    refreshSelected,
    refreshAll,
    forceReplacement,
    loadMore,
    retry,
    removeThread,
    rollbackRemoval,
    upsertChat,
    noteAllLocalMutation,
    noteLocalMutation,
  };
}
