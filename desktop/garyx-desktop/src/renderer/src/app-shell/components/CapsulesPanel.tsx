import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ArrowLeft, Copy, Link2, MoreHorizontal, RefreshCw, Trash2 } from 'lucide-react';

import type {
  DesktopCapsuleSummary,
  DesktopCapsulesPage,
  DesktopCustomAgent,
} from '@shared/contracts';

import type { ToastTone } from '../../toast';
import { useI18n, type Translate } from '../../i18n';
import {
  DropdownMenu,
  DropdownMenuTrigger,
} from '../../components/ui/dropdown-menu';
import {
  FloatingActionMenuContent,
  FloatingActionMenuItem,
} from '../../components/ui/floating-action-menu';
import { capsuleHtmlStore } from '../capsule-html-store';
import { useInViewport } from '../use-in-viewport';
import { CapsuleLivePreviewFrame } from './CapsuleLivePreviewFrame';

type CapsulesPanelProps = {
  agents: DesktopCustomAgent[];
  onToast?: (message: string, tone?: ToastTone) => void;
  /** Set when the route is `#/capsules/<id>`: render the focused preview. */
  selectedCapsuleIdFromRoute: string | null;
  onOpenCapsulePreview: (capsuleId: string) => void;
  onCloseCapsulePreview: () => void;
};

function capsuleTitle(capsule: DesktopCapsuleSummary | null | undefined, t: Translate): string {
  return capsule?.title?.trim() || t('Untitled Capsule');
}

function capsuleDeepLink(capsuleId: string): string {
  return `garyx://capsules/${capsuleId}`;
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

function describeCreator(
  capsule: DesktopCapsuleSummary,
  agentById: Map<string, DesktopCustomAgent>,
  t: Translate,
): string {
  const agentId = capsule.agentId?.trim() || '';
  const agent = agentId ? agentById.get(agentId) || null : null;
  if (agent) {
    return agent.displayName?.trim() || agent.agentId;
  }
  if (agentId) {
    return agentId;
  }
  const providerLabel = typeof capsule.providerType === 'string' ? capsule.providerType.trim() : '';
  return providerLabel || t('Agent');
}

function CapsuleGalleryCard({
  capsule,
  agentById,
  onOpen,
}: {
  capsule: DesktopCapsuleSummary;
  agentById: Map<string, DesktopCustomAgent>;
  onOpen: (capsuleId: string) => void;
}) {
  const { t } = useI18n();
  const ref = useRef<HTMLButtonElement | null>(null);
  const visible = useInViewport(ref);
  const title = capsuleTitle(capsule, t);
  const creator = describeCreator(capsule, agentById, t);
  const subline = `${formatRelativeTime(capsule.updatedAt)} · ${creator}`;
  const metaTooltip = `${t('Revision')} ${capsule.revision} · ${formatBytes(capsule.byteSize)}`;
  return (
    <button
      ref={ref}
      className="capsule-gallery-card"
      onClick={() => onOpen(capsule.id)}
      title={title}
      type="button"
    >
      <span className="capsule-card-preview-shell">
        <CapsuleLivePreviewFrame
          active={visible}
          capsuleId={capsule.id}
          mode="card"
          revision={capsule.revision}
          title={title}
        />
      </span>
      <span className="capsule-card-meta">
        <span className="capsule-card-title">{title}</span>
        <span className="capsule-card-subline" title={metaTooltip}>
          {subline}
        </span>
      </span>
    </button>
  );
}

function CapsulePreviewPage({
  capsule,
  missing,
  deleting,
  onBack,
  onRefresh,
  onCopyLink,
  onCopyId,
  onDelete,
}: {
  capsule: DesktopCapsuleSummary | null;
  missing: boolean;
  deleting: boolean;
  onBack: () => void;
  onRefresh: () => void;
  onCopyLink: () => void;
  onCopyId: () => void;
  onDelete: () => void;
}) {
  const { t } = useI18n();
  const title = capsuleTitle(capsule, t);
  return (
    <div className="capsule-preview-page">
      <header className="capsule-preview-toolbar">
        <button
          aria-label={t('Back')}
          className="capsule-toolbar-button"
          onClick={onBack}
          title={t('Back')}
          type="button"
        >
          <ArrowLeft size={16} />
        </button>
        <span className="capsule-preview-title">{title}</span>
        <div className="capsule-preview-toolbar-actions">
          <button
            aria-label={t('Refresh')}
            className="capsule-toolbar-button"
            disabled={!capsule}
            onClick={onRefresh}
            title={t('Refresh')}
            type="button"
          >
            <RefreshCw size={15} />
          </button>
          <button
            aria-label={t('Copy link')}
            className="capsule-toolbar-button"
            disabled={!capsule}
            onClick={onCopyLink}
            title={t('Copy link')}
            type="button"
          >
            <Link2 size={15} />
          </button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button
                aria-label={t('More actions')}
                className="capsule-toolbar-button"
                disabled={!capsule}
                title={t('More actions')}
                type="button"
              >
                <MoreHorizontal size={15} />
              </button>
            </DropdownMenuTrigger>
            <FloatingActionMenuContent align="end" sideOffset={4}>
              <FloatingActionMenuItem onSelect={onCopyId}>
                <Copy aria-hidden />
                {t('Copy ID')}
              </FloatingActionMenuItem>
              <FloatingActionMenuItem
                className="capsule-menu-destructive"
                disabled={deleting}
                onSelect={onDelete}
              >
                <Trash2 aria-hidden />
                {deleting ? t('Deleting') : t('Delete')}
              </FloatingActionMenuItem>
            </FloatingActionMenuContent>
          </DropdownMenu>
        </div>
      </header>
      <div className="capsule-preview-body">
        {capsule ? (
          <CapsuleLivePreviewFrame
            active
            capsuleId={capsule.id}
            mode="preview"
            revision={capsule.revision}
            title={title}
          />
        ) : missing ? (
          <div className="capsule-frame-state capsule-frame-deleted">
            {t('Capsule deleted')}
          </div>
        ) : (
          <div className="capsule-frame-state capsule-frame-skeleton" />
        )}
      </div>
    </div>
  );
}

export function CapsulesPanel({
  agents,
  onToast,
  selectedCapsuleIdFromRoute,
  onOpenCapsulePreview,
  onCloseCapsulePreview,
}: CapsulesPanelProps) {
  const { t } = useI18n();
  const [page, setPage] = useState<DesktopCapsulesPage | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [fallbackCapsule, setFallbackCapsule] = useState<DesktopCapsuleSummary | null>(null);
  const [fallbackMissing, setFallbackMissing] = useState(false);
  const listRequestIdRef = useRef(0);

  const capsules = page?.capsules || [];
  const agentById = useMemo(
    () => new Map(agents.map((agent) => [agent.agentId, agent] as const)),
    [agents],
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

  useEffect(() => {
    void loadCapsules();
  }, [loadCapsules]);

  const listedPreviewCapsule = useMemo(
    () =>
      selectedCapsuleIdFromRoute
        ? capsules.find((capsule) => capsule.id === selectedCapsuleIdFromRoute) || null
        : null,
    [capsules, selectedCapsuleIdFromRoute],
  );

  // Deep links can open a preview before the list has loaded the summary; fetch
  // it directly so the preview has a revision/title (a null result means the
  // Capsule is gone → tombstone).
  useEffect(() => {
    if (!selectedCapsuleIdFromRoute || listedPreviewCapsule) {
      setFallbackCapsule(null);
      setFallbackMissing(false);
      return;
    }
    let cancelled = false;
    setFallbackCapsule(null);
    setFallbackMissing(false);
    void window.garyxDesktop
      .getCapsule(selectedCapsuleIdFromRoute)
      .then((capsule) => {
        if (cancelled) {
          return;
        }
        if (capsule) {
          setFallbackCapsule(capsule);
        } else {
          setFallbackMissing(true);
        }
      })
      .catch(() => {
        // Transient: leave it loading (no tombstone) so a reconnect can recover.
      });
    return () => {
      cancelled = true;
    };
  }, [selectedCapsuleIdFromRoute, listedPreviewCapsule]);

  const previewCapsule = listedPreviewCapsule || fallbackCapsule;

  const handleCopyLink = useCallback(async () => {
    if (!previewCapsule) {
      return;
    }
    try {
      await window.garyxDesktop.copyTextToClipboard({
        text: capsuleDeepLink(previewCapsule.id),
      });
      onToast?.(t('Capsule link copied.'), 'success');
    } catch (cause) {
      onToast?.(cause instanceof Error ? cause.message : t('Failed to copy link.'), 'error');
    }
  }, [onToast, previewCapsule, t]);

  const handleCopyId = useCallback(async () => {
    if (!previewCapsule) {
      return;
    }
    try {
      await window.garyxDesktop.copyTextToClipboard({ text: previewCapsule.id });
      onToast?.(t('Capsule ID copied.'), 'success');
    } catch (cause) {
      onToast?.(cause instanceof Error ? cause.message : t('Failed to copy Capsule ID.'), 'error');
    }
  }, [onToast, previewCapsule, t]);

  const handleRefresh = useCallback(() => {
    if (!previewCapsule) {
      return;
    }
    capsuleHtmlStore.request(previewCapsule.id, previewCapsule.revision, {
      force: true,
    });
  }, [previewCapsule]);

  const handleDelete = useCallback(async () => {
    if (!previewCapsule) {
      return;
    }
    const title = capsuleTitle(previewCapsule, t);
    if (!window.confirm(t('Delete Capsule "{title}"?', { title }))) {
      return;
    }
    setDeletingId(previewCapsule.id);
    setError(null);
    try {
      await window.garyxDesktop.deleteCapsule({ capsuleId: previewCapsule.id });
      capsuleHtmlStore.invalidateCapsule(previewCapsule.id);
      onCloseCapsulePreview();
      await loadCapsules();
      onToast?.(t('Capsule deleted.'), 'success');
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
      onToast?.(cause instanceof Error ? cause.message : t('Failed to delete Capsule.'), 'error');
    } finally {
      setDeletingId(null);
    }
  }, [loadCapsules, onCloseCapsulePreview, onToast, previewCapsule, t]);

  if (selectedCapsuleIdFromRoute) {
    return (
      <div className="capsules-page capsules-page-preview">
        <CapsulePreviewPage
          capsule={previewCapsule}
          deleting={deletingId === previewCapsule?.id}
          missing={fallbackMissing}
          onBack={onCloseCapsulePreview}
          onCopyId={() => {
            void handleCopyId();
          }}
          onCopyLink={() => {
            void handleCopyLink();
          }}
          onDelete={() => {
            void handleDelete();
          }}
          onRefresh={handleRefresh}
        />
      </div>
    );
  }

  return (
    <div className="capsules-page" aria-busy={loading}>
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
        <div className="capsules-gallery-grid">
          {capsules.map((capsule) => (
            <CapsuleGalleryCard
              agentById={agentById}
              capsule={capsule}
              key={capsule.id}
              onOpen={onOpenCapsulePreview}
            />
          ))}
        </div>
      )}
    </div>
  );
}
