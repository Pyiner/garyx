import { Fragment, useEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import {
  ArrowDownUp,
  ArrowLeft,
  Check,
  ChevronRight,
  ExternalLink,
  LoaderCircle,
  Plus,
  RefreshCw,
  Square,
  Trash,
} from 'lucide-react';

import type {
  CandidatesResponse,
  CreateAutoResearchRunInput,
  DesktopAutoResearchIteration,
  DesktopAutoResearchRun,
  DesktopAutoResearchRunDetail,
  DesktopWorkspace,
  ResearchCandidate,
} from '@shared/contracts';

import { Badge } from '../../../components/ui/badge';
import { Button } from '../../../components/ui/button';
import { Card, CardContent } from '../../../components/ui/card';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '../../../components/ui/table';
import { useI18n } from '../../../i18n';
import { cn } from '../../../lib/utils';
import { RichMessageContent } from '../../../message-rich-content';

import { CreateRunDialog } from './CreateRunDialog';
import {
  firstNonEmptyLine,
  formatCompactTimestamp,
  formatDurationMinutes,
  formatRunStateLabel,
  formatTimestamp,
  isTerminalRunState,
  scoreColor,
  verdictFeedback,
  verdictScore,
  verdictSummary,
} from './helpers';

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

function pct(value: number, max: number): number {
  if (!Number.isFinite(value) || !Number.isFinite(max) || max <= 0) return 0;
  return Math.max(0, Math.min(100, (value / max) * 100));
}

function parseTime(value?: string | null): number | null {
  if (!value) return null;
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? null : parsed;
}

function runElapsedSecs(run: DesktopAutoResearchRun): number {
  const created = parseTime(run.createdAt);
  if (created == null) return 0;
  const updated = parseTime(run.updatedAt) ?? Date.now();
  const end = isTerminalRunState(run.state) ? updated : Math.max(updated, Date.now());
  return Math.max(0, Math.round((end - created) / 1000));
}

function shortWorkspace(path?: string | null): string {
  if (!path?.trim()) return '';
  const trimmed = path.trim();
  const parts = trimmed.split('/').filter(Boolean);
  if (trimmed.startsWith('~')) return trimmed;
  if (parts.length <= 2) return trimmed;
  return `~/${parts.slice(-2).join('/')}`;
}

function bestCandidate(candidates: ResearchCandidate[], explicitBestId?: string | null): ResearchCandidate | null {
  if (explicitBestId) {
    const explicit = candidates.find((candidate) => candidate.candidate_id === explicitBestId);
    if (explicit) return explicit;
  }
  return candidates
    .filter((candidate) => verdictScore(candidate.verdict) != null)
    .sort((a, b) => (verdictScore(b.verdict) ?? -1) - (verdictScore(a.verdict) ?? -1))[0] ?? null;
}

function RunStateBadge({
  state,
  active,
}: {
  state: string;
  active?: boolean;
}) {
  return (
    <Badge
      className={cn(
        'ar-state-badge',
        `ar-state-badge--${state.replaceAll('_', '-')}`,
        active ? 'ar-state-badge--live' : null,
      )}
      variant="secondary"
    >
      <span className="ar-state-badge-dot" />
      {formatRunStateLabel(state)}
    </Badge>
  );
}

function StatusMeter({
  label,
  value,
  max,
  valueLabel,
  tone = 'default',
}: {
  label: string;
  value: number;
  max: number;
  valueLabel: string;
  tone?: 'default' | 'accent';
}) {
  return (
    <div className="ar-status-meter">
      <div className="ar-status-meter-head">
        <span>{label}</span>
        <span className="ar-num">{valueLabel}</span>
      </div>
      <div className="ar-status-meter-track">
        <span
          className={cn('ar-status-meter-fill', tone === 'accent' ? 'ar-status-meter-fill--accent' : null)}
          style={{ width: `${pct(value, max)}%` }}
        />
      </div>
    </div>
  );
}

function ScoreProgression({
  activeIteration,
  candidates,
}: {
  activeIteration: number | null;
  candidates: ResearchCandidate[];
}) {
  const { t } = useI18n();
  const scored = [...candidates]
    .filter((candidate) => verdictScore(candidate.verdict) != null)
    .sort((a, b) => a.iteration - b.iteration);

  if (scored.length < 2) {
    return (
      <div className="ar-score-chart-empty">
        <span>{t('No score yet')}</span>
      </div>
    );
  }

  const width = 300;
  const height = 64;
  const padX = 12;
  const padY = 10;
  const minIteration = scored[0]?.iteration ?? 0;
  const maxIteration = scored[scored.length - 1]?.iteration ?? minIteration + 1;
  const iterationRange = Math.max(1, maxIteration - minIteration);
  const points = scored.map((candidate) => {
    const score = verdictScore(candidate.verdict) ?? 0;
    return {
      candidate,
      score,
      x: padX + ((candidate.iteration - minIteration) / iterationRange) * (width - padX * 2),
      y: padY + (1 - score / 10) * (height - padY * 2),
    };
  });
  const linePath = points
    .map((point, index) => `${index === 0 ? 'M' : 'L'} ${point.x.toFixed(1)} ${point.y.toFixed(1)}`)
    .join(' ');
  const areaPath = `${linePath} L ${points[points.length - 1].x.toFixed(1)} ${height} L ${points[0].x.toFixed(1)} ${height} Z`;

  return (
    <svg className="ar-score-chart" preserveAspectRatio="none" viewBox={`0 0 ${width} ${height}`}>
      <defs>
        <linearGradient id="ar-score-chart-fill" x1="0" x2="0" y1="0" y2="1">
          <stop offset="0%" stopColor="currentColor" stopOpacity="0.12" />
          <stop offset="100%" stopColor="currentColor" stopOpacity="0" />
        </linearGradient>
      </defs>
      <line className="ar-score-chart-axis" x1={padX} x2={width - padX} y1={padY + 6} y2={padY + 6} />
      <line className="ar-score-chart-axis" x1={padX} x2={width - padX} y1={height - padY} y2={height - padY} />
      <path className="ar-score-chart-area" d={areaPath} />
      <path className="ar-score-chart-line" d={linePath} />
      {points.map((point, index) => {
        const isLast = index === points.length - 1;
        const isActive = point.candidate.iteration === activeIteration;
        return (
          <circle
            className={cn(
              'ar-score-chart-dot',
              isLast ? 'ar-score-chart-dot--last' : null,
              isActive ? 'ar-score-chart-dot--active' : null,
            )}
            cx={point.x}
            cy={point.y}
            key={point.candidate.candidate_id}
            r={isLast ? 3.2 : 2.6}
          >
            <title>{`i${point.candidate.iteration}: ${point.score.toFixed(1)}/10`}</title>
          </circle>
        );
      })}
    </svg>
  );
}

function CandidatePanel({
  candidate,
  iteration,
  onOpenThread,
}: {
  candidate: ResearchCandidate | null;
  iteration: DesktopAutoResearchIteration;
  onOpenThread: (threadId: string) => void;
}) {
  const { t } = useI18n();
  const feedback = verdictFeedback(candidate?.verdict);
  const score = verdictScore(candidate?.verdict);

  return (
    <div className="ar-iteration-body">
      <div className="ar-iteration-body-grid">
        <div className="ar-iteration-panel">
          <div className="ar-iteration-panel-head">
            <span>{t('Candidate output')}</span>
            {iteration.workThreadId ? (
              <Button onClick={() => onOpenThread(iteration.workThreadId!)} size="xs" variant="ghost">
                {t('Work thread')}
                <ExternalLink />
              </Button>
            ) : null}
          </div>
          <div className="ar-iteration-panel-body">
            {candidate?.output?.trim() ? (
              <RichMessageContent altPrefix="auto-research-candidate" text={candidate.output} />
            ) : (
              <p className="codex-command-row-desc">{t('No candidate output yet')}</p>
            )}
          </div>
        </div>
        <div className="ar-iteration-panel">
          <div className="ar-iteration-panel-head">
            <span>
              {t('Verifier feedback')}
              {score != null ? <span className="ar-panel-score"> · {score.toFixed(1)}/10</span> : null}
            </span>
            {iteration.verifyThreadId ? (
              <Button onClick={() => onOpenThread(iteration.verifyThreadId!)} size="xs" variant="ghost">
                {t('Verify thread')}
                <ExternalLink />
              </Button>
            ) : null}
          </div>
          <div className="ar-iteration-panel-body">
            {feedback ? (
              <RichMessageContent altPrefix="auto-research-verdict" text={feedback} />
            ) : (
              <p className="codex-command-row-desc">{t('No verifier result yet')}</p>
            )}
          </div>
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
  const [expandedIterationIndex, setExpandedIterationIndex] = useState<number | null>(null);
  const detailRunIdRef = useRef<string | null>(null);

  const selectableWorkspaces = workspaces.filter((workspace) => workspace.available && workspace.path);
  const defaultWorkspacePath = currentWorkspace?.path || selectableWorkspaces[0]?.path || '';

  const timelineIterations = runDetail?.latestIteration
    && !iterations.some((iteration) => iteration.iterationIndex === runDetail.latestIteration?.iterationIndex)
    ? [...iterations, runDetail.latestIteration]
    : iterations;

  const detailCandidates = useMemo(
    () => (candidatesResponse?.candidates?.length ? candidatesResponse.candidates : runDetail?.run.candidates ?? []),
    [candidatesResponse?.candidates, runDetail?.run.candidates],
  );

  const candidateByIteration = useMemo(() => new Map(
    detailCandidates.map((candidate) => [candidate.iteration, candidate]),
  ), [detailCandidates]);

  const displayIterations = useMemo(
    () => [...timelineIterations].sort((a, b) => b.iterationIndex - a.iterationIndex),
    [timelineIterations],
  );

  const currentBestCandidate = bestCandidate(detailCandidates, candidatesResponse?.bestCandidateId);
  const bestScore = verdictScore(currentBestCandidate?.verdict);

  useEffect(() => {
    const runId = runDetail?.run.runId ?? null;
    setExpandedIterationIndex((current) => {
      if (!runId || detailRunIdRef.current !== runId) {
        return null;
      }
      if (current != null && timelineIterations.some((iteration) => iteration.iterationIndex === current)) {
        return current;
      }
      return null;
    });
    detailRunIdRef.current = runId;
  }, [runDetail?.run.runId, timelineIterations]);

  async function handleCreateRun(input: CreateAutoResearchRunInput) {
    await onCreateRun(input);
    setCreateDialogOpen(false);
  }

  function handleSelectRun(runId: string) {
    void onSelectRun(runId);
    setDetailOpen(true);
  }

  if (detailOpen && runDetail) {
    const run = runDetail.run;
    const elapsedSecs = runElapsedSecs(run);
    const activeIteration = runDetail.latestIteration?.iterationIndex ?? displayIterations[0]?.iterationIndex ?? null;
    const active = !isTerminalRunState(run.state);
    const selectedCandidateId = run.selectedCandidate ?? null;

    return (
      <div className="agents-hub ar-detail-page">
        <div className="agents-hub-hero ar-detail-toolbar">
          <div className="ar-detail-toolbar-left">
            <Button onClick={() => setDetailOpen(false)} size="sm" variant="ghost">
              <ArrowLeft />
              {t('Back')}
            </Button>
            <span className="ar-detail-run-id">{run.runId}</span>
          </div>
          <div className="ar-detail-toolbar-actions">
            <Button
              disabled={loading}
              onClick={() => { void onRefresh(run.runId); }}
              size="icon-sm"
              title={t('Refresh')}
              variant="ghost"
            >
              <RefreshCw className={loading ? 'ar-spin' : undefined} />
            </Button>
            {!isTerminalRunState(run.state) ? (
              <Button
                disabled={saving}
                onClick={() => { void onStop(run.runId); }}
                size="sm"
                variant="outline"
              >
                <Square />
                {t('Stop')}
              </Button>
            ) : null}
          </div>
        </div>

        <div className="ar-detail-title">
          <span className="ar-detail-task">{run.goal}</span>
        </div>

        <Card className="ar-status-card">
          <CardContent className="ar-status-card-content">
            <div className="ar-status-score-group">
              <RunStateBadge active={active} state={run.state} />
              <div className="ar-best-score">
                <span className="ar-best-score-value">{bestScore != null ? bestScore.toFixed(1) : '--'}</span>
                <span className="ar-best-score-max">/ 10</span>
                <span className="ar-best-score-label">{t('Best score')}</span>
              </div>
            </div>

            <div className="ar-status-divider" />

            <div className="ar-status-meters">
              <StatusMeter
                label={t('Iterations')}
                max={run.maxIterations}
                tone="accent"
                value={run.iterationsUsed}
                valueLabel={`${run.iterationsUsed} / ${run.maxIterations}`}
              />
              <StatusMeter
                label={t('Time budget')}
                max={run.timeBudgetSecs}
                value={elapsedSecs}
                valueLabel={`${formatDurationMinutes(elapsedSecs)} / ${formatDurationMinutes(run.timeBudgetSecs)}`}
              />
            </div>

            <div className="ar-status-divider" />

            <div className="ar-status-chart">
              <div className="ar-status-chart-head">
                <span>{t('Score progression')}</span>
                {bestScore != null ? (
                  <span className="ar-status-chart-delta">{t('best {score}', { score: bestScore.toFixed(1) })}</span>
                ) : null}
              </div>
              <ScoreProgression activeIteration={activeIteration} candidates={detailCandidates} />
            </div>
          </CardContent>
        </Card>

        <div className="ar-detail-meta-line">
          <span>{shortWorkspace(run.workspaceDir) || t('Workspace not set')}</span>
          <span className="ar-meta-dot" />
          <span>{t('Started {time}', { time: formatTimestamp(run.createdAt) })}</span>
          <span className="ar-meta-dot" />
          <span>{t('Updated {time}', { time: formatTimestamp(run.updatedAt) })}</span>
        </div>

        <div className="ar-iterations-head">
          <div>
            <h2>
              {t('Iterations')}
              <span>{displayIterations.length}</span>
            </h2>
          </div>
          <div className="ar-iterations-order">
            <ArrowDownUp />
            {t('Latest first')}
          </div>
        </div>

        {displayIterations.length ? (
          <Table className="ar-iterations-table">
            <TableHeader>
              <TableRow>
                <TableHead className="ar-iteration-table-index">{t('Iteration')}</TableHead>
                <TableHead>{t('Candidate output')}</TableHead>
                <TableHead className="ar-iteration-table-score">{t('Score')}</TableHead>
                <TableHead className="ar-iteration-table-action">{t('Actions')}</TableHead>
                <TableHead className="ar-iteration-table-expand" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {displayIterations.map((iteration) => {
                const candidate = candidateByIteration.get(iteration.iterationIndex) ?? null;
                const score = verdictScore(candidate?.verdict);
                const expanded = expandedIterationIndex === iteration.iterationIndex;
                const running = iteration.state !== 'completed';
                const isSelected = Boolean(candidate && selectedCandidateId === candidate.candidate_id);
                const isBest = Boolean(candidate && currentBestCandidate?.candidate_id === candidate.candidate_id);
                const canSelect = Boolean(onSelectCandidate && candidate?.verdict && !isSelected);
                const summary = firstNonEmptyLine(candidate?.output ?? '')
                  || verdictSummary(candidate?.verdict)
                  || (running ? t('This round is still running…') : t('No candidate output yet'));

                return (
                  <Fragment key={`${iteration.runId}:${iteration.iterationIndex}`}>
                    <TableRow
                      className={cn(
                        'ar-iteration-table-row',
                        expanded ? 'is-expanded' : null,
                        running ? 'is-running' : null,
                        isBest ? 'is-best' : null,
                        isSelected ? 'is-selected' : null,
                      )}
                      onClick={() => {
                        if (!running || candidate) {
                          setExpandedIterationIndex((current) => (
                            current === iteration.iterationIndex ? null : iteration.iterationIndex
                          ));
                        }
                      }}
                      onKeyDown={(event) => {
                        if (event.key === 'Enter' || event.key === ' ') {
                          event.preventDefault();
                          if (!running || candidate) {
                            setExpandedIterationIndex((current) => (
                              current === iteration.iterationIndex ? null : iteration.iterationIndex
                            ));
                          }
                        }
                      }}
                      role="button"
                      tabIndex={0}
                    >
                      <TableCell className="ar-iteration-index-cell">
                        <span className="ar-iteration-num">i{iteration.iterationIndex}</span>
                      </TableCell>
                      <TableCell className="ar-iteration-summary-cell">
                        <span className="ar-iteration-summary">
                          <span>{summary}</span>
                          {isSelected ? <Badge className="ar-selected-tag" variant="secondary">{t('Winner')}</Badge> : null}
                          {!isSelected && isBest && score != null ? <Badge className="ar-best-tag" variant="secondary">{t('Current best')}</Badge> : null}
                        </span>
                      </TableCell>
                      <TableCell>
                        <span className="ar-iteration-score" style={score != null ? { '--ar-score-color': scoreColor(score) } as CSSProperties : undefined}>
                          <span className="ar-iteration-score-value">{score != null ? score.toFixed(1) : '--'}</span>
                          <span className="ar-iteration-score-bar">
                            <span style={{ width: score != null ? `${pct(score, 10)}%` : '0%' }} />
                          </span>
                        </span>
                      </TableCell>
                      <TableCell className="ar-iteration-action-cell">
                        {running ? (
                          <Badge className="ar-running-tag" variant="secondary">
                            <LoaderCircle className="ar-spin" />
                            {formatRunStateLabel(iteration.state)}
                          </Badge>
                        ) : isSelected ? (
                          <Badge className="ar-winner-selected" variant="secondary">
                            <Check />
                            {t('Winner selected')}
                          </Badge>
                        ) : canSelect ? (
                          <Button
                            className={cn('ar-winner-button', isBest ? 'is-primary' : null)}
                            disabled={saving}
                            onClick={(event) => {
                              event.stopPropagation();
                              if (candidate) {
                                void onSelectCandidate?.(run.runId, candidate.candidate_id);
                              }
                            }}
                            size="sm"
                            variant={isBest ? 'default' : 'ghost'}
                          >
                            <Check />
                            {t('Select winner')}
                          </Button>
                        ) : (
                          <span className="ar-no-action">{t('Pending verdict')}</span>
                        )}
                      </TableCell>
                      <TableCell className="ar-iteration-chevron-cell">
                        <ChevronRight className="ar-iteration-chevron" />
                      </TableCell>
                    </TableRow>
                    {expanded ? (
                      <TableRow className="ar-iteration-expanded-row">
                        <TableCell className="ar-iteration-expanded-cell" colSpan={5}>
                          <CandidatePanel candidate={candidate} iteration={iteration} onOpenThread={onOpenThread} />
                        </TableCell>
                      </TableRow>
                    ) : null}
                  </Fragment>
                );
              })}
            </TableBody>
          </Table>
        ) : (
          <div className="ar-empty-iterations">
            {t('The run has not produced iteration records yet.')}
          </div>
        )}

        {createDialogOpen ? (
          <CreateRunDialog defaultWorkspacePath={defaultWorkspacePath} onClose={() => setCreateDialogOpen(false)} onSubmit={handleCreateRun} saving={saving} workspaces={workspaces} />
        ) : null}
      </div>
    );
  }

  return (
    <div className="agents-hub ar-runs-page">
      <div className="agents-hub-hero ar-runs-hero">
        <span className="ar-page-title">{t('Auto Research')}</span>
        <div className="agents-hub-controls">
          <Button onClick={() => setCreateDialogOpen(true)} size="sm">
            <Plus />
            {t('New Run')}
          </Button>
        </div>
      </div>

      {loading && !runs.length ? (
        <div className="agents-hub-empty-state">{t('Loading runs...')}</div>
      ) : runs.length ? (
        <Card className="ar-runs-table-card">
          <Table className="ar-runs-table">
            <TableHeader>
              <TableRow>
                <TableHead className="ar-run-goal-col">{t('Goal')}</TableHead>
                <TableHead className="ar-run-state-col">{t('State')}</TableHead>
                <TableHead className="ar-run-iterations-col">{t('Iterations')}</TableHead>
                <TableHead className="ar-run-updated-col">{t('Updated')}</TableHead>
                <TableHead className="ar-run-actions-col">{t('Actions')}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {runs.map((run) => {
                const runBest = bestCandidate(run.candidates, run.selectedCandidate);
                const runBestScore = verdictScore(runBest?.verdict);
                const workspace = shortWorkspace(run.workspaceDir) || t('Workspace not set');

                return (
                  <TableRow className="ar-run-row" key={run.runId} onClick={() => handleSelectRun(run.runId)}>
                    <TableCell className="ar-run-goal-cell">
                      <div className="ar-run-goal-title">{firstNonEmptyLine(run.goal) || run.goal}</div>
                      <div className="ar-run-goal-meta">
                        <span>{workspace}</span>
                        <span className="ar-meta-dot" />
                        <span>{run.runId}</span>
                      </div>
                    </TableCell>
                    <TableCell>
                      <RunStateBadge active={!isTerminalRunState(run.state) && run.state !== 'queued'} state={run.state} />
                    </TableCell>
                    <TableCell>
                      <div className="ar-run-iterations">
                        <span>{run.iterationsUsed}/{run.maxIterations}</span>
                        {runBestScore != null ? (
                          <span>{t('best {score}', { score: runBestScore.toFixed(1) })}</span>
                        ) : null}
                      </div>
                    </TableCell>
                    <TableCell className="ar-run-updated">
                      {formatCompactTimestamp(run.updatedAt)}
                    </TableCell>
                    <TableCell className="ar-run-actions-col">
                      <div className="ar-run-actions">
                        {!isTerminalRunState(run.state) ? (
                          <Button
                            disabled={saving}
                            onClick={(e) => { e.stopPropagation(); void onStop(run.runId); }}
                            size="xs"
                            variant="outline"
                          >
                            <Square />
                            {t('Stop')}
                          </Button>
                        ) : null}
                        <Button
                          aria-label={t('Delete')}
                          className="ar-run-delete-button"
                          disabled={saving}
                          onClick={(e) => { e.stopPropagation(); void onDelete(run.runId); }}
                          size="icon-xs"
                          title={t('Delete')}
                          variant="ghost"
                        >
                          <Trash />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </Card>
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
