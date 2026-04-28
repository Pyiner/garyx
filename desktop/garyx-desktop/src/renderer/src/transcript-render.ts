import type { TranscriptMessage } from '@shared/contracts';

import { canMergeToolTraceMessages, type ToolTraceMessage } from './tool-trace';

export type RenderTranscriptEntry =
  | {
      kind: 'message';
      key: string;
      message: TranscriptMessage;
    }
  | {
      kind: 'tool';
      key: string;
      toolUse?: TranscriptMessage;
      toolResult?: TranscriptMessage;
      defaultExpanded: boolean;
    };

export type RenderTranscriptBlock =
  | {
      kind: 'message';
      key: string;
      entry: Extract<RenderTranscriptEntry, { kind: 'message' }>;
    }
  | {
      kind: 'tool_group';
      key: string;
      entries: Array<Extract<RenderTranscriptEntry, { kind: 'tool' }>>;
    };

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

export function isToolRole(role: TranscriptMessage['role']): role is 'tool_use' | 'tool_result' {
  return role === 'tool_use' || role === 'tool_result';
}

export function extractToolUseId(message: TranscriptMessage): string | null {
  if (message.toolUseId) return message.toolUseId;
  const content = message.content;
  if (isRecord(content)) {
    const id =
      (typeof content.tool_use_id === 'string' && content.tool_use_id) ||
      (typeof content.toolUseId === 'string' && content.toolUseId);
    if (id) return id;
    if (isRecord(content.content)) {
      const innerId =
        (typeof content.content.tool_use_id === 'string' && content.content.tool_use_id) ||
        (typeof content.content.toolUseId === 'string' && content.content.toolUseId);
      if (innerId) return innerId;
    }
  }
  if (typeof message.text === 'string') {
    try {
      const parsed = JSON.parse(message.text);
      if (isRecord(parsed)) {
        const id =
          (typeof parsed.tool_use_id === 'string' && parsed.tool_use_id) ||
          (typeof parsed.toolUseId === 'string' && parsed.toolUseId);
        if (id) return id;
      }
    } catch {
      // plain text, ignore
    }
  }
  return null;
}

function stableSerialize(value: unknown): string {
  if (value === null || value === undefined) {
    return '';
  }
  if (typeof value === 'string') {
    return value;
  }
  if (typeof value !== 'object') {
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map((entry) => stableSerialize(entry)).join(',')}]`;
  }
  const entries = Object.entries(value)
    .filter(([, entryValue]) => entryValue !== undefined)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([key, entryValue]) => `${JSON.stringify(key)}:${stableSerialize(entryValue)}`);
  return `{${entries.join(',')}}`;
}

function toolMessageFingerprint(message: TranscriptMessage): string {
  return [
    message.role,
    extractToolUseId(message) || '',
    message.toolName || '',
    message.isError ? '1' : '0',
    message.text || '',
    stableSerialize(message.content),
    stableSerialize(message.metadata),
  ].join('::');
}

export function toolMessagesEquivalent(
  left: TranscriptMessage,
  right: TranscriptMessage,
): boolean {
  if (left.role !== right.role || !isToolRole(left.role) || !isToolRole(right.role)) {
    return false;
  }

  const leftToolUseId = extractToolUseId(left);
  const rightToolUseId = extractToolUseId(right);
  if (leftToolUseId && rightToolUseId) {
    return leftToolUseId === rightToolUseId;
  }

  return toolMessageFingerprint(left) === toolMessageFingerprint(right);
}

export function buildRenderableTranscript(messages: TranscriptMessage[]): RenderTranscriptEntry[] {
  const rendered: RenderTranscriptEntry[] = [];
  const pendingToolUses = new Map<string, number>();

  for (const message of messages) {
    if (!isToolRole(message.role)) {
      pendingToolUses.clear();
      rendered.push({
        kind: 'message',
        key: message.id,
        message,
      });
      continue;
    }

    const messageToolUseId = extractToolUseId(message);

    if (message.role === 'tool_result') {
      let matched = false;

      if (messageToolUseId) {
        const idx = pendingToolUses.get(messageToolUseId);
        if (idx !== undefined) {
          const current = rendered[idx];
          if (current?.kind === 'tool' && current.toolUse && !current.toolResult) {
            current.toolResult = message;
            pendingToolUses.delete(messageToolUseId);
            matched = true;
          }
        }
      }

      if (!matched) {
        for (const [key, idx] of pendingToolUses) {
          const current = rendered[idx];
          if (
            current?.kind === 'tool' &&
            current.toolUse &&
            !current.toolResult &&
            canMergeToolTraceMessages(
              current.toolUse as ToolTraceMessage,
              message as ToolTraceMessage,
            )
          ) {
            current.toolResult = message;
            pendingToolUses.delete(key);
            matched = true;
            break;
          }
        }
      }

      if (!matched) {
        rendered.push({
          kind: 'tool',
          key: message.id,
          toolResult: message,
          defaultExpanded: false,
        });
      }
      continue;
    }

    rendered.push({
      kind: 'tool',
      key: message.id,
      toolUse: message,
      defaultExpanded: false,
    });
    if (messageToolUseId) {
      pendingToolUses.set(messageToolUseId, rendered.length - 1);
    }
  }

  return rendered;
}

export function buildRenderTranscriptBlocks(
  entries: RenderTranscriptEntry[],
): RenderTranscriptBlock[] {
  const blocks: RenderTranscriptBlock[] = [];
  let pendingTools: Array<Extract<RenderTranscriptEntry, { kind: 'tool' }>> = [];

  function flushPendingTools() {
    if (!pendingTools.length) {
      return;
    }
    const firstKey = pendingTools[0]?.key || crypto.randomUUID();
    blocks.push({
      kind: 'tool_group',
      key: `tool-group:${firstKey}`,
      entries: pendingTools,
    });
    pendingTools = [];
  }

  for (const entry of entries) {
    if (entry.kind === 'tool') {
      pendingTools.push(entry);
      continue;
    }

    flushPendingTools();
    blocks.push({
      kind: 'message',
      key: entry.key,
      entry,
    });
  }

  flushPendingTools();
  return blocks;
}
