import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ArrowLeft, Check, ChevronDown, Circle, Loader2, Maximize2, MessageSquare, RefreshCcw, X } from 'lucide-react';
import ReactMarkdown from 'react-markdown';
import remarkBreaks from 'remark-breaks';
import remarkGfm from 'remark-gfm';

import { Badge } from '@/components/ui/badge';
import { Checkbox } from '@/components/ui/checkbox';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import { cn } from '@/lib/utils';

import type {
  DesktopTaskSummary,
  DesktopWorkflowChild,
  DesktopWorkflowRunDrilldown,
  DesktopWorkflowRunStatus,
} from '@shared/contracts';

import { RichMessageContent } from '../../message-rich-content';
import { getDesktopApi } from '../../platform/desktop-api';
import type { Translate } from '../../i18n';
import type { ToastTone } from '../../toast';
import { AgentAvatar } from './AgentAvatar';

const TERMINAL_STATUSES = new Set(['succeeded', 'failed', 'cancelled', 'skipped']);
const POLL_INTERVAL_MS = 4000;
const VIEW_MODE_STORAGE_KEY = 'garyx.workflowViewMode';
const INLINE_COLLECTION_LIMIT = 14;
const INLINE_COLLECTION_MAX_DEPTH = 2;

function readStoredViewMode(): WorkflowViewMode {
  try {
    const stored = window.localStorage.getItem(VIEW_MODE_STORAGE_KEY);
    return stored === 'console' || stored === 'timeline' ? stored : 'timeline';
  } catch {
    return 'timeline';
  }
}

type WorkflowRunsPanelProps = {
  task?: DesktopTaskSummary | null;
  taskId?: string | null;
  workflowRunId?: string | null;
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

function isWorkflowRunNotFoundError(error: unknown): boolean {
  const message =
    error instanceof Error ? error.message : String(error || '');
  return /workflow run not found|notfound|404/i.test(message);
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

function CollectionPreviewValue({
  label,
  onOpenValue,
  t,
  value,
}: {
  label: string;
  onOpenValue?: (label: string, value: unknown) => void;
  t: Translate;
  value: unknown[] | Record<string, unknown>;
}) {
  const summary = Array.isArray(value)
    ? valueKindLabel(value, t)
    : objectSummary(value) || valueKindLabel(value, t);
  if (!onOpenValue) {
    return (
      <span className="workflow-json-preview-static">
        <span className="workflow-json-preview-label">{label}</span>
        <span className="workflow-json-preview-meta">{summary}</span>
      </span>
    );
  }
  return (
    <button
      className="workflow-json-preview"
      onClick={() => onOpenValue(label, value)}
      type="button"
    >
      <span className="workflow-json-preview-label">{label}</span>
      <span className="workflow-json-preview-meta">{summary}</span>
      <span className="workflow-json-preview-action">{t('Open')}</span>
    </button>
  );
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
    if (depth >= INLINE_COLLECTION_MAX_DEPTH) {
      return (
        <CollectionPreviewValue
          label={label}
          onOpenValue={onOpenValue}
          t={t}
          value={value}
        />
      );
    }
    const visibleItems = value.slice(0, INLINE_COLLECTION_LIMIT);
    const hiddenCount = value.length - visibleItems.length;
    return (
      <details className="workflow-json-node" open={depth < 2}>
        <summary>
          <span className="workflow-json-node-label">{label}</span>
          <span className="workflow-json-node-meta">{valueKindLabel(value, t)}</span>
        </summary>
        <div className="workflow-json-children">
          {visibleItems.map((item, index) => {
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
          {hiddenCount > 0 ? (
            <button
              className="workflow-json-more"
              onClick={() => onOpenValue?.(label, value)}
              type="button"
            >
              {t('{count} more', { count: hiddenCount })}
            </button>
          ) : null}
        </div>
      </details>
    );
  }

  if (isPlainObject(value)) {
    const entries = Object.entries(value);
    if (depth >= INLINE_COLLECTION_MAX_DEPTH) {
      return (
        <CollectionPreviewValue
          label={label}
          onOpenValue={onOpenValue}
          t={t}
          value={value}
        />
      );
    }
    const visibleEntries = entries.slice(0, INLINE_COLLECTION_LIMIT);
    const hiddenCount = entries.length - visibleEntries.length;
    return (
      <details className="workflow-json-node" open={depth < 2}>
        <summary>
          <span className="workflow-json-node-label">{label}</span>
          <span className="workflow-json-node-meta">
            {objectSummary(value) || valueKindLabel(value, t)}
          </span>
        </summary>
        <div className="workflow-json-children">
          {visibleEntries.map(([key, entryValue]) => (
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
          {hiddenCount > 0 ? (
            <button
              className="workflow-json-more"
              onClick={() => onOpenValue?.(label, value)}
              type="button"
            >
              {t('{count} more', { count: hiddenCount })}
            </button>
          ) : null}
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
        <ReactMarkdown remarkPlugins={[remarkGfm, remarkBreaks]}>{parsed.value}</ReactMarkdown>
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

type WorkflowViewMode = 'console' | 'timeline';

function phaseDefaultExpanded(
  phase: WorkflowPhase,
  index: number,
  phases: WorkflowPhase[],
): boolean {
  if (phase.status === 'running') {
    return true;
  }
  if (phases.some((entry) => entry.status === 'running')) {
    return false;
  }
  // Idle/finished run: expand the last phase that actually produced agents so
  // the conversation does not collapse to a wall of empty headers.
  const lastWithChildren = [...phases]
    .reverse()
    .find((entry) => entry.children.length > 0);
  return lastWithChildren
    ? phase.key === lastWithChildren.key
    : index === phases.length - 1;
}

// Semantic state indicator shared by the timeline spine nodes. Running → spinner,
// done → filled check, failed → red cross, not-yet-reached → hollow ring.
function StepNode({ status }: { status: DesktopWorkflowRunStatus }) {
  if (status === 'running') {
    return (
      <Loader2 className="size-[15px] animate-spin text-foreground/65" aria-hidden />
    );
  }
  if (status === 'succeeded') {
    return (
      <span className="flex size-[15px] items-center justify-center rounded-full bg-foreground text-background">
        <Check className="size-2.5" strokeWidth={3.5} aria-hidden />
      </span>
    );
  }
  if (status === 'failed') {
    return (
      <span className="flex size-[15px] items-center justify-center rounded-full bg-destructive text-white">
        <X className="size-2.5" strokeWidth={3.5} aria-hidden />
      </span>
    );
  }
  return <Circle className="size-[15px] text-muted-foreground/35" strokeWidth={2} aria-hidden />;
}

// Plan checklist marker: shadcn Checkbox for reached/done state, spinner for the
// active step, red for a failure. Reads as a real todo list.
function PlanMarker({ status }: { status: DesktopWorkflowRunStatus }) {
  if (status === 'running') {
    return (
      <Loader2 className="size-[18px] animate-spin text-foreground/65" aria-hidden />
    );
  }
  if (status === 'failed') {
    return (
      <span className="flex size-[18px] items-center justify-center rounded-md bg-destructive text-white">
        <X className="size-3" strokeWidth={3} aria-hidden />
      </span>
    );
  }
  return (
    <Checkbox
      aria-hidden
      checked={status === 'succeeded'}
      className="pointer-events-none size-[18px]"
      disabled
      tabIndex={-1}
    />
  );
}

function runStatusBadgeClass(status: DesktopWorkflowRunStatus): string {
  switch (status) {
    case 'running':
      return 'border-transparent bg-[#edf7fb] text-[#2d6987]';
    case 'failed':
      return 'border-transparent bg-destructive/10 text-destructive';
    case 'succeeded':
      return 'border-transparent bg-secondary text-foreground';
    default:
      return 'border-transparent bg-muted text-muted-foreground';
  }
}

function AgentCard({
  child,
  onOpenThread,
  onViewResult,
  t,
}: {
  child: DesktopWorkflowChild;
  onOpenThread: (threadId: string) => void;
  onViewResult: (entry: { title: string; value: unknown }) => void;
  t: Translate;
}) {
  const label = childDisplayName(child);
  const agentName = childAgentDisplayName(child);
  const tokenUsage = formatTokenUsage(child.inputTokens, child.outputTokens, t);
  const cost = child.costUsd > 0 ? formatCost(child.costUsd) : null;
  const outcome = childOutcomeValue(child);
  const hasResult = outcome !== undefined && outcome !== null;
  const threadId = child.threadId;
  const canOpenThread = Boolean(threadId);
  const running = child.status === 'running';
  const failed = child.status === 'failed';
  // Second line: optional, restrained metadata.
  const meta = [
    agentName,
    tokenUsage,
    cost,
    child.toolCalls > 0 ? t('{count} tools', { count: child.toolCalls }) : null,
  ]
    .filter(Boolean)
    .join(' · ');
  const openThread = () => {
    if (threadId) {
      onOpenThread(threadId);
    }
  };
  return (
    <div
      className={cn(
        'group/agent flex items-center gap-3 rounded-lg border border-[#eee] bg-card p-2.5 text-left shadow-[0_1px_2px_rgba(0,0,0,0.03)] transition-colors',
        canOpenThread && 'cursor-pointer hover:border-[#dcdcdc] hover:bg-accent/50',
      )}
      onClick={canOpenThread ? openThread : undefined}
      onKeyDown={
        canOpenThread
          ? (event) => {
              if (event.key === 'Enter' || event.key === ' ') {
                event.preventDefault();
                openThread();
              }
            }
          : undefined
      }
      role={canOpenThread ? 'button' : undefined}
      tabIndex={canOpenThread ? 0 : undefined}
    >
      <span className="shrink-0">
        <AgentAvatar
          agentId={child.agentId || child.workflowChildRunId}
          displayName={agentName}
          role="member"
          size={36}
        />
      </span>
      <div className="min-w-0 flex-1 leading-tight">
        <div
          className={cn(
            'truncate text-[13px] font-medium text-foreground',
            failed && 'text-destructive',
          )}
          title={label}
        >
          {label}
        </div>
        {meta ? (
          <div className="mt-0.5 truncate text-xs text-muted-foreground">
            {meta}
          </div>
        ) : null}
      </div>
      <div className="flex shrink-0 items-center gap-0.5 self-center pl-1">
        {hasResult ? (
          <button
            aria-label={t('View result')}
            className="flex size-7 items-center justify-center rounded-md text-muted-foreground opacity-0 transition-[opacity,background-color] hover:bg-accent hover:text-foreground focus:opacity-100 group-hover/agent:opacity-100"
            onClick={(event) => {
              event.stopPropagation();
              onViewResult({ title: label, value: outcome });
            }}
            title={t('View result')}
            type="button"
          >
            <Maximize2 aria-hidden className="size-3.5" strokeWidth={1.9} />
          </button>
        ) : null}
        {running ? (
          <Loader2
            aria-hidden
            className="size-4 animate-spin text-foreground/55"
            strokeWidth={2.2}
          />
        ) : failed ? (
          <X aria-hidden className="size-4 text-destructive" strokeWidth={2.4} />
        ) : null}
      </div>
    </div>
  );
}

function WorkflowTimelineView({
  phases,
  workflow,
  onOpenThread,
  t,
}: {
  phases: WorkflowPhase[];
  workflow: DesktopWorkflowRunDrilldown['workflow'];
  onOpenThread: (threadId: string) => void;
  t: Translate;
}) {
  const [dialogEntry, setDialogEntry] = useState<{
    title: string;
    value: unknown;
  } | null>(null);
  const [expandedKeys, setExpandedKeys] = useState<Set<string>>(() => {
    const next = new Set<string>();
    phases.forEach((phase, index) => {
      if (phaseDefaultExpanded(phase, index, phases)) {
        next.add(phase.key);
      }
    });
    return next;
  });
  const phaseRefs = useRef<Map<string, HTMLElement>>(new Map());
  // Auto-open a phase the first time polling shows it running, so a live run
  // always reveals the active stage. Each key is expanded once, so a manual
  // collapse afterwards is respected.
  const autoExpandedRef = useRef<Set<string>>(new Set());
  useEffect(() => {
    const fresh = phases
      .filter((phase) => phase.status === 'running')
      .map((phase) => phase.key)
      .filter((key) => !autoExpandedRef.current.has(key));
    if (!fresh.length) {
      return;
    }
    fresh.forEach((key) => autoExpandedRef.current.add(key));
    setExpandedKeys((current) => {
      const next = new Set(current);
      fresh.forEach((key) => next.add(key));
      return next;
    });
  }, [phases]);

  const currentIndex =
    workflow.currentPhaseIndex === null ||
    workflow.currentPhaseIndex === undefined
      ? -1
      : workflow.currentPhaseIndex;
  const streamPhases = phases.filter(
    (phase) =>
      phase.children.length > 0 ||
      phase.status !== 'queued' ||
      (phase.index !== null && phase.index <= currentIndex),
  );
  const visiblePhases = streamPhases.length ? streamPhases : phases.slice(0, 1);
  const showFinal =
    workflow.status === 'succeeded' && Boolean(workflow.outputText?.trim());

  const toggle = (key: string) => {
    setExpandedKeys((current) => {
      const next = new Set(current);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
      }
      return next;
    });
  };
  const focusPhase = (key: string) => {
    setExpandedKeys((current) => {
      if (current.has(key)) {
        return current;
      }
      const next = new Set(current);
      next.add(key);
      return next;
    });
    phaseRefs.current
      .get(key)
      ?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  };

  const runStats = [
    workflow.totalChildren > 0
      ? t('{completed}/{total} children', {
          completed: workflow.completedChildren,
          total: workflow.totalChildren,
        })
      : null,
    workflow.failedChildren > 0
      ? t('{count} failed', { count: workflow.failedChildren })
      : null,
    workflow.totalCostUsd > 0 ? formatCost(workflow.totalCostUsd) : null,
    workflow.startedAt
      ? t('started {time}', {
          time: formatRelativeTime(workflow.startedAt, t),
        })
      : null,
  ].filter(Boolean);

  return (
    <div className="workflow-timeline">
      <div className="workflow-timeline-main">
        <div className="workflow-timeline-inner">
          {visiblePhases.map((phase) => {
            const expanded = expandedKeys.has(phase.key);
            const running = phase.status === 'running';
            return (
              <section
                className={`workflow-timeline-phase status-${phase.status}`}
                key={phase.key}
                ref={(el) => {
                  if (el) {
                    phaseRefs.current.set(phase.key, el);
                  } else {
                    phaseRefs.current.delete(phase.key);
                  }
                }}
              >
                <span className="workflow-phase-node">
                  <StepNode status={phase.status} />
                </span>
                <div className="workflow-phase-content">
                  <div
                    className={`turn-summary ${
                      expanded ? 'is-expanded' : 'is-collapsed'
                    } ${running ? 'is-running' : ''} has-body`}
                  >
                    <button
                      aria-expanded={expanded}
                      className="turn-summary-toggle"
                      onClick={() => toggle(phase.key)}
                      type="button"
                    >
                      <span className="turn-summary-label">{phase.title}</span>
                      {phase.children.length > 0 ? (
                        <span className="workflow-phase-progress">
                          {phase.completed}/{phase.children.length}
                        </span>
                      ) : null}
                      <ChevronDown
                        aria-hidden
                        className="turn-summary-chevron"
                        size={15}
                        strokeWidth={1.7}
                      />
                    </button>
                    <div aria-hidden className="turn-summary-divider" />
                    <div
                      aria-hidden={!expanded}
                      className="turn-summary-body"
                      inert={!expanded ? true : undefined}
                    >
                      <div className="turn-summary-body-inner">
                        {phase.detail ? (
                          <p className="workflow-phase-detail">{phase.detail}</p>
                        ) : null}
                        {phase.children.length ? (
                          <div className="workflow-agent-grid">
                            {phase.children.map((child) => (
                              <AgentCard
                                child={child}
                                key={child.workflowChildRunId}
                                onOpenThread={onOpenThread}
                                onViewResult={setDialogEntry}
                                t={t}
                              />
                            ))}
                          </div>
                        ) : null}
                      </div>
                    </div>
                  </div>
                </div>
              </section>
            );
          })}
          {showFinal ? (
            <article className="message-bubble assistant workflow-timeline-result">
              <RichMessageContent
                altPrefix="assistant"
                text={workflow.outputText || ''}
              />
            </article>
          ) : null}
        </div>
      </div>

      <aside className="workflow-plan-panel" aria-label={t('Plan')}>
        <div className="workflow-plan-section">
          <div className="workflow-plan-label">{t('Run')}</div>
          <div className="workflow-plan-meta">
            <Badge className={runStatusBadgeClass(workflow.status)}>
              {t(workflow.status)}
            </Badge>
          </div>
          {runStats.length ? (
            <div className="workflow-plan-stats">
              {runStats.map((stat, index) => (
                <span key={index}>{stat}</span>
              ))}
            </div>
          ) : null}
        </div>
        <div className="workflow-plan-section">
          <div className="workflow-plan-label">{t('Plan')}</div>
          <ol className="workflow-plan-list">
            {phases.map((phase) => (
              <li key={phase.key}>
                <div
                  className={`workflow-plan-item status-${phase.status}`}
                  onClick={() => focusPhase(phase.key)}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter' || event.key === ' ') {
                      event.preventDefault();
                      focusPhase(phase.key);
                    }
                  }}
                  role="button"
                  tabIndex={0}
                >
                  <span className="workflow-plan-check">
                    <PlanMarker status={phase.status} />
                  </span>
                  <span className="workflow-plan-item-title" title={phase.title}>
                    {phase.title}
                  </span>
                  {phase.children.length > 0 ? (
                    <span className="workflow-plan-count">
                      {phase.completed}/{phase.children.length}
                    </span>
                  ) : null}
                </div>
              </li>
            ))}
          </ol>
        </div>
      </aside>

      <ResultValueDialog
        entry={dialogEntry}
        onClose={() => setDialogEntry(null)}
        t={t}
      />
    </div>
  );
}

function RunCard({
  run,
  viewMode,
  onOpenThread,
  t,
}: {
  run: DesktopWorkflowRunDrilldown;
  viewMode: WorkflowViewMode;
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
  const runOutcome = workflow.outputText || '';

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
        viewMode === 'timeline' ? (
          <WorkflowTimelineView
            onOpenThread={onOpenThread}
            phases={phases}
            t={t}
            workflow={workflow}
          />
        ) : (
        <div className="workflow-run-console">
          <nav className="workflow-phase-column" aria-label={t('Workflow phases')}>
            <div className="workflow-console-label">{t('Phases')}</div>
            {phases.map((phase) => (
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
                  <span className="workflow-phase-title">{phase.title}</span>
                  {phase.children.length > 0 ? (
                    <span className="workflow-phase-count">
                      {phase.completed}/{phase.children.length}
                    </span>
                  ) : null}
                </span>
              </button>
            ))}
          </nav>

          <section className="workflow-agent-column">
            <div className="workflow-agent-column-head">
              <span className="workflow-console-label">
                {activePhase && activePhase.children.length > 0
                  ? t('{phase} · {count} agents', {
                      phase: activePhase.title,
                      count: activePhase.children.length,
                    })
                  : activePhase?.title || t('Agents')}
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
        )
      ) : runOutcome ? (
        <section className="workflow-result-section workflow-run-result-fallback workflow-result-markdown">
          <h4>{t('Workflow result')}</h4>
          <ReactMarkdown remarkPlugins={[remarkGfm, remarkBreaks]}>
            {runOutcome}
          </ReactMarkdown>
        </section>
      ) : null}

    </section>
  );
}

export function WorkflowRunsPanel({
  task,
  taskId,
  workflowRunId,
  onOpenTasks,
  onOpenThread,
  onToast,
  t,
}: WorkflowRunsPanelProps) {
  const [runs, setRuns] = useState<DesktopWorkflowRunDrilldown[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<WorkflowViewMode>(readStoredViewMode);

  useEffect(() => {
    try {
      window.localStorage.setItem(VIEW_MODE_STORAGE_KEY, viewMode);
    } catch {
      // Ignore storage failures (private mode, quota, etc.).
    }
  }, [viewMode]);
  const hasNonTerminal = runs.some((run) => !isTerminal(run.workflow.status));
  const shouldPoll =
    hasNonTerminal ||
    (runs.length === 0 &&
      (task?.status === 'in_progress' || Boolean(workflowRunId)));
  const mountedRef = useRef(true);
  const primaryWorkflow = runs[0]?.workflow || null;
  const awaitingWorkflowRunRecord =
    Boolean(workflowRunId) && !error && runs.length === 0;

  const load = useCallback(
    async (options?: { silent?: boolean }) => {
      if (!options?.silent) {
        setLoading(true);
      }
      try {
        const workflowRunKey = workflowRunId?.trim() || '';
        const taskKey = taskId?.trim() || '';
        const nextRuns = workflowRunKey
          ? [
              await getDesktopApi().getWorkflowRun({
                workflowRunId: workflowRunKey,
              }),
            ]
          : taskKey
            ? (await getDesktopApi().listTaskWorkflowRuns({
                taskId: taskKey,
                limit: 50,
              })).workflowRuns
            : [];
        if (!mountedRef.current) {
          return;
        }
        setRuns(nextRuns);
        setError(null);
      } catch (loadError) {
        if (!mountedRef.current) {
          return;
        }
        if (workflowRunId?.trim() && isWorkflowRunNotFoundError(loadError)) {
          setRuns([]);
          setError(null);
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
    [taskId, workflowRunId, onToast],
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
          {onOpenTasks ? (
            <button
              className="workflow-runs-back"
              onClick={onOpenTasks}
              type="button"
            >
              <ArrowLeft aria-hidden size={14} strokeWidth={1.8} />
              {t('Tasks')}
            </button>
          ) : (
            <span className="workflow-runs-header-spacer" />
          )}
          <div className="workflow-runs-header-actions">
            {runs.length ? (
              <ToggleGroup
                aria-label={t('Workflow view')}
                onValueChange={(value) => {
                  if (value === 'timeline' || value === 'console') {
                    setViewMode(value);
                  }
                }}
                type="single"
                value={viewMode}
                variant="outline"
              >
                <ToggleGroupItem
                  className="h-7 px-3 text-xs data-[state=on]:bg-accent data-[state=on]:text-foreground"
                  value="timeline"
                >
                  {t('Timeline')}
                </ToggleGroupItem>
                <ToggleGroupItem
                  className="h-7 px-3 text-xs data-[state=on]:bg-accent data-[state=on]:text-foreground"
                  value="console"
                >
                  {t('Console')}
                </ToggleGroupItem>
              </ToggleGroup>
            ) : null}
            {primaryWorkflow?.threadId && primaryWorkflow.threadId !== workflowRunId ? (
              <button
                className="tasks-icon-button"
                onClick={() => {
                  onOpenThread(primaryWorkflow.threadId);
                }}
                title={t('Open thread')}
                type="button"
              >
                <MessageSquare aria-hidden size={14} strokeWidth={1.8} />
              </button>
            ) : null}
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
          ) : awaitingWorkflowRunRecord ? (
            <div className="workflow-runs-state">{t('Loading workflow runs…')}</div>
          ) : runs.length ? (
            <div className="workflow-runs-list">
              {runs.map((run) => (
                <RunCard
                  key={run.workflow.workflowRunId}
                  onOpenThread={onOpenThread}
                  run={run}
                  t={t}
                  viewMode={viewMode}
                />
              ))}
            </div>
          ) : (
            <div className="workflow-runs-state">
              {taskId ? t('No workflow runs for this task.') : t('No workflow data for this thread.')}
            </div>
          )}
        </div>
      </section>
    </div>
  );
}
