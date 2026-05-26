import { type ReactNode, useEffect, useMemo, useState } from 'react';
import { Check, ChevronRight, Folder, FolderOpen } from 'lucide-react';

import type { DesktopWorkspace } from '@shared/contracts';

import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { useI18n } from '@/i18n';

type WorkspaceTreeNode = {
  name: string;
  path: string;
  workspace: DesktopWorkspace | null;
  children: WorkspaceTreeNode[];
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

function displayWorkspaceName(workspace: DesktopWorkspace): string {
  const name = workspace.name?.trim();
  if (name) return name;
  const path = normalizeWorkspacePath(workspace.path || '');
  return path.split('/').filter(Boolean).pop() || path || 'Workspace';
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

function buildWorkspaceTree(workspaces: DesktopWorkspace[] = []): WorkspaceTreeNode[] {
  const roots = new Map<string, WorkspaceTreeNode>();

  for (const workspace of workspaces) {
    const path = normalizeWorkspacePath(workspace.path || '');
    if (!path) continue;
    const split = splitAbsolutePath(path);
    if (!split) continue;

    let current: WorkspaceTreeNode;
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
      let child: WorkspaceTreeNode | undefined = current.children.find((item) => item.name === part);
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

  function sortNode(node: WorkspaceTreeNode): WorkspaceTreeNode {
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

function allExpandablePaths(nodes: WorkspaceTreeNode[]): string[] {
  const out: string[] = [];
  function visit(node: WorkspaceTreeNode) {
    if (node.children.length) {
      out.push(node.path);
      node.children.forEach(visit);
    }
  }
  nodes.forEach(visit);
  return out;
}

type WorkspacePathTreeProps = {
  nodes: WorkspaceTreeNode[];
  selectedPath: string;
  onSelect: (path: string) => void;
};

function WorkspacePathTree({ nodes, selectedPath, onSelect }: WorkspacePathTreeProps) {
  const { t } = useI18n();
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set(allExpandablePaths(nodes)));

  useEffect(() => {
    setExpanded(new Set(allExpandablePaths(nodes)));
  }, [nodes]);

  function renderNode(node: WorkspaceTreeNode, depth: number): ReactNode {
    const isExpanded = expanded.has(node.path);
    const isSelectable = Boolean(node.workspace?.path);
    const isSelected = normalizeWorkspacePath(selectedPath) === normalizeWorkspacePath(node.workspace?.path || '');
    const hasChildren = node.children.length > 0;

    return (
      <div key={node.path}>
        <div
          className={[
            'flex min-h-8 w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors',
            isSelectable ? 'hover:bg-muted/70' : 'text-muted-foreground',
            !isSelectable && !hasChildren ? 'cursor-default' : 'cursor-pointer',
            isSelected ? 'bg-muted text-foreground' : '',
          ].join(' ')}
          onClick={() => {
            if (isSelectable && node.workspace?.path) {
              onSelect(node.workspace.path);
              return;
            }
            if (hasChildren) {
              setExpanded((current) => {
                const next = new Set(current);
                if (next.has(node.path)) next.delete(node.path);
                else next.add(node.path);
                return next;
              });
            }
          }}
          role={isSelectable || hasChildren ? 'button' : undefined}
          tabIndex={isSelectable || hasChildren ? 0 : undefined}
          onKeyDown={(event) => {
            if (event.key !== 'Enter' && event.key !== ' ') return;
            event.preventDefault();
            if (isSelectable && node.workspace?.path) {
              onSelect(node.workspace.path);
              return;
            }
            if (hasChildren) {
              setExpanded((current) => {
                const next = new Set(current);
                if (next.has(node.path)) next.delete(node.path);
                else next.add(node.path);
                return next;
              });
            }
          }}
        >
          <span style={{ width: depth * 14 }} aria-hidden />
          {hasChildren ? (
            <button
              aria-label={isExpanded ? t('Collapse folder') : t('Expand folder')}
              className="grid size-5 place-items-center rounded-sm text-muted-foreground hover:bg-muted"
              onClick={(event) => {
                event.stopPropagation();
                setExpanded((current) => {
                  const next = new Set(current);
                  if (next.has(node.path)) next.delete(node.path);
                  else next.add(node.path);
                  return next;
                });
              }}
              type="button"
            >
              <ChevronRight
                aria-hidden
                className={isExpanded ? 'rotate-90 transition-transform' : 'transition-transform'}
                size={14}
              />
            </button>
          ) : (
            <span className="size-5" aria-hidden />
          )}
          {hasChildren && isExpanded ? (
            <FolderOpen aria-hidden className="size-4 text-muted-foreground" />
          ) : (
            <Folder aria-hidden className="size-4 text-muted-foreground" />
          )}
          <span className="min-w-0 flex-1 truncate">
            {node.workspace ? displayWorkspaceName(node.workspace) : node.name}
          </span>
          {isSelected ? <Check aria-hidden className="size-4 text-foreground" /> : null}
        </div>
        {hasChildren && isExpanded ? node.children.map((child) => renderNode(child, depth + 1)) : null}
      </div>
    );
  }

  return <div className="space-y-0.5">{nodes.map((node) => renderNode(node, 0))}</div>;
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
    <div className="grid gap-2">
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
          {t('Browse...')}
        </Button>
      </div>
      {invalid ? (
        <div className="text-xs text-destructive">
          {t('Workspace paths must be absolute directories.')}
        </div>
      ) : null}
      {!allowEmpty && !trimmed ? (
        <div className="text-xs text-muted-foreground">
          {t('Choose or enter an absolute directory path.')}
        </div>
      ) : null}
      {showKnownTree && tree.length ? (
        <div className="max-h-56 overflow-auto rounded-md border bg-background/80 p-1">
          <WorkspacePathTree nodes={tree} selectedPath={value} onSelect={onChange} />
        </div>
      ) : null}
    </div>
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
