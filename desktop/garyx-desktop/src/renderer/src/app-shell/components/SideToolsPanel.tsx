import {
  lazy,
  Suspense,
  useEffect,
  useMemo,
  useRef,
  useState,
  type Dispatch,
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
  type SetStateAction,
} from "react";
import {
  FileText,
  Globe,
  MessageSquare,
  Paperclip,
  Plus,
  Send,
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
  OpenChatStreamResult,
} from "@shared/contracts";

import { useI18n } from "../../i18n";

const SidePanelBrowserPage = lazy(() =>
  import("../../BrowserPage").then((module) => ({ default: module.BrowserPage }))
);

export type ThreadSideToolId = "files" | "chat" | "browser" | "terminal";

export type SideChatSubmitResult = Pick<
  OpenChatStreamResult,
  "response" | "status" | "threadId"
> & {
  title?: string | null;
};

export type SideChatSubmitInput = {
  message: string;
  threadId?: string | null;
  files?: MessageFileAttachment[];
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
  selectedWorkspaceFile?: SideToolWorkspaceFile | null;
  threadId?: string | null;
  workspaceBranch?: string | null;
  workspaceDirectoryPanel: ReactNode;
  workspaceFileFilter: string;
  workspaceMode?: DesktopWorkspaceMode | null;
  onRevealSelectedWorkspaceFile?: () => Promise<void> | void;
  onAddBrowserAnnotationComment: (request: BrowserAnnotationCommentRequest) => void;
  onSubmitSideChat: (input: SideChatSubmitInput) => Promise<SideChatSubmitResult>;
  onWorkspaceFileFilterChange: (value: string) => void;
};

type SideChatMessage = {
  id: string;
  role: "user" | "assistant";
  text: string;
  threadId?: string | null;
};

type PersistedSideChatState = {
  draft: string;
  messages: SideChatMessage[];
  sideThreadId: string | null;
};

function emptySideChatState(): PersistedSideChatState {
  return {
    draft: "",
    messages: [],
    sideThreadId: null,
  };
}

function readSideChatState(key: string): PersistedSideChatState {
  if (typeof window === "undefined") {
    return emptySideChatState();
  }
  try {
    const raw = window.sessionStorage.getItem(key);
    if (!raw) {
      return emptySideChatState();
    }
    const parsed = JSON.parse(raw) as Partial<PersistedSideChatState>;
    return {
      draft: typeof parsed.draft === "string" ? parsed.draft : "",
      messages: Array.isArray(parsed.messages)
        ? parsed.messages.filter((message): message is SideChatMessage => (
            typeof message?.id === "string" &&
            (message.role === "user" || message.role === "assistant") &&
            typeof message.text === "string"
          ))
        : [],
      sideThreadId: typeof parsed.sideThreadId === "string" ? parsed.sideThreadId : null,
    };
  } catch {
    return emptySideChatState();
  }
}

function writeSideChatState(key: string, state: PersistedSideChatState) {
  if (typeof window === "undefined") {
    return;
  }
  try {
    window.sessionStorage.setItem(key, JSON.stringify(state));
  } catch {
    // Ignore storage quota or privacy-mode failures; in-memory state still works.
  }
}

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

function SideChatTool({
  attachments,
  draft,
  error,
  messages,
  onClearAttachments,
  onDraftChange,
  onErrorChange,
  onMessagesChange,
  onOpenFiles,
  onRemoveAttachment,
  onSendingChange,
  onSideThreadIdChange,
  onSubmitSideChat,
  sending,
  sideThreadId,
}: {
  attachments: MessageFileAttachment[];
  draft: string;
  error: string | null;
  messages: SideChatMessage[];
  onClearAttachments: () => void;
  onDraftChange: Dispatch<SetStateAction<string>>;
  onErrorChange: Dispatch<SetStateAction<string | null>>;
  onMessagesChange: Dispatch<SetStateAction<SideChatMessage[]>>;
  onOpenFiles: () => void;
  onRemoveAttachment: (id: string) => void;
  onSendingChange: Dispatch<SetStateAction<boolean>>;
  onSideThreadIdChange: Dispatch<SetStateAction<string | null>>;
  onSubmitSideChat: ThreadSideToolsPanelProps["onSubmitSideChat"];
  sending: boolean;
  sideThreadId: string | null;
}) {
  const { t } = useI18n();
  const transcriptRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const node = transcriptRef.current;
    if (node) {
      node.scrollTop = node.scrollHeight;
    }
  }, [messages, sending]);

  async function submit(event?: FormEvent<HTMLFormElement>) {
    event?.preventDefault();
    const text = draft.trim();
    if (!text || sending) {
      return;
    }
    const userMessage: SideChatMessage = {
      id: `side-chat-user-${Date.now()}`,
      role: "user",
      text,
    };
    onMessagesChange((current) => [...current, userMessage]);
    onDraftChange("");
    onSendingChange(true);
    onErrorChange(null);
    try {
      const result = await onSubmitSideChat({
        message: text,
        threadId: sideThreadId,
        files: attachments,
      });
      onSideThreadIdChange(result.threadId);
      onClearAttachments();
      onMessagesChange((current) => [
        ...current,
        {
          id: `side-chat-assistant-${Date.now()}`,
          role: "assistant",
          text:
            result.response.trim() ||
            (result.status === "disconnected"
              ? t("Stream disconnected before a final response.")
              : t("Done.")),
          threadId: result.threadId,
        },
      ]);
    } catch (chatError) {
      onErrorChange(
        chatError instanceof Error
          ? chatError.message
          : t("Failed to start side chat."),
      );
    } finally {
      onSendingChange(false);
    }
  }

  return (
    <div className="side-tool-chat">
      <div className="side-tool-chat-transcript" ref={transcriptRef}>
        {messages.length ? (
          messages.map((message) => (
            <article
              className={`side-tool-chat-message is-${message.role}`}
              key={message.id}
            >
              <p>{message.text}</p>
            </article>
          ))
        ) : (
          <div className="side-tool-empty">
            {t("Start a focused side thread.")}
          </div>
        )}
        {sending ? (
          <article className="side-tool-chat-message is-assistant is-pending">
            <p>{t("Working…")}</p>
          </article>
        ) : null}
      </div>
      {error ? <div className="side-tool-error">{error}</div> : null}
      <form className="side-tool-chat-composer" onSubmit={(event) => void submit(event)}>
        {attachments.length ? (
          <div className="side-tool-chat-attachments">
            {attachments.map((file) => (
              <span className="side-tool-chat-attachment" key={file.id} title={file.path}>
                <FileText aria-hidden size={13} strokeWidth={1.8} />
                <span>{file.name}</span>
                <button
                  aria-label={t("Remove attachment")}
                  onClick={() => onRemoveAttachment(file.id)}
                  type="button"
                >
                  <X aria-hidden size={11} strokeWidth={2} />
                </button>
              </span>
            ))}
          </div>
        ) : null}
        <textarea
          aria-label={t("Side chat message")}
          onChange={(event) => onDraftChange(event.target.value)}
          onKeyDown={(event: KeyboardEvent<HTMLTextAreaElement>) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              void submit();
            }
          }}
          placeholder={t("Ask in side chat")}
          rows={3}
          value={draft}
        />
        <div className="side-tool-chat-actions">
          <button
            className="codex-icon-button"
            onClick={onOpenFiles}
            title={t("Add file")}
            type="button"
          >
            <Paperclip aria-hidden />
          </button>
          <button
            className="codex-icon-button side-tool-chat-send"
            disabled={!draft.trim() || sending}
            title={t("Send")}
            type="submit"
          >
            <Send aria-hidden />
          </button>
        </div>
      </form>
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
  threadId,
  workspaceDirectoryPanel,
  workspaceFileFilter,
  onRevealSelectedWorkspaceFile,
  onAddBrowserAnnotationComment,
  onSubmitSideChat,
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
  const [attachedFiles, setAttachedFiles] = useState<MessageFileAttachment[]>([]);
  const [sideChatDraft, setSideChatDraft] = useState("");
  const [sideChatMessages, setSideChatMessages] = useState<SideChatMessage[]>([]);
  const [sideChatThreadId, setSideChatThreadId] = useState<string | null>(null);
  const [sideChatSending, setSideChatSending] = useState(false);
  const [sideChatError, setSideChatError] = useState<string | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const sideChatStorageKey = useMemo(() => {
    const ownerKey = threadId?.trim() || activeWorkspacePath?.trim() || "global";
    return `garyx.side-tools.side-chat.${ownerKey}`;
  }, [activeWorkspacePath, threadId]);
  const sideChatHydratedKeyRef = useRef<string | null>(null);
  const activeTool = tools.find((tool) => tool.id === activeToolId) || tools[0];
  const openToolDescriptors = openTools
    .map((toolId) => tools.find((tool) => tool.id === toolId))
    .filter((tool): tool is ToolDescriptor => Boolean(tool));

  useEffect(() => {
    const persisted = readSideChatState(sideChatStorageKey);
    setSideChatDraft(persisted.draft);
    setSideChatMessages(persisted.messages);
    setSideChatThreadId(persisted.sideThreadId);
    setSideChatError(null);
    setSideChatSending(false);
    sideChatHydratedKeyRef.current = sideChatStorageKey;
  }, [sideChatStorageKey]);

  useEffect(() => {
    if (sideChatHydratedKeyRef.current !== sideChatStorageKey) {
      return;
    }
    writeSideChatState(sideChatStorageKey, {
      draft: sideChatDraft,
      messages: sideChatMessages,
      sideThreadId: sideChatThreadId,
    });
  }, [sideChatDraft, sideChatMessages, sideChatStorageKey, sideChatThreadId]);

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
    setAttachedFiles((current) =>
      current.some((entry) => entry.path === file.path) ? current : [...current, file],
    );
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
                onClick={() => setActiveToolId(tool.id)}
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
          <SideChatTool
            attachments={attachedFiles}
            draft={sideChatDraft}
            error={sideChatError}
            messages={sideChatMessages}
            onClearAttachments={() => setAttachedFiles([])}
            onDraftChange={setSideChatDraft}
            onErrorChange={setSideChatError}
            onMessagesChange={setSideChatMessages}
            onOpenFiles={() => openTool("files")}
            onRemoveAttachment={(id) =>
              setAttachedFiles((current) => current.filter((file) => file.id !== id))
            }
            onSendingChange={setSideChatSending}
            onSideThreadIdChange={setSideChatThreadId}
            onSubmitSideChat={onSubmitSideChat}
            sending={sideChatSending}
            sideThreadId={sideChatThreadId}
          />
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
