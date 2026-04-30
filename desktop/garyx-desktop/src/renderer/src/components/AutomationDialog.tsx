import React from 'react';

import type {
  DesktopAutomationSchedule,
} from '@shared/contracts';
import type { AutomationAgentOption } from '@renderer/app-shell/types';

import { Button } from '@/components/ui/button';
import { DirectoryInput } from '@/components/DirectoryInput';
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Textarea } from '@/components/ui/textarea';
import { useI18n } from '@/i18n';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type AutomationDraft = {
  label: string;
  prompt: string;
  agentId: string;
  workspacePath: string;
  schedule: DesktopAutomationSchedule;
};

export type AutomationDialogState = {
  mode: 'create' | 'edit';
  automationId?: string;
  draft: AutomationDraft;
};

export interface AutomationDialogProps {
  state: AutomationDialogState;
  agentOptions: AutomationAgentOption[];
  saving: boolean;
  onDraftChange: (mutator: (draft: AutomationDraft) => AutomationDraft) => void;
  onSubmit: (event: React.FormEvent<HTMLFormElement>) => void;
  onClose: () => void;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function defaultDailySchedule(): DesktopAutomationSchedule {
  return {
    kind: 'daily',
    time: '09:00',
    weekdays: ['mo', 'tu', 'we', 'th', 'fr'],
    timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC',
  };
}

function formatDateTimeLocalValue(value: Date): string {
  const year = value.getFullYear();
  const month = String(value.getMonth() + 1).padStart(2, '0');
  const day = String(value.getDate()).padStart(2, '0');
  const hours = String(value.getHours()).padStart(2, '0');
  const minutes = String(value.getMinutes()).padStart(2, '0');
  return `${year}-${month}-${day}T${hours}:${minutes}`;
}

function defaultOnceSchedule(): DesktopAutomationSchedule {
  const value = new Date();
  value.setSeconds(0, 0);
  value.setMinutes(0);
  value.setHours(value.getHours() + 1);
  return {
    kind: 'once',
    at: formatDateTimeLocalValue(value),
  };
}

const WEEKDAYS = ['mo', 'tu', 'we', 'th', 'fr', 'sa', 'su'] as const;

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function AutomationDialog({
  state,
  agentOptions,
  saving,
  onDraftChange,
  onSubmit,
  onClose,
}: AutomationDialogProps) {
  const { t } = useI18n();
  const { mode, draft } = state;

  return (
    <Dialog open onOpenChange={(open) => { if (!open) onClose(); }}>
      <DialogContent className="sm:max-w-[680px]">
        <DialogHeader>
          <DialogDescription className="text-[10px] font-semibold uppercase tracking-[0.18em] text-muted-foreground">
            {t('Automation')}
          </DialogDescription>
          <DialogTitle className="text-base font-semibold">
            {mode === 'create' ? t('Create Automation') : t('Edit Automation')}
          </DialogTitle>
        </DialogHeader>

        <form
          className="grid gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            onSubmit(event);
          }}
        >
          {/* Name */}
          <div className="grid gap-2">
            <Label className="text-[12px] font-medium">{t('Name')}</Label>
            <Input
              autoFocus
              placeholder={t('Daily repo triage')}
              value={draft.label}
              onChange={(e) =>
                onDraftChange((d) => ({ ...d, label: e.target.value }))
              }
            />
          </div>

          {/* Agent */}
          <div className="grid gap-2">
            <Label className="text-[12px] font-medium">{t('Agent or Team')}</Label>
            <select
              className="h-10 w-full rounded-lg border border-[#e1e1e1] bg-white px-3 py-2 text-[13px] outline-none focus-visible:border-[#ccc] focus-visible:ring-[3px] focus-visible:ring-ring/50"
              value={draft.agentId}
              onChange={(e) =>
                onDraftChange((d) => ({ ...d, agentId: e.target.value }))
              }
            >
              {draft.agentId && !agentOptions.some((option) => option.id === draft.agentId) ? (
                <option value={draft.agentId}>
                  {t('{name} (unavailable)', { name: draft.agentId })}
                </option>
              ) : null}
              {agentOptions.map((option) => (
                <option key={option.id} value={option.id}>
                  {option.label}
                </option>
              ))}
            </select>
          </div>

          {/* Directory */}
          <div className="grid gap-2">
            <Label className="text-[12px] font-medium" htmlFor="automation-workspace-dir">
              {t('Directory')}
            </Label>
            <DirectoryInput
              id="automation-workspace-dir"
              onChange={(value) =>
                onDraftChange((d) => ({ ...d, workspacePath: value }))
              }
              placeholder={t('/path/to/project')}
              value={draft.workspacePath}
            />
          </div>

          {/* Prompt */}
          <div className="grid gap-2">
            <Label className="text-[12px] font-medium">{t('Prompt')}</Label>
            <Textarea
              placeholder={t('Summarize Sentry issues that need action.')}
              rows={6}
              value={draft.prompt}
              onChange={(e) =>
                onDraftChange((d) => ({ ...d, prompt: e.target.value }))
              }
            />
          </div>

          {/* Schedule */}
          <div className="grid gap-3">
            <Label className="text-[12px] font-medium">{t('Schedule')}</Label>
            <div className="flex items-center gap-2">
              <Button
                type="button"
                variant={draft.schedule.kind === 'daily' ? 'default' : 'outline'}
                size="sm"
                className="h-8 rounded-full px-4 text-[12px]"
                onClick={() =>
                  onDraftChange((d) => ({
                    ...d,
                    schedule: d.schedule.kind === 'daily' ? d.schedule : defaultDailySchedule(),
                  }))
                }
              >
                {t('Daily')}
              </Button>
              <Button
                type="button"
                variant={draft.schedule.kind === 'interval' ? 'default' : 'outline'}
                size="sm"
                className="h-8 rounded-full px-4 text-[12px]"
                onClick={() =>
                  onDraftChange((d) => ({
                    ...d,
                    schedule: d.schedule.kind === 'interval' ? d.schedule : { kind: 'interval', hours: 24 },
                  }))
                }
              >
                {t('Interval')}
              </Button>
              <Button
                type="button"
                variant={draft.schedule.kind === 'once' ? 'default' : 'outline'}
                size="sm"
                className="h-8 rounded-full px-4 text-[12px]"
                onClick={() =>
                  onDraftChange((d) => ({
                    ...d,
                    schedule: d.schedule.kind === 'once' ? d.schedule : defaultOnceSchedule(),
                  }))
                }
              >
                {t('Once')}
              </Button>
            </div>

            {draft.schedule.kind === 'daily' ? (
              <div className="grid grid-cols-2 gap-3">
                {/* Time */}
                <div className="grid gap-2">
                  <Label className="text-[11px] text-muted-foreground">{t('Time')}</Label>
                  <Input
                    type="time"
                    value={draft.schedule.time}
                    onChange={(e) =>
                      onDraftChange((d) =>
                        d.schedule.kind === 'daily'
                          ? { ...d, schedule: { ...d.schedule, time: e.target.value } }
                          : d,
                      )
                    }
                  />
                </div>
                {/* Timezone */}
                <div className="grid gap-2">
                  <Label className="text-[11px] text-muted-foreground">{t('Timezone')}</Label>
                  <Input
                    placeholder="Asia/Shanghai"
                    value={draft.schedule.timezone}
                    onChange={(e) =>
                      onDraftChange((d) =>
                        d.schedule.kind === 'daily'
                          ? { ...d, schedule: { ...d.schedule, timezone: e.target.value } }
                          : d,
                      )
                    }
                  />
                </div>
                {/* Weekdays */}
                <div className="col-span-2 flex flex-wrap items-center gap-2">
                  {WEEKDAYS.map((weekday) => {
                    const selected =
                      draft.schedule.kind === 'daily' &&
                      draft.schedule.weekdays.includes(weekday);
                    return (
                      <Button
                        key={weekday}
                        type="button"
                        variant={selected ? 'default' : 'outline'}
                        size="sm"
                        className="h-[34px] min-w-[42px] rounded-full px-3 text-[12px]"
                        onClick={() =>
                          onDraftChange((d) => {
                            if (d.schedule.kind !== 'daily') return d;
                            const weekdays = d.schedule.weekdays.includes(weekday)
                              ? d.schedule.weekdays.filter((w) => w !== weekday)
                              : [...d.schedule.weekdays, weekday];
                            return { ...d, schedule: { ...d.schedule, weekdays } };
                          })
                        }
                      >
                        {weekday.toUpperCase()}
                      </Button>
                    );
                  })}
                </div>
              </div>
            ) : draft.schedule.kind === 'interval' ? (
              <div className="grid gap-2">
                <Label className="text-[11px] text-muted-foreground">{t('Every')}</Label>
                <div className="flex items-center gap-2.5">
                  <Input
                    type="number"
                    min={1}
                    className="max-w-[120px]"
                    value={draft.schedule.hours}
                    onChange={(e) =>
                      onDraftChange((d) =>
                        d.schedule.kind === 'interval'
                          ? { ...d, schedule: { ...d.schedule, hours: Math.max(1, Number(e.target.value) || 1) } }
                          : d,
                      )
                    }
                  />
                  <span className="text-[12px] text-muted-foreground">{t('hours')}</span>
                </div>
              </div>
            ) : (
              <div className="grid gap-2">
                <Label className="text-[11px] text-muted-foreground">{t('Run At')}</Label>
                <Input
                  type="datetime-local"
                  value={draft.schedule.at}
                  onChange={(e) =>
                    onDraftChange((d) =>
                      d.schedule.kind === 'once'
                        ? { ...d, schedule: { ...d.schedule, at: e.target.value } }
                        : d,
                    )
                  }
                />
                <p className="text-[11px] text-muted-foreground">
                  {t("Uses this machine's local time.")}
                </p>
              </div>
            )}
          </div>

          {/* Actions */}
          <DialogFooter className="pt-2">
            <Button
              type="button"
              variant="outline"
              className="h-8 rounded-full px-4 text-[12px]"
              onClick={onClose}
            >
              {t('Cancel')}
            </Button>
            <Button
              type="submit"
              className="h-8 rounded-full px-4 text-[12px] shadow-sm active:scale-[0.96]"
              disabled={saving}
            >
              {saving ? t('Saving…') : mode === 'create' ? t('Create') : t('Save')}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
