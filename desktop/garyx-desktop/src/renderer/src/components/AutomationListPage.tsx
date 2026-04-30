import React from 'react';
import { Settings2 } from 'lucide-react';

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
    selectedWorkspace(state, automation.workspacePath)?.name
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
            const workspace = selectedWorkspace(desktopState, automation.workspacePath);
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
                    <Settings2 aria-hidden size={15} strokeWidth={1.8} />
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
