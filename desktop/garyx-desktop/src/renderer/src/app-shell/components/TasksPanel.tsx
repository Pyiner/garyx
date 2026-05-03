import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type DragEvent,
  type FormEvent,
} from 'react';
import {
  ArrowRight,
  CheckCircle2,
  Columns3,
  List,
  MessageSquare,
  Plus,
  RefreshCcw,
} from 'lucide-react';

import type {
  DesktopCustomAgent,
  DesktopState,
  DesktopTaskPrincipal,
  DesktopTaskStatus,
  DesktopTaskSummary,
} from '@shared/contracts';

import { useI18n, type Translate } from '../../i18n';
import type { ToastTone } from '../../toast';
import { getDesktopApi } from '../../platform/desktop-api';

type TasksPanelProps = {
  agents: DesktopCustomAgent[];
  desktopState: DesktopState | null;
  onOpenThread: (threadId: string) => void;
  onToast: (message: string, tone?: ToastTone) => void;
};

type TaskViewMode = 'board' | 'list';

type TaskScopeOption = {
  value: string;
  label: string;
  meta: string;
  workspaceDir?: string | null;
};

const ALL_SCOPES = 'all';

const TASK_COLUMNS: Array<{
  status: DesktopTaskStatus;
  label: string;
  tone: string;
}> = [
  { status: 'todo', label: 'Todo', tone: 'todo' },
  { status: 'in_progress', label: 'In Progress', tone: 'progress' },
  { status: 'in_review', label: 'In Review', tone: 'review' },
  { status: 'done', label: 'Done', tone: 'done' },
];

const STATUS_LABELS: Record<DesktopTaskStatus, string> = {
  todo: 'Todo',
  in_progress: 'In Progress',
  in_review: 'In Review',
  done: 'Done',
};

const TASK_DRAG_MIME = 'application/x-garyx-task-ref';

const LOCAL_TASK_SCOPE: TaskScopeOption = {
  value: 'garyx/tasks',
  label: 'Garyx',
  meta: 'garyx/tasks',
  workspaceDir: null,
};

function formatScope(channel: string, accountId: string): string {
  return `${channel}/${accountId}`;
}

function taskScope(task: DesktopTaskSummary): string {
  return formatScope(task.scope.channel, task.scope.accountId);
}

function formatPrincipal(principal: DesktopTaskPrincipal | null | undefined, t: Translate): string {
  if (!principal) {
    return t('Unassigned');
  }
  if (principal.kind === 'human') {
    return `@${principal.userId}`;
  }
  return principal.agentId;
}

function formatTimestamp(value?: string | null): string {
  if (!value) {
    return '';
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return '';
  }
  const sameDay = date.toDateString() === new Date().toDateString();
  return new Intl.DateTimeFormat(undefined, sameDay
    ? { hour: 'numeric', minute: '2-digit' }
    : { month: 'short', day: 'numeric' },
  ).format(date);
}

function nextStatus(status: DesktopTaskStatus): {
  label: string;
  status: DesktopTaskStatus;
} {
  switch (status) {
    case 'todo':
      return { label: 'Start', status: 'in_progress' };
    case 'in_progress':
      return { label: 'Send to Review', status: 'in_review' };
    case 'in_review':
      return { label: 'Done', status: 'done' };
    case 'done':
      return { label: 'Reopen', status: 'todo' };
  }
}

function isTasksDisabled(error: string | null): boolean {
  return Boolean(error && /tasks are disabled|TasksDisabled/i.test(error));
}

function buildScopeOptions(state: DesktopState | null): TaskScopeOption[] {
  const byValue = new Map<string, TaskScopeOption>();
  const add = (option: TaskScopeOption) => {
    if (!option.value || byValue.has(option.value)) {
      return;
    }
    byValue.set(option.value, option);
  };

  add(LOCAL_TASK_SCOPE);

  for (const bot of state?.configuredBots || []) {
    const value = formatScope(bot.channel, bot.accountId);
    add({
      value,
      label: bot.displayName || value,
      meta: value,
      workspaceDir: bot.workspaceDir,
    });
  }

  return [...byValue.values()].sort((left, right) =>
    left.label.localeCompare(right.label),
  );
}

function taskCountLabel(count: number, t: Translate): string {
  return t('{count} tasks', { count });
}

export function TasksPanel({
  agents,
  desktopState,
  onOpenThread,
  onToast,
}: TasksPanelProps) {
  const { t } = useI18n();
  const scopeOptions = useMemo(
    () => buildScopeOptions(desktopState),
    [desktopState],
  );
  const [selectedScope, setSelectedScope] = useState<string>(ALL_SCOPES);
  const [viewMode, setViewMode] = useState<TaskViewMode>('board');
  const [tasks, setTasks] = useState<DesktopTaskSummary[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [mutatingTaskRef, setMutatingTaskRef] = useState<string | null>(null);
  const [draftOpen, setDraftOpen] = useState(false);
  const [draftTitle, setDraftTitle] = useState('');
  const [draftBody, setDraftBody] = useState('');
  const [draftStart, setDraftStart] = useState(false);
  const [draftAssignee, setDraftAssignee] = useState('');
  const [draftScope, setDraftScope] = useState('');
  const [creating, setCreating] = useState(false);
  const [draggingTaskRef, setDraggingTaskRef] = useState<string | null>(null);
  const [dropStatus, setDropStatus] = useState<DesktopTaskStatus | null>(null);
  const draggingTaskRefValue = useRef<string | null>(null);

  const selectedScopeOption = scopeOptions.find(
    (option) => option.value === selectedScope,
  );
  const createScopeOptions = scopeOptions;
  const createDisabled = !createScopeOptions.length;

  const loadTasks = useCallback(async (options?: { silent?: boolean }) => {
    if (!options?.silent) {
      setLoading(true);
    }
    setError(null);
    try {
      const page = await getDesktopApi().listTasks({
        scope: selectedScope === ALL_SCOPES ? null : selectedScope,
        includeDone: true,
        limit: 200,
      });
      setTasks(page.tasks);
      setTotal(page.total);
    } catch (loadError) {
      const message = loadError instanceof Error
        ? loadError.message
        : String(loadError || 'Failed to load tasks');
      setError(message);
      setTasks([]);
      setTotal(0);
    } finally {
      if (!options?.silent) {
        setLoading(false);
      }
    }
  }, [selectedScope]);

  useEffect(() => {
    if (selectedScope !== ALL_SCOPES && !scopeOptions.some((option) => option.value === selectedScope)) {
      setSelectedScope(scopeOptions[0]?.value || ALL_SCOPES);
    }
  }, [scopeOptions, selectedScope]);

  useEffect(() => {
    if (
      draftScope &&
      createScopeOptions.some((option) => option.value === draftScope)
    ) {
      return;
    }
    if (createScopeOptions[0]) {
      const nextDraftScope =
        selectedScope !== ALL_SCOPES &&
        createScopeOptions.some((option) => option.value === selectedScope)
          ? selectedScope
          : createScopeOptions[0].value;
      setDraftScope(
        nextDraftScope,
      );
    } else {
      setDraftScope('');
    }
  }, [createScopeOptions, draftScope, selectedScope]);

  useEffect(() => {
    void loadTasks();
  }, [loadTasks]);

  const tasksByStatus = useMemo(() => {
    const grouped: Record<DesktopTaskStatus, DesktopTaskSummary[]> = {
      todo: [],
      in_progress: [],
      in_review: [],
      done: [],
    };
    for (const task of tasks) {
      grouped[task.status].push(task);
    }
    return grouped;
  }, [tasks]);

  async function moveTask(task: DesktopTaskSummary, to: DesktopTaskStatus) {
    if (task.status === to) {
      return;
    }
    setMutatingTaskRef(task.taskRef);
    try {
      await getDesktopApi().updateTaskStatus({
        taskRef: task.taskRef,
        status: to,
      });
      await loadTasks({ silent: true });
      onToast(t('Task updated.'), 'success');
    } catch (moveError) {
      onToast(
        moveError instanceof Error ? moveError.message : t('Task update failed.'),
        'error',
      );
    } finally {
      setMutatingTaskRef(null);
    }
  }

  function draggedTask(event: DragEvent<HTMLElement>): DesktopTaskSummary | null {
    const taskRef =
      event.dataTransfer.getData(TASK_DRAG_MIME) ||
      event.dataTransfer.getData('text/plain') ||
      draggingTaskRefValue.current ||
      draggingTaskRef;
    return tasks.find((task) => task.taskRef === taskRef) || null;
  }

  function handleColumnDragOver(event: DragEvent<HTMLElement>, status: DesktopTaskStatus) {
    const task = draggedTask(event);
    if (!task || task.status === status || mutatingTaskRef === task.taskRef) {
      return;
    }
    event.preventDefault();
    event.dataTransfer.dropEffect = 'move';
    if (dropStatus !== status) {
      setDropStatus(status);
    }
  }

  function handleColumnDragLeave(event: DragEvent<HTMLElement>, status: DesktopTaskStatus) {
    const relatedTarget = event.relatedTarget;
    if (relatedTarget instanceof Node && event.currentTarget.contains(relatedTarget)) {
      return;
    }
    setDropStatus((current) => (current === status ? null : current));
  }

  function handleColumnDrop(event: DragEvent<HTMLElement>, status: DesktopTaskStatus) {
    event.preventDefault();
    const task = draggedTask(event);
    setDropStatus(null);
    setDraggingTaskRef(null);
    draggingTaskRefValue.current = null;
    if (!task || task.status === status || mutatingTaskRef === task.taskRef) {
      return;
    }
    void moveTask(task, status);
  }

  async function submitTask(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const title = draftTitle.trim();
    if (!title || !draftScope) {
      return;
    }
    const scope = createScopeOptions.find((option) => option.value === draftScope);
    const assignee = draftAssignee.trim();
    setCreating(true);
    try {
      await getDesktopApi().createTask({
        scope: draftScope,
        title,
        body: draftBody.trim() || null,
        assignee: assignee || null,
        start: draftStart || Boolean(assignee),
        workspaceDir: scope?.workspaceDir || null,
      });
      setDraftOpen(false);
      setDraftTitle('');
      setDraftBody('');
      setDraftStart(false);
      setDraftAssignee('');
      await loadTasks({ silent: true });
      onToast(t('Task created.'), 'success');
    } catch (createError) {
      onToast(
        createError instanceof Error ? createError.message : t('Task creation failed.'),
        'error',
      );
    } finally {
      setCreating(false);
    }
  }

  const disabled = isTasksDisabled(error);
  const visibleCount = tasks.length;
  const headerCount = loading
      ? t('Loading tasks…')
      : selectedScope === ALL_SCOPES
      ? taskCountLabel(total || visibleCount, t)
      : `${selectedScope} · ${taskCountLabel(total || visibleCount, t)}`;

  const renderTaskCard = (task: DesktopTaskSummary) => {
    const next = nextStatus(task.status);
    const busy = mutatingTaskRef === task.taskRef;
    const dragging = draggingTaskRef === task.taskRef;
    return (
      <article
        className={`tasks-card ${dragging ? 'is-dragging' : ''}`}
        draggable={!busy}
        key={task.taskRef}
        onDragEnd={() => {
          draggingTaskRefValue.current = null;
          setDraggingTaskRef(null);
          setDropStatus(null);
        }}
        onDragStart={(event) => {
          event.dataTransfer.effectAllowed = 'move';
          event.dataTransfer.setData(TASK_DRAG_MIME, task.taskRef);
          event.dataTransfer.setData('text/plain', task.taskRef);
          draggingTaskRefValue.current = task.taskRef;
          setDraggingTaskRef(task.taskRef);
        }}
      >
        <div className="tasks-card-topline">
          <span className="tasks-card-ref">{task.taskRef || `#${task.number}`}</span>
          <span className="tasks-card-scope">{taskScope(task)}</span>
        </div>
        <button
          className="tasks-card-title"
          disabled={!task.threadId}
          onClick={() => {
            if (task.threadId) {
              onOpenThread(task.threadId);
            }
          }}
          type="button"
        >
          {task.title}
        </button>
        <div className="tasks-card-meta">
          <span>{t('creator')} {formatPrincipal(task.creator, t)}</span>
          <span>{t('assignee')} {formatPrincipal(task.assignee, t)}</span>
        </div>
        <div className="tasks-card-footer">
          <span className="tasks-card-updated">{formatTimestamp(task.updatedAt)}</span>
          <div className="tasks-card-actions">
            <button
              className="tasks-icon-button"
              disabled={!task.threadId}
              onClick={() => {
                if (task.threadId) {
                  onOpenThread(task.threadId);
                }
              }}
              title={t('Open thread')}
              type="button"
            >
              <MessageSquare aria-hidden size={14} strokeWidth={1.8} />
            </button>
            <button
              className="tasks-move-button"
              disabled={busy}
              onClick={() => {
                void moveTask(task, next.status);
              }}
              type="button"
            >
              {task.status === 'done' ? (
                <RefreshCcw aria-hidden size={13} strokeWidth={1.8} />
              ) : (
                <ArrowRight aria-hidden size={13} strokeWidth={1.8} />
              )}
              {busy ? t('Saving…') : t(next.label)}
            </button>
          </div>
        </div>
      </article>
    );
  };

  return (
    <div className="tasks-page">
      <div className="tasks-page-header">
        <div className="tasks-page-title-block">
          <div className="tasks-page-title-row">
            <CheckCircle2 aria-hidden size={18} strokeWidth={1.8} />
            <h1 className="tasks-page-title">{t('Tasks')}</h1>
          </div>
          <p className="tasks-page-subtitle">{headerCount}</p>
        </div>
        <div className="tasks-header-actions">
          <button
            className="tasks-secondary-button"
            disabled={loading}
            onClick={() => {
              void loadTasks();
            }}
            type="button"
          >
            <RefreshCcw aria-hidden size={14} strokeWidth={1.8} />
            {loading ? t('Refreshing') : t('Refresh')}
          </button>
          <div aria-label={t('Task view')} className="tasks-segmented">
            <button
              className={viewMode === 'board' ? 'active' : ''}
              onClick={() => setViewMode('board')}
              type="button"
            >
              <Columns3 aria-hidden size={14} strokeWidth={1.8} />
              {t('Board')}
            </button>
            <button
              className={viewMode === 'list' ? 'active' : ''}
              onClick={() => setViewMode('list')}
              type="button"
            >
              <List aria-hidden size={14} strokeWidth={1.8} />
              {t('List')}
            </button>
          </div>
          <button
            className="tasks-primary-button"
            disabled={createDisabled}
            onClick={() => setDraftOpen((current) => !current)}
            type="button"
          >
            <Plus aria-hidden size={14} strokeWidth={1.8} />
            {t('New')}
          </button>
        </div>
      </div>

      <div className="tasks-filter-row">
        <label className="tasks-filter-label" htmlFor="tasks-scope">
          #
        </label>
        <select
          className="tasks-scope-select"
          id="tasks-scope"
          onChange={(event) => {
            setSelectedScope(event.target.value);
          }}
          value={selectedScope}
        >
          <option value={ALL_SCOPES}>{t('All channels')}</option>
          {scopeOptions.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label} · {option.value}
            </option>
          ))}
        </select>
        {selectedScopeOption ? (
          <span className="tasks-scope-meta">{selectedScopeOption.meta}</span>
        ) : null}
      </div>

      {draftOpen ? (
        <form className="tasks-create-panel" onSubmit={submitTask}>
          <div className="tasks-create-grid">
            <label className="tasks-field">
              <span>{t('Scope')}</span>
              <select
                onChange={(event) => setDraftScope(event.target.value)}
                value={draftScope}
              >
                {createScopeOptions.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label} · {option.value}
                  </option>
                ))}
              </select>
            </label>
            <label className="tasks-field tasks-title-field">
              <span>{t('Title')}</span>
              <input
                autoFocus
                onChange={(event) => setDraftTitle(event.target.value)}
                placeholder={t('Task title')}
                value={draftTitle}
              />
            </label>
            <label className="tasks-field">
              <span>{t('Assignee')}</span>
              <select
                onChange={(event) => setDraftAssignee(event.target.value)}
                value={draftAssignee}
              >
                <option value="">{t('Unassigned')}</option>
                {agents.map((agent) => (
                  <option key={agent.agentId} value={agent.agentId}>
                    {agent.displayName || agent.agentId}
                  </option>
                ))}
              </select>
            </label>
            <label className="tasks-field tasks-field-full">
              <span>{t('Body')}</span>
              <textarea
                onChange={(event) => setDraftBody(event.target.value)}
                placeholder={t('Optional task detail')}
                value={draftBody}
              />
            </label>
          </div>
          <div className="tasks-create-actions">
            <label className="tasks-checkbox">
              <input
                checked={draftStart || Boolean(draftAssignee.trim())}
                disabled={Boolean(draftAssignee.trim())}
                onChange={(event) => setDraftStart(event.target.checked)}
                type="checkbox"
              />
              <span>{draftAssignee.trim() ? t('Assigned tasks start automatically') : t('Start now')}</span>
            </label>
            <button className="tasks-secondary-button" onClick={() => setDraftOpen(false)} type="button">
              {t('Cancel')}
            </button>
            <button
              className="tasks-primary-button"
              disabled={!draftTitle.trim() || !draftScope || creating}
              type="submit"
            >
              <Plus aria-hidden size={14} strokeWidth={1.8} />
              {creating ? t('Creating…') : t('Create')}
            </button>
          </div>
        </form>
      ) : null}

      {error ? (
        <div className={`tasks-state ${disabled ? 'tasks-state-warning' : 'tasks-state-error'}`}>
          {disabled
            ? t('Tasks are disabled in the gateway config.')
            : error}
        </div>
      ) : null}

      {!error && !loading && !tasks.length ? (
        <div className="tasks-empty-state">
          {t('No tasks in this scope.')}
        </div>
      ) : null}

      {viewMode === 'board' ? (
        <div className="tasks-board" aria-busy={loading}>
          {TASK_COLUMNS.map((column) => {
            const columnTasks = tasksByStatus[column.status];
            return (
              <section
                className={`tasks-column ${dropStatus === column.status ? 'is-drop-target' : ''}`}
                key={column.status}
                onDragLeave={(event) => handleColumnDragLeave(event, column.status)}
                onDragOver={(event) => handleColumnDragOver(event, column.status)}
                onDrop={(event) => handleColumnDrop(event, column.status)}
              >
                <div className="tasks-column-header">
                  <span className={`tasks-status-chip tone-${column.tone}`}>
                    {t(column.label)}
                  </span>
                  <span className="tasks-column-count">{columnTasks.length}</span>
                </div>
                <div className="tasks-column-stack">
                  {columnTasks.length ? (
                    columnTasks.map(renderTaskCard)
                  ) : (
                    <div className="tasks-column-empty">
                      {t('No {status} tasks.', { status: t(column.label).toLowerCase() })}
                    </div>
                  )}
                </div>
              </section>
            );
          })}
        </div>
      ) : (
        <div className="tasks-list" aria-busy={loading}>
          <div className="tasks-list-header">
            <span>{t('Task')}</span>
            <span>{t('Status')}</span>
            <span>{t('Assignee')}</span>
            <span>{t('Updated')}</span>
            <span />
          </div>
          {tasks.map((task) => {
            const next = nextStatus(task.status);
            const busy = mutatingTaskRef === task.taskRef;
            return (
              <div className="tasks-list-row" key={task.taskRef}>
                <button
                  className="tasks-list-title"
                  disabled={!task.threadId}
                  onClick={() => {
                    if (task.threadId) {
                      onOpenThread(task.threadId);
                    }
                  }}
                  type="button"
                >
                  <span>{task.title}</span>
                  <small>{task.taskRef} · {taskScope(task)}</small>
                </button>
                <span className={`tasks-status-chip tone-${task.status.replace('in_', '')}`}>
                  {t(STATUS_LABELS[task.status])}
                </span>
                <span className="tasks-list-muted">{formatPrincipal(task.assignee, t)}</span>
                <span className="tasks-list-muted">{formatTimestamp(task.updatedAt)}</span>
                <div className="tasks-list-actions">
                  <button
                    className="tasks-icon-button"
                    disabled={!task.threadId}
                    onClick={() => {
                      if (task.threadId) {
                        onOpenThread(task.threadId);
                      }
                    }}
                    title={t('Open thread')}
                    type="button"
                  >
                    <MessageSquare aria-hidden size={14} strokeWidth={1.8} />
                  </button>
                  <button
                    className="tasks-move-button"
                    disabled={busy}
                    onClick={() => {
                      void moveTask(task, next.status);
                    }}
                    type="button"
                  >
                    {busy ? t('Saving…') : t(next.label)}
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
