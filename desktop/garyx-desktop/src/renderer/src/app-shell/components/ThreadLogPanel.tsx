import { useCallback, useEffect, useLayoutEffect, useRef } from 'react';
import type { RefObject } from 'react';

import { Button } from '@/components/ui/button';

import type { ThreadLogLine } from '../types';
import { useI18n } from '../../i18n';

type ThreadLogPanelProps = {
  activeThreadTitle: string | null;
  selectedThreadId: string | null;
  activeThreadLogsPath: string;
  activeThreadLogsHasUnread: boolean;
  threadLogsError: string | null;
  threadLogsLoading: boolean;
  threadLogLines: ThreadLogLine[];
  threadLogsRef: RefObject<HTMLDivElement | null>;
  onJumpToLatest: () => void;
  onContentScroll: () => void;
};

export function ThreadLogPanel({
  activeThreadTitle,
  selectedThreadId,
  activeThreadLogsPath,
  activeThreadLogsHasUnread,
  threadLogsError,
  threadLogsLoading,
  threadLogLines,
  threadLogsRef,
  onJumpToLatest,
  onContentScroll,
}: ThreadLogPanelProps) {
  const { t } = useI18n();
  const threadLogsLabel = activeThreadTitle || selectedThreadId || t('Current thread logs');
  const shouldFollowTailRef = useRef(true);
  const followTailFrameRef = useRef<number | null>(null);
  const latestLogLineKey = threadLogLines[threadLogLines.length - 1]?.key ?? '';

  const cancelScheduledFollowTailScroll = useCallback(() => {
    if (followTailFrameRef.current !== null) {
      window.cancelAnimationFrame(followTailFrameRef.current);
      followTailFrameRef.current = null;
    }
  }, []);

  const scrollToLatestIfFollowing = useCallback(
    (behavior: ScrollBehavior = 'auto', force = false) => {
      if (force) {
        shouldFollowTailRef.current = true;
      }
      const scrollOnce = (scrollBehavior: ScrollBehavior, allowForce = false) => {
        if (!allowForce && !shouldFollowTailRef.current) {
          return;
        }
        const node = threadLogsRef.current;
        if (!node) {
          return;
        }
        node.scrollTo({
          top: node.scrollHeight,
          behavior: scrollBehavior,
        });
      };

      cancelScheduledFollowTailScroll();
      scrollOnce(behavior, force);

      let frameCount = 0;
      const scheduleNextFrame = () => {
        followTailFrameRef.current = window.requestAnimationFrame(() => {
          followTailFrameRef.current = null;
          if (!shouldFollowTailRef.current) {
            return;
          }
          scrollOnce('auto');
          const node = threadLogsRef.current;
          const bottomDelta = node
            ? node.scrollHeight - node.scrollTop - node.clientHeight
            : 0;
          if (bottomDelta > 2 && frameCount < 10) {
            frameCount += 1;
            scheduleNextFrame();
          }
        });
      };

      scheduleNextFrame();
    },
    [cancelScheduledFollowTailScroll, threadLogsRef],
  );

  useLayoutEffect(() => {
    scrollToLatestIfFollowing('auto', true);
  }, [scrollToLatestIfFollowing, selectedThreadId]);

  useLayoutEffect(() => {
    if (!threadLogLines.length) {
      return;
    }
    scrollToLatestIfFollowing('auto');
  }, [latestLogLineKey, scrollToLatestIfFollowing, threadLogLines.length]);

  useEffect(() => cancelScheduledFollowTailScroll, [cancelScheduledFollowTailScroll]);

  function handleJumpToLatest() {
    scrollToLatestIfFollowing('smooth', true);
    onJumpToLatest();
  }

  function handleContentScroll() {
    const node = threadLogsRef.current;
    if (node) {
      shouldFollowTailRef.current =
        node.scrollHeight - node.scrollTop - node.clientHeight < 48;
    }
    onContentScroll();
  }

  return (
    <aside
      aria-label={threadLogsLabel}
      className="thread-log-panel"
      title={activeThreadLogsPath}
    >
      <div className="thread-log-panel-toolbar">
        <div className="thread-log-panel-title">{t('Gateway Logs')}</div>
        {activeThreadLogsHasUnread ? (
          <div className="thread-log-panel-actions">
            <Button
              className="thread-log-panel-latest"
              onClick={handleJumpToLatest}
              size="xs"
              type="button"
              variant="ghost"
            >
              {t('Latest')}
            </Button>
          </div>
        ) : null}
      </div>

      {threadLogsError ? (
        <div className="thread-log-panel-error">{threadLogsError}</div>
      ) : null}

      <div
        className="thread-log-panel-content"
        onScroll={handleContentScroll}
        ref={threadLogsRef}
      >
        {threadLogLines.length ? (
          threadLogLines.map((line) => (
            <div
              className={`thread-log-line ${line.level === 'error' ? 'thread-log-line-error' : ''}`}
              key={line.key}
            >
              <span className="thread-log-line-text">{line.text || '\u00A0'}</span>
            </div>
          ))
        ) : (
          <div className="thread-log-panel-empty">
            {threadLogsLoading ? t('Loading logs…') : t('No logs yet.')}
          </div>
        )}
      </div>
    </aside>
  );
}
