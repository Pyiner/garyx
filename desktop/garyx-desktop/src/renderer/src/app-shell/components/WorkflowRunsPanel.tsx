import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ArrowLeft, Maximize2, MessageSquare, RefreshCcw, X } from 'lucide-react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

import type {
  DesktopTaskSummary,
  DesktopWorkflowChild,
  DesktopWorkflowRunDrilldown,
  DesktopWorkflowRunStatus,
} from '@shared/contracts';

import { getDesktopApi } from '../../platform/desktop-api';
import type { Translate } from '../../i18n';
import type { ToastTone } from '../../toast';
import { AgentAvatar } from './AgentAvatar';

const TERMINAL_STATUSES = new Set(['succeeded', 'failed', 'cancelled', 'skipped']);
const POLL_INTERVAL_MS = 4000;

type WorkflowRunsPanelProps = {
  task?: DesktopTaskSummary | null;
  taskId: string;
  onOpenTasks?: () => void;
  onOpenThread: (threadId: string) => void;
  onToast: (message: string, tone?: ToastTone) => void;
  t: Translate;
};

function isTerminal(status: DesktopWorkflowRunStatus): boolean {
  return TERMINAL_STATUSES.has(status);
}

function statusToneClass(status: DesktopWorkflowRunStatus): string {
  switch (status) {
    case 'succeeded':
      return 'status-succeeded';
    case 'failed':
      return 'status-failed';
    case 'cancelled':
      return 'status-cancelled';
    case 'running':
      return 'status-running';
    default:
      return 'status-pending';
  }
}

function formatRelativeTime(
  value: string | null | undefined,
  t: Translate,
): string {
  if (!value) {
    return '';
  }
  const date = new Date(value);
  const time = date.getTime();
  if (Number.isNaN(time)) {
    return '';
  }
  const diffMs = Date.now() - time;
  const diffSec = Math.round(diffMs / 1000);
  if (diffSec < 5) {
    return t('just now');
  }
  if (diffSec < 60) {
    return t('{count}s ago', { count: diffSec });
  }
  const diffMin = Math.round(diffSec / 60);
  if (diffMin < 60) {
    return t('{count}m ago', { count: diffMin });
  }
  const diffHr = Math.round(diffMin / 60);
  if (diffHr < 24) {
    return t('{count}h ago', { count: diffHr });
  }
  return new Intl.DateTimeFormat(undefined, {
    month: 'short',
    day: 'numeric',
  }).format(date);
}

function formatTokens(value: number): string {
  if (!Number.isFinite(value) || value <= 0) {
    return '0';
  }
  if (value >= 1000) {
    return `${(value / 1000).toFixed(value >= 10000 ? 0 : 1)}k`;
  }
  return String(value);
}

function formatTokenUsage(
  inputTokens: number,
  outputTokens: number,
  t: Translate,
): string | null {
  const hasInput = Number.isFinite(inputTokens) && inputTokens > 0;
  const hasOutput = Number.isFinite(outputTokens) && outputTokens > 0;
  if (!hasInput && !hasOutput) {
    return null;
  }
  return t('{input} in / {output} out', {
    input: formatTokens(inputTokens),
    output: formatTokens(outputTokens),
  });
}

function formatCost(value: number): string {
  if (!Number.isFinite(value) || value <= 0) {
    return '$0.00';
  }
  return `$${value.toFixed(value < 0.1 ? 4 : 2)}`;
}

function formatDetailBlock(value: unknown): string {
  if (value === null || value === undefined) {
    return '';
  }
  const text =
    typeof value === 'string'
      ? value
      : (() => {
          try {
            return JSON.stringify(value, null, 2);
          } catch {
            return String(value);
          }
        })();
  return text;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return Boolean(value && typeof value === 'object' && !Array.isArray(value));
}

function parseInspectableValue(value: unknown): {
  raw: string;
  value: unknown;
  structured: boolean;
} {
  if (value === undefined) {
    return { raw: '', value: null, structured: false };
  }

  if (value === null) {
    return { raw: 'null', value: null, structured: false };
  }

  if (typeof value === 'string') {
    const trimmed = value.trim();
    if (
      (trimmed.startsWith('{') && trimmed.endsWith('}')) ||
      (trimmed.startsWith('[') && trimmed.endsWith(']'))
    ) {
      try {
        const parsed = JSON.parse(trimmed);
        return {
          raw: JSON.stringify(parsed, null, 2),
          value: parsed,
          structured: true,
        };
      } catch {
        // Fall through to plain text.
      }
    }
    return { raw: value, value, structured: false };
  }

  try {
    return {
      raw: JSON.stringify(value, null, 2),
      value,
      structured: isPlainObject(value) || Array.isArray(value),
    };
  } catch {
    return { raw: String(value), value: String(value), structured: false };
  }
}

type InspectableValue = ReturnType<typeof parseInspectableValue>;

function valueKindLabel(value: unknown, t: Translate): string {
  if (Array.isArray(value)) {
    return t('{count} items', { count: value.length });
  }
  if (isPlainObject(value)) {
    const count = Object.keys(value).length;
    return t('{count} fields', { count });
  }
  if (value === null) {
    return 'null';
  }
  return typeof value;
}

function objectSummary(value: Record<string, unknown>): string {
  const summaryKeys = ['title', 'name', 'label', 'status', 'relevance', 'url'];
  return summaryKeys
    .map((key) => {
      const field = value[key];
      return typeof field === 'string' && field.trim() ? field.trim() : '';
    })
    .filter(Boolean)
    .slice(0, 2)
    .join(' · ');
}

function looksMarkdownLike(value: string): boolean {
  return /(^|\n)(#{1,6}\s|[-*]\s|\d+\.\s|>\s|```|\|.+\||\[.+\]\(.+\))/.test(
    value,
  );
}

function truncateOneLine(value: string, maxLength = 220): string {
  const normalized = value.replace(/\s+/g, ' ').trim();
  if (normalized.length <= maxLength) {
    return normalized;
  }
  return `${normalized.slice(0, maxLength - 1)}…`;
}

function PrimitiveResultValue({ value }: { value: unknown }) {
  if (value === null) {
    return <span className="workflow-json-null">null</span>;
  }
  if (typeof value === 'boolean') {
    return <span className="workflow-json-boolean">{value ? 'true' : 'false'}</span>;
  }
  if (typeof value === 'number') {
    return <span className="workflow-json-number">{value}</span>;
  }
  return <span className="workflow-json-primitive">{String(value)}</span>;
}

function StringResultValue({
  label,
  onOpenValue,
  value,
  t,
}: {
  label: string;
  onOpenValue?: (label: string, value: unknown) => void;
  value: string;
  t: Translate;
}) {
  const shouldExpand = value.length > 180 || value.includes('\n') || looksMarkdownLike(value);

  if (!shouldExpand) {
    return <span className="workflow-json-string">"{value}"</span>;
  }

  return (
    <div className="workflow-string-result">
      <button
        className="workflow-string-preview"
        onClick={() => onOpenValue?.(label, value)}
        type="button"
      >
        <span>{truncateOneLine(value)}</span>
        <span className="workflow-string-preview-action">{t('Open')}</span>
      </button>
    </div>
  );
}

function InspectableResultValue({
  label,
  onOpenValue,
  value,
  depth = 0,
  t,
}: {
  label: string;
  onOpenValue?: (label: string, value: unknown) => void;
  value: unknown;
  depth?: number;
  t: Translate;
}) {
  if (typeof value === 'string') {
    return (
      <StringResultValue
        label={label}
        onOpenValue={onOpenValue}
        value={value}
        t={t}
      />
    );
  }

  if (Array.isArray(value)) {
    return (
      <details className="workflow-json-node" open={depth < 2}>
        <summary>
          <span className="workflow-json-node-label">{label}</span>
          <span className="workflow-json-node-meta">{valueKindLabel(value, t)}</span>
        </summary>
        <div className="workflow-json-children">
          {value.map((item, index) => {
            const summary = isPlainObject(item) ? objectSummary(item) : '';
            return (
              <div className="workflow-json-row" key={`${label}-${index}`}>
                <span className="workflow-json-key">
                  {index}
                  {summary ? <span className="workflow-json-row-summary">{summary}</span> : null}
                </span>
                <div className="workflow-json-value">
                  <InspectableResultValue
                    depth={depth + 1}
                    label={`${label}[${index}]`}
                    onOpenValue={onOpenValue}
                    t={t}
                    value={item}
                  />
                </div>
              </div>
            );
          })}
        </div>
      </details>
    );
  }

  if (isPlainObject(value)) {
    const entries = Object.entries(value);
    return (
      <details className="workflow-json-node" open={depth < 2}>
        <summary>
          <span className="workflow-json-node-label">{label}</span>
          <span className="workflow-json-node-meta">
            {objectSummary(value) || valueKindLabel(value, t)}
          </span>
        </summary>
        <div className="workflow-json-children">
          {entries.map(([key, entryValue]) => (
            <div className="workflow-json-row" key={key}>
              <span className="workflow-json-key">{key}</span>
              <div className="workflow-json-value">
                <InspectableResultValue
                  depth={depth + 1}
                  label={key}
                  onOpenValue={onOpenValue}
                  t={t}
                  value={entryValue}
                />
              </div>
            </div>
          ))}
        </div>
      </details>
    );
  }

  return <PrimitiveResultValue value={value} />;
}

function ResultReadableContent({
  onOpenValue,
  parsed,
  t,
}: {
  onOpenValue?: (label: string, value: unknown) => void;
  parsed: InspectableValue;
  t: Translate;
}) {
  if (parsed.structured) {
    return (
      <div className="workflow-json-inspector">
        <InspectableResultValue
          label={t('Result')}
          onOpenValue={onOpenValue}
          t={t}
          value={parsed.value}
        />
      </div>
    );
  }

  if (typeof parsed.value === 'string') {
    return (
      <div className="workflow-result-markdown">
        <ReactMarkdown remarkPlugins={[remarkGfm]}>{parsed.value}</ReactMarkdown>
      </div>
    );
  }

  return (
    <div className="workflow-json-inspector">
      <InspectableResultValue
        label={t('Result')}
        onOpenValue={onOpenValue}
        t={t}
        value={parsed.value}
      />
    </div>
  );
}

function ResultValueDialog({
  entry,
  onClose,
  t,
}: {
  entry: { title: string; value: unknown } | null;
  onClose: () => void;
  t: Translate;
}) {
  const [mode, setMode] = useState<'readable' | 'raw'>('readable');
  const [activeEntry, setActiveEntry] = useState(entry);
  const parsed = useMemo(
    () => parseInspectableValue(activeEntry?.value),
    [activeEntry?.value],
  );

  useEffect(() => {
    if (!entry) {
      return undefined;
    }
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        onClose();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [entry, onClose]);

  useEffect(() => {
    setActiveEntry(entry);
    setMode('readable');
  }, [entry]);

  if (!entry || !activeEntry || !parsed.raw) {
    return null;
  }

  return (
    <div
      className="workflow-result-dialog-backdrop"
      onMouseDown={onClose}
      role="presentation"
    >
      <section
        aria-label={t('Workflow result detail')}
        className="workflow-result-dialog"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header className="workflow-result-dialog-header">
          <div className="workflow-result-dialog-title">
            <span>{t('Agent result')}</span>
            <h3>{activeEntry.title}</h3>
            <p>{parsed.structured ? valueKindLabel(parsed.value, t) : t('Text')}</p>
          </div>
          <div className="workflow-result-dialog-actions">
            <span className="workflow-result-viewer-tabs">
              <button
                className={mode === 'readable' ? 'is-active' : ''}
                onClick={() => setMode('readable')}
                type="button"
              >
                {t('Readable')}
              </button>
              <button
                className={mode === 'raw' ? 'is-active' : ''}
                onClick={() => setMode('raw')}
                type="button"
              >
                {t('Raw')}
              </button>
            </span>
            <button
              className="workflow-result-dialog-close"
              onClick={onClose}
              title={t('Close')}
              type="button"
            >
              <X aria-hidden size={17} strokeWidth={1.8} />
            </button>
          </div>
        </header>
        <div className="workflow-result-dialog-body">
          {mode === 'raw' ? (
            <pre className="workflow-result-raw">{parsed.raw}</pre>
          ) : (
            <ResultReadableContent
              onOpenValue={(label, value) => {
                setActiveEntry({ title: label, value });
                setMode('readable');
              }}
              parsed={parsed}
              t={t}
            />
          )}
        </div>
      </section>
    </div>
  );
}

function StructuredResultViewer({
  value,
  fallback,
  t,
}: {
  value: unknown;
  fallback?: unknown;
  t: Translate;
}) {
  const inspectableValue = value === undefined ? fallback : value;
  const parsed = useMemo(
    () => parseInspectableValue(inspectableValue),
    [inspectableValue],
  );
  const [mode, setMode] = useState<'readable' | 'raw'>('readable');
  const [dialogEntry, setDialogEntry] = useState<{
    title: string;
    value: unknown;
  } | null>(null);

  if (!parsed.raw) {
    return null;
  }

  return (
    <div className="workflow-result-viewer">
      <div className="workflow-result-viewer-toolbar">
        <span className="workflow-result-viewer-kind">
          {parsed.structured ? valueKindLabel(parsed.value, t) : t('Text')}
        </span>
        <span className="workflow-result-viewer-actions">
          <span className="workflow-result-viewer-tabs">
            <button
              className={mode === 'readable' ? 'is-active' : ''}
              onClick={() => setMode('readable')}
              type="button"
            >
              {t('Readable')}
            </button>
            <button
              className={mode === 'raw' ? 'is-active' : ''}
              onClick={() => setMode('raw')}
              type="button"
            >
              {t('Raw')}
            </button>
          </span>
          <button
            className="workflow-result-viewer-open"
            onClick={() =>
              setDialogEntry({ title: t('Agent result'), value: inspectableValue })
            }
            title={t('Open large view')}
            type="button"
          >
            <Maximize2 aria-hidden size={13} strokeWidth={1.8} />
          </button>
        </span>
      </div>
      {mode === 'raw' ? (
        <pre className="workflow-result-raw">{parsed.raw}</pre>
      ) : (
        <ResultReadableContent
          onOpenValue={(label, nextValue) =>
            setDialogEntry({ title: label, value: nextValue })
          }
          parsed={parsed}
          t={t}
        />
      )}
      <ResultValueDialog
        entry={dialogEntry}
        onClose={() => setDialogEntry(null)}
        t={t}
      />
    </div>
  );
}

function StatusPill({
  status,
  t,
}: {
  status: DesktopWorkflowRunStatus;
  t: Translate;
}) {
  return (
    <span className={`workflow-status-pill ${statusToneClass(status)}`}>
      {t(status)}
    </span>
  );
}

type WorkflowPhase = {
  key: string;
  index: number | null;
  title: string;
  detail?: string;
  children: DesktopWorkflowChild[];
  status: DesktopWorkflowRunStatus;
  completed: number;
};

type WorkflowPhasePlan = {
  key: string;
  index: number;
  title: string;
  detail?: string;
};

function childDisplayName(child: DesktopWorkflowChild): string {
  return child.label || child.agentId || child.workflowChildRunId;
}

function childAgentDisplayName(child: DesktopWorkflowChild): string {
  return child.agentId || child.label || 'Agent';
}

function childSortTime(child: DesktopWorkflowChild): number {
  const raw = child.startedAt || child.queuedAt || child.updatedAt || '';
  const value = new Date(raw).getTime();
  return Number.isFinite(value) ? value : 0;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function asString(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value.trim() : null;
}

function asNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function workflowPhasePlan(
  workflow: DesktopWorkflowRunDrilldown['workflow'],
): WorkflowPhasePlan[] {
  const phases = Array.isArray(workflow.meta?.phases)
    ? workflow.meta.phases
    : [];
  return phases
    .map((entry, fallbackIndex) => {
      const record = asRecord(entry);
      if (!record) {
        return null;
      }
      const title = asString(record.title);
      if (!title) {
        return null;
      }
      const index = asNumber(record.index) ?? fallbackIndex;
      const detail = asString(record.detail) ?? undefined;
      return {
        key: `${index}:${title}`,
        index,
        title,
        ...(detail ? { detail } : {}),
      } satisfies WorkflowPhasePlan;
    })
    .filter((entry): entry is WorkflowPhasePlan => Boolean(entry))
    .sort((left, right) => left.index - right.index);
}

function phaseSortTime(phase: WorkflowPhase): number {
  const first = phase.children[0];
  return first ? childSortTime(first) : 0;
}

function buildWorkflowPhases(
  workflow: DesktopWorkflowRunDrilldown['workflow'],
  children: DesktopWorkflowChild[],
  t: Translate,
): WorkflowPhase[] {
  const grouped = new Map<string, WorkflowPhase>();
  const phaseByIndex = new Map<number, WorkflowPhase>();
  const phaseByTitle = new Map<string, WorkflowPhase>();

  for (const planned of workflowPhasePlan(workflow)) {
    const phase: WorkflowPhase = {
      key: planned.key,
      index: planned.index,
      title: planned.title,
      detail: planned.detail,
      children: [],
      status: 'queued',
      completed: 0,
    };
    grouped.set(phase.key, phase);
    phaseByIndex.set(planned.index, phase);
    phaseByTitle.set(planned.title, phase);
  }

  for (const child of children) {
    const index = child.phaseIndex ?? null;
    const title =
      child.phaseTitle ||
      (index === null
        ? t('Run')
        : t('Phase {number}', { number: index + 1 }));
    const key = `${index ?? 'none'}:${title}`;
    const existing =
      (index === null ? undefined : phaseByIndex.get(index)) ||
      phaseByTitle.get(title) ||
      grouped.get(key);
    const phase =
      existing ||
      ({
        key,
        index,
        title,
        children: [],
        status: 'queued',
        completed: 0,
      } satisfies WorkflowPhase);
    phase.children.push(child);
    grouped.set(key, phase);
    if (index !== null) {
      phaseByIndex.set(index, phase);
    }
    phaseByTitle.set(title, phase);
  }

  const phases = [...grouped.values()].sort((left, right) => {
    const leftIndex = left.index ?? Number.MAX_SAFE_INTEGER;
    const rightIndex = right.index ?? Number.MAX_SAFE_INTEGER;
    if (leftIndex !== rightIndex) {
      return leftIndex - rightIndex;
    }
    return (
      phaseSortTime(left) - phaseSortTime(right)
    );
  });

  for (const phase of phases) {
    phase.children.sort((left, right) => childSortTime(left) - childSortTime(right));
    phase.completed = phase.children.filter((child) => isTerminal(child.status)).length;
    if (phase.children.some((child) => child.status === 'running')) {
      phase.status = 'running';
    } else if (phase.children.some((child) => child.status === 'failed')) {
      phase.status = 'failed';
    } else if (phase.children.some((child) => child.status === 'cancelled')) {
      phase.status = 'cancelled';
    } else if (phase.children.length && phase.completed === phase.children.length) {
      phase.status = 'succeeded';
    } else {
      phase.status = 'queued';
    }
  }

  return phases;
}

function preferredPhaseKey(
  phases: WorkflowPhase[],
  workflow: DesktopWorkflowRunDrilldown['workflow'],
): string {
  const current = phases.find(
    (phase) =>
      workflow.currentPhaseIndex !== null &&
      workflow.currentPhaseIndex !== undefined &&
      phase.index === workflow.currentPhaseIndex,
  );
  return (
    current?.key ||
    phases.find((phase) => phase.status === 'running')?.key ||
    phases.find((phase) => phase.completed < phase.children.length)?.key ||
    phases[0]?.key ||
    ''
  );
}

function selectedChildForPhase(
  phase: WorkflowPhase | null,
): DesktopWorkflowChild | null {
  return (
    phase?.children.find((child) => child.status === 'running') ||
    phase?.children[0] ||
    null
  );
}

function childOutcomeValue(child: DesktopWorkflowChild): unknown {
  if (child.result !== null && child.result !== undefined) {
    return child.result;
  }
  if (child.resultText !== null && child.resultText !== undefined) {
    return child.resultText;
  }
  if (child.resultPreview !== null && child.resultPreview !== undefined) {
    return child.resultPreview;
  }
  return isTerminal(child.status) ? null : undefined;
}

function ChildResultPanel({
  child,
  onOpenThread,
  t,
}: {
  child: DesktopWorkflowChild | null;
  onOpenThread: (threadId: string) => void;
  t: Translate;
}) {
  if (!child) {
    return (
      <aside className="workflow-result-column workflow-result-empty">
        {t('Select an agent run to inspect the result.')}
      </aside>
    );
  }

  const prompt = formatDetailBlock(child.prompt);
  const outcome = childOutcomeValue(child);
  const hasOutcome = outcome !== undefined;
  const tokenUsage = formatTokenUsage(child.inputTokens, child.outputTokens, t);

  return (
    <aside className="workflow-result-column">
      <div className="workflow-result-head">
        <div className="workflow-result-title-row">
          <span className="workflow-agent-avatar">
            <AgentAvatar
              agentId={child.agentId || child.workflowChildRunId}
              displayName={childAgentDisplayName(child)}
              role="member"
              size={22}
            />
          </span>
          <div className="workflow-result-title-block">
            <span className="workflow-result-title" title={childDisplayName(child)}>
              {childDisplayName(child)}
            </span>
            <span className="workflow-result-meta">
              {[child.phaseTitle, child.agentId, child.resultMode]
                .filter(Boolean)
                .join(' · ')}
            </span>
          </div>
        </div>
        <StatusPill status={child.status} t={t} />
      </div>

      <div className="workflow-result-stats">
        {tokenUsage ? <span>{tokenUsage}</span> : null}
        {child.toolCalls > 0 ? (
          <span>{t('{count} tools', { count: child.toolCalls })}</span>
        ) : null}
        {child.costUsd > 0 ? <span>{formatCost(child.costUsd)}</span> : null}
        {child.threadId ? (
          <button
            className="workflow-child-open"
            onClick={() => onOpenThread(child.threadId as string)}
            title={t('Open thread')}
            type="button"
          >
            <MessageSquare aria-hidden size={13} strokeWidth={1.8} />
          </button>
        ) : null}
      </div>

      {prompt ? (
        <section className="workflow-result-section">
          <h4>{t('Prompt')}</h4>
          <pre>{prompt}</pre>
        </section>
      ) : null}

      <section className="workflow-result-section">
        <div className="workflow-result-section-header">
          <h4>{t('Agent result')}</h4>
        </div>
        {child.error ? (
          <p className="workflow-child-error">{child.error}</p>
        ) : hasOutcome ? (
          <StructuredResultViewer t={t} value={outcome} />
        ) : (
          <p>{t('Still running…')}</p>
        )}
      </section>
    </aside>
  );
}

function RunCard({
  run,
  onOpenThread,
  t,
}: {
  run: DesktopWorkflowRunDrilldown;
  onOpenThread: (threadId: string) => void;
  t: Translate;
}) {
  const { workflow, children } = run;
  const phases = useMemo(
    () => buildWorkflowPhases(workflow, children, t),
    [workflow, children, t],
  );
  const [selectedPhaseKey, setSelectedPhaseKey] = useState(() =>
    preferredPhaseKey(phases, workflow),
  );
  const activePhase =
    phases.find((phase) => phase.key === selectedPhaseKey) || phases[0] || null;
  const [selectedChildId, setSelectedChildId] = useState(() => {
    const child = selectedChildForPhase(activePhase);
    return child?.workflowChildRunId || '';
  });
  const selectedChild =
    activePhase?.children.find(
      (child) => child.workflowChildRunId === selectedChildId,
    ) ||
    selectedChildForPhase(activePhase);
  const runOutcome = workflow.summary || '';

  useEffect(() => {
    const preferred = preferredPhaseKey(phases, workflow);
    if (!selectedPhaseKey || !phases.some((phase) => phase.key === selectedPhaseKey)) {
      setSelectedPhaseKey(preferred);
    }
  }, [phases, workflow, selectedPhaseKey]);

  useEffect(() => {
    if (!activePhase) {
      return;
    }
    if (
      selectedChildId &&
      activePhase.children.some(
        (child) => child.workflowChildRunId === selectedChildId,
      )
    ) {
      return;
    }
    setSelectedChildId(selectedChildForPhase(activePhase)?.workflowChildRunId || '');
  }, [activePhase, selectedChildId]);

  return (
    <section className="workflow-run-card">
      {workflow.error ? (
        <p className="workflow-run-error">{workflow.error}</p>
      ) : null}

      {phases.length ? (
        <div className="workflow-run-console">
          <nav className="workflow-phase-column" aria-label={t('Workflow phases')}>
            <div className="workflow-console-label">{t('Phases')}</div>
            {phases.map((phase, index) => (
              <button
                className={`workflow-phase-item ${
                  phase.key === activePhase?.key ? 'is-active' : ''
                }`}
                key={phase.key}
                onClick={() => {
                  setSelectedPhaseKey(phase.key);
                  setSelectedChildId(
                    selectedChildForPhase(phase)?.workflowChildRunId || '',
                  );
                }}
                type="button"
              >
                <span className="workflow-phase-main">
                  <span className="workflow-phase-title">
                    {phase.index === null ? index + 1 : phase.index + 1}.{' '}
                    {phase.title}
                  </span>
                  <span className="workflow-phase-count">
                    {phase.completed}/{phase.children.length}
                  </span>
                </span>
              </button>
            ))}
          </nav>

          <section className="workflow-agent-column">
            <div className="workflow-agent-column-head">
              <span className="workflow-console-label">
                {activePhase
                  ? t('{phase} · {count} agents', {
                      phase: activePhase.title,
                      count: activePhase.children.length,
                    })
                  : t('Agents')}
              </span>
              {activePhase ? <StatusPill status={activePhase.status} t={t} /> : null}
            </div>
            <div className="workflow-agent-list">
              {activePhase?.children.map((child) => {
                const label = childDisplayName(child);
                const tokenUsage = formatTokenUsage(
                  child.inputTokens,
                  child.outputTokens,
                  t,
                );
                const childCost =
                  child.costUsd > 0 ? formatCost(child.costUsd) : null;
                const agentDisplayName = childAgentDisplayName(child);
                return (
                  <button
                    className={`workflow-agent-item ${
                      child.workflowChildRunId === selectedChild?.workflowChildRunId
                        ? 'is-active'
                        : ''
                    }`}
                    key={child.workflowChildRunId}
                    onClick={() => setSelectedChildId(child.workflowChildRunId)}
                    type="button"
                  >
                    <span className="workflow-agent-avatar">
                      <AgentAvatar
                        agentId={child.agentId || child.workflowChildRunId}
                        displayName={agentDisplayName}
                        role="member"
                        size={22}
                      />
                    </span>
                    <span className="workflow-agent-main">
                      <span className="workflow-agent-title" title={label}>
                        {label}
                      </span>
                      <span className="workflow-agent-meta">
                        {[
                          child.agentId,
                          tokenUsage,
                          childCost,
                          child.toolCalls > 0
                            ? t('{count} tools', { count: child.toolCalls })
                            : null,
                        ]
                          .filter(Boolean)
                          .join(' · ')}
                      </span>
                    </span>
                    <StatusPill status={child.status} t={t} />
                  </button>
                );
              })}
            </div>
          </section>

          <ChildResultPanel
            child={selectedChild}
            onOpenThread={onOpenThread}
            t={t}
          />
        </div>
      ) : runOutcome ? (
        <section className="workflow-result-section workflow-run-result-fallback">
          <h4>{t('Workflow result')}</h4>
          <pre>{runOutcome}</pre>
        </section>
      ) : null}

    </section>
  );
}

export function WorkflowRunsPanel({
  task,
  taskId,
  onOpenTasks,
  onOpenThread,
  onToast,
  t,
}: WorkflowRunsPanelProps) {
  const [runs, setRuns] = useState<DesktopWorkflowRunDrilldown[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const hasNonTerminal = runs.some((run) => !isTerminal(run.workflow.status));
  const shouldPoll =
    hasNonTerminal || (runs.length === 0 && task?.status === 'in_progress');
  const mountedRef = useRef(true);
  const taskLabel = task?.taskId || taskId;
  const primaryWorkflow = runs[0]?.workflow || null;
  const headerTitle = primaryWorkflow?.name || t('Workflow run');
  const headerMeta = [
    taskLabel,
    primaryWorkflow
      ? t('{completed}/{total} children', {
          completed: primaryWorkflow.completedChildren,
          total: primaryWorkflow.totalChildren,
        })
      : null,
    primaryWorkflow && primaryWorkflow.failedChildren > 0
      ? t('{count} failed', { count: primaryWorkflow.failedChildren })
      : null,
    primaryWorkflow && primaryWorkflow.totalCostUsd > 0
      ? formatCost(primaryWorkflow.totalCostUsd)
      : null,
    primaryWorkflow?.startedAt
      ? t('started {time}', {
          time: formatRelativeTime(primaryWorkflow.startedAt, t),
        })
      : null,
  ].filter(Boolean);

  const load = useCallback(
    async (options?: { silent?: boolean }) => {
      if (!options?.silent) {
        setLoading(true);
      }
      try {
        const page = await getDesktopApi().listTaskWorkflowRuns({
          taskId,
          limit: 50,
        });
        if (!mountedRef.current) {
          return;
        }
        setRuns(page.workflowRuns);
        setError(null);
      } catch (loadError) {
        if (!mountedRef.current) {
          return;
        }
        const message =
          loadError instanceof Error
            ? loadError.message
            : String(loadError || 'Failed to load workflow runs');
        setError(message);
        if (!options?.silent) {
          onToast(message, 'error');
        }
      } finally {
        if (mountedRef.current && !options?.silent) {
          setLoading(false);
        }
      }
    },
    [taskId, onToast],
  );

  useEffect(() => {
    mountedRef.current = true;
    void load();
    return () => {
      mountedRef.current = false;
    };
  }, [load]);

  useEffect(() => {
    if (!shouldPoll) {
      return;
    }
    const handle = window.setInterval(() => {
      void load({ silent: true });
    }, POLL_INTERVAL_MS);
    return () => {
      window.clearInterval(handle);
    };
  }, [shouldPoll, load]);

  return (
    <div className="workflow-runs-page">
      <section
        aria-label={t('Workflow runs')}
        className="workflow-runs-panel"
      >
        <div className="workflow-runs-header">
          <div className="workflow-runs-title-block">
            <div className="workflow-runs-title-row">
              {onOpenTasks ? (
                <button
                  className="workflow-runs-back"
                  onClick={onOpenTasks}
                  type="button"
                >
                  <ArrowLeft aria-hidden size={14} strokeWidth={1.8} />
                  {t('Tasks')}
                </button>
              ) : null}
              <h2>{headerTitle}</h2>
              {primaryWorkflow ? (
                <StatusPill status={primaryWorkflow.status} t={t} />
              ) : null}
            </div>
            {headerMeta.length ? (
              <span className="workflow-runs-subtitle">
                {headerMeta.join(' · ')}
              </span>
            ) : null}
          </div>
          <div className="workflow-runs-header-actions">
            <button
              className="tasks-icon-button"
              disabled={loading}
              onClick={() => {
                void load();
              }}
              title={t('Refresh')}
              type="button"
            >
              <RefreshCcw aria-hidden size={14} strokeWidth={1.8} />
            </button>
          </div>
        </div>

        <div className="workflow-runs-body">
          {loading ? (
            <div className="workflow-runs-state">{t('Loading workflow runs…')}</div>
          ) : error ? (
            <div className="workflow-runs-state workflow-runs-state-error">
              {error}
            </div>
          ) : runs.length ? (
            <div className="workflow-runs-list">
              {runs.map((run) => (
                <RunCard
                  key={run.workflow.workflowRunId}
                  onOpenThread={onOpenThread}
                  run={run}
                  t={t}
                />
              ))}
            </div>
          ) : (
            <div className="workflow-runs-state">
              {t('No workflow runs for this task.')}
            </div>
          )}
        </div>
      </section>
    </div>
  );
}
