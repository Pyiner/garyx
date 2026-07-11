import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type DragEvent,
  type FormEvent,
} from 'react';
import { createPortal } from 'react-dom';
import {
  CheckCircle2,
  Columns3,
  GitBranch,
  Laptop,
  List,
  MessageSquare,
  Plus,
  Play,
  RefreshCcw,
  RotateCcw,
  Send,
  StopCircle,
  Trash,
  UserPlus,
  type LucideIcon,
} from 'lucide-react';

import type {
  DesktopBotConsoleSummary,
  DesktopCustomAgent,
  DesktopTaskNotificationTarget,
  DesktopTaskPrincipal,
  DesktopTaskStatus,
  DesktopTaskSummary,
  DesktopWorkspace,
  DesktopWorkspaceMode,
} from '@shared/contracts';

import { useI18n, type Translate } from '../../i18n';
import type { ToastTone } from '../../toast';
import { getDesktopApi } from '../../platform/desktop-api';
import { useChannelPluginCatalog } from '../../channel-plugins/useChannelPluginCatalog';
import { ChannelLogo } from '../../channel-logo';
import {
  Field,
  FieldGroup,
  FieldLabel,
} from '../../components/ui/field';
import {
  DropdownMenu,
  DropdownMenuGroup,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuTrigger,
} from '../../components/ui/dropdown-menu';
import {
  FloatingActionMenuContent,
  FloatingActionMenuItem,
  FloatingActionMenuSubContent,
  FloatingActionMenuSubTrigger,
} from '../../components/ui/floating-action-menu';
import { Input } from '../../components/ui/input';
import { WorkspacePathPicker } from '../../components/WorkspacePathPicker';
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from '../../components/ui/select';
import { Textarea } from '../../components/ui/textarea';
import {
  buildAgentPickerOptions,
} from '../agent-options';
import { AgentOptionRow } from './AgentOptionAvatar';
import { AgentsIcon, MoreDotsIcon } from '../icons';

type TasksPanelProps = {
  agents: DesktopCustomAgent[];
  botGroups: DesktopBotConsoleSummary[];
  workspaces: DesktopWorkspace[];
  workspaceMutation: string | null;
  onAddWorkspace: (path: string) => Promise<DesktopWorkspace | null>;
  onOpenThread: (threadId: string) => void;
  onTaskCreated?: () => void;
  onToast: (message: string, tone?: ToastTone) => void;
};

type TaskViewMode = 'board' | 'list';

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

const TASK_DRAG_MIME = 'application/x-garyx-task-id';
const ALL_BOTS_FILTER_VALUE = '__all_bots__';
const UNASSIGNED_ASSIGNEE_VALUE = '__unassigned__';

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

function taskStatusMenuIcon(status: DesktopTaskStatus): LucideIcon {
  switch (status) {
    case 'todo':
      return Play;
    case 'in_progress':
      return Send;
    case 'in_review':
      return CheckCircle2;
    case 'done':
      return RotateCcw;
  }
}

function taskCountLabel(count: number, t: Translate): string {
  return t('{count} tasks', { count });
}

function taskBotFilterValue(group: DesktopBotConsoleSummary): string {
  const channel = group.channel.trim();
  const accountId = group.accountId.trim();
  return channel && accountId ? `${channel}:${accountId}` : group.id;
}

function taskBotFilterLabel(group: DesktopBotConsoleSummary): string {
  return group.title || taskBotFilterValue(group);
}

function TaskBotFilterOption({
  allBots = false,
  group,
  iconDataUrl,
  label,
}: {
  allBots?: boolean;
  group?: DesktopBotConsoleSummary | null;
  iconDataUrl?: string | null;
  label: string;
}) {
  return (
    <span className="tasks-bot-filter-option">
      {allBots ? (
        <span aria-hidden className="tasks-bot-filter-all-icon">
          <AgentsIcon />
        </span>
      ) : (
        <ChannelLogo
          channel={group?.channel || 'bot'}
          className="channel-logo tasks-bot-filter-logo"
          iconDataUrl={iconDataUrl}
          fallbackLabel={label}
        />
      )}
      <span className="tasks-bot-filter-label">{label}</span>
    </span>
  );
}

function taskNotificationTargetFromSelection(
  value: string,
  botGroups: DesktopBotConsoleSummary[],
): DesktopTaskNotificationTarget | null {
  if (value === 'none') {
    return { kind: 'none' };
  }
  if (!value.startsWith('bot:')) {
    return null;
  }
  const botId = value.slice('bot:'.length);
  const group = botGroups.find((candidate) => candidate.id === botId);
  if (!group) {
    return null;
  }
  return {
    kind: 'bot',
    channel: group.channel,
    accountId: group.accountId,
  };
}

export function TasksPanel({
  agents,
  botGroups,
  workspaces,
  workspaceMutation,
  onAddWorkspace,
  onOpenThread,
  onTaskCreated,
  onToast,
}: TasksPanelProps) {
  const { t } = useI18n();
  const { entries: pluginCatalog } = useChannelPluginCatalog();
  const [viewMode, setViewMode] = useState<TaskViewMode>('board');
  const [tasks, setTasks] = useState<DesktopTaskSummary[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [botFilter, setBotFilter] = useState('');
  const [mutatingTaskId, setMutatingTaskId] = useState<string | null>(null);
  const [draftOpen, setDraftOpen] = useState(false);
  const [draftTitle, setDraftTitle] = useState('');
  const [draftBody, setDraftBody] = useState('');
  const [draftAssignee, setDraftAssignee] = useState('');
  const [draftWorkspaceDir, setDraftWorkspaceDir] = useState('');
  const [draftWorkspaceMode, setDraftWorkspaceMode] =
    useState<DesktopWorkspaceMode>('local');
  const [draftWorkspaceGitStatus, setDraftWorkspaceGitStatus] = useState<{
    workspacePath: string;
    isGitRepo: boolean;
  } | null>(null);
  const [draftNotificationTarget, setDraftNotificationTarget] = useState('none');
  const [creating, setCreating] = useState(false);
  const [draggingTaskId, setDraggingTaskId] = useState<string | null>(null);
  const [dropStatus, setDropStatus] = useState<DesktopTaskStatus | null>(null);
  const draggingTaskIdValue = useRef<string | null>(null);
  const agentOptions = useMemo(
    () => buildAgentPickerOptions(agents, { labelStyle: 'display' }),
    [agents],
  );
  const botFilterOptions = useMemo(() => {
    const seen = new Set<string>();
    return botGroups.flatMap((group) => {
      const value = taskBotFilterValue(group);
      if (!value || seen.has(value)) {
        return [];
      }
      seen.add(value);
      return [{
        group,
        id: group.id || value,
        label: taskBotFilterLabel(group),
        value,
      }];
    });
  }, [botGroups]);

  const iconDataUrlByChannel = useMemo(
    () =>
      new Map(
        (pluginCatalog || []).map((entry) => [
          entry.id.toLowerCase(),
          entry.icon_data_url || null,
        ]),
      ),
    [pluginCatalog],
  );

  const selectedBotFilterOption = botFilter
    ? botFilterOptions.find((option) => option.value === botFilter) || null
    : null;
  const selectableWorkspaces = useMemo(
    () =>
      workspaces.filter(
        (workspace) =>
          workspace.available &&
          workspace.path &&
          workspace.kind === 'local',
      ),
    [workspaces],
  );
  const draftWorktreeCapable = Boolean(
    draftWorkspaceGitStatus?.workspacePath === draftWorkspaceDir &&
      draftWorkspaceGitStatus.isGitRepo,
  );

  useEffect(() => {
    let cancelled = false;
    setDraftWorkspaceGitStatus(null);
    if (!draftWorkspaceDir.trim()) {
      setDraftWorkspaceMode('local');
      return;
    }
    void window.garyxDesktop
      .getWorkspaceGitStatus({ workspacePath: draftWorkspaceDir })
      .then((status) => {
        if (cancelled) return;
        setDraftWorkspaceGitStatus({
          workspacePath: draftWorkspaceDir,
          isGitRepo: status.isGitRepo,
        });
        if (!status.isGitRepo) {
          setDraftWorkspaceMode('local');
        }
      })
      .catch(() => {
        if (cancelled) return;
        setDraftWorkspaceGitStatus(null);
        setDraftWorkspaceMode('local');
      });
    return () => {
      cancelled = true;
    };
  }, [draftWorkspaceDir]);

  const loadTasks = useCallback(async (options?: { silent?: boolean }) => {
    if (!options?.silent) {
      setLoading(true);
    }
    setError(null);
    try {
      const page = await getDesktopApi().listTasks({
        includeDone: true,
        sourceBot: botFilter || null,
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
  }, [botFilter]);

  useEffect(() => {
    void loadTasks();
  }, [loadTasks]);

  useEffect(() => {
    if (botFilter && !botFilterOptions.some((option) => option.value === botFilter)) {
      setBotFilter('');
    }
  }, [botFilter, botFilterOptions]);

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
    setMutatingTaskId(task.taskId);
    try {
      await getDesktopApi().updateTaskStatus({
        taskId: task.taskId,
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
      setMutatingTaskId(null);
    }
  }

  async function stopTask(task: DesktopTaskSummary) {
    if (task.status !== 'in_progress') {
      return;
    }
    setMutatingTaskId(task.taskId);
    try {
      await getDesktopApi().stopTask({ taskId: task.taskId });
      await loadTasks({ silent: true });
      onToast(t('Task stopped.'), 'success');
    } catch (stopError) {
      onToast(
        stopError instanceof Error ? stopError.message : t('Task stop failed.'),
        'error',
      );
    } finally {
      setMutatingTaskId(null);
    }
  }

  async function assignTask(task: DesktopTaskSummary, principal: string) {
    if (task.assignee || !principal.trim()) {
      return;
    }
    setMutatingTaskId(task.taskId);
    try {
      await getDesktopApi().assignTask({
        taskId: task.taskId,
        principal,
      });
      await loadTasks({ silent: true });
      onToast(t('Task assigned.'), 'success');
    } catch (assignError) {
      onToast(
        assignError instanceof Error ? assignError.message : t('Task assign failed.'),
        'error',
      );
    } finally {
      setMutatingTaskId(null);
    }
  }

  async function deleteTask(task: DesktopTaskSummary) {
    const confirmed = window.confirm(t(
      'Delete task {taskId}? The task will leave task lists, but the backing thread and transcript stay available.',
      { taskId: task.taskId || `#TASK-${task.number}` },
    ));
    if (!confirmed) {
      return;
    }
    setMutatingTaskId(task.taskId);
    try {
      await getDesktopApi().deleteTask({ taskId: task.taskId });
      setTasks((current) => current.filter((candidate) => candidate.taskId !== task.taskId));
      setTotal((current) => Math.max(0, current - 1));
      await loadTasks({ silent: true });
      onToast(t('Task deleted.'), 'success');
    } catch (deleteError) {
      onToast(
        deleteError instanceof Error ? deleteError.message : t('Task delete failed.'),
        'error',
      );
    } finally {
      setMutatingTaskId(null);
    }
  }

  function draggedTask(event: DragEvent<HTMLElement>): DesktopTaskSummary | null {
    const taskId =
      event.dataTransfer.getData(TASK_DRAG_MIME) ||
      event.dataTransfer.getData('text/plain') ||
      draggingTaskIdValue.current ||
      draggingTaskId;
    return tasks.find((task) => task.taskId === taskId) || null;
  }

  function handleColumnDragOver(event: DragEvent<HTMLElement>, status: DesktopTaskStatus) {
    const task = draggedTask(event);
    if (!task || task.status === status || mutatingTaskId === task.taskId) {
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
    setDraggingTaskId(null);
    draggingTaskIdValue.current = null;
    if (!task || task.status === status || mutatingTaskId === task.taskId) {
      return;
    }
    void moveTask(task, status);
  }

  function resetDraft() {
    setDraftOpen(false);
    setDraftTitle('');
    setDraftBody('');
    setDraftAssignee('');
    setDraftWorkspaceDir('');
    setDraftWorkspaceMode('local');
    setDraftNotificationTarget('none');
  }

  function openTaskPrimary(task: DesktopTaskSummary) {
    if (task.threadId) {
      onOpenThread(task.threadId);
    }
  }

  async function submitTask(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const title = draftTitle.trim();
    if (!title) {
      return;
    }
    const notificationTarget = taskNotificationTargetFromSelection(
      draftNotificationTarget,
      botGroups,
    );
    if (!notificationTarget) {
      onToast(t('Choose Do not notify or a bot.'), 'error');
      return;
    }

    const assignee = draftAssignee.trim();
    if (!assignee) {
      onToast(t('Choose an agent.'), 'error');
      return;
    }
    const executor = assignee
      ? { type: 'agent' as const, agentId: assignee }
      : null;
    setCreating(true);
    try {
      await getDesktopApi().createTask({
        title,
        body: draftBody.trim() || null,
        executor,
        start: Boolean(executor),
        workspaceDir: draftWorkspaceDir.trim() || null,
        workspaceMode: draftWorkspaceMode,
        notificationTarget,
      });
      onTaskCreated?.();
      resetDraft();
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

  function handleDraftWorkspaceChange(value: string) {
    setDraftWorkspaceDir(value);
    setDraftWorkspaceMode('local');
  }

  const visibleCount = tasks.length;
  const headerCount = loading
      ? t('Loading tasks…')
      : taskCountLabel(total || visibleCount, t);

  const renderTaskOverflowMenu = (task: DesktopTaskSummary, busy: boolean) => {
    const next = nextStatus(task.status);
    const StatusIcon = taskStatusMenuIcon(task.status);
    const taskMenuLabel = t('More actions for {name}', {
      name: task.taskId || `#TASK-${task.number}`,
    });
    return (
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            aria-label={taskMenuLabel}
            className="tasks-icon-button"
            disabled={busy}
            title={taskMenuLabel}
            type="button"
          >
            <MoreDotsIcon size={14} />
          </button>
        </DropdownMenuTrigger>
        <FloatingActionMenuContent align="end">
          <DropdownMenuGroup>
            {!task.assignee ? (
              agents.length ? (
                <DropdownMenuSub>
                  <FloatingActionMenuSubTrigger
                    className="tasks-menu-subtrigger"
                    disabled={busy}
                  >
                    <UserPlus aria-hidden />
                    {t('Assign to')}
                  </FloatingActionMenuSubTrigger>
                  <FloatingActionMenuSubContent sideOffset={6}>
                    {agentOptions.map((option) => {
                      return (
                        <FloatingActionMenuItem
                          className="tasks-agent-menu-item"
                          disabled={busy}
                          key={option.id}
                          onSelect={() => {
                            void assignTask(task, option.id);
                          }}
                        >
                          <AgentOptionRow
                            option={option}
                          />
                        </FloatingActionMenuItem>
                      );
                    })}
                  </FloatingActionMenuSubContent>
                </DropdownMenuSub>
              ) : (
                <FloatingActionMenuItem disabled>
                  {t('No agents available')}
                </FloatingActionMenuItem>
              )
            ) : null}
            <FloatingActionMenuItem
              disabled={busy}
              onSelect={() => {
                void moveTask(task, next.status);
              }}
            >
              <StatusIcon aria-hidden />
              {t(next.label)}
            </FloatingActionMenuItem>
          </DropdownMenuGroup>
          <DropdownMenuSeparator />
          <DropdownMenuGroup>
            <FloatingActionMenuItem
              disabled={busy}
              onSelect={() => {
                void deleteTask(task);
              }}
              variant="destructive"
            >
              <Trash aria-hidden />
              {t('Delete task')}
            </FloatingActionMenuItem>
          </DropdownMenuGroup>
        </FloatingActionMenuContent>
      </DropdownMenu>
    );
  };

  const renderTaskCard = (task: DesktopTaskSummary) => {
    const busy = mutatingTaskId === task.taskId;
    const dragging = draggingTaskId === task.taskId;
    return (
      <article
        className={`tasks-card ${dragging ? 'is-dragging' : ''}`}
        draggable={!busy}
        key={task.taskId}
        onDragEnd={() => {
          draggingTaskIdValue.current = null;
          setDraggingTaskId(null);
          setDropStatus(null);
        }}
        onDragStart={(event) => {
          event.dataTransfer.effectAllowed = 'move';
          event.dataTransfer.setData(TASK_DRAG_MIME, task.taskId);
          event.dataTransfer.setData('text/plain', task.taskId);
          draggingTaskIdValue.current = task.taskId;
          setDraggingTaskId(task.taskId);
        }}
      >
        <div className="tasks-card-topline">
          <span className="tasks-card-id">{task.taskId || `#TASK-${task.number}`}</span>
        </div>
        <button
          className="tasks-card-title"
          disabled={!task.threadId}
          onClick={() => {
            openTaskPrimary(task);
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
            {task.status === 'in_progress' ? (
              <button
                aria-label={t('Stop task')}
                className="tasks-icon-button"
                disabled={busy}
                onClick={() => {
                  void stopTask(task);
                }}
                title={t('Stop task')}
                type="button"
              >
                <StopCircle aria-hidden size={14} strokeWidth={1.8} />
              </button>
            ) : null}
            {renderTaskOverflowMenu(task, busy)}
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
            <h1 className="tasks-page-title">{t('Tasks')}</h1>
          </div>
          <p className="tasks-page-subtitle">{headerCount}</p>
        </div>
        <div className="tasks-header-actions">
          <Field className="tasks-filter-control" orientation="horizontal">
            <FieldLabel className="sr-only" htmlFor="tasks-bot-filter-select">
              {t('Bot')}
            </FieldLabel>
            <Select
              value={botFilter || ALL_BOTS_FILTER_VALUE}
              onValueChange={(value) => {
                setBotFilter(value === ALL_BOTS_FILTER_VALUE ? '' : value);
              }}
            >
              <SelectTrigger
                aria-label={t('Filter by bot')}
                className="tasks-filter-trigger"
                id="tasks-bot-filter-select"
                size="sm"
              >
                <span aria-hidden className="tasks-filter-trigger-label">{t('Bot')}</span>
                <TaskBotFilterOption
                  group={selectedBotFilterOption?.group || null}
                  iconDataUrl={
                    selectedBotFilterOption
                      ? iconDataUrlByChannel.get(selectedBotFilterOption.group.channel.toLowerCase()) || null
                      : null
                  }
                  allBots={!selectedBotFilterOption}
                  label={selectedBotFilterOption?.label || t('All bots')}
                />
              </SelectTrigger>
              <SelectContent align="end" className="tasks-bot-filter-content" position="popper" sideOffset={4}>
                <SelectGroup>
                  <SelectItem textValue={t('All bots')} value={ALL_BOTS_FILTER_VALUE}>
                    <TaskBotFilterOption allBots label={t('All bots')} />
                  </SelectItem>
                  {botFilterOptions.map((option) => (
                    <SelectItem key={option.value} textValue={option.label} value={option.value}>
                      <TaskBotFilterOption
                        group={option.group}
                        iconDataUrl={iconDataUrlByChannel.get(option.group.channel.toLowerCase()) || null}
                        label={option.label}
                      />
                    </SelectItem>
                  ))}
                </SelectGroup>
              </SelectContent>
            </Select>
          </Field>
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
            aria-label={t('New task')}
            className="tasks-primary-button"
            onClick={() => setDraftOpen((current) => !current)}
            type="button"
          >
            <Plus aria-hidden size={14} strokeWidth={1.8} />
            {t('New')}
          </button>
        </div>
      </div>

      {draftOpen && typeof document !== 'undefined' ? createPortal(
        <div className="tasks-modal-backdrop" role="presentation">
          <form
            aria-modal="true"
            className="tasks-create-panel"
            onSubmit={submitTask}
            role="dialog"
          >
            <div className="tasks-create-header">
              <h2>{t('New task')}</h2>
            </div>
            <FieldGroup className="tasks-create-grid">
              <Field className="tasks-field tasks-title-field">
                <FieldLabel>{t('Title')}</FieldLabel>
                <Input
                  autoFocus
                  onChange={(event) => setDraftTitle(event.target.value)}
                  placeholder={t('Task title')}
                  value={draftTitle}
                />
              </Field>
              <div className="tasks-executor-section tasks-field-full">
                <div className="tasks-executor-heading">
                  <FieldLabel>{t('Executor')}</FieldLabel>
                </div>
                <div className="tasks-executor-panel">
                  <Select
                    value={draftAssignee || UNASSIGNED_ASSIGNEE_VALUE}
                    onValueChange={(value) => {
                      setDraftAssignee(value === UNASSIGNED_ASSIGNEE_VALUE ? '' : value);
                    }}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent
                      className="tasks-create-select-content"
                      position="popper"
                      sideOffset={4}
                    >
                      <SelectGroup>
                        <SelectLabel>{t('Agents')}</SelectLabel>
                        <SelectItem value={UNASSIGNED_ASSIGNEE_VALUE}>
                          {t('Unassigned')}
                        </SelectItem>
                        {agentOptions.map((option) => (
                          <SelectItem key={option.id} value={option.id}>
                            <AgentOptionRow option={option} />
                          </SelectItem>
                        ))}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                </div>
              </div>
              <Field className="tasks-field tasks-field-full">
                <FieldLabel>{t('Workspace')}</FieldLabel>
                <div className="tasks-workspace-controls">
                  <WorkspacePathPicker
                    addWorkspaceLabel={
                      workspaceMutation === 'add'
                        ? t('Opening folder…')
                        : t('Add workspace')
                    }
                    onAddWorkspace={onAddWorkspace}
                    onChange={handleDraftWorkspaceChange}
                    placeholder={t('Select a workspace')}
                    value={draftWorkspaceDir}
                    workspaces={selectableWorkspaces}
                  />
                  {draftWorktreeCapable ? (
                    <div className="tasks-workspace-mode-row">
                      <span className="tasks-workspace-mode-label">
                        {t('Workspace mode')}
                      </span>
                      <Select
                        value={draftWorkspaceMode}
                        onValueChange={(value) => {
                          setDraftWorkspaceMode(value as DesktopWorkspaceMode);
                        }}
                      >
                        <SelectTrigger
                          aria-label={t('Workspace mode')}
                          className="tasks-workspace-mode-trigger"
                        >
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent
                          align="start"
                          className="tasks-workspace-mode-content"
                          position="popper"
                          sideOffset={4}
                        >
                          <SelectGroup>
                            <SelectLabel>{t('Workspace mode')}</SelectLabel>
                            <SelectItem value="local">
                              <Laptop aria-hidden size={15} strokeWidth={1.8} />
                              {t('Local mode')}
                            </SelectItem>
                            <SelectItem value="worktree">
                              <GitBranch aria-hidden size={15} strokeWidth={1.8} />
                              {t('Worktree')}
                            </SelectItem>
                          </SelectGroup>
                        </SelectContent>
                      </Select>
                    </div>
                  ) : null}
                </div>
              </Field>
              <Field className="tasks-field tasks-field-full">
                <FieldLabel>{t('Notification')}</FieldLabel>
                <Select
                  value={draftNotificationTarget}
                  onValueChange={setDraftNotificationTarget}
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent
                    className="tasks-create-select-content"
                    position="popper"
                    sideOffset={4}
                  >
                    <SelectGroup>
                      <SelectLabel>{t('Notification')}</SelectLabel>
                      <SelectItem value="none">{t('Do not notify')}</SelectItem>
                      {botGroups.map((group) => (
                        <SelectItem key={group.id} value={`bot:${group.id}`}>
                          {group.title || `${group.channel}:${group.accountId}`}
                        </SelectItem>
                      ))}
                    </SelectGroup>
                  </SelectContent>
                </Select>
              </Field>
              <Field className="tasks-field tasks-field-full">
                <FieldLabel>{t('Body')}</FieldLabel>
                <Textarea
                  onChange={(event) => setDraftBody(event.target.value)}
                  placeholder={t('Optional task detail')}
                  value={draftBody}
                />
              </Field>
            </FieldGroup>
            <div className="tasks-create-actions">
              <button className="tasks-secondary-button" onClick={() => setDraftOpen(false)} type="button">
                {t('Cancel')}
              </button>
              <button
                className="tasks-primary-button"
                disabled={
                  !draftTitle.trim() ||
                  creating ||
                  !draftAssignee.trim()
                }
                type="submit"
              >
                <Plus aria-hidden size={14} strokeWidth={1.8} />
                {creating ? t('Creating…') : t('Create')}
              </button>
            </div>
          </form>
        </div>,
        document.body
      ) : null}

      {error ? (
        <div className="tasks-state tasks-state-error">{error}</div>
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
            const busy = mutatingTaskId === task.taskId;
            return (
              <div className="tasks-list-row" key={task.taskId}>
                <button
                  className="tasks-list-title"
                  disabled={!task.threadId}
                  onClick={() => {
                    openTaskPrimary(task);
                  }}
                  type="button"
                >
                  <span>{task.title}</span>
                  <small>{task.taskId}</small>
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
                  {task.status === 'in_progress' ? (
                    <button
                      aria-label={t('Stop task')}
                      className="tasks-icon-button"
                      disabled={busy}
                      onClick={() => {
                        void stopTask(task);
                      }}
                      title={t('Stop task')}
                      type="button"
                    >
                      <StopCircle aria-hidden size={14} strokeWidth={1.8} />
                    </button>
                  ) : null}
                  {renderTaskOverflowMenu(task, busy)}
                </div>
              </div>
            );
          })}
        </div>
      )}

    </div>
  );
}
