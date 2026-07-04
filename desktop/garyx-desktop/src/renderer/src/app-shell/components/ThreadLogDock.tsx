// Thread log dock (endgame architecture batch 5b, "Local state colocation
// list": ThreadLogPanel owns log text/path/cursor/loading/unread).
//
// The dock owns the gateway-log polling and its content state, moved
// verbatim from AppShell. It mounts only while the log panel is open
// (ThreadPage renders it behind its threadLogsOpen flag), so the legacy
// effect's open/content-view gates become the mount boundary and the
// 1s polling cadence re-renders only this subtree, not the shell.
//
// The unread flag is the one piece the shell still needs (the
// conversation-header badge); it is mirrored up through onUnreadChange,
// which fires only when the flag actually flips.

import { useEffect, useMemo, useRef, useState } from "react";

import {
  buildThreadLogLines,
  keepRecentThreadLogLines,
} from "../diagnostics-helpers";
import { ThreadLogPanel } from "./ThreadLogPanel";

type ThreadLogDockProps = {
  activeThreadTitle: string | null;
  onUnreadChange: (hasUnread: boolean) => void;
  threadId: string | null;
};

export function ThreadLogDock({
  activeThreadTitle,
  onUnreadChange,
  threadId,
}: ThreadLogDockProps) {
  const [threadLogsText, setThreadLogsText] = useState("");
  const [threadLogsPath, setThreadLogsPath] = useState("");
  const [threadLogsLoading, setThreadLogsLoading] = useState(false);
  const [threadLogsError, setThreadLogsError] = useState<string | null>(null);
  const [threadLogsHasUnread, setThreadLogsHasUnreadRaw] = useState(false);
  const threadLogsCursorRef = useRef(0);
  const threadLogsRef = useRef<HTMLDivElement | null>(null);
  const onUnreadChangeRef = useRef(onUnreadChange);
  useEffect(() => {
    onUnreadChangeRef.current = onUnreadChange;
  });

  const setThreadLogsHasUnread = setThreadLogsHasUnreadRaw;
  // Mirror the unread flag to the shell (header badge) after commit —
  // state updaters must stay pure, so the notification is an effect.
  useEffect(() => {
    onUnreadChangeRef.current(threadLogsHasUnread);
  }, [threadLogsHasUnread]);
  // The dock unmounts when the panel closes; the badge must not stay lit.
  useEffect(() => {
    return () => {
      onUnreadChangeRef.current(false);
    };
  }, []);

  function threadLogsNearBottom() {
    const node = threadLogsRef.current;
    if (!node) {
      return true;
    }
    return node.scrollHeight - node.scrollTop - node.clientHeight < 48;
  }

  function scrollThreadLogsToLatest(behavior: ScrollBehavior = "auto") {
    const node = threadLogsRef.current;
    if (!node) {
      return;
    }
    node.scrollTo({
      top: node.scrollHeight,
      behavior,
    });
  }

  useEffect(() => {
    if (!threadId) {
      return;
    }

    let cancelled = false;
    let polling = false;

    setThreadLogsLoading(true);
    setThreadLogsError(null);
    setThreadLogsHasUnread(false);
    setThreadLogsText("");
    setThreadLogsPath("");
    threadLogsCursorRef.current = 0;

    const loadLogs = async (cursor?: number) => {
      if (cancelled || polling) {
        return;
      }
      polling = true;
      const wasNearBottom = threadLogsNearBottom();
      try {
        const chunk = await window.garyxDesktop.getThreadLogs(
          threadId,
          cursor,
        );
        if (cancelled) {
          return;
        }
        setThreadLogsPath(chunk.path);
        threadLogsCursorRef.current = chunk.cursor;
        setThreadLogsError(null);
        setThreadLogsLoading(false);
        if (chunk.reset) {
          setThreadLogsText(keepRecentThreadLogLines(chunk.text));
          setThreadLogsHasUnread(false);
          window.requestAnimationFrame(() => {
            scrollThreadLogsToLatest("auto");
          });
          return;
        }
        if (!chunk.text) {
          return;
        }
        setThreadLogsText((current) =>
          keepRecentThreadLogLines(current + chunk.text),
        );
        if (wasNearBottom) {
          setThreadLogsHasUnread(false);
          window.requestAnimationFrame(() => {
            scrollThreadLogsToLatest("auto");
          });
        } else {
          setThreadLogsHasUnread(true);
        }
      } catch (loadError) {
        if (!cancelled) {
          setThreadLogsLoading(false);
          setThreadLogsError(
            loadError instanceof Error
              ? loadError.message
              : "Failed to load thread logs",
          );
        }
      } finally {
        polling = false;
      }
    };

    void loadLogs();
    const timer = window.setInterval(() => {
      if (document.hidden) {
        return;
      }
      void loadLogs(threadLogsCursorRef.current);
    }, 1000);

    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [threadId]);

  useEffect(() => {
    if (!threadId) {
      return;
    }
    setThreadLogsHasUnread(false);
    window.requestAnimationFrame(() => {
      scrollThreadLogsToLatest("auto");
    });
  }, [threadId]);

  const threadLogLines = useMemo(
    () => buildThreadLogLines(threadLogsText),
    [threadLogsText],
  );
  const activeThreadLogsPath = threadLogsPath || "Waiting for log file";

  return (
    <ThreadLogPanel
      activeThreadLogsHasUnread={threadLogsHasUnread}
      activeThreadLogsPath={activeThreadLogsPath}
      activeThreadTitle={activeThreadTitle}
      threadLogLines={threadLogLines}
      onContentScroll={() => {
        if (threadLogsNearBottom()) {
          setThreadLogsHasUnread(false);
        }
      }}
      onJumpToLatest={() => {
        setThreadLogsHasUnread(false);
        scrollThreadLogsToLatest("smooth");
      }}
      selectedThreadId={threadId}
      threadLogsError={threadLogsError}
      threadLogsLoading={threadLogsLoading}
      threadLogsRef={threadLogsRef}
    />
  );
}
