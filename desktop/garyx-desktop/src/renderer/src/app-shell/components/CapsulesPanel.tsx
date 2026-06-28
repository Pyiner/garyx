import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Copy, RefreshCw, Trash2 } from 'lucide-react';

import type {
  DesktopCapsuleSummary,
  DesktopCapsulesPage,
  DesktopCustomAgent,
} from '@shared/contracts';

import type { ToastTone } from '../../toast';
import { useI18n, type Translate } from '../../i18n';
import { AgentOptionAvatar } from './AgentOptionAvatar';

type CapsulesPanelProps = {
  agents: DesktopCustomAgent[];
  onToast?: (message: string, tone?: ToastTone) => void;
};

type HtmlErrorMap = Record<string, string | null>;
type HtmlCache = Record<string, string>;

function capsuleTitle(capsule: DesktopCapsuleSummary | null | undefined, t: Translate): string {
  return capsule?.title?.trim() || t('Untitled Capsule');
}

function cacheKey(capsule: DesktopCapsuleSummary): string {
  return [capsule.id, capsule.revision, capsule.htmlSha256 || 'no-sha'].join(':');
}

function formatBytes(bytes: number): string {
  const safeBytes = Math.max(0, Number.isFinite(bytes) ? bytes : 0);
  if (safeBytes < 1024) {
    return `${safeBytes} B`;
  }
  const kib = safeBytes / 1024;
  if (kib < 1024) {
    return `${kib >= 10 ? Math.round(kib) : kib.toFixed(1)} KB`;
  }
  const mib = kib / 1024;
  return `${mib >= 10 ? Math.round(mib) : mib.toFixed(1)} MB`;
}

function formatRelativeTime(value?: string | null): string {
  if (!value) {
    return 'Unknown';
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  const diffMs = Date.now() - date.getTime();
  if (diffMs < 60_000) {
    return 'now';
  }
  const minutes = Math.floor(diffMs / 60_000);
  if (minutes < 60) {
    return `${minutes}m`;
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    return `${hours}h`;
  }
  const days = Math.floor(hours / 24);
  if (days < 14) {
    return `${days}d`;
  }
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
  }).format(date);
}

function formatTimestamp(value?: string | null): string {
  if (!value) {
    return 'Unknown';
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  }).format(date);
}

function describeCreator(
  capsule: DesktopCapsuleSummary,
  agentById: Map<string, DesktopCustomAgent>,
  t: Translate,
): { agent: DesktopCustomAgent | null; label: string; detail: string | null } {
  const agentId = capsule.agentId?.trim() || '';
  const agent = agentId ? agentById.get(agentId) || null : null;
  if (agent) {
    return {
      agent,
      label: agent.displayName?.trim() || agent.agentId,
      detail: agent.agentId,
    };
  }
  if (agentId) {
    return {
      agent: null,
      label: agentId,
      detail: typeof capsule.providerType === 'string' ? capsule.providerType.trim() || null : null,
    };
  }
  const providerLabel = typeof capsule.providerType === 'string' ? capsule.providerType.trim() : '';
  if (providerLabel) {
    return {
      agent: null,
      label: providerLabel,
      detail: null,
    };
  }
  return {
    agent: null,
    label: t('Agent'),
    detail: null,
  };
}

function CreatorBadge({
  agentById,
  capsule,
}: {
  agentById: Map<string, DesktopCustomAgent>;
  capsule: DesktopCapsuleSummary;
}) {
  const { t } = useI18n();
  const creator = describeCreator(capsule, agentById, t);
  return (
    <span className="capsules-agent-badge" title={creator.detail || creator.label}>
      <AgentOptionAvatar
        agentId={creator.agent?.agentId || capsule.agentId || null}
        avatarDataUrl={creator.agent?.avatarDataUrl || null}
        kind={creator.agent?.builtIn ? 'builtin' : 'agent'}
        label={creator.label}
        providerIcon={creator.agent?.providerIcon || null}
        providerType={creator.agent?.providerType || null}
      />
      <span className="capsules-agent-label">{creator.label}</span>
    </span>
  );
}

export function CapsulesPanel({ agents, onToast }: CapsulesPanelProps) {
  const { t } = useI18n();
  const [page, setPage] = useState<DesktopCapsulesPage | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedCapsuleId, setSelectedCapsuleId] = useState<string | null>(null);
  const [htmlByKey, setHtmlByKey] = useState<HtmlCache>({});
  const [htmlErrorById, setHtmlErrorById] = useState<HtmlErrorMap>({});
  const [htmlLoadingId, setHtmlLoadingId] = useState<string | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const listRequestIdRef = useRef(0);
  const htmlRequestIdRef = useRef(0);

  const capsules = page?.capsules || [];
  const agentById = useMemo(
    () => new Map(agents.map((agent) => [agent.agentId, agent] as const)),
    [agents],
  );
  const selectedCapsule = useMemo(
    () => capsules.find((capsule) => capsule.id === selectedCapsuleId) || capsules[0] || null,
    [capsules, selectedCapsuleId],
  );
  const selectedCacheKey = selectedCapsule ? cacheKey(selectedCapsule) : null;
  const selectedHtml = selectedCacheKey ? htmlByKey[selectedCacheKey] : undefined;
  const selectedHtmlError = selectedCapsule ? htmlErrorById[selectedCapsule.id] || null : null;
  const selectedHtmlLoading = Boolean(
    selectedCapsule && htmlLoadingId === selectedCapsule.id && selectedHtml === undefined,
  );

  const loadCapsules = useCallback(async () => {
    const requestId = listRequestIdRef.current + 1;
    listRequestIdRef.current = requestId;
    setLoading(true);
    setError(null);
    try {
      const result = await window.garyxDesktop.listCapsules();
      if (listRequestIdRef.current !== requestId) {
        return;
      }
      setPage(result);
      setSelectedCapsuleId((current) => {
        if (current && result.capsules.some((capsule) => capsule.id === current)) {
          return current;
        }
        return result.capsules[0]?.id || null;
      });
    } catch (cause) {
      if (listRequestIdRef.current !== requestId) {
        return;
      }
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      if (listRequestIdRef.current === requestId) {
        setLoading(false);
      }
    }
  }, []);

  const loadSelectedHtml = useCallback(async (
    capsule: DesktopCapsuleSummary,
    options: { force?: boolean } = {},
  ) => {
    const key = cacheKey(capsule);
    if (!options.force && Object.prototype.hasOwnProperty.call(htmlByKey, key)) {
      return;
    }
    const requestId = htmlRequestIdRef.current + 1;
    htmlRequestIdRef.current = requestId;
    setHtmlLoadingId(capsule.id);
    setHtmlErrorById((current) => ({ ...current, [capsule.id]: null }));
    try {
      const html = await window.garyxDesktop.getCapsuleHtml(capsule.id);
      if (htmlRequestIdRef.current !== requestId) {
        return;
      }
      setHtmlByKey((current) => ({ ...current, [key]: html }));
    } catch (cause) {
      if (htmlRequestIdRef.current !== requestId) {
        return;
      }
      setHtmlErrorById((current) => ({
        ...current,
        [capsule.id]: cause instanceof Error ? cause.message : String(cause),
      }));
    } finally {
      if (htmlRequestIdRef.current === requestId) {
        setHtmlLoadingId((current) => (current === capsule.id ? null : current));
      }
    }
  }, [htmlByKey]);

  useEffect(() => {
    void loadCapsules();
  }, [loadCapsules]);

  useEffect(() => {
    if (!selectedCapsule) {
      return;
    }
    void loadSelectedHtml(selectedCapsule);
  }, [loadSelectedHtml, selectedCapsule]);

  const handleCopyId = useCallback(async () => {
    if (!selectedCapsule) {
      return;
    }
    try {
      await window.garyxDesktop.copyTextToClipboard({ text: selectedCapsule.id });
      onToast?.(t('Capsule ID copied.'), 'success');
    } catch (cause) {
      onToast?.(
        cause instanceof Error ? cause.message : t('Failed to copy Capsule ID.'),
        'error',
      );
    }
  }, [onToast, selectedCapsule, t]);

  const handleDelete = useCallback(async () => {
    if (!selectedCapsule) {
      return;
    }
    const title = capsuleTitle(selectedCapsule, t);
    if (!window.confirm(t('Delete Capsule "{title}"?', { title }))) {
      return;
    }
    setDeletingId(selectedCapsule.id);
    setError(null);
    try {
      await window.garyxDesktop.deleteCapsule({ capsuleId: selectedCapsule.id });
      setHtmlByKey((current) => {
        const next: HtmlCache = {};
        for (const [key, value] of Object.entries(current)) {
          if (!key.startsWith(`${selectedCapsule.id}:`)) {
            next[key] = value;
          }
        }
        return next;
      });
      setSelectedCapsuleId(null);
      await loadCapsules();
      onToast?.(t('Capsule deleted.'), 'success');
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      onToast?.(cause instanceof Error ? cause.message : t('Failed to delete Capsule.'), 'error');
    } finally {
      setDeletingId(null);
    }
  }, [loadCapsules, onToast, selectedCapsule, t]);

  return (
    <div className="capsules-page" aria-busy={loading || Boolean(htmlLoadingId)}>
      <header className="capsules-page-header">
        <div className="capsules-page-title-block">
          <div className="capsules-page-title-row">
            <h1 className="capsules-page-title">{t('Capsules')}</h1>
            <span className="tasks-status-chip tone-progress">{capsules.length}</span>
          </div>
          <p className="capsules-page-subtitle">
            {t('Self-contained HTML created by agents.')}
          </p>
        </div>
        <div className="capsules-header-actions">
          <button
            className="tasks-secondary-button capsules-refresh-button"
            disabled={loading}
            onClick={() => {
              void loadCapsules();
            }}
            type="button"
          >
            <RefreshCw size={14} />
            {loading ? t('Refreshing') : t('Refresh')}
          </button>
        </div>
      </header>

      {error ? <div className="tasks-state tasks-state-error">{error}</div> : null}

      {!capsules.length && !loading && !error ? (
        <div className="tasks-empty-state">{t('No Capsules yet.')}</div>
      ) : (
        <div className="capsules-layout">
          <div className="capsules-list" role="listbox" aria-label={t('Capsules')}>
            {capsules.map((capsule) => {
              const active = selectedCapsule?.id === capsule.id;
              return (
                <button
                  aria-selected={active}
                  className={`capsules-list-row ${active ? 'active' : ''}`}
                  key={capsule.id}
                  onClick={() => {
                    setSelectedCapsuleId(capsule.id);
                  }}
                  role="option"
                  type="button"
                >
                  <span className="capsules-list-row-main">
                    <span className="capsules-list-title">{capsuleTitle(capsule, t)}</span>
                    {capsule.description ? (
                      <span className="capsules-list-description">{capsule.description}</span>
                    ) : null}
                  </span>
                  <span className="capsules-list-meta">
                    <span>{formatRelativeTime(capsule.updatedAt)}</span>
                    <span>{formatBytes(capsule.byteSize)}</span>
                  </span>
                  <CreatorBadge agentById={agentById} capsule={capsule} />
                </button>
              );
            })}
          </div>

          <section className="capsules-detail" aria-label={t('Capsule runner')}>
            {selectedCapsule ? (
              <>
                <header className="capsules-detail-header">
                  <div className="capsules-detail-copy">
                    <h2>{capsuleTitle(selectedCapsule, t)}</h2>
                    {selectedCapsule.description ? <p>{selectedCapsule.description}</p> : null}
                    <div className="capsules-detail-meta">
                      <span>{t('Revision')} {selectedCapsule.revision}</span>
                      <span>{formatBytes(selectedCapsule.byteSize)}</span>
                      <span>{formatTimestamp(selectedCapsule.updatedAt)}</span>
                      <span className="capsules-detail-id">{selectedCapsule.id}</span>
                    </div>
                  </div>
                  <div className="capsules-detail-actions">
                    <button
                      className="tasks-secondary-button capsules-action-button"
                      onClick={() => {
                        void handleCopyId();
                      }}
                      type="button"
                    >
                      <Copy size={14} />
                      {t('Copy ID')}
                    </button>
                    <button
                      className="tasks-secondary-button capsules-action-button"
                      disabled={htmlLoadingId === selectedCapsule.id}
                      onClick={() => {
                        void loadSelectedHtml(selectedCapsule, { force: true });
                      }}
                      type="button"
                    >
                      <RefreshCw size={14} />
                      {t('Refresh HTML')}
                    </button>
                    <button
                      className="tasks-secondary-button capsules-action-button capsules-delete-button"
                      disabled={deletingId === selectedCapsule.id}
                      onClick={() => {
                        void handleDelete();
                      }}
                      type="button"
                    >
                      <Trash2 size={14} />
                      {deletingId === selectedCapsule.id ? t('Deleting') : t('Delete')}
                    </button>
                  </div>
                </header>

                {selectedHtmlError ? (
                  <div className="tasks-state tasks-state-error">{selectedHtmlError}</div>
                ) : selectedHtmlLoading ? (
                  <div className="tasks-state">{t('Loading Capsule HTML…')}</div>
                ) : selectedHtml !== undefined && selectedCacheKey ? (
                  <div className="capsules-runner-shell">
                    {/* Capsule HTML is untrusted: fetch through main-process auth, then run in an opaque-origin iframe. */}
                    <iframe
                      className="capsules-runner-frame"
                      key={selectedCacheKey}
                      sandbox="allow-scripts"
                      srcDoc={selectedHtml}
                      title={capsuleTitle(selectedCapsule, t)}
                    />
                  </div>
                ) : (
                  <div className="tasks-state">{t('Select a Capsule to run it.')}</div>
                )}
              </>
            ) : (
              <div className="tasks-empty-state">{t('Select a Capsule to run it.')}</div>
            )}
          </section>
        </div>
      )}
    </div>
  );
}
