import { useEffect, useState } from 'react';

import type {
  CandidatesResponse,
  CandidateVerdict,
  CreateAutoResearchRunInput,
  DesktopAutoResearchIteration,
  DesktopAutoResearchRun,
  DesktopAutoResearchRunDetail,
  DesktopWorkspace,
  ResearchCandidate,
} from '@shared/contracts';

import { Button } from '../../../components/ui/button';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '../../../components/ui/table';
import { RichMessageContent } from '../../../message-rich-content';

import { CreateRunDialog } from './CreateRunDialog';
import {
  formatCompactTimestamp,
  formatRunStateLabel,
  formatTimestamp,
  firstNonEmptyLine,
  isTerminalRunState,
  verdictFeedback,
  verdictHeadline,
  verdictScore,
  verdictSummary,
  runPreview,
  runStatePillClass,
  scoreBgColor,
  scoreColor,
  statusIcon,
} from './helpers';
import { Section } from './ProgressBar';
import { useI18n } from '../../../i18n';

type AutoResearchPanelProps = {
  loading: boolean;
  saving: boolean;
  runs: DesktopAutoResearchRun[];
  runDetail: DesktopAutoResearchRunDetail | null;
  iterations: DesktopAutoResearchIteration[];
  candidatesResponse: CandidatesResponse | null;
  workspaces: DesktopWorkspace[];
  currentWorkspace: DesktopWorkspace | null;
  onCreateRun: (input: CreateAutoResearchRunInput) => Promise<void>;
  onRefresh: (runId: string) => Promise<void>;
  onSelectRun: (runId: string) => Promise<void>;
  onOpenThread: (threadId: string) => void;
  onStop: (runId: string) => Promise<void>;
  onDelete: (runId: string) => Promise<void>;
  onSelectCandidate?: (runId: string, candidateId: string) => Promise<void>;
};

function ScoreRing({ score }: { score: number | null }) {
  if (score == null) {
    return (
      <div
        style={{
          display: 'flex',
          height: 44,
          width: 44,
          alignItems: 'center',
          justifyContent: 'center',
          borderRadius: 9999,
          border: '1px solid var(--color-token-border)',
          background: 'var(--color-token-bg-tertiary)',
          color: 'var(--color-token-description-foreground)',
          fontSize: 11,
          fontWeight: 700,
        }}
      >
        --
      </div>
    );
  }
  return (
    <div
      style={{
        display: 'flex',
        height: 44,
        width: 44,
        alignItems: 'center',
        justifyContent: 'center',
        borderRadius: 9999,
        border: `2px solid ${scoreColor(score)}`,
        backgroundColor: scoreBgColor(score),
        color: scoreColor(score),
        fontSize: 13,
        fontWeight: 700,
      }}
    >
      {score.toFixed(1)}
    </div>
  );
}

function CandidateArtifact({
  emptyText,
  text,
}: {
  text?: string | null;
  emptyText: string;
}) {
  if (!text?.trim()) {
    return <p className="codex-command-row-desc">{emptyText}</p>;
  }
  return (
    <div style={{ borderRadius: 12, border: '1px solid var(--color-token-border)', background: 'var(--color-token-bg-secondary)', padding: 16, minWidth: 0, overflow: 'hidden', wordBreak: 'break-word' }}>
      <RichMessageContent altPrefix="auto-research" text={text} />
    </div>
  );
}

function VerdictDetails({
  verdict,
}: {
  verdict?: CandidateVerdict | null;
}) {
  const { t } = useI18n();

  if (!verdict) {
    return <p className="codex-command-row-desc">{t('No verifier feedback has landed yet.')}</p>;
  }

  const score = verdictScore(verdict);
  const feedback = verdictFeedback(verdict);

  return (
    <div style={{ display: 'grid', gap: 16 }}>
      <div style={{ display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: 12 }}>
        <ScoreRing score={score} />
        <div style={{ minWidth: 0, flex: 1 }}>
          <p style={{ fontSize: 13, fontWeight: 600, color: 'var(--color-token-text-primary)' }}>
            {verdictHeadline(verdict)}
          </p>
          {feedback ? (
            <p className="codex-command-row-desc" style={{ marginTop: 4, whiteSpace: 'pre-wrap' }}>
              {feedback}
            </p>
          ) : null}
        </div>
      </div>
    </div>
  );
}

export function AutoResearchPanel({
  loading,
  saving,
  runs,
  runDetail,
  iterations,
  candidatesResponse,
  workspaces,
  currentWorkspace,
  onCreateRun,
  onRefresh,
  onSelectRun,
  onOpenThread,
  onStop,
  onDelete,
  onSelectCandidate,
}: AutoResearchPanelProps) {
  const { t } = useI18n();
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [detailOpen, setDetailOpen] = useState(false);
  const [selectedIterationIndex, setSelectedIterationIndex] = useState<number | null>(null);

  const selectableWorkspaces = workspaces.filter((workspace) => workspace.available && workspace.path);

  const timelineIterations = runDetail?.latestIteration
    && !iterations.some((iteration) => iteration.iterationIndex === runDetail.latestIteration?.iterationIndex)
    ? [...iterations, runDetail.latestIteration]
    : iterations;

  const rankedCandidates = candidatesResponse?.candidates ?? [];

  useEffect(() => {
    if (!runDetail) {
      setSelectedIterationIndex(null);
      return;
    }
    setSelectedIterationIndex((current) => (
      current != null && timelineIterations.some((iteration) => iteration.iterationIndex === current)
        ? current
        : null
    ));
  }, [runDetail?.run.runId, runDetail?.activeThreadId, timelineIterations.length]);

  async function handleCreateRun(input: CreateAutoResearchRunInput) {
    await onCreateRun(input);
    setCreateDialogOpen(false);
  }

  function handleSelectRun(runId: string) {
    void onSelectRun(runId);
    setDetailOpen(true);
  }

  const selectedIteration = selectedIterationIndex != null
    ? timelineIterations.find((iteration) => iteration.iterationIndex === selectedIterationIndex) ?? null
    : null;
  const selectedIterationCandidate = selectedIteration
    ? rankedCandidates.find((candidate) => candidate.iteration === selectedIteration.iterationIndex) ?? null
    : null;
  const defaultWorkspacePath = currentWorkspace?.path || selectableWorkspaces[0]?.path || '';

  // ── Detail view ──
  if (detailOpen && runDetail) {
    return (
      <div className="agents-hub" style={{ overflowY: 'auto', overflowX: 'hidden' }}>
        {/* Back + header */}
        <div className="agents-hub-hero">
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            <Button onClick={() => setDetailOpen(false)} size="sm" variant="ghost">
              &larr; {t('Back')}
            </Button>
            {selectedIteration ? (
              <Button onClick={() => setSelectedIterationIndex(null)} size="sm" variant="ghost">
                &larr; {t('All Iterations')}
              </Button>
            ) : null}
            <span className={`${runStatePillClass(runDetail.run.state)}${!isTerminalRunState(runDetail.run.state) ? ' ar-pulse' : ''}`}>
              <span style={{ marginRight: 3 }}>{statusIcon(runDetail.run.state)}</span>
              {formatRunStateLabel(runDetail.run.state)}
            </span>
            {runDetail.activeThreadId ? (
              <span style={{ display: 'inline-flex', alignItems: 'center', gap: 4, fontSize: 11, color: 'var(--color-token-description-foreground)' }}>
                <span className="ar-pulse" style={{ height: 6, width: 6, borderRadius: 9999, background: 'var(--color-token-text-primary)', display: 'inline-block' }} />
                {t('Active')}
              </span>
            ) : null}
          </div>
          <div style={{ display: 'flex', gap: 6 }}>
            <Button
              disabled={loading}
              onClick={() => { void onRefresh(runDetail.run.runId); }}
              size="sm"
              variant="outline"
            >
              {loading ? t('Refreshing…') : t('Refresh')}
            </Button>
            {!isTerminalRunState(runDetail.run.state) ? (
              <Button
                disabled={saving}
                onClick={() => { void onStop(runDetail.run.runId); }}
                size="sm"
                variant="destructive"
              >
                {t('Stop')}
              </Button>
            ) : null}
          </div>
        </div>

        {/* Goal title */}
        <div className="ar-detail-title">
          <span className="ar-detail-task">{runDetail.run.goal}</span>
          <span className="ar-detail-meta">
            <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--text-xs)', opacity: 0.5 }}>{runDetail.run.runId}</span>
            {' · '}{t('Updated {time}', { time: formatTimestamp(runDetail.run.updatedAt) })}
          </span>
        </div>

        {selectedIteration ? (
          <>
            <Section
              description={t('Iteration {index} detail.', { index: selectedIteration.iterationIndex })}
              title={t('Iteration {index}', { index: selectedIteration.iterationIndex })}
            >
              <div style={{ display: 'grid', gap: 16 }}>
                <div style={{ display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: 8 }}>
                  <span className={`${runStatePillClass(selectedIteration.state)}${selectedIteration.state !== 'completed' ? ' ar-pulse' : ''}`} style={{ fontSize: 10 }}>
                    <span style={{ marginRight: 2 }}>{statusIcon(selectedIteration.state)}</span>
                    {formatRunStateLabel(selectedIteration.state)}
                  </span>
                  {verdictScore(selectedIterationCandidate?.verdict) != null ? (
                    <span className="codex-sync-pill ok">{verdictHeadline(selectedIterationCandidate?.verdict)}</span>
                  ) : null}
                  <span className="codex-command-row-desc">
                    {formatTimestamp(selectedIteration.startedAt)}{selectedIteration.completedAt ? ` — ${formatTimestamp(selectedIteration.completedAt)}` : ''}
                  </span>
                  {selectedIteration.workThreadId ? (
                    <button className="codex-section-action" onClick={() => onOpenThread(selectedIteration.workThreadId!)} type="button">{t('Open Work Thread')}</button>
                  ) : null}
                  {selectedIteration.verifyThreadId ? (
                    <button className="codex-section-action" onClick={() => onOpenThread(selectedIteration.verifyThreadId!)} type="button">{t('Open Verify Thread')}</button>
                  ) : null}
                </div>

                <Section description={t('What the worker actually produced in this round.')} title={t('Candidate Output')}>
                  <CandidateArtifact
                    emptyText={t('The worker has not emitted a candidate artifact yet.')}
                    text={selectedIterationCandidate?.output}
                  />
                </Section>

                <Section description={t('What the verifier said about this round.')} title={t('Verifier Feedback')}>
                  <VerdictDetails verdict={selectedIterationCandidate?.verdict} />
                </Section>
              </div>
            </Section>

            {createDialogOpen ? (
              <CreateRunDialog defaultWorkspacePath={defaultWorkspacePath} onClose={() => setCreateDialogOpen(false)} onSubmit={handleCreateRun} saving={saving} workspaces={workspaces} />
            ) : null}
          </>
        ) : (
          <>
        <div
          style={{
            display: 'flex',
            flexWrap: 'wrap',
            gap: 8,
            alignItems: 'center',
            color: 'var(--color-token-description-foreground)',
            fontSize: 12,
          }}
        >
          <span className="codex-sync-pill ok">
            {t('{used}/{max} iterations', {
              used: runDetail.run.iterationsUsed,
              max: runDetail.run.maxIterations,
            })}
          </span>
          {runDetail.run.selectedCandidate ? (
            <span className="codex-sync-pill">{t('Selected: {candidate}', { candidate: runDetail.run.selectedCandidate })}</span>
          ) : null}
          <span className="codex-command-row-desc">{t('Updated {time}', { time: formatTimestamp(runDetail.run.updatedAt) })}</span>
        </div>

        <Section description={t('One row per round. Click a row or the detail action to inspect that iteration.')} title={t('Iterations')}>
          {timelineIterations.length ? (
            <Table className="agents-hub-table">
              <TableHeader>
                <TableRow>
                  <TableHead style={{ width: '10%' }}>{t('Iteration')}</TableHead>
                  <TableHead style={{ width: '12%' }}>{t('State')}</TableHead>
                  <TableHead style={{ width: '11%' }}>{t('Score')}</TableHead>
                  <TableHead style={{ width: '25%' }}>{t('Output')}</TableHead>
                  <TableHead style={{ width: '26%' }}>{t('Verify')}</TableHead>
                  <TableHead style={{ width: '10%' }}>{t('Started')}</TableHead>
                  <TableHead style={{ width: '6%' }} className="text-right">{t('Detail')}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {timelineIterations.map((iteration) => {
                  const iterationCandidate = rankedCandidates.find((candidate) => candidate.iteration === iteration.iterationIndex) ?? null;
                  const iterationVerdict = iterationCandidate?.verdict ?? null;
                  return (
                    <TableRow
                      className="cursor-pointer"
                      key={`${iteration.runId}:${iteration.iterationIndex}`}
                      onClick={() => setSelectedIterationIndex(iteration.iterationIndex)}
                    >
                      <TableCell>
                        <div className="agents-hub-cell-name">{t('Iteration {index}', { index: iteration.iterationIndex })}</div>
                        <div className="agents-hub-cell-id">
                          {iteration.completedAt ? t('Completed') : t('In progress')}
                        </div>
                      </TableCell>
                      <TableCell>
                        <span className={`${runStatePillClass(iteration.state)}${iteration.state !== 'completed' ? ' ar-pulse' : ''}`} style={{ fontSize: 10 }}>
                          <span style={{ marginRight: 2 }}>{statusIcon(iteration.state)}</span>
                          {formatRunStateLabel(iteration.state)}
                        </span>
                      </TableCell>
                      <TableCell>
                        {verdictScore(iterationVerdict) != null
                          ? verdictHeadline(iterationVerdict)
                          : t('Pending')}
                      </TableCell>
                      <TableCell>
                        <div className="agents-hub-cell-name" style={{ display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical', overflow: 'hidden' }}>
                          {firstNonEmptyLine(iterationCandidate?.output || '') || t('No candidate output yet')}
                        </div>
                      </TableCell>
                      <TableCell>
                        <div className="agents-hub-cell-name" style={{ display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical', overflow: 'hidden' }}>
                          {verdictSummary(iterationVerdict) || t('No verifier result yet')}
                        </div>
                        <div className="agents-hub-cell-id" style={{ display: '-webkit-box', WebkitLineClamp: 1, WebkitBoxOrient: 'vertical', overflow: 'hidden' }}>
                          {verdictScore(iterationVerdict) != null ? verdictHeadline(iterationVerdict) : t('Pending')}
                        </div>
                      </TableCell>
                      <TableCell>
                        {formatCompactTimestamp(iteration.startedAt)}
                      </TableCell>
                      <TableCell className="text-right" style={{ display: 'flex', gap: 4, justifyContent: 'flex-end' }}>
                        {onSelectCandidate && iterationCandidate && iterationCandidate.verdict && runDetail.run.selectedCandidate !== iterationCandidate.candidate_id ? (
                          <Button
                            onClick={(event) => {
                              event.stopPropagation();
                              void onSelectCandidate(runDetail.run.runId, iterationCandidate.candidate_id);
                            }}
                            size="sm"
                            variant="outline"
                            title={t('Select this candidate as the winner')}
                          >
                            ✓
                          </Button>
                        ) : null}
                        <Button
                          onClick={(event) => {
                            event.stopPropagation();
                            setSelectedIterationIndex(iteration.iterationIndex);
                          }}
                          size="sm"
                          variant="ghost"
                        >
                          {t('View')}
                        </Button>
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          ) : (
            <p className="codex-command-row-desc">{t('The run has not produced iteration records yet.')}</p>
          )}
        </Section>

        {createDialogOpen ? (
          <CreateRunDialog defaultWorkspacePath={defaultWorkspacePath} onClose={() => setCreateDialogOpen(false)} onSubmit={handleCreateRun} saving={saving} workspaces={workspaces} />
        ) : null}
          </>
        )}
      </div>
    );
  }

  // ── Table list view ──
  return (
    <div className="agents-hub">
      <div className="agents-hub-hero">
        <span className="ar-page-title">{t('Auto Research')}</span>
        <div className="agents-hub-controls">
          <Button onClick={() => setCreateDialogOpen(true)} size="sm">
            {t('+ New Run')}
          </Button>
        </div>
      </div>

      {loading && !runs.length ? (
        <div className="agents-hub-empty-state">{t('Loading runs...')}</div>
      ) : runs.length ? (
        <Table className="agents-hub-table">
          <TableHeader>
            <TableRow>
              <TableHead style={{ width: '40%' }}>{t('Goal')}</TableHead>
              <TableHead style={{ width: '15%' }}>{t('State')}</TableHead>
              <TableHead style={{ width: '15%' }}>{t('Iterations')}</TableHead>
              <TableHead style={{ width: '15%' }}>{t('Updated')}</TableHead>
              <TableHead style={{ width: '15%' }} className="text-right">{t('Actions')}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {runs.map((run) => (
              <TableRow className="cursor-pointer" key={run.runId} onClick={() => handleSelectRun(run.runId)}>
                <TableCell>
                  <div className="agents-hub-cell-name" style={{ display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical', overflow: 'hidden' }}>
                    {run.goal}
                  </div>
                  <div className="agents-hub-cell-id">{runPreview(run)}</div>
                </TableCell>
                <TableCell>
                  <span className={`${runStatePillClass(run.state)}${!isTerminalRunState(run.state) && run.state !== 'queued' ? ' ar-pulse' : ''}`}>
                    <span style={{ marginRight: 2 }}>{statusIcon(run.state)}</span>
                    {formatRunStateLabel(run.state)}
                  </span>
                </TableCell>
                <TableCell>
                  {run.iterationsUsed}/{run.maxIterations}
                </TableCell>
                <TableCell>
                  {formatCompactTimestamp(run.updatedAt)}
                </TableCell>
                <TableCell className="text-right">
                  <div className="agents-hub-row-actions">
                    {!isTerminalRunState(run.state) ? (
                      <Button
                        disabled={saving}
                        onClick={(e) => { e.stopPropagation(); void onStop(run.runId); }}
                        size="sm"
                        variant="destructive"
                      >
                        {t('Stop')}
                      </Button>
                    ) : null}
                    <Button
                      disabled={saving}
                      onClick={(e) => { e.stopPropagation(); void onDelete(run.runId); }}
                      size="sm"
                      variant="ghost"
                      className="text-destructive"
                    >
                      {t('Delete')}
                    </Button>
                  </div>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      ) : (
        <div className="agents-hub-empty-state" style={{ padding: '48px 0' }}>
          {t('No runs yet. Click + New Run to start a bounded research loop.')}
        </div>
      )}

      {createDialogOpen ? (
        <CreateRunDialog defaultWorkspacePath={defaultWorkspacePath} onClose={() => setCreateDialogOpen(false)} onSubmit={handleCreateRun} saving={saving} workspaces={workspaces} />
      ) : null}
    </div>
  );
}
