import type { Dispatch, SetStateAction } from 'react';

import type { DesktopChannelEndpoint, DesktopState, DesktopWorkspace } from '@shared/contracts';

import { SETTINGS_TABS, type SettingsTabId } from '../../GatewaySettingsPanel';
import { BotSidebar } from '../../BotSidebar';
import { WorkspaceThreadSidebar } from '../../WorkspaceThreadSidebar';
import { UpdatePill } from './UpdatePill';
import { buildBotGroups } from '../../bot-console-model';
import { buildWorkspaceThreadGroups } from '../../thread-model';
import type { ContentView } from '../types';
import {
  AgentsIcon,
  AutomationIcon,
  AutoResearchIcon,
  BackIcon,
  BrowserIcon,
  MemoryIcon,
  NewThreadIcon,
  SettingsIcon,
  SettingsTabIcon,
  SkillsIcon,
} from '../icons';
import { useI18n } from '../../i18n';

type AppLeftRailProps = {
  isSettingsView: boolean;
  isAutomationView: boolean;
  isAutoResearchView: boolean;
  isAgentsView: boolean;
  isTeamsView: boolean;
  isSkillsView: boolean;
  isBrowserView: boolean;
  showAutoResearch: boolean;
  settingsActiveTab: SettingsTabId;
  selectedAutomationId: string | null;
  botGroups: ReturnType<typeof buildBotGroups>;
  desktopState: DesktopState | null;
  deletingThreadId: string | null;
  formatThreadTimestamp: (value?: string | null) => string;
  isThreadRuntimeBusy: (threadId: string) => boolean;
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
  onOpenMemory: () => void;
  onOpenBot: (group: ReturnType<typeof buildBotGroups>[number]) => void;
  onOpenBotEndpoint: (endpoint: DesktopChannelEndpoint) => void;
  onAddBot: () => void;
  onBeginRenameWorkspace: (workspace: DesktopWorkspace) => void;
  onCancelRenameWorkspace: () => void;
  onCreateThreadForWorkspace: (workspacePath: string) => void;
  onDeleteThread: (threadId: string) => void;
  onOpenFolder: () => void;
  onOpenThread: (threadId: string) => void;
  onRequestRemoveWorkspace: (workspace: DesktopWorkspace) => void;
  onSelectWorkspace: (workspacePath: string, preferredThreadId?: string | null) => void;
  onSubmitRenameWorkspace: (workspacePath: string) => void;
  setContentView: Dispatch<SetStateAction<ContentView>>;
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
  isBrowserView,
  showAutoResearch,
  settingsActiveTab,
  selectedAutomationId,
  botGroups,
  desktopState,
  deletingThreadId,
  formatThreadTimestamp,
  isThreadRuntimeBusy,
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
  onOpenMemory,
  onOpenBot,
  onOpenBotEndpoint,
  onAddBot,
  onBeginRenameWorkspace,
  onCancelRenameWorkspace,
  onCreateThreadForWorkspace,
  onDeleteThread,
  onOpenFolder,
  onOpenThread,
  onRequestRemoveWorkspace,
  onSelectWorkspace,
  onSubmitRenameWorkspace,
  setContentView,
  setWorkspaceMenuOpenPath,
  setWorkspaceNameDraft,
  onOpenSettings,
  onSidebarResizeStart,
  sidebarResizing,
}: AppLeftRailProps) {
  const { t } = useI18n();
  const isThreadView = !isSettingsView && !isAutomationView && !isAutoResearchView && !isAgentsView && !isTeamsView && !isSkillsView && !isBrowserView;
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
            <button
              className="sidebar-action"
              onClick={onOpenMemory}
              type="button"
            >
              <MemoryIcon />
              <span>{t('Memory')}</span>
            </button>
          </nav>

          <BotSidebar
            formatThreadTimestamp={formatThreadTimestamp}
            groups={botGroups}
            onAddBot={onAddBot}
            onOpenBot={onOpenBot}
            onOpenEndpoint={onOpenBotEndpoint}
            selectedThreadId={visibleSelectedThreadId}
          />

          <WorkspaceThreadSidebar
            deletingThreadId={deletingThreadId}
            desktopState={desktopState}
            formatThreadTimestamp={formatThreadTimestamp}
            isThreadRuntimeBusy={isThreadRuntimeBusy}
            onBeginRenameWorkspace={onBeginRenameWorkspace}
            onCancelRenameWorkspace={onCancelRenameWorkspace}
            onCreateThreadForWorkspace={onCreateThreadForWorkspace}
            onDeleteThread={onDeleteThread}
            onOpenFolder={onOpenFolder}
            onOpenThread={onOpenThread}
            onRequestRemoveWorkspace={onRequestRemoveWorkspace}
            onSelectWorkspace={onSelectWorkspace}
            onSubmitRenameWorkspace={onSubmitRenameWorkspace}
            renamingWorkspacePath={renamingWorkspacePath}
            selectedThreadId={visibleSelectedThreadId}
            setContentView={setContentView}
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
