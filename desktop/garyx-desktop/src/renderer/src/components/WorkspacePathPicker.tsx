import { useEffect, useId, useMemo, useState } from 'react';
import { ArrowLeft, Check, ChevronRight, Folder, FolderOpen, FolderPlus, MinusCircle } from 'lucide-react';

import type { DesktopLocalDirectoryEntry, DesktopWorkspace } from '@shared/contracts';

import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Field, FieldDescription, FieldError, FieldGroup } from '@/components/ui/field';
import { Separator } from '@/components/ui/separator';
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { useI18n } from '@/i18n';
import { cn } from '@/lib/utils';

const EMPTY_WORKSPACE_VALUE = '__garyx_empty_workspace__';
const ADD_WORKSPACE_VALUE = '__garyx_add_workspace__';
const CURRENT_WORKSPACE_VALUE = '__garyx_current_workspace__';

export type WorkspacePathPickerProps = {
  value: string;
  onChange: (next: string) => void;
  workspaces?: DesktopWorkspace[];
  id?: string;
  placeholder?: string;
  disabled?: boolean;
  allowEmpty?: boolean;
  className?: string;
  fieldClassName?: string;
  triggerClassName?: string;
  contentClassName?: string;
  addWorkspaceLabel?: string;
  onAddWorkspace?: (path: string) => Promise<DesktopWorkspace | null>;
};

export type WorkspacePathPickerDialogProps = {
  open: boolean;
  title: string;
  description?: string;
  initialPath?: string;
  workspaces?: DesktopWorkspace[];
  saving?: boolean;
  onCancel: () => void;
  onConfirm: (path: string) => Promise<void> | void;
};

function normalizeWorkspacePath(path: string): string {
  const normalized = path.trim().replace(/\\/g, '/');
  if (normalized === '/') return normalized;
  return normalized.replace(/\/+$/g, '');
}

export function isAbsoluteWorkspacePath(path: string): boolean {
  const normalized = normalizeWorkspacePath(path);
  if (!normalized) return false;
  if (normalized.startsWith('/')) return true;
  if (/^[A-Za-z]:\//.test(normalized)) return true;
  return normalized.startsWith('//');
}

function workspacePathKey(path?: string | null): string {
  return normalizeWorkspacePath(path || '').toLowerCase();
}

function workspaceLeafName(path: string): string {
  const normalized = normalizeWorkspacePath(path);
  if (!normalized) return '';
  const parts = normalized.split('/').filter(Boolean);
  return parts.at(-1) || normalized;
}

function workspaceCompactPath(path: string): string {
  const normalized = normalizeWorkspacePath(path);
  const parts = normalized.split('/').filter(Boolean);
  if (parts.length <= 2) return normalized;
  return `.../${parts.slice(-2).join('/')}`;
}

function workspaceLabel(path: string): string {
  return workspaceLeafName(path) || path;
}

type WorkspacePathSummaryProps = {
  path: string;
  placeholder: string;
};

function WorkspacePathSummary({ path, placeholder }: WorkspacePathSummaryProps) {
  const trimmed = path.trim();
  return (
    <span className="flex min-w-0 flex-col gap-0.5 text-left">
      <span className={cn('truncate text-sm', trimmed ? 'font-medium' : 'font-normal text-muted-foreground')}>
        {trimmed ? workspaceLabel(trimmed) : placeholder}
      </span>
      {trimmed ? (
        <span className="workspace-path-secondary truncate text-xs text-muted-foreground">
          {trimmed}
        </span>
      ) : null}
    </span>
  );
}

function normalizeWorkspaceOptions(workspaces: DesktopWorkspace[] = []): string[] {
  const seen = new Set<string>();
  return workspaces
    .map((workspace) => workspace.path?.trim() || '')
    .filter((path) => {
      if (!path) return false;
      const key = workspacePathKey(path);
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
}

type LocalDirectoryBrowserProps = {
  selectedPath: string;
  disabled?: boolean;
  onSelect: (path: string) => void;
};

function LocalDirectoryBrowser({ selectedPath, disabled = false, onSelect }: LocalDirectoryBrowserProps) {
  const { t } = useI18n();
  const [currentPath, setCurrentPath] = useState(selectedPath);
  const [parentPath, setParentPath] = useState<string | null>(null);
  const [entries, setEntries] = useState<DesktopLocalDirectoryEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const normalizedSelected = normalizeWorkspacePath(selectedPath);
  const normalizedCurrent = normalizeWorkspacePath(currentPath);
  const isCurrentSelected = Boolean(normalizedCurrent && normalizedCurrent === normalizedSelected);

  useEffect(() => {
    setCurrentPath(selectedPath);
  }, [selectedPath]);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    window.garyxDesktop
      .listWorkspaceDirectories({ path: currentPath || null })
      .then((listing) => {
        if (cancelled) return;
        setCurrentPath(listing.path);
        setParentPath(listing.parentPath);
        setEntries(listing.entries);
      })
      .catch((nextError) => {
        if (cancelled) return;
        setError(nextError instanceof Error ? nextError.message : t('Unable to load folders.'));
        setEntries([]);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [currentPath, t]);

  return (
    <div className="overflow-hidden rounded-xl border border-border/70 bg-background text-foreground">
      <div className="flex items-center gap-2 px-3 py-3">
        <Button
          aria-label={t('Back')}
          disabled={!parentPath || disabled || loading}
          onClick={() => {
            if (parentPath) setCurrentPath(parentPath);
          }}
          size="icon-sm"
          type="button"
          variant="ghost"
        >
          <ArrowLeft />
        </Button>
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-medium">
            {workspaceLeafName(currentPath) || currentPath || t('Folders')}
          </div>
          <div className="truncate text-xs text-muted-foreground">
            {currentPath || t('Choose a folder')}
          </div>
        </div>
        {currentPath ? (
          <Button
            disabled={disabled || loading || isCurrentSelected}
            onClick={() => onSelect(normalizeWorkspacePath(currentPath))}
            size="sm"
            type="button"
            variant={isCurrentSelected ? 'secondary' : 'outline'}
          >
            {isCurrentSelected ? <Check /> : <FolderOpen />}
            {isCurrentSelected ? t('Selected') : t('Use this folder')}
          </Button>
        ) : null}
      </div>
      <Separator />
      <div className="max-h-80 overflow-auto p-1">
        {loading ? (
          <div className="px-3 py-8 text-center text-sm text-muted-foreground">
            {t('Loading folders...')}
          </div>
        ) : error ? (
          <div className="px-3 py-8 text-center text-sm text-destructive">
            {error}
          </div>
        ) : entries.length ? (
          <div className="flex flex-col gap-0.5">
            {entries.map((entry) => (
              <button
                className={cn(
                  'flex min-h-11 w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm transition-colors',
                  disabled ? 'cursor-not-allowed opacity-50' : 'hover:bg-accent hover:text-accent-foreground',
                )}
                disabled={disabled}
                key={entry.path}
                onClick={() => setCurrentPath(entry.path)}
                type="button"
              >
                <Folder aria-hidden className="text-muted-foreground" />
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-medium">{entry.name}</span>
                  <span className="block truncate text-xs text-muted-foreground">
                    {workspaceCompactPath(entry.path)}
                  </span>
                </span>
                <ChevronRight aria-hidden className="text-muted-foreground" />
              </button>
            ))}
          </div>
        ) : (
          <div className="px-3 py-8 text-center text-sm text-muted-foreground">
            {t('No folders here.')}
          </div>
        )}
      </div>
    </div>
  );
}

type WorkspaceAddDialogProps = {
  open: boolean;
  initialPath: string;
  saving?: boolean;
  onOpenChange: (open: boolean) => void;
  onAdd: (path: string) => Promise<void>;
};

function WorkspaceAddDialog({ open, initialPath, saving = false, onOpenChange, onAdd }: WorkspaceAddDialogProps) {
  const { t } = useI18n();
  const [draft, setDraft] = useState(initialPath);

  useEffect(() => {
    if (open) setDraft(initialPath);
  }, [initialPath, open]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent size="wide">
        <DialogHeader>
          <DialogTitle>{t('Add Workspace')}</DialogTitle>
          <DialogDescription>{t('Choose a folder')}</DialogDescription>
        </DialogHeader>
        <LocalDirectoryBrowser
          disabled={saving}
          onSelect={setDraft}
          selectedPath={draft}
        />
        <DialogFooter>
          <Button disabled={saving} onClick={() => onOpenChange(false)} type="button" variant="outline">
            {t('Cancel')}
          </Button>
          <Button
            disabled={saving || !isAbsoluteWorkspacePath(draft)}
            onClick={() => {
              void onAdd(normalizeWorkspacePath(draft));
            }}
            type="button"
          >
            {saving ? t('Saving...') : t('Save')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function WorkspacePathPicker({
  value,
  onChange,
  workspaces = [],
  id,
  placeholder,
  disabled = false,
  allowEmpty = true,
  className,
  fieldClassName,
  triggerClassName,
  contentClassName,
  addWorkspaceLabel,
  onAddWorkspace,
}: WorkspacePathPickerProps) {
  const { t } = useI18n();
  const generatedId = useId();
  const inputId = id ?? `workspace-path-${generatedId}`;
  const errorId = `${inputId}-error`;
  const hintId = `${inputId}-hint`;
  const [addOpen, setAddOpen] = useState(false);
  const [savingAdd, setSavingAdd] = useState(false);
  const optionPaths = useMemo(() => normalizeWorkspaceOptions(workspaces), [workspaces]);
  const trimmed = value.trim();
  const invalid = Boolean(trimmed && !isAbsoluteWorkspacePath(trimmed));
  const selectedKey = workspacePathKey(trimmed);
  const selectedOption = optionPaths.find((path) => workspacePathKey(path) === selectedKey) || null;
  const selectedMissing = Boolean(trimmed && !selectedOption);
  const selectValue = trimmed
    ? selectedOption || CURRENT_WORKSPACE_VALUE
    : allowEmpty
      ? EMPTY_WORKSPACE_VALUE
      : undefined;
  const describedBy = [
    invalid ? errorId : null,
    !allowEmpty && !trimmed ? hintId : null,
  ].filter(Boolean).join(' ') || undefined;

  async function addWorkspace(path: string) {
    setSavingAdd(true);
    try {
      const added = onAddWorkspace
        ? await onAddWorkspace(path)
        : await window.garyxDesktop.addWorkspaceByPath({ path }).then((result) => result.workspace || null);
      if (!added?.path) {
        return;
      }
      onChange(normalizeWorkspacePath(added.path));
      setAddOpen(false);
    } finally {
      setSavingAdd(false);
    }
  }

  return (
    <FieldGroup className={cn('gap-2', className)}>
      <Field className={cn('gap-2', fieldClassName)} data-invalid={invalid || undefined}>
        <Select
          disabled={disabled}
          value={selectValue}
          onValueChange={(nextValue) => {
            if (nextValue === ADD_WORKSPACE_VALUE) {
              setAddOpen(true);
              return;
            }
            if (nextValue === EMPTY_WORKSPACE_VALUE) {
              onChange('');
              return;
            }
            if (nextValue === CURRENT_WORKSPACE_VALUE) {
              return;
            }
            onChange(nextValue);
          }}
        >
          <SelectTrigger
            aria-describedby={describedBy}
            aria-invalid={invalid || undefined}
            className={cn('h-auto min-h-11 w-full justify-between', triggerClassName)}
            id={inputId}
          >
            <SelectValue placeholder={placeholder || t('Choose workspace')} />
          </SelectTrigger>
          <SelectContent position="popper" align="start" className={contentClassName}>
            <SelectGroup>
              <SelectLabel>{t('Workspace')}</SelectLabel>
              {allowEmpty ? (
                <SelectItem value={EMPTY_WORKSPACE_VALUE}>
                  <span className="flex items-center gap-2">
                    <MinusCircle />
                    {t('No workspace')}
                  </span>
                </SelectItem>
              ) : null}
              {selectedMissing ? (
                <SelectItem value={CURRENT_WORKSPACE_VALUE}>
                  <WorkspacePathSummary path={trimmed} placeholder={placeholder || t('Choose workspace')} />
                </SelectItem>
              ) : null}
              {optionPaths.map((path) => (
                <SelectItem key={path} value={path}>
                  <WorkspacePathSummary path={path} placeholder={placeholder || t('Choose workspace')} />
                </SelectItem>
              ))}
            </SelectGroup>
            <SelectSeparator />
            <SelectGroup>
              <SelectItem value={ADD_WORKSPACE_VALUE}>
                <span className="flex items-center gap-2">
                  <FolderPlus />
                  {addWorkspaceLabel || t('Add workspace…')}
                </span>
              </SelectItem>
            </SelectGroup>
          </SelectContent>
        </Select>
        {invalid ? (
          <FieldError id={errorId}>{t('Use an absolute path.')}</FieldError>
        ) : null}
        {!allowEmpty && !trimmed ? (
          <FieldDescription id={hintId}>{t('Choose workspace')}</FieldDescription>
        ) : null}
      </Field>
      <WorkspaceAddDialog
        initialPath={trimmed}
        onAdd={addWorkspace}
        onOpenChange={setAddOpen}
        open={addOpen}
        saving={savingAdd}
      />
    </FieldGroup>
  );
}

export function WorkspacePathPickerDialog({
  open,
  title,
  description,
  initialPath = '',
  saving = false,
  onCancel,
  onConfirm,
}: WorkspacePathPickerDialogProps) {
  const { t } = useI18n();
  const [draft, setDraft] = useState(initialPath);
  const trimmed = draft.trim();
  const canConfirm = Boolean(trimmed && isAbsoluteWorkspacePath(trimmed) && !saving);

  useEffect(() => {
    if (open) setDraft(initialPath);
  }, [initialPath, open]);

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen && !saving) onCancel();
      }}
    >
      <DialogContent size="wide">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          {description ? <DialogDescription>{description}</DialogDescription> : null}
        </DialogHeader>
        <LocalDirectoryBrowser
          disabled={saving}
          onSelect={setDraft}
          selectedPath={draft}
        />
        <DialogFooter>
          <Button disabled={saving} onClick={onCancel} type="button" variant="outline">
            {t('Cancel')}
          </Button>
          <Button
            disabled={!canConfirm}
            onClick={() => {
              void onConfirm(normalizeWorkspacePath(trimmed));
            }}
            type="button"
          >
            {saving ? t('Saving...') : t('Save')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
