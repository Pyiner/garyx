import type {
  DesktopApiProviderType,
  DesktopClaudeAuthSession,
  DesktopClaudeAuthStatus,
  DesktopClaudeCodeAccount,
  DesktopClaudeCodeAccountSelection,
  DesktopClaudeCodeAccounts,
  DesktopCodingUsage,
  DesktopModelUsage,
  DesktopProviderModelOption,
  DesktopProviderModels,
  DesktopProviderRecentSession,
  DesktopProviderUsage,
  DesktopQuotaRecoveryRetryResult,
  DesktopScopedUsageLimit,
  DesktopSettings,
  DesktopUsageWindow,
  ListProviderRecentSessionsInput,
  StartDesktopClaudeAuthInput,
} from "@shared/contracts";
import {
  GatewayContractError,
  GatewayRequestError,
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

interface ClaudeCodeAccountsPayload {
  active_account_id?: unknown;
  accounts?: unknown;
  refreshed_at?: unknown;
}

interface ClaudeAuthPayload {
  login_id?: unknown;
  account_id?: unknown;
  status?: unknown;
  url?: unknown;
  auth_status?: unknown;
  error?: unknown;
  exit_code?: unknown;
}

function optionalContractString(
  record: Record<string, unknown>,
  field: string,
  path: string,
): string | null {
  if (!hasContractField(record, field) || record[field] === null) {
    return null;
  }
  return requireContractString(record[field], `${path}.${field}`);
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

function mapScopedUsageLimit(value: unknown, path: string): DesktopScopedUsageLimit {
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
    kind: requireContractNonEmptyString(
      requireContractField(record, "kind", path),
      `${path}.kind`,
    ),
    window: mapUsageWindow(
      requireContractField(record, "window", path),
      `${path}.window`,
    ),
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
    scopedLimits: hasContractField(record, "scoped_limits")
      ? requireContractArray(record.scoped_limits, `${path}.scoped_limits`).map((limit, index) =>
          mapScopedUsageLimit(limit, `${path}.scoped_limits[${index}]`),
        )
      : [],
    models: hasContractField(record, "models")
      ? requireContractArray(record.models, `${path}.models`).map((model, index) =>
          mapModelUsage(model, `${path}.models[${index}]`),
        )
      : [],
    error: hasContractField(record, "error")
      ? requireContractString(record.error, `${path}.error`)
      : null,
    errorCode: hasContractField(record, "error_code")
      ? requireContractString(record.error_code, `${path}.error_code`)
      : null,
    retryAfterSeconds: hasContractField(record, "retry_after_seconds")
      ? requireContractInteger(record.retry_after_seconds, `${path}.retry_after_seconds`)
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
    "readRetryable",
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
    "readRetryable",
    {
      signal: AbortSignal.timeout(15000),
    },
  );

  return mapCodingUsage(payload);
}

function mapClaudeCodeAccount(value: unknown, index: number): DesktopClaudeCodeAccount {
  const path = `Claude Code accounts.accounts[${index}]`;
  const record = requireContractRecord(value, path);
  return {
    id: optionalContractString(record, "id", path),
    name: requireContractNonEmptyString(
      requireContractField(record, "name", path),
      `${path}.name`,
    ),
    systemDefault: requireContractBoolean(
      requireContractField(record, "system_default", path),
      `${path}.system_default`,
    ),
    selected: requireContractBoolean(
      requireContractField(record, "selected", path),
      `${path}.selected`,
    ),
    email: optionalContractString(record, "email", path),
    organization: optionalContractString(record, "organization", path),
    plan: optionalContractString(record, "plan", path),
    authMethod: optionalContractString(record, "auth_method", path),
    usage: mapProviderUsage(
      requireContractField(record, "usage", path),
      `${path}.usage`,
    ),
  };
}

function mapClaudeCodeAccounts(value: unknown): DesktopClaudeCodeAccounts {
  const path = "Claude Code accounts";
  const record = requireContractRecord(value, path);
  return {
    activeAccountId: optionalContractString(record, "active_account_id", path),
    accounts: requireContractArray(
      requireContractField(record, "accounts", path),
      `${path}.accounts`,
    ).map(mapClaudeCodeAccount),
    refreshedAt: requireContractNonEmptyString(
      requireContractField(record, "refreshed_at", path),
      `${path}.refreshed_at`,
    ),
  };
}

function mapClaudeAuthSession(value: unknown): DesktopClaudeAuthSession {
  const path = "Claude Code auth session";
  const record = requireContractRecord(value, path);
  const rawStatus = requireContractNonEmptyString(
    requireContractField(record, "status", path),
    `${path}.status`,
  );
  const statuses: DesktopClaudeAuthStatus[] = [
    "starting",
    "waiting_for_code",
    "submitted",
    "succeeded",
    "failed",
  ];
  if (!statuses.includes(rawStatus as DesktopClaudeAuthStatus)) {
    throw new GatewayContractError(`${path}.status`, "must be a known auth status");
  }
  const authStatus = hasContractField(record, "auth_status") && record.auth_status !== null
    ? requireContractRecord(record.auth_status, `${path}.auth_status`)
    : null;
  return {
    loginId: requireContractNonEmptyString(
      requireContractField(record, "login_id", path),
      `${path}.login_id`,
    ),
    accountId: optionalContractString(record, "account_id", path),
    status: rawStatus as DesktopClaudeAuthStatus,
    authorizationUrl: optionalContractString(record, "url", path),
    authStatus,
    error: optionalContractString(record, "error", path),
    exitCode:
      hasContractField(record, "exit_code") && record.exit_code !== null
        ? requireContractInteger(record.exit_code, `${path}.exit_code`)
        : null,
  };
}

export async function listClaudeCodeAccounts(
  settings: DesktopSettings,
): Promise<DesktopClaudeCodeAccounts> {
  const payload = await requestJson<ClaudeCodeAccountsPayload>(
    settings,
    "/api/providers/claude_code/accounts",
    "readRetryable",
    { signal: AbortSignal.timeout(30000) },
  );
  return mapClaudeCodeAccounts(payload);
}

export async function selectClaudeCodeAccount(
  settings: DesktopSettings,
  accountId: string | null,
): Promise<DesktopClaudeCodeAccountSelection> {
  const payload = await requestJson<unknown>(
    settings,
    "/api/providers/claude_code/accounts/active",
    "mutationSingleAttempt",
    {
      method: "PUT",
      signal: AbortSignal.timeout(15000),
      body: JSON.stringify({ account_id: accountId }),
    },
  );
  const path = "Claude Code account selection";
  const record = requireContractRecord(payload, path);
  const recovery = hasContractField(record, "recovery")
    ? requireContractRecord(record.recovery, `${path}.recovery`)
    : {};
  return {
    activeAccountId: optionalContractString(record, "active_account_id", path),
    selectionChanged: hasContractField(record, "selection_changed")
      ? requireContractBoolean(record.selection_changed, `${path}.selection_changed`)
      : true,
    recovery: {
      matchedThreads: hasContractField(recovery, "matched_threads")
        ? requireContractInteger(recovery.matched_threads, `${path}.recovery.matched_threads`)
        : 0,
      expeditedThreads: hasContractField(recovery, "expedited_threads")
        ? requireContractInteger(recovery.expedited_threads, `${path}.recovery.expedited_threads`)
        : 0,
      alreadyClaimedThreads: hasContractField(recovery, "already_claimed_threads")
        ? requireContractInteger(
          recovery.already_claimed_threads,
          `${path}.recovery.already_claimed_threads`,
        )
        : 0,
    },
    recoveryWarning: optionalContractString(record, "recovery_warning", path),
  };
}

export async function retryThreadQuotaRecovery(
  settings: DesktopSettings,
  threadId: string,
): Promise<DesktopQuotaRecoveryRetryResult> {
  try {
    await requestJson<unknown>(
      settings,
      `/api/threads/${encodeURIComponent(threadId)}/quota-recovery/retry`,
      "mutationSingleAttempt",
      { method: "POST", signal: AbortSignal.timeout(15000) },
    );
    return { status: "accepted" };
  } catch (error) {
    if (error instanceof GatewayRequestError && error.status === 404) {
      let code: string | null = null;
      try {
        const body = JSON.parse(error.body) as { error?: unknown };
        code = typeof body.error === "string" ? body.error : null;
      } catch {
        // An older gateway may return plain text or HTML for the unknown route.
      }
      return {
        status: code === "quota_recovery_not_found" ? "settled" : "unsupported",
      };
    }
    throw error;
  }
}

export async function renameClaudeCodeAccount(
  settings: DesktopSettings,
  accountId: string,
  name: string,
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/providers/claude_code/accounts/${encodeURIComponent(accountId)}`,
    "mutationSingleAttempt",
    {
      method: "PATCH",
      signal: AbortSignal.timeout(15000),
      body: JSON.stringify({ name }),
    },
  );
}

export async function deleteClaudeCodeAccount(
  settings: DesktopSettings,
  accountId: string,
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/providers/claude_code/accounts/${encodeURIComponent(accountId)}`,
    "mutationSingleAttempt",
    { method: "DELETE", signal: AbortSignal.timeout(15000) },
  );
}

export async function startClaudeCodeAuth(
  settings: DesktopSettings,
  input: StartDesktopClaudeAuthInput,
): Promise<DesktopClaudeAuthSession> {
  const payload = await requestJson<ClaudeAuthPayload>(
    settings,
    "/api/providers/claude_code/auth/start",
    "mutationSingleAttempt",
    {
      method: "POST",
      signal: AbortSignal.timeout(45000),
      body: JSON.stringify({
        mode: input.mode || "claudeai",
        sso: input.sso || false,
        email: input.email || null,
        managed_account_name: input.managedAccountName || null,
        account_id: input.accountId || null,
      }),
    },
  );
  return mapClaudeAuthSession(payload);
}

export async function submitClaudeCodeAuth(
  settings: DesktopSettings,
  loginId: string,
  code: string,
): Promise<DesktopClaudeAuthSession> {
  const payload = await requestJson<ClaudeAuthPayload>(
    settings,
    `/api/providers/claude_code/auth/${encodeURIComponent(loginId)}/submit`,
    "mutationSingleAttempt",
    {
      method: "POST",
      signal: AbortSignal.timeout(15000),
      body: JSON.stringify({ code }),
    },
  );
  return mapClaudeAuthSession(payload);
}

export async function getClaudeCodeAuth(
  settings: DesktopSettings,
  loginId: string,
): Promise<DesktopClaudeAuthSession> {
  const payload = await requestJson<ClaudeAuthPayload>(
    settings,
    `/api/providers/claude_code/auth/${encodeURIComponent(loginId)}`,
    "readRetryable",
    { signal: AbortSignal.timeout(15000) },
  );
  return mapClaudeAuthSession(payload);
}

export async function cancelClaudeCodeAuth(
  settings: DesktopSettings,
  loginId: string,
): Promise<DesktopClaudeAuthSession> {
  const payload = await requestJson<ClaudeAuthPayload>(
    settings,
    `/api/providers/claude_code/auth/${encodeURIComponent(loginId)}`,
    "mutationSingleAttempt",
    { method: "DELETE", signal: AbortSignal.timeout(15000) },
  );
  return mapClaudeAuthSession(payload);
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
    "readRetryable",
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
