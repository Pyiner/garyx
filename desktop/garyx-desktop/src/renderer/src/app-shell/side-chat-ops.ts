// Side-chat operations shared by the shell and the panel (endgame batch
// 5b-7b, docs/design/appshell-sidechat-colocation.md). Both entry points
// live OUTSIDE the panel's lifetime: `ensureSideChatThread` is invoked by
// the dock header's chat auto-open (panel not yet mounted) and by the
// panel's composer submit. It is a plain async command over the session
// store + mirror facades.

import type {
  DesktopState,
  DesktopThreadSummary,
} from "@shared/contracts";

import type { GatewayMirror } from "../gateway-mirror/mirror";
import { requestDesktopStateResult } from "../pinned-order-ingress.ts";
import type { MessageMap } from "./types";
import type { SideChatSessions } from "./side-chat-sessions";

export interface SideChatOpsContext {
  sessions: SideChatSessions;
  mirror: GatewayMirror;
  /** Render-time shell truth, captured by the caller at dispatch. */
  sourceThreadId: string | null;
  activeThread: DesktopThreadSummary | null;
  threadSummaryById: Map<string, DesktopThreadSummary>;
  setDesktopState: React.Dispatch<React.SetStateAction<DesktopState | null>>;
  setError: (error: string | null) => void;
}

export function sideChatForkAgentId(
  sourceThread: Pick<DesktopThreadSummary, "agentId"> | null | undefined,
): string | null {
  return sourceThread?.agentId?.trim() || null;
}

/**
 * Resolve (or create) the side thread bound to the active source thread.
 * Verbatim orchestration from the dissolved controller: adopt an existing
 * binding when the thread is still openable, otherwise create a hidden
 * fork with in-flight de-dupe through the session store.
 */
export async function ensureSideChatThread(
  ctx: SideChatOpsContext,
): Promise<string | null> {
  const {
    sessions,
    mirror,
    activeThread,
    threadSummaryById,
    setDesktopState,
    setError,
  } = ctx;
  const sourceThreadId = ctx.sourceThreadId;
  if (!sourceThreadId) {
    return null;
  }

  const existingThreadId =
    sessions.threadFor(sourceThreadId) ||
    sessions.restorePersisted(sourceThreadId);
  if (existingThreadId) {
    try {
      if (await mirror.ensureThreadOpenable(existingThreadId)) {
        sessions.rememberSideThreadId(existingThreadId);
        sessions.rememberThread(sourceThreadId, existingThreadId);
        sessions.setError(sourceThreadId, null);
        return existingThreadId;
      }
    } catch {
      sessions.forgetThread(sourceThreadId, existingThreadId);
    }
  }

  const inFlight = sessions.creationPromiseFor(sourceThreadId);
  if (inFlight) {
    return inFlight;
  }

  const creation = (async () => {
    sessions.setCreating(sourceThreadId, true);
    sessions.setError(sourceThreadId, null);

    try {
      const sourceThread =
        threadSummaryById.get(sourceThreadId) || activeThread || null;
      const created = await requestDesktopStateResult(
        () => window.garyxDesktop.createThread({
          title: "Side chat",
          agentId: sideChatForkAgentId(sourceThread),
          forkFromThreadId: sourceThreadId,
          metadata: {
            source: "side_chat",
            hidden: true,
            side_chat_parent_thread_id: sourceThreadId,
          },
        }),
        (response) => response.state,
      );
      // The main-process snapshot already carries every retained hidden
      // session for the current scope (the dedicated hidden-session store
      // folds them in), so it is committed AS-IS: spreading it would strip
      // the ingress envelope and let a stale-gateway state slip past the
      // identity checks.
      setDesktopState(created.state);
      mirror.updateMessagesByThread((current: MessageMap) => ({
        ...current,
        [created.thread.id]: current[created.thread.id] || [],
      }));
      sessions.rememberSideThreadId(created.thread.id);
      sessions.rememberThread(sourceThreadId, created.thread.id);
      return created.thread.id;
    } catch (createError) {
      const message =
        createError instanceof Error
          ? createError.message
          : "Failed to start side chat.";
      sessions.setError(sourceThreadId, message);
      setError(message);
      return null;
    } finally {
      sessions.setCreating(sourceThreadId, false);
      sessions.setCreationPromise(sourceThreadId, null);
    }
  })();

  sessions.setCreationPromise(sourceThreadId, creation);
  return creation;
}
