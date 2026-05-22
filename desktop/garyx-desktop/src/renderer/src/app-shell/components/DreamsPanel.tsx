import { useCallback, useEffect, useMemo, useState } from 'react';
import { RefreshCw, Sparkles } from 'lucide-react';

import type { DesktopDreamTopic, DesktopDreamsPage } from '@shared/contracts';

import { useI18n } from '../../i18n';
import { formatTimestamp } from './auto-research/helpers';

type DreamsPanelProps = {
  onOpenThread: (threadId: string) => void;
};

function confidenceLabel(value: number): string {
  return `${Math.round(Math.max(0, Math.min(1, value)) * 100)}%`;
}

function dreamRangeLabel(dream: DesktopDreamTopic): string {
  const latest = formatTimestamp(dream.lastMessageAt) || dream.lastMessageAt;
  const earliest = formatTimestamp(dream.firstMessageAt) || dream.firstMessageAt;
  return earliest && latest && earliest !== latest ? `${earliest} - ${latest}` : latest || earliest;
}

export function DreamsPanel({ onOpenThread }: DreamsPanelProps) {
  const { t } = useI18n();
  const [page, setPage] = useState<DesktopDreamsPage | null>(null);
  const [loading, setLoading] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const dreams = page?.dreams || [];
  const subtitle = useMemo(() => {
    if (!page) {
      return t('Last 24 hours');
    }
    const scan = page.scan || page.latestScan;
    if (!scan?.createdAt) {
      return t('Last 24 hours');
    }
    const at = formatTimestamp(scan.createdAt) || scan.createdAt;
    return `${t('Last scan')} ${at}`;
  }, [page, t]);

  const loadDreams = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await window.garyxDesktop.listDreams({ sinceHours: 24, limit: 80 });
      setPage(result);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  const scanDreams = useCallback(async () => {
    setScanning(true);
    setError(null);
    try {
      const result = await window.garyxDesktop.scanDreams({
        sinceHours: 24,
        mode: 'auto',
        limit: 600,
      });
      setPage(result);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setScanning(false);
    }
  }, []);

  useEffect(() => {
    void loadDreams();
  }, [loadDreams]);

  return (
    <div className="dreams-page" aria-busy={loading || scanning}>
      <header className="dreams-page-header">
        <div className="dreams-page-title-block">
          <div className="dreams-page-title-row">
            <h1 className="dreams-page-title">{t('Dreams')}</h1>
            <span className="tasks-status-chip tone-progress">{dreams.length}</span>
          </div>
          <p className="dreams-page-subtitle">{subtitle}</p>
        </div>
        <div className="dreams-header-actions">
          <button
            className="tasks-secondary-button"
            disabled={loading || scanning}
            onClick={() => {
              void loadDreams();
            }}
            type="button"
          >
            <RefreshCw size={14} />
            {t('Refresh')}
          </button>
          <button
            className="tasks-primary-button"
            disabled={loading || scanning}
            onClick={() => {
              void scanDreams();
            }}
            type="button"
          >
            <Sparkles size={14} />
            {scanning ? t('Scanning') : t('Scan')}
          </button>
        </div>
      </header>

      {error ? <div className="tasks-state tasks-state-error">{error}</div> : null}

      {!dreams.length && !loading && !error ? (
        <div className="tasks-empty-state">{t('No dreams yet.')}</div>
      ) : (
        <div className="dreams-list">
          {dreams.map((dream) => (
            <article className="dreams-topic" key={dream.dreamId}>
              <button
                className="dreams-topic-main dreams-topic-open"
                disabled={!dream.spans.length}
                onClick={() => {
                  const firstSpan = dream.spans[0];
                  if (firstSpan) {
                    onOpenThread(firstSpan.threadId);
                  }
                }}
                type="button"
              >
                <div className="dreams-topic-title-row">
                  <h2>{dream.title}</h2>
                  <span className="tasks-status-chip tone-review">
                    {confidenceLabel(dream.confidence)}
                  </span>
                </div>
                <p className="dreams-topic-summary">{dream.summary}</p>
                <div className="dreams-topic-meta">
                  <span>{dreamRangeLabel(dream)}</span>
                  <span>{dream.messageCount} {t('messages')}</span>
                  <span>{dream.source}</span>
                </div>
              </button>
              <div className="dreams-spans">
                {dream.spans.map((span) => (
                  <button
                    className="dreams-span-row"
                    key={span.spanId}
                    onClick={() => {
                      onOpenThread(span.threadId);
                    }}
                    type="button"
                  >
                    <span className="dreams-span-thread">{span.threadId}</span>
                    <span className="dreams-span-range">#{span.startSeq}-{span.endSeq}</span>
                    <span className="dreams-span-excerpt">{span.excerpt}</span>
                  </button>
                ))}
              </div>
            </article>
          ))}
        </div>
      )}
    </div>
  );
}
