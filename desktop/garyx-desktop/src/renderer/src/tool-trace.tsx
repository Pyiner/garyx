import { memo, type ComponentType, type ReactNode, useEffect, useMemo, useState } from 'react';

import type { RenderToolFieldProjection } from '@shared/contracts';

import { Brain, Folder, Globe, LayoutList, ListChecks, ListTree, MessageCircleQuestion, Search, Wrench } from 'lucide-react';

import { CodexMessageIcon, type CodexMessageIconName } from './codex-message-icons';
import {
  resolveMergedToolTrace,
  type MergedToolTrace,
  type ToolTraceMessage,
} from './tool-trace-registry';
import {
  MessageImageAttachmentFrame,
  MessagePathImageAttachmentFrame,
  type MessageImagePreviewLoader,
} from './message-rich-content';
import { useI18n, type AppLocale, type Translate } from './i18n';

type ToolTraceEntry = {
  key: string;
  toolUse?: ToolTraceMessage;
  toolResult?: ToolTraceMessage;
  projection?: RenderToolFieldProjection;
};

type ToolTraceTreeNode = {
  entry: ToolTraceEntry;
  children: ToolTraceTreeNode[];
};

const ICON_SIZE = 16;
const ICON_STROKE = 1.6;

const CODEX_TOOL_ICON_MAP: Partial<Record<string, CodexMessageIconName>> = {
  '⌘': 'command',
  '≡': 'read',
  '✎': 'edit',
};

const TOOL_ICON_MAP: Record<string, ComponentType<{ size?: number; strokeWidth?: number }>> = {
  '⌕': Search,
  '◌': Folder,
  '↗': Globe,
  '◇': ListTree,
  '☑': ListChecks,
  '▤': LayoutList,
  '?': MessageCircleQuestion,
  '⊚': Wrench,
  '·': Brain,
};

function ToolIcon({ icon }: { icon: string }) {
  const codexIcon = CODEX_TOOL_ICON_MAP[icon];
  if (codexIcon) {
    return <CodexMessageIcon name={codexIcon} size={ICON_SIZE} />;
  }
  const Component = TOOL_ICON_MAP[icon];
  if (Component) {
    return <Component size={ICON_SIZE} strokeWidth={ICON_STROKE} />;
  }
  return <Wrench size={ICON_SIZE} strokeWidth={ICON_STROKE} />;
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
          {merged.summary || merged.resultSummary || merged.badges.length || merged.diffStats ? (
            <span className="tool-trace-meta-row">
              {merged.summary ? (
                <span className="tool-trace-summary" title={merged.summary}>
                  {merged.summary}
                </span>
              ) : null}
              {merged.resultSummary ? (
                <span className="tool-trace-result-summary" title={merged.resultSummary}>
                  {merged.resultSummary}
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
          key={node.entry.key}
          nestedChildren={node.children.length ? <ToolTraceTree nodes={node.children} onThreadNavigate={onThreadNavigate} /> : null}
          onThreadNavigate={onThreadNavigate}
          projection={node.entry.projection}
          toolResult={node.entry.toolResult}
          toolUse={node.entry.toolUse}
        />
      ))}
    </>
  );
}

const FILE_TRACE_TITLES = new Set([
  'Changed',
  'Created',
  'Deleted',
  'Edit',
  'Moved',
  'Updated',
  'Write',
]);

function countLabel(
  count: number,
  singularKey: string,
  pluralKey: string,
  t: Translate,
): string {
  return t(count === 1 ? singularKey : pluralKey, { count });
}

function summarizeToolTraceEntries(
  entries: ToolTraceEntry[],
  t: Translate,
  locale: AppLocale,
): string {
  let commandCount = 0;
  let otherCount = 0;
  const fileKeys = new Set<string>();

  for (const entry of entries) {
    const merged = resolveMergedToolTrace(entry.toolUse, entry.toolResult, entry.projection);
    if (merged.icon === '⌘' || merged.title === 'Command') {
      commandCount += 1;
      continue;
    }
    if (merged.icon === '✎' || FILE_TRACE_TITLES.has(merged.title)) {
      fileKeys.add(merged.badges[0] || entry.key);
      continue;
    }
    otherCount += 1;
  }

  const parts: string[] = [];
  const fileCount = fileKeys.size;
  if (fileCount) {
    parts.push(countLabel(fileCount, 'Edited {count} file', 'Edited {count} files', t));
  }
  if (commandCount) {
    parts.push(countLabel(commandCount, 'Ran {count} command', 'Ran {count} commands', t));
  }
  if (otherCount || !parts.length) {
    parts.push(countLabel(otherCount || entries.length, 'Used {count} tool', 'Used {count} tools', t));
  }

  return parts.join(locale === 'zh-CN' ? '，' : ', ');
}

function resolveToolTraceGroupIcon(entries: ToolTraceEntry[]): string {
  const mergedEntries = entries.map((entry) =>
    resolveMergedToolTrace(entry.toolUse, entry.toolResult, entry.projection),
  );

  // Codex chooses the icon from the first summary category, whose category
  // order is file changes, exploration, then commands. This is why a mixed
  // "edited files, read files, ran commands" group carries the edit glyph.
  if (mergedEntries.some((merged) => merged.icon === '✎' || FILE_TRACE_TITLES.has(merged.title))) {
    return '✎';
  }
  if (mergedEntries.some((merged) => merged.icon === '≡')) {
    return '≡';
  }
  if (mergedEntries.some((merged) => merged.icon === '⌘' || merged.title === 'Command')) {
    return '⌘';
  }
  return mergedEntries[0]?.icon || '⊚';
}

type ToolTraceGroupImage =
  | {
      kind: 'embedded';
      key: string;
      image: MergedToolTrace['resultImages'][number];
    }
  | {
      kind: 'path';
      key: string;
      image: MergedToolTrace['pathImages'][number];
    };

function collectToolTraceGroupImages(entries: ToolTraceEntry[]): ToolTraceGroupImage[] {
  const images: ToolTraceGroupImage[] = [];
  const seenSources = new Set<string>();
  const seenPaths = new Set<string>();
  for (const entry of entries) {
    const merged = resolveMergedToolTrace(entry.toolUse, entry.toolResult, entry.projection);
    for (const image of merged.resultImages) {
      if (seenSources.has(image.src)) {
        continue;
      }
      seenSources.add(image.src);
      images.push({ kind: 'embedded', key: `${entry.key}:${image.key}`, image });
    }
    if (merged.resultImages.length > 0) {
      continue;
    }
    for (const image of merged.pathImages) {
      if (seenPaths.has(image.path)) {
        continue;
      }
      seenPaths.add(image.path);
      images.push({ kind: 'path', key: `${entry.key}:${image.key}`, image });
    }
  }
  return images;
}

function ToolTraceGroupComponent({
  active = false,
  entries,
  defaultExpanded,
  loadImagePreview,
  onThreadNavigate,
}: {
  active?: boolean;
  entries: ToolTraceEntry[];
  defaultExpanded: boolean;
  loadImagePreview?: MessageImagePreviewLoader;
  onThreadNavigate?: (threadId: string) => void;
}) {
  const { locale, t } = useI18n();
  const [expanded, setExpanded] = useState(defaultExpanded);
  const [userControlled, setUserControlled] = useState(false);
  const summary = useMemo(
    () => summarizeToolTraceEntries(entries, t, locale),
    [entries, locale, t],
  );
  const groupIcon = useMemo(() => resolveToolTraceGroupIcon(entries), [entries]);
  const treeNodes = useMemo(() => buildToolTraceTree(entries), [entries]);
  const groupImages = useMemo(() => collectToolTraceGroupImages(entries), [entries]);

  useEffect(() => {
    if (!userControlled) {
      setExpanded(defaultExpanded);
    }
  }, [defaultExpanded, userControlled]);

  return (
    <div className={`tool-trace-group ${expanded ? 'is-expanded' : 'is-collapsed'} ${active ? 'is-active' : ''}`}>
      <button
        aria-expanded={expanded}
        aria-label={expanded ? t('Collapse tool calls') : t('Expand tool calls')}
        className="tool-trace-group-header"
        onClick={() => {
          setUserControlled(true);
          setExpanded((current) => !current);
        }}
        type="button"
      >
        <span className="tool-trace-group-leading">
          <span className="tool-trace-group-icon">
            <ToolIcon icon={groupIcon} />
          </span>
          <span className="tool-trace-group-summary">{summary}</span>
        </span>
        <CodexMessageIcon
          className="tool-trace-group-chevron"
          name="chevron"
          size={14}
        />
      </button>
      {groupImages.length > 0 ? (
        <div className="tool-trace-images tool-trace-group-images">
          {groupImages.map((item) => {
            if (item.kind === 'embedded') {
              return (
                <MessageImageAttachmentFrame
                  compact
                  key={item.key}
                  segment={item.image}
                />
              );
            }
            return loadImagePreview ? (
              <MessagePathImageAttachmentFrame
                alt={item.image.alt}
                compact
                imageKey={item.image.key}
                key={item.key}
                loadImagePreview={loadImagePreview}
                path={item.image.path}
              />
            ) : null;
          })}
        </div>
      ) : null}
      <div
        aria-hidden={!expanded}
        className="tool-trace-group-panel"
        inert={!expanded ? true : undefined}
      >
        <div className="tool-trace-group-panel-inner">
          <div className="tool-trace-group-list">
            <ToolTraceTree
              nodes={treeNodes}
              onThreadNavigate={onThreadNavigate}
            />
          </div>
        </div>
      </div>
    </div>
  );
}

export const ToolTraceGroup = memo(
  ToolTraceGroupComponent,
  (previous, next) =>
    previous.active === next.active &&
    previous.defaultExpanded === next.defaultExpanded &&
    previous.entries === next.entries &&
    previous.loadImagePreview === next.loadImagePreview,
);

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
  projection,
  nestedChildren,
  onThreadNavigate,
}: {
  toolUse?: ToolTraceMessage;
  toolResult?: ToolTraceMessage;
  projection?: RenderToolFieldProjection;
  nestedChildren?: ReactNode;
  onThreadNavigate?: (threadId: string) => void;
}) {
  const { t } = useI18n();
  const merged = useMemo(
    () => resolveMergedToolTrace(toolUse, toolResult, projection),
    [projection, toolResult, toolUse],
  );
  const [expanded, setExpanded] = useState(false);
  const targetThreadId = extractTargetThreadId(toolResult);
  const hasDetails = Boolean(merged.inputDetail || merged.resultDetail || nestedChildren);

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
