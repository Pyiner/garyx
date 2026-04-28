import type {
  CandidateVerdict,
  DesktopAutoResearchRun,
} from '@shared/contracts';

import type { LoopNodeStatus } from './types';

/* ── Score color helpers (backed by CSS custom properties in styles.css) ── */

export function scoreColor(score: number): string {
  if (score <= 3) return 'var(--ar-score-low)';
  if (score <= 6) return 'var(--ar-score-mid)';
  if (score <= 8) return 'var(--ar-score-high)';
  return 'var(--ar-score-top)';
}

export function scoreBgColor(score: number): string {
  if (score <= 3) return 'var(--ar-score-low-bg)';
  if (score <= 6) return 'var(--ar-score-mid-bg)';
  if (score <= 8) return 'var(--ar-score-high-bg)';
  return 'var(--ar-score-top-bg)';
}

export function scoreBorderColor(score: number): string {
  if (score <= 3) return 'var(--ar-score-low-border)';
  if (score <= 6) return 'var(--ar-score-mid-border)';
  if (score <= 8) return 'var(--ar-score-high-border)';
  return 'var(--ar-score-top-border)';
}

/* ── Status / state helpers ── */

export function statusIcon(status: string): string {
  if (status === 'running' || status === 'researching' || status === 'judging' || status === 'queued') return '⏳';
  if (status === 'blocked' || status === 'budget_exhausted') return '⚠';
  if (status === 'user_stopped') return '■';
  return '○';
}

export function runStatePillClass(state: string): string {
  if (state === 'blocked' || state === 'budget_exhausted') {
    return 'codex-sync-pill fail';
  }
  return 'codex-sync-pill';
}

export function isTerminalRunState(state: string): boolean {
  return ['budget_exhausted', 'blocked', 'user_stopped'].includes(state);
}

export function loopNodeStatusIcon(status: LoopNodeStatus): React.ReactNode {
  if (status === 'running') {
    return (
      <svg className="ar-spin" style={{ height: 12, width: 12 }} viewBox="0 0 16 16" fill="none">
        <circle cx="8" cy="8" r="6" stroke="currentColor" strokeWidth="2" strokeDasharray="28" strokeDashoffset="8" strokeLinecap="round" />
      </svg>
    );
  }
  if (status === 'completed') {
    return <span style={{ fontSize: 11 }}>✓</span>;
  }
  return <span style={{ fontSize: 10 }}>○</span>;
}

/* ── Formatting helpers ── */

export function formatRunStateLabel(value: string): string {
  return value.replaceAll('_', ' ');
}

export function formatTimestamp(value?: string | null): string {
  if (!value) {
    return 'Unknown';
  }

  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) {
    return value;
  }

  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  }).format(parsed);
}

export function formatCompactTimestamp(value?: string | null): string {
  if (!value) {
    return 'Unknown';
  }

  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) {
    return value;
  }

  const now = new Date();
  const sameDay = parsed.toDateString() === now.toDateString();

  return new Intl.DateTimeFormat(undefined, sameDay
    ? { hour: 'numeric', minute: '2-digit' }
    : { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' },
  ).format(parsed);
}

export function formatDurationMinutes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) {
    return 'Unknown';
  }

  if (value < 3600) {
    return `${Math.round(value / 60)} min`;
  }

  const hours = Math.floor(value / 3600);
  const minutes = Math.round((value % 3600) / 60);
  return minutes ? `${hours}h ${minutes}m` : `${hours}h`;
}

export function firstNonEmptyLine(value: string): string {
  return value
    .split('\n')
    .map((line) => line.trim())
    .find(Boolean) || '';
}

export function verdictScore(
  verdict?: CandidateVerdict | null,
): number | null {
  if (!verdict) return null;
  return verdict.score;
}

export function verdictFeedback(
  verdict?: CandidateVerdict | null,
): string {
  if (!verdict) return '';
  return verdict.feedback || '';
}

export function verdictSummary(
  verdict?: CandidateVerdict | null,
): string {
  if (!verdict) {
    return 'Verifier has not produced a verdict yet.';
  }
  return verdict.feedback || 'Verifier produced a verdict.';
}

export function verdictHeadline(
  verdict?: CandidateVerdict | null,
): string {
  const score = verdictScore(verdict);
  if (score == null) {
    return 'Pending verdict';
  }
  return `${score.toFixed(1)}/10`;
}

export function runPreview(run: DesktopAutoResearchRun): string {
  const preview = firstNonEmptyLine(run.goal);
  if (preview) {
    return preview;
  }
  if (run.terminalReason) {
    return `Stopped because ${formatRunStateLabel(run.terminalReason)}.`;
  }
  return 'Inspect the current brief, candidate, and verdict.';
}

