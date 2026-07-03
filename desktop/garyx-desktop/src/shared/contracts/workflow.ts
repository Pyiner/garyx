import type { DesktopState } from "./state.ts";
import type { DesktopThreadSummary } from "./thread.ts";
import type { DesktopWorkspaceMode } from "./workspace.ts";

/**
 * Reusable workflow definition discovered by the gateway under
 * `~/.garyx/workflows`. Mirrors `workflow_definition_package_json`. The list is
 * empty until a package is installed with `garyx workflow definition upsert`.
 */
export interface DesktopWorkflowDefinition {
  workflowId: string;
  version: number;
  name: string;
  description: string;
  /** Text-input metadata for product surfaces. Not a generated form schema. */
  input?: Record<string, unknown> | null;
  defaults?: Record<string, unknown> | null;
  packageDir?: string | null;
  createdAt?: string | null;
  updatedAt?: string | null;
}

export interface DesktopWorkflowSourceDocument {
  workflowId: string;
  path: string;
  content: string;
  mediaType: string;
  language: string;
}

export type DesktopWorkflowRunStatus =
  | "running"
  | "succeeded"
  | "failed"
  | "cancelled"
  | string;

/** Mirrors `workflow_run_json`. Token/cost counters are roll-ups across children. */
export interface DesktopWorkflowRun {
  workflowRunId: string;
  threadId: string;
  /** Legacy run-id alias kept while gateway payloads migrate. */
  workflowId: string;
  taskId?: string | null;
  taskThreadId?: string | null;
  parentThreadId?: string | null;
  name?: string | null;
  description?: string | null;
  status: DesktopWorkflowRunStatus;
  currentPhaseIndex?: number | null;
  meta?: Record<string, unknown> | null;
  input?: unknown | null;
  outputText?: string | null;
  error?: string | null;
  workspaceDir?: string | null;
  totalChildren: number;
  completedChildren: number;
  failedChildren: number;
  totalInputTokens: number;
  totalOutputTokens: number;
  totalToolCalls: number;
  totalCostUsd: number;
  createdAt?: string | null;
  startedAt?: string | null;
  finishedAt?: string | null;
  updatedAt?: string | null;
}

/** Mirrors `workflow_child_json` — one dispatched child agent run. */
export interface DesktopWorkflowChild {
  workflowChildRunId: string;
  workflowRunId?: string | null;
  workflowId: string;
  threadId?: string | null;
  phaseIndex?: number | null;
  phaseTitle?: string | null;
  label?: string | null;
  agentId?: string | null;
  status: DesktopWorkflowRunStatus;
  prompt?: string | null;
  resultMode?: string | null;
  schema?: unknown | null;
  resultText?: string | null;
  result?: unknown | null;
  resultPreview?: string | null;
  error?: string | null;
  inputTokens: number;
  outputTokens: number;
  toolCalls: number;
  costUsd: number;
  queuedAt?: string | null;
  startedAt?: string | null;
  finishedAt?: string | null;
  updatedAt?: string | null;
}

/** Mirrors `workflow_event_json` — one durable run event. */
export interface DesktopWorkflowEvent {
  eventSeq: number;
  eventType: string;
  workflowRunId?: string | null;
  workflowChildRunId?: string | null;
  threadId?: string | null;
  payload?: unknown;
  createdAt?: string | null;
}

export interface DesktopWorkflowPresentationCounts {
  total: number;
  completed: number;
  failedChildren: number;
  runningChildren: number;
  queuedChildren: number;
  skippedChildren: number;
  totalPhases: number;
  completedPhases: number;
  totalInputTokens: number;
  totalOutputTokens: number;
  totalToolCalls: number;
  costUsd: number;
}

export interface DesktopWorkflowPresentationPhase {
  phaseId: string;
  index?: number | null;
  title: string;
  detail?: string | null;
  status: DesktopWorkflowRunStatus;
  active: boolean;
  counts: {
    completed: number;
    total: number;
    failedChildren: number;
  };
  children: DesktopWorkflowChild[];
}

export interface DesktopWorkflowPresentationPhaseStatus {
  phaseId: string;
  index?: number | null;
  title: string;
  status: DesktopWorkflowRunStatus;
  active: boolean;
  completedChildren: number;
  totalChildren: number;
  failedChildren: number;
}

export interface DesktopWorkflowPresentationOutcome {
  kind: string;
  status: DesktopWorkflowRunStatus;
  hasOutputText: boolean;
  hasResult: boolean;
  error?: string | null;
}

export interface DesktopWorkflowPresentation {
  version: number;
  workflowRunId: string;
  threadId: string;
  workflowDefinitionId?: string | null;
  taskId?: string | null;
  taskThreadId?: string | null;
  title: string;
  description?: string | null;
  status: DesktopWorkflowRunStatus;
  counts: DesktopWorkflowPresentationCounts;
  activePhase?: {
    phaseId: string;
    index?: number | null;
    title: string;
    detail?: string | null;
  } | null;
  phaseStatus: DesktopWorkflowPresentationPhaseStatus[];
  phases: DesktopWorkflowPresentationPhase[];
  childCards: DesktopWorkflowChild[];
  outcome: DesktopWorkflowPresentationOutcome;
  outputText?: string | null;
  result?: unknown | null;
  error?: string | null;
  terminalComplete: boolean;
  stale: boolean;
  staleReason?: string | null;
  snapshotVersion: number;
  latestEventSeq: number;
  eventsSeed: {
    count: number;
    latestSeedEventSeq: number;
    truncated: boolean;
  };
}

export interface DesktopWorkflowRunDrilldown {
  workflow: DesktopWorkflowRun;
  children: DesktopWorkflowChild[];
  events: DesktopWorkflowEvent[];
  presentation?: DesktopWorkflowPresentation | null;
}

export interface GetWorkflowRunInput {
  workflowRunId: string;
}

export interface GetWorkflowDefinitionSourceInput {
  workflowId: string;
}

export interface StartWorkflowThreadInput {
  workflowId: string;
  input?: unknown;
  workspacePath?: string | null;
  workspaceMode?: DesktopWorkspaceMode;
  name?: string | null;
  description?: string | null;
}

export interface StartWorkflowThreadResult {
  state: DesktopState;
  thread: DesktopThreadSummary;
  workflowRunId: string;
  dispatch?: unknown;
  workflowDefinition?: DesktopWorkflowDefinition | null;
}
