import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  Copy,
  FileText,
  FolderOpen,
  Globe,
  ListTodo,
  MessageSquare,
  PanelRightClose,
  PanelRightOpen,
  Plus,
  RefreshCcw,
  Terminal as TerminalIcon,
  X,
} from "lucide-react";

import type {
  BrowserAnnotationCommentRequest,
  DesktopTaskPrincipal,
  DesktopTaskStatus,
  DesktopTaskSummary,
  DesktopTerminalEvent,
  DesktopTerminalState,
  DesktopWorkspaceFilePreview,
  DesktopWorkspaceMode,
} from "@shared/contracts";

import { WorkspaceFilePreview } from "../../workspace-file-preview";
import { Package } from "lucide-react";
import { PanelIcon } from "../icons";
import { useI18n } from "../../i18n";

// Perf round 2026-07: xterm (and its CSS) only load when the Terminal
// tool tab actually mounts — it was ~90 modules inside the boot bundle.
const SideTerminalTool = lazy(() =>
  import("./SideTerminalTool").then((module) => ({
    default: module.SideTerminalTool,
  })),
);
import { workspaceFileAbsolutePath } from "../workspace-helpers";
import { CapsuleLivePreviewFrame } from "./CapsuleLivePreviewFrame";
import {
  capsuleIdFromTabKey,
  capsuleTabKey,
  closeTab,
  isCapsuleTabKey,
  shouldCollapseFileDirectoryForPreview,
  workspacePreviewDirectoryCollapseKey,
  type SideTabKey,
  type ThreadSideToolId,
} from "./side-tools-panel-model";

const SidePanelBrowserPage = lazy(() =>
  import("../../BrowserPage").then((module) => ({ default: module.BrowserPage }))
);

export type { ThreadSideToolId } from "./side-tools-panel-model";

/** A capsule open as a tab in the dock (#TASK-1470). */
export type SideCapsuleTab = {
  capsuleId: string;
  revision: number;
  title: string;
};

export type SideToolWorkspaceFile = {
  name: string;
  relativePath: string;
  absolutePath: string;
  mediaType?: string | null;
};

type ThreadSideToolsPanelProps = {
  activeWorkspaceName?: string | null;
  activeWorkspacePath?: string | null;
  activeThreadId?: string | null;
  selectedWorkspaceFile?: SideToolWorkspaceFile | null;
  workspaceBranch?: string | null;
  workspaceDirectoryPanel: ReactNode;
  workspaceFileFilter: string;
  workspaceFilePreview?: DesktopWorkspaceFilePreview | null;
  workspaceFilePreviewError?: string | null;
  workspaceFilePreviewLoading?: boolean;
  workspaceMode?: DesktopWorkspaceMode | null;
  workspacePreviewOpen?: boolean;
  workspacePreviewTitle?: string;
  sideChatPanel: ReactNode;
  /** Whether a workspace is attached. Built-in tools (files/terminal/…) need
   * one; without it the dock only hosts capsule tabs and hides the add menu. */
  hasWorkspace: boolean;
  /** Capsules currently open as tabs (gateway-owned; #TASK-1470). */
  openCapsuleTabs: SideCapsuleTab[];
  /** Capsule to activate next; consumed via onActivatePendingCapsuleHandled. */
  pendingActiveCapsuleId?: string | null;
  onActivatePendingCapsuleHandled: () => void;
  onCloseCapsuleTab: (capsuleId: string) => void;
  onCloseWorkspacePreview?: () => void;
  onLocalFileLinkClick?: (absolutePath: string) => void;
  onRevealSelectedWorkspaceFile?: () => Promise<void> | void;
  onAddBrowserAnnotationComment: (request: BrowserAnnotationCommentRequest) => void;
  onCloseSideTools: () => void;
  onOpenTaskThread?: (task: DesktopTaskSummary) => Promise<void> | void;
  onOpenSideChat: () => void;
  onWorkspaceFileFilterChange: (value: string) => void;
};

type ToolDescriptor = {
  id: ThreadSideToolId;
  label: string;
  shortcut: string;
  icon: typeof FileText;
};

const TASK_STATUS_LABELS: Record<DesktopTaskStatus, string> = {
  todo: "Todo",
  in_progress: "In Progress",
  in_review: "In Review",
  done: "Done",
};

const TASK_STATUS_TONES: Record<DesktopTaskStatus, string> = {
  todo: "todo",
  in_progress: "progress",
  in_review: "review",
  done: "done",
};

function formatTaskPrincipal(
  principal: DesktopTaskPrincipal | null | undefined,
  t: ReturnType<typeof useI18n>["t"],
): string {
  if (!principal) {
    return t("Unassigned");
  }
  if (principal.kind === "human") {
    return `@${principal.userId}`;
  }
  return principal.agentId;
}

function formatTaskTimestamp(value?: string | null): string {
  if (!value) {
    return "";
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return "";
  }
  const sameDay = date.toDateString() === new Date().toDateString();
  return new Intl.DateTimeFormat(
    undefined,
    sameDay
      ? { hour: "numeric", minute: "2-digit" }
      : { month: "short", day: "numeric" },
  ).format(date);
}

function isTasksDisabled(error: string | null): boolean {
  return Boolean(error && /tasks are disabled|TasksDisabled/i.test(error));
}

function taskDisplayId(task: DesktopTaskSummary): string {
  return task.taskId || `#TASK-${task.number}`;
}

function taskTabLabel(task: DesktopTaskSummary): string {
  return task.title.trim() || taskDisplayId(task);
}

function SideThreadTasksTool({
  sourceThreadId,
  onOpenTaskThread,
}: {
  sourceThreadId?: string | null;
  onOpenTaskThread?: (task: DesktopTaskSummary) => Promise<void> | void;
}) {
  const { t } = useI18n();
  const [tasks, setTasks] = useState<DesktopTaskSummary[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestIdRef = useRef(0);

  const loadTasks = useCallback(async (options?: { silent?: boolean }) => {
    const threadId = sourceThreadId?.trim() || "";
    const requestId = requestIdRef.current + 1;
    requestIdRef.current = requestId;

    if (!threadId) {
      setTasks([]);
      setTotal(0);
      setError(null);
      setLoading(false);
      return;
    }

    if (!options?.silent) {
      setLoading(true);
    }
    setError(null);
    try {
      const page = await window.garyxDesktop.listTasks({
        includeDone: true,
        sourceThread: threadId,
        limit: 200,
      });
      if (requestIdRef.current !== requestId) {
        return;
      }
      setTasks(page.tasks);
      setTotal(page.total);
    } catch (loadError) {
      if (requestIdRef.current !== requestId) {
        return;
      }
      setTasks([]);
      setTotal(0);
      setError(
        loadError instanceof Error
          ? loadError.message
          : String(loadError || "Failed to load tasks"),
      );
    } finally {
      if (requestIdRef.current === requestId && !options?.silent) {
        setLoading(false);
      }
    }
  }, [sourceThreadId]);

  useEffect(() => {
    void loadTasks();
  }, [loadTasks]);

  const visibleCount = tasks.length;
  const disabled = isTasksDisabled(error);
  const countLabel = loading
    ? t("Loading tasks…")
    : t("{count} tasks", { count: total || visibleCount });

  return (
    <div className="side-tool-tasks" aria-busy={loading}>
      <header className="side-tool-tasks-header">
        <div className="side-tool-tasks-title-block">
          <div className="side-tool-tasks-title-row">
            <ListTodo aria-hidden size={16} strokeWidth={1.8} />
            <strong>{t("Tasks")}</strong>
          </div>
          <span>{countLabel}</span>
        </div>
        <button
          aria-label={t("Refresh")}
          className="codex-icon-button side-tool-tasks-refresh"
          disabled={!sourceThreadId || loading}
          onClick={() => {
            void loadTasks();
          }}
          title={t("Refresh")}
          type="button"
        >
          <RefreshCcw aria-hidden size={14} strokeWidth={1.8} />
        </button>
      </header>

      {error ? (
        <div
          className={`side-tool-tasks-state ${
            disabled ? "is-warning" : "is-error"
          }`}
        >
          {disabled ? t("Tasks are disabled in the gateway config.") : error}
        </div>
      ) : null}

      {!sourceThreadId ? (
        <div className="side-tool-tasks-empty">
          {t("Open a thread to see its tasks.")}
        </div>
      ) : loading && !tasks.length ? (
        <div className="side-tool-tasks-empty">{t("Loading tasks…")}</div>
      ) : !error && !tasks.length ? (
        <div className="side-tool-tasks-empty">
          {t("No tasks from this thread yet.")}
        </div>
      ) : (
        <div className="side-tool-tasks-list">
          {tasks.map((task) => {
            const taskId = taskDisplayId(task);
            const canOpen = Boolean(task.threadId && onOpenTaskThread);
            return (
              <article className="side-tool-task-card" key={task.taskId || taskId}>
                <div className="side-tool-task-topline">
                  <span className="side-tool-task-id">{taskId}</span>
                  <span
                    className={`tasks-status-chip tone-${TASK_STATUS_TONES[task.status]}`}
                  >
                    {t(TASK_STATUS_LABELS[task.status])}
                  </span>
                </div>
                <button
                  className="side-tool-task-title"
                  disabled={!canOpen}
                  onClick={() => {
                    if (task.threadId && onOpenTaskThread) {
                      void onOpenTaskThread(task);
                    }
                  }}
                  type="button"
                >
                  {task.title}
                </button>
                <div className="side-tool-task-meta">
                  <span>
                    {t("assignee")} {formatTaskPrincipal(task.assignee, t)}
                  </span>
                  {formatTaskTimestamp(task.updatedAt) ? (
                    <span>{formatTaskTimestamp(task.updatedAt)}</span>
                  ) : null}
                </div>
                {task.threadId ? (
                  <button
                    className="side-tool-task-open"
                    disabled={!onOpenTaskThread}
                    onClick={() => {
                      if (onOpenTaskThread) {
                        void onOpenTaskThread(task);
                      }
                    }}
                    type="button"
                  >
                    <MessageSquare aria-hidden size={13} strokeWidth={1.8} />
                    {t("Open thread")}
                  </button>
                ) : null}
              </article>
            );
          })}
        </div>
      )}
    </div>
  );
}

export function ThreadSideToolsPanel({
  activeThreadId,
  activeWorkspacePath,
  selectedWorkspaceFile,
  sideChatPanel,
  workspaceDirectoryPanel,
  workspaceFileFilter,
  workspaceFilePreview = null,
  workspaceFilePreviewError = null,
  workspaceFilePreviewLoading = false,
  workspacePreviewOpen = false,
  workspacePreviewTitle = "",
  hasWorkspace,
  openCapsuleTabs,
  pendingActiveCapsuleId = null,
  onActivatePendingCapsuleHandled,
  onCloseCapsuleTab,
  onCloseWorkspacePreview,
  onLocalFileLinkClick,
  onRevealSelectedWorkspaceFile,
  onAddBrowserAnnotationComment,
  onCloseSideTools,
  onOpenTaskThread,
  onOpenSideChat,
  onWorkspaceFileFilterChange,
}: ThreadSideToolsPanelProps) {
  const { t } = useI18n();
  const tools = useMemo<ToolDescriptor[]>(
    () => [
      { id: "files", label: t("Files"), shortcut: "⌘P", icon: FileText },
      { id: "tasks", label: t("Tasks"), shortcut: "", icon: ListTodo },
      { id: "chat", label: t("Side Chat"), shortcut: "", icon: MessageSquare },
      { id: "browser", label: t("Browser"), shortcut: "⌘T", icon: Globe },
      { id: "terminal", label: t("Terminal"), shortcut: "⌃`", icon: TerminalIcon },
    ],
    [t],
  );
  // The panel opens with no tool chosen so the body shows a tool picker;
  // workspace file previews still force the Files tool open below.
  const [openTools, setOpenTools] = useState<ThreadSideToolId[]>([]);
  // The active tab can be a built-in tool or a capsule (`capsule:<id>`); the
  // panel owns this single source of truth so closing a tab repicks across
  // both kinds without crossing component boundaries (#TASK-1470).
  const [activeTabKey, setActiveTabKey] = useState<SideTabKey | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [taskThreadTabTitle, setTaskThreadTabTitle] = useState<string | null>(
    null,
  );
  const [filePathCopied, setFilePathCopied] = useState(false);
  const [fileDirectoryCollapsed, setFileDirectoryCollapsed] = useState(false);
  const [browserMenuObstructionBottom, setBrowserMenuObstructionBottom] =
    useState<number | null>(null);
  const addToolShellRef = useRef<HTMLDivElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const previewDirectoryCollapseKeyRef = useRef<string | null>(null);
  const filePathCopiedTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const activeTool =
    activeTabKey && !isCapsuleTabKey(activeTabKey)
      ? tools.find((tool) => tool.id === activeTabKey) || null
      : null;
  const activeCapsuleId =
    activeTabKey && isCapsuleTabKey(activeTabKey)
      ? capsuleIdFromTabKey(activeTabKey)
      : null;
  const FileDirectoryToggleIcon = fileDirectoryCollapsed
    ? PanelRightOpen
    : PanelRightClose;
  const previewCopyPath = selectedWorkspaceFile?.absolutePath ||
    (workspaceFilePreview?.workspacePath && workspaceFilePreview.path
      ? workspaceFileAbsolutePath(workspaceFilePreview.workspacePath, workspaceFilePreview.path)
      : "");
  const shouldShowWorkspacePreview = Boolean(
    workspacePreviewOpen ||
      workspaceFilePreviewLoading ||
      workspaceFilePreviewError ||
      workspaceFilePreview,
  );
  const previewDirectoryCollapseKey = workspacePreviewDirectoryCollapseKey({
    shouldShowWorkspacePreview,
    workspaceFilePreviewPath: workspaceFilePreview?.path,
    workspacePreviewTitle,
  });
  const openToolDescriptors = openTools
    .map((toolId) => tools.find((tool) => tool.id === toolId))
    .filter((tool): tool is ToolDescriptor => Boolean(tool));
  // Built-in tool tabs first, then capsule tabs, in open order. Closing repicks
  // the active tab across this combined list.
  const combinedOpenTabs: SideTabKey[] = [
    ...openTools,
    ...openCapsuleTabs.map((capsule) => capsuleTabKey(capsule.capsuleId)),
  ];
  useEffect(() => {
    return () => {
      if (filePathCopiedTimeoutRef.current) {
        clearTimeout(filePathCopiedTimeoutRef.current);
      }
    };
  }, []);

  useEffect(() => {
    setFilePathCopied(false);
  }, [previewCopyPath]);

  useEffect(() => {
    setTaskThreadTabTitle(null);
  }, [activeThreadId]);

  useEffect(() => {
    if (activeTabKey === "browser") {
      return;
    }

    void window.garyxDesktop.updateBrowserBounds({
      x: 0,
      y: 0,
      width: 0,
      height: 0,
      visible: false,
    });
  }, [activeTabKey]);

  useEffect(() => {
    if (!shouldShowWorkspacePreview) {
      return;
    }
    setOpenTools((current) =>
      current.includes("files") ? current : ["files", ...current],
    );
    setActiveTabKey("files");
  }, [shouldShowWorkspacePreview, workspaceFilePreview?.path, workspacePreviewTitle]);

  // A capsule open request (from a transcript capsule card) opens/activates its
  // tab, then is consumed so re-clicking the same capsule re-activates it.
  useEffect(() => {
    if (!pendingActiveCapsuleId) {
      return;
    }
    setActiveTabKey(capsuleTabKey(pendingActiveCapsuleId));
    onActivatePendingCapsuleHandled();
  }, [pendingActiveCapsuleId, onActivatePendingCapsuleHandled]);

  // If the active capsule tab is removed externally (closed elsewhere or cleared
  // on a thread switch), repick the last remaining tab so nothing dangles.
  useEffect(() => {
    if (!activeTabKey || !isCapsuleTabKey(activeTabKey)) {
      return;
    }
    const stillOpen = openCapsuleTabs.some(
      (capsule) => capsuleTabKey(capsule.capsuleId) === activeTabKey,
    );
    if (stillOpen) {
      return;
    }
    const remaining: SideTabKey[] = [
      ...openTools,
      ...openCapsuleTabs.map((capsule) => capsuleTabKey(capsule.capsuleId)),
    ];
    setActiveTabKey(remaining.length ? remaining[remaining.length - 1] : null);
  }, [openCapsuleTabs, openTools, activeTabKey]);

  useEffect(() => {
    const previousPreviewKey = previewDirectoryCollapseKeyRef.current;
    if (!previewDirectoryCollapseKey) {
      previewDirectoryCollapseKeyRef.current = null;
      return;
    }
    if (
      shouldCollapseFileDirectoryForPreview({
        previousPreviewKey,
        nextPreviewKey: previewDirectoryCollapseKey,
      })
    ) {
      setFileDirectoryCollapsed(true);
    }
    previewDirectoryCollapseKeyRef.current = previewDirectoryCollapseKey;
  }, [previewDirectoryCollapseKey]);

  useEffect(() => {
    if (!menuOpen) {
      return;
    }

    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (target instanceof Node && addToolShellRef.current?.contains(target)) {
        return;
      }
      setMenuOpen(false);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setMenuOpen(false);
      }
    };

    document.addEventListener("pointerdown", handlePointerDown, true);
    document.addEventListener("keydown", handleKeyDown, true);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown, true);
      document.removeEventListener("keydown", handleKeyDown, true);
    };
  }, [menuOpen]);

  useEffect(() => {
    if (!menuOpen || activeTabKey !== "browser") {
      return;
    }

    const handleBrowserMouseDown = () => {
      setMenuOpen(false);
    };
    window.garyxDesktop.subscribeBrowserPageMouseDown(handleBrowserMouseDown);
    return () => {
      window.garyxDesktop.unsubscribeBrowserPageMouseDown(handleBrowserMouseDown);
    };
  }, [activeTabKey, menuOpen]);

  useLayoutEffect(() => {
    if (!menuOpen || activeTabKey !== "browser") {
      setBrowserMenuObstructionBottom(null);
      return;
    }

    const measure = () => {
      const rect = menuRef.current?.getBoundingClientRect();
      setBrowserMenuObstructionBottom(rect ? Math.ceil(rect.bottom + 8) : null);
    };

    measure();
    const observer = new ResizeObserver(measure);
    if (menuRef.current) {
      observer.observe(menuRef.current);
    }
    window.addEventListener("resize", measure);
    window.addEventListener("scroll", measure, true);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", measure, true);
    };
  }, [activeTabKey, menuOpen]);

  async function copySelectedWorkspaceFilePath() {
    if (!previewCopyPath) {
      return;
    }
    await window.garyxDesktop.copyTextToClipboard({ text: previewCopyPath });
    setFilePathCopied(true);
    if (filePathCopiedTimeoutRef.current) {
      clearTimeout(filePathCopiedTimeoutRef.current);
    }
    filePathCopiedTimeoutRef.current = setTimeout(() => {
      setFilePathCopied(false);
      filePathCopiedTimeoutRef.current = null;
    }, 1200);
  }

  function handleBrowserAnnotationCommentRequest(request: BrowserAnnotationCommentRequest) {
    if (!request.comment.trim()) {
      return;
    }
    onAddBrowserAnnotationComment(request);
  }

  function openTool(
    toolId: ThreadSideToolId,
    options?: { taskThreadTabTitle?: string | null },
  ) {
    setOpenTools((current) =>
      current.includes(toolId) ? current : [...current, toolId],
    );
    setActiveTabKey(toolId);
    setMenuOpen(false);
    if (toolId === "chat") {
      setTaskThreadTabTitle(options?.taskThreadTabTitle || null);
      onOpenSideChat();
    }
  }

  async function openTaskThreadInSideChat(task: DesktopTaskSummary) {
    if (!onOpenTaskThread) {
      return;
    }
    try {
      await onOpenTaskThread(task);
      openTool("chat", { taskThreadTabTitle: taskTabLabel(task) });
    } catch {
      // AppShell owns the user-visible error state.
    }
  }

  function closeTabByKey(key: SideTabKey) {
    // Repick the active tab across the combined (built-in + capsule) track, then
    // dispatch the removal to the owning store: capsule tabs are gateway-owned
    // (AppShell), built-in tools are local to this panel.
    const { activeKey } = closeTab(combinedOpenTabs, activeTabKey, key);
    setActiveTabKey(activeKey);
    const capsuleId = capsuleIdFromTabKey(key);
    if (capsuleId !== null) {
      onCloseCapsuleTab(capsuleId);
    } else {
      setOpenTools((current) => current.filter((id) => id !== key));
    }
  }

  function closeSideTools() {
    void window.garyxDesktop.updateBrowserBounds({
      x: 0,
      y: 0,
      width: 0,
      height: 0,
      visible: false,
    });
    onCloseSideTools();
  }

  return (
    <aside
      className={`thread-side-tools-panel is-${activeCapsuleId ? "capsule" : activeTabKey ?? "picker"}-active`}
    >
      <div className="side-tools-tabs">
        <div className="side-tools-tab-cluster">
          <div className="side-tools-tab-track" role="tablist">
            {openToolDescriptors.map((tool) => {
              const Icon = tool.icon;
              const selected = tool.id === activeTabKey;
              const tabLabel =
                tool.id === "chat" && taskThreadTabTitle
                  ? taskThreadTabTitle
                  : tool.label;
              return (
                <div
                  className={`side-tools-tab-shell ${selected ? "is-active" : ""}`}
                  key={tool.id}
                >
                  <button
                    aria-selected={selected}
                    className={`side-tools-tab ${selected ? "is-active" : ""}`}
                    onClick={() => {
                      setActiveTabKey(tool.id);
                      if (tool.id === "chat" && !taskThreadTabTitle) {
                        onOpenSideChat();
                      }
                    }}
                    role="tab"
                    title={tabLabel}
                    type="button"
                  >
                    <Icon aria-hidden size={14} strokeWidth={1.8} />
                    <span className="side-tools-tab-label">{tabLabel}</span>
                  </button>
                  <button
                    aria-label={t("Close")}
                    className="side-tools-tab-close"
                    onPointerDown={(event) => {
                      event.stopPropagation();
                    }}
                    onClick={(event) => {
                      event.stopPropagation();
                      closeTabByKey(tool.id);
                    }}
                    title={t("Close")}
                    type="button"
                  >
                    <X aria-hidden size={12} strokeWidth={1.9} />
                  </button>
                </div>
              );
            })}
            {openCapsuleTabs.map((capsule) => {
              const key = capsuleTabKey(capsule.capsuleId);
              const selected = key === activeTabKey;
              const tabLabel = capsule.title.trim() || t("Untitled Capsule");
              return (
                <div
                  className={`side-tools-tab-shell ${selected ? "is-active" : ""}`}
                  key={key}
                >
                  <button
                    aria-selected={selected}
                    className={`side-tools-tab ${selected ? "is-active" : ""}`}
                    onClick={() => setActiveTabKey(key)}
                    role="tab"
                    title={tabLabel}
                    type="button"
                  >
                    <Package aria-hidden size={14} strokeWidth={1.8} />
                    <span className="side-tools-tab-label">{tabLabel}</span>
                  </button>
                  <button
                    aria-label={t("Close")}
                    className="side-tools-tab-close"
                    onPointerDown={(event) => {
                      event.stopPropagation();
                    }}
                    onClick={(event) => {
                      event.stopPropagation();
                      closeTabByKey(key);
                    }}
                    title={t("Close")}
                    type="button"
                  >
                    <X aria-hidden size={12} strokeWidth={1.9} />
                  </button>
                </div>
              );
            })}
          </div>
          {hasWorkspace ? (
            <div className="side-tools-add-shell" ref={addToolShellRef}>
              <button
                aria-expanded={menuOpen}
                aria-haspopup="menu"
                className="codex-icon-button side-tools-add"
                onClick={() => setMenuOpen((current) => !current)}
                title={t("Add tool")}
                type="button"
              >
                <Plus aria-hidden />
              </button>
              {menuOpen ? (
                <div className="side-tools-menu" ref={menuRef} role="menu">
                  {tools.map((tool) => {
                    const Icon = tool.icon;
                    return (
                      <button
                        className="side-tools-menu-item"
                        key={tool.id}
                        onClick={() => openTool(tool.id)}
                        role="menuitem"
                        type="button"
                      >
                        <Icon aria-hidden size={15} strokeWidth={1.8} />
                        <span>{tool.label}</span>
                        {tool.shortcut ? <kbd>{tool.shortcut}</kbd> : null}
                      </button>
                    );
                  })}
                </div>
              ) : null}
            </div>
          ) : null}
        </div>
        <div className="side-tools-header-actions">
          <button
            aria-label={t("Hide side tools")}
            className="codex-icon-button side-tools-collapse"
            onClick={closeSideTools}
            title={t("Hide side tools")}
            type="button"
          >
            <PanelIcon />
          </button>
        </div>
      </div>

      <div
        className={`side-tools-body is-${
          activeCapsuleId ? "capsule" : activeTool?.id ?? "picker"
        }`}
      >
        {!activeTool && !activeCapsuleId && hasWorkspace ? (
          <div className="side-tools-picker">
            <div className="side-tools-picker-list">
              {tools.map((tool) => {
                const Icon = tool.icon;
                return (
                  <button
                    className="side-tools-picker-item"
                    key={tool.id}
                    onClick={() => openTool(tool.id)}
                    type="button"
                  >
                    <Icon aria-hidden size={15} strokeWidth={1.8} />
                    <span className="side-tools-picker-label">{tool.label}</span>
                    {tool.shortcut ? <kbd>{tool.shortcut}</kbd> : null}
                  </button>
                );
              })}
            </div>
          </div>
        ) : null}
        {openCapsuleTabs.map((capsule) => {
          const key = capsuleTabKey(capsule.capsuleId);
          // Keep only the active capsule's frame mounted (matches the built-in
          // tool bodies); the shared HTML store caches so re-activating reloads
          // fast. `active` gates the iframe HTML fetch.
          if (key !== activeTabKey) {
            return null;
          }
          return (
            <div className="side-tool-capsule" key={key}>
              <CapsuleLivePreviewFrame
                active
                capsuleId={capsule.capsuleId}
                mode="preview"
                revision={capsule.revision}
                title={capsule.title.trim() || t("Untitled Capsule")}
              />
            </div>
          );
        })}
        {activeTool?.id === "files" ? (
          <div
            className={`side-tool-files ${
              fileDirectoryCollapsed ? "is-directory-collapsed" : ""
            }`}
          >
            <section className={`side-tool-file-preview-panel ${shouldShowWorkspacePreview ? "" : "is-empty"}`}>
              {shouldShowWorkspacePreview ? (
                <>
                  <div className="side-tool-file-preview-header">
                    <div className="side-tool-file-preview-copy">
                      <FileText aria-hidden size={14} strokeWidth={1.8} />
                      <span title={workspacePreviewTitle}>
                        {selectedWorkspaceFile?.relativePath ||
                          workspaceFilePreview?.path ||
                          workspaceFilePreview?.name ||
                          workspacePreviewTitle}
                      </span>
                    </div>
                    <div className="side-tool-file-preview-actions">
                      <button
                        aria-label={filePathCopied ? t("Copied") : t("Copy file path")}
                        className={filePathCopied ? "is-copied" : ""}
                        disabled={!previewCopyPath}
                        onClick={() => {
                          void copySelectedWorkspaceFilePath();
                        }}
                        title={filePathCopied ? t("Copied") : t("Copy file path")}
                        type="button"
                      >
                        <Copy aria-hidden size={13} strokeWidth={1.8} />
                      </button>
                      {onRevealSelectedWorkspaceFile ? (
                        <button
                          aria-label={t("Show in Finder")}
                          disabled={!selectedWorkspaceFile}
                          onClick={() => void onRevealSelectedWorkspaceFile()}
                          title={t("Show in Finder")}
                          type="button"
                        >
                          <FolderOpen aria-hidden size={13} strokeWidth={1.8} />
                        </button>
                      ) : null}
                      {onCloseWorkspacePreview ? (
                        <button
                          aria-label={t("Close")}
                          onClick={onCloseWorkspacePreview}
                          title={t("Close")}
                          type="button"
                        >
                          <X aria-hidden size={13} strokeWidth={1.8} />
                        </button>
                      ) : null}
                    </div>
                  </div>
                  {workspaceFilePreviewError ? (
                    <div className="workspace-file-error side-tool-file-preview-error">
                      {workspaceFilePreviewError}
                    </div>
                  ) : null}
                  <div className="side-tool-file-preview-body">
                    {workspaceFilePreviewLoading ? (
                      <div className="workspace-file-empty">{t("Loading preview…")}</div>
                    ) : (
                      <WorkspaceFilePreview
                        onLocalFileLinkClick={onLocalFileLinkClick}
                        preview={workspaceFilePreview}
                      />
                    )}
                  </div>
                </>
              ) : (
                <div className="side-tool-file-preview-empty">
                  <FolderOpen aria-hidden size={30} strokeWidth={1.65} />
                  <strong>{t("Open file")}</strong>
                  <span>{t("Choose from the workspace directory tree")}</span>
                </div>
              )}
            </section>
            <aside className="side-tool-file-browser">
              <div className="side-tool-filter-shell">
                <input
                  aria-label={t("Filter files")}
                  disabled={fileDirectoryCollapsed}
                  onChange={(event) => onWorkspaceFileFilterChange(event.target.value)}
                  placeholder={t("Filter files…")}
                  type="search"
                  value={workspaceFileFilter}
                />
                <button
                  aria-label={
                    fileDirectoryCollapsed
                      ? t("Show file directory")
                      : t("Hide file directory")
                  }
                  aria-pressed={!fileDirectoryCollapsed}
                  className="codex-icon-button side-tools-file-directory-toggle"
                  onClick={() => setFileDirectoryCollapsed((current) => !current)}
                  title={
                    fileDirectoryCollapsed
                      ? t("Show file directory")
                      : t("Hide file directory")
                  }
                  type="button"
                >
                  <FileDirectoryToggleIcon aria-hidden />
                </button>
              </div>
              <div
                aria-hidden={fileDirectoryCollapsed ? true : undefined}
                className="side-tool-file-tree"
              >
                {workspaceDirectoryPanel}
              </div>
            </aside>
          </div>
        ) : null}
        {activeTool?.id === "chat" ? (
          <div className="side-tool-chat-thread">{sideChatPanel}</div>
        ) : null}
        {activeTool?.id === "tasks" ? (
          <SideThreadTasksTool
            onOpenTaskThread={openTaskThreadInSideChat}
            sourceThreadId={activeThreadId}
          />
        ) : null}
        {activeTool?.id === "browser" ? (
          <Suspense
            fallback={<div className="browser-page browser-page-side-panel browser-side-panel-loading" />}
          >
            <SidePanelBrowserPage
              obstructionBottom={browserMenuObstructionBottom}
              onAnnotationCommentRequest={handleBrowserAnnotationCommentRequest}
              variant="side-panel"
            />
          </Suspense>
        ) : null}
        {activeTool?.id === "terminal" ? (
          <Suspense fallback={null}>
            <SideTerminalTool cwd={activeWorkspacePath} />
          </Suspense>
        ) : null}
      </div>
    </aside>
  );
}
