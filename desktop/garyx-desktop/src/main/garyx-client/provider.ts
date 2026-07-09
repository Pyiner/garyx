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
import { asFiniteNumber, asString, parseRecord, requestJson } from "./http.ts";

interface ProviderRecentSessionsPayload {
  sessions?: unknown[];
}

interface ProviderModelOptionPayload {
  id?: unknown;
  label?: unknown;
  description?: unknown;
  recommended?: unknown;
  default_reasoning_effort?: unknown;
  defaultReasoningEffort?: unknown;
  supported_reasoning_efforts?: unknown;
  supportedReasoningEfforts?: unknown;
  service_tiers?: unknown;
  serviceTiers?: unknown;
}

interface ProviderModelsPayload {
  provider_type?: unknown;
  providerType?: unknown;
  supports_model_selection?: unknown;
  supportsModelSelection?: unknown;
  models?: unknown;
  supports_reasoning_effort_selection?: unknown;
  supportsReasoningEffortSelection?: unknown;
  reasoning_efforts?: unknown;
  reasoningEfforts?: unknown;
  supports_service_tier_selection?: unknown;
  supportsServiceTierSelection?: unknown;
  service_tiers?: unknown;
  serviceTiers?: unknown;
  default_model?: unknown;
  defaultModel?: unknown;
  default_reasoning_effort?: unknown;
  defaultReasoningEffort?: unknown;
  source?: unknown;
  error?: unknown;
}

interface UsageWindowPayload {
  used_percent?: unknown;
  usedPercent?: unknown;
  remaining_percent?: unknown;
  remainingPercent?: unknown;
  resets_at?: unknown;
  resetsAt?: unknown;
  reset_after_seconds?: unknown;
  resetAfterSeconds?: unknown;
}

interface ModelUsagePayload {
  id?: unknown;
  name?: unknown;
  remaining_fraction?: unknown;
  remainingFraction?: unknown;
  remaining_percent?: unknown;
  remainingPercent?: unknown;
  used_percent?: unknown;
  usedPercent?: unknown;
  resets_at?: unknown;
  resetsAt?: unknown;
  reset_after_seconds?: unknown;
  resetAfterSeconds?: unknown;
  description?: unknown;
}

interface ProviderUsagePayload {
  id?: unknown;
  name?: unknown;
  available?: unknown;
  stale?: unknown;
  plan?: unknown;
  weekly?: unknown;
  session?: unknown;
  models?: unknown;
  error?: unknown;
}

interface CodingUsagePayload {
  providers?: unknown;
  refreshed_at?: unknown;
  refreshedAt?: unknown;
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
  if (value === "gemini_cli") {
    return "gemini_cli";
  }
  if (value === "gpt" || value === "openai" || value === "garyx_native") {
    return "gpt";
  }
  if (value === "anthropic" || value === "claude_llm" || value === "claude_model") {
    return "anthropic";
  }
  if (value === "google" || value === "gemini_llm" || value === "google_gemini" || value === "gemini_model") {
    return "google";
  }
  return "claude_code";
}

function mapProviderModelOption(
  value: ProviderModelOptionPayload,
): DesktopProviderModelOption | null {
  const id = typeof value.id === "string" ? value.id.trim() : "";
  if (!id) {
    return null;
  }
  const label =
    typeof value.label === "string" && value.label.trim()
      ? value.label.trim()
      : id;
  return {
    id,
    label,
    description:
      typeof value.description === "string" && value.description.trim()
        ? value.description.trim()
        : null,
    recommended: value.recommended === true,
    defaultReasoningEffort:
      typeof value.default_reasoning_effort === "string"
        ? value.default_reasoning_effort
        : typeof value.defaultReasoningEffort === "string"
          ? value.defaultReasoningEffort
          : null,
    supportedReasoningEfforts: mapProviderModelOptionArray(
      value.supported_reasoning_efforts || value.supportedReasoningEfforts,
    ),
    serviceTiers: mapProviderModelOptionArray(
      value.service_tiers || value.serviceTiers,
    ),
  };
}

function mapProviderModelOptionArray(
  value: unknown,
): DesktopProviderModelOption[] {
  const options: DesktopProviderModelOption[] = [];
  if (!Array.isArray(value)) {
    return options;
  }
  for (const item of value) {
    if (item && typeof item === "object") {
      const option = mapProviderModelOption(item as ProviderModelOptionPayload);
      if (option) {
        options.push(option);
      }
    }
  }
  return options;
}

function mapProviderModels(value: ProviderModelsPayload): DesktopProviderModels {
  const models = mapProviderModelOptionArray(value.models);
  const rawReasoningEfforts = value.reasoning_efforts || value.reasoningEfforts;
  const reasoningEfforts = mapProviderModelOptionArray(rawReasoningEfforts);
  const rawServiceTiers = value.service_tiers || value.serviceTiers;
  const serviceTiers = mapProviderModelOptionArray(rawServiceTiers);

  return {
    providerType: normalizeDesktopProviderType(
      value.provider_type || value.providerType,
    ),
    supportsModelSelection:
      value.supports_model_selection === true ||
      value.supportsModelSelection === true,
    models,
    supportsReasoningEffortSelection:
      value.supports_reasoning_effort_selection === true ||
      value.supportsReasoningEffortSelection === true,
    reasoningEfforts,
    supportsServiceTierSelection:
      value.supports_service_tier_selection === true ||
      value.supportsServiceTierSelection === true,
    serviceTiers,
    defaultModel:
      typeof value.default_model === "string"
        ? value.default_model
        : typeof value.defaultModel === "string"
          ? value.defaultModel
          : null,
    defaultReasoningEffort:
      typeof value.default_reasoning_effort === "string"
        ? value.default_reasoning_effort
        : typeof value.defaultReasoningEffort === "string"
          ? value.defaultReasoningEffort
          : null,
    source: typeof value.source === "string" ? value.source : "",
    error: typeof value.error === "string" ? value.error : null,
  };
}

function mapUsageWindow(value: unknown): DesktopUsageWindow | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  const payload = value as UsageWindowPayload;
  const usedPercent = asFiniteNumber(payload.used_percent ?? payload.usedPercent) ?? 0;
  const remainingPercent = asFiniteNumber(payload.remaining_percent ?? payload.remainingPercent)
    ?? Math.max(0, 100 - usedPercent);
  return {
    usedPercent,
    remainingPercent,
    resetsAt: asString(payload.resets_at ?? payload.resetsAt) ?? null,
    resetAfterSeconds: asFiniteNumber(payload.reset_after_seconds ?? payload.resetAfterSeconds) ?? null,
  };
}

function mapModelUsage(value: unknown): DesktopModelUsage | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  const payload = value as ModelUsagePayload;
  const id = asString(payload.id);
  const name = asString(payload.name) || id;
  if (!id || !name) {
    return null;
  }
  const remainingFraction = asFiniteNumber(payload.remaining_fraction ?? payload.remainingFraction) ?? 0;
  const remainingPercent = asFiniteNumber(payload.remaining_percent ?? payload.remainingPercent)
    ?? remainingFraction * 100;
  const usedPercent = asFiniteNumber(payload.used_percent ?? payload.usedPercent)
    ?? Math.max(0, 100 - remainingPercent);
  return {
    id,
    name,
    remainingFraction,
    remainingPercent,
    usedPercent,
    resetsAt: asString(payload.resets_at ?? payload.resetsAt) ?? null,
    resetAfterSeconds: asFiniteNumber(payload.reset_after_seconds ?? payload.resetAfterSeconds) ?? null,
    description: asString(payload.description) ?? null,
  };
}

function mapProviderUsage(value: unknown): DesktopProviderUsage | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  const payload = value as ProviderUsagePayload;
  const id = asString(payload.id);
  const name = asString(payload.name) || id;
  if (!id || !name) {
    return null;
  }
  const models = Array.isArray(payload.models)
    ? payload.models.map(mapModelUsage).filter((model): model is DesktopModelUsage => Boolean(model))
    : [];
  return {
    id,
    name,
    available: payload.available === true,
    stale: payload.stale === true,
    plan: asString(payload.plan) ?? null,
    weekly: mapUsageWindow(payload.weekly),
    session: mapUsageWindow(payload.session),
    models,
    error: asString(payload.error) ?? null,
  };
}

function mapCodingUsage(value: CodingUsagePayload): DesktopCodingUsage {
  const providers = Array.isArray(value.providers)
    ? value.providers
        .map(mapProviderUsage)
        .filter((provider): provider is DesktopProviderUsage => Boolean(provider))
    : [];
  return {
    providers,
    refreshedAt: asString(value.refreshed_at ?? value.refreshedAt) ?? null,
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
): DesktopProviderRecentSession | null {
  const record = parseRecord(value);
  const sessionId =
    asString(record.sessionId) || asString(record.session_id) || "";
  const providerHint =
    asString(record.providerHint) || asString(record.provider_hint) || "";
  if (
    !sessionId ||
    !["claude", "codex", "gemini"].includes(providerHint)
  ) {
    return null;
  }
  return {
    providerType:
      asString(record.providerType) || asString(record.provider_type) || "",
    providerHint: providerHint as DesktopProviderRecentSession["providerHint"],
    sessionId,
    title: asString(record.title) || sessionId,
    workspaceDir:
      asString(record.workspaceDir) || asString(record.workspace_dir) || "",
    updatedAt:
      asString(record.updatedAt) ||
      asString(record.updated_at) ||
      new Date(0).toISOString(),
    path: asString(record.path) || null,
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
  const records = Array.isArray(payload.sessions) ? payload.sessions : [];
  return records
    .map(mapProviderRecentSession)
    .filter((entry): entry is DesktopProviderRecentSession => Boolean(entry));
}
