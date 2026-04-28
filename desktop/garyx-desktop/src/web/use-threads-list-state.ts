import { useCallback, useEffect, useMemo, useState } from 'react';

import type { DesktopThreadSummary } from '@shared/contracts';

import { fetchThreads } from './web-api';
import type { WebRoute } from './web-route';

function isHeartbeatThread(thread: DesktopThreadSummary): boolean {
  const threadId = (thread.id || '').toLowerCase();
  return threadId.includes('::heartbeat::') || threadId.startsWith('heartbeat::');
}

export function useThreadsListState(_route: Extract<WebRoute, { view: 'threads' }>) {
  const [threads, setThreads] = useState<DesktopThreadSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<'normal' | 'heartbeat'>('normal');

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setThreads(await fetchThreads());
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : 'failed to load threads');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const normalThreadsCount = useMemo(() => {
    return threads.filter((thread) => !isHeartbeatThread(thread)).length;
  }, [threads]);

  const heartbeatThreadsCount = useMemo(() => {
    return threads.filter(isHeartbeatThread).length;
  }, [threads]);

  const visibleThreads = useMemo(() => {
    return threads.filter((thread) => (filter === 'heartbeat' ? isHeartbeatThread(thread) : !isHeartbeatThread(thread)));
  }, [filter, threads]);

  return {
    threads,
    visibleThreads,
    loading,
    error,
    filter,
    setFilter,
    refresh,
    normalThreadsCount,
    heartbeatThreadsCount,
    totalThreadsCount: threads.length,
  };
}
