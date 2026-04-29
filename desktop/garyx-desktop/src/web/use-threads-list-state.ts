import { useCallback, useEffect, useState } from 'react';

import type { DesktopThreadSummary } from '@shared/contracts';

import { fetchThreads } from './web-api';
import type { WebRoute } from './web-route';

export function useThreadsListState(_route: Extract<WebRoute, { view: 'threads' }>) {
  const [threads, setThreads] = useState<DesktopThreadSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

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

  return {
    threads,
    visibleThreads: threads,
    loading,
    error,
    refresh,
    totalThreadsCount: threads.length,
  };
}
