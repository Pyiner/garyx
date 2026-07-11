import {
  useRef,
  type ComponentProps,
  type KeyboardEvent,
  type ReactNode,
} from "react";

import type { RecentThreadFeedState, RecentThreadFilter } from "./app-shell/recent-thread-feeds";
import {
  ThreadConversationSidebar,
  type ThreadRailRow,
} from "./ThreadConversationSidebar";
import { useI18n } from "./i18n";
import {
  recentConversationPresentation,
  recentFilterForArrowKey,
  type RecentFeedFooterKind,
} from "./recent-conversation-sidebar-model";

type RecentConversationSidebarProps = {
  collapseLabel: string;
  feed: RecentThreadFeedState;
  formatThreadTimestamp: (value?: string | null) => string;
  logo: ReactNode;
  onClose: () => void;
  onLoadMore: () => void;
  onRailResizeStart?: ComponentProps<
    typeof ThreadConversationSidebar
  >["onRailResizeStart"];
  onRetry: () => void;
  onSelectFilter: (filter: RecentThreadFilter) => void;
  railResizing?: boolean;
  rows: ThreadRailRow[];
  selectedFilter: RecentThreadFilter;
};

const FILTERS: RecentThreadFilter[] = ["all", "nonTask"];

export function RecentConversationSidebar({
  collapseLabel,
  feed,
  formatThreadTimestamp,
  logo,
  onClose,
  onLoadMore,
  onRailResizeStart,
  onRetry,
  onSelectFilter,
  railResizing,
  rows,
  selectedFilter,
}: RecentConversationSidebarProps) {
  const { t } = useI18n();
  const tabRefs = useRef<Record<RecentThreadFilter, HTMLButtonElement | null>>({
    all: null,
    nonTask: null,
  });
  const presentation = recentConversationPresentation(
    feed,
    rows.length,
    selectedFilter,
  );
  const emptyLabel = presentation.emptyLabelKey
    ? t(presentation.emptyLabelKey)
    : undefined;
  const listFooter = recentFeedFooter({
    kind: presentation.footerKind,
    onRetry,
    t,
  });

  function handleTabKeyDown(
    event: KeyboardEvent<HTMLButtonElement>,
    filter: RecentThreadFilter,
  ) {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") {
      return;
    }
    event.preventDefault();
    const nextFilter = recentFilterForArrowKey(filter, event.key);
    onSelectFilter(nextFilter);
    tabRefs.current[nextFilter]?.focus();
  }

  return (
    <ThreadConversationSidebar
      ariaLabel={t("Recent threads")}
      className="recent-conversation-rail"
      collapseLabel={collapseLabel}
      emptyLabel={emptyLabel}
      formatThreadTimestamp={formatThreadTimestamp}
      headerAccessory={
        <div
          aria-label={t("Recent filter")}
          className="recent-filter-tabs"
          role="tablist"
        >
          {FILTERS.map((filter) => {
            const label = filter === "all" ? t("All") : t("Chats");
            const selected = filter === selectedFilter;
            return (
              <button
                aria-selected={selected}
                className={selected ? "active" : undefined}
                key={filter}
                onClick={() => onSelectFilter(filter)}
                onKeyDown={(event) => handleTabKeyDown(event, filter)}
                ref={(node) => {
                  tabRefs.current[filter] = node;
                }}
                role="tab"
                tabIndex={selected ? 0 : -1}
                type="button"
              >
                {label}
              </button>
            );
          })}
        </div>
      }
      listFooter={listFooter}
      logo={logo}
      onClose={onClose}
      onNearListEnd={feed.loadGate === "ready" ? onLoadMore : undefined}
      onRailResizeStart={onRailResizeStart}
      railResizing={railResizing}
      rowClassName="recent-conversation-row-shell"
      rows={rows}
      title={t("Recent")}
    />
  );
}

function recentFeedFooter({
  kind,
  onRetry,
  t,
}: {
  kind: RecentFeedFooterKind;
  onRetry: () => void;
  t: (key: string) => string;
}): ReactNode {
  if (kind === "initialLoading") {
    return (
      <div aria-label={t("Loading recent threads")} className="recent-feed-skeleton" role="status">
        <span />
        <span />
        <span />
      </div>
    );
  }
  if (kind === "initialFailure" || kind === "cachedRefreshFailure") {
    return (
      <button className="recent-feed-footer failed" onClick={onRetry} type="button">
        {kind === "cachedRefreshFailure"
          ? t("Couldn't refresh · Retry")
          : t("Recent threads unavailable · Retry")}
      </button>
    );
  }
  if (kind === "loadingMore") {
    return (
      <div className="recent-feed-footer loading" role="status">
        <span aria-hidden className="recent-feed-spinner" />
        {t("Loading more")}
      </div>
    );
  }
  if (kind === "loadMoreFailure") {
    return (
      <button className="recent-feed-footer failed" onClick={onRetry} type="button">
        {t("Couldn't load more · Retry")}
      </button>
    );
  }
  if (kind === "idle") {
    return <div aria-hidden className="recent-feed-footer idle" />;
  }
  return null;
}
