import { mkdir, readFile, realpath, stat, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';

import type {
  DesktopMemoryDocument,
  ReadMemoryDocumentInput,
  SaveMemoryDocumentInput,
} from '@shared/contracts';
import { DEFAULT_GARY_HOME_DIR } from './config-paths';

function garyHomeDir(): string {
  return DEFAULT_GARY_HOME_DIR;
}

function autoMemoryDir(): string {
  return join(garyHomeDir(), 'auto-memory');
}

function autoMemoryAutomationKey(automationId: string): string {
  const trimmed = automationId.trim();
  const base = trimmed || 'automation';
  let sanitized = '';
  for (const ch of base) {
    if (/^[a-z0-9]$/i.test(ch)) {
      sanitized += ch.toLowerCase();
    } else if (!sanitized.endsWith('-')) {
      sanitized += '-';
    }
  }
  const normalized = sanitized.replace(/^-+|-+$/g, '');
  return normalized || 'automation';
}

function fnv1a64Hex(value: string): string {
  let hash = 0xcbf29ce484222325n;
  const prime = 0x100000001b3n;
  const mask = 0xffffffffffffffffn;
  for (const byte of Buffer.from(value, 'utf8')) {
    hash ^= BigInt(byte);
    hash = (hash * prime) & mask;
  }
  return hash.toString(16).padStart(16, '0');
}

function sanitizeWorkspaceDisplayName(workspacePath: string): string {
  const normalized = workspacePath.replace(/[\\/]+$/, '');
  const segments = normalized.split(/[\\/]/).filter(Boolean);
  const base = segments[segments.length - 1] || 'workspace';
  let sanitized = '';
  for (const ch of base) {
    if (/^[a-z0-9]$/i.test(ch)) {
      sanitized += ch.toLowerCase();
    } else if (!sanitized.endsWith('-')) {
      sanitized += '-';
    }
  }
  const trimmed = sanitized.replace(/^-+|-+$/g, '');
  return trimmed || 'workspace';
}

async function autoMemoryWorkspaceKey(workspacePath: string): Promise<string> {
  const normalized = workspacePath.trim();
  if (!normalized) {
    throw new Error('Workspace memory requires a workspacePath.');
  }
  const canonicalPath = await realpath(normalized).catch(() => normalized);
  return `${sanitizeWorkspaceDisplayName(canonicalPath)}-${fnv1a64Hex(canonicalPath)}`;
}

async function resolveMemoryPath(
  input: ReadMemoryDocumentInput,
): Promise<{
  scope: DesktopMemoryDocument['scope'];
  automationId: string | null;
  workspacePath: string | null;
  path: string;
}> {
  if (input.scope === 'automation') {
    const automationId = input.automationId?.trim() || '';
    if (!automationId) {
      throw new Error('Automation memory requires an automationId.');
    }
    return {
      scope: 'automation',
      automationId,
      workspacePath: null,
      path: join(
        autoMemoryDir(),
        'automations',
        autoMemoryAutomationKey(automationId),
        'memory.md',
      ),
    };
  }

  if (input.scope === 'workspace') {
    const workspacePath = input.workspacePath?.trim() || '';
    return {
      scope: 'workspace',
      automationId: null,
      workspacePath,
      path: join(
        autoMemoryDir(),
        'workspaces',
        await autoMemoryWorkspaceKey(workspacePath),
        'memory.md',
      ),
    };
  }

  return {
    scope: 'global',
    automationId: null,
    workspacePath: null,
    path: join(autoMemoryDir(), 'memory.md'),
  };
}

export async function readMemoryDocument(
  input: ReadMemoryDocumentInput,
): Promise<DesktopMemoryDocument> {
  const target = await resolveMemoryPath(input);
  try {
    const [content, fileStat] = await Promise.all([
      readFile(target.path, 'utf8'),
      stat(target.path),
    ]);
    return {
      ...target,
      content,
      exists: true,
      modifiedAt: fileStat.mtime.toISOString(),
    };
  } catch (error) {
    if ((error as NodeJS.ErrnoException | null)?.code === 'ENOENT') {
      return {
        ...target,
        content: '',
        exists: false,
        modifiedAt: null,
      };
    }
    throw error;
  }
}

export async function saveMemoryDocument(
  input: SaveMemoryDocumentInput,
): Promise<DesktopMemoryDocument> {
  const target = await resolveMemoryPath(input);
  await mkdir(dirname(target.path), { recursive: true });
  await writeFile(target.path, input.content.replace(/\r\n?/g, '\n'), 'utf8');
  return readMemoryDocument(input);
}
