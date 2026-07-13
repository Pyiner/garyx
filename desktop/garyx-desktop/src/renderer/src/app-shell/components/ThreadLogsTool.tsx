// Gateway-log content for the Logs tab inside the right side-tools rail.
// The polling loop and content state remain colocated here, so switching
// away from the tab or closing the rail unmounts this subtree and stops work.

import { useEffect, useMemo, useRef, useState } from "react";

import {
  buildThreadLogLines,
  keepRecentThreadLogLines,
} from "../diagnostics-helpers";
import { ThreadLogPanel } from "./ThreadLogPanel";

type ThreadLogsToolProps = {
  activeThreadTitle: string | null;
  threadId: string | null;
};

export function ThreadLogsTool({
  activeThreadTitle,
  threadId,
}: ThreadLogsToolProps) {
  const [threadLogsText, setThreadLogsText] = useState("");
  const [threadLogsPath, setThreadLogsPath] = useState("");
  const [threadLogsLoading, setThreadLogsLoading] = useState(false);
  const [threadLogsError, setThreadLogsError] = useState<string | null>(null);
  const [threadLogsHasUnread, setThreadLogsHasUnread] = useState(false);
  const threadLogsCursorRef = useRef(0);
  const threadLogsRef = useRef<HTMLDivElement | null>(null);

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
