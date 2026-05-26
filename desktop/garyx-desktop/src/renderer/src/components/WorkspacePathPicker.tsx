import { useEffect, useMemo, useState } from 'react';
import { ArrowLeft, Check, ChevronRight, Folder, FolderOpen } from 'lucide-react';

import type { DesktopWorkspace } from '@shared/contracts';

import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Field, FieldDescription, FieldGroup } from '@/components/ui/field';
import { Input } from '@/components/ui/input';
import { Separator } from '@/components/ui/separator';
import { useI18n } from '@/i18n';
import { cn } from '@/lib/utils';

type WorkspaceDirectoryNode = {
  name: string;
  path: string;
  workspace: DesktopWorkspace | null;
  children: WorkspaceDirectoryNode[];
};

export type WorkspacePathPickerProps = {
  value: string;
  onChange: (next: string) => void;
  workspaces?: DesktopWorkspace[];
  id?: string;
  placeholder?: string;
  disabled?: boolean;
  allowEmpty?: boolean;
  showKnownTree?: boolean;
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

function splitAbsolutePath(path: string): { root: string; parts: string[] } | null {
  const normalized = normalizeWorkspacePath(path);
  if (!isAbsoluteWorkspacePath(normalized)) return null;
  const driveMatch = normalized.match(/^([A-Za-z]:)\/?(.*)$/);
  if (driveMatch) {
    return {
      root: driveMatch[1],
      parts: driveMatch[2].split('/').filter(Boolean),
    };
  }
  if (normalized.startsWith('//')) {
    const parts = normalized.slice(2).split('/').filter(Boolean);
    return { root: '//', parts };
  }
  return { root: '/', parts: normalized.slice(1).split('/').filter(Boolean) };
}

function childPath(parent: string, segment: string): string {
  if (parent === '/') return `/${segment}`;
  if (parent === '//') return `//${segment}`;
  if (/^[A-Za-z]:$/.test(parent)) return `${parent}/${segment}`;
  return `${parent}/${segment}`;
}

function buildWorkspaceTree(workspaces: DesktopWorkspace[] = []): WorkspaceDirectoryNode[] {
  const roots = new Map<string, WorkspaceDirectoryNode>();

  for (const workspace of workspaces) {
    const path = normalizeWorkspacePath(workspace.path || '');
    if (!path) continue;
    const split = splitAbsolutePath(path);
    if (!split) continue;

    let current: WorkspaceDirectoryNode;
    const existingRoot = roots.get(split.root);
    if (existingRoot) {
      current = existingRoot;
    } else {
      current = {
        name: split.root,
        path: split.root,
        workspace: null,
        children: [],
      };
      roots.set(split.root, current);
    }

    let currentPath = split.root;
    for (const part of split.parts) {
      currentPath = childPath(currentPath, part);
      let child: WorkspaceDirectoryNode | undefined = current.children.find((item) => item.name === part);
      if (!child) {
        child = {
          name: part,
          path: currentPath,
          workspace: null,
          children: [],
        };
        current.children.push(child);
      }
      current = child;
    }
    current.workspace = workspace;
  }

  function sortNode(node: WorkspaceDirectoryNode): WorkspaceDirectoryNode {
    return {
      ...node,
      children: node.children
        .sort((left, right) => left.name.localeCompare(right.name))
        .map(sortNode),
    };
  }

  return Array.from(roots.values())
    .sort((left, right) => left.name.localeCompare(right.name))
    .map(sortNode);
}

function findWorkspaceNode(nodes: WorkspaceDirectoryNode[], path: string): WorkspaceDirectoryNode | null {
  const normalized = normalizeWorkspacePath(path);
  for (const node of nodes) {
    if (normalizeWorkspacePath(node.path) === normalized) return node;
    const child = findWorkspaceNode(node.children, normalized);
    if (child) return child;
  }
  return null;
}

function firstWorkspacePath(nodes: WorkspaceDirectoryNode[]): string {
  for (const node of nodes) {
    if (node.workspace?.path) return node.workspace.path;
    const childPathValue = firstWorkspacePath(node.children);
    if (childPathValue) return childPathValue;
  }
  return '';
}

function parentWorkspacePath(path: string): string {
  const normalized = normalizeWorkspacePath(path);
  const split = splitAbsolutePath(normalized);
  if (!split || split.parts.length === 0) return '';
  if (split.parts.length === 1) return split.root;
  return split.parts.slice(0, -1).reduce((current, part) => childPath(current, part), split.root);
}

function workspaceLeafName(path: string): string {
  const normalized = normalizeWorkspacePath(path);
  if (!normalized) return '';
  const split = splitAbsolutePath(normalized);
  if (!split || split.parts.length === 0) return normalized;
  return split.parts[split.parts.length - 1];
}

function workspaceCompactPath(path: string): string {
  const normalized = normalizeWorkspacePath(path);
  const split = splitAbsolutePath(normalized);
  if (!split) return normalized;
  if (split.parts.length <= 2) return normalized;
  return `.../${split.parts.slice(-2).join('/')}`;
}

function initialBrowserPath(nodes: WorkspaceDirectoryNode[], selectedPath: string): string {
  const normalizedSelected = normalizeWorkspacePath(selectedPath);
  if (normalizedSelected && findWorkspaceNode(nodes, normalizedSelected)) {
    return parentWorkspacePath(normalizedSelected);
  }
  const firstPath = firstWorkspacePath(nodes);
  return firstPath ? parentWorkspacePath(firstPath) : '';
}

type WorkspacePathBrowserProps = {
  nodes: WorkspaceDirectoryNode[];
  selectedPath: string;
  disabled?: boolean;
  onSelect: (path: string) => void;
};

function WorkspacePathBrowser({ nodes, selectedPath, disabled = false, onSelect }: WorkspacePathBrowserProps) {
  const { t } = useI18n();
  const [currentPath, setCurrentPath] = useState(() => initialBrowserPath(nodes, selectedPath));

  useEffect(() => {
    setCurrentPath((current) => {
      if (!current || findWorkspaceNode(nodes, current)) return current;
      return initialBrowserPath(nodes, selectedPath);
    });
  }, [nodes, selectedPath]);

  const currentNode = currentPath ? findWorkspaceNode(nodes, currentPath) : null;
  const rows = currentPath ? currentNode?.children ?? [] : nodes;
  const normalizedSelected = normalizeWorkspacePath(selectedPath);
  const normalizedCurrent = normalizeWorkspacePath(currentPath);
  const canUseCurrent = Boolean(normalizedCurrent && isAbsoluteWorkspacePath(normalizedCurrent));
  const isCurrentSelected = canUseCurrent && normalizedSelected === normalizedCurrent;

  function activateNode(node: WorkspaceDirectoryNode) {
    if (disabled) return;
    if (node.children.length > 0) {
      setCurrentPath(node.path);
      return;
    }
    onSelect(node.workspace?.path || node.path);
  }

  return (
    <div className="overflow-hidden rounded-lg border bg-card text-card-foreground shadow-sm">
      <div className="flex items-center gap-2 px-3 py-2.5">
        <Button
          aria-label={t('Back')}
          disabled={!currentPath || disabled}
          onClick={() => setCurrentPath(parentWorkspacePath(currentPath))}
          size="icon-sm"
          type="button"
          variant="ghost"
        >
          <ArrowLeft />
        </Button>
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-medium">
            {currentPath ? workspaceLeafName(currentPath) || currentPath : t('Known folders')}
          </div>
          <div className="truncate text-xs text-muted-foreground">
            {currentPath ? workspaceCompactPath(currentPath) : t('Saved workspace roots')}
          </div>
        </div>
        {canUseCurrent ? (
          <Button
            disabled={disabled}
            onClick={() => onSelect(currentPath)}
            size="sm"
            type="button"
            variant={isCurrentSelected ? 'secondary' : 'outline'}
          >
            {isCurrentSelected ? <Check /> : <FolderOpen />}
            {isCurrentSelected ? t('Selected') : t('Use folder')}
          </Button>
        ) : null}
      </div>
      <Separator />
      <div className="max-h-60 overflow-auto p-1">
        {rows.length ? (
          <div className="space-y-0.5">
            {rows.map((node) => {
              const normalizedNodePath = normalizeWorkspacePath(node.workspace?.path || node.path);
              const isSelected = normalizedSelected === normalizedNodePath;
              const hasChildren = node.children.length > 0;
              return (
                <button
                  className={cn(
                    'flex min-h-11 w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm transition-colors',
                    disabled ? 'cursor-not-allowed opacity-50' : 'hover:bg-accent hover:text-accent-foreground',
                    isSelected ? 'bg-accent text-accent-foreground' : '',
                  )}
                  disabled={disabled}
                  key={node.path}
                  onClick={() => activateNode(node)}
                  type="button"
                >
                  {hasChildren ? (
                    <FolderOpen aria-hidden className="size-4 text-muted-foreground" />
                  ) : (
                    <Folder aria-hidden className="size-4 text-muted-foreground" />
                  )}
                  <span className="min-w-0 flex-1">
                    <span className="block truncate font-medium">{node.name}</span>
                    <span className="block truncate text-xs text-muted-foreground">
                      {workspaceCompactPath(node.path)}
                    </span>
                  </span>
                  {isSelected ? (
                    <Check aria-hidden className="size-4 text-foreground" />
                  ) : hasChildren ? (
                    <ChevronRight aria-hidden className="size-4 text-muted-foreground" />
                  ) : null}
                </button>
              );
            })}
          </div>
        ) : (
          <div className="px-3 py-8 text-center text-sm text-muted-foreground">
            {t('No saved folders at this level.')}
          </div>
        )}
      </div>
    </div>
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
  showKnownTree = true,
}: WorkspacePathPickerProps) {
  const { t } = useI18n();
  const tree = useMemo(() => buildWorkspaceTree(workspaces), [workspaces]);
  const trimmed = value.trim();
  const invalid = Boolean(trimmed && !isAbsoluteWorkspacePath(trimmed));

  async function handleBrowse() {
    const picked = await window.garyxDesktop.pickDirectory({
      defaultPath: trimmed || null,
    });
    if (picked) {
      onChange(picked);
    }
  }

  return (
    <FieldGroup className="gap-3">
      <Field className="gap-2" data-invalid={invalid || undefined}>
        <div className="flex items-center gap-2">
          <Input
            className="flex-1"
            disabled={disabled}
            id={id}
            onChange={(event) => onChange(event.target.value)}
            placeholder={placeholder || t('/path/to/project')}
            type="text"
            value={value}
          />
          <Button
            className="shrink-0"
            disabled={disabled}
            onClick={handleBrowse}
            size="sm"
            type="button"
            variant="outline"
          >
            <Folder />
            {t('Browse...')}
          </Button>
        </div>
        {invalid ? (
          <FieldDescription className="text-destructive">
            {t('Workspace paths must be absolute directories.')}
          </FieldDescription>
        ) : null}
        {!allowEmpty && !trimmed ? (
          <FieldDescription>{t('Choose or enter an absolute directory path.')}</FieldDescription>
        ) : null}
      </Field>
      {showKnownTree && tree.length ? (
        <Field className="gap-2">
          <div className="flex items-center justify-between gap-2">
            <FieldDescription>{t('Saved folders')}</FieldDescription>
            <Badge variant="outline">{workspaces.length}</Badge>
          </div>
          <WorkspacePathBrowser
            disabled={disabled}
            nodes={tree}
            onSelect={onChange}
            selectedPath={value}
          />
        </Field>
      ) : null}
    </FieldGroup>
  );
}

export function WorkspacePathPickerDialog({
  open,
  title,
  description,
  initialPath = '',
  workspaces = [],
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
      <DialogContent className="sm:max-w-[680px]" size="wide">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          {description ? <DialogDescription>{description}</DialogDescription> : null}
        </DialogHeader>
        <WorkspacePathPicker
          allowEmpty={false}
          disabled={saving}
          onChange={setDraft}
          value={draft}
          workspaces={workspaces}
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
