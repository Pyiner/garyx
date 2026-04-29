import { IconPlugConnected } from '@tabler/icons-react';

import type { KeyboardEvent, RefObject } from 'react';

import type { DesktopBotConsoleSummary } from '@shared/contracts';

import { ChannelLogo } from './channel-logo';
import { useChannelPluginCatalog } from './channel-plugins/useChannelPluginCatalog';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
} from './components/ui/select';
import { useI18n } from './i18n';

const NO_BOT_VALUE = '__garyx_no_bot__';

type ConversationHeaderTitleProps = {
  activeThread: { id: string } | null;
  activeThreadBot: DesktopBotConsoleSummary | null;
  activeThreadBotId: string | null;
  activeThreadTitle: string | null;
  activeWorkspaceName: string | null;
  bindingMutation: string | null;
  botGroups: DesktopBotConsoleSummary[];
  canEditThreadTitle: boolean;
  contextText: string | null;
  editingThreadTitle: boolean;
  isAutomationView: boolean;
  isBotsView: boolean;
  isSkillsView: boolean;
  onBeginEdit: () => void;
  onCancelEdit: () => void;
  onSaveTitle: () => void;
  onSetBotBinding: (botId: string | null) => void;
  onTitleDraftChange: (value: string) => void;
  setPendingBotId: (botId: string | null) => void;
  titleDraft: string;
  titleInputRef: RefObject<HTMLInputElement | null>;
};

export function ConversationHeaderTitle({
  activeThread,
  activeThreadBot,
  activeThreadBotId,
  activeThreadTitle,
  activeWorkspaceName,
  bindingMutation,
  botGroups,
  canEditThreadTitle,
  contextText,
  editingThreadTitle,
  isAutomationView,
  isBotsView,
  isSkillsView,
  onBeginEdit,
  onCancelEdit,
  onSaveTitle,
  onSetBotBinding,
  onTitleDraftChange,
  setPendingBotId,
  titleDraft,
  titleInputRef,
}: ConversationHeaderTitleProps) {
  const { t } = useI18n();
  const { entries: pluginCatalog } = useChannelPluginCatalog();
  const iconDataUrlByChannel = new Map(
    (pluginCatalog || []).map((entry) => [entry.id.toLowerCase(), entry.icon_data_url || null]),
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

  const handleTitleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      onSaveTitle();
    }
    if (event.key === 'Escape') {
      event.preventDefault();
      onCancelEdit();
    }
  };
  const selectedBot = activeThreadBotId
    ? botGroups.find((group) => group.id === activeThreadBotId) || activeThreadBot
    : null;
  const selectedBotIcon = selectedBot
    ? iconDataUrlByChannel.get(selectedBot.channel.toLowerCase()) || null
    : null;
  const botBindingDisabled = bindingMutation === 'bot-binding';

  return (
    <div className="conversation-header-copy">
      <div className="conversation-heading-stack">
        <div className="conversation-heading-row">
          {canEditThreadTitle && editingThreadTitle ? (
            <input
              ref={titleInputRef}
              aria-label={t('Thread title')}
              className="conversation-title-input"
              onBlur={onSaveTitle}
              onChange={(event) => {
                onTitleDraftChange(event.target.value);
              }}
              onKeyDown={handleTitleKeyDown}
              size={Math.max(titleDraft.length + 2, 8)}
              value={titleDraft}
            />
          ) : canEditThreadTitle ? (
            <button
              className="conversation-title-button"
              onClick={onBeginEdit}
              title={t('Click to rename thread')}
              type="button"
            >
              <span className="conversation-title-text">
                {fallbackTitle}
              </span>
            </button>
          ) : (
            <h2 title={staticTitleHint}>{staticTitle}</h2>
          )}
          {!isAutomationView && !isSkillsView && !isBotsView && (activeThread || activeThreadBotId) ? (
            <Select
              disabled={botBindingDisabled}
              onValueChange={(value) => {
                const nextBotId = value === NO_BOT_VALUE ? null : value;
                if (activeThread) {
                  onSetBotBinding(nextBotId);
                } else {
                  setPendingBotId(nextBotId);
                }
              }}
              value={activeThreadBotId || NO_BOT_VALUE}
            >
              <SelectTrigger
                aria-label={t('Thread bot binding')}
                className={`thread-bot-inline-trigger ${selectedBot ? '' : 'empty'}`}
                title={selectedBot ? t('Bound to {name}', { name: selectedBot.title }) : t('Bind bot')}
              >
                {selectedBot ? (
                  <ChannelLogo
                    channel={selectedBot.channel}
                    className="channel-logo header-channel-logo"
                    iconDataUrl={selectedBotIcon}
                    fallbackLabel={selectedBot.title}
                  />
                ) : (
                  <IconPlugConnected aria-hidden className="icon" size={14} stroke={1.7} />
                )}
              </SelectTrigger>
              <SelectContent className="thread-bot-select-content">
                <SelectItem className="thread-bot-select-item" textValue={t('No bot')} value={NO_BOT_VALUE}>
                  <span className="thread-bot-select-option">
                    <IconPlugConnected aria-hidden className="icon" size={16} stroke={1.7} />
                    <span className="thread-bot-select-label">{t('No bot')}</span>
                  </span>
                </SelectItem>
                {botGroups.map((group) => (
                  <SelectItem
                    className="thread-bot-select-item"
                    key={group.id}
                    textValue={group.title}
                    value={group.id}
                  >
                    <span className="thread-bot-select-option">
                      <ChannelLogo
                        channel={group.channel}
                        className="channel-logo thread-bot-select-logo"
                        iconDataUrl={iconDataUrlByChannel.get(group.channel.toLowerCase()) || null}
                        fallbackLabel={group.title}
                      />
                      <span className="thread-bot-select-label">{group.title}</span>
                    </span>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          ) : null}
          {contextText ? (
            <span className="conversation-context">{contextText}</span>
          ) : null}
        </div>
      </div>
    </div>
  );
}
