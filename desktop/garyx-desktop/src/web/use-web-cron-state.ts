import { useCallback, useEffect, useMemo, useState } from 'react';

import { fetchCronJobs, fetchCronRuns, type CronJobsPayload, type CronRunsPayload } from './web-api';
import type { WebRoute } from './web-route';

export function useWebCronState(_route: Extract<WebRoute, { view: 'cron' }>) {
  const [jobsPayload, setJobsPayload] = useState<CronJobsPayload | null>(null);
  const [runsPayload, setRunsPayload] = useState<CronRunsPayload | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [nextJobs, nextRuns] = await Promise.all([
        fetchCronJobs(),
        fetchCronRuns(),
      ]);
      setJobsPayload(nextJobs);
      setRunsPayload(nextRuns);
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : 'failed to load cron state');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const stats = useMemo(() => {
    const durations = (runsPayload?.runs || [])
      .map((run) => run.duration_ms)
      .filter((value): value is number => typeof value === 'number' && Number.isFinite(value));
    if (!durations.length) {
      return {
        avgDurationMs: null,
        maxDurationMs: null,
      };
    }
    const total = durations.reduce((sum, value) => sum + value, 0);
    return {
      avgDurationMs: total / durations.length,
      maxDurationMs: Math.max(...durations),
    };
  }, [runsPayload]);

  return {
    jobsPayload,
    runsPayload,
    loading,
    error,
    refresh,
    ...stats,
  };
}
