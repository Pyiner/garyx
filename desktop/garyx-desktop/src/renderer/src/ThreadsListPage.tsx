import type { DesktopThreadSummary } from '@shared/contracts';

import {
  UIButton,
  UIBadge,
  UICard,
  UICardContent,
} from './ui';

type ThreadsListPageProps = {
  threads: DesktopThreadSummary[];
  loading: boolean;
  error?: string | null;
  totalThreadsCount: number;
  onOpenThread?: (threadId: string) => void;
  onRefresh?: () => void;
};

function formatTimestamp(value?: string | null): string {
  if (!value) {
    return '';
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleString(undefined, { hour12: false });
}

export function ThreadsListPage({
  threads,
  loading,
  error,
  totalThreadsCount,
  onOpenThread,
  onRefresh,
}: ThreadsListPageProps) {
  return (
    <div className="thread-history-shell">
      <div className="thread-history-page shadcn-shell">
        <section className="thread-history-compact-header ui-card">
          <div className="thread-history-compact-copy">
            <p className="shadcn-kicker">Threads</p>
            <h1 className="thread-history-title">Thread overview</h1>
            <p className="thread-history-inline-meta">
              Showing {threads.length} · total {totalThreadsCount}
            </p>
          </div>
          <div className="thread-history-compact-actions">
            {onRefresh ? (
              <UIButton onClick={onRefresh} size="sm" type="button" variant="outline">
                Refresh
              </UIButton>
            ) : null}
          </div>
        </section>

        {error ? (
          <div className="bot-console-error" role="alert">
            {error}
          </div>
        ) : null}

        {loading ? (
          <div className="empty-state">
            <span className="eyebrow">Threads</span>
            <h3>Loading threads</h3>
          </div>
        ) : !threads.length ? (
          <div className="empty-state">
            <span className="eyebrow">Threads</span>
            <h3>No threads found</h3>
            <p>No threads are available.</p>
          </div>
        ) : (
          <div className="thread-overview-grid">
            {threads.map((thread) => (
              <UICard
                className={`thread-overview-card${onOpenThread ? '' : ' thread-overview-card-static'}`}
                key={thread.id}
                onClick={onOpenThread ? () => onOpenThread(thread.id) : undefined}
                role={onOpenThread ? 'button' : undefined}
                tabIndex={onOpenThread ? 0 : undefined}
                onKeyDown={(event) => {
                  if (!onOpenThread) {
                    return;
                  }
                  if (event.key === 'Enter' || event.key === ' ') {
                    event.preventDefault();
                    onOpenThread(thread.id);
                  }
                }}
              >
                <div className="thread-overview-card-head">
                  <strong className="thread-overview-title" title={thread.title || thread.id}>
                    {thread.title || thread.id}
                  </strong>
                  <UIBadge className="bot-console-status status-connected">Thread</UIBadge>
                </div>
                <UICardContent className="thread-overview-card-body">
                  <div className="small-note">
                    <code>{thread.id}</code>
                  </div>
                  <div className="small-note">
                    {thread.messageCount || 0} messages
                    {thread.workspacePath ? ` · ${thread.workspacePath}` : ''}
                  </div>
                  <div className="small-note">
                    Updated {formatTimestamp(thread.updatedAt)}
                  </div>
                  <div className="thread-overview-preview">
                    {thread.lastMessagePreview || 'No preview available'}
                  </div>
                </UICardContent>
              </UICard>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
