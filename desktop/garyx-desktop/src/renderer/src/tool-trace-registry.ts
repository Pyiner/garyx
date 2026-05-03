import type { TranscriptMessage } from '@shared/contracts';

type ToolRole = 'tool_use' | 'tool_result';

export type ToolTraceMessage = Pick<
  TranscriptMessage,
  'role' | 'text' | 'content' | 'toolUseId' | 'toolName' | 'metadata' | 'isError'
>;

type ToolTraceEnvelope = {
  role: ToolRole;
  wrapper: Record<string, unknown> | null;
  payload: unknown;
  toolUseId: string | null;
  toolName: string | null;
  metadata: Record<string, unknown> | null;
  isError: boolean;
};

type ParsedToolTrace = {
  role: ToolRole;
  provider: string;
  toolUseId: string | null;
  toolKey: string;
  toolName: string;
  payload: unknown;
  input?: Record<string, unknown> | null;
  result?: unknown;
  metadata: Record<string, unknown> | null;
  isError: boolean;
};

type ToolTraceStatusTone = 'neutral' | 'progress' | 'error';

type ToolTraceStatus = {
  label: string;
  tone: ToolTraceStatusTone;
};

type DiffStats = {
  added: number;
  removed: number;
};

type ToolTraceSide = {
  title: string;
  summary?: string;
  badges?: string[];
  status?: ToolTraceStatus;
  detail?: string;
  detailLabel?: string;
  icon?: string;
  diffStats?: DiffStats;
};

export type MergedToolTrace = {
  title: string;
  summary?: string;
  resultSummary?: string;
  badges: string[];
  diffStats?: DiffStats;
  status?: ToolTraceStatus;
  inputDetail?: string;
  inputLabel?: string;
  resultDetail?: string;
  resultLabel?: string;
  icon: string;
  isError: boolean;
};

type ToolTraceParser = {
  id: string;
  parse: (envelope: ToolTraceEnvelope) => ParsedToolTrace | null;
};

type ToolTraceAdapter = {
  id: string;
  matches: (trace: ParsedToolTrace) => boolean;
  present: (trace: ParsedToolTrace) => ToolTraceSide;
};

const CODEX_ITEM_TYPES = [
  'hookPrompt',
  'plan',
  'reasoning',
  'commandExecution',
  'fileChange',
  'mcpToolCall',
  'dynamicToolCall',
  'collabAgentToolCall',
  'webSearch',
  'imageView',
  'imageGeneration',
  'enteredReviewMode',
  'exitedReviewMode',
  'contextCompaction',
];

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function asArray(value: unknown): unknown[] | null {
  return Array.isArray(value) ? value : null;
}

function asString(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value.trim() : undefined;
}

function asNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
}

function asBoolean(value: unknown): boolean | undefined {
  return typeof value === 'boolean' ? value : undefined;
}

function canonicalCodexItemType(value: string | undefined): string | undefined {
  const normalized = value?.trim();
  if (!normalized) {
    return undefined;
  }
  return CODEX_ITEM_TYPES.find((entry) => entry.toLowerCase() === normalized.toLowerCase());
}

function parseMaybeJson(value: unknown): unknown {
  if (typeof value !== 'string') {
    return value;
  }
  const trimmed = value.trim();
  if (!trimmed.startsWith('{') && !trimmed.startsWith('[')) {
    return value;
  }
  try {
    return JSON.parse(trimmed);
  } catch {
    return value;
  }
}

function stringifyUnknown(value: unknown): string {
  if (typeof value === 'string') {
    return value;
  }
  if (value === null || value === undefined) {
    return '';
  }
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function normalizeWhitespace(value: string | undefined): string | undefined {
  const trimmed = value?.trim();
  return trimmed ? trimmed.replace(/\s+/g, ' ').trim() : undefined;
}

function truncateSingleLine(value: string | undefined, maxLength = 180): string | undefined {
  const singleLine = normalizeWhitespace(value);
  if (!singleLine) {
    return undefined;
  }
  return singleLine.length > maxLength ? `${singleLine.slice(0, maxLength - 3)}...` : singleLine;
}

function truncateDetail(value: string | undefined, maxLines = 30, maxLength = 4000): string | undefined {
  if (!value) {
    return undefined;
  }
  let text = value.length > maxLength ? `${value.slice(0, maxLength)}…` : value;
  const lines = text.split('\n');
  if (lines.length > maxLines) {
    text = `${lines.slice(0, maxLines).join('\n')}\n… (${lines.length - maxLines} more lines)`;
  }
  return text || undefined;
}

function truncateMiddleSingleLine(value: string | undefined, maxLength = 120): string | undefined {
  const singleLine = normalizeWhitespace(value);
  if (!singleLine) {
    return undefined;
  }
  if (singleLine.length <= maxLength) {
    return singleLine;
  }

  const headLength = Math.ceil((maxLength - 3) / 2);
  const tailLength = Math.floor((maxLength - 3) / 2);
  return `${singleLine.slice(0, headLength)}...${singleLine.slice(singleLine.length - tailLength)}`;
}

function firstMeaningfulLine(value: string | undefined): string | undefined {
  const normalized = value?.replace(/\r/g, '\n');
  if (!normalized) {
    return undefined;
  }
  const line = normalized
    .split('\n')
    .map((entry) => entry.trim())
    .find(Boolean);
  return line || undefined;
}

function unwrapMatchingQuotes(value: string | undefined): string | undefined {
  const trimmed = value?.trim();
  if (!trimmed || trimmed.length < 2) {
    return trimmed;
  }
  const first = trimmed[0];
  const last = trimmed[trimmed.length - 1];
  if ((first === '\'' || first === '"') && last === first) {
    return trimmed.slice(1, -1).trim();
  }
  return trimmed;
}

function simplifyShellCommand(command: string | undefined): string | undefined {
  let normalized = command?.trim();
  if (!normalized) {
    return undefined;
  }

  const launchers = [
    '/bin/bash -lc ',
    'bash -lc ',
    '/bin/sh -lc ',
    'sh -lc ',
    '/bin/zsh -lc ',
    'zsh -lc ',
  ];
  for (const launcher of launchers) {
    if (normalized.startsWith(launcher)) {
      normalized = unwrapMatchingQuotes(normalized.slice(launcher.length)) || normalized;
      break;
    }
  }

  normalized = normalized
    .replace(/\s+\|\|\s+true\b/g, '')
    .replace(/\s+2>&1\b/g, '')
    .replace(/\s+/g, ' ')
    .trim();

  const sleepMatch = normalized.match(/^sleep\s+([0-9.]+)\s+(.+)$/i);
  if (sleepMatch?.[1] && sleepMatch[2]) {
    normalized = `wait ${sleepMatch[1]}s, ${sleepMatch[2].trim()}`;
  }

  return normalized;
}

function shortenInlinePaths(value: string | undefined): string | undefined {
  const normalized = value?.trim();
  if (!normalized) {
    return undefined;
  }

  return normalized.replace(/(?:~\/|\/)[^\s'"`]+/g, (match) => {
    if (!match.startsWith('/') && !match.startsWith('~/')) {
      return match;
    }
    return pathTail(match) || match;
  });
}

function tokenizeShellCommand(command: string | undefined): string[] {
  const normalized = command?.trim();
  if (!normalized) {
    return [];
  }
  const tokens = normalized.match(/"[^"]*"|'[^']*'|\S+/g) || [];
  return tokens.map((token) => unwrapMatchingQuotes(token) || token);
}

function describeShellCommand(command: string | undefined): string | undefined {
  const simplified = simplifyShellCommand(command);
  if (!simplified) {
    return undefined;
  }

  let prefix = '';
  let body = simplified;
  const waitMatch = simplified.match(/^wait\s+([0-9.]+s?),\s+(.+)$/i);
  if (waitMatch?.[1] && waitMatch[2]) {
    prefix = `wait ${waitMatch[1]}, `;
    body = waitMatch[2].trim();
  }

  const tokens = tokenizeShellCommand(body);
  const executable = tokens[0]?.split('/').pop()?.toLowerCase();
  const source = tokens[1];
  const destination = tokens[2];

  switch (executable) {
    case 'cp':
      if (source && destination) {
        return `${prefix}copy ${pathTail(source) || source} -> ${pathTail(destination) || destination}`;
      }
      break;
    case 'mv':
      if (source && destination) {
        return `${prefix}move ${pathTail(source) || source} -> ${pathTail(destination) || destination}`;
      }
      break;
    case 'ls':
      if (source) {
        return `${prefix}list ${pathTail(source) || source}`;
      }
      return `${prefix}list files`;
    case 'head':
      if (source) {
        return `${prefix}head ${pathTail(source) || source}`;
      }
      break;
    case 'tail':
      if (source) {
        return `${prefix}tail ${pathTail(source) || source}`;
      }
      break;
    case 'curl': {
      const outputIndex = tokens.findIndex((token) => token === '-o' || token === '--output');
      const outputFile = outputIndex >= 0 ? tokens[outputIndex + 1] : undefined;
      const urlToken = tokens.find((token) => /^https?:\/\//i.test(token));
      const url = parseUrl(urlToken);
      if (outputFile && url) {
        return `${prefix}download ${pathTail(outputFile) || outputFile} from ${url.host}`;
      }
      break;
    }
    default:
      break;
  }

  return `${prefix}${truncateMiddleSingleLine(shortenInlinePaths(body), 112) || body}`;
}

function looksLikeErrorText(value: unknown): boolean {
  if (typeof value !== 'string') {
    return false;
  }
  const normalized = value.trim();
  return /^error(?:\b|:)/i.test(normalized) || /^failed(?:\b|:)/i.test(normalized);
}

function summarizeTextPayload(value: unknown, maxLength = 160): string | undefined {
  if (typeof value === 'string') {
    return truncateSingleLine(firstMeaningfulLine(value) || value, maxLength);
  }

  const record = asRecord(parseMaybeJson(value));
  if (!record) {
    return undefined;
  }

  const primaryText =
    asString(record.aggregatedOutput) ||
    asString(record.stdout) ||
    asString(record.stderr) ||
    asString(record.text) ||
    asString(record.result) ||
    asString(record.message);

  if (primaryText) {
    return truncateSingleLine(firstMeaningfulLine(primaryText) || primaryText, maxLength);
  }

  return undefined;
}

function toolStatusFromState(status: string | undefined): ToolTraceStatus | undefined {
  switch (status?.trim().toLowerCase()) {
    case 'in_progress':
    case 'inprogress':
    case 'running':
    case 'started':
      return { label: 'running', tone: 'progress' };
    case 'failed':
    case 'declined':
    case 'errored':
    case 'error':
    case 'canceled':
    case 'cancelled':
      return { label: status.toLowerCase(), tone: 'error' };
    case 'completed':
    case 'done':
    case 'success':
      return { label: 'done', tone: 'neutral' };
    default:
      return undefined;
  }
}

function commandStatus(payload: Record<string, unknown> | null): ToolTraceStatus | undefined {
  const exitCode =
    asNumber(payload?.exitCode) ??
    asNumber(asRecord(payload?.output)?.exitCode);
  if (exitCode !== undefined) {
    if (exitCode === 0) {
      return undefined;
    }
    return {
      label: `exit ${exitCode}`,
      tone: 'error',
    };
  }
  const status = asString(payload?.status)?.toLowerCase();
  if (status === 'inprogress') {
    return { label: 'running', tone: 'progress' };
  }
  if (status === 'failed' || status === 'declined' || status === 'errored') {
    return { label: status, tone: 'error' };
  }
  return undefined;
}

function prettyPrintRecord(record: Record<string, unknown>, keys: string[]): string {
  const lines: string[] = [];
  for (const key of keys) {
    const value = record[key];
    if (value === null || value === undefined) {
      continue;
    }
    const text = typeof value === 'string' ? value : stringifyUnknown(value);
    if (!text.trim()) {
      continue;
    }
    lines.push(`${key}: ${text}`);
  }
  return lines.join('\n');
}

function formatCommandInvocation(payload: Record<string, unknown> | null | undefined): string {
  if (!payload) {
    return '';
  }
  const command = shortenInlinePaths(simplifyShellCommand(asString(payload.command)) || asString(payload.command));
  const cwd = pathTail(asString(payload.cwd)) || asString(payload.cwd);
  return prettyPrintRecord(
    {
      command,
      cwd,
    },
    ['command', 'cwd'],
  ) || stringifyUnknown(payload);
}

function formatReadInvocation(payload: Record<string, unknown> | null | undefined): string {
  if (!payload) {
    return '';
  }

  const path = extractPath(payload);
  const offset = asNumber(payload.offset);
  const limit =
    asNumber(payload.limit) ??
    asNumber(payload.count) ??
    asNumber(payload.max_lines);
  const startLine = asNumber(payload.start_line);
  const endLine = asNumber(payload.end_line);

  return (
    prettyPrintRecord(
      {
        path,
        offset: offset !== undefined ? String(offset) : undefined,
        limit: limit !== undefined ? String(limit) : undefined,
        start_line: startLine !== undefined ? String(startLine) : undefined,
        end_line: endLine !== undefined ? String(endLine) : undefined,
      },
      ['path', 'offset', 'limit', 'start_line', 'end_line'],
    ) || stringifyUnknown(payload)
  );
}

function pathTail(path: string | undefined): string | undefined {
  if (!path) {
    return undefined;
  }
  const normalized = path.replace(/\\/g, '/');
  const parts = normalized.split('/').filter(Boolean);
  if (parts.length <= 2) {
    return normalized;
  }
  return parts.slice(-2).join('/');
}

function countRangeLines(start?: number, end?: number): number | undefined {
  if (start === undefined || end === undefined) {
    return undefined;
  }
  if (!Number.isFinite(start) || !Number.isFinite(end) || end < start) {
    return undefined;
  }
  return end - start + 1;
}

function describeLineWindow(input: Record<string, unknown> | null | undefined): string | undefined {
  if (!input) {
    return undefined;
  }
  const limit =
    asNumber(input.limit) ??
    asNumber(input.count) ??
    asNumber(input.max_lines);
  if (limit && limit > 0) {
    return `${limit} lines`;
  }

  const rangeCount = countRangeLines(
    asNumber(input.start_line),
    asNumber(input.end_line),
  );
  if (rangeCount && rangeCount > 0) {
    return `${rangeCount} lines`;
  }

  return undefined;
}

function parseUrl(input: string | undefined): URL | null {
  if (!input) {
    return null;
  }
  try {
    return new URL(input);
  } catch {
    return null;
  }
}

function normalizeBadge(value: string | undefined): string | undefined {
  return truncateMiddleSingleLine(value, 44);
}

function dedupeBadges(values: Array<string | undefined>): string[] {
  const seen = new Set<string>();
  const output: string[] = [];
  for (const value of values) {
    const normalized = normalizeBadge(value);
    if (!normalized || seen.has(normalized)) {
      continue;
    }
    seen.add(normalized);
    output.push(normalized);
  }
  return output;
}

function extractPath(record: Record<string, unknown> | null | undefined): string | undefined {
  if (!record) {
    return undefined;
  }
  return (
    asString(record.file_path) ||
    asString(record.filePath) ||
    asString(record.path) ||
    asString(record.file)
  );
}

function describeTodos(input: Record<string, unknown> | null | undefined): string | undefined {
  const todos = asArray(input?.todos);
  if (!todos?.length) {
    return undefined;
  }

  let completed = 0;
  let inProgress = 0;
  for (const entry of todos) {
    const status = asString(asRecord(entry)?.status)?.toLowerCase();
    if (status === 'completed' || status === 'done') {
      completed += 1;
    } else if (status === 'in_progress' || status === 'active') {
      inProgress += 1;
    }
  }

  const pending = todos.length - completed - inProgress;
  const parts = [`${todos.length} items`];
  if (inProgress > 0) {
    parts.push(`${inProgress} active`);
  }
  if (pending > 0) {
    parts.push(`${pending} pending`);
  }
  if (completed > 0) {
    parts.push(`${completed} done`);
  }
  return parts.join(' · ');
}

function humanizeOperation(operation: string | undefined): string {
  const normalized = operation?.trim().toLowerCase();
  switch (normalized) {
    case 'create':
    case 'created':
    case 'write':
      return 'Created';
    case 'delete':
    case 'deleted':
      return 'Deleted';
    case 'move':
    case 'rename':
      return 'Moved';
    case 'replace':
    case 'update':
    case 'updated':
      return 'Updated';
    default:
      return 'Changed';
  }
}

function humanizeToolLabel(value: string | undefined): string {
  const label = value?.trim();
  if (!label) {
    return 'Tool';
  }
  const normalized = label.replace(/[_-]+/g, ' ');
  return normalized.charAt(0).toUpperCase() + normalized.slice(1);
}

function buildToolTraceEnvelope(message: ToolTraceMessage): ToolTraceEnvelope {
  const fallbackRole: ToolRole = message.role === 'tool_result' ? 'tool_result' : 'tool_use';
  const payloadValue = message.content ?? parseMaybeJson(message.text);
  const parsed = parseMaybeJson(payloadValue);
  const record = asRecord(parsed);

  if (!record) {
    return {
      role: fallbackRole,
      wrapper: null,
      payload: parsed,
      toolUseId: message.toolUseId ?? null,
      toolName: message.toolName ?? null,
      metadata: message.metadata ?? null,
      isError:
        message.isError ??
        (fallbackRole === 'tool_result' && looksLikeErrorText(parsed)),
    };
  }

  const role = record.role === 'tool_result' ? 'tool_result' : fallbackRole;
  const payload = 'content' in record ? parseMaybeJson(record.content) : record;
  const wrapper =
    'content' in record ||
    'toolName' in record ||
    'tool_name' in record ||
    'toolUseId' in record ||
    'tool_use_id' in record
      ? record
      : null;

  const metadataRecord = asRecord(record.metadata);
  const toolUseId =
    message.toolUseId ??
    asString(record.toolUseId) ??
    asString(record.tool_use_id) ??
    null;
  const toolName =
    message.toolName ??
    asString(record.toolName) ??
    asString(record.tool_name) ??
    null;
  const metadata = message.metadata ?? metadataRecord;
  return {
    role,
    wrapper,
    payload,
    toolUseId,
    toolName,
    metadata,
    isError:
      message.isError ??
      asBoolean(record.isError) ??
      asBoolean(record.is_error) ??
      false,
  };
}

function parseClaudeTrace(envelope: ToolTraceEnvelope): ParsedToolTrace | null {
  const record = asRecord(envelope.payload);
  if (!record) {
    return null;
  }

  const source = asString(envelope.metadata?.source);
  const looksLikeToolUse =
    envelope.role === 'tool_use' &&
    typeof record.tool === 'string' &&
    !('type' in record);
  const looksLikeToolResult =
    envelope.role === 'tool_result' &&
    !('type' in record) &&
    ('result' in record || 'text' in record);

  if (!looksLikeToolUse && !looksLikeToolResult && source !== 'claude_sdk') {
    return null;
  }

  const toolName = asString(record.tool) || envelope.toolName || 'Tool';
  const resultValue =
    record.result ??
    record.text ??
    envelope.payload;
  const resultText = stringifyUnknown(resultValue);

  return {
    role: envelope.role,
    provider: source || 'claude_sdk',
    toolUseId: envelope.toolUseId,
    toolKey: toolName,
    toolName,
    payload: record,
    input: envelope.role === 'tool_use' ? asRecord(record.input) || record : undefined,
    result: envelope.role === 'tool_result' ? resultValue : undefined,
    metadata: envelope.metadata,
    isError:
      envelope.isError ||
      asBoolean(record.is_error) ||
      looksLikeErrorText(resultText) ||
      false,
  };
}

function parseCodexTrace(envelope: ToolTraceEnvelope): ParsedToolTrace | null {
  const record = asRecord(envelope.payload);
  if (!record) {
    return null;
  }

  const itemType = canonicalCodexItemType(
    asString(record.type) ||
      asString(envelope.metadata?.item_type) ||
      asString(envelope.metadata?.itemType) ||
      envelope.toolName ||
      undefined,
  );
  if (!itemType) {
    return null;
  }

  const status = asString(record.status)?.toLowerCase();
  const exitCode = asNumber(record.exitCode);
  const resultValue =
    itemType === 'commandExecution'
      ? record.output ?? envelope.payload
      : itemType === 'mcpToolCall'
        ? record.result ?? envelope.payload
        : itemType === 'dynamicToolCall' || itemType === 'collabAgentToolCall'
          ? record.result ?? record.output ?? envelope.payload
          : itemType === 'imageGeneration'
            ? record.result ? '[generated image]' : envelope.payload
            : envelope.payload;

  return {
    role: envelope.role,
    provider: asString(envelope.metadata?.source) || 'codex_app_server',
    toolUseId: envelope.toolUseId,
    toolKey: itemType,
    toolName: envelope.toolName || itemType,
    payload: record,
    input: envelope.role === 'tool_use' ? record : undefined,
    result: envelope.role === 'tool_result' ? resultValue : undefined,
    metadata: envelope.metadata,
    isError:
      envelope.isError ||
      status === 'failed' ||
      status === 'declined' ||
      (itemType === 'commandExecution' && exitCode !== undefined && exitCode !== 0),
  };
}

const TOOL_TRACE_PARSERS: ToolTraceParser[] = [
  {
    id: 'claude_sdk',
    parse: parseClaudeTrace,
  },
  {
    id: 'codex_app_server',
    parse: parseCodexTrace,
  },
];

function parseToolTraceMessage(message: ToolTraceMessage): ParsedToolTrace {
  const envelope = buildToolTraceEnvelope(message);
  for (const parser of TOOL_TRACE_PARSERS) {
    const parsed = parser.parse(envelope);
    if (parsed) {
      return parsed;
    }
  }

  const fallbackDetail = stringifyUnknown(envelope.payload);
  const fallbackTitle =
    envelope.toolName ||
    (envelope.role === 'tool_result' ? 'Tool result' : 'Tool');

  return {
    role: envelope.role,
    provider: asString(envelope.metadata?.source) || 'unknown',
    toolUseId: envelope.toolUseId,
    toolKey: fallbackTitle,
    toolName: fallbackTitle,
    payload: envelope.payload,
    input: envelope.role === 'tool_use' ? asRecord(envelope.payload) : undefined,
    result: envelope.role === 'tool_result' ? envelope.payload : undefined,
    metadata: envelope.metadata,
    isError:
      envelope.isError ||
      (envelope.role === 'tool_result' && looksLikeErrorText(fallbackDetail)),
  };
}

function isCodexProvider(value: string | undefined): boolean {
  const normalized = value?.trim().toLowerCase();
  return normalized === 'codex' || normalized === 'codex_app_server';
}

export function shouldRenderToolTraceMessage(message: ToolTraceMessage): boolean {
  const parsed = parseToolTraceMessage(message);
  return !(isCodexProvider(parsed.provider) && parsed.toolKey === 'reasoning');
}

function exactToolAdapter(toolKeys: string[], presenter: (trace: ParsedToolTrace) => ToolTraceSide): ToolTraceAdapter {
  return {
    id: toolKeys.join(','),
    matches: (trace) => toolKeys.includes(trace.toolKey),
    present: presenter,
  };
}

function presentClaudeBash(trace: ParsedToolTrace): ToolTraceSide {
  const input = trace.input || asRecord(trace.payload);
  const payload = asRecord(trace.payload);
  const command = describeShellCommand(asString(input?.command) || asString(payload?.command));

  let summary: string | undefined;
  let detail: string | undefined;
  let detailLabel: string | undefined;
  if (trace.role === 'tool_use') {
    summary = truncateSingleLine(command);
    detail = formatCommandInvocation(input || payload);
    detailLabel = 'Call';
  }

  return {
    title: 'Command',
    summary,
    detail,
    detailLabel,
    icon: '⌘',
  };
}

function presentClaudeRead(trace: ParsedToolTrace): ToolTraceSide {
  const input = trace.input || asRecord(trace.payload);
  const path = extractPath(input);
  const windowLabel = describeLineWindow(input);

  let detail: string | undefined;
  let detailLabel: string | undefined;
  if (trace.role === 'tool_use') {
    detail = formatReadInvocation(input);
    detailLabel = 'Call';
  }

  return {
    title: windowLabel ? `Read ${windowLabel}` : 'Read',
    badges: dedupeBadges([pathTail(path) || path]),
    detail,
    detailLabel,
    icon: '≡',
  };
}

function presentClaudeWrite(trace: ParsedToolTrace): ToolTraceSide {
  const input = trace.input || asRecord(trace.payload);
  const path = extractPath(input);
  const content = asString(input?.content);
  return {
    title: 'Write',
    badges: dedupeBadges([pathTail(path) || path]),
    detail: trace.role === 'tool_use' && content ? truncateSingleLine(firstMeaningfulLine(content), 200) : undefined,
    detailLabel: trace.role === 'tool_use' && content ? 'Content' : undefined,
    icon: '✎',
  };
}

function splitDiffLines(value: string | undefined): string[] {
  if (!value) {
    return [];
  }
  const normalized = value.replace(/\r\n?/g, '\n');
  if (!normalized) {
    return [];
  }
  const lines = normalized.split('\n');
  if (lines[lines.length - 1] === '') {
    lines.pop();
  }
  return lines;
}

function countChangedLines(oldValue: string | undefined, newValue: string | undefined): DiffStats | undefined {
  const oldLines = splitDiffLines(oldValue);
  const newLines = splitDiffLines(newValue);
  if (!oldLines.length && !newLines.length) {
    return undefined;
  }

  const lcs = Array.from({ length: oldLines.length + 1 }, () =>
    Array<number>(newLines.length + 1).fill(0));

  for (let oldIndex = oldLines.length - 1; oldIndex >= 0; oldIndex -= 1) {
    for (let newIndex = newLines.length - 1; newIndex >= 0; newIndex -= 1) {
      lcs[oldIndex][newIndex] = oldLines[oldIndex] === newLines[newIndex]
        ? lcs[oldIndex + 1][newIndex + 1] + 1
        : Math.max(lcs[oldIndex + 1][newIndex], lcs[oldIndex][newIndex + 1]);
    }
  }

  const unchangedLineCount = lcs[0][0];
  return {
    added: newLines.length - unchangedLineCount,
    removed: oldLines.length - unchangedLineCount,
  };
}

function presentClaudeEdit(trace: ParsedToolTrace): ToolTraceSide {
  const input = trace.input || asRecord(trace.payload);
  const path = extractPath(input);
  const oldStr = asString(input?.old_string);
  const newStr = asString(input?.new_string);

  let detail: string | undefined;
  let detailLabel: string | undefined;
  let diffStats: DiffStats | undefined;

  if (trace.role === 'tool_use') {
    diffStats = countChangedLines(oldStr, newStr);
    if (oldStr || newStr) {
      const parts: string[] = [];
      if (oldStr) parts.push(`- ${truncateSingleLine(firstMeaningfulLine(oldStr), 120)}`);
      if (newStr) parts.push(`+ ${truncateSingleLine(firstMeaningfulLine(newStr), 120)}`);
      detail = parts.join('\n');
      detailLabel = 'Diff';
    }
  }

  return {
    title: 'Edit',
    badges: dedupeBadges([pathTail(path) || path]),
    diffStats,
    detail,
    detailLabel,
    icon: '✎',
  };
}

function presentClaudeGrep(trace: ParsedToolTrace): ToolTraceSide {
  const input = trace.input || asRecord(trace.payload);
  const path = extractPath(input);
  const pattern = asString(input?.pattern) || asString(input?.query);

  let detail: string | undefined;
  let detailLabel: string | undefined;
  if (trace.role === 'tool_use') {
    const parts: Record<string, unknown> = {};
    if (path) parts.path = pathTail(path) || path;
    const glob = asString(input?.glob);
    if (glob) parts.glob = glob;
    const type = asString(input?.type);
    if (type) parts.type = type;
    const rendered = prettyPrintRecord(parts as Record<string, string>, Object.keys(parts));
    if (rendered) {
      detail = rendered;
      detailLabel = 'Filter';
    }
  }

  return {
    title: 'Grep',
    summary: truncateSingleLine(pattern),
    detail,
    detailLabel,
    icon: '⌕',
  };
}

function presentClaudeGlob(trace: ParsedToolTrace): ToolTraceSide {
  const input = trace.input || asRecord(trace.payload);
  const path = extractPath(input);
  const pattern = asString(input?.pattern) || asString(input?.glob);
  return {
    title: 'Glob',
    summary: truncateSingleLine(pattern),
    badges: dedupeBadges([pathTail(path) || path]),
    icon: '◌',
  };
}

function presentClaudeWebFetch(trace: ParsedToolTrace): ToolTraceSide {
  const input = trace.input || asRecord(trace.payload);
  const url = asString(input?.url);
  const parsedUrl = parseUrl(url);
  return {
    title: 'Fetch',
    summary: truncateMiddleSingleLine(url, 136),
    badges: dedupeBadges([parsedUrl?.host]),
    icon: '↗',
  };
}

function presentClaudeWebSearch(trace: ParsedToolTrace): ToolTraceSide {
  const input = trace.input || asRecord(trace.payload);
  const query = asString(input?.query) || asString(input?.search_term);
  return {
    title: 'Search',
    summary: truncateSingleLine(query),
    icon: '⌕',
  };
}

function presentClaudeTask(trace: ParsedToolTrace): ToolTraceSide {
  const input = trace.input || asRecord(trace.payload);
  const description =
    asString(input?.description) ||
    asString(input?.prompt) ||
    asString(input?.task);
  const worker = asString(input?.subagent_type) || asString(input?.agent);
  return {
    title: 'Agent',
    summary: truncateSingleLine(description),
    badges: dedupeBadges([worker]),
    icon: '◇',
  };
}

function presentClaudeTodoWrite(trace: ParsedToolTrace): ToolTraceSide {
  const input = trace.input || asRecord(trace.payload);
  return {
    title: 'Todo',
    summary: truncateSingleLine(describeTodos(input), 120),
    icon: '☑',
  };
}

function presentClaudePlanMode(trace: ParsedToolTrace): ToolTraceSide {
  return {
    title: 'Plan mode',
    summary: trace.toolKey === 'EnterPlanMode' ? 'Entered' : 'Exited',
    icon: '▤',
  };
}

function presentClaudeQuestion(trace: ParsedToolTrace): ToolTraceSide {
  const input = trace.input || asRecord(trace.payload);
  const question =
    asString(input?.question) ||
    asString(input?.prompt) ||
    asString(input?.description);
  return {
    title: 'Question',
    summary: truncateSingleLine(question),
    icon: '?',
  };
}

function presentCodexCommand(trace: ParsedToolTrace): ToolTraceSide {
  const payload = asRecord(trace.payload);
  const command = describeShellCommand(asString(payload?.command));

  let summary: string | undefined;
  let detail: string | undefined;
  let detailLabel: string | undefined;
  if (trace.role === 'tool_use') {
    summary = truncateSingleLine(command, 132);
    detail = formatCommandInvocation(payload);
    detailLabel = 'Call';
  }

  return {
    title: 'Command',
    summary,
    status: commandStatus(payload),
    detail,
    detailLabel,
    icon: '⌘',
  };
}

function presentCodexFileChange(trace: ParsedToolTrace): ToolTraceSide {
  const payload = asRecord(trace.payload);
  const path = extractPath(payload);
  const operation = asString(payload?.operation);
  return {
    title: humanizeOperation(operation),
    badges: dedupeBadges([pathTail(path) || path]),
    detail: stringifyUnknown(trace.role === 'tool_use' ? payload : trace.result ?? payload),
    detailLabel: trace.role === 'tool_use' ? 'Change' : 'Result',
    icon: '✎',
  };
}

function presentCodexMcp(trace: ParsedToolTrace): ToolTraceSide {
  const payload = asRecord(trace.payload);
  const server = asString(payload?.server);
  const tool = asString(payload?.tool);
  const argumentsRecord = asRecord(payload?.arguments);
  const path = extractPath(argumentsRecord);
  return {
    title: humanizeToolLabel(tool),
    summary: truncateMiddleSingleLine(pathTail(path) || path || tool, 124),
    badges: dedupeBadges([
      server,
    ]),
    status: toolStatusFromState(asString(payload?.status)),
    detail: stringifyUnknown(
      trace.role === 'tool_use'
        ? payload?.arguments ?? payload
        : trace.result ?? payload?.result ?? payload,
    ),
    detailLabel: trace.role === 'tool_use' ? 'Call' : 'Response',
    icon: '⊚',
  };
}

function presentCodexDynamicTool(trace: ParsedToolTrace): ToolTraceSide {
  const payload = asRecord(trace.payload);
  const namespace = asString(payload?.namespace);
  const tool = asString(payload?.tool) || trace.toolName;
  const input = payload?.arguments ?? payload?.input ?? payload?.params ?? payload;
  const result = trace.result ?? payload?.result ?? payload?.output ?? payload;
  return {
    title: humanizeToolLabel(tool),
    summary: truncateSingleLine(
      summarizeTextPayload(input, 128) || asString(payload?.description) || tool,
      128,
    ),
    badges: dedupeBadges([namespace]),
    status: toolStatusFromState(asString(payload?.status)),
    detail: stringifyUnknown(trace.role === 'tool_use' ? input : result),
    detailLabel: trace.role === 'tool_use' ? 'Call' : 'Response',
    icon: '⊚',
  };
}

function presentCodexPlan(trace: ParsedToolTrace): ToolTraceSide {
  const payload = asRecord(trace.payload);
  const text = asString(payload?.text) || asString(payload?.plan);
  return {
    title: 'Plan',
    summary: truncateSingleLine(firstMeaningfulLine(text), 132),
    detail: text || stringifyUnknown(payload ?? trace.payload),
    detailLabel: 'Plan',
    icon: '▤',
  };
}

function presentCodexReasoning(trace: ParsedToolTrace): ToolTraceSide {
  const payload = asRecord(trace.payload);
  const summaryValue = payload?.summary;
  const summary = Array.isArray(summaryValue)
    ? summaryValue
        .map((entry) => (typeof entry === 'string' ? entry : summarizeTextPayload(entry, 80)))
        .filter(Boolean)
        .join(' · ')
    : summarizeTextPayload(summaryValue, 132);
  const detail =
    Array.isArray(summaryValue)
      ? summaryValue.map((entry) => stringifyUnknown(entry)).join('\n')
      : stringifyUnknown(payload?.content ?? summaryValue ?? payload ?? trace.payload);
  return {
    title: 'Reasoning',
    summary: truncateSingleLine(summary, 132),
    detail: truncateDetail(detail),
    detailLabel: 'Summary',
    icon: '·',
  };
}

function presentCodexImageGeneration(trace: ParsedToolTrace): ToolTraceSide {
  const payload = asRecord(trace.payload);
  const prompt =
    asString(payload?.prompt) ||
    asString(payload?.revisedPrompt) ||
    asString(payload?.revised_prompt);
  const status = toolStatusFromState(asString(payload?.status));
  const resultReady =
    trace.role === 'tool_result' &&
    typeof payload?.result === 'string' &&
    payload.result.trim().length > 0;
  return {
    title: 'Image generation',
    summary: resultReady ? 'Image ready' : truncateSingleLine(prompt, 132),
    status,
    detail:
      trace.role === 'tool_use'
        ? prettyPrintRecord(payload || {}, ['prompt', 'size', 'aspect_ratio', 'image_size'])
        : resultReady
          ? 'Image result is shown below.'
          : truncateDetail(stringifyUnknown(payload ?? trace.payload)),
    detailLabel: trace.role === 'tool_use' ? 'Prompt' : 'Result',
    icon: '◌',
  };
}

function presentCodexImageView(trace: ParsedToolTrace): ToolTraceSide {
  const payload = asRecord(trace.payload);
  const path = extractPath(payload);
  return {
    title: 'Image view',
    summary: truncateMiddleSingleLine(pathTail(path) || path, 124),
    badges: dedupeBadges([asString(payload?.media_type) || asString(payload?.mediaType)]),
    detail: stringifyUnknown(payload ?? trace.payload),
    detailLabel: 'Image',
    icon: '◌',
  };
}

function presentCodexSearch(trace: ParsedToolTrace): ToolTraceSide {
  const payload = asRecord(trace.payload);
  const query = asString(payload?.query) || asString(payload?.search);
  return {
    title: 'Search',
    summary: truncateSingleLine(query, 132),
    status: toolStatusFromState(asString(payload?.status)),
    detail: stringifyUnknown(trace.role === 'tool_result' ? trace.result ?? payload : payload),
    detailLabel: trace.role === 'tool_use' ? 'Query' : 'Result',
    icon: '⌕',
  };
}

function presentCodexMode(trace: ParsedToolTrace): ToolTraceSide {
  return {
    title:
      trace.toolKey === 'enteredReviewMode'
        ? 'Entered review mode'
        : 'Exited review mode',
    detail: stringifyUnknown(trace.payload),
    detailLabel: 'Event',
    icon: '▤',
  };
}

function presentCodexActivity(trace: ParsedToolTrace): ToolTraceSide {
  const payload = asRecord(trace.payload);
  return {
    title: humanizeToolLabel(trace.toolKey),
    summary: summarizeTextPayload(trace.role === 'tool_use' ? payload ?? trace.payload : trace.result ?? trace.payload),
    status: toolStatusFromState(asString(payload?.status)),
    detail: truncateDetail(stringifyUnknown(trace.role === 'tool_use' ? payload ?? trace.payload : trace.result ?? trace.payload)),
    detailLabel: trace.role === 'tool_use' ? 'Event' : 'Result',
    icon: '·',
  };
}

function presentFallback(trace: ParsedToolTrace): ToolTraceSide {
  const payload = asRecord(trace.payload);
  const path = extractPath(trace.input || payload);
  const detail = stringifyUnknown(trace.role === 'tool_use' ? trace.input ?? trace.payload : trace.result ?? trace.payload);
  return {
    title: humanizeToolLabel(trace.toolName),
    summary: summarizeTextPayload(trace.role === 'tool_use' ? trace.input ?? trace.payload : trace.result ?? trace.payload),
    badges: dedupeBadges([pathTail(path) || path]),
    detail,
    detailLabel: trace.role === 'tool_use' ? 'Call' : 'Result',
    icon: '·',
  };
}

const TOOL_TRACE_ADAPTERS: ToolTraceAdapter[] = [
  exactToolAdapter(['Bash'], presentClaudeBash),
  exactToolAdapter(['Read'], presentClaudeRead),
  exactToolAdapter(['Write'], presentClaudeWrite),
  exactToolAdapter(['Edit'], presentClaudeEdit),
  exactToolAdapter(['Grep'], presentClaudeGrep),
  exactToolAdapter(['Glob'], presentClaudeGlob),
  exactToolAdapter(['WebFetch'], presentClaudeWebFetch),
  exactToolAdapter(['WebSearch'], presentClaudeWebSearch),
  exactToolAdapter(['Task', 'Agent'], presentClaudeTask),
  exactToolAdapter(['TodoWrite'], presentClaudeTodoWrite),
  exactToolAdapter(['EnterPlanMode', 'ExitPlanMode'], presentClaudePlanMode),
  exactToolAdapter(['AskUserQuestion'], presentClaudeQuestion),
  exactToolAdapter(['commandExecution'], presentCodexCommand),
  exactToolAdapter(['fileChange'], presentCodexFileChange),
  exactToolAdapter(['mcpToolCall'], presentCodexMcp),
  exactToolAdapter(['dynamicToolCall', 'collabAgentToolCall'], presentCodexDynamicTool),
  exactToolAdapter(['plan'], presentCodexPlan),
  exactToolAdapter(['reasoning'], presentCodexReasoning),
  exactToolAdapter(['imageGeneration'], presentCodexImageGeneration),
  exactToolAdapter(['imageView'], presentCodexImageView),
  exactToolAdapter(['webSearch'], presentCodexSearch),
  exactToolAdapter(['enteredReviewMode', 'exitedReviewMode'], presentCodexMode),
  exactToolAdapter(['hookPrompt', 'contextCompaction'], presentCodexActivity),
];

function resolveToolTraceSide(trace: ParsedToolTrace): ToolTraceSide {
  for (const adapter of TOOL_TRACE_ADAPTERS) {
    if (adapter.matches(trace)) {
      return adapter.present(trace);
    }
  }
  return presentFallback(trace);
}

export function resolveMergedToolTrace(
  toolUse?: ToolTraceMessage,
  toolResult?: ToolTraceMessage,
): MergedToolTrace {
  const parsedUse = toolUse ? parseToolTraceMessage(toolUse) : null;
  const parsedResult = toolResult ? parseToolTraceMessage(toolResult) : null;
  const useSide = parsedUse ? resolveToolTraceSide(parsedUse) : null;
  const resultSide = parsedResult ? resolveToolTraceSide(parsedResult) : null;
  const resolvedStatus = parsedResult ? resultSide?.status : resultSide?.status || useSide?.status;

  return {
    title: useSide?.title || resultSide?.title || 'Tool',
    summary: useSide?.summary,
    resultSummary: undefined,
    badges: dedupeBadges([...(useSide?.badges || []), ...(resultSide?.badges || [])]),
    diffStats: useSide?.diffStats || resultSide?.diffStats,
    status: resolvedStatus,
    inputDetail: useSide?.detail,
    inputLabel: useSide?.detailLabel,
    resultDetail: resultSide?.detail || (parsedResult?.result ? truncateDetail(stringifyUnknown(parsedResult.result)) : undefined),
    resultLabel: resultSide?.detailLabel || (resultSide?.detail || parsedResult?.result ? 'Result' : undefined),
    icon: useSide?.icon || resultSide?.icon || '·',
    isError: Boolean(parsedUse?.isError || parsedResult?.isError),
  };
}

export function canMergeToolTraceMessages(
  toolUseMessage: ToolTraceMessage,
  toolResultMessage: ToolTraceMessage,
): boolean {
  if (toolUseMessage.role !== 'tool_use' || toolResultMessage.role !== 'tool_result') {
    return false;
  }

  const parsedUse = parseToolTraceMessage(toolUseMessage);
  const parsedResult = parseToolTraceMessage(toolResultMessage);

  if (parsedUse.toolUseId && parsedResult.toolUseId) {
    return parsedUse.toolUseId === parsedResult.toolUseId;
  }

  if (parsedUse.provider !== parsedResult.provider) {
    return false;
  }

  if (parsedUse.toolKey === parsedResult.toolKey) {
    return true;
  }

  return (
    parsedUse.toolKey === 'Tool' ||
    parsedUse.toolKey === 'Tool result' ||
    parsedResult.toolKey === 'Tool' ||
    parsedResult.toolKey === 'Tool result'
  );
}
