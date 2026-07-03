import type {
  DesktopSettings,
  DesktopWorkflowChild,
  DesktopWorkflowDefinition,
  DesktopWorkflowEvent,
  DesktopWorkflowPresentation,
  DesktopWorkflowPresentationCounts,
  DesktopWorkflowPresentationPhase,
  DesktopWorkflowPresentationPhaseStatus,
  DesktopWorkflowRun,
  DesktopWorkflowRunDrilldown,
  DesktopWorkflowSourceDocument,
  GetWorkflowDefinitionSourceInput,
  GetWorkflowRunInput,
  StartWorkflowThreadInput,
  StartWorkflowThreadResult,
} from "@shared/contracts";
import { asBoolean, asFiniteNumber, asString, parseRecord, requestJson } from "./http.ts";
import { mapThreadSummary } from "./threads.ts";
import type { ThreadSummaryPayload } from "./threads.ts";

interface WorkflowDefinitionsPayload {
  workflowDefinitions?: unknown[];
  workflow_definitions?: unknown[];
}

interface WorkflowSourcePayload {
  workflowId?: string;
  workflow_id?: string;
  path?: string;
  content?: string;
  mediaType?: string;
  media_type?: string;
  language?: string;
}

interface WorkflowThreadStartPayload {
  dispatch?: unknown;
  workflowRunId?: string;
  workflow_run_id?: string;
  thread?: ThreadSummaryPayload;
  workflowDefinition?: unknown;
  workflow_definition?: unknown;
}

function asRecordOrNull(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function mapWorkflowDefinition(value: unknown): DesktopWorkflowDefinition | null {
  const record = parseRecord(value);
  const workflowId =
    asString(record.workflowId) || asString(record.workflow_id) || "";
  if (!workflowId) {
    return null;
  }
  return {
    workflowId,
    version: asFiniteNumber(record.version) ?? 1,
    name: asString(record.name) || workflowId,
    description: asString(record.description) || "",
    input: asRecordOrNull(record.input),
    defaults: asRecordOrNull(record.defaults),
    packageDir:
      asString(record.packageDir) || asString(record.package_dir) || null,
    createdAt: asString(record.createdAt) || asString(record.created_at) || null,
    updatedAt: asString(record.updatedAt) || asString(record.updated_at) || null,
  };
}

function mapWorkflowSource(value: WorkflowSourcePayload): DesktopWorkflowSourceDocument {
  return {
    workflowId: asString(value.workflowId) || asString(value.workflow_id) || "",
    path: asString(value.path) || "",
    content: asString(value.content) || "",
    mediaType: asString(value.mediaType) || asString(value.media_type) || "text/plain",
    language: asString(value.language) || "text",
  };
}

function mapWorkflowRun(value: unknown): DesktopWorkflowRun {
  const record = parseRecord(value);
  const workflowRunId =
    asString(record.workflowRunId) ||
    asString(record.workflow_run_id) ||
    asString(record.workflowId) ||
    asString(record.workflow_id) ||
    "";
  return {
    workflowRunId,
    threadId: asString(record.threadId) || asString(record.thread_id) || workflowRunId,
    workflowId: workflowRunId,
    taskId: asString(record.taskId) || asString(record.task_id) || null,
    taskThreadId:
      asString(record.taskThreadId) || asString(record.task_thread_id) || null,
    parentThreadId:
      asString(record.parentThreadId) ||
      asString(record.parent_thread_id) ||
      null,
    name: asString(record.name) || null,
    description: asString(record.description) || null,
    status: asString(record.status) || "running",
    currentPhaseIndex:
      asFiniteNumber(record.currentPhaseIndex) ??
      asFiniteNumber(record.current_phase_index) ??
      null,
    meta: asRecordOrNull(record.meta),
    input: record.input ?? null,
    outputText:
      asString(record.outputText) ||
      asString(record.output_text) ||
      // Transitional: gateway binaries predating the outputText contract still
      // emit the run's final text as `summary`. Harmless once the gateway only
      // sends outputText (it stops emitting summary entirely).
      asString(record.summary) ||
      null,
    error: asString(record.error) || null,
    workspaceDir:
      asString(record.workspaceDir) || asString(record.workspace_dir) || null,
    totalChildren:
      asFiniteNumber(record.totalChildren) ??
      asFiniteNumber(record.total_children) ??
      0,
    completedChildren:
      asFiniteNumber(record.completedChildren) ??
      asFiniteNumber(record.completed_children) ??
      0,
    failedChildren:
      asFiniteNumber(record.failedChildren) ??
      asFiniteNumber(record.failed_children) ??
      0,
    totalInputTokens:
      asFiniteNumber(record.totalInputTokens) ??
      asFiniteNumber(record.total_input_tokens) ??
      0,
    totalOutputTokens:
      asFiniteNumber(record.totalOutputTokens) ??
      asFiniteNumber(record.total_output_tokens) ??
      0,
    totalToolCalls:
      asFiniteNumber(record.totalToolCalls) ??
      asFiniteNumber(record.total_tool_calls) ??
      0,
    totalCostUsd:
      asFiniteNumber(record.totalCostUsd) ??
      asFiniteNumber(record.total_cost_usd) ??
      0,
    createdAt: asString(record.createdAt) || asString(record.created_at) || null,
    startedAt: asString(record.startedAt) || asString(record.started_at) || null,
    finishedAt:
      asString(record.finishedAt) || asString(record.finished_at) || null,
    updatedAt: asString(record.updatedAt) || asString(record.updated_at) || null,
  };
}

function mapWorkflowChild(value: unknown): DesktopWorkflowChild | null {
  const record = parseRecord(value);
  const workflowChildRunId =
    asString(record.workflowChildRunId) ||
    asString(record.workflow_child_run_id) ||
    "";
  if (!workflowChildRunId) {
    return null;
  }
  return {
    workflowChildRunId,
    workflowRunId:
      asString(record.workflowRunId) ||
      asString(record.workflow_run_id) ||
      asString(record.workflowId) ||
      asString(record.workflow_id) ||
      null,
    workflowId:
      asString(record.workflowId) || asString(record.workflow_id) || "",
    threadId: asString(record.threadId) || asString(record.thread_id) || null,
    phaseIndex:
      asFiniteNumber(record.phaseIndex) ??
      asFiniteNumber(record.phase_index) ??
      null,
    phaseTitle:
      asString(record.phaseTitle) || asString(record.phase_title) || null,
    label: asString(record.label) || null,
    agentId: asString(record.agentId) || asString(record.agent_id) || null,
    status: asString(record.status) || "running",
    prompt: asString(record.prompt) || null,
    resultMode:
      asString(record.resultMode) || asString(record.result_mode) || null,
    schema: record.schema ?? null,
    resultText:
      asString(record.resultText) || asString(record.result_text) || null,
    result: record.result ?? null,
    resultPreview:
      asString(record.resultPreview) || asString(record.result_preview) || null,
    error: asString(record.error) || null,
    inputTokens:
      asFiniteNumber(record.inputTokens) ??
      asFiniteNumber(record.input_tokens) ??
      0,
    outputTokens:
      asFiniteNumber(record.outputTokens) ??
      asFiniteNumber(record.output_tokens) ??
      0,
    toolCalls:
      asFiniteNumber(record.toolCalls) ??
      asFiniteNumber(record.tool_calls) ??
      0,
    costUsd:
      asFiniteNumber(record.costUsd) ?? asFiniteNumber(record.cost_usd) ?? 0,
    queuedAt: asString(record.queuedAt) || asString(record.queued_at) || null,
    startedAt: asString(record.startedAt) || asString(record.started_at) || null,
    finishedAt:
      asString(record.finishedAt) || asString(record.finished_at) || null,
    updatedAt: asString(record.updatedAt) || asString(record.updated_at) || null,
  };
}

function mapWorkflowEvent(value: unknown): DesktopWorkflowEvent | null {
  const record = parseRecord(value);
  const eventSeq =
    asFiniteNumber(record.eventSeq) ?? asFiniteNumber(record.event_seq);
  const eventType =
    asString(record.eventType) || asString(record.event_type) || "";
  if (eventSeq === undefined || !eventType) {
    return null;
  }
  return {
    eventSeq,
    eventType,
    workflowRunId:
      asString(record.workflowRunId) ||
      asString(record.workflow_run_id) ||
      asString(record.workflowId) ||
      asString(record.workflow_id) ||
      null,
    workflowChildRunId:
      asString(record.workflowChildRunId) ||
      asString(record.workflow_child_run_id) ||
      null,
    threadId: asString(record.threadId) || asString(record.thread_id) || null,
    payload: record.payload ?? null,
    createdAt: asString(record.createdAt) || asString(record.created_at) || null,
  };
}

function mapWorkflowPresentationCounts(
  value: unknown,
): DesktopWorkflowPresentationCounts {
  const record = parseRecord(value);
  return {
    total: asFiniteNumber(record.total) ?? 0,
    completed: asFiniteNumber(record.completed) ?? 0,
    failedChildren: asFiniteNumber(record.failedChildren) ?? 0,
    runningChildren: asFiniteNumber(record.runningChildren) ?? 0,
    queuedChildren: asFiniteNumber(record.queuedChildren) ?? 0,
    skippedChildren: asFiniteNumber(record.skippedChildren) ?? 0,
    totalPhases: asFiniteNumber(record.totalPhases) ?? 0,
    completedPhases: asFiniteNumber(record.completedPhases) ?? 0,
    totalInputTokens: asFiniteNumber(record.totalInputTokens) ?? 0,
    totalOutputTokens: asFiniteNumber(record.totalOutputTokens) ?? 0,
    totalToolCalls: asFiniteNumber(record.totalToolCalls) ?? 0,
    costUsd: asFiniteNumber(record.costUsd) ?? 0,
  };
}

function mapWorkflowPresentationPhase(
  value: unknown,
): DesktopWorkflowPresentationPhase | null {
  const record = parseRecord(value);
  const phaseId = asString(record.phaseId) || '';
  const title = asString(record.title) || '';
  if (!phaseId || !title) {
    return null;
  }
  const counts = parseRecord(record.counts);
  const children = Array.isArray(record.children)
    ? record.children
        .map(mapWorkflowChild)
        .filter((entry): entry is DesktopWorkflowChild => Boolean(entry))
    : [];
  return {
    phaseId,
    index: asFiniteNumber(record.index) ?? null,
    title,
    detail: asString(record.detail) || null,
    status: asString(record.status) || 'queued',
    active: asBoolean(record.active) ?? false,
    counts: {
      completed: asFiniteNumber(counts.completed) ?? 0,
      total: asFiniteNumber(counts.total) ?? children.length,
      failedChildren: asFiniteNumber(counts.failedChildren) ?? 0,
    },
    children,
  };
}

function mapWorkflowPresentationPhaseStatus(
  value: unknown,
): DesktopWorkflowPresentationPhaseStatus | null {
  const record = parseRecord(value);
  const phaseId = asString(record.phaseId) || '';
  const title = asString(record.title) || '';
  if (!phaseId || !title) {
    return null;
  }
  return {
    phaseId,
    index: asFiniteNumber(record.index) ?? null,
    title,
    status: asString(record.status) || 'queued',
    active: asBoolean(record.active) ?? false,
    completedChildren: asFiniteNumber(record.completedChildren) ?? 0,
    totalChildren: asFiniteNumber(record.totalChildren) ?? 0,
    failedChildren: asFiniteNumber(record.failedChildren) ?? 0,
  };
}

function mapWorkflowPresentation(
  value: unknown,
): DesktopWorkflowPresentation | null {
  const record = parseRecord(value);
  const workflowRunId = asString(record.workflowRunId) || '';
  if (!workflowRunId) {
    return null;
  }
  const activePhase = asRecordOrNull(record.activePhase);
  const outcome = parseRecord(record.outcome);
  const eventsSeed = parseRecord(record.eventsSeed);
  const phases = Array.isArray(record.phases)
    ? record.phases
        .map(mapWorkflowPresentationPhase)
        .filter((entry): entry is DesktopWorkflowPresentationPhase =>
          Boolean(entry),
        )
    : [];
  const phaseStatus = Array.isArray(record.phaseStatus)
    ? record.phaseStatus
        .map(mapWorkflowPresentationPhaseStatus)
        .filter((entry): entry is DesktopWorkflowPresentationPhaseStatus =>
          Boolean(entry),
        )
    : [];
  const childCards = Array.isArray(record.childCards)
    ? record.childCards
        .map(mapWorkflowChild)
        .filter((entry): entry is DesktopWorkflowChild => Boolean(entry))
    : [];
  return {
    version: asFiniteNumber(record.version) ?? 1,
    workflowRunId,
    threadId: asString(record.threadId) || workflowRunId,
    workflowDefinitionId: asString(record.workflowDefinitionId) || null,
    taskId: asString(record.taskId) || null,
    taskThreadId: asString(record.taskThreadId) || null,
    title: asString(record.title) || 'Workflow run',
    description: asString(record.description) || null,
    status: asString(record.status) || 'running',
    counts: mapWorkflowPresentationCounts(record.counts),
    activePhase: activePhase
      ? {
          phaseId: asString(activePhase.phaseId) || '',
          index: asFiniteNumber(activePhase.index) ?? null,
          title: asString(activePhase.title) || '',
          detail: asString(activePhase.detail) || null,
        }
      : null,
    phaseStatus,
    phases,
    childCards,
    outcome: {
      kind: asString(outcome.kind) || 'running',
      status: asString(outcome.status) || 'running',
      hasOutputText: asBoolean(outcome.hasOutputText) ?? false,
      hasResult: asBoolean(outcome.hasResult) ?? false,
      error: asString(outcome.error) || null,
    },
    outputText: asString(record.outputText) || null,
    result: record.result ?? null,
    error: asString(record.error) || null,
    terminalComplete: asBoolean(record.terminalComplete) ?? false,
    stale: asBoolean(record.stale) ?? false,
    staleReason: asString(record.staleReason) || null,
    snapshotVersion: asFiniteNumber(record.snapshotVersion) ?? 0,
    latestEventSeq: asFiniteNumber(record.latestEventSeq) ?? 0,
    eventsSeed: {
      count: asFiniteNumber(eventsSeed.count) ?? 0,
      latestSeedEventSeq: asFiniteNumber(eventsSeed.latestSeedEventSeq) ?? 0,
      truncated: asBoolean(eventsSeed.truncated) ?? false,
    },
  };
}

function mapWorkflowRunDrilldown(value: unknown): DesktopWorkflowRunDrilldown {
  const record = parseRecord(value);
  const children = Array.isArray(record.children)
    ? record.children
        .map(mapWorkflowChild)
        .filter((entry): entry is DesktopWorkflowChild => Boolean(entry))
    : [];
  const events = Array.isArray(record.events)
    ? record.events
        .map(mapWorkflowEvent)
        .filter((entry): entry is DesktopWorkflowEvent => Boolean(entry))
    : [];
  return {
    workflow: mapWorkflowRun(record.workflow),
    children,
    events,
    presentation: mapWorkflowPresentation(record.presentation),
  };
}

export async function listWorkflowDefinitions(
  settings: DesktopSettings,
): Promise<DesktopWorkflowDefinition[]> {
  const payload = await requestJson<WorkflowDefinitionsPayload>(
    settings,
    "/api/workflow-definitions",
    {
      signal: AbortSignal.timeout(8000),
    },
  );
  const records = Array.isArray(payload.workflowDefinitions)
    ? payload.workflowDefinitions
    : Array.isArray(payload.workflow_definitions)
      ? payload.workflow_definitions
      : [];
  return records
    .map(mapWorkflowDefinition)
    .filter((entry): entry is DesktopWorkflowDefinition => Boolean(entry));
}

export async function getWorkflowDefinitionSource(
  settings: DesktopSettings,
  input: GetWorkflowDefinitionSourceInput,
): Promise<DesktopWorkflowSourceDocument> {
  const workflowId = input.workflowId?.trim() || "";
  if (!workflowId) {
    throw new Error("workflowId is required");
  }
  const payload = await requestJson<WorkflowSourcePayload>(
    settings,
    `/api/workflow-definitions/${encodeURIComponent(workflowId)}/source`,
    {
      signal: AbortSignal.timeout(8000),
    },
  );
  return mapWorkflowSource(payload);
}

export async function getWorkflowRun(
  settings: DesktopSettings,
  input: GetWorkflowRunInput,
): Promise<DesktopWorkflowRunDrilldown> {
  const workflowRunId = input.workflowRunId?.trim() || "";
  if (!workflowRunId) {
    throw new Error("workflowRunId is required");
  }
  const payload = await requestJson<unknown>(
    settings,
    `/api/workflows/${encodeURIComponent(workflowRunId)}`,
    {
      signal: AbortSignal.timeout(8000),
    },
  );
  return mapWorkflowRunDrilldown(payload);
}

export async function startWorkflowThread(
  settings: DesktopSettings,
  input: StartWorkflowThreadInput,
): Promise<Omit<StartWorkflowThreadResult, "state">> {
  const workflowId = input.workflowId?.trim() || "";
  if (!workflowId) {
    throw new Error("workflowId is required");
  }
  const payload = await requestJson<WorkflowThreadStartPayload>(
    settings,
    `/api/workflow-definitions/${encodeURIComponent(workflowId)}/runs`,
    {
      method: "POST",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        input: input.input ?? null,
        workspaceDir: input.workspacePath || undefined,
        name: input.name || undefined,
        description: input.description || undefined,
        createdBy: "desktop",
      }),
    },
  );
  const thread = payload.thread ? mapThreadSummary(payload.thread) : null;
  if (!thread?.id) {
    throw new Error("Gateway did not return a workflow thread.");
  }
  return {
    thread,
    workflowRunId:
      asString(payload.workflowRunId) ||
      asString(payload.workflow_run_id) ||
      thread.id,
    dispatch: payload.dispatch,
    workflowDefinition: mapWorkflowDefinition(
      payload.workflowDefinition || payload.workflow_definition,
    ),
  };
}
