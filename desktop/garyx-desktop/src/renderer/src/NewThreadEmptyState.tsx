import { useCallback, useEffect, useMemo, useState } from "react";
import { ChevronDown, Folder, GitBranch, History, Laptop, Plus, RefreshCw } from 'lucide-react';

import type {
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
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { WorkspaceSelectDialog } from "@/components/WorkspacePathPicker";
import { useI18n } from "./i18n";

const GIT_STATUS_CHECK_DELAY_MS = 120;
const RESUME_PROVIDER_OPTIONS: Array<{
  value: DesktopSessionProviderHint;
  label: string;
}> = [
  { value: "codex", label: "Codex" },
  { value: "claude", label: "Claude Code" },
];
const workspaceGitStatusCache = new Map<string, DesktopWorkspaceGitStatus>();
type NewThreadEmptyStateProps = {
  newThreadWorkspaceEntry: DesktopWorkspace | null;
  selectableNewThreadWorkspaces: DesktopWorkspace[];
  workspaceMutation: string | null;
  workspaceMode: DesktopWorkspaceMode;
  onAddWorkspace: () => void;
  onSelectWorkspace: (workspacePath: string) => void;
  onWorkspaceModeChange: (workspaceMode: DesktopWorkspaceMode) => void;
  onResumeProviderSession: (
    sessionId: string,
    providerHint?: DesktopSessionProviderHint | null,
  ) => Promise<void>;
};

export function NewThreadEmptyState({
  newThreadWorkspaceEntry,
  selectableNewThreadWorkspaces,
  workspaceMutation,
  workspaceMode,
  onAddWorkspace,
  onSelectWorkspace,
  onWorkspaceModeChange,
  onResumeProviderSession,
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
  const [workspacePickerOpen, setWorkspacePickerOpen] = useState(false);

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
    if (!trimmed) {
      setResumeError(t("Paste a session ID to continue."));
      return;
    }
    setResumeLoading(true);
    setResumeError(null);
    try {
      if (selected) {
        await onResumeProviderSession(
          trimmed,
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
            <button
              aria-label={t("Workspace for the new thread")}
              className="new-thread-workspace-trigger workspace-picker-trigger"
              onClick={() => setWorkspacePickerOpen(true)}
              type="button"
            >
              <Folder aria-hidden size={16} strokeWidth={1.7} />
              <span className="new-thread-menu-text">
                {selectedWorkspace?.name ?? t("Select a workspace")}
              </span>
              <ChevronDown aria-hidden size={14} strokeWidth={1.8} />
            </button>
          ) : (
            <Button
              variant="outline"
              className="new-thread-workspace-trigger justify-center"
              disabled={workspaceMutation === "add"}
              onClick={onAddWorkspace}
            >
              <Plus aria-hidden size={14} strokeWidth={1.8} />
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
                    <Laptop aria-hidden size={16} strokeWidth={1.7} />
                    <span className="new-thread-menu-text">{t("Local mode")}</span>
                  </SelectItem>
                  <SelectItem value="worktree">
                    <GitBranch aria-hidden size={16} strokeWidth={1.7} />
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
            <History aria-hidden size={13} strokeWidth={1.8} />
            {t("Resume")}
          </Button>
        </div>
      </div>

      <WorkspaceSelectDialog
        addWorkspaceBusy={workspaceMutation === "add"}
        onAddWorkspace={onAddWorkspace}
        onClose={() => setWorkspacePickerOpen(false)}
        onSelect={(workspacePath) => {
          onWorkspaceModeChange("local");
          onSelectWorkspace(workspacePath);
        }}
        open={workspacePickerOpen}
        selectedPath={selectedWorkspace?.path || ""}
        workspaces={selectableNewThreadWorkspaces}
      />

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
          className="new-thread-resume-picker"
          showCloseButton={!resumeLoading}
          size="compact"
        >
          <DialogHeader className="new-thread-resume-header">
            <div className="new-thread-resume-title-row">
              <DialogTitle>{t("Resume session")}</DialogTitle>
              <Button
                aria-label={t("Refresh")}
                className="new-thread-resume-refresh"
                disabled={resumeLoading || recentSessionsLoading}
                onClick={() => {
                  if (selectedRecentSession) {
                    setSelectedRecentSession(null);
                    setResumeSessionId("");
                  }
                  void loadRecentSessions(resumeProviderTab, true);
                }}
                size="icon"
                type="button"
                variant="ghost"
              >
                <RefreshCw aria-hidden size={13} strokeWidth={1.8} />
              </Button>
            </div>
          </DialogHeader>

          <section className="new-thread-resume-recent">
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
                    const providerChanged = resumeProviderTab !== provider.value;
                    setResumeProviderTab(provider.value);
                    if (providerChanged && selectedRecentSession) {
                      setResumeSessionId("");
                    }
                    if (providerChanged) {
                      setSelectedRecentSession(null);
                    }
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
                        setResumeSessionId(session.sessionId);
                        setResumeError(null);
                      }}
                      type="button"
                    >
                      <span
                        aria-hidden
                        className="new-thread-resume-session-indicator"
                      >
                        {isSelected ? "✓" : ""}
                      </span>
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

          <div className="new-thread-resume-target-row">
            <Input
              aria-label={t("Existing provider session ID")}
              className="new-thread-resume-session-input"
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
            <Button
              className="new-thread-resume-confirm"
              disabled={resumeLoading || !resumeSessionId.trim()}
              onClick={() => void submitResume()}
              type="button"
            >
              {resumeLoading ? t("Resuming…") : t("Resume")}
            </Button>
          </div>
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
