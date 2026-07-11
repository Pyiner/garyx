import { useCallback, useEffect, useRef, useState } from "react";

import type { DesktopThreadSummary } from "@shared/contracts";

import {
  completeRecentThreadRequest,
  consumeRecentThreadMutationFollowUp,
  createRecentThreadFeedsState,
  failRecentThreadRequest,
  ingestRecentThreadSummaries,
  noteRecentThreadFilterLocalMutation,
  noteRecentThreadLocalMutation,
  removeThreadFromRecentFeeds,
  requestRecentThreadLoadMore,
  requestRecentThreadRefresh,
  resetRecentThreadFeedsScope,
  rollbackRecentThreadRemoval,
  recentThreadTasksQuery,
  selectRecentThreadFilter,
  selectedRecentThreadFeed,
  selectedRecentThreadSummaries,
  upsertChatInRecentFeeds,
  type RecentThreadFilter,
  type RecentThreadFeedsState,
  type RecentThreadRemovalRollback,
  type RecentThreadRequestTicket,
} from "./recent-thread-feeds";

type RecentThreadFeedsControllerOptions = {
  enabled: boolean;
  gatewayScope: string;
  sharedSummaries: DesktopThreadSummary[];
};

export type RecentThreadFeedsController = {
  state: RecentThreadFeedsState;
  selectedFeed: ReturnType<typeof selectedRecentThreadFeed>;
  selectedThreads: DesktopThreadSummary[];
  selectFilter: (filter: RecentThreadFilter) => void;
  refreshSelected: () => void;
  refreshAll: () => void;
  loadMore: () => void;
  retry: () => void;
  removeThread: (threadId: string) => RecentThreadRemovalRollback;
  rollbackRemoval: (rollback: RecentThreadRemovalRollback) => void;
  upsertChat: (summary: DesktopThreadSummary) => void;
  noteAllLocalMutation: () => void;
  noteLocalMutation: () => void;
};

const FILTERS: RecentThreadFilter[] = ["all", "nonTask"];

export function useRecentThreadFeeds({
  enabled,
  gatewayScope,
  sharedSummaries,
}: RecentThreadFeedsControllerOptions): RecentThreadFeedsController {
  const [state, setState] = useState(() =>
    createRecentThreadFeedsState(gatewayScope),
  );
  const stateRef = useRef(state);
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

  const execute = useCallback(
    async (ticket: RecentThreadRequestTicket) => {
      try {
        const page = await window.garyxDesktop.listRecentThreads({
          gatewayScope: ticket.gatewayScope,
          tasks: recentThreadTasksQuery(ticket.filter),
          limit: ticket.limit,
          offset: ticket.offset,
        });
        commit((current) =>
          completeRecentThreadRequest(current, ticket, page),
        );
      } catch (error) {
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
    [commit],
  );

  const issueRefresh = useCallback(
    (filter: RecentThreadFilter) => {
      const decision = requestRecentThreadRefresh(stateRef.current, filter);
      if (!decision.ticket) {
        return;
      }
      stateRef.current = decision.state;
      setState(decision.state);
      void execute(decision.ticket);
    },
    [execute],
  );

  const issueLoadMore = useCallback(
    (retry: boolean) => {
      const current = stateRef.current;
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
      void execute(decision.ticket);
    },
    [execute],
  );

  useEffect(() => {
    queuedRefreshesRef.current.clear();
    commit((current) => resetRecentThreadFeedsScope(current, gatewayScope));
  }, [commit, gatewayScope]);

  useEffect(() => {
    commit((current) =>
      ingestRecentThreadSummaries(current, sharedSummaries),
    );
  }, [commit, sharedSummaries]);

  useEffect(() => {
    if (!enabled || !gatewayScope) {
      return;
    }
    issueRefresh(stateRef.current.selectedFilter);
  }, [enabled, gatewayScope, issueRefresh]);

  useEffect(() => {
    for (const filter of FILTERS) {
      if (
        queuedRefreshesRef.current.has(filter) &&
        !state.feeds[filter].isRefreshingHead
      ) {
        queuedRefreshesRef.current.delete(filter);
        issueRefresh(filter);
      }
    }
  }, [issueRefresh, state]);

  useEffect(() => {
    let next = stateRef.current;
    const tickets: RecentThreadRequestTicket[] = [];
    for (const filter of FILTERS) {
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
      void execute(ticket);
    }
  }, [execute, state]);

  const selectFilter = useCallback(
    (filter: RecentThreadFilter) => {
      commit((current) => selectRecentThreadFilter(current, filter));
      issueRefresh(filter);
    },
    [commit, issueRefresh],
  );

  const refreshSelected = useCallback(() => {
    issueRefresh(stateRef.current.selectedFilter);
  }, [issueRefresh]);

  const refreshAll = useCallback(() => {
    if (stateRef.current.feeds.all.isRefreshingHead) {
      // A task may be created after the active All request captured its
      // server snapshot. Queue one follow-up head request instead of letting
      // coalescing make that task invisible until the rail is reopened.
      queuedRefreshesRef.current.add("all");
      return;
    }
    issueRefresh("all");
  }, [issueRefresh]);

  const loadMore = useCallback(() => {
    issueLoadMore(false);
  }, [issueLoadMore]);

  const retry = useCallback(() => {
    const current = stateRef.current;
    const feed = selectedRecentThreadFeed(current);
    if (feed.headFailure) {
      issueRefresh(current.selectedFilter);
      return;
    }
    issueLoadMore(true);
  }, [issueLoadMore, issueRefresh]);

  const removeThread = useCallback(
    (threadId: string) => {
      const result = removeThreadFromRecentFeeds(
        stateRef.current,
        threadId,
      );
      stateRef.current = result.state;
      setState(result.state);
      return result.rollback;
    },
    [],
  );

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

  // Effects perform the owned reset, but the render projection masks the old
  // gateway synchronously so one frame can never expose a previous scope.
  const visibleState =
    state.gatewayScope === gatewayScope
      ? state
      : resetRecentThreadFeedsScope(state, gatewayScope);
  if (stateRef.current.gatewayScope !== gatewayScope) {
    stateRef.current = visibleState;
  }

  return {
    state: visibleState,
    selectedFeed: selectedRecentThreadFeed(visibleState),
    selectedThreads: selectedRecentThreadSummaries(visibleState),
    selectFilter,
    refreshSelected,
    refreshAll,
    loadMore,
    retry,
    removeThread,
    rollbackRemoval,
    upsertChat,
    noteAllLocalMutation,
    noteLocalMutation,
  };
}
