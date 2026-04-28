import { useCallback, useEffect, useState } from 'react';

import { fetchAgentView, fetchOverview } from './web-api';
import type { WebRoute } from './web-route';

export function useWebStatusState(_route: Extract<WebRoute, { view: 'status' }>) {
  const [overview, setOverview] = useState<Record<string, unknown> | null>(null);
  const [agentView, setAgentView] = useState<Record<string, unknown> | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [nextOverview, nextAgentView] = await Promise.all([
        fetchOverview(),
        fetchAgentView(),
      ]);
      setOverview(nextOverview);
      setAgentView(nextAgentView);
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : 'failed to load gateway status');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return {
    overview,
    agentView,
    loading,
    error,
    refresh,
  };
}
