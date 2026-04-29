import type { ResearchCandidate } from '@shared/contracts';

import { RichMessageContent } from '../../../message-rich-content';

import { useI18n } from '../../../i18n';
import { scoreBgColor, scoreColor } from './helpers';

/* Candidate leaderboard row */
export function CandidateRow({
  candidate,
  rank,
  isBest,
  isSelected,
  isExpanded,
  onToggle,
  onSelect,
  preview,
  candidateText,
  saving,
}: {
  candidate: ResearchCandidate;
  rank: number;
  isBest: boolean;
  isSelected: boolean;
  isExpanded: boolean;
  onToggle: () => void;
  onSelect: () => void;
  preview?: string | null;
  candidateText?: string | null;
  saving: boolean;
}) {
  const { t } = useI18n();
  const score = candidate.verdict?.score ?? 0;

  return (
    <div
      style={{
        borderRadius: 12,
        border: `1px solid ${isExpanded ? 'var(--color-token-border-focus, var(--color-token-border))' : 'var(--color-token-border)'}`,
        background: isExpanded ? 'var(--color-token-bg-secondary)' : 'var(--color-token-bg-primary)',
        transition: 'background 150ms, border-color 150ms',
      }}
    >
      <button
        style={{ display: 'flex', width: '100%', alignItems: 'center', gap: 12, padding: '12px 16px', textAlign: 'left' }}
        onClick={onToggle}
        type="button"
      >
        {/* Rank */}
        <span style={{
          display: 'flex', height: 24, width: 24, flexShrink: 0, alignItems: 'center', justifyContent: 'center',
          borderRadius: 9999, background: 'var(--color-token-bg-tertiary)', fontSize: 11, fontWeight: 700, color: 'var(--color-token-description-foreground)',
        }}>
          {isBest ? '\uD83D\uDC51' : `#${rank}`}
        </span>
        {/* Score circle */}
        <div
          style={{
            display: 'flex', height: 32, width: 32, flexShrink: 0, alignItems: 'center', justifyContent: 'center',
            borderRadius: 9999, border: `2px solid ${scoreColor(score)}`,
            backgroundColor: scoreBgColor(score), color: scoreColor(score), fontSize: 12, fontWeight: 700,
          }}
        >
          {score.toFixed(1)}
        </div>
        {/* Summary */}
        <div style={{ minWidth: 0, flex: 1 }}>
          <p style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: 13, fontWeight: 500, color: 'var(--color-token-text-primary)' }}>
            {t('Iter {iteration}: {text}', {
              iteration: candidate.iteration,
              text: preview || candidate.output || t('No candidate output'),
            })}
          </p>
          {candidate.output ? (
            <p className="codex-command-row-desc" style={{ marginTop: 4, display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical', overflow: 'hidden' }}>
              {candidate.output}
            </p>
          ) : null}
        </div>
      </button>

      {/* Expanded detail */}
      {isExpanded && (
        <div style={{ borderTop: '1px solid var(--color-token-border)', padding: '12px 16px', display: 'flex', flexDirection: 'column', gap: 12 }}>
          {candidateText ? (
            <div>
              <p style={{ fontSize: 11, fontWeight: 600, textTransform: 'uppercase', letterSpacing: '0.14em', color: 'var(--color-token-description-foreground)' }}>{t('Candidate Output')}</p>
              <div style={{ marginTop: 6, borderRadius: 12, border: '1px solid var(--color-token-border)', background: 'var(--color-token-bg-secondary)', padding: 12 }}>
                <RichMessageContent altPrefix="auto-research-candidate" text={candidateText} />
              </div>
            </div>
          ) : null}

          {candidate.verdict ? (
            <div>
              <p style={{ fontSize: 11, fontWeight: 600, textTransform: 'uppercase', letterSpacing: '0.14em', color: 'var(--color-token-description-foreground)' }}>{t('Verdict')}</p>
              <div style={{ marginTop: 4, display: 'flex', alignItems: 'center', gap: 8 }}>
                <span style={{ fontSize: 14, fontWeight: 700, color: scoreColor(candidate.verdict.score) }}>
                  {candidate.verdict.score.toFixed(1)}/10
                </span>
              </div>
              {candidate.verdict.feedback ? (
                <p style={{ marginTop: 8, fontSize: 12, lineHeight: 1.5, color: 'var(--color-token-description-foreground)' }}>
                  {candidate.verdict.feedback}
                </p>
              ) : null}
            </div>
          ) : null}

          {/* Select winner button */}
          {!isSelected ? (
            <button
              className="codex-section-action"
              disabled={saving}
              onClick={(e) => {
                e.stopPropagation();
                onSelect();
              }}
              style={{ alignSelf: 'flex-start', color: 'var(--color-token-text-primary)', fontWeight: 500 }}
              type="button"
            >
              {t('Select as Winner')}
            </button>
          ) : (
            <span className="codex-sync-pill ok">
              {t('Selected Winner')}
            </span>
          )}
        </div>
      )}
    </div>
  );
}
