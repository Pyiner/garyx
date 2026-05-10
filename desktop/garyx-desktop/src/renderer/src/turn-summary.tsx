import { type ReactNode, useEffect, useState } from 'react';

import { IconChevronDown } from '@tabler/icons-react';

import { useI18n } from './i18n';
import type { TurnRow } from './turn-render';

const ICON_SIZE = 15;
const ICON_STROKE = 1.7;

function formatElapsed(seconds: number): string {
  const safe = Math.max(0, Math.round(seconds));
  if (safe < 60) {
    return `${safe}s`;
  }
  const minutes = Math.floor(safe / 60);
  const remainder = safe % 60;
  if (minutes < 60) {
    return remainder > 0 ? `${minutes}m ${remainder}s` : `${minutes}m`;
  }
  const hours = Math.floor(minutes / 60);
  const restMinutes = minutes % 60;
  return restMinutes > 0 ? `${hours}h ${restMinutes}m` : `${hours}h`;
}

function parseTimestamp(value: string | null | undefined): number | null {
  if (!value) return null;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function computeElapsed(
  turn: TurnRow,
  isRunning: boolean,
  nowMs: number,
  fallbackStartMs: number,
): number {
  const start = parseTimestamp(turn.startedAt) ?? fallbackStartMs;
  if (isRunning) {
    return Math.max(0, (nowMs - start) / 1000);
  }
  const end = parseTimestamp(turn.finishedAt);
  // For finished turns without a recorded finish timestamp we don't have a
  // truthful answer; better to drop the duration than let it creep up over
  // time as the parent re-renders.
  if (end === null) return 0;
  return Math.max(0, (end - start) / 1000);
}

/**
 * Codex-parity collapsible header rendered above each multi-step
 * assistant turn. While the run is in flight the panel auto-expands and
 * the "已处理 Xs" counter ticks every second; when the stream finishes
 * it auto-collapses (unless the user manually toggled it).
 */
export function TurnSummary({
  turn,
  children,
  forceRunning = false,
}: {
  turn: TurnRow;
  children?: ReactNode;
  /**
   * When true, treat the turn as still in flight even if no block in it
   * is `pending=true`. ThreadPage uses this for the bottom-most turn
   * while the gateway/agent run is still active — covers the gap
   * between an assistant message finishing streaming and a tool call
   * settling.
   */
  forceRunning?: boolean;
}) {
  const isRunning = turn.isRunning || forceRunning;
  const { t } = useI18n();
  const [expanded, setExpanded] = useState(isRunning);
  const [userControlled, setUserControlled] = useState(false);
  const [nowMs, setNowMs] = useState(() => Date.now());
  // Fallback start time when the turn doesn't carry a real timestamp
  // (e.g. the synthetic placeholder rendered while we wait for the gateway
  // to ack a freshly-submitted prompt).
  const [mountStartMs] = useState(() => Date.now());

  // Auto-sync expanded state with isRunning until the user clicks once.
  useEffect(() => {
    if (!userControlled) {
      setExpanded(isRunning);
    }
  }, [isRunning, userControlled]);

  // Live ticker for the running counter.
  useEffect(() => {
    if (!isRunning) return;
    setNowMs(Date.now());
    const id = window.setInterval(() => setNowMs(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [isRunning]);

  const elapsed = computeElapsed(turn, isRunning, nowMs, mountStartMs);
  const elapsedLabel = elapsed >= 1 ? formatElapsed(elapsed) : null;
  // English Codex distinguishes "Working for X" (live) vs "Worked for X"
  // (done); the bundled zh-CN translation collapses both to "已处理 X".
  // Use distinct labels so the running state reads unambiguously.
  const summaryLabel = isRunning
    ? elapsedLabel
      ? t('Working for {duration}', { duration: elapsedLabel })
      : t('Working')
    : elapsedLabel
      ? t('Worked for {duration}', { duration: elapsedLabel })
      : t('Worked');
  const hasBody = Boolean(children);

  return (
    <div
      className={`turn-summary ${expanded ? 'is-expanded' : 'is-collapsed'} ${isRunning ? 'is-running' : ''} ${hasBody ? 'has-body' : 'no-body'}`}
    >
      <button
        aria-expanded={expanded}
        aria-label={
          expanded ? t('Collapse turn details') : t('Expand turn details')
        }
        className="turn-summary-toggle"
        onClick={() => {
          setUserControlled(true);
          setExpanded((current) => !current);
        }}
        type="button"
      >
        <span className="turn-summary-label">{summaryLabel}</span>
        <IconChevronDown
          aria-hidden
          className="turn-summary-chevron"
          size={ICON_SIZE}
          stroke={ICON_STROKE}
        />
      </button>
      <div aria-hidden className="turn-summary-divider" />
      {hasBody ? (
        <div
          aria-hidden={!expanded}
          className="turn-summary-body"
          inert={!expanded ? true : undefined}
        >
          {children}
        </div>
      ) : null}
    </div>
  );
}

