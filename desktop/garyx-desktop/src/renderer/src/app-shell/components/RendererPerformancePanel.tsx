import { useI18n } from '../../i18n';
import type { RendererPerformanceSnapshot } from '../../perf-metrics';

type RendererPerformancePanelProps = {
  snapshot: RendererPerformanceSnapshot;
};

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

export function RendererPerformancePanel({
  snapshot,
}: RendererPerformancePanelProps) {
  const { t } = useI18n();
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
