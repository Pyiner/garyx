import type {
  RenderToolDiffRecipe,
  RenderToolFieldProjection,
  RenderToolFieldSelector,
  RenderToolKind,
  RenderToolValueSelector,
  TranscriptMessage,
} from '@shared/contracts';

import {
  imageSourceFromUnknown,
  type TranscriptSegment,
} from './message-rich-content-core';

export type ToolResultImageSegment = Extract<TranscriptSegment, { kind: 'image' }>;

export type ToolPathImageRef = {
  key: string;
  path: string;
  alt: string;
};

export type ToolTraceMessage = Pick<
  TranscriptMessage,
  | 'role'
  | 'text'
  | 'content'
  | 'input'
  | 'result'
  | 'toolUseId'
  | 'toolName'
  | 'metadata'
  | 'isError'
>;

type ToolTraceStatusTone = 'neutral' | 'progress' | 'error';

type ToolTraceStatus = {
  label: string;
  tone: ToolTraceStatusTone;
};

type DiffStats = {
  added: number;
  removed: number;
};

export type ToolTraceDiffLine = {
  kind: 'added' | 'removed' | 'context';
  text: string;
};

export type MergedToolTrace = {
  title: string;
  summary?: string;
  badges: string[];
  diffStats?: DiffStats;
  status?: ToolTraceStatus;
  pathDetail?: string;
  pathLabel?: string;
  inputDetail?: string;
  inputLabel?: string;
  diffLines?: ToolTraceDiffLine[];
  resultDetail?: string;
  resultLabel?: string;
  /** Image blocks extracted from the tool result, rendered as thumbnails. */
  resultImages: ToolResultImageSegment[];
  /** Gateway-side image paths referenced by an image or image-file read. */
  pathImages: ToolPathImageRef[];
  icon: string;
  isError: boolean;
};

const IMAGE_PATH_EXTENSIONS = new Set([
  'png',
  'jpg',
  'jpeg',
  'gif',
  'webp',
  'heic',
  'heif',
  'bmp',
  'tiff',
  'svg',
]);

function isImagePath(path: string): boolean {
  const fileName = path.split(/[\\/]/).filter(Boolean).pop() || '';
  const separator = fileName.lastIndexOf('.');
  return separator >= 0
    && IMAGE_PATH_EXTENSIONS.has(fileName.slice(separator + 1).toLowerCase());
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
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

function firstMeaningfulLine(value: string | undefined): string | undefined {
  const normalized = value?.replace(/\r/g, '\n');
  if (!normalized) {
    return undefined;
  }
  return normalized
    .split('\n')
    .map((entry) => entry.trim())
    .find(Boolean);
}

function firstProjectedMeaningfulLine(value: string | undefined): string | undefined {
  // Tool output may be megabytes. The detail keeps the exact selected string,
  // while the one-line header only inspects a bounded prefix.
  return firstMeaningfulLine(value?.slice(0, 4096));
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

function truncateMiddleSingleLine(value: string | undefined, maxLength = 120): string | undefined {
  const singleLine = normalizeWhitespace(value);
  if (!singleLine || singleLine.length <= maxLength) {
    return singleLine;
  }
  const headLength = Math.ceil((maxLength - 3) / 2);
  const tailLength = Math.floor((maxLength - 3) / 2);
  return `${singleLine.slice(0, headLength)}...${singleLine.slice(singleLine.length - tailLength)}`;
}

function dedupeBadges(values: Array<string | undefined>): string[] {
  const seen = new Set<string>();
  const output: string[] = [];
  for (const value of values) {
    const normalized = truncateMiddleSingleLine(value, 44);
    if (!normalized || seen.has(normalized)) {
      continue;
    }
    seen.add(normalized);
    output.push(normalized);
  }
  return output;
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

function humanizeToolLabel(value: string | undefined): string {
  const label = value?.trim();
  if (!label) {
    return 'Tool';
  }
  const normalized = label.replace(/[_-]+/g, ' ');
  return normalized.charAt(0).toUpperCase() + normalized.slice(1);
}

/**
 * The single decision for "this record is an embedded image in a tool
 * result": an explicit image block, or an untyped record carrying inline
 * base64 (`source.data`). Typed non-image blocks and URL-only records are not
 * images here even though message bubbles render them leniently.
 */
function isToolResultImageRecord(record: Record<string, unknown>): boolean {
  const type = typeof record.type === 'string' ? record.type.trim().toLowerCase() : '';
  if (type === 'image') {
    return true;
  }
  if (type) {
    return false;
  }
  const source = asRecord(record.source);
  return Boolean(source && typeof source.data === 'string' && source.data.trim());
}

function collectToolResultImageSegments(value: unknown): ToolResultImageSegment[] {
  const segments: ToolResultImageSegment[] = [];
  const visit = (node: unknown, path: string) => {
    if (Array.isArray(node)) {
      node.forEach((entry, index) => visit(entry, `${path}:${index}`));
      return;
    }
    const record = asRecord(node);
    if (!record) {
      return;
    }
    if (isToolResultImageRecord(record)) {
      const src = imageSourceFromUnknown(record);
      if (src) {
        segments.push({
          kind: 'image',
          key: `${path}:image`,
          src,
          alt: 'Tool result image',
        });
      }
      return;
    }
    for (const key of ['result', 'content', 'output']) {
      if (key in record) {
        visit(record[key], `${path}:${key}`);
      }
    }
  };
  visit(value, 'tool-result');
  return segments;
}

function projectionRootValue(
  message: ToolTraceMessage | undefined,
  selector: RenderToolValueSelector | undefined,
): unknown {
  if (!message || !selector) {
    return undefined;
  }
  switch (selector.root) {
    case 'content':
      return message.content;
    case 'input':
      return message.input;
    case 'result':
      return message.result;
    case 'text':
      return message.text;
  }
}

function projectionPathValue(
  message: ToolTraceMessage | undefined,
  selector: RenderToolValueSelector | undefined,
): unknown {
  let value = projectionRootValue(message, selector);
  for (const component of selector?.path || []) {
    value = parseMaybeJson(value);
    if (Array.isArray(value)) {
      const index = Number(component);
      if (!Number.isInteger(index) || index < 0 || index >= value.length) {
        return undefined;
      }
      value = value[index];
      continue;
    }
    const record = asRecord(value);
    if (!record || !(component in record)) {
      return undefined;
    }
    value = record[component];
  }
  return value;
}

function unwrapProjectedString(value: string): string {
  let encoded = value;
  if (!value.startsWith('"') || !value.endsWith('"')) {
    // Some providers wrap a short JSON scalar in whitespace. Keep this bounded
    // so megabytes of stdout are never copied just to test for unwrapping.
    if (value.length > 16_384) {
      return value;
    }
    encoded = value.trim();
  }
  if (encoded.length >= 2 && encoded.startsWith('"') && encoded.endsWith('"')) {
    try {
      const parsed = JSON.parse(encoded);
      if (typeof parsed === 'string') {
        return parsed;
      }
    } catch {
      // Keep provider text verbatim when it only resembles a JSON string.
    }
  }
  return value;
}

function collectProjectedResultImages(
  toolResult: ToolTraceMessage | undefined,
  projection: RenderToolFieldProjection,
): ToolResultImageSegment[] {
  if (!toolResult || projection.result?.format !== 'image') {
    return [];
  }
  return collectToolResultImageSegments([
    parseMaybeJson(toolResult.result),
    parseMaybeJson(toolResult.content),
  ]);
}

function collectProjectedPathImages(
  toolUse: ToolTraceMessage | undefined,
  toolResult: ToolTraceMessage | undefined,
  projection: RenderToolFieldProjection,
): ToolPathImageRef[] {
  if (projection.kind !== 'image' && projection.kind !== 'file_read') {
    return [];
  }
  const candidates = [
    projection.call && ['image', 'path'].includes(projection.call.format)
      ? projectionPathValue(toolUse, projection.call)
      : undefined,
    projection.result && ['image', 'path'].includes(projection.result.format)
      ? projectionPathValue(toolResult, projection.result)
      : undefined,
  ];
  const seen = new Set<string>();
  const images: ToolPathImageRef[] = [];
  for (const candidate of candidates) {
    if (typeof candidate !== 'string' || candidate.length > 16_384) {
      continue;
    }
    const path = unwrapProjectedString(candidate).trim();
    if (
      !path
      || seen.has(path)
      || (projection.kind === 'file_read' && !isImagePath(path))
    ) {
      continue;
    }
    seen.add(path);
    images.push({
      key: `projected-image:${path}`,
      path,
      alt: path.split(/[\\/]/).filter(Boolean).pop() || 'Tool image',
    });
  }
  return images;
}

function projectionDisplayValue(
  message: ToolTraceMessage | undefined,
  selector: RenderToolFieldSelector | undefined,
): string | undefined {
  if (!selector) {
    return undefined;
  }
  switch (selector.format) {
    case 'image':
      return undefined;
    case 'text':
    case 'code':
    case 'path':
    case 'json':
      break;
    default:
      // Presentation vocabulary is intentionally lenient on desktop. An
      // unknown/retired scalar format degrades only this field, not the row.
      return undefined;
  }
  const value = projectionPathValue(message, selector);
  if (value === null || value === undefined) {
    return undefined;
  }
  if (typeof value === 'string') {
    const text = unwrapProjectedString(value);
    return text.length ? text : undefined;
  }
  const text = stringifyUnknown(value);
  return text.trim() ? text : undefined;
}

function projectionRawString(
  message: ToolTraceMessage | undefined,
  selector: RenderToolValueSelector | null | undefined,
): string | undefined {
  if (!selector) {
    return undefined;
  }
  const value = projectionPathValue(message, selector);
  return typeof value === 'string' ? value : undefined;
}

function rawLines(value: string | undefined): string[] {
  return value === undefined || value.length === 0 ? [] : value.split('\n');
}

function unifiedDiffLine(line: string): ToolTraceDiffLine {
  if (line.startsWith('+++') || line.startsWith('---')) {
    return { kind: 'context', text: line };
  }
  if (line.startsWith('+')) {
    return { kind: 'added', text: line.slice(1) };
  }
  if (line.startsWith('-')) {
    return { kind: 'removed', text: line.slice(1) };
  }
  return { kind: 'context', text: line };
}

function resolveProjectionDiff(
  recipe: RenderToolDiffRecipe | undefined,
  toolUse: ToolTraceMessage | undefined,
  toolResult: ToolTraceMessage | undefined,
): ToolTraceDiffLine[] | undefined {
  if (!recipe || !Array.isArray(recipe.segments)) {
    return undefined;
  }
  const source = recipe.source === 'tool_use'
    ? toolUse
    : recipe.source === 'tool_result'
      ? toolResult
      : undefined;
  if (!source) {
    return undefined;
  }
  const lines: ToolTraceDiffLine[] = [];
  for (const segment of recipe.segments) {
    const record = asRecord(segment);
    const unified = asRecord(record?.unified);
    if (unified) {
      for (const line of rawLines(projectionRawString(source, unified.text as RenderToolValueSelector))) {
        lines.push(unifiedDiffLine(line));
      }
      continue;
    }
    const pair = asRecord(record?.pair);
    if (!pair) {
      continue;
    }
    for (const line of rawLines(
      projectionRawString(source, pair.old as RenderToolValueSelector | null | undefined),
    )) {
      lines.push({ kind: 'removed', text: line });
    }
    for (const line of rawLines(
      projectionRawString(source, pair.new as RenderToolValueSelector | null | undefined),
    )) {
      lines.push({ kind: 'added', text: line });
    }
  }
  return lines.length ? lines : undefined;
}

function projectionDiffStats(
  lines: ToolTraceDiffLine[] | undefined,
): DiffStats | undefined {
  if (!lines) {
    return undefined;
  }
  const stats = lines.reduce(
    (current, line) => {
      if (line.kind === 'added') current.added += 1;
      if (line.kind === 'removed') current.removed += 1;
      return current;
    },
    { added: 0, removed: 0 },
  );
  return stats.added || stats.removed ? stats : undefined;
}

function projectionLabel(selector: RenderToolFieldSelector | undefined): string | undefined {
  switch (selector?.label) {
    case 'url':
      return 'URL';
    case 'call':
      return 'Call';
    case 'command':
      return 'Command';
    case 'file':
      return 'File';
    case 'query':
      return 'Query';
    case 'prompt':
      return 'Prompt';
    case 'parameters':
      return 'Parameters';
    case 'content':
      return 'Content';
    case 'output':
      return 'Output';
    case 'result':
      return 'Result';
    case 'response':
      return 'Response';
    case 'image':
      return 'Image';
    case 'error':
      return 'Error';
    default:
      return undefined;
  }
}

function projectionTitle(kind: RenderToolKind, toolName: string | undefined): string {
  switch (kind) {
    case 'command':
      return 'Command';
    case 'file_read':
      return 'Read';
    case 'file_write':
      return 'Write';
    case 'file_edit':
      return 'Edit';
    case 'search':
      return 'Search';
    case 'web':
      return 'Web';
    case 'agent':
      return 'Agent';
    case 'task':
      return 'Task';
    case 'image':
      return 'Image';
    case 'system':
      return 'Activity';
    case 'generic':
      return humanizeToolLabel(toolName);
  }
}

function projectionIcon(kind: RenderToolKind): string {
  switch (kind) {
    case 'command':
      return '⌘';
    case 'file_read':
      return '≡';
    case 'file_write':
    case 'file_edit':
      return '✎';
    case 'search':
      return '⌕';
    case 'web':
      return '↗';
    case 'agent':
      return '◇';
    case 'task':
      return '☑';
    case 'image':
      return '◌';
    case 'system':
    case 'generic':
      return '·';
  }
}

function projectionMetaBadges(projection: RenderToolFieldProjection): string[] {
  const badges: string[] = [];
  if (projection.exit_code !== undefined) {
    badges.push(`exit ${projection.exit_code}`);
  }
  if (projection.duration_ms !== undefined) {
    const duration = projection.duration_ms;
    badges.push(duration >= 1000 ? `${(duration / 1000).toFixed(1)} s` : `${duration} ms`);
  }
  return badges;
}

function projectionStatus(projection: RenderToolFieldProjection): ToolTraceStatus | undefined {
  if (projection.exit_code !== undefined && projection.exit_code !== 0) {
    return { label: `exit ${projection.exit_code}`, tone: 'error' };
  }
  return toolStatusFromState(projection.status);
}

function projectionIsError(projection: RenderToolFieldProjection): boolean {
  if (projection.exit_code !== undefined && projection.exit_code !== 0) {
    return true;
  }
  return ['failed', 'declined', 'errored', 'error', 'canceled', 'cancelled'].includes(
    projection.status?.trim().toLowerCase() || '',
  );
}

function genericToolFallback(toolUse?: ToolTraceMessage, toolResult?: ToolTraceMessage): MergedToolTrace {
  return {
    title: 'Tool',
    badges: [],
    resultImages: [],
    pathImages: [],
    icon: '·',
    isError: Boolean(toolUse?.isError || toolResult?.isError),
  };
}

export function resolveMergedToolTrace(
  toolUse?: ToolTraceMessage,
  toolResult?: ToolTraceMessage,
  projection?: RenderToolFieldProjection,
): MergedToolTrace {
  if (!projection) {
    return genericToolFallback(toolUse, toolResult);
  }

  // Completion may contribute call-side detail that was not known when the
  // paired tool use was committed, so resolve the selector across both bodies.
  const projectedCall =
    projectionDisplayValue(toolUse, projection.call) ??
    projectionDisplayValue(toolResult, projection.call);
  const projectedResult = projectionDisplayValue(toolResult, projection.result);
  const resolvedSummary =
    projectionDisplayValue(toolUse, projection.summary) ??
    projectionDisplayValue(toolResult, projection.summary);
  const projectedPath = projection.summary?.format === 'path'
    ? resolvedSummary
    : projection.call?.format === 'path'
      ? projectedCall
      : undefined;
  const projectedSummaryValue =
    (projection.summary?.format === 'path' ? undefined : resolvedSummary) ??
    (projection.call?.format === 'json' ? undefined : projectedCall);
  const projectedSummary = truncateSingleLine(
    firstProjectedMeaningfulLine(projectedSummaryValue),
    132,
  );
  const projectedPathBadge = pathTail(projectedPath) || projectedPath;
  const diffLines = resolveProjectionDiff(projection.diff, toolUse, toolResult);

  return {
    title: projectionTitle(projection.kind, projection.tool_name),
    summary: projectedSummary,
    badges: dedupeBadges([
      projectedPathBadge,
      ...projectionMetaBadges(projection),
    ]),
    status: projectionStatus(projection),
    pathDetail: projection.summary?.format === 'path' ? resolvedSummary : undefined,
    pathLabel: projection.summary?.format === 'path'
      ? projectionLabel(projection.summary)
      : undefined,
    inputDetail: projectedCall,
    inputLabel: projectionLabel(projection.call),
    diffLines,
    diffStats: projectionDiffStats(diffLines),
    resultDetail: projectedResult,
    resultLabel: projectionLabel(projection.result),
    resultImages: collectProjectedResultImages(toolResult, projection),
    pathImages: collectProjectedPathImages(toolUse, toolResult, projection),
    icon: projectionIcon(projection.kind),
    isError: Boolean(
      toolUse?.isError ||
      toolResult?.isError ||
      projectionIsError(projection)
    ),
  };
}
