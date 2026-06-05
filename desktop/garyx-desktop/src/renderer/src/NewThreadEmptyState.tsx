import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  IconDeviceLaptop,
  IconFolder,
  IconGitBranch,
  IconHistory,
  IconPlus,
  IconRefresh,
  IconSearch,
  IconSparkles,
} from "@tabler/icons-react";

import type {
  DesktopDreamTopic,
  DesktopDreamsPage,
  DesktopProviderRecentSession,
  DesktopSessionProviderHint,
  DesktopWorkspace,
  DesktopWorkspaceGitStatus,
  DesktopWorkspaceMode,
} from "@shared/contracts";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useI18n } from "./i18n";

const ADD_WORKSPACE_VALUE = "__add_workspace__";
const MISSING_WORKSPACE_VALUE_PREFIX = "__missing_workspace__:";
const GIT_STATUS_CHECK_DELAY_MS = 120;
const HOME_DREAMS_LIMIT = 4;
const HOME_DREAMS_CACHE_STALE_MS = 45_000;
const RESUME_PROVIDER_OPTIONS: Array<{
  value: DesktopSessionProviderHint;
  label: string;
}> = [
  { value: "codex", label: "Codex" },
  { value: "claude", label: "Claude Code" },
  { value: "gemini", label: "Gemini CLI" },
];
const workspaceGitStatusCache = new Map<string, DesktopWorkspaceGitStatus>();
let homeDreamsCache: { page: DesktopDreamsPage; cachedAt: number } | null =
  null;
let pendingHomeDreamsLoad: Promise<DesktopDreamsPage> | null = null;

type NewThreadEmptyStateProps = {
  newThreadWorkspaceEntry: DesktopWorkspace | null;
  selectableNewThreadWorkspaces: DesktopWorkspace[];
  workspaceMutation: string | null;
  workspaceMode: DesktopWorkspaceMode;
  onAddWorkspace: () => void;
  onSelectWorkspace: (workspacePath: string) => void;
  onWorkspaceModeChange: (workspaceMode: DesktopWorkspaceMode) => void;
  onOpenDreamThread: (threadId: string) => void;
  onResumeProviderSession: (
    sessionId: string,
    providerHint?: DesktopSessionProviderHint | null,
  ) => Promise<void>;
  showDreams: boolean;
};

export function NewThreadEmptyState({
  newThreadWorkspaceEntry,
  selectableNewThreadWorkspaces,
  workspaceMutation,
  workspaceMode,
  onAddWorkspace,
  onSelectWorkspace,
  onWorkspaceModeChange,
  onOpenDreamThread,
  onResumeProviderSession,
  showDreams,
}: NewThreadEmptyStateProps) {
  const { t } = useI18n();
  const [resumeOpen, setResumeOpen] = useState(false);
  const [resumeLoading, setResumeLoading] = useState(false);
  const [resumeError, setResumeError] = useState<string | null>(null);
  const [resumeSessionId, setResumeSessionId] = useState("");
  const [selectedRecentSession, setSelectedRecentSession] =
    useState<DesktopProviderRecentSession | null>(null);
  const [resumeProviderTab, setResumeProviderTab] =
    useState<DesktopSessionProviderHint>("codex");
  const [recentSessionsByProvider, setRecentSessionsByProvider] = useState<
    Partial<Record<DesktopSessionProviderHint, DesktopProviderRecentSession[]>>
  >({});
  const [recentSessionsLoading, setRecentSessionsLoading] = useState(false);
  const [recentSessionsError, setRecentSessionsError] = useState<string | null>(
    null,
  );
  const [gitStatusResult, setGitStatusResult] = useState<{
    workspacePath: string;
    status: DesktopWorkspaceGitStatus;
  } | null>(null);

  useEffect(() => {
    setResumeError(null);
  }, [resumeOpen, resumeSessionId]);

  const selectedWorkspace = useMemo(
    () =>
      selectableNewThreadWorkspaces.find(
        (workspace) => workspace.path === (newThreadWorkspaceEntry?.path || ""),
      ) ??
      selectableNewThreadWorkspaces[0] ??
      null,
    [newThreadWorkspaceEntry?.path, selectableNewThreadWorkspaces],
  );
  const selectedWorkspacePath = selectedWorkspace?.path?.trim() || "";
  const worktreeCapable = Boolean(
    gitStatusResult?.workspacePath === selectedWorkspacePath &&
      gitStatusResult.status.isGitRepo,
  );
  useEffect(() => {
    let cancelled = false;
    setGitStatusResult(null);
    if (!selectedWorkspacePath) {
      onWorkspaceModeChange("local");
      return;
    }
    const cachedStatus = workspaceGitStatusCache.get(selectedWorkspacePath);
    if (cachedStatus) {
      setGitStatusResult({
        workspacePath: selectedWorkspacePath,
        status: cachedStatus,
      });
      if (!cachedStatus.isGitRepo) {
        onWorkspaceModeChange("local");
      }
      return;
    }

    const timeout = window.setTimeout(() => {
      void window.garyxDesktop
        .getWorkspaceGitStatus({ workspacePath: selectedWorkspacePath })
        .then((status) => {
          if (cancelled) return;
          workspaceGitStatusCache.set(selectedWorkspacePath, status);
          setGitStatusResult({ workspacePath: selectedWorkspacePath, status });
          if (!status.isGitRepo) {
            onWorkspaceModeChange("local");
          }
        })
        .catch(() => {
          if (cancelled) return;
          setGitStatusResult(null);
          onWorkspaceModeChange("local");
        });
    }, GIT_STATUS_CHECK_DELAY_MS);
    return () => {
      cancelled = true;
      window.clearTimeout(timeout);
    };
  }, [onWorkspaceModeChange, selectedWorkspacePath]);

  function closeResume() {
    setResumeOpen(false);
    setResumeSessionId("");
    setSelectedRecentSession(null);
    setResumeError(null);
    setRecentSessionsError(null);
  }

  const loadRecentSessions = useCallback(
    async (provider: DesktopSessionProviderHint, force = false) => {
      if (!force && recentSessionsByProvider[provider]) {
        return;
      }
      setRecentSessionsLoading(true);
      setRecentSessionsError(null);
      try {
        const sessions = await window.garyxDesktop.listProviderRecentSessions({
          provider,
          limit: 10,
        });
        setRecentSessionsByProvider((current) => ({
          ...current,
          [provider]: sessions,
        }));
      } catch (error) {
        setRecentSessionsError(
          error instanceof Error
            ? error.message
            : t("Failed to load recent sessions."),
        );
      } finally {
        setRecentSessionsLoading(false);
      }
    },
    [recentSessionsByProvider, t],
  );

  useEffect(() => {
    if (!resumeOpen) {
      return;
    }
    void loadRecentSessions(resumeProviderTab);
  }, [loadRecentSessions, resumeOpen, resumeProviderTab]);

  async function submitResume() {
    const selected = selectedRecentSession;
    const trimmed = resumeSessionId.trim();
    if (!selected && !trimmed) {
      setResumeError(t("Paste a session ID to continue."));
      return;
    }
    setResumeLoading(true);
    setResumeError(null);
    try {
      if (selected) {
        await onResumeProviderSession(
          selected.sessionId,
          selected.providerHint,
        );
      } else {
        await onResumeProviderSession(trimmed);
      }
      closeResume();
    } catch (error) {
      setResumeError(
        error instanceof Error ? error.message : t("Resume failed."),
      );
    } finally {
      setResumeLoading(false);
    }
  }

  const recentSessions = recentSessionsByProvider[resumeProviderTab] ?? [];
  const selectedRecentSessionKey = selectedRecentSession
    ? recentSessionKey(selectedRecentSession)
    : null;

  return (
    <>
      <div className="new-thread-empty-state">
        <div className="new-thread-option-row">
          {selectableNewThreadWorkspaces.length ? (
            <Select
              onValueChange={(value) => {
                if (value === ADD_WORKSPACE_VALUE) {
                  onAddWorkspace();
                  return;
                }
                if (value.startsWith(MISSING_WORKSPACE_VALUE_PREFIX)) {
                  return;
                }
                onWorkspaceModeChange("local");
                onSelectWorkspace(value);
              }}
              value={selectedWorkspace?.path ?? ""}
            >
              <SelectTrigger
                aria-label={t("Workspace for the new thread")}
                className="new-thread-workspace-trigger"
              >
                <SelectValue placeholder={t("Select a workspace")} />
              </SelectTrigger>
              <SelectContent
                align="start"
                className="new-thread-workspace-menu min-w-[var(--radix-select-trigger-width)]"
                position="popper"
                side="bottom"
                sideOffset={-1}
              >
                <SelectGroup>
                  <SelectLabel>
                    <IconSearch aria-hidden size={16} stroke={1.7} />
                    {t("Search projects")}
                  </SelectLabel>
                  {selectableNewThreadWorkspaces.map((workspace) => {
                    const value =
                      workspace.path ||
                      `${MISSING_WORKSPACE_VALUE_PREFIX}${workspace.name}`;
                    return (
                      <SelectItem
                        disabled={!workspace.available || !workspace.path}
                        key={workspace.path || workspace.name}
                        value={value}
                      >
                        <IconFolder aria-hidden size={16} stroke={1.7} />
                        <span className="new-thread-menu-text">
                          {workspace.available && workspace.path
                            ? workspace.name
                            : t("{name} (Unavailable)", {
                                name: workspace.name,
                              })}
                        </span>
                      </SelectItem>
                    );
                  })}
                </SelectGroup>
                <SelectSeparator />
                <SelectGroup>
                  <SelectItem
                    value={ADD_WORKSPACE_VALUE}
                    disabled={workspaceMutation === "add"}
                  >
                    <IconFolder aria-hidden size={16} stroke={1.7} />
                    <span className="new-thread-menu-text">
                      {workspaceMutation === "add"
                        ? t("Opening folder…")
                        : t("Choose folder…")}
                    </span>
                  </SelectItem>
                </SelectGroup>
              </SelectContent>
            </Select>
          ) : (
            <Button
              variant="outline"
              className="new-thread-workspace-trigger justify-center"
              disabled={workspaceMutation === "add"}
              onClick={onAddWorkspace}
            >
              <IconPlus aria-hidden size={14} stroke={1.8} />
              {workspaceMutation === "add"
                ? t("Opening folder…")
                : t("Choose a folder to begin")}
            </Button>
          )}

          {worktreeCapable ? (
            <Select
              onValueChange={(value) =>
                onWorkspaceModeChange(value as DesktopWorkspaceMode)
              }
              value={workspaceMode}
            >
              <SelectTrigger
                aria-label={t("Workspace mode")}
                className="new-thread-mode-trigger"
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent
                align="start"
                className="new-thread-mode-menu"
                position="popper"
                side="bottom"
                sideOffset={-1}
              >
                <SelectGroup>
                  <SelectLabel>{t("Workspace mode")}</SelectLabel>
                  <SelectItem value="local">
                    <IconDeviceLaptop aria-hidden size={16} stroke={1.7} />
                    <span className="new-thread-menu-text">{t("Local mode")}</span>
                  </SelectItem>
                  <SelectItem value="worktree">
                    <IconGitBranch aria-hidden size={16} stroke={1.7} />
                    <span className="new-thread-menu-text">{t("Worktree")}</span>
                  </SelectItem>
                </SelectGroup>
              </SelectContent>
            </Select>
          ) : null}

          <Button
            variant="ghost"
            size="sm"
            className="new-thread-resume-link"
            onClick={() => setResumeOpen(true)}
          >
            <IconHistory aria-hidden size={13} stroke={1.8} />
            {t("Resume")}
          </Button>
        </div>
        {showDreams ? (
          <NewThreadDreamsSummary onOpenThread={onOpenDreamThread} />
        ) : null}
      </div>

      <Dialog
        onOpenChange={(open) => {
          if (resumeLoading) return;
          if (!open) {
            closeResume();
            return;
          }
          setResumeOpen(true);
        }}
        open={resumeOpen}
      >
        <DialogContent
          className="sm:max-w-[420px]"
          showCloseButton={!resumeLoading}
          size="compact"
        >
          <DialogHeader>
            <DialogTitle>{t("Resume session")}</DialogTitle>
            <DialogDescription>
              {t("Paste a session ID or choose one of the latest local sessions.")}
            </DialogDescription>
          </DialogHeader>

          <Input
            aria-label={t("Existing provider session ID")}
            autoFocus
            disabled={resumeLoading}
            onChange={(event) => {
              setResumeSessionId(event.target.value);
              setSelectedRecentSession(null);
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.nativeEvent.isComposing) {
                event.preventDefault();
                void submitResume();
              }
            }}
            placeholder={t("Session ID")}
            spellCheck={false}
            value={resumeSessionId}
          />

          <section className="new-thread-resume-recent">
            <div className="new-thread-resume-recent-header">
              <span>{t("Choose recent")}</span>
              <Button
                aria-label={t("Refresh")}
                disabled={resumeLoading || recentSessionsLoading}
                onClick={() => {
                  setSelectedRecentSession(null);
                  void loadRecentSessions(resumeProviderTab, true);
                }}
                size="icon"
                type="button"
                variant="ghost"
              >
                <IconRefresh aria-hidden size={13} stroke={1.8} />
              </Button>
            </div>
            <div
              aria-label={t("Provider")}
              className="new-thread-resume-tabs"
              role="tablist"
            >
              {RESUME_PROVIDER_OPTIONS.map((provider) => (
                <button
                  aria-selected={resumeProviderTab === provider.value}
                  className="new-thread-resume-tab"
                  disabled={resumeLoading}
                  key={provider.value}
                  onClick={() => {
                    setResumeProviderTab(provider.value);
                    setSelectedRecentSession(null);
                  }}
                  role="tab"
                  type="button"
                >
                  {provider.label}
                </button>
              ))}
            </div>
            <div className="new-thread-resume-list">
              {recentSessionsLoading && !recentSessions.length ? (
                <p className="new-thread-resume-empty">{t("Loading…")}</p>
              ) : recentSessionsError ? (
                <p className="new-thread-resume-error">{recentSessionsError}</p>
              ) : recentSessions.length ? (
                recentSessions.map((session) => {
                  const sessionKey = recentSessionKey(session);
                  const isSelected = selectedRecentSessionKey === sessionKey;
                  return (
                    <button
                      aria-pressed={isSelected}
                      className={`new-thread-resume-session-row${
                        isSelected ? " is-selected" : ""
                      }`}
                      disabled={resumeLoading}
                      key={sessionKey}
                      onClick={() => {
                        setSelectedRecentSession(session);
                        setResumeError(null);
                      }}
                      type="button"
                    >
                      <span className="new-thread-resume-session-main">
                        <strong>{session.title}</strong>
                        <span>{workspaceLabel(session.workspaceDir)}</span>
                      </span>
                      <span className="new-thread-resume-session-meta">
                        <span>{formatRelativeTime(session.updatedAt)}</span>
                        <code>{shortSessionId(session.sessionId)}</code>
                      </span>
                    </button>
                  );
                })
              ) : (
                <p className="new-thread-resume-empty">
                  {t("No recent sessions found.")}
                </p>
              )}
            </div>
          </section>

          {resumeError ? (
            <p className="new-thread-resume-error">{resumeError}</p>
          ) : null}

          <DialogFooter>
            <Button
              variant="ghost"
              disabled={resumeLoading}
              onClick={closeResume}
              type="button"
            >
              {t("Cancel")}
            </Button>
            <Button
              disabled={
                resumeLoading ||
                (!selectedRecentSession && !resumeSessionId.trim())
              }
              onClick={() => void submitResume()}
              type="button"
            >
              {resumeLoading ? t("Resuming…") : t("Confirm")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

function recentSessionKey(session: DesktopProviderRecentSession): string {
  return `${session.providerHint}:${session.sessionId}`;
}

function shortSessionId(sessionId: string): string {
  const value = sessionId.trim();
  if (value.length <= 12) {
    return value;
  }
  return `${value.slice(0, 8)}…${value.slice(-4)}`;
}

function workspaceLabel(workspaceDir: string): string {
  const normalized = workspaceDir.trim();
  if (!normalized) {
    return "No workspace";
  }
  return normalized.split("/").filter(Boolean).pop() || normalized;
}

function formatRelativeTime(value?: string | null): string {
  if (!value) {
    return "";
  }
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) {
    return value;
  }
  const diffMs = parsed.getTime() - Date.now();
  const absMs = Math.abs(diffMs);
  const formatter = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  if (absMs < 60_000) {
    return formatter.format(Math.round(diffMs / 1000), "second");
  }
  if (absMs < 3_600_000) {
    return formatter.format(Math.round(diffMs / 60_000), "minute");
  }
  if (absMs < 86_400_000) {
    return formatter.format(Math.round(diffMs / 3_600_000), "hour");
  }
  return formatter.format(Math.round(diffMs / 86_400_000), "day");
}

function formatDreamTime(value?: string | null): string {
  if (!value) {
    return "";
  }
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) {
    return value;
  }
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(parsed);
}

function dreamTimeRange(dream: DesktopDreamTopic): string {
  const start = formatDreamTime(dream.firstMessageAt);
  const end = formatDreamTime(dream.lastMessageAt);
  if (start && end && start !== end) {
    return `${start} - ${end}`;
  }
  return end || start;
}

function firstDreamThreadId(dream: DesktopDreamTopic): string | null {
  return dream.spans[0]?.threadId ?? null;
}

function updateHomeDreamsCache(page: DesktopDreamsPage): DesktopDreamsPage {
  homeDreamsCache = { page, cachedAt: Date.now() };
  return page;
}

function loadHomeDreamsPage(options?: {
  force?: boolean;
}): Promise<DesktopDreamsPage> {
  const force = options?.force ?? false;
  const now = Date.now();
  if (
    !force &&
    homeDreamsCache &&
    now - homeDreamsCache.cachedAt < HOME_DREAMS_CACHE_STALE_MS
  ) {
    return Promise.resolve(homeDreamsCache.page);
  }
  if (!force && pendingHomeDreamsLoad) {
    return pendingHomeDreamsLoad;
  }

  const request = window.garyxDesktop
    .listDreams({
      sinceHours: 24,
      limit: HOME_DREAMS_LIMIT,
    })
    .then(updateHomeDreamsCache)
    .finally(() => {
      if (pendingHomeDreamsLoad === request) {
        pendingHomeDreamsLoad = null;
      }
    });
  pendingHomeDreamsLoad = request;
  return request;
}

function NewThreadDreamsSummary({
  onOpenThread,
}: {
  onOpenThread: (threadId: string) => void;
}) {
  const { t } = useI18n();
  const mountedRef = useRef(false);
  const [page, setPage] = useState<DesktopDreamsPage | null>(null);
  const [loading, setLoading] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const dreams = page?.dreams.slice(0, HOME_DREAMS_LIMIT) ?? [];
  const scan = page?.scan ?? page?.latestScan ?? null;
  const subtitle = scan?.createdAt
    ? `${t("Last scan")} ${formatDreamTime(scan.createdAt)}`
    : t("Last 24 hours");

  const loadDreams = useCallback(async (options?: { force?: boolean }) => {
    setLoading(true);
    setError(null);
    try {
      const result = await loadHomeDreamsPage(options);
      if (!mountedRef.current) {
        return;
      }
      setPage(result);
    } catch (cause) {
      if (!mountedRef.current) {
        return;
      }
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      if (mountedRef.current) {
        setLoading(false);
      }
    }
  }, []);

  const scanDreams = useCallback(async () => {
    setScanning(true);
    setError(null);
    try {
      const result = await window.garyxDesktop.scanDreams({
        sinceHours: 24,
        mode: "auto",
        limit: 600,
      });
      updateHomeDreamsCache(result);
      if (!mountedRef.current) {
        return;
      }
      setPage(result);
    } catch (cause) {
      if (!mountedRef.current) {
        return;
      }
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      if (mountedRef.current) {
        setScanning(false);
      }
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    void loadDreams();
    return () => {
      mountedRef.current = false;
    };
  }, [loadDreams]);

  return (
    <section
      aria-busy={loading || scanning}
      aria-label={t("Dreams")}
      className="new-thread-dreams"
    >
      <div className="new-thread-dreams-header">
        <div className="new-thread-dreams-title-block">
          <div className="new-thread-dreams-title-row">
            <h2>{t("Dreams")}</h2>
            <span className="new-thread-dreams-count">{dreams.length}</span>
          </div>
          <p>{subtitle}</p>
        </div>
        <div className="new-thread-dreams-actions">
          <Button
            aria-label={t("Refresh")}
            className="new-thread-dreams-action"
            disabled={loading || scanning}
            onClick={() => {
              void loadDreams({ force: true });
            }}
            size="icon"
            type="button"
            variant="ghost"
          >
            <IconRefresh aria-hidden size={14} stroke={1.8} />
          </Button>
          <Button
            className="new-thread-dreams-scan"
            disabled={loading || scanning}
            onClick={() => {
              void scanDreams();
            }}
            size="sm"
            type="button"
            variant="ghost"
          >
            <IconSparkles aria-hidden size={14} stroke={1.8} />
            {scanning ? t("Scanning") : t("Scan")}
          </Button>
        </div>
      </div>

      {error ? <p className="new-thread-dreams-error">{error}</p> : null}

      {!dreams.length && !loading && !error ? (
        <p className="new-thread-dreams-empty">{t("No dreams yet.")}</p>
      ) : null}

      {dreams.length ? (
        <div className="new-thread-dreams-list">
          {dreams.map((dream) => {
            const threadId = firstDreamThreadId(dream);
            return (
              <button
                className="new-thread-dream-row"
                disabled={!threadId}
                key={dream.dreamId}
                onClick={() => {
                  if (threadId) {
                    onOpenThread(threadId);
                  }
                }}
                type="button"
              >
                <span className="new-thread-dream-row-main">
                  <strong>{dream.title}</strong>
                  <span>{dream.summary}</span>
                </span>
                <span className="new-thread-dream-row-meta">
                  <span>{dreamTimeRange(dream)}</span>
                  <span>
                    {dream.messageCount} {t("messages")}
                  </span>
                </span>
              </button>
            );
          })}
        </div>
      ) : loading ? (
        <p className="new-thread-dreams-empty">{t("Loading…")}</p>
      ) : null}
    </section>
  );
}
