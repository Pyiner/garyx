import {
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
  useEffect,
  useRef,
  useState,
} from 'react';
import { Archive, PanelLeftClose } from 'lucide-react';

import { AgentOptionAvatar } from './app-shell/components/AgentOptionAvatar';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from './components/ui/tooltip';
import { useI18n } from './i18n';
import type { ThreadAvatarIdentity } from './thread-avatar';
import { threadRailIsNearListEnd } from './thread-conversation-sidebar-model';

export type ThreadRailRow = {
  /** Stable React key and inline-confirm key. */
  key: string;
  title: string;
  titleTooltip?: string;
  time?: string | null;
  avatar?: ThreadAvatarIdentity | null;
  /** Translation key for an inline status badge (e.g. bot thread state). */
  badge?: string | null;
  isActive: boolean;
  isBusy?: boolean;
  /** Defaults to true; when false the row is rendered disabled. */
  openable?: boolean;
  onOpen: () => void;
  /** Per-row archive handler. Omit to render the row without an action. */
  onArchive?: () => void;
};

type ThreadConversationSidebarProps = {
  ariaLabel: string;
  /** Extra modifier appended to the shared `bot-conversation-rail` shell. */
  className?: string;
  /** Fully-formed logo node (caller owns any wrapper/styling). */
  logo?: ReactNode;
  title: string;
  titleTooltip?: string;
  collapseLabel: string;
  /** Shown when there are no rows. Omit to render an empty list silently. */
  emptyLabel?: string;
  /** Optional domain-owned control rendered between the title and list. */
  headerAccessory?: ReactNode;
  /** Optional domain-owned footer rendered inside the shared scroll area. */
  listFooter?: ReactNode;
  /** Called when the shared scroll area reaches its near-tail threshold. */
  onNearListEnd?: () => void;
  rowClassName?: string;
  rows: ThreadRailRow[];
  formatThreadTimestamp: (value?: string | null) => string;
  onClose: () => void;
  onRailResizeStart?: (event: ReactPointerEvent<HTMLDivElement>) => void;
  railResizing?: boolean;
};

/**
 * Shared secondary "thread list" rail behind Workspaces, Bots, and Recent.
 * Each caller maps its data into {@link ThreadRailRow}s. Archive is the single
 * unified row action; omit `onArchive` for a read-only row.
 */
export function ThreadConversationSidebar({
  ariaLabel,
  className,
  logo,
  title,
  titleTooltip,
  collapseLabel,
  emptyLabel,
  headerAccessory,
  listFooter,
  onNearListEnd,
  rowClassName,
  rows,
  formatThreadTimestamp,
  onClose,
  onRailResizeStart,
  railResizing,
}: ThreadConversationSidebarProps) {
  const { t } = useI18n();
  const [confirmKey, setConfirmKey] = useState<string | null>(null);
  const confirmTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!confirmKey) {
      return;
    }
    confirmTimerRef.current = setTimeout(() => {
      setConfirmKey(null);
    }, 3000);
    return () => {
      if (confirmTimerRef.current) {
        clearTimeout(confirmTimerRef.current);
      }
    };
  }, [confirmKey]);

  useEffect(() => {
    const list = listRef.current;
    if (onNearListEnd && list && threadRailIsNearListEnd(list)) {
      onNearListEnd();
    }
  }, [listFooter, onNearListEnd, rows.length]);

  return (
    <TooltipProvider>
    <aside aria-label={ariaLabel} className={`bot-conversation-rail ${className ?? ''}`.trim()}>
      <div className="bot-conversation-header">
        <div className="bot-conversation-heading">
          {logo ?? null}
          <div className="bot-conversation-title-copy">
            <div className="bot-conversation-title" title={titleTooltip ?? title}>
              {title}
            </div>
          </div>
        </div>
        <button
          aria-label={collapseLabel}
          className="bot-conversation-collapse"
          onClick={onClose}
          title={collapseLabel}
          type="button"
        >
          <PanelLeftClose aria-hidden size={15} strokeWidth={1.8} />
        </button>
      </div>

      {headerAccessory ?? null}

      <div
        className="bot-conversation-list"
        onScroll={(event) => {
          if (onNearListEnd && threadRailIsNearListEnd(event.currentTarget)) {
            onNearListEnd();
          }
        }}
        ref={listRef}
      >
        {rows.length ? (
          rows.map((row) => {
            const hasAction = Boolean(row.onArchive);
            const isConfirming = confirmKey === row.key;
            const openable = row.openable !== false;
            return (
              <div
                className={`bot-conversation-row-shell ${rowClassName ?? ''} ${row.isActive ? 'active' : ''} ${hasAction ? '' : 'no-delete'}`
                  .replace(/\s+/g, ' ')
                  .trim()}
                key={row.key}
                onMouseLeave={() => {
                  if (confirmKey === row.key) {
                    setConfirmKey(null);
                  }
                }}
              >
                <button
                  aria-current={row.isActive ? 'page' : undefined}
                  className={`bot-conversation-row ${row.avatar ? 'with-avatar' : ''}`.trim()}
                  disabled={!openable}
                  onClick={() => {
                    if (openable) {
                      row.onOpen();
                    }
                  }}
                  type="button"
                >
                  {row.avatar ? (
                    <span className="thread-row-avatar-wrap">
                      <AgentOptionAvatar
                        agentId={row.avatar.agentId}
                        avatarDataUrl={row.avatar.avatarDataUrl}
                        className="thread-row-agent-avatar"
                        kind={row.avatar.kind}
                        label={row.avatar.label}
                        providerIcon={row.avatar.providerIcon}
                        providerType={row.avatar.providerType}
                        size="default"
                      />
                      {row.isBusy ? (
                        <span aria-label={t('Loading')} className="thread-row-typing-badge" role="status">
                          <span />
                          <span />
                          <span />
                        </span>
                      ) : null}
                    </span>
                  ) : null}
                  <div className="bot-conversation-row-main">
                    <span className="bot-conversation-row-title" title={row.titleTooltip ?? row.title}>
                      {row.title}
                    </span>
                    {row.badge ? <span className="bot-thread-badge">{t(row.badge)}</span> : null}
                  </div>
                  <span className="bot-conversation-row-time">{formatThreadTimestamp(row.time)}</span>
                </button>
                {hasAction ? (
                  isConfirming ? (
                    <button
                      aria-label={t('Confirm archive {name}', { name: row.title })}
                      className="thread-delete-button confirm"
                      style={{ opacity: 1, pointerEvents: 'auto' }}
                      onClick={(event) => {
                        event.stopPropagation();
                        setConfirmKey(null);
                        row.onArchive?.();
                      }}
                      tabIndex={-1}
                      type="button"
                    >
                      {t('Confirm')}
                    </button>
                  ) : (
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <button
                          aria-label={t('Archive {name}', { name: row.title })}
                          className="thread-delete-button"
                          onClick={(event) => {
                            event.stopPropagation();
                            setConfirmKey(row.key);
                          }}
                          tabIndex={-1}
                          type="button"
                        >
                          <Archive aria-hidden />
                        </button>
                      </TooltipTrigger>
                      <TooltipContent>{t('Archive thread')}</TooltipContent>
                    </Tooltip>
                  )
                ) : null}
              </div>
            );
          })
        ) : emptyLabel ? (
          <p className="workspace-empty-note">{emptyLabel}</p>
        ) : null}
        {listFooter ?? null}
      </div>
      {onRailResizeStart ? (
        <div
          className={`sidebar-resizer ${railResizing ? 'is-resizing' : ''}`}
          onPointerDown={onRailResizeStart}
        >
          <div className="sidebar-resizer-line" />
        </div>
      ) : null}
    </aside>
    </TooltipProvider>
  );
}
