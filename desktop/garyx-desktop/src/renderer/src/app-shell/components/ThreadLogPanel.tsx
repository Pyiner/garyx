import type { RefObject } from 'react';

import { Button } from '@/components/ui/button';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';

import type { ClientLogEntry, ThreadLogLine, ThreadLogTab } from '../types';
import { useI18n } from '../../i18n';
import type { RendererPerformanceSnapshot } from '../../perf-metrics';

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
  performanceSnapshot: RendererPerformanceSnapshot;
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
  performanceSnapshot,
  expandedClientLogEntries,
  threadLogsRef,
  onJumpToLatest,
  onSelectTab,
  onContentScroll,
  onToggleClientLogEntry,
}: ThreadLogPanelProps) {
  const { t } = useI18n();
  const threadLogsLabel = activeThreadTitle || selectedThreadId || t('Current thread logs');

  return (
    <aside
      aria-label={threadLogsLabel}
      className="thread-log-panel"
      title={activeThreadLogsPath}
    >
      <div className="thread-log-panel-toolbar">
        <ToggleGroup
          aria-label={t('Log sources')}
          className="thread-log-panel-tabs"
          onValueChange={(value) => {
            if (value === 'client' || value === 'mobile' || value === 'performance') {
              onSelectTab(value);
            }
          }}
          size="sm"
          spacing={0}
          type="single"
          value={threadLogsActiveTab}
          variant="outline"
        >
          <ToggleGroupItem value="client">{t('Client Logs')}</ToggleGroupItem>
          <ToggleGroupItem value="mobile">{t('Gateway Logs')}</ToggleGroupItem>
          <ToggleGroupItem value="performance">{t('Performance')}</ToggleGroupItem>
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
              {t('Latest')}
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
                      <span className="thread-log-client-entry-caret">{expanded ? t('Hide') : t('Show')}</span>
                    </button>
                    {expanded ? (
                      <pre className="thread-log-client-entry-detail">{entry.detail}</pre>
                    ) : null}
                  </div>
                );
              })}
            </div>
          ) : (
            <div className="thread-log-panel-empty">{t('No client stream events yet.')}</div>
          )
        ) : threadLogsActiveTab === 'performance' ? (
          <PerformanceLogView snapshot={performanceSnapshot} t={t} />
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
              {threadLogsLoading ? t('Loading logs…') : t('No logs yet.')}
            </div>
          )
        )}
      </div>
    </aside>
  );
}

function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) {
    return '0 MB';
  }
  const mib = value / 1024 / 1024;
  if (mib < 1024) {
    return `${mib.toFixed(mib >= 100 ? 0 : 1)} MB`;
  }
  return `${(mib / 1024).toFixed(2)} GB`;
}

function formatDuration(value: number | undefined): string {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return '';
  }
  return `${value.toFixed(value >= 100 ? 0 : 1)}ms`;
}

function formatClock(value: number): string {
  const date = new Date(value);
  const hours = String(date.getHours()).padStart(2, '0');
  const minutes = String(date.getMinutes()).padStart(2, '0');
  const seconds = String(date.getSeconds()).padStart(2, '0');
  return `${hours}:${minutes}:${seconds}`;
}

function PerformanceLogView({
  snapshot,
  t,
}: {
  snapshot: RendererPerformanceSnapshot;
  t: (value: string, params?: Record<string, string | number>) => string;
}) {
  const memory = snapshot.latestMemory;
  const memoryRatio = memory && memory.jsHeapSizeLimit > 0
    ? memory.usedJSHeapSize / memory.jsHeapSizeLimit
    : null;
  const memoryPercent = memoryRatio === null
    ? null
    : `${Math.round(memoryRatio * 100)}%`;

  return (
    <div className="thread-log-performance">
      <section className={`thread-log-performance-summary status-${snapshot.status}`}>
        <div>
          <div className="thread-log-performance-eyebrow">{t('Renderer Health')}</div>
          <div className="thread-log-performance-title">{t(snapshot.status)}</div>
        </div>
        <p>{t(snapshot.summary)}</p>
      </section>

      <div className="thread-log-performance-grid">
        <MetricCard
          label={t('API calls')}
          value={snapshot.totals.api_call}
          hint={t('Slow or failed IPC/gateway calls')}
        />
        <MetricCard
          label={t('API hook')}
          value={snapshot.desktopApiMonitorInstalled ? t('On') : t('Off')}
          hint={snapshot.desktopApiMonitorInstalled
            ? t('Preload IPC timing is active')
            : t('Preload IPC timing is unavailable')}
        />
        <MetricCard
          label={t('Frame stalls')}
          value={snapshot.totals.frame_stall}
          hint={t('Large requestAnimationFrame gaps')}
        />
        <MetricCard
          label={t('Long tasks')}
          value={snapshot.totals.long_task}
          hint={t('Main-thread blocking work')}
        />
        <MetricCard
          label={t('JS heap')}
          value={memory ? formatBytes(memory.usedJSHeapSize) : t('Unavailable')}
          hint={memory && memoryPercent
            ? `${memoryPercent} of ${formatBytes(memory.jsHeapSizeLimit)}`
            : t('Chromium memory API unavailable')}
        />
      </div>

      <div className="thread-log-performance-section-title">{t('Recent performance events')}</div>
      {snapshot.events.length ? (
        <div className="thread-log-client-list">
          {snapshot.events.slice().reverse().map((entry) => (
            <details
              className={`thread-log-client-entry thread-log-performance-entry severity-${entry.severity}`}
              key={entry.key}
            >
              <summary className="thread-log-performance-entry-head">
                <span className={`thread-log-client-entry-type type-${entry.kind.replace(/_/g, '-')}`}>
                  {entry.kind}
                </span>
                <span className="thread-log-client-entry-summary" title={entry.summary}>
                  {entry.summary}
                </span>
                <span className="thread-log-performance-entry-time">
                  {entry.durationMs ? formatDuration(entry.durationMs) : entry.timestamp}
                </span>
              </summary>
              <pre className="thread-log-client-entry-detail">{entry.detail}</pre>
            </details>
          ))}
        </div>
      ) : (
        <div className="thread-log-panel-empty">
          {t('No performance events yet. Leave this panel open while reproducing a slowdown.')}
        </div>
      )}

      <div className="thread-log-performance-footnote">
        {t('Monitor started at')} {formatClock(snapshot.startedAt)} · {t('Updated')} {formatClock(snapshot.generatedAt)}
      </div>
    </div>
  );
}

function MetricCard({
  label,
  value,
  hint,
}: {
  label: string;
  value: string | number;
  hint: string;
}) {
  return (
    <div className="thread-log-performance-metric">
      <span>{label}</span>
      <strong>{value}</strong>
      <small>{hint}</small>
    </div>
  );
}
