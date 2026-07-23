import type {
  DesktopAutomationActivityEntry,
  DesktopAutomationActivityFeed,
  DesktopAutomationAgentResolution,
  DesktopAutomationSchedule,
  DesktopAutomationStatus,
  DesktopAutomationSummary,
  DesktopAutomationValidationState,
  DesktopSettings,
} from "@shared/contracts";
import {
  GatewayContractError,
  REMOTE_STATE_FETCH_TIMEOUT_MS,
  hasContractField,
  requestJson,
  requireContractArray,
  requireContractBoolean,
  requireContractField,
  requireContractNonEmptyString,
  requireContractNonNegativeInteger,
  requireContractRecord,
  requireContractString,
} from "./http.ts";

interface AutomationSummaryPayload {
  id?: string;
  label?: string | null;
  prompt?: string | null;
  agentId?: string | null;
  agentResolution?: string;
  effectiveAgentId?: string | null;
  enabled?: boolean;
  workspaceDir?: string | null;
  targetThreadId?: string | null;
  threadId?: string | null;
  nextRun?: string | null;
  lastRunAt?: string | null;
  lastStatus?: string | null;
  unreadHintTimestamp?: string | null;
  threadMode?: string | null;
  schedule?: unknown;
  validationState?: string;
  validationError?: string | null;
}

interface AutomationsPayload {
  automations?: AutomationSummaryPayload[];
}

interface AutomationActivityPayload {
  items?: Array<{
    runId?: string;
    status?: string | null;
    startedAt?: string | null;
    finishedAt?: string | null;
    durationMs?: number | null;
    excerpt?: string | null;
    threadId?: string | null;
  }>;
  threadId?: string | null;
  count?: number;
}

function mapAutomationStatus(
  value: unknown,
  path: string,
): DesktopAutomationStatus {
  switch (value) {
    case "success":
    case "running":
    case "never_run":
      return "success";
    case "failed":
      return "failed";
    default:
      throw new GatewayContractError(path, "must be a current JobRunStatus");
  }
}

function mapAutomationSchedule(
  value: unknown,
  path: string,
): DesktopAutomationSchedule {
  const record = requireContractRecord(value, path);
  if (record.kind === "daily") {
    return {
      kind: "daily",
      time: requireContractNonEmptyString(
        requireContractField(record, "time", path),
        `${path}.time`,
      ),
      weekdays: requireContractArray(
        requireContractField(record, "weekdays", path),
        `${path}.weekdays`,
      ).map((entry, index) =>
        requireContractNonEmptyString(entry, `${path}.weekdays[${index}]`),
      ),
      timezone: requireContractNonEmptyString(
        requireContractField(record, "timezone", path),
        `${path}.timezone`,
      ),
    };
  }

  if (record.kind === "once") {
    return {
      kind: "once",
      at: requireContractNonEmptyString(
        requireContractField(record, "at", path),
        `${path}.at`,
      ),
    };
  }

  if (record.kind === "interval") {
    const hours = requireContractNonNegativeInteger(
      requireContractField(record, "hours", path),
      `${path}.hours`,
    );
    if (hours < 1) {
      throw new GatewayContractError(`${path}.hours`, "must be at least 1");
    }
    return { kind: "interval", hours };
  }

  if (record.kind === "monthly") {
    requireContractNonNegativeInteger(
      requireContractField(record, "day", path),
      `${path}.day`,
    );
    requireContractNonEmptyString(
      requireContractField(record, "time", path),
      `${path}.time`,
    );
    requireContractNonEmptyString(
      requireContractField(record, "timezone", path),
      `${path}.timezone`,
    );
    // The current shared desktop model has no monthly variant. Preserve the
    // existing explicit presentation until that shared contract is expanded.
    return { kind: "interval", hours: 24 };
  }

  throw new GatewayContractError(
    `${path}.kind`,
    "must be daily, interval, monthly, or once",
  );
}

function optionalAutomationString(
  record: Record<string, unknown>,
  field: string,
  path: string,
): string | null {
  if (!hasContractField(record, field)) {
    return null;
  }
  return requireContractString(record[field], `${path}.${field}`);
}

function requiredNullableAutomationId(
  record: Record<string, unknown>,
  field: string,
  path: string,
): string | null {
  const value = requireContractField(record, field, path);
  if (value === null) {
    return null;
  }
  return requireContractNonEmptyString(value, `${path}.${field}`);
}

function mapAutomationAgentResolution(
  value: unknown,
  path: string,
): DesktopAutomationAgentResolution {
  if (value === "resolved" || value === "follow_thread" || value === "target_missing") {
    return value;
  }
  throw new GatewayContractError(
    path,
    "must be resolved, follow_thread, or target_missing",
  );
}

function mapAutomationValidationState(
  value: unknown,
  path: string,
): DesktopAutomationValidationState {
  if (value === "valid" || value === "invalid") {
    return value;
  }
  throw new GatewayContractError(path, "must be valid or invalid");
}

function mapAutomationSummary(value: unknown, index?: number): DesktopAutomationSummary {
  const path = index === undefined
    ? "automation summary"
    : `automation list.automations[${index}]`;
  const record = requireContractRecord(value, path);
  requireContractNonEmptyString(
    requireContractField(record, "threadMode", path),
    `${path}.threadMode`,
  );
  return {
    id: requireContractNonEmptyString(
      requireContractField(record, "id", path),
      `${path}.id`,
    ),
    label: requireContractNonEmptyString(
      requireContractField(record, "label", path),
      `${path}.label`,
    ),
    prompt: requireContractString(
      requireContractField(record, "prompt", path),
      `${path}.prompt`,
    ),
    agentId: requiredNullableAutomationId(record, "agentId", path),
    agentResolution: mapAutomationAgentResolution(
      requireContractField(record, "agentResolution", path),
      `${path}.agentResolution`,
    ),
    effectiveAgentId: requiredNullableAutomationId(record, "effectiveAgentId", path),
    enabled: requireContractBoolean(
      requireContractField(record, "enabled", path),
      `${path}.enabled`,
    ),
    workspacePath: requireContractString(
      requireContractField(record, "workspaceDir", path),
      `${path}.workspaceDir`,
    ),
    targetThreadId: optionalAutomationString(record, "targetThreadId", path) ?? "",
    threadId: optionalAutomationString(record, "threadId", path) ?? "",
    nextRun: requireContractNonEmptyString(
      requireContractField(record, "nextRun", path),
      `${path}.nextRun`,
    ),
    lastRunAt: optionalAutomationString(record, "lastRunAt", path),
    lastStatus: mapAutomationStatus(
      requireContractField(record, "lastStatus", path),
      `${path}.lastStatus`,
    ),
    unreadHintTimestamp: optionalAutomationString(
      record,
      "unreadHintTimestamp",
      path,
    ),
    schedule: mapAutomationSchedule(
      requireContractField(record, "schedule", path),
      `${path}.schedule`,
    ),
    validationState: mapAutomationValidationState(
      requireContractField(record, "validationState", path),
      `${path}.validationState`,
    ),
    validationError: optionalAutomationString(record, "validationError", path),
  };
}

function mapAutomationActivityEntry(
  value: unknown,
  path: string,
): DesktopAutomationActivityEntry {
  const record = requireContractRecord(value, path);
  const optionalString = (field: string): string | null => {
    if (!hasContractField(record, field)) {
      return null;
    }
    return requireContractString(record[field], `${path}.${field}`);
  };
  return {
    runId: requireContractNonEmptyString(
      requireContractField(record, "runId", path),
      `${path}.runId`,
    ),
    status: mapAutomationStatus(
      requireContractField(record, "status", path),
      `${path}.status`,
    ),
    startedAt: requireContractNonEmptyString(
      requireContractField(record, "startedAt", path),
      `${path}.startedAt`,
    ),
    finishedAt: optionalString("finishedAt"),
    durationMs: hasContractField(record, "durationMs")
      ? requireContractNonNegativeInteger(record.durationMs, `${path}.durationMs`)
      : null,
    excerpt: optionalString("excerpt")?.trim() || null,
    threadId: requireContractString(
      requireContractField(record, "threadId", path),
      `${path}.threadId`,
    ),
  };
}

export async function fetchAutomations(
  settings: DesktopSettings,
): Promise<DesktopAutomationSummary[]> {
  const payload = await requestJson<AutomationsPayload>(
    settings,
    "/api/automations",
    "readRetryable",
    {
      signal: AbortSignal.timeout(REMOTE_STATE_FETCH_TIMEOUT_MS),
    },
  );

  const record = requireContractRecord(payload, "automation list");
  return requireContractArray(
    requireContractField(record, "automations", "automation list"),
    "automation list.automations",
  ).map(mapAutomationSummary);
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
    "mutationSingleAttempt",
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
    "mutationSingleAttempt",
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
    "mutationSingleAttempt",
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
    "readRetryable",
    {
      signal: AbortSignal.timeout(8000),
    },
  );

  const record = requireContractRecord(payload, "automation activity");
  const threadId = requireContractField(record, "threadId", "automation activity");
  return {
    automationId,
    threadId: threadId === null
      ? ""
      : requireContractString(threadId, "automation activity.threadId"),
    count: requireContractNonNegativeInteger(
      requireContractField(record, "count", "automation activity"),
      "automation activity.count",
    ),
    items: requireContractArray(
      requireContractField(record, "items", "automation activity"),
      "automation activity.items",
    ).map((entry, index) =>
      mapAutomationActivityEntry(
        entry,
        `automation activity.items[${index}]`,
      ),
    ),
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
  }>(
    settings,
    `/api/automations/${encodeURIComponent(automationId)}/run-now`,
    "mutationSingleAttempt",
    {
      method: "POST",
      signal: AbortSignal.timeout(8000),
    },
  );

  return mapAutomationActivityEntry(payload, "run automation response");
}
