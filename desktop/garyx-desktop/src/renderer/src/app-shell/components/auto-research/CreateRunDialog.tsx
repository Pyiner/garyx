import { useState } from 'react';

import type {
  CreateAutoResearchRunInput,
  DesktopWorkspace,
} from '@shared/contracts';

import { Button } from '../../../components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../../../components/ui/dialog';
import { Input } from '../../../components/ui/input';
import { Label } from '../../../components/ui/label';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../../../components/ui/select';
import { Textarea } from '../../../components/ui/textarea';

import {
  DEFAULT_MAX_ITERATIONS,
  DEFAULT_TIME_BUDGET_MINUTES,
} from './types';
import { useI18n } from '../../../i18n';

type CreateRunDialogProps = {
  saving: boolean;
  workspaces: DesktopWorkspace[];
  defaultWorkspacePath: string;
  onSubmit: (input: CreateAutoResearchRunInput) => Promise<void>;
  onClose: () => void;
};

export function CreateRunDialog({
  saving,
  workspaces,
  defaultWorkspacePath,
  onSubmit,
  onClose,
}: CreateRunDialogProps) {
  const { t } = useI18n();
  const [goal, setGoal] = useState('');
  const [maxIterations, setMaxIterations] = useState(DEFAULT_MAX_ITERATIONS);
  const [timeBudgetMinutes, setTimeBudgetMinutes] = useState(DEFAULT_TIME_BUDGET_MINUTES);
  const [selectedWorkspacePath, setSelectedWorkspacePath] = useState(defaultWorkspacePath);
  const [showAdvanced, setShowAdvanced] = useState(false);

  const selectableWorkspaces = workspaces.filter((workspace) => workspace.available && workspace.path);

  const parsedMaxIterations = Number.parseInt(maxIterations, 10);
  const parsedTimeBudgetMinutes = Number.parseInt(timeBudgetMinutes, 10);
  const canCreateRun = Boolean(
    goal.trim()
    && selectedWorkspacePath
    && Number.isFinite(parsedMaxIterations)
    && parsedMaxIterations > 0
    && Number.isFinite(parsedTimeBudgetMinutes)
    && parsedTimeBudgetMinutes > 0,
  );

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();

    await onSubmit({
      goal,
      workspaceDir: selectedWorkspacePath || undefined,
      maxIterations: Number.isFinite(parsedMaxIterations) && parsedMaxIterations > 0
        ? parsedMaxIterations
        : undefined,
      timeBudgetSecs: Number.isFinite(parsedTimeBudgetMinutes) && parsedTimeBudgetMinutes > 0
        ? parsedTimeBudgetMinutes * 60
        : undefined,
    });
  }

  return (
    <Dialog open onOpenChange={(open) => { if (!open) onClose(); }}>
      <DialogContent className="sm:max-w-[720px]">
        <DialogHeader>
          <DialogTitle>{t('Create Auto Research Run')}</DialogTitle>
          <DialogDescription>
            {t('Define the goal, workspace, and budget for a bounded work-and-verify loop.')}
          </DialogDescription>
        </DialogHeader>

        <form
          style={{ display: 'grid', gap: 16 }}
          onSubmit={(event) => {
            void handleSubmit(event);
          }}
        >
          <div className="codex-form-field">
            <Label className="codex-form-label" htmlFor="auto-research-goal">{t('Goal')}</Label>
            <Textarea
              id="auto-research-goal"
              onChange={(event) => setGoal(event.target.value)}
              placeholder={t('What should Garyx figure out or produce?')}
              rows={5}
              value={goal}
            />
          </div>

          <div className="codex-form-field">
            <Label className="codex-form-label" htmlFor="auto-research-workspace">{t('Workspace')}</Label>
            <Select onValueChange={setSelectedWorkspacePath} value={selectedWorkspacePath}>
              <SelectTrigger aria-label={t('Workspace')} className="w-full" id="auto-research-workspace">
                <SelectValue placeholder={t('Choose workspace')} />
              </SelectTrigger>
              <SelectContent>
                {selectableWorkspaces.length ? (
                  selectableWorkspaces.map((workspace) => (
                    <SelectItem key={workspace.path || workspace.name} value={workspace.path || ''}>
                      {workspace.name}
                    </SelectItem>
                  ))
                ) : (
                  <SelectItem disabled value="no-workspaces">{t('No workspaces available')}</SelectItem>
                )}
              </SelectContent>
            </Select>
          </div>

          <button
            type="button"
            className="ar-advanced-toggle"
            onClick={() => setShowAdvanced(!showAdvanced)}
          >
            <svg style={{ width: 10, height: 10, transition: 'transform var(--duration-fast)', transform: showAdvanced ? 'rotate(90deg)' : 'none' }} viewBox="0 0 10 10" fill="none">
              <path d="M3.5 1.5L7 5 3.5 8.5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
            {t('Advanced options')}
          </button>

          {showAdvanced && (
            <div style={{ display: 'grid', gap: 12 }}>
              <div style={{ display: 'grid', gap: 12, gridTemplateColumns: '1fr 1fr' }}>
                <div className="codex-form-field">
                  <Label className="codex-form-label" htmlFor="auto-research-max-iterations">{t('Max iterations')}</Label>
                  <Input id="auto-research-max-iterations" inputMode="numeric" min={1} onChange={(event) => setMaxIterations(event.target.value)} placeholder="10" type="number" value={maxIterations} />
                </div>
                <div className="codex-form-field">
                  <Label className="codex-form-label" htmlFor="auto-research-time-budget">{t('Time budget (min)')}</Label>
                  <Input id="auto-research-time-budget" inputMode="numeric" min={1} onChange={(event) => setTimeBudgetMinutes(event.target.value)} placeholder="15" type="number" value={timeBudgetMinutes} />
                </div>
              </div>
            </div>
          )}

          <DialogFooter style={{ justifyContent: 'flex-end' }}>
            <Button disabled={saving} onClick={onClose} type="button" variant="outline">{t('Cancel')}</Button>
            <Button disabled={saving || !canCreateRun} type="submit">
              {saving ? t('Starting...') : t('Start Run')}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
