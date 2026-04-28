import type { DesktopWorkspace, DesktopWorkspaceFileEntry } from '@shared/contracts';

import type { WorkspaceDirectoryState } from './types';

export function compactPathLabel(path?: string | null): string {
  const trimmed = path?.trim() || '';
  if (!trimmed) {
    return 'Workspace unavailable';
  }

  const segments = trimmed.split(/[\\/]/).filter(Boolean);
  return segments[segments.length - 1] || trimmed;
}

export function workspaceDirectoryKey(workspacePath: string, directoryPath = ''): string {
  return `${workspacePath}::${directoryPath}`;
}

export function parentDirectoryPath(path: string): string {
  const normalized = path.trim().replace(/^\/+|\/+$/g, '');
  if (!normalized) {
    return '';
  }
  const segments = normalized.split('/').filter(Boolean);
  if (segments.length <= 1) {
    return '';
  }
  return segments.slice(0, -1).join('/');
}

function normalizeWorkspaceRootPath(path: string): string {
  return path.trim().replace(/\/+$/, '');
}

function relativeWorkspaceFilePath(workspacePath: string, absolutePath: string): string | null {
  const normalizedWorkspacePath = normalizeWorkspaceRootPath(workspacePath);
  const normalizedAbsolutePath = absolutePath.trim();
  if (!normalizedWorkspacePath || !normalizedAbsolutePath || !normalizedAbsolutePath.startsWith('/')) {
    return null;
  }
  if (normalizedAbsolutePath === normalizedWorkspacePath) {
    return '';
  }
  const prefix = `${normalizedWorkspacePath}/`;
  if (!normalizedAbsolutePath.startsWith(prefix)) {
    return null;
  }
  return normalizedAbsolutePath.slice(prefix.length);
}

export function workspaceFileAbsolutePath(workspacePath: string, filePath: string): string {
  const normalizedWorkspacePath = normalizeWorkspaceRootPath(workspacePath);
  if (normalizedWorkspacePath === '/') {
    return `/${filePath}`;
  }
  return `${normalizedWorkspacePath}/${filePath}`;
}

function absoluteFileName(path: string): string {
  const normalized = path.trim().replace(/\/+$/, '');
  const lastSlash = normalized.lastIndexOf('/');
  if (lastSlash < 0 || lastSlash === normalized.length - 1) {
    return '';
  }
  return normalized.slice(lastSlash + 1);
}

function absoluteFileParentPath(path: string): string | null {
  const normalized = path.trim().replace(/\/+$/, '');
  if (!normalized.startsWith('/')) {
    return null;
  }
  const lastSlash = normalized.lastIndexOf('/');
  if (lastSlash < 0 || lastSlash === normalized.length - 1) {
    return null;
  }
  return lastSlash === 0 ? '/' : normalized.slice(0, lastSlash);
}

export function resolveLocalFilePreviewTarget(
  workspaces: DesktopWorkspace[],
  absolutePath: string,
): { workspacePath: string; filePath: string } | null {
  const candidates = workspaces
    .filter((workspace) => workspace.available && Boolean(workspace.path))
    .map((workspace) => {
      const workspacePath = workspace.path?.trim() || '';
      const filePath = relativeWorkspaceFilePath(workspacePath, absolutePath);
      if (!workspacePath || !filePath) {
        return null;
      }
      return {
        workspacePath,
        filePath,
      };
    })
    .filter((candidate): candidate is { workspacePath: string; filePath: string } => Boolean(candidate));

  if (candidates.length) {
    return candidates.sort(
      (left, right) => right.workspacePath.length - left.workspacePath.length,
    )[0] || null;
  }

  const workspacePath = absoluteFileParentPath(absolutePath);
  const filePath = absoluteFileName(absolutePath);
  if (!workspacePath || !filePath) {
    return null;
  }
  return { workspacePath, filePath };
}

function directoryAncestors(path: string): string[] {
  const normalized = path.trim().replace(/^\/+|\/+$/g, '');
  if (!normalized) {
    return [''];
  }
  const segments = normalized.split('/').filter(Boolean);
  const directories = [''];
  let current = '';
  for (const segment of segments) {
    current = current ? `${current}/${segment}` : segment;
    directories.push(current);
  }
  return directories;
}

export function expandWorkspaceDirectoryState(
  current: Record<string, boolean>,
  workspacePath: string,
  directoryPath: string,
): Record<string, boolean> {
  const next = { ...current };
  for (const path of directoryAncestors(directoryPath)) {
    next[workspaceDirectoryKey(workspacePath, path)] = true;
  }
  return next;
}

export function findWorkspaceFileEntry(
  directories: Record<string, WorkspaceDirectoryState>,
  workspacePath: string,
  targetPath: string,
): DesktopWorkspaceFileEntry | null {
  const parentPath = parentDirectoryPath(targetPath);
  const parent = directories[workspaceDirectoryKey(workspacePath, parentPath)];
  if (!parent) {
    return null;
  }
  return parent.entries.find((entry) => entry.path === targetPath) || null;
}
