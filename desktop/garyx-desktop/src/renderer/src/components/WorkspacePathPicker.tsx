import { useEffect, useId, useMemo, useState } from 'react';
import { ArrowLeft, Check, ChevronDown, ChevronRight, Folder, FolderOpen } from 'lucide-react';
import {
  IconCheck,
  IconCircleMinus,
  IconFolder,
  IconPlus,
  IconSearch,
} from '@tabler/icons-react';

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
import { Input } from '@/components/ui/input';
import { Separator } from '@/components/ui/separator';
import { useI18n } from '@/i18n';
import { cn } from '@/lib/utils';

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

type LocalDirectoryBrowserProps = {
  selectedPath: string;
  disabled?: boolean;
  onSelect: (path: string) => void;
};

function LocalDirectoryBrowser({
  selectedPath,
  disabled = false,
  onSelect,
}: LocalDirectoryBrowserProps) {
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
      <div className="flex items-center gap-2 px-3 py-2.5">
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
                  'flex min-h-9 w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-sm transition-colors',
                  disabled ? 'cursor-not-allowed opacity-50' : 'hover:bg-accent hover:text-accent-foreground',
                )}
                disabled={disabled}
                key={entry.path}
                onClick={() => setCurrentPath(entry.path)}
                type="button"
              >
                <Folder aria-hidden className="size-4 text-muted-foreground" />
                <span className="min-w-0 flex-1 truncate">{entry.name}</span>
                <ChevronRight aria-hidden className="size-4 text-muted-foreground/70" />
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
          {isAbsoluteWorkspacePath(draft) ? (
            <Button
              disabled={saving}
              onClick={() => {
                void onAdd(normalizeWorkspacePath(draft));
              }}
              type="button"
            >
              {saving ? t('Saving...') : t('Save')}
            </Button>
          ) : null}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** Row shape for {@link WorkspaceSelectDialog}; `DesktopWorkspace` satisfies it. */
export type WorkspaceSelectDialogWorkspace = Pick<DesktopWorkspace, 'name' | 'path' | 'available'>;

export type WorkspaceSelectDialogProps = {
  open: boolean;
  /** Defaults to "Choose workspace". */
  title?: string;
  workspaces?: WorkspaceSelectDialogWorkspace[];
  /** Offer a "No workspace" row that selects the empty path. */
  allowEmpty?: boolean;
  selectedPath?: string;
  onSelect: (workspacePath: string) => void;
  onClose: () => void;
  /** Renders the "Choose folder…" footer row that opens the caller's
   * add-workspace flow. */
  onAddWorkspace?: () => void;
  addWorkspaceLabel?: string;
  addWorkspaceBusy?: boolean;
};

/** Shared floating workspace picker: search field, workspace rows, and a
 * "Choose folder…" footer. Every workspace selector — the new-thread picker
 * and the in-form pickers — opens this dialog. */
export function WorkspaceSelectDialog({
  open,
  title,
  workspaces = [],
  allowEmpty = false,
  selectedPath = '',
  onSelect,
  onClose,
  onAddWorkspace,
  addWorkspaceLabel,
  addWorkspaceBusy = false,
}: WorkspaceSelectDialogProps) {
  const { t } = useI18n();
  const [query, setQuery] = useState('');
  const selectedKey = workspacePathKey(selectedPath);
  const normalizedQuery = query.trim().toLowerCase();
  const matchesQuery = (name: string, path: string | null) =>
    !normalizedQuery ||
    name.toLowerCase().includes(normalizedQuery) ||
    (path || '').toLowerCase().includes(normalizedQuery);

  const seenPaths = new Set<string>();
  const localRows = workspaces.filter((workspace) => {
    const key = workspacePathKey(workspace.path);
    if (!key) return true;
    if (seenPaths.has(key)) return false;
    seenPaths.add(key);
    return true;
  });
  // Keep a selected-but-unlisted path visible so the check mark always has
  // a row (parity with the old in-form dropdown's "current value" item).
  const rows: WorkspaceSelectDialogWorkspace[] =
    selectedKey && !seenPaths.has(selectedKey)
      ? [
          { name: workspaceLabel(selectedPath), path: selectedPath.trim(), available: true },
          ...localRows,
        ]
      : localRows;
  const filteredRows = rows.filter((workspace) => matchesQuery(workspace.name, workspace.path));

  const closeDialog = () => {
    setQuery('');
    onClose();
  };

  return (
    <Dialog
      onOpenChange={(nextOpen) => {
        if (!nextOpen) closeDialog();
      }}
      open={open}
    >
      <DialogContent className="workspace-picker-dialog" size="compact">
        <DialogHeader>
          <DialogTitle>{title || t('Choose workspace')}</DialogTitle>
        </DialogHeader>
        <div className="workspace-picker-search">
          <IconSearch aria-hidden size={15} stroke={1.7} />
          <Input
            autoFocus
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t('Search projects')}
            value={query}
          />
        </div>
        <div className="workspace-picker-list">
          {allowEmpty && !normalizedQuery ? (
            <button
              className="workspace-picker-row"
              data-active={selectedKey ? undefined : ''}
              onClick={() => {
                onSelect('');
                closeDialog();
              }}
              type="button"
            >
              <IconCircleMinus aria-hidden size={16} stroke={1.7} />
              <span className="workspace-picker-name">{t('No workspace')}</span>
              {selectedKey ? null : (
                <IconCheck
                  aria-hidden
                  className="workspace-picker-check"
                  size={15}
                  stroke={2}
                />
              )}
            </button>
          ) : null}
          {filteredRows.map((workspace) => {
            const rowKey = workspacePathKey(workspace.path);
            const isSelected = Boolean(rowKey && rowKey === selectedKey);
            return (
              <button
                className="workspace-picker-row"
                data-active={isSelected ? '' : undefined}
                disabled={!workspace.available || !workspace.path}
                key={workspace.path || workspace.name}
                onClick={() => {
                  if (!workspace.path) {
                    return;
                  }
                  onSelect(workspace.path);
                  closeDialog();
                }}
                type="button"
              >
                <IconFolder aria-hidden size={16} stroke={1.7} />
                <span className="workspace-picker-name">
                  {workspace.available && workspace.path
                    ? workspace.name
                    : t('{name} (Unavailable)', { name: workspace.name })}
                </span>
                <span className="workspace-picker-path">{workspace.path}</span>
                {isSelected ? (
                  <IconCheck
                    aria-hidden
                    className="workspace-picker-check"
                    size={15}
                    stroke={2}
                  />
                ) : null}
              </button>
            );
          })}
          {!filteredRows.length ? (
            <p className="workspace-picker-empty">{t('No matching projects.')}</p>
          ) : null}
        </div>
        {onAddWorkspace ? (
          <div className="workspace-picker-footer">
            <button
              className="workspace-picker-row"
              disabled={addWorkspaceBusy}
              onClick={() => {
                closeDialog();
                onAddWorkspace();
              }}
              type="button"
            >
              <IconPlus aria-hidden size={15} stroke={1.8} />
              <span className="workspace-picker-name">
                {addWorkspaceBusy
                  ? t('Opening folder…')
                  : addWorkspaceLabel || t('Choose folder…')}
              </span>
            </button>
          </div>
        ) : null}
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
  addWorkspaceLabel,
  onAddWorkspace,
}: WorkspacePathPickerProps) {
  const { t } = useI18n();
  const generatedId = useId();
  const inputId = id ?? `workspace-path-${generatedId}`;
  const errorId = `${inputId}-error`;
  const hintId = `${inputId}-hint`;
  const [pickerOpen, setPickerOpen] = useState(false);
  const [addOpen, setAddOpen] = useState(false);
  const [savingAdd, setSavingAdd] = useState(false);
  // Pathless entries cannot be picked as a form value; hide them like the
  // previous dropdown options did.
  const selectableWorkspaces = useMemo(
    () => workspaces.filter((workspace) => workspace.path?.trim()),
    [workspaces],
  );
  const trimmed = value.trim();
  const invalid = Boolean(trimmed && !isAbsoluteWorkspacePath(trimmed));
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
        {/* data-slot keeps the form-context trigger styling that targets the
            shadcn Select trigger (automation/tasks/agents-hub forms). */}
        <button
          aria-describedby={describedBy}
          aria-expanded={pickerOpen}
          aria-haspopup="dialog"
          aria-invalid={invalid || undefined}
          className={cn(
            'flex h-auto min-h-11 w-full select-none items-center justify-between gap-2 rounded-md border border-input bg-background px-3 py-2 text-left text-sm shadow-xs transition-colors outline-none disabled:cursor-not-allowed disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0',
            triggerClassName,
          )}
          data-slot="select-trigger"
          disabled={disabled}
          id={inputId}
          onClick={() => setPickerOpen(true)}
          type="button"
        >
          <WorkspacePathSummary path={trimmed} placeholder={placeholder || t('Choose workspace')} />
          <ChevronDown aria-hidden className="size-4 text-muted-foreground opacity-50" />
        </button>
        {invalid ? (
          <FieldError id={errorId}>{t('Use an absolute path.')}</FieldError>
        ) : null}
        {!allowEmpty && !trimmed ? (
          <FieldDescription id={hintId}>{t('Choose workspace')}</FieldDescription>
        ) : null}
      </Field>
      <WorkspaceSelectDialog
        addWorkspaceLabel={addWorkspaceLabel}
        allowEmpty={allowEmpty}
        onAddWorkspace={() => setAddOpen(true)}
        onClose={() => setPickerOpen(false)}
        onSelect={onChange}
        open={pickerOpen}
        selectedPath={trimmed}
        workspaces={selectableWorkspaces}
      />
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
  const canSave = Boolean(trimmed && isAbsoluteWorkspacePath(trimmed));

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
          {canSave ? (
            <Button
              disabled={saving}
              onClick={() => {
                void onConfirm(normalizeWorkspacePath(trimmed));
              }}
              type="button"
            >
              {saving ? t('Saving...') : t('Save')}
            </Button>
          ) : null}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
