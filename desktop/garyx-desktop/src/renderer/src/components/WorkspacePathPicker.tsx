import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { ArrowLeft, ChevronDown, ChevronRight, Folder, Search as SearchIcon } from 'lucide-react';

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
import { decodeDirectoryListingError } from '@shared/workspace-payload';
import { useWorkspaceDataAdapter, useWorkspaceEpoch } from './workspace-data-adapter';
import { WorkspacePickerContent } from './WorkspacePickerContent';

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
  gatewayHome?: string | null;
  onAddWorkspace?: (
    path: string,
    name?: string | null,
  ) => Promise<DesktopWorkspace | null>;
};

export type WorkspacePathPickerDialogProps = {
  open: boolean;
  title: string;
  description?: string;
  initialPath?: string;
  workspaces?: DesktopWorkspace[];
  saving?: boolean;
  onCancel: () => void;
  onConfirm: (path: string, name: string | null) => Promise<void> | void;
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
  /** Reports every successfully loaded directory: the folder currently in
   *  view IS the selection (Codex add-workspace behavior — no explicit
   *  "use this folder" step). */
  onCurrentPathChange: (path: string) => void;
};

function breadcrumbSegments(path: string): Array<{ label: string; path: string }> {
  const normalized = normalizeWorkspacePath(path);
  if (!normalized || !normalized.startsWith('/')) {
    return [];
  }
  const parts = normalized.split('/').filter(Boolean);
  const segments: Array<{ label: string; path: string }> = [
    { label: '/', path: '/' },
  ];
  let acc = '';
  for (const part of parts) {
    acc += `/${part}`;
    segments.push({ label: part, path: acc });
  }
  return segments;
}

/** Remote directory browser v2: an editable breadcrumb path bar (segment
 * jumps; typing/pasting an absolute path navigates on Enter), a local
 * filter, git badges per entry, and typed navigation errors rendered
 * inline while the browser stays on its current directory. */
function LocalDirectoryBrowser({
  selectedPath,
  disabled = false,
  onCurrentPathChange,
}: LocalDirectoryBrowserProps) {
  const { t } = useI18n();
  const adapter = useWorkspaceDataAdapter();
  // The last successfully loaded listing is the anchor: failed navigations
  // never move the browser away from it.
  const [requestedPath, setRequestedPath] = useState<string | null>(
    selectedPath || null,
  );
  const [listing, setListing] = useState<{
    path: string;
    parentPath: string | null;
    entries: DesktopLocalDirectoryEntry[];
  } | null>(null);
  const [loading, setLoading] = useState(true);
  const [navigationError, setNavigationError] = useState<string | null>(null);
  const [pathEditing, setPathEditing] = useState(false);
  const [pathDraft, setPathDraft] = useState('');
  const [filter, setFilter] = useState('');

  const currentPath = listing?.path || '';
  // Guards the selected-path sync below: a draft update this browser just
  // reported (via onCurrentPathChange) must not echo back into
  // requestedPath, or a blank-path open would reload the default listing
  // it only just resolved.
  const lastReportedPathRef = useRef<string | null>(null);

  useEffect(() => {
    const normalized = normalizeWorkspacePath(selectedPath);
    if (normalized && normalized === lastReportedPathRef.current) {
      return;
    }
    setRequestedPath(selectedPath || null);
  }, [selectedPath]);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    adapter
      .listDirectories({ path: requestedPath })
      .then((nextListing) => {
        if (cancelled) return;
        setListing(nextListing);
        setNavigationError(null);
        setFilter('');
        // The folder in view is the selection; keep the caller's draft in
        // lockstep with every successful navigation.
        const reportedPath = normalizeWorkspacePath(nextListing.path);
        lastReportedPathRef.current = reportedPath;
        onCurrentPathChange(reportedPath);
      })
      .catch((nextError) => {
        if (cancelled) return;
        const message =
          nextError instanceof Error ? nextError.message : String(nextError);
        const typed = decodeDirectoryListingError(message);
        // Inline error; the last good listing stays put.
        setNavigationError(
          typed ? typed.message : message || t('Unable to load folders.'),
        );
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [adapter, onCurrentPathChange, requestedPath, t]);

  const navigate = (path: string | null) => {
    setNavigationError(null);
    setPathEditing(false);
    setRequestedPath(path);
  };

  const submitPathDraft = () => {
    const trimmed = pathDraft.trim();
    if (!trimmed) {
      setPathEditing(false);
      return;
    }
    navigate(trimmed);
  };

  const normalizedFilter = filter.trim().toLowerCase();
  const visibleEntries = normalizedFilter && listing
    ? listing.entries.filter((entry) =>
        entry.name.toLowerCase().includes(normalizedFilter),
      )
    : listing?.entries || [];

  return (
    <div className="overflow-hidden rounded-xl border border-border/70 bg-background text-foreground">
      <div className="flex items-center gap-2 px-3 py-2.5">
        <Button
          aria-label={t('Back')}
          disabled={!listing?.parentPath || disabled || loading}
          onClick={() => {
            if (listing?.parentPath) navigate(listing.parentPath);
          }}
          size="icon-sm"
          type="button"
          variant="ghost"
        >
          <ArrowLeft />
        </Button>
        <div className="min-w-0 flex-1">
          {pathEditing ? (
            <Input
              aria-label={t('Folder path')}
              autoFocus
              className="h-8 font-mono text-xs"
              onBlur={() => setPathEditing(false)}
              onChange={(event) => setPathDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  event.preventDefault();
                  submitPathDraft();
                } else if (event.key === 'Escape') {
                  event.preventDefault();
                  setPathEditing(false);
                }
              }}
              placeholder={t('Type an absolute folder path')}
              value={pathDraft}
            />
          ) : (
            <button
              aria-label={t('Edit folder path')}
              className="workspace-breadcrumb"
              disabled={disabled || loading}
              onClick={() => {
                setPathDraft(currentPath);
                setPathEditing(true);
              }}
              type="button"
            >
              {breadcrumbSegments(currentPath).map((segment, index, all) => (
                <span
                  className="workspace-breadcrumb-segment-wrap"
                  key={segment.path}
                >
                  <span
                    className="workspace-breadcrumb-segment"
                    onClick={(event) => {
                      if (index === all.length - 1) {
                        return;
                      }
                      event.stopPropagation();
                      navigate(segment.path);
                    }}
                    role="link"
                  >
                    {segment.label}
                  </span>
                  {index > 0 && index < all.length - 1 ? (
                    <span className="workspace-breadcrumb-separator">/</span>
                  ) : null}
                </span>
              ))}
              {!currentPath ? (
                <span className="text-muted-foreground">{t('Choose a folder')}</span>
              ) : null}
            </button>
          )}
        </div>
      </div>
      {navigationError ? (
        <div className="workspace-browser-error" role="alert">
          {navigationError}
        </div>
      ) : null}
      <Separator />
      <div className="flex items-center gap-2 px-3 py-1.5">
        <SearchIcon aria-hidden className="size-3.5 text-muted-foreground" />
        <Input
          aria-label={t('Filter folders')}
          className="h-7 border-0 px-0 shadow-none focus-visible:ring-0"
          onChange={(event) => setFilter(event.target.value)}
          placeholder={t('Filter folders')}
          value={filter}
        />
      </div>
      <Separator />
      <div className="max-h-80 overflow-auto p-1">
        {loading && !listing ? (
          <div className="px-3 py-8 text-center text-sm text-muted-foreground">
            {t('Loading folders...')}
          </div>
        ) : visibleEntries.length ? (
          <div className="flex flex-col gap-0.5">
            {visibleEntries.map((entry) => (
              <button
                className={cn(
                  'flex min-h-9 w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-sm transition-colors',
                  disabled ? 'cursor-not-allowed opacity-50' : 'hover:bg-accent hover:text-accent-foreground',
                )}
                disabled={disabled}
                key={entry.path}
                onClick={() => navigate(entry.path)}
                type="button"
              >
                <Folder aria-hidden className="size-4 text-muted-foreground" />
                <span className="min-w-0 flex-1 truncate">{entry.name}</span>
                {entry.gitRepo ? (
                  <span className="workspace-entry-git-badge">git</span>
                ) : null}
                <ChevronRight aria-hidden className="size-4 text-muted-foreground/70" />
              </button>
            ))}
          </div>
        ) : (
          <div className="px-3 py-8 text-center text-sm text-muted-foreground">
            {normalizedFilter ? t('No matches') : t('No folders here.')}
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
  onAdd: (path: string, name: string | null) => Promise<void>;
};

function WorkspaceAddDialog({ open, initialPath, saving = false, onOpenChange, onAdd }: WorkspaceAddDialogProps) {
  const { t } = useI18n();
  const [draft, setDraft] = useState(initialPath);
  const [nameDraft, setNameDraft] = useState('');
  const [nameTouched, setNameTouched] = useState(false);
  const defaultName = workspaceLeafName(draft);
  const effectiveName = nameTouched ? nameDraft : defaultName;

  useEffect(() => {
    if (open) {
      setDraft(initialPath);
      setNameDraft('');
      setNameTouched(false);
    }
  }, [initialPath, open]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="workspace-add-dialog" size="compact">
        <DialogHeader>
          <DialogTitle>{t('Add Workspace')}</DialogTitle>
          <DialogDescription>{t('Choose a folder')}</DialogDescription>
        </DialogHeader>
        <LocalDirectoryBrowser
          disabled={saving}
          onCurrentPathChange={setDraft}
          selectedPath={draft}
        />
        {isAbsoluteWorkspacePath(draft) ? (
          <Field className="gap-1.5">
            <label className="text-xs text-muted-foreground" htmlFor="workspace-add-name">
              {t('Workspace name')}
            </label>
            <Input
              disabled={saving}
              id="workspace-add-name"
              onChange={(event) => {
                setNameDraft(event.target.value);
                setNameTouched(true);
              }}
              placeholder={defaultName}
              value={effectiveName}
            />
          </Field>
        ) : null}
        <DialogFooter>
          <Button disabled={saving} onClick={() => onOpenChange(false)} type="button" variant="outline">
            {t('Cancel')}
          </Button>
          {isAbsoluteWorkspacePath(draft) ? (
            <Button
              disabled={saving}
              onClick={() => {
                void onAdd(
                  normalizeWorkspacePath(draft),
                  effectiveName.trim() || null,
                );
              }}
              type="button"
            >
              {saving ? t('Saving...') : t('Add workspace')}
            </Button>
          ) : null}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export type WorkspaceSelectDialogProps = {
  open: boolean;
  /** Defaults to "Choose workspace". */
  title?: string;
  workspaces?: DesktopWorkspace[];
  gatewayHome?: string | null;
  /** Offer a "No workspace" row that selects the empty path. */
  allowEmpty?: boolean;
  selectedPath?: string;
  onSelect: (workspacePath: string) => void;
  onClose: () => void;
  /** Renders the "Add workspace…" footer row that opens the caller's
   * add-workspace flow. */
  onAddWorkspace?: () => void;
  addWorkspaceBusy?: boolean;
};

/** Shared floating workspace picker: every in-form workspace selector opens
 * this dialog, whose body is the same {@link WorkspacePickerContent} the
 * composer chip popover renders — one picker, two hosts. */
export function WorkspaceSelectDialog({
  open,
  title,
  workspaces = [],
  gatewayHome = null,
  allowEmpty = false,
  selectedPath = '',
  onSelect,
  onClose,
  onAddWorkspace,
  addWorkspaceBusy = false,
}: WorkspaceSelectDialogProps) {
  const { t } = useI18n();
  const selectedKey = workspacePathKey(selectedPath);
  const seenPaths = new Set<string>();
  const pickerWorkspaces = workspaces.filter((workspace) => {
    const key = workspacePathKey(workspace.path);
    if (!key) return true;
    if (seenPaths.has(key)) return false;
    seenPaths.add(key);
    return true;
  });

  const closeDialog = () => {
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
        <WorkspacePickerContent
          addWorkspaceBusy={addWorkspaceBusy}
          allowNone={allowEmpty}
          gatewayHome={gatewayHome}
          noneSelected={allowEmpty && !selectedKey}
          onAddWorkspace={onAddWorkspace ? () => {
            closeDialog();
            onAddWorkspace();
          } : undefined}
          onSelectNone={() => {
            onSelect('');
            closeDialog();
          }}
          onSelectPath={(path) => {
            onSelect(path);
            closeDialog();
          }}
          selectedPath={selectedPath}
          workspaces={pickerWorkspaces}
        />
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
  gatewayHome = null,
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

  const adapter = useWorkspaceDataAdapter();
  const workspaceEpoch = useWorkspaceEpoch();
  const workspaceEpochRef = useRef(workspaceEpoch);
  useEffect(() => {
    workspaceEpochRef.current = workspaceEpoch;
  }, [workspaceEpoch]);
  const [fetchedCatalog, setFetchedCatalog] = useState<{
    workspaces: DesktopWorkspace[];
    gatewayHome: string | null;
  } | null>(null);

  // Gateway switch: close transient surfaces and drop the previous
  // gateway's catalog.
  useEffect(() => {
    setPickerOpen(false);
    setAddOpen(false);
    setFetchedCatalog(null);
    // A stale add's finally is epoch-guarded, so the switch owns resetting
    // this local busy flag (otherwise a remount-free embedder stays stuck
    // on Saving forever).
    setSavingAdd(false);
  }, [workspaceEpoch]);

  useEffect(() => {
    if (!pickerOpen || workspaces.length > 0 || fetchedCatalog) {
      return;
    }
    let cancelled = false;
    void adapter
      .listCatalog()
      .then((catalog) => {
        if (!cancelled) setFetchedCatalog(catalog);
      })
      .catch(() => {
        // The picker still renders (browse/add remain available).
      });
    return () => {
      cancelled = true;
    };
  }, [adapter, fetchedCatalog, pickerOpen, workspaces.length]);

  const effectiveWorkspaces = workspaces.length > 0
    ? selectableWorkspaces
    : fetchedCatalog?.workspaces ?? [];
  const effectiveGatewayHome = gatewayHome ?? fetchedCatalog?.gatewayHome ?? null;

  async function addWorkspace(path: string, name?: string | null) {
    setSavingAdd(true);
    const epoch = workspaceEpoch;
    try {
      const added = onAddWorkspace
        ? await onAddWorkspace(path, name)
        : await adapter.addWorkspace(path, name);
      if (epoch !== workspaceEpochRef.current) {
        return;
      }
      // The catalog changed; drop the cached copy so the next open lists
      // the new row.
      setFetchedCatalog(null);
      if (!added?.path) {
        return;
      }
      onChange(normalizeWorkspacePath(added.path));
      setAddOpen(false);
    } finally {
      if (epoch === workspaceEpochRef.current) {
        setSavingAdd(false);
      }
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
        allowEmpty={allowEmpty}
        gatewayHome={effectiveGatewayHome}
        onAddWorkspace={() => setAddOpen(true)}
        onClose={() => setPickerOpen(false)}
        onSelect={onChange}
        open={pickerOpen}
        selectedPath={trimmed}
        workspaces={effectiveWorkspaces}
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
  const [nameDraft, setNameDraft] = useState('');
  const [nameTouched, setNameTouched] = useState(false);
  const trimmed = draft.trim();
  const canSave = Boolean(trimmed && isAbsoluteWorkspacePath(trimmed));
  const defaultName = workspaceLeafName(trimmed);
  const effectiveName = nameTouched ? nameDraft : defaultName;

  useEffect(() => {
    if (open) {
      setDraft(initialPath);
      setNameDraft('');
      setNameTouched(false);
    }
  }, [initialPath, open]);

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen && !saving) onCancel();
      }}
    >
      <DialogContent className="workspace-add-dialog" size="compact">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          {description ? <DialogDescription>{description}</DialogDescription> : null}
        </DialogHeader>
        <LocalDirectoryBrowser
          disabled={saving}
          onCurrentPathChange={setDraft}
          selectedPath={draft}
        />
        {canSave ? (
          <Field className="gap-1.5">
            <label className="text-xs text-muted-foreground" htmlFor="workspace-picker-dialog-name">
              {t('Workspace name')}
            </label>
            <Input
              disabled={saving}
              id="workspace-picker-dialog-name"
              onChange={(event) => {
                setNameDraft(event.target.value);
                setNameTouched(true);
              }}
              placeholder={defaultName}
              value={effectiveName}
            />
          </Field>
        ) : null}
        <DialogFooter>
          <Button disabled={saving} onClick={onCancel} type="button" variant="outline">
            {t('Cancel')}
          </Button>
          {canSave ? (
            <Button
              disabled={saving}
              onClick={() => {
                void onConfirm(
                  normalizeWorkspacePath(trimmed),
                  effectiveName.trim() || null,
                );
              }}
              type="button"
            >
              {saving ? t('Saving...') : t('Add workspace')}
            </Button>
          ) : null}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
