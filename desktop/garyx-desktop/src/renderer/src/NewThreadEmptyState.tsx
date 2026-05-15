import { useEffect, useMemo, useState } from "react";
import {
  IconDeviceLaptop,
  IconFolder,
  IconGitBranch,
  IconHistory,
  IconPlus,
  IconSearch,
} from "@tabler/icons-react";

import type {
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
      onWorkspaceModeChange("direct");
      return;
    }
    const cachedStatus = workspaceGitStatusCache.get(selectedWorkspacePath);
    if (cachedStatus) {
      setGitStatusResult({
        workspacePath: selectedWorkspacePath,
        status: cachedStatus,
      });
      if (!cachedStatus.isGitRepo) {
        onWorkspaceModeChange("direct");
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
            onWorkspaceModeChange("direct");
          }
        })
        .catch(() => {
          if (cancelled) return;
          setGitStatusResult(null);
          onWorkspaceModeChange("direct");
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
    setResumeError(null);
  }

  async function submitResume() {
    const trimmed = resumeSessionId.trim();
    if (!trimmed) {
      setResumeError(t("Paste a session ID to continue."));
      return;
    }
    setResumeLoading(true);
    setResumeError(null);
    try {
      await onResumeProviderSession(trimmed);
      closeResume();
    } catch (error) {
      setResumeError(
        error instanceof Error ? error.message : t("Resume failed."),
      );
    } finally {
      setResumeLoading(false);
    }
  }

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
                onWorkspaceModeChange("direct");
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
                    const value = workspace.path || `${MISSING_WORKSPACE_VALUE_PREFIX}${workspace.name}`;
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
                            : t("{name} (Unavailable)", { name: workspace.name })}
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
                  <SelectItem value="direct">
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
              {t("Paste a Claude, Codex, or Gemini session ID. Garyx will recover its workspace and bind a thread to it.")}
            </DialogDescription>
          </DialogHeader>

          <Input
            aria-label={t("Existing provider session ID")}
            autoFocus
            disabled={resumeLoading}
            onChange={(event) => setResumeSessionId(event.target.value)}
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
              disabled={resumeLoading || !resumeSessionId.trim()}
              onClick={() => void submitResume()}
              type="button"
            >
              {resumeLoading ? t("Searching…") : t("Resume")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
