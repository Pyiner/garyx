import {
  DEFAULT_DESKTOP_SETTINGS,
  type ConnectionStatus,
  type DesktopChatStreamEvent,
} from '@shared/contracts';

import { stringifyJsonBlock } from '../gateway-settings';
import type { ClientLogEntry, GatewayIndicatorTone, ThreadLogLine } from './types';

const THREAD_LOG_TIMESTAMP_PATTERN = /^\d{4}-\d{2}-\d{2}T\S+\s+/;
export const MAX_CLIENT_STREAM_LOG_ENTRIES = 500;
export const THREAD_LOG_PANEL_MIN_WIDTH = 280;
export const THREAD_LOG_PANEL_MAX_WIDTH = 760;
const THREAD_LOG_PANEL_MIN_MAIN_WIDTH = 540;
const THREAD_LOG_PANEL_RESIZER_WIDTH = 10;
const GATEWAY_OFFLINE_THRESHOLD = 3;

export function buildThreadLogLines(rawText: string): ThreadLogLine[] {
  return rawText.split(/\r?\n/).map((line, index) => {
    const timestampMatch = line.match(THREAD_LOG_TIMESTAMP_PATTERN);
    const rawTimestamp = timestampMatch?.[0]?.trim() || '';
    const text = line.replace(THREAD_LOG_TIMESTAMP_PATTERN, '');
    const level = /\bERROR\b/.test(text) ? 'error' : 'default';
    return {
      key: `thread-log-line-${index}`,
      timestamp: formatThreadLogTimestamp(rawTimestamp),
      text,
      level,
    };
  });
}

function formatThreadLogTimestamp(value: string): string | undefined {
  const trimmed = value.trim();
  if (!trimmed) {
    return undefined;
  }
  const date = new Date(trimmed);
  if (Number.isNaN(date.getTime())) {
    return trimmed;
  }

  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  const hours = String(date.getHours()).padStart(2, '0');
  const minutes = String(date.getMinutes()).padStart(2, '0');
  const seconds = String(date.getSeconds()).padStart(2, '0');
  const timezone = Intl.DateTimeFormat(undefined, { timeZoneName: 'short' })
    .formatToParts(date)
    .find((part) => part.type === 'timeZoneName')
    ?.value;
  return `${year}-${month}-${day} ${hours}:${minutes}:${seconds}${timezone ? ` ${timezone}` : ''}`;
}

function formatThreadLogClock(value: number): string {
  const date = new Date(value);
  const hours = String(date.getHours()).padStart(2, '0');
  const minutes = String(date.getMinutes()).padStart(2, '0');
  const seconds = String(date.getSeconds()).padStart(2, '0');
  const milliseconds = String(date.getMilliseconds()).padStart(3, '0');
  return `${hours}:${minutes}:${seconds}.${milliseconds}`;
}

function compactClientLogText(value: string): string {
  return value.replace(/\s+/g, ' ').trim();
}

function truncateClientLogText(value: string, maxChars = 160): string {
  if (value.length <= maxChars) {
    return value;
  }
  return `${value.slice(0, Math.max(0, maxChars - 1))}…`;
}

function shortenClientLogId(value: string | null | undefined): string {
  const text = String(value || '').trim();
  if (!text) {
    return '';
  }
  return text.length <= 18 ? text : `${text.slice(0, 8)}…${text.slice(-6)}`;
}

function stringifyClientLogPayload(value: unknown, maxChars = 4_000): string {
  const text = typeof value === 'string' ? value : stringifyJsonBlock(value);
  if (text.length <= maxChars) {
    return text;
  }
  return `${text.slice(0, maxChars)}\n…[truncated ${text.length - maxChars} chars]`;
}

function summarizeClientStreamEvent(event: DesktopChatStreamEvent): string {
  switch (event.type) {
    case 'assistant_delta':
      return `len=${event.delta.length} ${truncateClientLogText(compactClientLogText(JSON.stringify(event.delta)))}`;
    case 'tool_use':
    case 'tool_result': {
      const summaryParts = [
        event.message.toolName || 'tool',
        event.message.toolUseId ? `#${shortenClientLogId(event.message.toolUseId)}` : '',
        event.message.isError ? 'error' : '',
      ].filter(Boolean);
      return summaryParts.join(' · ');
    }
    case 'error':
      return truncateClientLogText(compactClientLogText(event.error), 220);
    case 'accepted':
    case 'assistant_boundary':
    case 'done':
    case 'user_ack':
      return `run=${shortenClientLogId(event.runId)}`;
    default:
      return '';
  }
}

function buildClientStreamLogDetail(event: DesktopChatStreamEvent): string {
  switch (event.type) {
    case 'assistant_delta':
      return stringifyClientLogPayload({
        type: event.type,
        runId: event.runId,
        delta: event.delta,
      });
    case 'tool_use':
    case 'tool_result':
      return stringifyClientLogPayload({
        type: event.type,
        runId: event.runId,
        message: event.message,
      });
    case 'error':
      return stringifyClientLogPayload({
        type: event.type,
        runId: event.runId,
        error: event.error,
      });
    default:
      return stringifyClientLogPayload(event);
  }
}

export function buildClientStreamLogEntry(
  event: DesktopChatStreamEvent,
  key: string,
): ClientLogEntry {
  const now = Date.now();
  return {
    key,
    timestamp: formatThreadLogClock(now),
    eventType: event.type,
    summary: summarizeClientStreamEvent(event),
    detail: buildClientStreamLogDetail(event),
    level: event.type === 'error' ? 'error' : 'default',
  };
}

export function clampThreadLogsPanelWidth(
  width: number,
  layoutWidth?: number | null,
): number {
  const baseWidth = Number.isFinite(width) ? width : DEFAULT_DESKTOP_SETTINGS.threadLogsPanelWidth;
  const layoutMax = layoutWidth && layoutWidth > 0
    ? Math.max(
        THREAD_LOG_PANEL_MIN_WIDTH,
        layoutWidth - THREAD_LOG_PANEL_MIN_MAIN_WIDTH - THREAD_LOG_PANEL_RESIZER_WIDTH,
      )
    : THREAD_LOG_PANEL_MAX_WIDTH;
  return Math.max(
    THREAD_LOG_PANEL_MIN_WIDTH,
    Math.min(THREAD_LOG_PANEL_MAX_WIDTH, Math.min(layoutMax, Math.round(baseWidth))),
  );
}

export function computeGatewayIndicator(input: {
  status: ConnectionStatus | null;
  failureCount: number;
  recovering: boolean;
  reason?: string | null;
}): { tone: GatewayIndicatorTone; label: string } | null {
  if (input.status?.ok) {
    return null;
  }

  if (input.recovering || input.failureCount < GATEWAY_OFFLINE_THRESHOLD) {
    return {
      tone: 'syncing',
      label: input.reason || 'Reconnecting to gateway…',
    };
  }

  return {
    tone: 'offline',
    label: input.reason || 'Gateway offline',
  };
}
