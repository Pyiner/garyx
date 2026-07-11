import React from 'react';
import { Plus, Settings2, Trash } from 'lucide-react';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { MoreDotsIcon } from '../app-shell/icons';

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
    || t('Workspace not set')
  );
}

function getTargetLabel(
  state: DesktopState | null,
  automation: DesktopAutomationSummary,
  workspaceLabel: string,
  t: Translate,
): string {
  const targetThreadId = automation.targetThreadId?.trim();
  if (!targetThreadId) {
    return workspaceLabel;
  }
  const thread = state?.threads.find((entry) => entry.id === targetThreadId);
  return t('Thread: {name}', { name: thread?.title || targetThreadId });
}

function getAgentLabel(
  state: DesktopState | null,
  agents: DesktopCustomAgent[],
  automation: DesktopAutomationSummary,
): string | null {
  const targetThreadId = automation.targetThreadId?.trim();
  if (targetThreadId) {
    // A thread-bound automation runs under the thread's own agent; the
    // automation-level agent does not apply. Derive the pill from the
    // thread, and show none rather than a wrong default when unknown.
    const thread = state?.threads.find((entry) => entry.id === targetThreadId);
    const threadAgentId = thread?.agentId?.trim();
    if (!threadAgentId) {
      return null;
    }
    const match = agents.find((agent) => agent.agentId === threadAgentId);
    return match?.displayName || threadAgentId;
  }
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
  <Plus aria-hidden size={14} />
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
    <div className="codex-section" style={{ padding: '6px 20px 0', height: '100%', display: 'flex', flexDirection: 'column', minHeight: 0, overflow: 'hidden' }}>
      <div className="mgmt-page-header">
        <div className="mgmt-page-title-block">
          <h1 className="mgmt-page-title">{t('Automations')}</h1>
          <p className="mgmt-page-subtitle">{t('{count} total', { count: automations.length })}</p>
        </div>
        <div className="mgmt-page-actions">
          <button className="mgmt-primary-button" onClick={onCreateAutomation} type="button">
            {PlusIcon} {t('New')}
          </button>
        </div>
      </div>

      {!automations.length ? (
        <div className="codex-empty-state">{t('No automations yet. Create your first scheduled prompt.')}</div>
      ) : (
        <div className="codex-list-card" style={{ flex: '1 1 0', minHeight: 0, overflowY: 'auto' }}>
          {automations.map((automation) => {
            const wsLabel = getWorkspaceLabel(desktopState, automation, t);
            const targetLabel = getTargetLabel(desktopState, automation, wsLabel, t);
            const agentLabel = getAgentLabel(desktopState, agents, automation);
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
                    {agentLabel ? (
                      <span className="codex-sync-pill">{agentLabel}</span>
                    ) : null}
                    <span className="codex-sync-pill ok">{formatSchedule(automation.schedule, t)}</span>
                    {workspace && !workspace.available && (
                      <span className="codex-sync-pill fail">{t('Workspace unavailable')}</span>
                    )}
                    <span className="codex-command-row-desc">
                      {targetLabel}
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
                    disabled={!automation.threadId && !automation.targetThreadId}
                    onClick={() => onOpenThread(automation)}
                    type="button"
                  >
                    {t('Thread')}
                  </button>
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <button
                        aria-label={t('More actions for {name}', { name: automation.label })}
                        className="bot-table-action-button"
                        type="button"
                      >
                        <MoreDotsIcon size={14} />
                      </button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end" sideOffset={4}>
                      <DropdownMenuItem
                        disabled={isDeleting}
                        onSelect={() => onDelete(automation)}
                        variant="destructive"
                      >
                        <Trash aria-hidden />
                        {t('Delete')}
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
