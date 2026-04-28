import type { RefObject } from 'react';

import { Button } from '@/components/ui/button';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';

import type { ClientLogEntry, ThreadLogLine, ThreadLogTab } from '../types';

type ThreadLogPanelProps = {
  activeThreadTitle: string | null;
  selectedThreadId: string | null;
  activeThreadLogsPath: string;
  activeThreadLogsHasUnread: boolean;
  threadLogsActiveTab: ThreadLogTab;
  threadLogsError: string | null;
  threadLogsLoading: boolean;
  clientThreadLogEntries: ClientLogEntry[];
  mobileThreadLogLines: ThreadLogLine[];
  expandedClientLogEntries: Record<string, boolean>;
  threadLogsRef: RefObject<HTMLDivElement | null>;
  onJumpToLatest: () => void;
  onSelectTab: (tab: ThreadLogTab) => void;
  onContentScroll: () => void;
  onToggleClientLogEntry: (entryKey: string) => void;
};

export function ThreadLogPanel({
  activeThreadTitle,
  selectedThreadId,
  activeThreadLogsPath,
  activeThreadLogsHasUnread,
  threadLogsActiveTab,
  threadLogsError,
  threadLogsLoading,
  clientThreadLogEntries,
  mobileThreadLogLines,
  expandedClientLogEntries,
  threadLogsRef,
  onJumpToLatest,
  onSelectTab,
  onContentScroll,
  onToggleClientLogEntry,
}: ThreadLogPanelProps) {
  const threadLogsLabel = activeThreadTitle || selectedThreadId || 'Current thread logs';

  return (
    <aside
      aria-label={threadLogsLabel}
      className="thread-log-panel"
      title={activeThreadLogsPath}
    >
      <div className="thread-log-panel-toolbar">
        <ToggleGroup
          aria-label="Log sources"
          className="thread-log-panel-tabs"
          onValueChange={(value) => {
            if (value === 'client' || value === 'mobile') {
              onSelectTab(value);
            }
          }}
          size="sm"
          spacing={0}
          type="single"
          value={threadLogsActiveTab}
          variant="outline"
        >
          <ToggleGroupItem value="client">Client Logs</ToggleGroupItem>
          <ToggleGroupItem value="mobile">Mobile Logs</ToggleGroupItem>
        </ToggleGroup>
        <div className="thread-log-panel-actions">
          {activeThreadLogsHasUnread ? (
            <Button
              className="thread-log-panel-latest"
              onClick={onJumpToLatest}
              size="xs"
              type="button"
              variant="ghost"
            >
              Latest
            </Button>
          ) : null}
        </div>
      </div>

      {threadLogsActiveTab === 'mobile' && threadLogsError ? (
        <div className="thread-log-panel-error">{threadLogsError}</div>
      ) : null}

      <div
        className="thread-log-panel-content"
        onScroll={onContentScroll}
        ref={threadLogsRef}
      >
        {threadLogsActiveTab === 'client' ? (
          clientThreadLogEntries.length ? (
            <div className="thread-log-client-list">
              {clientThreadLogEntries.map((entry) => {
                const expanded = Boolean(expandedClientLogEntries[entry.key]);
                return (
                  <div
                    className={`thread-log-client-entry ${entry.level === 'error' ? 'thread-log-client-entry-error' : ''}`}
                    key={entry.key}
                  >
                    <button
                      aria-expanded={expanded}
                      className="thread-log-client-entry-toggle"
                      onClick={() => {
                        onToggleClientLogEntry(entry.key);
                      }}
                      type="button"
                    >
                      <span className={`thread-log-client-entry-type type-${entry.eventType.replace(/_/g, '-')}`}>
                        {entry.eventType}
                      </span>
                      <span className="thread-log-client-entry-summary" title={entry.summary}>
                        {entry.summary || '\u00A0'}
                      </span>
                      <span className="thread-log-client-entry-caret">{expanded ? 'Hide' : 'Show'}</span>
                    </button>
                    {expanded ? (
                      <pre className="thread-log-client-entry-detail">{entry.detail}</pre>
                    ) : null}
                  </div>
                );
              })}
            </div>
          ) : (
            <div className="thread-log-panel-empty">No client stream events yet.</div>
          )
        ) : (
          mobileThreadLogLines.length ? (
            mobileThreadLogLines.map((line) => (
              <div
                className={`thread-log-line ${line.level === 'error' ? 'thread-log-line-error' : ''}`}
                key={line.key}
              >
                <span className="thread-log-line-text">{line.text || '\u00A0'}</span>
              </div>
            ))
          ) : (
            <div className="thread-log-panel-empty">
              {threadLogsLoading ? 'Loading logs…' : 'No logs yet.'}
            </div>
          )
        )}
      </div>
    </aside>
  );
}
