import { ThreadInfoPopover } from './ThreadInfoPopover';
import { PanelIcon } from './app-shell/icons';
import type { ThreadRuntimeInfo } from '@shared/contracts';
import { useI18n } from './i18n';

type ConversationHeaderActionsProps = {
  gatewayStatusLabel: string | null;
  gatewayStatusTone: 'syncing' | 'offline' | null;
  inspectorOpen: boolean;
  isAutomationView: boolean;
  isBotsView: boolean;
  isSkillsView: boolean;
  selectedThreadId: string | null;
  threadInfo: ThreadRuntimeInfo | null;
  threadInfoLoaded: boolean;
  onCreateAutomation: () => void;
  onOpenThreads: () => void;
  onToggleInspector: () => void;
};

function DirectoryIcon() {
  return <PanelIcon />;
}

export function ConversationHeaderActions({
  gatewayStatusLabel,
  gatewayStatusTone,
  inspectorOpen,
  isAutomationView,
  isBotsView,
  isSkillsView,
  selectedThreadId,
  threadInfo,
  threadInfoLoaded,
  onCreateAutomation,
  onOpenThreads,
  onToggleInspector,
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
          {selectedThreadId ? (
            <span
              className="thread-task-tree-header-slot"
              data-thread-task-tree-trigger-host
            />
          ) : null}
          <ThreadInfoPopover
            threadId={selectedThreadId}
            threadInfo={threadInfo}
            threadInfoLoaded={threadInfoLoaded}
          />
          {selectedThreadId ? (
            <button
              aria-expanded={inspectorOpen}
              aria-label={inspectorOpen ? t('Hide side tools') : t('Show side tools')}
              className={`conversation-header-action-button conversation-header-action-icon ${inspectorOpen ? 'is-open' : ''}`}
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
