import { memo, type ReactNode, useEffect, useState } from 'react';

import { ChevronDown } from 'lucide-react';

import { useI18n } from './i18n';
import type { TurnRow } from './render-view-model';

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
 * the elapsed-time counter ticks every second; when the stream finishes
 * it auto-collapses (unless the user manually toggled it).
 */
function TurnSummaryComponent({
  turn,
  children,
}: {
  turn: TurnRow;
  children?: ReactNode;
}) {
  const isRunning = turn.isRunning;
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
  const elapsedLabel = formatElapsed(elapsed);
  // English Codex distinguishes the live and done elapsed-time labels. Use
  // distinct keys so every locale can render the running state unambiguously.
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
        <ChevronDown
          aria-hidden
          className="turn-summary-chevron"
          size={ICON_SIZE}
          strokeWidth={ICON_STROKE}
        />
      </button>
      <div aria-hidden className="turn-summary-divider" />
      {hasBody ? (
        <div
          aria-hidden={!expanded}
          className="turn-summary-body"
          inert={!expanded ? true : undefined}
        >
          <div className="turn-summary-body-inner">{children}</div>
        </div>
      ) : null}
    </div>
  );
}

export const TurnSummary = memo(
  TurnSummaryComponent,
  (previous, next) => previous.turn === next.turn,
);
