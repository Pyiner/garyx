import { IconPlugConnected } from '@tabler/icons-react';

import type { KeyboardEvent, RefObject } from 'react';

import type { DesktopBotConsoleSummary } from '@shared/contracts';

import { ChannelLogo } from './channel-logo';

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
  const fallbackTitle = activeThreadTitle || activeWorkspaceName || 'Select a thread';
  const staticTitle = isAutomationView
    ? 'Automation'
    : isSkillsView
      ? 'Skills'
      : isBotsView
        ? 'Bots'
        : fallbackTitle;

  const staticTitleHint = isAutomationView
    ? 'Automation'
    : isSkillsView
      ? 'Skills'
      : isBotsView
        ? 'Bots'
        : activeThreadTitle || 'Select a thread';

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

  return (
    <div className="conversation-header-copy">
      <div className="conversation-heading-stack">
        <div className="conversation-heading-row">
          {canEditThreadTitle && editingThreadTitle ? (
            <input
              ref={titleInputRef}
              aria-label="Thread title"
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
              title="Click to rename thread"
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
            <label
              className={`thread-bot-inline-trigger ${activeThreadBot ? '' : 'empty'}`}
              title={activeThreadBot ? `Bound to ${activeThreadBot.title}` : 'Bind bot'}
            >
              {activeThreadBot ? (
                <ChannelLogo
                  channel={activeThreadBot.channel}
                  className="channel-logo header-channel-logo"
                />
              ) : (
                <IconPlugConnected aria-hidden className="icon" size={14} stroke={1.7} />
              )}
              <select
                aria-label="Thread bot binding"
                className="thread-bot-inline-select"
                disabled={bindingMutation === 'bot-binding'}
                onChange={(event) => {
                  const nextBotId = event.target.value || null;
                  if (activeThread) {
                    onSetBotBinding(nextBotId);
                  } else {
                    setPendingBotId(nextBotId);
                  }
                }}
                value={activeThreadBotId || ''}
              >
                <option value="">No bot</option>
                {botGroups.map((group) => (
                  <option key={group.id} value={group.id}>
                    {group.title}
                  </option>
                ))}
              </select>
            </label>
          ) : null}
          {contextText ? (
            <span className="conversation-context">{contextText}</span>
          ) : null}
        </div>
      </div>
    </div>
  );
}
