import {
  lazy,
  Suspense,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  ChevronDown,
  Copy,
  FileText,
  FolderOpen,
  Globe,
  MessageSquare,
  Plus,
  Terminal as TerminalIcon,
  X,
} from "lucide-react";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal as XTermTerminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";

import type {
  BrowserAnnotationCommentRequest,
  DesktopTerminalEvent,
  DesktopTerminalState,
  DesktopWorkspaceFilePreview,
  DesktopWorkspaceMode,
} from "@shared/contracts";

import { WorkspaceFilePreview } from "../../workspace-file-preview";
import { PanelIcon } from "../icons";
import { useI18n } from "../../i18n";
import { workspaceFileAbsolutePath } from "../workspace-helpers";

const SidePanelBrowserPage = lazy(() =>
  import("../../BrowserPage").then((module) => ({ default: module.BrowserPage }))
);

export type ThreadSideToolId = "files" | "chat" | "browser" | "terminal";

export type SideToolWorkspaceFile = {
  name: string;
  relativePath: string;
  absolutePath: string;
  mediaType?: string | null;
};

type ThreadSideToolsPanelProps = {
  activeWorkspaceName?: string | null;
  activeWorkspacePath?: string | null;
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

function appendTerminalOutput(output: string, data: string): string {
  const nextOutput = `${output}${data}`;
  return nextOutput.length > MAX_RENDERER_TERMINAL_OUTPUT_LENGTH
    ? nextOutput.slice(nextOutput.length - MAX_RENDERER_TERMINAL_OUTPUT_LENGTH)
    : nextOutput;
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

  return (
    <div className="side-tool-terminal">
      <div className="side-tool-terminal-header">
        <div className="side-tool-terminal-session">
          <TerminalIcon aria-hidden size={14} strokeWidth={1.8} />
          {state?.sessions.length ? (
            <>
              <select
                aria-label={t("Terminal session")}
                onChange={(event) => {
                  void window.garyxDesktop
                    .activateTerminalSession({ sessionId: event.target.value })
                    .then(setState);
                }}
                value={session?.id || ""}
              >
                {state.sessions.map((entry) => (
                  <option key={entry.id} value={entry.id}>
                    {entry.title}
                  </option>
                ))}
              </select>
              <ChevronDown aria-hidden className="side-tool-terminal-session-chevron" size={13} />
            </>
          ) : (
            <span>{t("Terminal")}</span>
          )}
        </div>
        <div className="side-tool-terminal-header-actions">
          {session ? (
            <button
              className="codex-icon-button"
              onClick={() => {
                void window.garyxDesktop
                  .closeTerminalSession({ sessionId: session.id })
                  .then(setState);
              }}
              title={t("Close terminal")}
              type="button"
            >
              <X aria-hidden />
            </button>
          ) : null}
          <button
            className="codex-icon-button"
            onClick={() => {
              void createSession();
            }}
            title={t("New terminal")}
            type="button"
          >
            <Plus aria-hidden />
          </button>
        </div>
      </div>
      <div
        aria-label={t("Terminal input")}
        className="side-tool-terminal-output"
        ref={terminalHostRef}
      />
      {creating ? <div className="side-tool-terminal-status">{t("Starting…")}</div> : null}
      {session && !session.running ? (
        <div className="side-tool-terminal-status">{t("Terminal exited")}</div>
      ) : null}
    </div>
  );
}

export function ThreadSideToolsPanel({
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
  onOpenSideChat,
  onWorkspaceFileFilterChange,
}: ThreadSideToolsPanelProps) {
  const { t } = useI18n();
  const tools = useMemo<ToolDescriptor[]>(
    () => [
      { id: "files", label: t("Files"), shortcut: "⌘P", icon: FileText },
      { id: "chat", label: t("Side Chat"), shortcut: "", icon: MessageSquare },
      { id: "browser", label: t("Browser"), shortcut: "⌘T", icon: Globe },
      { id: "terminal", label: t("Terminal"), shortcut: "⌃`", icon: TerminalIcon },
    ],
    [t],
  );
  const [openTools, setOpenTools] = useState<ThreadSideToolId[]>(["files"]);
  const [activeToolId, setActiveToolId] = useState<ThreadSideToolId>("files");
  const [menuOpen, setMenuOpen] = useState(false);
  const [filePathCopied, setFilePathCopied] = useState(false);
  const filePathCopiedTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const activeTool = tools.find((tool) => tool.id === activeToolId) || tools[0];
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

  function openTool(toolId: ThreadSideToolId) {
    setOpenTools((current) =>
      current.includes(toolId) ? current : [...current, toolId],
    );
    setActiveToolId(toolId);
    setMenuOpen(false);
    if (toolId === "chat") {
      onOpenSideChat();
    }
  }

  function closeTool(toolId: ThreadSideToolId) {
    setOpenTools((current) => {
      const next = current.filter((id) => id !== toolId);
      if (!next.length) {
        setActiveToolId("files");
        return ["files"];
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
    <aside className={`thread-side-tools-panel is-${activeToolId}-active`}>
      <div className="side-tools-tabs">
        <div className="side-tools-tab-cluster">
          <div className="side-tools-tab-track" role="tablist">
            {openToolDescriptors.map((tool) => {
              const Icon = tool.icon;
              const selected = tool.id === activeToolId;
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
                      if (tool.id === "chat") {
                        onOpenSideChat();
                      }
                    }}
                    role="tab"
                    type="button"
                  >
                    <Icon aria-hidden size={14} strokeWidth={1.8} />
                    <span className="side-tools-tab-label">{tool.label}</span>
                  </button>
                  {openTools.length > 1 ? (
                    <button
                      aria-label={t("Close")}
                      className="side-tools-tab-close"
                      onClick={(event) => {
                        event.stopPropagation();
                        closeTool(tool.id);
                      }}
                      title={t("Close")}
                      type="button"
                    >
                      <X aria-hidden size={12} strokeWidth={1.9} />
                    </button>
                  ) : null}
                </div>
              );
            })}
          </div>
          <div className="side-tools-add-shell">
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
              <div className="side-tools-menu" role="menu">
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

      <div className={`side-tools-body is-${activeTool.id}`}>
        {activeTool.id === "files" ? (
          <div className="side-tool-files">
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
                  onChange={(event) => onWorkspaceFileFilterChange(event.target.value)}
                  placeholder={t("Filter files…")}
                  type="search"
                  value={workspaceFileFilter}
                />
              </div>
              <div className="side-tool-file-tree">{workspaceDirectoryPanel}</div>
            </aside>
          </div>
        ) : null}
        {activeTool.id === "chat" ? (
          <div className="side-tool-chat-thread">{sideChatPanel}</div>
        ) : null}
        {activeTool.id === "browser" ? (
          <Suspense
            fallback={<div className="browser-page browser-page-side-panel browser-side-panel-loading" />}
          >
            <SidePanelBrowserPage
              onAnnotationCommentRequest={handleBrowserAnnotationCommentRequest}
              variant="side-panel"
            />
          </Suspense>
        ) : null}
        {activeTool.id === "terminal" ? (
          <SideTerminalTool cwd={activeWorkspacePath} />
        ) : null}
      </div>
    </aside>
  );
}
