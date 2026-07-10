import type { Dispatch, SetStateAction } from 'react';

import type { DesktopWorkspace } from '@shared/contracts';

import { SETTINGS_TABS, type SettingsTabId } from '../../settings-tabs';
import { BotSidebar } from '../../BotSidebar';
import { PinnedThreadsSidebar, type PinnedThreadRow } from '../../PinnedThreadsSidebar';
import { WorkspaceThreadSidebar } from '../../WorkspaceThreadSidebar';
import { UpdatePill } from './UpdatePill';
import { buildBotGroups } from '../../bot-console-model';
import { buildWorkspaceThreadGroups } from '../../thread-model';
import {
  AgentsIcon,
  AutomationIcon,
  BackIcon,
  CapsulesIcon,
  NewThreadIcon,
  RecentIcon,
  SettingsIcon,
  SettingsTabIcon,
  SkillsIcon,
  TasksIcon,
} from '../icons';
import { useI18n } from '../../i18n';

type AppLeftRailProps = {
  gatewayIdentitySlot?: React.ReactNode;
  isSettingsView: boolean;
  isAutomationView: boolean;
  isCapsulesView: boolean;
  isAgentsView: boolean;
  isSkillsView: boolean;
  isTasksView: boolean;
  isBrowserView: boolean;
  recentRailOpen: boolean;
  settingsActiveTab: SettingsTabId;
  selectedAutomationId: string | null;
  activeBotConversationGroupId: string | null;
  activeWorkspaceThreadGroupPath: string | null;
  botGroups: ReturnType<typeof buildBotGroups>;
  pinnedThreadRows: PinnedThreadRow[];
  workspaceThreadGroups: ReturnType<typeof buildWorkspaceThreadGroups>;
  selectedThreadId: string | null;
  workspaceMenuOpenPath: string | null;
  workspaceMutation: 'add' | 'assign' | 'relink' | 'remove' | null;
  onSelectSettingsTab: (tabId: SettingsTabId) => void;
  onBackToThreads: () => void;
  onNewThread: () => void;
  onOpenRecent: () => void;
  onSelectAutomation: (automationId: string | null) => void;
  onOpenCapsules: () => void;
  onOpenAgents: () => void;
  onOpenSkills: () => void;
  onOpenTasks: () => void;
  onOpenBot: (group: ReturnType<typeof buildBotGroups>[number]) => void;
  onOpenPinnedThread: (threadId: string) => void;
  onUnpinThread: (threadId: string) => void;
  onArchivePinnedThread: (threadId: string) => void;
  onToggleBotConversationGroup: (group: ReturnType<typeof buildBotGroups>[number]) => void;
  onToggleWorkspaceThreadGroup: (workspacePath: string) => void;
  onAddBot: () => void;
  onAddWorkspace: () => void;
  onCreateThreadForWorkspace: (workspacePath: string) => void;
  onRequestRemoveWorkspace: (workspace: DesktopWorkspace) => void;
  setWorkspaceMenuOpenPath: Dispatch<SetStateAction<string | null>>;
  onOpenSettings: () => void;
  onSidebarResizeStart: (event: React.PointerEvent<HTMLDivElement>) => void;
  sidebarResizing: boolean;
  formatThreadTimestamp: (value?: string | null) => string;
};

export function AppLeftRail({
  gatewayIdentitySlot,
  isSettingsView,
  isAutomationView,
  isCapsulesView,
  isAgentsView,
  isSkillsView,
  isTasksView,
  isBrowserView,
  recentRailOpen,
  settingsActiveTab,
  selectedAutomationId,
  activeBotConversationGroupId,
  activeWorkspaceThreadGroupPath,
  botGroups,
  pinnedThreadRows,
  workspaceThreadGroups,
  selectedThreadId,
  workspaceMenuOpenPath,
  workspaceMutation,
  onSelectSettingsTab,
  onBackToThreads,
  onNewThread,
  onOpenRecent,
  onSelectAutomation,
  onOpenCapsules,
  onOpenAgents,
  onOpenSkills,
  onOpenTasks,
  onOpenBot,
  onOpenPinnedThread,
  onUnpinThread,
  onArchivePinnedThread,
  onToggleBotConversationGroup,
  onToggleWorkspaceThreadGroup,
  onAddBot,
  onAddWorkspace,
  onCreateThreadForWorkspace,
  onRequestRemoveWorkspace,
  setWorkspaceMenuOpenPath,
  onOpenSettings,
  onSidebarResizeStart,
  sidebarResizing,
  formatThreadTimestamp,
}: AppLeftRailProps) {
  const { t } = useI18n();
  const isThreadView = !isSettingsView && !isAutomationView && !isCapsulesView && !isAgentsView && !isSkillsView && !isTasksView && !isBrowserView;
  const visibleSelectedThreadId = isThreadView ? selectedThreadId : null;
  return (
    <aside className={`left-rail ${isSettingsView ? 'settings-rail-shell' : ''}`}>
      {isSettingsView ? (
        <nav
          aria-label={t('Settings navigation')}
          className="sidebar-nav settings-sidebar-nav"
        >
          <button
            className="sidebar-action sidebar-back-action"
            onClick={onBackToThreads}
            type="button"
          >
            <BackIcon />
            <span>{t('Back to App')}</span>
          </button>

          <div className="settings-rail-list">
            {SETTINGS_TABS.map((tab) => (
              <button
                key={tab.id}
                className={`settings-rail-item ${tab.id === settingsActiveTab ? 'active' : ''}`}
                onClick={() => {
                  onSelectSettingsTab(tab.id);
                }}
                type="button"
              >
                <span className="settings-rail-item-icon">
                  <SettingsTabIcon tabId={tab.id} />
                </span>
                <span className="settings-rail-item-label">{t(tab.label)}</span>
              </button>
            ))}
          </div>
        </nav>
      ) : (
        <>
          <div className="sidebar-update-slot">
            <UpdatePill />
          </div>
          <nav
            aria-label={t('Primary actions')}
            className="sidebar-nav"
          >
            <button
              className="sidebar-action"
              onClick={onNewThread}
              type="button"
            >
              <NewThreadIcon />
              <span>{t('New Thread')}</span>
            </button>
            <button
              className={`sidebar-action ${isAutomationView ? 'active' : ''}`}
              onClick={() => {
                onSelectAutomation(selectedAutomationId);
              }}
              type="button"
            >
              <AutomationIcon />
              <span>{t('Automation')}</span>
            </button>
            <button
              className={`sidebar-action ${isCapsulesView ? 'active' : ''}`}
              onClick={onOpenCapsules}
              type="button"
            >
              <CapsulesIcon />
              <span>{t('Capsules')}</span>
            </button>
            <button
              className={`sidebar-action ${isTasksView ? 'active' : ''}`}
              onClick={onOpenTasks}
              type="button"
            >
              <TasksIcon />
              <span>{t('Tasks')}</span>
            </button>
            <button
              className={`sidebar-action ${isAgentsView ? 'active' : ''}`}
              onClick={onOpenAgents}
              type="button"
            >
              <AgentsIcon />
              <span>{t('Agents')}</span>
            </button>
            <button
              className={`sidebar-action ${isSkillsView ? 'active' : ''}`}
              onClick={onOpenSkills}
              type="button"
            >
              <SkillsIcon />
              <span>{t('Skills')}</span>
            </button>
            <button
              className={`sidebar-action ${recentRailOpen ? 'active' : ''}`}
              onClick={onOpenRecent}
              type="button"
            >
              <RecentIcon />
              <span>{t('Recent')}</span>
            </button>
          </nav>

          <div className="sidebar-scroll-area">
            <PinnedThreadsSidebar
              formatThreadTimestamp={formatThreadTimestamp}
              onArchiveThread={onArchivePinnedThread}
              onOpenThread={onOpenPinnedThread}
              onUnpinThread={onUnpinThread}
              rows={pinnedThreadRows}
            />

            <BotSidebar
              activeConversationGroupId={activeBotConversationGroupId}
              groups={botGroups}
              onAddBot={onAddBot}
              onOpenBot={onOpenBot}
              onToggleConversationGroup={onToggleBotConversationGroup}
              selectedThreadId={visibleSelectedThreadId}
            />

            <WorkspaceThreadSidebar
              activeWorkspacePath={activeWorkspaceThreadGroupPath}
              onAddWorkspace={onAddWorkspace}
              onCreateThreadForWorkspace={onCreateThreadForWorkspace}
              onRequestRemoveWorkspace={onRequestRemoveWorkspace}
              onToggleWorkspaceThreads={onToggleWorkspaceThreadGroup}
              setWorkspaceMenuOpenPath={setWorkspaceMenuOpenPath}
              workspaceMenuOpenPath={workspaceMenuOpenPath}
              workspaceMutation={workspaceMutation}
              workspaceThreadGroups={workspaceThreadGroups}
            />
          </div>

          <div className="sidebar-footer">
            {gatewayIdentitySlot ?? (
              <button
                className={`sidebar-action ${isSettingsView ? 'active' : ''}`}
                onClick={onOpenSettings}
                type="button"
              >
                <SettingsIcon />
                <span>{t('Settings')}</span>
              </button>
            )}
          </div>
        </>
      )}
      <div
        className={`sidebar-resizer ${sidebarResizing ? "is-resizing" : ""}`}
        onPointerDown={onSidebarResizeStart}
      >
        <div className="sidebar-resizer-line" />
      </div>
    </aside>
  );
}
