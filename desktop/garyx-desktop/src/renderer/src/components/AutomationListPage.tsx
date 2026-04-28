import React from 'react';

import type {
  DesktopAutomationSchedule,
  DesktopAutomationSummary,
  DesktopCustomAgent,
  DesktopState,
} from '@shared/contracts';

import { useI18n, type Translate } from '@/i18n';
import { selectedWorkspace } from '@/thread-model';

// ---------------------------------------------------------------------------
// Helpers (migrated from App.tsx)
// ---------------------------------------------------------------------------

function formatTimestamp(value?: string | null): string {
  if (!value) return '';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return '';
  const now = new Date();
  const sameDay = date.toDateString() === now.toDateString();
  return new Intl.DateTimeFormat(undefined, sameDay
    ? { hour: 'numeric', minute: '2-digit' }
    : { month: 'short', day: 'numeric' },
  ).format(date);
}

function compactPathLabel(path?: string | null): string {
  const trimmed = path?.trim() || '';
  if (!trimmed) return '';
  const segments = trimmed.split(/[\\/]/).filter(Boolean);
  return segments[segments.length - 1] || trimmed;
}

function getWorkspaceLabel(
  state: DesktopState | null,
  automation: DesktopAutomationSummary,
  t: Translate,
): string {
  return (
    selectedWorkspace(state, automation.workspaceId)?.name
    || compactPathLabel(automation.workspacePath)
    || t('Workspace unavailable')
  );
}

function getAgentLabel(
  agents: DesktopCustomAgent[],
  automation: DesktopAutomationSummary,
): string {
  const match = agents.find((agent) => agent.agentId === automation.agentId);
  return match?.displayName || automation.agentId || 'Claude';
}

function formatOneTimeSchedule(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return '';
  const date = new Date(trimmed);
  if (Number.isNaN(date.getTime())) return trimmed;
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  }).format(date);
}

function formatSchedule(schedule: DesktopAutomationSchedule, t: Translate): string {
  if (schedule.kind === 'interval') {
    return t('Every {hours}h', { hours: schedule.hours });
  }
  if (schedule.kind === 'once') {
    return t('One-time · {time}', { time: formatOneTimeSchedule(schedule.at) || schedule.at });
  }
  const weekdays = schedule.weekdays.length
    ? schedule.weekdays.map((d) => d.slice(0, 2).toUpperCase()).join(' ')
    : t('Daily');
  return `${schedule.time} · ${weekdays} · ${schedule.timezone}`;
}

function statusInfo(automation: DesktopAutomationSummary) {
  if (
    automation.schedule.kind === 'once'
    && !automation.enabled
    && automation.lastStatus === 'success'
    && automation.lastRunAt
  ) {
    return { label: 'Completed', pillClass: '' as const };
  }
  if (automation.lastStatus === 'failed') return { label: 'Failed', pillClass: 'fail' as const };
  if (automation.lastStatus === 'skipped') return { label: 'Skipped', pillClass: '' as const };
  if (automation.schedule.kind === 'once' && automation.enabled) {
    return { label: 'Scheduled', pillClass: 'ok' as const };
  }
  if (!automation.enabled) return { label: 'Paused', pillClass: '' as const };
  return { label: 'Healthy', pillClass: 'ok' as const };
}

function hasUnread(
  state: DesktopState | null,
  automation: DesktopAutomationSummary,
): boolean {
  const unreadAt = automation.unreadHintTimestamp || automation.lastRunAt || null;
  if (!unreadAt) return false;
  const seenAt = state?.lastSeenRunAtByAutomation?.[automation.id];
  if (!seenAt) return true;
  return unreadAt > seenAt;
}

// ---------------------------------------------------------------------------
// SVG icons
// ---------------------------------------------------------------------------

const PlusIcon = (
  <svg aria-hidden width="14" height="14" viewBox="0 0 20 20" fill="none">
    <path d="M9.33496 16.5V10.665H3.5C3.13273 10.665 2.83496 10.3673 2.83496 10C2.83496 9.63273 3.13273 9.33496 3.5 9.33496H9.33496V3.5C9.33496 3.13273 9.63273 2.83496 10 2.83496C10.3673 2.83496 10.665 3.13273 10.665 3.5V9.33496H16.5C16.8673 9.33496 17.165 9.63273 17.165 10C17.165 10.3673 16.8673 10.665 16.5 10.665H10.665V16.5C10.665 16.8673 10.3673 17.165 10 17.165C9.63273 17.165 9.33496 16.8673 9.33496 16.5Z" fill="currentColor"/>
  </svg>
);

const GearIcon = (
  <svg aria-hidden width="18" height="18" viewBox="0 0 21 21" fill="none">
    <path d="M10.7228 2.53564C11.5515 2.53564 12.3183 2.97502 12.7374 3.68994L13.5587 5.09033L13.6124 5.15967C13.6736 5.22007 13.7566 5.2556 13.8448 5.25635L15.4601 5.26904L15.6144 5.27588C16.3826 5.33292 17.0775 5.76649 17.465 6.43994L17.7931 7.01123L17.8663 7.14697C18.1815 7.78943 18.1843 8.54208 17.8741 9.18701L17.8028 9.32275L17.0001 10.7446C16.9427 10.8467 16.9426 10.9717 17.0001 11.0737L17.8028 12.4946L17.8741 12.6313C18.1842 13.2763 18.1816 14.029 17.8663 14.6714L17.7931 14.8071L17.465 15.3784C17.0774 16.0517 16.3825 16.4855 15.6144 16.5425L15.4601 16.5483L13.8448 16.562C13.7565 16.5628 13.6736 16.5982 13.6124 16.6587L13.5587 16.7271L12.7374 18.1284C12.3183 18.8432 11.5514 19.2827 10.7228 19.2827H10.0763C9.29958 19.2826 8.57714 18.8964 8.14465 18.2593L8.06261 18.1284L7.24133 16.7271C7.1966 16.6509 7.12417 16.5966 7.04113 16.5737L6.95519 16.562L5.33996 16.5483C4.56297 16.542 3.84347 16.1503 3.41613 15.5093L3.33508 15.3784L3.00695 14.8071C2.59564 14.0921 2.59168 13.2129 2.99719 12.4946L3.79894 11.0737L3.83215 10.9937C3.84657 10.9383 3.84652 10.88 3.83215 10.8247L3.79894 10.7446L2.99719 9.32275C2.59184 8.60451 2.59571 7.72612 3.00695 7.01123L3.33508 6.43994L3.41613 6.30908C3.84345 5.66796 4.56288 5.27538 5.33996 5.26904L6.95519 5.25635L7.04113 5.24463C7.12427 5.22177 7.1966 5.16664 7.24133 5.09033L8.06261 3.68994L8.14465 3.55908C8.57712 2.92179 9.29949 2.5358 10.0763 2.53564H10.7228ZM11.9855 10.9087C11.9853 10.0336 11.2755 9.32399 10.4005 9.32373C9.52524 9.32373 8.81474 10.0335 8.81457 10.9087C8.81457 11.7841 9.52513 12.4937 10.4005 12.4937C11.2757 12.4934 11.9855 11.7839 11.9855 10.9087ZM13.3146 10.9087C13.3146 12.5184 12.0102 13.8235 10.4005 13.8237C8.7906 13.8237 7.48547 12.5186 7.48547 10.9087C7.48564 9.29893 8.7907 7.99365 10.4005 7.99365C12.0101 7.99391 13.3144 9.29909 13.3146 10.9087Z" fill="currentColor"/>
  </svg>
);

const TrashIcon = (
  <svg aria-hidden width="16" height="16" viewBox="0 0 20 20" fill="none">
    <path d="M5.5 2.5H14.5V4.5H5.5V2.5ZM3.5 5.5H16.5V6.83333H15.1667V15.5C15.1667 16.2364 14.5697 16.8333 13.8333 16.8333H6.16667C5.43029 16.8333 4.83333 16.2364 4.83333 15.5V6.83333H3.5V5.5ZM6.16667 6.83333V15.5H13.8333V6.83333H6.16667ZM8.16667 8.83333H9.5V13.5H8.16667V8.83333ZM10.5 8.83333H11.8333V13.5H10.5V8.83333Z" fill="currentColor"/>
  </svg>
);

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

export interface AutomationListPageProps {
  automations: DesktopAutomationSummary[];
  agents: DesktopCustomAgent[];
  desktopState: DesktopState | null;
  automationMutation: string | null;
  onRunNow: (automation: DesktopAutomationSummary) => void;
  onToggleEnabled: (automation: DesktopAutomationSummary, enabled: boolean) => void;
  onEdit: (automation: DesktopAutomationSummary) => void;
  onOpenMemory: (automation: DesktopAutomationSummary) => void;
  onOpenThread: (automation: DesktopAutomationSummary) => void;
  onDelete: (automation: DesktopAutomationSummary) => void;
  onCreateAutomation: () => void;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function AutomationListPage({
  automations,
  agents,
  desktopState,
  automationMutation,
  onRunNow,
  onToggleEnabled,
  onEdit,
  onOpenMemory,
  onOpenThread,
  onDelete,
  onCreateAutomation,
}: AutomationListPageProps) {
  const { t } = useI18n();

  return (
    <div className="codex-section" style={{ padding: '20px 20px 0', height: '100%', display: 'flex', flexDirection: 'column', minHeight: 0, overflow: 'hidden' }}>
      <div className="codex-section-header">
        <span className="codex-section-title">{t('Automations')}</span>
        <button className="codex-section-action" onClick={onCreateAutomation} type="button">
          {PlusIcon} {t('New')}
        </button>
      </div>

      {!automations.length ? (
        <div className="codex-empty-state">{t('No automations yet. Create your first scheduled prompt.')}</div>
      ) : (
        <div className="codex-list-card" style={{ flex: '1 1 0', minHeight: 0, overflowY: 'auto' }}>
          {automations.map((automation) => {
            const wsLabel = getWorkspaceLabel(desktopState, automation, t);
            const agentLabel = getAgentLabel(agents, automation);
            const workspace = selectedWorkspace(desktopState, automation.workspaceId);
            const nextTitle = automation.schedule.kind === 'once' ? t('Run At') : t('Next');
            const nextRunLabel = automation.schedule.kind === 'once'
              ? formatOneTimeSchedule(automation.schedule.at) || formatTimestamp(automation.nextRun) || automation.nextRun
              : formatTimestamp(automation.nextRun) || automation.nextRun;
            const isRunning = automationMutation === `run:${automation.id}`;
            const isToggling = automationMutation === `toggle:${automation.id}`;
            const isDeleting = automationMutation === `delete:${automation.id}`;
            const unread = hasUnread(desktopState, automation);
            const status = statusInfo(automation);

            return (
              <div
                className="codex-list-row"
                key={automation.id}
                style={{ minHeight: 64, padding: '8px 16px', opacity: automation.enabled ? 1 : 0.55 }}
              >
                <div style={{ display: 'flex', flexDirection: 'column', gap: 4, minWidth: 0, flex: 1 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                    <span className="codex-list-row-name">{automation.label}</span>
                    {unread && (
                      <span style={{ width: 6, height: 6, borderRadius: '50%', background: '#4f8df7', flexShrink: 0 }} />
                    )}
                  </div>
                  <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap', alignItems: 'center' }}>
                    <span className={`codex-sync-pill ${status.pillClass}`}>{t(status.label)}</span>
                    <span className="codex-sync-pill">{agentLabel}</span>
                    <span className="codex-sync-pill ok">{formatSchedule(automation.schedule, t)}</span>
                    {!workspace?.available && (
                      <span className="codex-sync-pill fail">{t('Workspace unavailable')}</span>
                    )}
                    <span className="codex-command-row-desc">
                      {wsLabel}
                      {automation.lastRunAt ? ` · ${t('Last')}: ${formatTimestamp(automation.lastRunAt)}` : ''}
                      {nextRunLabel ? ` · ${nextTitle}: ${nextRunLabel}` : ''}
                    </span>
                  </div>
                </div>
                <div className="codex-list-row-actions">
                  <button
                    className="codex-section-action"
                    disabled={isRunning}
                    onClick={() => onRunNow(automation)}
                    type="button"
                  >
                    {isRunning ? t('Running...') : t('Run')}
                  </button>
                  <button
                    className="codex-section-action"
                    disabled={isToggling}
                    onClick={() => onToggleEnabled(automation, !automation.enabled)}
                    type="button"
                  >
                    {automation.enabled ? t('Pause') : t('Resume')}
                  </button>
                  <button
                    className="codex-section-action"
                    onClick={() => onOpenMemory(automation)}
                    type="button"
                  >
                    {t('Memory')}
                  </button>
                  <button
                    className="codex-icon-button"
                    onClick={() => onEdit(automation)}
                    title={t('Edit')}
                    type="button"
                  >
                    {GearIcon}
                  </button>
                  <button
                    className="codex-section-action"
                    disabled={!automation.threadId}
                    onClick={() => onOpenThread(automation)}
                    type="button"
                  >
                    {t('Thread')}
                  </button>
                  <button
                    className="codex-icon-button codex-icon-button-danger"
                    disabled={isDeleting}
                    onClick={() => onDelete(automation)}
                    title={t('Delete')}
                    type="button"
                  >
                    {TrashIcon}
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
