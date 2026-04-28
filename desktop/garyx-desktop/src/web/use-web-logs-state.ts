import { useCallback, useEffect, useState } from 'react';

import { fetchLogsTail, parseLogLine, type LogTailPayload, type ParsedLogLine } from './web-api';
import type { WebRoute } from './web-route';

export function useWebLogsState(_route: Extract<WebRoute, { view: 'logs' }>) {
  const [payload, setPayload] = useState<LogTailPayload | null>(null);
  const [lines, setLines] = useState<ParsedLogLine[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [level, setLevel] = useState('');

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const nextPayload = await fetchLogsTail(level);
      setPayload(nextPayload);
      setLines((nextPayload.lines || []).map(parseLogLine).reverse());
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : 'failed to load gateway logs');
    } finally {
      setLoading(false);
    }
  }, [level]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return {
    payload,
    lines,
    loading,
    error,
    level,
    setLevel,
    refresh,
  };
}
