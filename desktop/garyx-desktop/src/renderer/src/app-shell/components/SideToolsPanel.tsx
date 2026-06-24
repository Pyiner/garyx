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
import { FitAddon } from "@xterm/addon-fit";
import { Terminal as XTermTerminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";

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
import { PanelIcon } from "../icons";
import { useI18n } from "../../i18n";
import { workspaceFileAbsolutePath } from "../workspace-helpers";
import {
  shouldCollapseFileDirectoryForPreview,
  workspacePreviewDirectoryCollapseKey,
} from "./side-tools-panel-model";

const SidePanelBrowserPage = lazy(() =>
  import("../../BrowserPage").then((module) => ({ default: module.BrowserPage }))
);

export type ThreadSideToolId = "files" | "tasks" | "chat" | "browser" | "terminal";

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

function activeTerminalSession(state: DesktopTerminalState | null) {
  if (!state?.activeSessionId) {
    return null;
  }
  return state.sessions.find((session) => session.id === state.activeSessionId) || null;
}

const MAX_RENDERER_TERMINAL_OUTPUT_LENGTH = 160_000;

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

function appendTerminalOutput(output: string, data: string): string {
  const nextOutput = `${output}${data}`;
  return nextOutput.length > MAX_RENDERER_TERMINAL_OUTPUT_LENGTH
    ? nextOutput.slice(nextOutput.length - MAX_RENDERER_TERMINAL_OUTPUT_LENGTH)
    : nextOutput;
}

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

function SideTerminalTool({ cwd }: { cwd?: string | null }) {
  const { t } = useI18n();
  const [state, setState] = useState<DesktopTerminalState | null>(null);
  const [creating, setCreating] = useState(false);
  const [terminalReady, setTerminalReady] = useState(false);
  const terminalHostRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<XTermTerminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const renderedSessionIdRef = useRef<string | null>(null);
  const renderedOutputRef = useRef("");
  const activeSessionRef = useRef<ReturnType<typeof activeTerminalSession>>(null);
  const session = activeTerminalSession(state);

  useEffect(() => {
    activeSessionRef.current = session;
  }, [session]);

  useEffect(() => {
    const node = terminalHostRef.current;
    if (!node) {
      return;
    }
    const terminal = new XTermTerminal({
      allowProposedApi: false,
      convertEol: true,
      cursorBlink: true,
      fontFamily: '"SFMono-Regular", "Cascadia Code", Menlo, Monaco, Consolas, monospace',
      fontSize: 12,
      lineHeight: 1.24,
      scrollback: 3_000,
      theme: {
        background: "#ffffff",
        foreground: "#1a1c1f",
        cursor: "#1a1c1f",
        selectionBackground: "#cfe8ff",
        black: "#1a1c1f",
        blue: "#2563eb",
        brightBlack: "#7c7f85",
        brightBlue: "#1d4ed8",
        brightCyan: "#0e7490",
        brightGreen: "#15803d",
        brightMagenta: "#7c3aed",
        brightRed: "#b91c1c",
        brightWhite: "#334155",
        brightYellow: "#b45309",
        cyan: "#0891b2",
        green: "#16a34a",
        magenta: "#9333ea",
        red: "#dc2626",
        white: "#5f6368",
        yellow: "#ca8a04",
      },
    });
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(node);
    terminalRef.current = terminal;
    fitAddonRef.current = fitAddon;

    const resizeToHost = () => {
      try {
        fitAddon.fit();
        const active = activeSessionRef.current;
        if (active?.running) {
          void window.garyxDesktop.resizeTerminalSession({
            sessionId: active.id,
            cols: terminal.cols,
            rows: terminal.rows,
          });
        }
      } catch {
        // The terminal can briefly report a zero-sized host while tabs switch.
      }
    };

    const dataDisposable = terminal.onData((data) => {
      const active = activeSessionRef.current;
      if (!active?.running) {
        return;
      }
      void window.garyxDesktop.writeTerminalInput({
        sessionId: active.id,
        data,
      });
    });
    const resizeDisposable = terminal.onResize(({ cols, rows }) => {
      const active = activeSessionRef.current;
      if (active?.running) {
        void window.garyxDesktop.resizeTerminalSession({
          sessionId: active.id,
          cols,
          rows,
        });
      }
    });
    const observer = new ResizeObserver(resizeToHost);
    observer.observe(node);
    requestAnimationFrame(resizeToHost);
    setTerminalReady(true);

    return () => {
      setTerminalReady(false);
      observer.disconnect();
      dataDisposable.dispose();
      resizeDisposable.dispose();
      terminal.dispose();
      terminalRef.current = null;
      fitAddonRef.current = null;
      renderedSessionIdRef.current = null;
      renderedOutputRef.current = "";
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    const handleEvent = (event: DesktopTerminalEvent) => {
      if (disposed) {
        return;
      }
      if (event.type === "state") {
        setState(event.state);
        return;
      }
      if (activeSessionRef.current?.id === event.sessionId) {
        terminalRef.current?.write(event.data);
        renderedSessionIdRef.current = event.sessionId;
        renderedOutputRef.current = appendTerminalOutput(renderedOutputRef.current, event.data);
      }
      setState((current) => {
        if (!current) {
          return current;
        }
        return {
          ...current,
          sessions: current.sessions.map((entry) =>
            entry.id === event.sessionId
              ? {
                  ...entry,
                  output: appendTerminalOutput(entry.output, event.data),
                  updatedAt: new Date().toISOString(),
                }
              : entry,
          ),
        };
      });
    };
    void window.garyxDesktop.listTerminalState().then((nextState) => {
      if (!disposed) {
        setState(nextState);
      }
    });
    window.garyxDesktop.subscribeTerminalEvents(handleEvent);
    return () => {
      disposed = true;
      window.garyxDesktop.unsubscribeTerminalEvents(handleEvent);
    };
  }, []);

  async function createSession() {
    if (creating) {
      return;
    }
    setCreating(true);
    const terminal = terminalRef.current;
    await window.garyxDesktop
      .createTerminalSession({
        cwd,
        cols: terminal?.cols,
        rows: terminal?.rows,
      })
      .then(setState)
      .finally(() => setCreating(false));
  }

  useEffect(() => {
    if (state === null || state.sessions.length > 0 || creating || !terminalReady) {
      return;
    }
    void createSession();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [creating, state, terminalReady]);

  useEffect(() => {
    const terminal = terminalRef.current;
    if (!terminal || !terminalReady) {
      return;
    }
    if (!session) {
      terminal.clear();
      renderedSessionIdRef.current = null;
      renderedOutputRef.current = "";
      return;
    }
    if (renderedSessionIdRef.current !== session.id) {
      terminal.reset();
      terminal.write(session.output);
      renderedSessionIdRef.current = session.id;
      renderedOutputRef.current = session.output;
      return;
    }
    const rendered = renderedOutputRef.current;
    if (session.output.startsWith(rendered)) {
      const delta = session.output.slice(rendered.length);
      if (delta) {
        terminal.write(delta);
      }
    } else {
      terminal.reset();
      terminal.write(session.output);
    }
    renderedOutputRef.current = session.output;
  }, [session?.id, session?.output, terminalReady]);

  // The side tools panel already manages tabs; the terminal body stays a
  // single session with no inner session chrome. Closing the exited session
  // lets the auto-create effect start a fresh one.
  function restartExitedSession() {
    const exited = activeSessionRef.current;
    if (!exited || exited.running) {
      return;
    }
    void window.garyxDesktop
      .closeTerminalSession({ sessionId: exited.id })
      .then(setState);
  }

  return (
    <div className="side-tool-terminal">
      <div
        aria-label={t("Terminal input")}
        className="side-tool-terminal-output"
        ref={terminalHostRef}
      />
      {creating ? <div className="side-tool-terminal-status">{t("Starting…")}</div> : null}
      {session && !session.running && !creating ? (
        <button
          className="side-tool-terminal-status side-tool-terminal-restart"
          onClick={restartExitedSession}
          type="button"
        >
          {t("Terminal exited · Restart")}
        </button>
      ) : null}
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
  const [activeToolId, setActiveToolId] = useState<ThreadSideToolId | null>(
    null,
  );
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
  const activeTool = activeToolId
    ? tools.find((tool) => tool.id === activeToolId) || null
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
    if (activeToolId === "browser") {
      return;
    }

    void window.garyxDesktop.updateBrowserBounds({
      x: 0,
      y: 0,
      width: 0,
      height: 0,
      visible: false,
    });
  }, [activeToolId]);

  useEffect(() => {
    if (!shouldShowWorkspacePreview) {
      return;
    }
    setOpenTools((current) =>
      current.includes("files") ? current : ["files", ...current],
    );
    setActiveToolId("files");
  }, [shouldShowWorkspacePreview, workspaceFilePreview?.path, workspacePreviewTitle]);

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
    if (!menuOpen || activeToolId !== "browser") {
      return;
    }

    const handleBrowserMouseDown = () => {
      setMenuOpen(false);
    };
    window.garyxDesktop.subscribeBrowserPageMouseDown(handleBrowserMouseDown);
    return () => {
      window.garyxDesktop.unsubscribeBrowserPageMouseDown(handleBrowserMouseDown);
    };
  }, [activeToolId, menuOpen]);

  useLayoutEffect(() => {
    if (!menuOpen || activeToolId !== "browser") {
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
  }, [activeToolId, menuOpen]);

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
    setActiveToolId(toolId);
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

  function closeTool(toolId: ThreadSideToolId) {
    setOpenTools((current) => {
      const next = current.filter((id) => id !== toolId);
      if (!next.length) {
        setActiveToolId(null);
        return next;
      }
      if (activeToolId === toolId) {
        setActiveToolId(next[next.length - 1]);
      }
      return next;
    });
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
      className={`thread-side-tools-panel is-${activeToolId ?? "picker"}-active`}
    >
      <div className="side-tools-tabs">
        <div className="side-tools-tab-cluster">
          <div className="side-tools-tab-track" role="tablist">
            {openToolDescriptors.map((tool) => {
              const Icon = tool.icon;
              const selected = tool.id === activeToolId;
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
                      setActiveToolId(tool.id);
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
                      closeTool(tool.id);
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

      <div className={`side-tools-body is-${activeTool?.id ?? "picker"}`}>
        {!activeTool ? (
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
          <SideTerminalTool cwd={activeWorkspacePath} />
        ) : null}
      </div>
    </aside>
  );
}
