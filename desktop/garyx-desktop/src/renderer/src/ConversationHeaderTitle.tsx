import type { KeyboardEvent, RefObject } from 'react';

import { useI18n } from './i18n';

type ConversationHeaderTitleProps = {
  activeThreadTitle: string | null;
  activeWorkspaceName: string | null;
  canEditThreadTitle: boolean;
  contextText: string | null;
  editingThreadTitle: boolean;
  isAutomationView: boolean;
  isBotsView: boolean;
  isSkillsView: boolean;
  onBeginEdit: () => void;
  onCancelEdit: () => void;
  onSaveTitle: () => void;
  onTitleDraftChange: (value: string) => void;
  titleDraft: string;
  titleInputRef: RefObject<HTMLInputElement | null>;
};

export function ConversationHeaderTitle({
  activeThreadTitle,
  activeWorkspaceName,
  canEditThreadTitle,
  contextText,
  editingThreadTitle,
  isAutomationView,
  isBotsView,
  isSkillsView,
  onBeginEdit,
  onCancelEdit,
  onSaveTitle,
  onTitleDraftChange,
  titleDraft,
  titleInputRef,
}: ConversationHeaderTitleProps) {
  const { t } = useI18n();
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
          {contextText ? (
            <span className="conversation-context">{contextText}</span>
          ) : null}
        </div>
      </div>
    </div>
  );
}
