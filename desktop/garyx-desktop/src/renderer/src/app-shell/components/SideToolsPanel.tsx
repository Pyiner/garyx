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
  FileText,
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
  DesktopWorkspaceMode,
  MessageFileAttachment,
} from "@shared/contracts";

import { useI18n } from "../../i18n";

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
  workspaceMode?: DesktopWorkspaceMode | null;
  sideChatPanel: ReactNode;
  onRevealSelectedWorkspaceFile?: () => Promise<void> | void;
  onAddBrowserAnnotationComment: (request: BrowserAnnotationCommentRequest) => void;
  onAttachFileToSideChat: (file: MessageFileAttachment) => void;
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
      lineHeight: 1.22,
      scrollback: 3_000,
      theme: {
        background: "#111214",
        foreground: "#e7e7e7",
        cursor: "#f5f5f5",
        selectionBackground: "#4b5563",
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
        {state?.sessions.length ? (
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
        ) : (
          <span>{t("Terminal")}</span>
        )}
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
  onRevealSelectedWorkspaceFile,
  onAddBrowserAnnotationComment,
  onAttachFileToSideChat,
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
  const activeTool = tools.find((tool) => tool.id === activeToolId) || tools[0];
  const openToolDescriptors = openTools
    .map((toolId) => tools.find((tool) => tool.id === toolId))
    .filter((tool): tool is ToolDescriptor => Boolean(tool));

  function attachSelectedWorkspaceFile() {
    if (!selectedWorkspaceFile) {
      return;
    }
    const file: MessageFileAttachment = {
      id: `side-chat-file-${selectedWorkspaceFile.absolutePath}`,
      name: selectedWorkspaceFile.name,
      mediaType: selectedWorkspaceFile.mediaType || "",
      path: selectedWorkspaceFile.absolutePath,
    };
    onAttachFileToSideChat(file);
    openTool("chat");
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

  return (
    <aside className={`thread-side-tools-panel is-${activeToolId}-active`}>
      <div className="side-tools-tabs">
        <div className="side-tools-tab-track" role="tablist">
          {openToolDescriptors.map((tool) => {
            const Icon = tool.icon;
            return (
              <button
                aria-selected={tool.id === activeToolId}
                className={`side-tools-tab ${tool.id === activeToolId ? "is-active" : ""}`}
                key={tool.id}
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
                <span>{tool.label}</span>
                {openTools.length > 1 ? (
                  <span
                    className="side-tools-tab-close"
                    onClick={(event) => {
                      event.stopPropagation();
                      closeTool(tool.id);
                    }}
                    role="button"
                    tabIndex={-1}
                  >
                    <X aria-hidden size={12} strokeWidth={1.9} />
                  </span>
                ) : null}
              </button>
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

      <div className={`side-tools-body is-${activeTool.id}`}>
        {activeTool.id === "files" ? (
          <div className="side-tool-files">
            <div className="side-tool-section-heading">
              <span>{t("Open file")}</span>
            </div>
            <p className="side-tool-section-subtitle">
              {t("Choose from the workspace directory tree")}
            </p>
            <div className="side-tool-filter-shell">
              <input
                aria-label={t("Filter files")}
                onChange={(event) => onWorkspaceFileFilterChange(event.target.value)}
                placeholder={t("Filter files…")}
                type="search"
                value={workspaceFileFilter}
              />
            </div>
            {selectedWorkspaceFile ? (
              <div className="side-tool-selected-file">
                <div>
                  <FileText aria-hidden size={14} strokeWidth={1.8} />
                  <span title={selectedWorkspaceFile.relativePath}>
                    {selectedWorkspaceFile.relativePath}
                  </span>
                </div>
                <div className="side-tool-selected-file-actions">
                  <button onClick={attachSelectedWorkspaceFile} type="button">
                    {t("Attach to chat")}
                  </button>
                  {onRevealSelectedWorkspaceFile ? (
                    <button onClick={() => void onRevealSelectedWorkspaceFile()} type="button">
                      {t("Reveal")}
                    </button>
                  ) : null}
                </div>
              </div>
            ) : null}
            <div className="side-tool-file-tree">{workspaceDirectoryPanel}</div>
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
