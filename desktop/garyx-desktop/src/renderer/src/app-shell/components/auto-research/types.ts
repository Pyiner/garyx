export type LoopNodeStatus = 'pending' | 'running' | 'completed';

export type LoopNode = {
  key: string;
  label: string;
  status: LoopNodeStatus;
  summary: string;
  body: string;
  threadId: string | null;
  iterationIndex: number | null;
  bullets: string[];
};

export const DEFAULT_MAX_ITERATIONS = '3';
export const DEFAULT_TIME_BUDGET_MINUTES = '15';
