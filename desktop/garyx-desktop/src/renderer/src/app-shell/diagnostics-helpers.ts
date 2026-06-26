import type { ConnectionStatus } from '@shared/contracts';

import type { GatewayIndicatorTone, ThreadLogLine } from './types';

const THREAD_LOG_TIMESTAMP_PATTERN = /^\d{4}-\d{2}-\d{2}T\S+\s+/;
export const MAX_GATEWAY_THREAD_LOG_LINES = 100;
export const THREAD_LOG_PANEL_MIN_WIDTH = 280;
export const THREAD_LOG_PANEL_MAX_WIDTH = 760;
const THREAD_LOG_PANEL_MIN_MAIN_WIDTH = 540;
const THREAD_LOG_PANEL_RESIZER_WIDTH = 10;
const DEFAULT_THREAD_LOG_PANEL_WIDTH = 360;
export const SIDE_TOOLS_PANEL_MIN_WIDTH = 520;
export const SIDE_TOOLS_PANEL_MAX_WIDTH = 1180;
export const SIDE_TOOLS_PANEL_DEFAULT_RATIO = 0.6;
const SIDE_TOOLS_PANEL_MIN_MAIN_WIDTH = 540;
const SIDE_TOOLS_PANEL_RESIZER_WIDTH = 10;
const DEFAULT_SIDE_TOOLS_PANEL_WIDTH = 720;
const GATEWAY_OFFLINE_THRESHOLD = 3;

export function keepRecentThreadLogLines(
  rawText: string,
  maxLines = MAX_GATEWAY_THREAD_LOG_LINES,
): string {
  if (maxLines <= 0 || !rawText) {
    return '';
  }

  const hasTrailingNewline = /\r?\n$/.test(rawText);
  const lines = rawText.split(/\r?\n/);
  const logLines = hasTrailingNewline ? lines.slice(0, -1) : lines;
  const tail = logLines.slice(-maxLines).join('\n');
  return hasTrailingNewline && tail ? `${tail}\n` : tail;
}

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

export function clampThreadLogsPanelWidth(
  width: number,
  layoutWidth?: number | null,
): number {
  const baseWidth = Number.isFinite(width) ? width : DEFAULT_THREAD_LOG_PANEL_WIDTH;
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

export function clampSideToolsPanelWidth(
  width: number,
  layoutWidth?: number | null,
): number {
  const baseWidth = Number.isFinite(width)
    ? width
    : DEFAULT_SIDE_TOOLS_PANEL_WIDTH;
  const layoutMax = layoutWidth && layoutWidth > 0
    ? Math.max(
        SIDE_TOOLS_PANEL_MIN_WIDTH,
        layoutWidth -
          SIDE_TOOLS_PANEL_MIN_MAIN_WIDTH -
          SIDE_TOOLS_PANEL_RESIZER_WIDTH,
      )
    : SIDE_TOOLS_PANEL_MAX_WIDTH;
  return Math.max(
    SIDE_TOOLS_PANEL_MIN_WIDTH,
    Math.min(
      SIDE_TOOLS_PANEL_MAX_WIDTH,
      Math.min(layoutMax, Math.round(baseWidth)),
    ),
  );
}

export function defaultSideToolsPanelWidth(layoutWidth?: number | null): number {
  const baseWidth = layoutWidth && layoutWidth > 0
    ? layoutWidth * SIDE_TOOLS_PANEL_DEFAULT_RATIO
    : DEFAULT_SIDE_TOOLS_PANEL_WIDTH;
  return clampSideToolsPanelWidth(baseWidth, layoutWidth);
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
