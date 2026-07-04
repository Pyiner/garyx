// Conversation title feature root (endgame architecture batch 5b,
// "Local state colocation list": ConversationHeaderTitle owns title edit
// state).
//
// Owns the edit lifecycle — draft, editing flag, saving flag, the input
// ref and its focus/select effect — so per-keystroke draft updates
// re-render only this root, not the shell. The shell passes the derived
// context (active thread, canEdit, view flags) and keeps an imperative
// handle for the one out-of-band writer: the transcript controller's
// remote title sync, whose not-editing guard moves here with the state.

import {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useRef,
  useState,
} from "react";

import type {
  DesktopBotConsoleSummary,
  DesktopState,
  DesktopThreadSummary,
} from "@shared/contracts";
import { DEFAULT_SESSION_TITLE } from "@shared/contracts";

import { ConversationHeaderTitle } from "../../ConversationHeaderTitle";
import { getDesktopApi } from "../../platform/desktop-api";
import { saveThreadTitle } from "../../thread-controller";

export interface ConversationTitleHandle {
  /**
   * Remote title sync (transcript-controller seam): adopt the new title
   * into the draft unless the user is mid-edit — the guard the legacy
   * caller applied before its setTitleDraft.
   */
  syncTitle(nextTitle: string): void;
}

type ConversationTitleRootProps = {
  activeAutomationThread: boolean;
  activeThread: Pick<DesktopThreadSummary, "id" | "title"> | null;
  activeThreadBot: DesktopBotConsoleSummary | null;
  activeWorkspaceName: string | null;
  archiveThreadDisabled: boolean;
  canEditThreadTitle: boolean;
  contextText: string | null;
  isAutomationView: boolean;
  isBotsView: boolean;
  isSkillsView: boolean;
  isThreadPinned: boolean;
  onArchiveThread: () => void;
  onTogglePinnedThread: () => void;
  setDesktopState: (value: DesktopState) => void;
  setError: (value: string | null) => void;
};

export const ConversationTitleRoot = forwardRef<
  ConversationTitleHandle,
  ConversationTitleRootProps
>(function ConversationTitleRoot(
  {
    activeAutomationThread,
    activeThread,
    activeThreadBot,
    activeWorkspaceName,
    archiveThreadDisabled,
    canEditThreadTitle,
    contextText,
    isAutomationView,
    isBotsView,
    isSkillsView,
    isThreadPinned,
    onArchiveThread,
    onTogglePinnedThread,
    setDesktopState,
    setError,
  },
  ref,
) {
  const [titleDraft, setTitleDraft] = useState(DEFAULT_SESSION_TITLE);
  const [savingTitle, setSavingTitle] = useState(false);
  const [editingThreadTitle, setEditingThreadTitle] = useState(false);
  const threadTitleInputRef = useRef<HTMLInputElement | null>(null);
  const editingThreadTitleRef = useRef(false);
  editingThreadTitleRef.current = editingThreadTitle;

  useImperativeHandle(
    ref,
    () => ({
      syncTitle: (nextTitle: string) => {
        if (!editingThreadTitleRef.current) {
          setTitleDraft(nextTitle);
        }
      },
    }),
    [],
  );

  useEffect(() => {
    if (!canEditThreadTitle && editingThreadTitle) {
      setEditingThreadTitle(false);
    }
  }, [canEditThreadTitle, editingThreadTitle]);

  // Legacy shell effect: leaving the thread (or switching threads) always
  // exits edit mode, even between two editable threads.
  const activeThreadId = activeThread?.id ?? null;
  useEffect(() => {
    setEditingThreadTitle(false);
  }, [activeThreadId]);

  useEffect(() => {
    if (!editingThreadTitle) {
      setTitleDraft(activeThread?.title || DEFAULT_SESSION_TITLE);
    }
  }, [editingThreadTitle, activeThread?.title]);

  useEffect(() => {
    if (!editingThreadTitle) {
      return;
    }
    const node = threadTitleInputRef.current;
    if (!node) {
      return;
    }
    node.focus();
    node.select();
  }, [editingThreadTitle]);

  function beginThreadTitleEdit() {
    if (!canEditThreadTitle || !activeThread) {
      return;
    }
    setTitleDraft(activeThread.title || DEFAULT_SESSION_TITLE);
    setEditingThreadTitle(true);
  }

  async function handleSaveTitle(options?: { closeEditor?: boolean }) {
    await saveThreadTitle({
      api: getDesktopApi(),
      activeThread: activeThread,
      activeAutomationThread,
      titleDraft,
      closeEditor: options?.closeEditor,
      defaultTitle: DEFAULT_SESSION_TITLE,
      setError,
      setSavingTitle,
      setDesktopState,
      setTitleDraft,
      setEditingThreadTitle,
    });
  }

  function cancelThreadTitleEdit() {
    setEditingThreadTitle(false);
    setTitleDraft(activeThread?.title || DEFAULT_SESSION_TITLE);
  }

  return (
    <ConversationHeaderTitle
      activeThreadBot={activeThreadBot}
      activeThreadTitle={activeThread?.title || null}
      activeWorkspaceName={activeWorkspaceName}
      canEditThreadTitle={canEditThreadTitle}
      contextText={contextText}
      editingThreadTitle={editingThreadTitle}
      isAutomationView={isAutomationView}
      isBotsView={isBotsView}
      isSkillsView={isSkillsView}
      isThreadPinned={isThreadPinned}
      archiveThreadDisabled={archiveThreadDisabled}
      onBeginEdit={beginThreadTitleEdit}
      onArchiveThread={onArchiveThread}
      onCancelEdit={cancelThreadTitleEdit}
      onSaveTitle={() => {
        void handleSaveTitle({ closeEditor: true });
      }}
      onTogglePinnedThread={onTogglePinnedThread}
      onTitleDraftChange={setTitleDraft}
      savingTitle={savingTitle}
      titleDraft={titleDraft}
      titleInputRef={threadTitleInputRef}
    />
  );
});
