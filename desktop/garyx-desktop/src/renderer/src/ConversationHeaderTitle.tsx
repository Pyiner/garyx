import { useEffect, useMemo, useState, type FormEvent, type RefObject } from 'react';
import { Archive, MoreHorizontal, Pencil, Star, StarOff, X } from 'lucide-react';

import type { DesktopBotConsoleSummary } from '@shared/contracts';

import { PinIcon, PinOffIcon } from './app-shell/icons';
import { ChannelLogo } from './channel-logo';
import { useChannelPluginCatalog } from './channel-plugins/useChannelPluginCatalog';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from './components/ui/dropdown-menu';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from './components/ui/dialog';
import { useI18n } from './i18n';

type ConversationHeaderTitleProps = {
  activeThreadBot: DesktopBotConsoleSummary | null;
  activeThreadTitle: string | null;
  activeWorkspaceName: string | null;
  canEditThreadTitle: boolean;
  contextText: string | null;
  editingThreadTitle: boolean;
  isAutomationView: boolean;
  isBotsView: boolean;
  isSkillsView: boolean;
  isThreadFavorite: boolean;
  isThreadPinned: boolean;
  archiveThreadDisabled: boolean;
  onBeginEdit: () => void;
  onArchiveThread: () => void;
  onCancelEdit: () => void;
  onSaveTitle: () => void;
  onToggleFavoriteThread: () => void;
  onTogglePinnedThread: () => void;
  onTitleDraftChange: (value: string) => void;
  savingTitle: boolean;
  titleDraft: string;
  titleInputRef: RefObject<HTMLInputElement | null>;
};

export function ConversationHeaderTitle({
  activeThreadBot,
  activeThreadTitle,
  activeWorkspaceName,
  canEditThreadTitle,
  contextText,
  editingThreadTitle,
  isAutomationView,
  isBotsView,
  isSkillsView,
  isThreadFavorite,
  isThreadPinned,
  archiveThreadDisabled,
  onBeginEdit,
  onArchiveThread,
  onCancelEdit,
  onSaveTitle,
  onToggleFavoriteThread,
  onTogglePinnedThread,
  onTitleDraftChange,
  savingTitle,
  titleDraft,
  titleInputRef,
}: ConversationHeaderTitleProps) {
  const { t } = useI18n();
  const [archiveConfirming, setArchiveConfirming] = useState(false);
  const { entries: pluginCatalog } = useChannelPluginCatalog();
  const iconDataUrlByChannel = useMemo(
    () =>
      new Map(
        (pluginCatalog || []).map((entry) => [
          entry.id.toLowerCase(),
          entry.icon_data_url || null,
        ]),
      ),
    [pluginCatalog],
  );
  const fallbackTitle = activeThreadTitle || activeWorkspaceName || t('Select a thread');
  const staticTitle = isAutomationView
    ? t('Automation')
    : isSkillsView
      ? t('Skills')
      : isBotsView
        ? t('Bots')
        : fallbackTitle;

  const staticTitleHint = isAutomationView
    ? t('Automation')
    : isSkillsView
      ? t('Skills')
      : isBotsView
        ? t('Bots')
        : activeThreadTitle || t('Select a thread');

  const handleRenameSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    onSaveTitle();
  };

  useEffect(() => {
    if (!archiveConfirming) {
      return;
    }
    const timer = globalThis.setTimeout(() => {
      setArchiveConfirming(false);
    }, 3000);
    return () => {
      globalThis.clearTimeout(timer);
    };
  }, [archiveConfirming]);

  useEffect(() => {
    setArchiveConfirming(false);
  }, [activeThreadTitle, archiveThreadDisabled]);

  return (
    <div className="conversation-header-copy">
      <div className="conversation-heading-stack">
        <div className="conversation-heading-row">
          {canEditThreadTitle ? (
            <div className="conversation-title-group">
              {isThreadPinned ? (
                <PinIcon className="conversation-title-pin" />
              ) : null}
              <span className="conversation-title-text" title={fallbackTitle}>
                {fallbackTitle}
              </span>
            </div>
          ) : (
            <h2 title={staticTitleHint}>{staticTitle}</h2>
          )}
          {contextText ? (
            <span className="conversation-context">{contextText}</span>
          ) : null}
          {activeThreadBot ? (
            <span
              aria-label={t('Bound to {name}', { name: activeThreadBot.title })}
              className="conversation-bot-binding-status"
              title={t('Bound to {name}', { name: activeThreadBot.title })}
            >
              <ChannelLogo
                channel={activeThreadBot.channel}
                className="channel-logo header-channel-logo conversation-bot-binding-logo"
                iconDataUrl={
                  iconDataUrlByChannel.get(activeThreadBot.channel.toLowerCase()) || null
                }
                fallbackLabel={activeThreadBot.title}
              />
              <span className="conversation-bot-binding-name">
                {activeThreadBot.title}
              </span>
            </span>
          ) : null}
          {canEditThreadTitle ? (
            <>
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <button
                    aria-label={t('Thread actions')}
                    className="icon-menu-trigger"
                    title={t('Thread actions')}
                    type="button"
                  >
                    <MoreHorizontal aria-hidden size={18} strokeWidth={2} />
                  </button>
                </DropdownMenuTrigger>
                <DropdownMenuContent
                  align="start"
                  className="thread-title-menu-content"
                >
                  <DropdownMenuItem onSelect={onTogglePinnedThread}>
                    {isThreadPinned ? <PinOffIcon /> : <PinIcon />}
                    <span>
                      {isThreadPinned ? t('Unpin conversation') : t('Pin conversation')}
                    </span>
                    <DropdownMenuShortcut>⌥⌘P</DropdownMenuShortcut>
                  </DropdownMenuItem>
                  <DropdownMenuItem onSelect={onToggleFavoriteThread}>
                    {isThreadFavorite ? <StarOff aria-hidden /> : <Star aria-hidden />}
                    <span>
                      {isThreadFavorite
                        ? t('Unfavorite conversation')
                        : t('Favorite conversation')}
                    </span>
                  </DropdownMenuItem>
                  <DropdownMenuItem onSelect={onBeginEdit}>
                    <Pencil aria-hidden />
                    <span>{t('Rename conversation')}</span>
                    <DropdownMenuShortcut>⌥⌘R</DropdownMenuShortcut>
                  </DropdownMenuItem>
                  <DropdownMenuItem
                    disabled={archiveThreadDisabled}
                    onSelect={(event) => {
                      if (!archiveConfirming) {
                        event.preventDefault();
                        setArchiveConfirming(true);
                        return;
                      }
                      setArchiveConfirming(false);
                      onArchiveThread();
                    }}
                    variant={archiveConfirming ? 'destructive' : 'default'}
                  >
                    <Archive aria-hidden />
                    <span>{archiveConfirming ? t('Confirm') : t('Archive conversation')}</span>
                    <DropdownMenuShortcut>⇧⌘A</DropdownMenuShortcut>
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
              <Dialog
                open={editingThreadTitle}
                onOpenChange={(open) => {
                  if (!open) {
                    onCancelEdit();
                  }
                }}
              >
                <DialogContent
                  className="thread-rename-dialog"
                  showCloseButton={false}
                  size="narrow"
                >
                  <form className="thread-rename-form" onSubmit={handleRenameSubmit}>
                    <button
                      aria-label={t('Close')}
                      className="thread-rename-close"
                      onClick={onCancelEdit}
                      type="button"
                    >
                      <X aria-hidden size={18} strokeWidth={2} />
                    </button>
                    <div className="thread-rename-copy">
                      <DialogTitle className="thread-rename-title">
                        {t('Rename conversation')}
                      </DialogTitle>
                      <DialogDescription className="thread-rename-description">
                        {t('Keep it short and easy to recognize')}
                      </DialogDescription>
                    </div>
                    <input
                      ref={titleInputRef}
                      aria-label={t('Thread title')}
                      className="thread-rename-input"
                      disabled={savingTitle}
                      onChange={(event) => {
                        onTitleDraftChange(event.target.value);
                      }}
                      value={titleDraft}
                    />
                    <div className="thread-rename-actions">
                      <button
                        className="thread-rename-button thread-rename-button-secondary"
                        disabled={savingTitle}
                        onClick={onCancelEdit}
                        type="button"
                      >
                        {t('Cancel')}
                      </button>
                      <button
                        className="thread-rename-button thread-rename-button-primary"
                        disabled={savingTitle}
                        type="submit"
                      >
                        {t('Save')}
                      </button>
                    </div>
                  </form>
                </DialogContent>
              </Dialog>
            </>
          ) : null}
        </div>
      </div>
    </div>
  );
}
