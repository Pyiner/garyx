import { useEffect, useMemo, useState } from "react";
import {
  IconHistory,
  IconPlus,
  IconSparkles,
} from "@tabler/icons-react";

import type {
  DesktopSessionProviderHint,
  DesktopWorkspace,
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

type NewThreadEmptyStateProps = {
  newThreadWorkspaceEntry: DesktopWorkspace | null;
  selectableNewThreadWorkspaces: DesktopWorkspace[];
  workspaceMutation: string | null;
  onAddWorkspace: () => void;
  onSelectWorkspace: (workspaceId: string) => void;
  onResumeProviderSession: (
    sessionId: string,
    providerHint?: DesktopSessionProviderHint | null,
  ) => Promise<void>;
};

export function NewThreadEmptyState({
  newThreadWorkspaceEntry,
  selectableNewThreadWorkspaces,
  workspaceMutation,
  onAddWorkspace,
  onSelectWorkspace,
  onResumeProviderSession,
}: NewThreadEmptyStateProps) {
  const { t } = useI18n();
  const [resumeOpen, setResumeOpen] = useState(false);
  const [resumeLoading, setResumeLoading] = useState(false);
  const [resumeError, setResumeError] = useState<string | null>(null);
  const [resumeSessionId, setResumeSessionId] = useState("");

  useEffect(() => {
    setResumeError(null);
  }, [resumeOpen, resumeSessionId]);

  const selectedWorkspace = useMemo(
    () =>
      selectableNewThreadWorkspaces.find(
        (workspace) => workspace.id === (newThreadWorkspaceEntry?.id || ""),
      ) ??
      selectableNewThreadWorkspaces[0] ??
      null,
    [newThreadWorkspaceEntry?.id, selectableNewThreadWorkspaces],
  );

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
        <div className="new-thread-empty-mark" aria-hidden>
          <IconSparkles size={22} stroke={1.5} />
        </div>
        <h3>{t("Start a new thread")}</h3>

        {selectableNewThreadWorkspaces.length ? (
          <Select
            onValueChange={(value) => {
              if (value === ADD_WORKSPACE_VALUE) {
                onAddWorkspace();
                return;
              }
              onSelectWorkspace(value);
            }}
            value={selectedWorkspace?.id ?? ""}
          >
            <SelectTrigger
              aria-label={t("Workspace for the new thread")}
              className="new-thread-workspace-trigger"
              title={newThreadWorkspaceEntry?.path ?? undefined}
            >
              <SelectValue placeholder={t("Select a workspace")} />
            </SelectTrigger>
            <SelectContent
              align="start"
              className="min-w-[var(--radix-select-trigger-width)]"
            >
              <SelectGroup>
              <SelectLabel>{t("Folders")}</SelectLabel>
                {selectableNewThreadWorkspaces.map((workspace) => (
                  <SelectItem
                    disabled={!workspace.available}
                    key={workspace.id}
                    value={workspace.id}
                  >
                    {workspace.available
                      ? workspace.name
                      : t("{name} (Unavailable)", { name: workspace.name })}
                  </SelectItem>
                ))}
              </SelectGroup>
              <SelectSeparator />
              <SelectItem
                value={ADD_WORKSPACE_VALUE}
                disabled={workspaceMutation === "add"}
              >
                <IconPlus aria-hidden size={13} stroke={1.8} />
                {workspaceMutation === "add"
                  ? t("Opening folder…")
                  : t("Choose folder…")}
              </SelectItem>
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

        <Button
          variant="ghost"
          size="sm"
          className="new-thread-resume-link"
          onClick={() => setResumeOpen(true)}
        >
          <IconHistory aria-hidden size={13} stroke={1.8} />
          {t("Resume existing session")}
        </Button>
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
