import { IconFolder, IconPlugConnected } from '@tabler/icons-react';

import {
  ConversationTeamMembers,
  type ConversationTeamSummary,
} from './ConversationTeamMembers';
import { ThreadInfoPopover } from './ThreadInfoPopover';
import type { ThreadRuntimeInfo } from '@shared/contracts';
import { useI18n } from './i18n';

type ConversationHeaderActionsProps = {
  gatewayStatusLabel: string | null;
  gatewayStatusTone: 'syncing' | 'offline' | null;
  hasWorkspaceDirectory: boolean;
  inspectorOpen: boolean;
  isAutomationView: boolean;
  isBotsView: boolean;
  isSkillsView: boolean;
  selectedThreadId: string | null;
  teamSummary: ConversationTeamSummary | null;
  threadInfo: ThreadRuntimeInfo | null;
  threadInfoLoaded: boolean;
  threadLogsHasUnread: boolean;
  threadLogsOpen: boolean;
  onCreateAutomation: () => void;
  onOpenThread: (threadId: string) => void;
  onOpenThreads: () => void;
  onToggleInspector: () => void;
  onToggleThreadLogs: () => void;
};

function QueueIcon({ className }: { className?: string }) {
  return <IconPlugConnected aria-hidden className={className || 'icon'} size={14} stroke={1.6} />;
}

function DirectoryIcon() {
  return <IconFolder aria-hidden className="icon icon-tight" size={15} stroke={1.7} />;
}

export function ConversationHeaderActions({
  gatewayStatusLabel,
  gatewayStatusTone,
  hasWorkspaceDirectory,
  inspectorOpen,
  isAutomationView,
  isBotsView,
  isSkillsView,
  selectedThreadId,
  teamSummary,
  threadInfo,
  threadInfoLoaded,
  threadLogsHasUnread,
  threadLogsOpen,
  onCreateAutomation,
  onOpenThread,
  onOpenThreads,
  onToggleInspector,
  onToggleThreadLogs,
}: ConversationHeaderActionsProps) {
  const { t } = useI18n();
  return (
    <div className="conversation-header-actions">
      {isAutomationView ? (
        <button
          className="primary-button"
          onClick={onCreateAutomation}
          type="button"
        >
          <span>{t('New Automation')}</span>
        </button>
      ) : isBotsView ? (
        <button
          className="toolbar-button toolbar-button-strong utility-button"
          onClick={onOpenThreads}
          type="button"
        >
          <span>{t('Threads')}</span>
        </button>
      ) : isSkillsView ? null : (
        <>
          {teamSummary ? (
            <ConversationTeamMembers
              onOpenThread={onOpenThread}
              teamSummary={teamSummary}
            />
          ) : null}
          {gatewayStatusTone && gatewayStatusLabel ? (
            <div
              aria-live="polite"
              className={`gateway-status-pill is-${gatewayStatusTone}`}
              role="status"
              title={gatewayStatusLabel}
            >
              <span className="gateway-status-dot" />
              <span>{gatewayStatusLabel}</span>
            </div>
          ) : null}
          <ThreadInfoPopover
            threadId={selectedThreadId}
            threadInfo={threadInfo}
            threadInfoLoaded={threadInfoLoaded}
          />
          <button
            aria-expanded={threadLogsOpen}
            className={`conversation-header-action-button conversation-header-action-logs ${threadLogsHasUnread ? 'conversation-header-action-button-unread' : ''} ${threadLogsOpen ? 'is-open' : ''}`}
            disabled={!selectedThreadId}
            onClick={onToggleThreadLogs}
            type="button"
          >
            <QueueIcon />
            <span>{threadLogsOpen ? t('Close Logs') : t('Logs')}</span>
          </button>
          {selectedThreadId ? (
            <button
              aria-expanded={inspectorOpen}
              aria-label={inspectorOpen ? t('Hide file directory') : t('Show file directory')}
              className={`conversation-header-action-button conversation-header-action-icon ${inspectorOpen ? 'is-open' : ''}`}
              disabled={!hasWorkspaceDirectory}
              onClick={onToggleInspector}
              type="button"
            >
              <DirectoryIcon />
            </button>
          ) : null}
        </>
      )}
    </div>
  );
}
