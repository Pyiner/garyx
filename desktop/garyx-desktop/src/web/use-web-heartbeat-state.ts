import { useCallback, useEffect, useState } from 'react';

import { fetchHeartbeatSummary, triggerHeartbeat } from './web-api';
import type { WebRoute } from './web-route';

export function useWebHeartbeatState(_route: Extract<WebRoute, { view: 'heartbeat' }>) {
  const [summary, setSummary] = useState<Record<string, unknown> | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [triggering, setTriggering] = useState(false);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setSummary(await fetchHeartbeatSummary());
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : 'failed to load heartbeat summary');
    } finally {
      setLoading(false);
    }
  }, []);

  const trigger = useCallback(async () => {
    setTriggering(true);
    setError(null);
    try {
      await triggerHeartbeat();
      await refresh();
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : 'failed to trigger heartbeat');
    } finally {
      setTriggering(false);
    }
  }, [refresh]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return {
    summary,
    loading,
    error,
    triggering,
    refresh,
    trigger,
  };
}
