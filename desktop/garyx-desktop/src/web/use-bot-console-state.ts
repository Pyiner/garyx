import { useCallback, useEffect, useMemo, useState } from 'react';

import type { DesktopBotConsoleSummary } from '@shared/contracts';

import { fetchBotConsoles } from './web-api';
import type { WebRoute } from './web-route';

export function useBotConsoleState(route: Extract<WebRoute, { view: 'bot-console' }>) {
  const [groups, setGroups] = useState<DesktopBotConsoleSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const nextGroups = await fetchBotConsoles();
      const filteredGroups = nextGroups.filter((group) => {
        if (route.botId && group.id !== route.botId) {
          return false;
        }
        if (route.endpointKey) {
          return group.endpoints.some((endpoint) => endpoint.endpointKey === route.endpointKey);
        }
        return true;
      });
      setGroups(filteredGroups);
      const hasBotMatch = route.botId
        ? nextGroups.some((group) => group.id === route.botId)
        : true;
      const hasEndpointMatch = route.endpointKey
        ? nextGroups.some((group) => group.endpoints.some((endpoint) => endpoint.endpointKey === route.endpointKey))
        : true;
      if ((route.botId || route.endpointKey) && filteredGroups.length === 0 && (!hasBotMatch || !hasEndpointMatch)) {
        setError('deep link target not found in current bot console payload');
      }
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : 'failed to load bot console');
    } finally {
      setLoading(false);
    }
  }, [route.botId, route.endpointKey]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const totalEndpoints = useMemo(() => {
    return groups.reduce((count, group) => count + group.endpointCount, 0);
  }, [groups]);

  return {
    groups,
    loading,
    error,
    status,
    totalEndpoints,
    refresh,
  };
}
