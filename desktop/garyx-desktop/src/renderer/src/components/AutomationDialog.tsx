import React from 'react';
import { Check, ChevronDown } from 'lucide-react';

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
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { Textarea } from '@/components/ui/textarea';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import {
  AgentOptionAvatar,
  AgentOptionRow,
} from '@/app-shell/components/AgentOptionAvatar';
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

function compactPath(value?: string | null): string {
  const trimmed = value?.trim() || '';
  if (!trimmed) return '';
  const parts = trimmed.split('/').filter(Boolean);
  if (parts.length <= 2) return trimmed;
  return `…/${parts.slice(-2).join('/')}`;
}

function threadTitle(thread: DesktopThreadSummary): string {
  return thread.title?.trim() || thread.id;
}

function threadSubtitle(thread: DesktopThreadSummary): string {
  return compactPath(thread.workspacePath) || thread.id;
}

function threadAgentOption(
  thread: DesktopThreadSummary | null,
  agentOptions: AutomationAgentOption[],
): AutomationAgentOption | null {
  if (!thread) return null;
  const teamId = thread.teamId?.trim();
  if (teamId) {
    const team = agentOptions.find((option) => option.kind === 'team' && option.id === teamId);
    if (team) return team;
  }
  const agentId = thread.agentId?.trim();
  if (agentId) {
    const agent = agentOptions.find((option) => option.id === agentId);
    if (agent) return agent;
  }
  return null;
}

function AutomationThreadPicker({
  agentOptions,
  fallbackAgent,
  value,
  threads,
  onChange,
}: {
  agentOptions: AutomationAgentOption[];
  fallbackAgent?: AutomationAgentOption | null;
  value: string;
  threads: DesktopThreadSummary[];
  onChange: (value: string) => void;
}) {
  const { t } = useI18n();
  const selectedThread = threads.find((thread) => thread.id === value) || null;
  const selectedAgent = threadAgentOption(selectedThread, agentOptions) ?? fallbackAgent;
  const missingThreadId = value.trim() && !selectedThread ? value.trim() : '';

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className="group flex min-h-12 w-full items-center gap-3 rounded-md border border-input bg-background px-3 py-2 text-left shadow-xs transition-colors outline-none hover:bg-[#fafaf9] focus-visible:ring-2 focus-visible:ring-ring/35"
        >
          <ThreadPickerAvatar
            agentId={selectedThread?.agentId}
            fallbackLabel={selectedThread ? threadTitle(selectedThread) : missingThreadId || t('Thread')}
            option={selectedAgent}
            teamId={selectedThread?.teamId}
          />
          <span className="min-w-0 flex-1">
            <span className="block truncate text-[13px] font-medium leading-5 text-foreground">
              {selectedThread ? threadTitle(selectedThread) : missingThreadId || t('Choose thread')}
            </span>
            <span className="block truncate text-[11px] leading-4 text-muted-foreground">
              {selectedThread
                ? threadSubtitle(selectedThread)
                : missingThreadId
                  ? t('Thread not loaded')
                  : t('Recent threads')}
            </span>
          </span>
          <ChevronDown
            aria-hidden
            className="size-4 shrink-0 text-muted-foreground transition-transform group-data-[state=open]:rotate-180"
            strokeWidth={1.8}
          />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        className="max-h-[340px] w-[var(--radix-dropdown-menu-trigger-width)] overflow-y-auto p-1.5"
      >
        <DropdownMenuLabel>{t('Recent Threads')}</DropdownMenuLabel>
        {missingThreadId ? (
          <DropdownMenuItem
            className="items-start gap-3 px-2.5 py-2.5"
            onSelect={() => onChange(missingThreadId)}
          >
            <ThreadPickerRow
              active
              agentOptions={agentOptions}
              fallbackAgent={fallbackAgent}
              subtitle={missingThreadId}
              title={missingThreadId}
            />
          </DropdownMenuItem>
        ) : null}
        {threads.length ? (
          threads.map((thread) => {
            const active = thread.id === value;
            return (
              <DropdownMenuItem
                key={thread.id}
                className="items-start gap-3 px-2.5 py-2.5"
                onSelect={() => onChange(thread.id)}
              >
                <ThreadPickerRow
                  active={active}
                  agentOptions={agentOptions}
                  fallbackAgent={fallbackAgent}
                  thread={thread}
                  subtitle={threadSubtitle(thread)}
                  title={threadTitle(thread)}
                />
              </DropdownMenuItem>
            );
          })
        ) : (
          <DropdownMenuItem disabled className="px-2.5 py-2 text-[12px] text-muted-foreground">
            {t('No existing threads are loaded yet.')}
          </DropdownMenuItem>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function ThreadPickerAvatar({
  agentId,
  fallbackLabel,
  option,
  teamId,
}: {
  agentId?: string | null;
  fallbackLabel: string;
  option?: AutomationAgentOption | null;
  teamId?: string | null;
}) {
  return (
    <AgentOptionAvatar
      agentId={option?.id ?? agentId ?? teamId}
      avatarDataUrl={option?.avatarDataUrl}
      className="mt-0.5"
      kind={option?.kind ?? (teamId ? 'team' : 'agent')}
      label={option?.label ?? fallbackLabel}
      providerIcon={option?.providerIcon}
      providerType={option?.providerType}
      size="sm"
    />
  );
}

function ThreadPickerRow({
  active,
  agentOptions,
  fallbackAgent,
  subtitle,
  thread,
  title,
}: {
  active: boolean;
  agentOptions: AutomationAgentOption[];
  fallbackAgent?: AutomationAgentOption | null;
  subtitle: string;
  thread?: DesktopThreadSummary | null;
  title: string;
}) {
  const option = threadAgentOption(thread ?? null, agentOptions) ?? fallbackAgent;

  return (
    <div className="flex min-w-0 flex-1 items-start gap-3">
      <ThreadPickerAvatar
        agentId={thread?.agentId}
        fallbackLabel={title}
        option={option}
        teamId={thread?.teamId}
      />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[13px] font-medium leading-5 text-foreground">
          {title}
        </span>
        <span className="block truncate text-[11px] leading-4 text-muted-foreground">
          {subtitle}
        </span>
      </span>
      <span className="mt-1 flex size-4 shrink-0 items-center justify-center text-foreground">
        {active ? <Check aria-hidden size={14} strokeWidth={2} /> : null}
      </span>
    </div>
  );
}

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
              <AutomationThreadPicker
                agentOptions={agentOptions}
                fallbackAgent={agentOptions.find((option) => option.id === draft.agentId)}
                value={draft.targetThreadId || ''}
                threads={threadOptions}
                onChange={(value) =>
                  onDraftChange((d) => {
                    const thread = threadOptions.find((entry) => entry.id === value);
                    return {
                      ...d,
                      targetThreadId: value,
                      workspacePath: thread?.workspacePath || d.workspacePath,
                    };
                  })
                }
              />
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
