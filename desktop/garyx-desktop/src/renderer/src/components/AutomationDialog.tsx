import React from 'react';

import type {
  DesktopAutomationSchedule,
  DesktopThreadSummary,
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
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
} from '@/components/ui/field';
import { Input } from '@/components/ui/input';
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { Textarea } from '@/components/ui/textarea';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import { AgentOptionRow } from '@/app-shell/components/AgentOptionAvatar';
import { useI18n } from '@/i18n';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type AutomationDraft = {
  label: string;
  prompt: string;
  agentId: string;
  targetMode: 'new_thread' | 'existing_thread';
  targetThreadId: string;
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
  threadOptions: DesktopThreadSummary[];
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
  threadOptions,
  saving,
  onDraftChange,
  onSubmit,
  onClose,
}: AutomationDialogProps) {
  const { t } = useI18n();
  const { mode, draft } = state;

  return (
    <Dialog open onOpenChange={(open) => { if (!open) onClose(); }}>
      <DialogContent className="sm:max-w-[680px]" size="form">
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
          <FieldGroup className="gap-4">
          <Field>
            <FieldLabel>{t('Name')}</FieldLabel>
            <Input
              autoFocus
              placeholder={t('Daily repo triage')}
              value={draft.label}
              onChange={(e) =>
                onDraftChange((d) => ({ ...d, label: e.target.value }))
              }
            />
          </Field>

          <Field>
            <FieldLabel>{t('Agent or Team')}</FieldLabel>
            <Select
              value={draft.agentId || undefined}
              onValueChange={(value) =>
                onDraftChange((d) => ({ ...d, agentId: value }))
              }
            >
              <SelectTrigger className="w-full">
                <SelectValue placeholder={t('Choose agent')} />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  <SelectLabel>{t('Agents')}</SelectLabel>
                  {draft.agentId && !agentOptions.some((option) => option.id === draft.agentId) ? (
                    <SelectItem value={draft.agentId}>
                      <AgentOptionRow
                        agentId={draft.agentId}
                        kind="agent"
                        label={t('{name} (unavailable)', { name: draft.agentId })}
                      />
                    </SelectItem>
                  ) : null}
                  {agentOptions.map((option) => (
                    <SelectItem key={option.id} value={option.id}>
                      <AgentOptionRow
                        option={option}
                      />
                    </SelectItem>
                  ))}
                </SelectGroup>
              </SelectContent>
            </Select>
          </Field>

          <Field>
            <FieldLabel>{t('Run In')}</FieldLabel>
            <ToggleGroup
              className="automation-schedule-toggle"
              type="single"
              value={draft.targetMode}
              onValueChange={(value) => {
                if (value === 'new_thread') {
                  onDraftChange((d) => ({ ...d, targetMode: 'new_thread', targetThreadId: '' }));
                } else if (value === 'existing_thread') {
                  onDraftChange((d) => {
                    const fallbackThread = threadOptions.find((thread) => thread.id === d.targetThreadId)
                      || threadOptions[0];
                    return {
                      ...d,
                      targetMode: 'existing_thread',
                      targetThreadId: fallbackThread?.id || d.targetThreadId,
                      workspacePath: fallbackThread?.workspacePath || d.workspacePath,
                    };
                  });
                }
              }}
              size="sm"
              variant="outline"
            >
              <ToggleGroupItem value="new_thread">
                {t('New Thread')}
              </ToggleGroupItem>
              <ToggleGroupItem value="existing_thread">
                {t('Existing Thread')}
              </ToggleGroupItem>
            </ToggleGroup>
            <FieldDescription>
              {draft.targetMode === 'existing_thread'
                ? t('Each run posts the prompt into the selected thread.')
                : t('Each run creates a fresh automation thread in the selected directory.')}
            </FieldDescription>
          </Field>

          {draft.targetMode === 'existing_thread' ? (
            <Field>
              <FieldLabel>{t('Thread')}</FieldLabel>
              <Select
                value={draft.targetThreadId || undefined}
                onValueChange={(value) =>
                  onDraftChange((d) => {
                    const thread = threadOptions.find((entry) => entry.id === value);
                    return {
                      ...d,
                      targetThreadId: value,
                      workspacePath: thread?.workspacePath || d.workspacePath,
                    };
                  })
                }
              >
                <SelectTrigger className="w-full">
                  <SelectValue placeholder={t('Choose thread')} />
                </SelectTrigger>
                <SelectContent>
                  <SelectGroup>
                    <SelectLabel>{t('Recent Threads')}</SelectLabel>
                    {draft.targetThreadId && !threadOptions.some((thread) => thread.id === draft.targetThreadId) ? (
                      <SelectItem value={draft.targetThreadId}>
                        {draft.targetThreadId}
                      </SelectItem>
                    ) : null}
                    {threadOptions.map((thread) => (
                      <SelectItem key={thread.id} value={thread.id}>
                        <div className="flex min-w-0 flex-col">
                          <span className="truncate">{thread.title || thread.id}</span>
                          {thread.workspacePath ? (
                            <span className="truncate text-[11px] text-muted-foreground">
                              {thread.workspacePath}
                            </span>
                          ) : null}
                        </div>
                      </SelectItem>
                    ))}
                  </SelectGroup>
                </SelectContent>
              </Select>
              {!threadOptions.length ? (
                <FieldDescription>
                  {t('No existing threads are loaded yet.')}
                </FieldDescription>
              ) : null}
            </Field>
          ) : (
            <Field>
              <FieldLabel htmlFor="automation-workspace-dir">
                {t('Directory')}
              </FieldLabel>
              <DirectoryInput
                id="automation-workspace-dir"
                onChange={(value) =>
                  onDraftChange((d) => ({ ...d, workspacePath: value }))
                }
                placeholder={t('/path/to/project')}
                value={draft.workspacePath}
              />
            </Field>
          )}

          <Field>
            <FieldLabel>{t('Prompt')}</FieldLabel>
            <Textarea
              placeholder={t('Summarize Sentry issues that need action.')}
              rows={6}
              value={draft.prompt}
              onChange={(e) =>
                onDraftChange((d) => ({ ...d, prompt: e.target.value }))
              }
            />
          </Field>

          <Field>
            <FieldLabel>{t('Schedule')}</FieldLabel>
            <ToggleGroup
              className="automation-schedule-toggle"
              type="single"
              value={draft.schedule.kind}
              onValueChange={(value) => {
                if (value === 'daily') {
                  onDraftChange((d) => ({
                    ...d,
                    schedule: d.schedule.kind === 'daily' ? d.schedule : defaultDailySchedule(),
                  }));
                } else if (value === 'interval') {
                  onDraftChange((d) => ({
                    ...d,
                    schedule: d.schedule.kind === 'interval' ? d.schedule : { kind: 'interval', hours: 24 },
                  }));
                } else if (value === 'once') {
                  onDraftChange((d) => ({
                    ...d,
                    schedule: d.schedule.kind === 'once' ? d.schedule : defaultOnceSchedule(),
                  }));
                }
              }}
              size="sm"
              variant="outline"
            >
              <ToggleGroupItem value="daily">
                {t('Daily')}
              </ToggleGroupItem>
              <ToggleGroupItem value="interval">
                {t('Interval')}
              </ToggleGroupItem>
              <ToggleGroupItem value="once">
                {t('Once')}
              </ToggleGroupItem>
            </ToggleGroup>

            {draft.schedule.kind === 'daily' ? (
              <div className="grid grid-cols-2 gap-3">
                <Field>
                  <FieldLabel>{t('Time')}</FieldLabel>
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
                </Field>
                <Field>
                  <FieldLabel>{t('Timezone')}</FieldLabel>
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
                </Field>
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
              <Field>
                <FieldLabel>{t('Every')}</FieldLabel>
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
              </Field>
            ) : (
              <Field>
                <FieldLabel>{t('Run At')}</FieldLabel>
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
                <FieldDescription>
                  {t("Uses this machine's local time.")}
                </FieldDescription>
              </Field>
            )}
          </Field>
          </FieldGroup>

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
