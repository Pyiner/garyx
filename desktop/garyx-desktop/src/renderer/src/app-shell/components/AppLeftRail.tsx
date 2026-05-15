import type { Dispatch, SetStateAction } from 'react';

import type { DesktopWorkspace } from '@shared/contracts';

import { SETTINGS_TABS, type SettingsTabId } from '../../settings-tabs';
import { BotSidebar } from '../../BotSidebar';
import { WorkspaceThreadSidebar } from '../../WorkspaceThreadSidebar';
import { UpdatePill } from './UpdatePill';
import { buildBotGroups } from '../../bot-console-model';
import { buildWorkspaceThreadGroups } from '../../thread-model';
import {
  AgentsIcon,
  AutomationIcon,
  AutoResearchIcon,
  BackIcon,
  BrowserIcon,
  NewThreadIcon,
  SettingsIcon,
  SettingsTabIcon,
  SkillsIcon,
  TasksIcon,
} from '../icons';
import { useI18n } from '../../i18n';

type AppLeftRailProps = {
  isSettingsView: boolean;
  isAutomationView: boolean;
  isAutoResearchView: boolean;
  isAgentsView: boolean;
  isTeamsView: boolean;
  isSkillsView: boolean;
  isTasksView: boolean;
  isBrowserView: boolean;
  showAutoResearch: boolean;
  settingsActiveTab: SettingsTabId;
  selectedAutomationId: string | null;
  activeBotConversationGroupId: string | null;
  activeWorkspaceThreadGroupPath: string | null;
  botGroups: ReturnType<typeof buildBotGroups>;
  workspaceThreadGroups: ReturnType<typeof buildWorkspaceThreadGroups>;
  renamingWorkspacePath: string | null;
  selectedThreadId: string | null;
  workspaceMenuOpenPath: string | null;
  workspaceMutation: 'add' | 'assign' | 'relink' | 'remove' | null;
  workspaceNameDraft: string;
  onSelectSettingsTab: (tabId: SettingsTabId) => void;
  onBackToThreads: () => void;
  onNewThread: () => void;
  onSelectAutomation: (automationId: string | null) => void;
  onOpenAutoResearch: () => void;
  onOpenBrowser: () => void;
  onOpenAgents: () => void;
  onOpenSkills: () => void;
  onOpenTasks: () => void;
  onOpenBot: (group: ReturnType<typeof buildBotGroups>[number]) => void;
  onToggleBotConversationGroup: (group: ReturnType<typeof buildBotGroups>[number]) => void;
  onToggleWorkspaceThreadGroup: (workspacePath: string) => void;
  onAddBot: () => void;
  onAddWorkspace: () => void;
  onBeginRenameWorkspace: (workspace: DesktopWorkspace) => void;
  onCancelRenameWorkspace: () => void;
  onCreateThreadForWorkspace: (workspacePath: string) => void;
  onRequestRemoveWorkspace: (workspace: DesktopWorkspace) => void;
  onSubmitRenameWorkspace: (workspacePath: string) => void;
  setWorkspaceMenuOpenPath: Dispatch<SetStateAction<string | null>>;
  setWorkspaceNameDraft: Dispatch<SetStateAction<string>>;
  onOpenSettings: () => void;
  onSidebarResizeStart: (event: React.PointerEvent<HTMLDivElement>) => void;
  sidebarResizing: boolean;
};

export function AppLeftRail({
  isSettingsView,
  isAutomationView,
  isAutoResearchView,
  isAgentsView,
  isTeamsView,
  isSkillsView,
  isTasksView,
  isBrowserView,
  showAutoResearch,
  settingsActiveTab,
  selectedAutomationId,
  activeBotConversationGroupId,
  activeWorkspaceThreadGroupPath,
  botGroups,
  workspaceThreadGroups,
  renamingWorkspacePath,
  selectedThreadId,
  workspaceMenuOpenPath,
  workspaceMutation,
  workspaceNameDraft,
  onSelectSettingsTab,
  onBackToThreads,
  onNewThread,
  onSelectAutomation,
  onOpenAutoResearch,
  onOpenBrowser,
  onOpenAgents,
  onOpenSkills,
  onOpenTasks,
  onOpenBot,
  onToggleBotConversationGroup,
  onToggleWorkspaceThreadGroup,
  onAddBot,
  onAddWorkspace,
  onBeginRenameWorkspace,
  onCancelRenameWorkspace,
  onCreateThreadForWorkspace,
  onRequestRemoveWorkspace,
  onSubmitRenameWorkspace,
  setWorkspaceMenuOpenPath,
  setWorkspaceNameDraft,
  onOpenSettings,
  onSidebarResizeStart,
  sidebarResizing,
}: AppLeftRailProps) {
  const { t } = useI18n();
  const isThreadView = !isSettingsView && !isAutomationView && !isAutoResearchView && !isAgentsView && !isTeamsView && !isSkillsView && !isTasksView && !isBrowserView;
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
              className={`sidebar-action ${isTasksView ? 'active' : ''}`}
              onClick={onOpenTasks}
              type="button"
            >
              <TasksIcon />
              <span>{t('Tasks')}</span>
            </button>
            <button
              className={`sidebar-action ${isBrowserView ? 'active' : ''}`}
              onClick={onOpenBrowser}
              type="button"
            >
              <BrowserIcon />
              <span>{t('Browser')}</span>
            </button>
            {showAutoResearch ? (
              <button
                className={`sidebar-action ${isAutoResearchView ? 'active' : ''}`}
                onClick={onOpenAutoResearch}
                type="button"
              >
                <AutoResearchIcon />
                <span>{t('Auto Research')}</span>
              </button>
            ) : null}
            <button
              className={`sidebar-action ${isAgentsView || isTeamsView ? 'active' : ''}`}
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
          </nav>

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
            onBeginRenameWorkspace={onBeginRenameWorkspace}
            onCancelRenameWorkspace={onCancelRenameWorkspace}
            onCreateThreadForWorkspace={onCreateThreadForWorkspace}
            onRequestRemoveWorkspace={onRequestRemoveWorkspace}
            onSubmitRenameWorkspace={onSubmitRenameWorkspace}
            onToggleWorkspaceThreads={onToggleWorkspaceThreadGroup}
            renamingWorkspacePath={renamingWorkspacePath}
            setWorkspaceMenuOpenPath={setWorkspaceMenuOpenPath}
            setWorkspaceNameDraft={setWorkspaceNameDraft}
            workspaceMenuOpenPath={workspaceMenuOpenPath}
            workspaceMutation={workspaceMutation}
            workspaceNameDraft={workspaceNameDraft}
            workspaceThreadGroups={workspaceThreadGroups}
          />

          <div className="sidebar-footer">
            <button
              className={`sidebar-action ${isSettingsView ? 'active' : ''}`}
              onClick={onOpenSettings}
              type="button"
            >
              <SettingsIcon />
              <span>{t('Settings')}</span>
            </button>
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
