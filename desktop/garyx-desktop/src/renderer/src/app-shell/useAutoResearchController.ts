import { useEffect, useRef, useState } from 'react';

import type {
  CandidatesResponse,
  CreateAutoResearchRunInput,
  DesktopAutoResearchIteration,
  DesktopAutoResearchRun,
  DesktopAutoResearchRunDetail,
} from '@shared/contracts';

export function useAutoResearchController(
  enabled: boolean,
  setError: React.Dispatch<React.SetStateAction<string | null>>,
) {
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [runs, setRuns] = useState<DesktopAutoResearchRun[]>([]);
  const [runDetail, setRunDetail] = useState<DesktopAutoResearchRunDetail | null>(null);
  const [iterations, setIterations] = useState<DesktopAutoResearchIteration[]>([]);
  const [candidatesResponse, setCandidatesResponse] = useState<CandidatesResponse | null>(null);
  const pollRunIdRef = useRef<string | null>(null);

  async function loadRuns(selectedRunId?: string | null) {
    setLoading(true);
    setError(null);
    try {
      const nextRuns = await window.garyxDesktop.listAutoResearchRuns({ limit: 50 });
      setRuns(nextRuns);
      const preferredRunId = selectedRunId
        || pollRunIdRef.current
        || runDetail?.run.runId
        || nextRuns[0]?.runId
        || null;
      if (!preferredRunId) {
        setRunDetail(null);
        setIterations([]);
        pollRunIdRef.current = null;
        return null;
      }
      return await loadRun(preferredRunId, false);
    } catch (error) {
      setError(error instanceof Error ? error.message : 'Failed to load Auto Research runs');
      throw error;
    } finally {
      setLoading(false);
    }
  }

  async function createRun(input: CreateAutoResearchRunInput) {
    setSaving(true);
    setError(null);
    try {
      const run = await window.garyxDesktop.createAutoResearchRun(input);
      const [detail, nextIterations] = await Promise.all([
        window.garyxDesktop.getAutoResearchRun(run.runId),
        window.garyxDesktop.listAutoResearchIterations(run.runId),
      ]);
      setRuns((current) => [run, ...current.filter((item) => item.runId !== run.runId)]);
      setRunDetail(detail);
      setIterations(nextIterations);
      pollRunIdRef.current = run.runId;
      return detail;
    } catch (error) {
      setError(error instanceof Error ? error.message : 'Failed to create Auto Research run');
      throw error;
    } finally {
      setSaving(false);
    }
  }

  async function loadRun(runId: string, manageLoading = true) {
    if (manageLoading) {
      setLoading(true);
      setError(null);
    }
    try {
      const [detail, nextIterations] = await Promise.all([
        window.garyxDesktop.getAutoResearchRun(runId),
        window.garyxDesktop.listAutoResearchIterations(runId),
      ]);
      setRuns((current) => {
        const next = [detail.run, ...current.filter((item) => item.runId !== detail.run.runId)];
        next.sort((left, right) => right.updatedAt.localeCompare(left.updatedAt));
        return next;
      });
      setRunDetail(detail);
      setIterations(nextIterations);
      pollRunIdRef.current = runId;
      try {
        const resp = await window.garyxDesktop.listAutoResearchCandidates({ runId });
        setCandidatesResponse(resp);
      } catch {
        setCandidatesResponse(null);
      }
      return detail;
    } catch (error) {
      if (manageLoading) {
        setError(error instanceof Error ? error.message : 'Failed to load Auto Research run');
      }
      throw error;
    } finally {
      if (manageLoading) {
        setLoading(false);
      }
    }
  }

  async function selectCandidate(runId: string, candidateId: string) {
    setSaving(true);
    setError(null);
    try {
      const run = await window.garyxDesktop.selectAutoResearchCandidate({ runId, candidateId });
      setRuns((current) =>
        current.map((item) => (item.runId === run.runId ? run : item)),
      );
      setRunDetail((current) => {
        if (!current) return { run, latestIteration: null };
        return { ...current, run };
      });
      // Refresh candidates after selection
      try {
        const resp = await window.garyxDesktop.listAutoResearchCandidates({ runId });
        setCandidatesResponse(resp);
      } catch {
        // ignore
      }
      return run;
    } catch (error) {
      setError(error instanceof Error ? error.message : 'Failed to select candidate');
      throw error;
    } finally {
      setSaving(false);
    }
  }

  async function stopRun(runId: string) {
    setSaving(true);
    setError(null);
    try {
      const run = await window.garyxDesktop.stopAutoResearchRun({ runId, reason: 'user_requested' });
      setRuns((current) =>
        current.map((item) => (item.runId === run.runId ? run : item)),
      );
      setRunDetail((current) => {
        if (!current) {
          return { run, latestIteration: null };
        }
        return { ...current, run };
      });
      pollRunIdRef.current = null;
      return run;
    } catch (error) {
      setError(error instanceof Error ? error.message : 'Failed to stop Auto Research run');
      throw error;
    } finally {
      setSaving(false);
    }
  }

  async function deleteRun(runId: string) {
    setSaving(true);
    setError(null);
    try {
      await window.garyxDesktop.deleteAutoResearchRun(runId);
      setRuns((current) => current.filter((item) => item.runId !== runId));
      if (runDetail?.run.runId === runId) {
        setRunDetail(null);
      }
      pollRunIdRef.current = null;
    } catch (error) {
      setError(error instanceof Error ? error.message : 'Failed to delete Auto Research run');
      throw error;
    } finally {
      setSaving(false);
    }
  }

  useEffect(() => {
    if (!enabled) {
      return;
    }
    void loadRuns().catch(() => {});
  }, [enabled]);

  useEffect(() => {
    if (!enabled) {
      return;
    }
    const currentRunId = pollRunIdRef.current || runDetail?.run.runId || null;
    const currentState = runDetail?.run.state || null;
    if (!currentRunId) {
      return;
    }
    if (
      currentState === 'budget_exhausted'
      || currentState === 'blocked'
      || currentState === 'user_stopped'
    ) {
      pollRunIdRef.current = null;
      return;
    }

    const timer = window.setInterval(() => {
      void loadRun(currentRunId).catch(() => {});
    }, 1500);

    return () => {
      window.clearInterval(timer);
    };
  }, [enabled, runDetail?.run.runId, runDetail?.run.state]);

  return {
    loading,
    saving,
    runs,
    runDetail,
    iterations,
    candidatesResponse,
    createRun,
    loadRuns,
    loadRun,
    stopRun,
    deleteRun,
    selectCandidate,
  };
}
