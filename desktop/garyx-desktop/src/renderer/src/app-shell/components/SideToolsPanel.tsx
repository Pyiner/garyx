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
  GitBranch,
  Globe,
  MessageSquare,
  Mic,
  MoreHorizontal,
  Paperclip,
  Plus,
  RefreshCw,
  Send,
  Terminal,
  X,
  Zap,
} from "lucide-react";

import type {
  DesktopWorkspaceGitDetails,
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
          <button className="side-tool-chat-model" type="button">
            <Zap aria-hidden size={13} strokeWidth={1.8} />
            <span>5.5</span>
          </button>
          <button className="codex-icon-button" title={t("Dictate")} type="button">
            <Mic aria-hidden />
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
  const [input, setInput] = useState("");
  const [creating, setCreating] = useState(false);
  const outputRef = useRef<HTMLPreElement | null>(null);
  const session = activeTerminalSession(state);

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
                  output: `${entry.output}${event.data}`,
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

  useEffect(() => {
    if (state === null || state.sessions.length > 0 || creating) {
      return;
    }
    setCreating(true);
    void window.garyxDesktop
      .createTerminalSession({ cwd })
      .then(setState)
      .finally(() => setCreating(false));
  }, [creating, cwd, state]);

  useEffect(() => {
    const node = outputRef.current;
    if (node) {
      node.scrollTop = node.scrollHeight;
    }
  }, [session?.output]);

  async function sendInput() {
    const value = input;
    if (!session || !session.running || !value.trim()) {
      return;
    }
    setInput("");
    await window.garyxDesktop.writeTerminalInput({
      sessionId: session.id,
      data: `${value}\n`,
    });
  }

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
              void window.garyxDesktop.createTerminalSession({ cwd }).then(setState);
            }}
            title={t("New terminal")}
            type="button"
          >
            <Plus aria-hidden />
          </button>
        </div>
      </div>
      <pre className="side-tool-terminal-output" ref={outputRef}>
        {session?.output || (creating ? t("Starting…") : "")}
      </pre>
      <textarea
        aria-label={t("Terminal input")}
        className="side-tool-terminal-input"
        disabled={!session?.running}
        onChange={(event) => setInput(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && !event.shiftKey) {
            event.preventDefault();
            void sendInput();
          }
        }}
        placeholder={session?.running ? t("Terminal input") : t("Terminal exited")}
        rows={2}
        value={input}
      />
    </div>
  );
}

export function ThreadSideToolsPanel({
  activeWorkspaceName,
  activeWorkspacePath,
  selectedWorkspaceFile,
  threadId,
  workspaceBranch,
  workspaceDirectoryPanel,
  workspaceFileFilter,
  workspaceMode,
  onRevealSelectedWorkspaceFile,
  onSubmitSideChat,
  onWorkspaceFileFilterChange,
}: ThreadSideToolsPanelProps) {
  const { t } = useI18n();
  const tools = useMemo<ToolDescriptor[]>(
    () => [
      { id: "files", label: t("Files"), shortcut: "⌘P", icon: FileText },
      { id: "chat", label: t("Side Chat"), shortcut: "", icon: MessageSquare },
      { id: "browser", label: t("Browser"), shortcut: "⌘T", icon: Globe },
      { id: "terminal", label: t("Terminal"), shortcut: "⌃`", icon: Terminal },
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
  const [gitDetails, setGitDetails] = useState<DesktopWorkspaceGitDetails | null>(null);
  const [gitLoading, setGitLoading] = useState(false);
  const [gitBusy, setGitBusy] = useState(false);
  const [gitPanelOpen, setGitPanelOpen] = useState(false);
  const [gitMessage, setGitMessage] = useState("");
  const [gitError, setGitError] = useState<string | null>(null);
  const [gitOutput, setGitOutput] = useState<string | null>(null);
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

  async function refreshGitDetails() {
    if (!activeWorkspacePath) {
      setGitDetails(null);
      return;
    }
    setGitLoading(true);
    setGitError(null);
    try {
      const nextDetails = await window.garyxDesktop.getWorkspaceGitDetails({
        workspacePath: activeWorkspacePath,
      });
      setGitDetails(nextDetails);
    } catch (error) {
      setGitError(error instanceof Error ? error.message : t("Failed to load changes."));
      setGitDetails(null);
    } finally {
      setGitLoading(false);
    }
  }

  useEffect(() => {
    setGitPanelOpen(false);
    setGitMessage("");
    setGitOutput(null);
    void refreshGitDetails();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeWorkspacePath]);

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

  const changeLabel = gitLoading
    ? t("Loading…")
    : gitDetails?.isGitRepo
      ? gitDetails.changedCount > 0
        ? t("{count} changed", { count: String(gitDetails.changedCount) })
        : gitDetails.ahead > 0
          ? t("{count} ahead", { count: String(gitDetails.ahead) })
          : t("Clean")
      : t("No Git repo");

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

  async function commitChanges() {
    const message = gitMessage.trim();
    if (!activeWorkspacePath || !message || gitBusy) {
      return;
    }
    setGitBusy(true);
    setGitError(null);
    setGitOutput(null);
    try {
      const result = await window.garyxDesktop.commitWorkspaceChanges({
        workspacePath: activeWorkspacePath,
        message,
      });
      setGitDetails(result.status);
      setGitMessage("");
      setGitOutput(result.output || t("Committed changes."));
    } catch (error) {
      setGitError(error instanceof Error ? error.message : t("Commit failed."));
    } finally {
      setGitBusy(false);
    }
  }

  async function pushChanges() {
    if (!activeWorkspacePath || gitBusy) {
      return;
    }
    setGitBusy(true);
    setGitError(null);
    setGitOutput(null);
    try {
      const result = await window.garyxDesktop.pushWorkspaceBranch({
        workspacePath: activeWorkspacePath,
      });
      setGitDetails(result.status);
      setGitOutput(result.output || t("Pushed branch."));
    } catch (error) {
      setGitError(error instanceof Error ? error.message : t("Push failed."));
    } finally {
      setGitBusy(false);
    }
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
    <aside className="thread-side-tools-panel">
      <div className="side-tools-context">
        <div className="side-tools-context-title">{t("Environment Info")}</div>
        <div className="side-tools-context-row">
          <span>{t("Changes")}</span>
          <span>{changeLabel}</span>
        </div>
        {gitDetails?.isGitRepo ? (
          <div className="side-tools-context-row">
            <span>{t("Branch")}</span>
            <span>
              {gitDetails.currentBranch || workspaceBranch?.trim() || t("Detached")}
              {gitDetails.ahead || gitDetails.behind
                ? ` +${gitDetails.ahead} / -${gitDetails.behind}`
                : ""}
            </span>
          </div>
        ) : null}
        <div className="side-tools-context-row">
          <span>{t("Workspace")}</span>
          <span>{activeWorkspaceName || activeWorkspacePath || t("Local")}</span>
        </div>
        <div className="side-tools-context-row">
          <span>{t("Mode")}</span>
          <span>{workspaceMode === "local" ? t("Local") : workspaceMode || t("Local")}</span>
        </div>
        <div className="side-tools-context-actions">
          <button
            className="side-tools-context-action"
            disabled={!gitDetails?.isGitRepo}
            onClick={() => setGitPanelOpen((current) => !current)}
            type="button"
          >
            {gitDetails?.ahead && !gitDetails.isDirty ? t("Push changes") : t("Commit or push")}
          </button>
          <button
            className="codex-icon-button side-tools-refresh"
            disabled={!activeWorkspacePath || gitLoading}
            onClick={() => void refreshGitDetails()}
            title={t("Refresh changes")}
            type="button"
          >
            <RefreshCw aria-hidden />
          </button>
        </div>
        {gitPanelOpen && gitDetails?.isGitRepo ? (
          <div className="side-tools-git-panel">
            <div className="side-tools-git-summary">
              <GitBranch aria-hidden size={14} strokeWidth={1.8} />
              <span>
                {gitDetails.changedCount
                  ? t("{count} changed files", { count: String(gitDetails.changedCount) })
                  : t("Working tree clean")}
              </span>
            </div>
            {gitDetails.files.length ? (
              <div className="side-tools-git-files">
                {gitDetails.files.slice(0, 6).map((file) => (
                  <span key={`${file.status}:${file.path}`}>
                    <code>{file.status}</code>
                    <span>{file.path}</span>
                  </span>
                ))}
                {gitDetails.files.length > 6 ? (
                  <span>{t("{count} more", { count: String(gitDetails.files.length - 6) })}</span>
                ) : null}
              </div>
            ) : null}
            {gitDetails.isDirty ? (
              <div className="side-tools-git-commit">
                <input
                  onChange={(event) => setGitMessage(event.target.value)}
                  placeholder={t("Commit message")}
                  value={gitMessage}
                />
                <button disabled={!gitMessage.trim() || gitBusy} onClick={() => void commitChanges()} type="button">
                  {gitBusy ? t("Working…") : t("Commit")}
                </button>
              </div>
            ) : null}
            <button
              className="side-tools-git-push"
              disabled={gitBusy || gitDetails.isDirty}
              onClick={() => void pushChanges()}
              type="button"
            >
              {gitBusy ? t("Working…") : t("Push")}
            </button>
            {gitError ? <div className="side-tool-error">{gitError}</div> : null}
            {gitOutput ? <pre className="side-tools-git-output">{gitOutput}</pre> : null}
          </div>
        ) : null}
        {gitError && !gitPanelOpen ? <div className="side-tool-error">{gitError}</div> : null}
        <div className="side-tools-context-title side-tools-context-title-secondary">
          {t("Sources")}
        </div>
        {attachedFiles.length ? (
          <div className="side-tools-sources">
            {attachedFiles.map((file) => (
              <span key={file.id} title={file.path}>{file.name}</span>
            ))}
          </div>
        ) : (
          <div className="side-tools-context-empty">{t("No sources")}</div>
        )}
      </div>

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
              <button className="codex-icon-button" title={t("More")} type="button">
                <MoreHorizontal aria-hidden />
              </button>
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
            <SidePanelBrowserPage variant="side-panel" />
          </Suspense>
        ) : null}
        {activeTool.id === "terminal" ? (
          <SideTerminalTool cwd={activeWorkspacePath} />
        ) : null}
      </div>
    </aside>
  );
}
