import type { ResearchCandidate } from '@shared/contracts';

import { scoreColor } from './helpers';

/* Score progression SVG chart */
export function ScoreProgressionChart({ candidates }: { candidates: ResearchCandidate[] }) {
  const sorted = [...candidates]
    .filter((c) => c.verdict)
    .sort((a, b) => a.iteration - b.iteration);
  if (sorted.length < 2) return null;

  const W = 400;
  const H = 140;
  const PAD_X = 36;
  const PAD_Y = 20;
  const minIter = sorted[0].iteration;
  const maxIter = sorted[sorted.length - 1].iteration;
  const iterRange = Math.max(1, maxIter - minIter);
  const maxScore = 10;

  function x(iter: number) {
    return PAD_X + ((iter - minIter) / iterRange) * (W - PAD_X * 2);
  }
  function y(score: number) {
    return PAD_Y + (1 - score / maxScore) * (H - PAD_Y * 2);
  }

  // Best-so-far line
  const bestLine: { iter: number; score: number }[] = [];
  let bestSoFar = 0;
  for (const c of sorted) {
    const s = c.verdict!.score;
    if (s > bestSoFar) bestSoFar = s;
    bestLine.push({ iter: c.iteration, score: bestSoFar });
  }

  const bestPath = bestLine
    .map((p, i) => `${i === 0 ? 'M' : 'L'}${x(p.iter).toFixed(1)},${y(p.score).toFixed(1)}`)
    .join(' ');

  return (
    <svg viewBox={`0 0 ${W} ${H}`} style={{ width: '100%', maxHeight: 160 }}>
      {/* Grid lines */}
      {[0, 2, 4, 6, 8, 10].map((v) => (
        <g key={v}>
          <line x1={PAD_X} y1={y(v)} x2={W - PAD_X} y2={y(v)} stroke="var(--color-token-border)" strokeWidth="0.5" />
          <text x={PAD_X - 6} y={y(v) + 3} textAnchor="end" fontSize="9" fill="var(--color-token-description-foreground)">{v}</text>
        </g>
      ))}
      {/* Best-so-far line */}
      <path d={bestPath} fill="none" stroke="var(--ar-score-top)" strokeWidth="1.5" strokeDasharray="4 2" opacity="0.6" />
      {/* Candidate dots */}
      {sorted.map((c) => {
        const s = c.verdict!.score;
        return (
          <circle
            key={c.iteration}
            cx={x(c.iteration)}
            cy={y(s)}
            r={4}
            fill={scoreColor(s)}
            stroke="var(--color-token-bg-primary, white)"
            strokeWidth="1.5"
          >
            <title>Iter {c.iteration}: {s.toFixed(1)}</title>
          </circle>
        );
      })}
      {/* X-axis labels */}
      {sorted.map((c) => (
        <text key={`lbl-${c.iteration}`} x={x(c.iteration)} y={H - 2} textAnchor="middle" fontSize="9" fill="var(--color-token-description-foreground)">
          {c.iteration}
        </text>
      ))}
    </svg>
  );
}
