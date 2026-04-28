import { type ComponentType, type ReactNode, useEffect, useState } from 'react';

import {
  IconTerminal2,
  IconFileText,
  IconPencil,
  IconSearch,
  IconFolder,
  IconWorldWww,
  IconSubtask,
  IconListCheck,
  IconLayoutList,
  IconMessageQuestion,
  IconTool,
  IconBrain,
  IconDownload,
} from '@tabler/icons-react';

import {
  canMergeToolTraceMessages,
  resolveMergedToolTrace,
  type MergedToolTrace,
  type ToolTraceMessage,
} from './tool-trace-registry';
import { useI18n } from './i18n';

export { canMergeToolTraceMessages, type ToolTraceMessage } from './tool-trace-registry';

type ToolTraceEntry = {
  key: string;
  toolUse?: ToolTraceMessage;
  toolResult?: ToolTraceMessage;
  defaultExpanded: boolean;
};

type ToolTraceTreeNode = {
  entry: ToolTraceEntry;
  children: ToolTraceTreeNode[];
};

const ICON_SIZE = 16;
const ICON_STROKE = 1.6;

const TOOL_ICON_MAP: Record<string, ComponentType<{ size?: number; stroke?: number }>> = {
  '⌘': IconTerminal2,
  '≡': IconFileText,
  '✎': IconPencil,
  '⌕': IconSearch,
  '◌': IconFolder,
  '↗': IconWorldWww,
  '◇': IconSubtask,
  '☑': IconListCheck,
  '▤': IconLayoutList,
  '?': IconMessageQuestion,
  '⊚': IconTool,
  '·': IconBrain,
};

function ToolIcon({ icon }: { icon: string }) {
  const Component = TOOL_ICON_MAP[icon];
  if (Component) {
    return <Component size={ICON_SIZE} stroke={ICON_STROKE} />;
  }
  return <IconTool size={ICON_SIZE} stroke={ICON_STROKE} />;
}

function DiffStatsLabel({ added, removed }: { added: number; removed: number }) {
  return (
    <span className="tool-trace-diff-stats">
      {added > 0 ? <span className="diff-added">+{added}</span> : null}
      {removed > 0 ? <span className="diff-removed">-{removed}</span> : null}
    </span>
  );
}

function ToolTraceHeader({ merged }: { merged: MergedToolTrace }) {
  return (
    <>
      <div className="tool-trace-main">
        <span className="tool-trace-icon">
          <ToolIcon icon={merged.icon} />
        </span>
        <span className="tool-trace-copy">
          <span className="tool-trace-title" title={merged.title}>
            {merged.title}
          </span>
          {merged.summary || merged.badges.length || merged.diffStats ? (
            <span className="tool-trace-meta-row">
              {merged.summary ? (
                <span className="tool-trace-summary" title={merged.summary}>
                  {merged.summary}
                </span>
              ) : null}
              {merged.badges.length ? (
                <span className="tool-trace-badges">
                  {merged.badges.map((badge) => (
                    <span className="tool-trace-badge" key={badge} title={badge}>
                      {badge}
                    </span>
                  ))}
                </span>
              ) : null}
              {merged.diffStats ? (
                <DiffStatsLabel added={merged.diffStats.added} removed={merged.diffStats.removed} />
              ) : null}
            </span>
          ) : null}
        </span>
      </div>
      {merged.status ? (
        <div className="tool-trace-actions">
          <span className={`tool-trace-status is-${merged.status.tone}`}>
            {merged.status.label}
          </span>
        </div>
      ) : null}
    </>
  );
}

function classifyDiffLine(line: string): 'added' | 'removed' | 'hunk' | 'plain' {
  if (line.startsWith('+++') || line.startsWith('---')) {
    return 'hunk';
  }
  if (line.startsWith('+')) {
    return 'added';
  }
  if (line.startsWith('-')) {
    return 'removed';
  }
  if (line.startsWith('@@')) {
    return 'hunk';
  }
  return 'plain';
}

function ToolTraceBody({ content, label }: { content: string; label?: string }) {
  if (label === 'Diff') {
    return (
      <pre className="tool-trace-body tool-trace-body-diff">
        {content.split('\n').map((line, index) => (
          <span
            className={`tool-trace-diff-line is-${classifyDiffLine(line)}`}
            key={`${index}:${line}`}
          >
            {line || ' '}
          </span>
        ))}
      </pre>
    );
  }

  return <pre className="tool-trace-body">{content}</pre>;
}

function extractParentToolUseId(message?: ToolTraceMessage): string | null {
  const metadata =
    message?.metadata && typeof message.metadata === 'object'
      ? message.metadata as Record<string, unknown>
      : null;
  const value = metadata?.parent_tool_use_id ?? metadata?.parentToolUseId;
  return typeof value === 'string' && value.trim() ? value.trim() : null;
}

function resolveEntryToolUseId(entry: ToolTraceEntry): string | null {
  return entry.toolUse?.toolUseId || entry.toolResult?.toolUseId || null;
}

function resolveEntryParentToolUseId(entry: ToolTraceEntry): string | null {
  return extractParentToolUseId(entry.toolUse) || extractParentToolUseId(entry.toolResult);
}

function buildToolTraceTree(entries: ToolTraceEntry[]): ToolTraceTreeNode[] {
  const nodes = entries.map((entry) => ({
    entry,
    children: [],
  }));
  const nodesByToolUseId = new Map<string, ToolTraceTreeNode>();

  for (const node of nodes) {
    const toolUseId = resolveEntryToolUseId(node.entry);
    if (toolUseId && !nodesByToolUseId.has(toolUseId)) {
      nodesByToolUseId.set(toolUseId, node);
    }
  }

  const roots: ToolTraceTreeNode[] = [];
  for (const node of nodes) {
    const parentToolUseId = resolveEntryParentToolUseId(node.entry);
    const parent = parentToolUseId ? nodesByToolUseId.get(parentToolUseId) : null;
    if (parent && parent !== node) {
      parent.children.push(node);
      continue;
    }
    roots.push(node);
  }

  return roots;
}

function ToolTraceTree({
  nodes,
  onThreadNavigate,
}: {
  nodes: ToolTraceTreeNode[];
  onThreadNavigate?: (threadId: string) => void;
}) {
  return (
    <>
      {nodes.map((node) => (
        <ToolTraceLine
          defaultExpanded={node.entry.defaultExpanded}
          key={node.entry.key}
          nestedChildren={node.children.length ? <ToolTraceTree nodes={node.children} onThreadNavigate={onThreadNavigate} /> : null}
          onThreadNavigate={onThreadNavigate}
          toolResult={node.entry.toolResult}
          toolUse={node.entry.toolUse}
        />
      ))}
    </>
  );
}

export function ToolTraceGroup({
  entries,
  onThreadNavigate,
}: {
  entries: ToolTraceEntry[];
  onThreadNavigate?: (threadId: string) => void;
}) {
  return <ToolTraceTree nodes={buildToolTraceTree(entries)} onThreadNavigate={onThreadNavigate} />;
}

function extractTargetThreadId(toolResult?: ToolTraceMessage): string | null {
  try {
    const text = toolResult?.text?.trim();
    if (!text || !text.startsWith('{')) return null;
    const parsed = JSON.parse(text);
    if (typeof parsed?.target_thread_id === 'string' && parsed.target_thread_id.trim()) {
      return parsed.target_thread_id.trim();
    }
  } catch { /* not json */ }
  return null;
}

export function ToolTraceLine({
  toolUse,
  toolResult,
  defaultExpanded,
  nestedChildren,
  onThreadNavigate,
}: {
  toolUse?: ToolTraceMessage;
  toolResult?: ToolTraceMessage;
  defaultExpanded: boolean;
  nestedChildren?: ReactNode;
  onThreadNavigate?: (threadId: string) => void;
}) {
  const { t } = useI18n();
  const merged = resolveMergedToolTrace(toolUse, toolResult);
  const [expanded, setExpanded] = useState(defaultExpanded);
  const targetThreadId = extractTargetThreadId(toolResult);
  const hasDetails = Boolean(merged.inputDetail || merged.resultDetail || nestedChildren);

  useEffect(() => {
    setExpanded(defaultExpanded);
  }, [defaultExpanded]);

  return (
    <div className={`tool-trace ${merged.isError ? 'is-error' : ''} ${!hasDetails ? 'is-static' : ''}`}>
      {hasDetails ? (
        <button
          aria-expanded={expanded}
          className="tool-trace-header"
          onClick={() => {
            setExpanded((current) => !current);
          }}
          tabIndex={-1}
          type="button"
        >
          <ToolTraceHeader merged={merged} />
        </button>
      ) : (
        <div className="tool-trace-header">
          <ToolTraceHeader merged={merged} />
        </div>
      )}
      {targetThreadId && onThreadNavigate ? (
        <div className="tool-trace-navigate">
          <button
            className="tool-trace-navigate-btn"
            onClick={() => onThreadNavigate(targetThreadId)}
            type="button"
          >
            Open thread &rarr;
          </button>
        </div>
      ) : null}
      {expanded && hasDetails ? (
        <div className="tool-trace-details">
          {merged.inputDetail ? (
            <div className="tool-trace-section">
              <span className="tool-trace-section-label">
                {merged.inputLabel || t('Call')}
              </span>
              <ToolTraceBody content={merged.inputDetail} label={merged.inputLabel} />
            </div>
          ) : null}
          {merged.resultDetail ? (
            <div className="tool-trace-section">
              <span className="tool-trace-section-label">
                {merged.resultLabel || t('Result')}
              </span>
              <ToolTraceBody content={merged.resultDetail} label={merged.resultLabel} />
            </div>
          ) : null}
          {nestedChildren ? (
            <div className="tool-trace-section">
              <span className="tool-trace-section-label">{t('Activity')}</span>
              <div className="tool-trace-children">
                <div className="tool-trace-children-scroll">
                  {nestedChildren}
                </div>
              </div>
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
