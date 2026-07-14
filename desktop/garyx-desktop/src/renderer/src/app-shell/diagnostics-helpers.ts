import type { ConnectionStatus } from '@shared/contracts';

import type { GatewayIndicatorTone, ThreadLogLine } from './types';
import {
  SIDE_PANEL_MIN_MAIN_WIDTH,
  SIDE_PANEL_RESIZER_WIDTH,
  horizontalLayoutPolicy,
  type LayoutPolicyName,
} from './responsive-layout-model.ts';

// Matches both the current gateway stamp (`2026-07-07 17:06:37.123`, local
// wall clock, space-separated) and older RFC3339 lines (`...T...+08:00`/`Z`).
const THREAD_LOG_TIMESTAMP_PATTERN = /^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}\S*\s+/;
export const MAX_GATEWAY_THREAD_LOG_LINES = 100;
export const SIDE_TOOLS_PANEL_MAX_WIDTH = 1180;
const GATEWAY_OFFLINE_THRESHOLD = 3;

/**
 * The side-tools default width IS the policy min width (one knob); the layout
 * model normalizes with the same value, so keep every UI clamp on this getter.
 */
export function sideToolsPanelMinWidth(policy: LayoutPolicyName): number {
  return horizontalLayoutPolicy(policy).sideToolsMinWidth;
}

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
  // Unified human-readable style: local wall-clock time, timezone implicit.
  return `${year}-${month}-${day} ${hours}:${minutes}:${seconds}`;
}

export function clampSideToolsPanelWidth(
  policy: LayoutPolicyName,
  width: number,
  layoutWidth?: number | null,
): number {
  const minWidth = sideToolsPanelMinWidth(policy);
  const baseWidth = Number.isFinite(width) ? width : minWidth;
  const layoutMax = layoutWidth && layoutWidth > 0
    ? Math.max(
        minWidth,
        layoutWidth -
          SIDE_PANEL_MIN_MAIN_WIDTH -
          SIDE_PANEL_RESIZER_WIDTH,
      )
    : SIDE_TOOLS_PANEL_MAX_WIDTH;
  return Math.max(
    minWidth,
    Math.min(
      SIDE_TOOLS_PANEL_MAX_WIDTH,
      Math.min(layoutMax, Math.round(baseWidth)),
    ),
  );
}

export function defaultSideToolsPanelWidth(
  policy: LayoutPolicyName,
  layoutWidth?: number | null,
): number {
  return clampSideToolsPanelWidth(
    policy,
    sideToolsPanelMinWidth(policy),
    layoutWidth,
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
