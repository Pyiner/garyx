import { mkdir, readFile, stat, writeFile } from 'node:fs/promises';
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

function memoryKey(value: string, fallback: string): string {
  const trimmed = value.trim();
  const base = trimmed || fallback;
  let sanitized = '';
  for (const ch of base) {
    if (/^[a-z0-9]$/i.test(ch)) {
      sanitized += ch.toLowerCase();
    } else if (!sanitized.endsWith('-')) {
      sanitized += '-';
    }
  }
  const normalized = sanitized.replace(/^-+|-+$/g, '');
  return normalized || fallback;
}

async function resolveMemoryPath(
  input: ReadMemoryDocumentInput,
): Promise<{
  scope: DesktopMemoryDocument['scope'];
  agentId: string | null;
  automationId: string | null;
  path: string;
}> {
  if (input.scope === 'agent') {
    const agentId = input.agentId?.trim() || '';
    if (!agentId) {
      throw new Error('Agent memory requires an agentId.');
    }
    return {
      scope: 'agent',
      agentId,
      automationId: null,
      path: join(garyHomeDir(), 'agents', memoryKey(agentId, 'agent'), 'memory.md'),
    };
  }

  if (input.scope === 'automation') {
    const automationId = input.automationId?.trim() || '';
    if (!automationId) {
      throw new Error('Automation memory requires an automationId.');
    }
    return {
      scope: 'automation',
      agentId: null,
      automationId,
      path: join(garyHomeDir(), 'automations', memoryKey(automationId, 'automation'), 'memory.md'),
    };
  }

  throw new Error('Unsupported memory scope.');
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
