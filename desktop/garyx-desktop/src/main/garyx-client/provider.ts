import type {
  DesktopApiProviderType,
  DesktopCodingUsage,
  DesktopModelUsage,
  DesktopProviderModelOption,
  DesktopProviderModels,
  DesktopProviderRecentSession,
  DesktopProviderUsage,
  DesktopSettings,
  DesktopUsageWindow,
  ListProviderRecentSessionsInput,
} from "@shared/contracts";
import {
  GatewayContractError,
  hasContractField,
  requestJson,
  requireContractArray,
  requireContractBoolean,
  requireContractField,
  requireContractFiniteNumber,
  requireContractInteger,
  requireContractNonEmptyString,
  requireContractRecord,
  requireContractString,
} from "./http.ts";

interface ProviderRecentSessionsPayload {
  sessions?: unknown[];
}

interface ProviderModelsPayload {
  provider_type?: unknown;
  supports_model_selection?: unknown;
  models?: unknown;
  supports_reasoning_effort_selection?: unknown;
  reasoning_efforts?: unknown;
  supports_service_tier_selection?: unknown;
  service_tiers?: unknown;
  default_model?: unknown;
  default_reasoning_effort?: unknown;
  source?: unknown;
  error?: unknown;
}

interface CodingUsagePayload {
  providers?: unknown;
  refreshed_at?: unknown;
}

export function normalizeDesktopProviderType(value: unknown): DesktopApiProviderType {
  if (value === "codex_app_server") {
    return "codex_app_server";
  }
  if (value === "antigravity" || value === "agy" || value === "antigravity_cli") {
    return "antigravity";
  }
  if (value === "traex" || value === "trae" || value === "trae_cli" || value === "traecli") {
    return "traex";
  }
  return "claude_code";
}

function mapProviderModelOption(
  value: unknown,
  path: string,
): DesktopProviderModelOption {
  const record = requireContractRecord(value, path);
  const optionalString = (field: string): string | null => {
    if (!hasContractField(record, field)) {
      return null;
    }
    return requireContractString(record[field], `${path}.${field}`);
  };
  const optionalBoolean = (field: string): boolean => {
    if (!hasContractField(record, field)) {
      return false;
    }
    return requireContractBoolean(record[field], `${path}.${field}`);
  };
  return {
    id: requireContractNonEmptyString(
      requireContractField(record, "id", path),
      `${path}.id`,
    ),
    label: requireContractNonEmptyString(
      requireContractField(record, "label", path),
      `${path}.label`,
    ),
    description: optionalString("description"),
    recommended: optionalBoolean("recommended"),
    defaultReasoningEffort: optionalString("default_reasoning_effort"),
    supportedReasoningEfforts: mapProviderModelOptionArray(
      record.supported_reasoning_efforts,
      `${path}.supported_reasoning_efforts`,
      true,
    ),
    serviceTiers: mapProviderModelOptionArray(
      record.service_tiers,
      `${path}.service_tiers`,
      true,
    ),
  };
}

function mapProviderModelOptionArray(
  value: unknown,
  path: string,
  mayBeOmitted = false,
): DesktopProviderModelOption[] {
  if (value === undefined && mayBeOmitted) {
    return [];
  }
  return requireContractArray(value, path).map((item, index) =>
    mapProviderModelOption(item, `${path}[${index}]`),
  );
}

function mapReasoningEffortOption(
  value: unknown,
  path: string,
): DesktopProviderModelOption {
  const record = requireContractRecord(value, path);
  return {
    id: requireContractNonEmptyString(
      requireContractField(record, "id", path),
      `${path}.id`,
    ),
    label: requireContractNonEmptyString(
      requireContractField(record, "label", path),
      `${path}.label`,
    ),
    description: hasContractField(record, "description")
      ? requireContractString(record.description, `${path}.description`)
      : null,
    recommended: hasContractField(record, "recommended")
      ? requireContractBoolean(record.recommended, `${path}.recommended`)
      : false,
    defaultReasoningEffort: null,
    supportedReasoningEfforts: [],
    serviceTiers: [],
  };
}

function mapReasoningEffortOptionArray(
  value: unknown,
  path: string,
): DesktopProviderModelOption[] {
  if (value === undefined) {
    return [];
  }
  return requireContractArray(value, path).map((item, index) =>
    mapReasoningEffortOption(item, `${path}[${index}]`),
  );
}

function mapWireProviderType(value: unknown, path: string): DesktopApiProviderType {
  switch (value) {
    case "claude_code":
    case "codex_app_server":
    case "traex":
    case "antigravity":
      return value;
    default:
      throw new GatewayContractError(path, "must be a current provider type");
  }
}

function mapProviderModels(value: unknown): DesktopProviderModels {
  const path = "provider models";
  const record = requireContractRecord(value, path);
  return {
    providerType: mapWireProviderType(
      requireContractField(record, "provider_type", path),
      `${path}.provider_type`,
    ),
    supportsModelSelection: requireContractBoolean(
      requireContractField(record, "supports_model_selection", path),
      `${path}.supports_model_selection`,
    ),
    models: mapProviderModelOptionArray(
      requireContractField(record, "models", path),
      `${path}.models`,
    ),
    supportsReasoningEffortSelection: hasContractField(
      record,
      "supports_reasoning_effort_selection",
    )
      ? requireContractBoolean(
          record.supports_reasoning_effort_selection,
          `${path}.supports_reasoning_effort_selection`,
        )
      : false,
    reasoningEfforts: mapReasoningEffortOptionArray(
      record.reasoning_efforts,
      `${path}.reasoning_efforts`,
    ),
    supportsServiceTierSelection: hasContractField(
      record,
      "supports_service_tier_selection",
    )
      ? requireContractBoolean(
          record.supports_service_tier_selection,
          `${path}.supports_service_tier_selection`,
        )
      : false,
    serviceTiers: mapProviderModelOptionArray(
      record.service_tiers,
      `${path}.service_tiers`,
      true,
    ),
    defaultModel: hasContractField(record, "default_model")
      ? requireContractString(record.default_model, `${path}.default_model`)
      : null,
    defaultReasoningEffort: hasContractField(
      record,
      "default_reasoning_effort",
    )
      ? requireContractString(
          record.default_reasoning_effort,
          `${path}.default_reasoning_effort`,
        )
      : null,
    source: requireContractNonEmptyString(
      requireContractField(record, "source", path),
      `${path}.source`,
    ),
    error: hasContractField(record, "error")
      ? requireContractString(record.error, `${path}.error`)
      : null,
  };
}

function mapUsageWindow(value: unknown, path: string): DesktopUsageWindow {
  const record = requireContractRecord(value, path);
  return {
    usedPercent: requireContractFiniteNumber(
      requireContractField(record, "used_percent", path),
      `${path}.used_percent`,
    ),
    remainingPercent: requireContractFiniteNumber(
      requireContractField(record, "remaining_percent", path),
      `${path}.remaining_percent`,
    ),
    resetsAt: hasContractField(record, "resets_at")
      ? requireContractString(record.resets_at, `${path}.resets_at`)
      : null,
    resetAfterSeconds: hasContractField(record, "reset_after_seconds")
      ? requireContractInteger(
          record.reset_after_seconds,
          `${path}.reset_after_seconds`,
        )
      : null,
  };
}

function mapModelUsage(value: unknown, path: string): DesktopModelUsage {
  const record = requireContractRecord(value, path);
  return {
    id: requireContractNonEmptyString(
      requireContractField(record, "id", path),
      `${path}.id`,
    ),
    name: requireContractNonEmptyString(
      requireContractField(record, "name", path),
      `${path}.name`,
    ),
    remainingFraction: requireContractFiniteNumber(
      requireContractField(record, "remaining_fraction", path),
      `${path}.remaining_fraction`,
    ),
    remainingPercent: requireContractFiniteNumber(
      requireContractField(record, "remaining_percent", path),
      `${path}.remaining_percent`,
    ),
    usedPercent: requireContractFiniteNumber(
      requireContractField(record, "used_percent", path),
      `${path}.used_percent`,
    ),
    resetsAt: hasContractField(record, "resets_at")
      ? requireContractString(record.resets_at, `${path}.resets_at`)
      : null,
    resetAfterSeconds: hasContractField(record, "reset_after_seconds")
      ? requireContractInteger(
          record.reset_after_seconds,
          `${path}.reset_after_seconds`,
        )
      : null,
    description: hasContractField(record, "description")
      ? requireContractString(record.description, `${path}.description`)
      : null,
  };
}

function mapProviderUsage(value: unknown, path: string): DesktopProviderUsage {
  const record = requireContractRecord(value, path);
  return {
    id: requireContractNonEmptyString(
      requireContractField(record, "id", path),
      `${path}.id`,
    ),
    name: requireContractNonEmptyString(
      requireContractField(record, "name", path),
      `${path}.name`,
    ),
    available: requireContractBoolean(
      requireContractField(record, "available", path),
      `${path}.available`,
    ),
    stale: hasContractField(record, "stale")
      ? requireContractBoolean(record.stale, `${path}.stale`)
      : false,
    plan: hasContractField(record, "plan")
      ? requireContractString(record.plan, `${path}.plan`)
      : null,
    weekly: hasContractField(record, "weekly")
      ? mapUsageWindow(record.weekly, `${path}.weekly`)
      : null,
    session: hasContractField(record, "session")
      ? mapUsageWindow(record.session, `${path}.session`)
      : null,
    models: hasContractField(record, "models")
      ? requireContractArray(record.models, `${path}.models`).map((model, index) =>
          mapModelUsage(model, `${path}.models[${index}]`),
        )
      : [],
    error: hasContractField(record, "error")
      ? requireContractString(record.error, `${path}.error`)
      : null,
  };
}

function mapCodingUsage(value: unknown): DesktopCodingUsage {
  const path = "coding usage";
  const record = requireContractRecord(value, path);
  return {
    providers: requireContractArray(
      requireContractField(record, "providers", path),
      `${path}.providers`,
    ).map((provider, index) =>
      mapProviderUsage(provider, `${path}.providers[${index}]`),
    ),
    refreshedAt: requireContractNonEmptyString(
      requireContractField(record, "refreshed_at", path),
      `${path}.refreshed_at`,
    ),
  };
}

export async function listProviderModels(
  settings: DesktopSettings,
  providerType: DesktopApiProviderType,
): Promise<DesktopProviderModels> {
  const payload = await requestJson<ProviderModelsPayload>(
    settings,
    `/api/provider-models/${encodeURIComponent(providerType)}`,
    {
      signal: AbortSignal.timeout(30000),
    },
  );

  return mapProviderModels(payload);
}

export async function getCodingUsage(
  settings: DesktopSettings,
): Promise<DesktopCodingUsage> {
  const payload = await requestJson<CodingUsagePayload>(
    settings,
    "/api/usage/coding",
    {
      signal: AbortSignal.timeout(15000),
    },
  );

  return mapCodingUsage(payload);
}

function mapProviderRecentSession(
  value: unknown,
  index: number,
): DesktopProviderRecentSession {
  const path = `recent provider sessions.sessions[${index}]`;
  const record = requireContractRecord(value, path);
  const providerHint = requireContractNonEmptyString(
    requireContractField(record, "providerHint", path),
    `${path}.providerHint`,
  );
  if (providerHint !== "claude" && providerHint !== "codex") {
    throw new GatewayContractError(
      `${path}.providerHint`,
      "must be claude or codex",
    );
  }
  return {
    providerType: mapWireProviderType(
      requireContractField(record, "providerType", path),
      `${path}.providerType`,
    ),
    providerHint,
    sessionId: requireContractNonEmptyString(
      requireContractField(record, "sessionId", path),
      `${path}.sessionId`,
    ),
    title: requireContractString(
      requireContractField(record, "title", path),
      `${path}.title`,
    ),
    workspaceDir: requireContractString(
      requireContractField(record, "workspaceDir", path),
      `${path}.workspaceDir`,
    ),
    updatedAt: requireContractNonEmptyString(
      requireContractField(record, "updatedAt", path),
      `${path}.updatedAt`,
    ),
    path: requireContractString(
      requireContractField(record, "path", path),
      `${path}.path`,
    ),
  };
}

export async function listProviderRecentSessions(
  settings: DesktopSettings,
  input?: ListProviderRecentSessionsInput,
): Promise<DesktopProviderRecentSession[]> {
  const query = new URLSearchParams();
  if (input?.provider) {
    query.set("provider", input.provider);
  }
  query.set("limit", String(Math.max(1, Math.min(50, input?.limit || 10))));
  const payload = await requestJson<ProviderRecentSessionsPayload>(
    settings,
    `/api/provider-sessions/recent?${query.toString()}`,
    {
      signal: AbortSignal.timeout(8000),
    },
  );
  const record = requireContractRecord(payload, "recent provider sessions");
  return requireContractArray(
    requireContractField(record, "sessions", "recent provider sessions"),
    "recent provider sessions.sessions",
  ).map(mapProviderRecentSession);
}
