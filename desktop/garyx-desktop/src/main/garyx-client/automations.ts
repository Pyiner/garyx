import type {
  DesktopAutomationActivityEntry,
  DesktopAutomationActivityFeed,
  DesktopAutomationSchedule,
  DesktopAutomationStatus,
  DesktopAutomationSummary,
  DesktopSettings,
} from "@shared/contracts";
import { REMOTE_STATE_FETCH_TIMEOUT_MS, asString, parseRecord, requestJson } from "./http.ts";

interface AutomationSummaryPayload {
  id?: string;
  label?: string | null;
  prompt?: string | null;
  agent_id?: string | null;
  agentId?: string | null;
  enabled?: boolean;
  workspace_dir?: string | null;
  workspaceDir?: string | null;
  target_thread_id?: string | null;
  targetThreadId?: string | null;
  thread_id?: string | null;
  threadId?: string | null;
  next_run?: string | null;
  nextRun?: string | null;
  last_run_at?: string | null;
  lastRunAt?: string | null;
  last_status?: string | null;
  lastStatus?: string | null;
  unread_hint_timestamp?: string | null;
  unreadHintTimestamp?: string | null;
  schedule?: unknown;
}

interface AutomationsPayload {
  automations?: AutomationSummaryPayload[];
}

interface AutomationActivityPayload {
  items?: Array<{
    run_id?: string;
    status?: string | null;
    started_at?: string | null;
    finished_at?: string | null;
    duration_ms?: number | null;
    excerpt?: string | null;
    thread_id?: string | null;
  }>;
  threadId?: string;
  count?: number;
}

function normalizeAutomationStatus(value: unknown): DesktopAutomationStatus {
  return value === "failed" || value === "skipped" ? value : "success";
}

function mapAutomationSchedule(value: unknown): DesktopAutomationSchedule {
  const record = parseRecord(value);
  if (record.kind === "daily") {
    return {
      kind: "daily",
      time: asString(record.time) || "09:00",
      weekdays: Array.isArray(record.weekdays)
        ? record.weekdays.filter(
            (entry): entry is string => typeof entry === "string",
          )
        : [],
      timezone:
        asString(record.timezone) ||
        Intl.DateTimeFormat().resolvedOptions().timeZone ||
        "UTC",
    };
  }

  if (record.kind === "once") {
    return {
      kind: "once",
      at: asString(record.at) || "",
    };
  }

  const hoursValue =
    typeof record.hours === "number" && Number.isFinite(record.hours)
      ? Math.max(1, Math.round(record.hours))
      : 24;
  return {
    kind: "interval",
    hours: hoursValue,
  };
}

function mapAutomationSummary(
  value: AutomationSummaryPayload,
): DesktopAutomationSummary {
  const agentId = value.agentId ?? value.agent_id;
  return {
    id: value.id || "",
    label:
      typeof value.label === "string" && value.label.trim()
        ? value.label.trim()
        : value.id || "",
    prompt: typeof value.prompt === "string" ? value.prompt : "",
    agentId:
      typeof agentId === "string" && agentId.trim() ? agentId.trim() : "claude",
    enabled: value.enabled !== false,
    workspacePath: value.workspaceDir || value.workspace_dir || "",
    targetThreadId: value.targetThreadId || value.target_thread_id || "",
    threadId: value.threadId || value.thread_id || "",
    nextRun: value.nextRun || value.next_run || new Date(0).toISOString(),
    lastRunAt: value.lastRunAt ?? value.last_run_at ?? null,
    lastStatus: normalizeAutomationStatus(
      value.lastStatus ?? value.last_status,
    ),
    unreadHintTimestamp:
      value.unreadHintTimestamp ?? value.unread_hint_timestamp ?? null,
    schedule: mapAutomationSchedule(value.schedule),
  };
}

function mapAutomationActivityEntry(
  value: NonNullable<AutomationActivityPayload["items"]>[number],
): DesktopAutomationActivityEntry {
  return {
    runId: value.run_id || "",
    status: normalizeAutomationStatus(value.status),
    startedAt: value.started_at || new Date(0).toISOString(),
    finishedAt: value.finished_at ?? null,
    durationMs:
      typeof value.duration_ms === "number" &&
      Number.isFinite(value.duration_ms)
        ? value.duration_ms
        : null,
    excerpt:
      typeof value.excerpt === "string" && value.excerpt.trim()
        ? value.excerpt.trim()
        : null,
    threadId: value.thread_id || "",
  };
}

export async function fetchAutomations(
  settings: DesktopSettings,
): Promise<DesktopAutomationSummary[]> {
  const payload = await requestJson<AutomationsPayload>(
    settings,
    "/api/automations",
    {
      signal: AbortSignal.timeout(REMOTE_STATE_FETCH_TIMEOUT_MS),
    },
  );

  return Array.isArray(payload.automations)
    ? payload.automations.map(mapAutomationSummary)
    : [];
}

export async function createRemoteAutomation(
  settings: DesktopSettings,
  input: {
    label: string;
    prompt: string;
    agentId?: string;
    workspacePath?: string;
    targetThreadId?: string | null;
    schedule: DesktopAutomationSchedule;
  },
): Promise<DesktopAutomationSummary> {
  const payload = await requestJson<AutomationSummaryPayload>(
    settings,
    "/api/automations",
    {
      method: "POST",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        label: input.label,
        prompt: input.prompt,
        agentId: input.agentId || undefined,
        workspaceDir: input.workspacePath || undefined,
        targetThreadId: input.targetThreadId || undefined,
        schedule: input.schedule,
      }),
    },
  );
  return mapAutomationSummary(payload);
}

export async function updateRemoteAutomation(
  settings: DesktopSettings,
  automationId: string,
  input: {
    label?: string;
    prompt?: string;
    agentId?: string;
    workspacePath?: string;
    targetThreadId?: string | null;
    schedule?: DesktopAutomationSchedule;
    enabled?: boolean;
  },
): Promise<DesktopAutomationSummary> {
  const payload = await requestJson<AutomationSummaryPayload>(
    settings,
    `/api/automations/${encodeURIComponent(automationId)}`,
    {
      method: "PATCH",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        label: input.label,
        prompt: input.prompt,
        agentId: input.agentId,
        workspaceDir: input.workspacePath,
        targetThreadId: input.targetThreadId,
        schedule: input.schedule,
        enabled: input.enabled,
      }),
    },
  );
  return mapAutomationSummary(payload);
}

export async function deleteRemoteAutomation(
  settings: DesktopSettings,
  automationId: string,
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/automations/${encodeURIComponent(automationId)}`,
    {
      method: "DELETE",
      signal: AbortSignal.timeout(8000),
    },
  );
}

export async function fetchAutomationActivity(
  settings: DesktopSettings,
  automationId: string,
): Promise<DesktopAutomationActivityFeed> {
  const payload = await requestJson<AutomationActivityPayload>(
    settings,
    `/api/automations/${encodeURIComponent(automationId)}/activity`,
    {
      signal: AbortSignal.timeout(8000),
    },
  );

  return {
    automationId,
    threadId: payload.threadId || "",
    count:
      typeof payload.count === "number" && Number.isFinite(payload.count)
        ? payload.count
        : 0,
    items: Array.isArray(payload.items)
      ? payload.items.map(mapAutomationActivityEntry)
      : [],
  };
}

export async function runRemoteAutomationNow(
  settings: DesktopSettings,
  automationId: string,
): Promise<DesktopAutomationActivityEntry> {
  const payload = await requestJson<{
    runId?: string;
    status?: string | null;
    startedAt?: string | null;
    finishedAt?: string | null;
    durationMs?: number | null;
    excerpt?: string | null;
    threadId?: string | null;
  }>(settings, `/api/automations/${encodeURIComponent(automationId)}/run-now`, {
    method: "POST",
    signal: AbortSignal.timeout(8000),
  });

  return {
    runId: payload.runId || "",
    status: normalizeAutomationStatus(payload.status),
    startedAt: payload.startedAt || new Date(0).toISOString(),
    finishedAt: payload.finishedAt ?? null,
    durationMs:
      typeof payload.durationMs === "number" &&
      Number.isFinite(payload.durationMs)
        ? payload.durationMs
        : null,
    excerpt:
      typeof payload.excerpt === "string" && payload.excerpt.trim()
        ? payload.excerpt.trim()
        : null,
    threadId: payload.threadId || "",
  };
}
