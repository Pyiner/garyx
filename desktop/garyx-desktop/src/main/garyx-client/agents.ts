import type {
  CreateCustomAgentInput,
  DeleteCustomAgentInput,
  DesktopAgentCatalog,
  DesktopCustomAgent,
  DesktopProviderIconDescriptor,
  DesktopProviderIconKey,
  DesktopSettings,
  SetDefaultCustomAgentInput,
  ToggleCustomAgentInput,
  UpdateCustomAgentInput,
} from "@shared/contracts";
import {
  GatewayContractError,
  hasContractField,
  requestJson,
  requireContractArray,
  requireContractBoolean,
  requireContractField,
  requireContractNonEmptyString,
  requireContractRecord,
  requireContractString,
} from "./http.ts";
import { normalizeDesktopProviderType } from "./provider.ts";

interface CustomAgentPayload {
  agent_id?: string;
  display_name?: string;
  provider_type?: string;
  model?: string | null;
  model_reasoning_effort?: string | null;
  model_service_tier?: string | null;
  provider_env?: Record<string, string> | null;
  default_workspace_dir?: string | null;
  avatar_data_url?: string | null;
  provider_icon?: ProviderIconDescriptorPayload | null;
  system_prompt?: string | null;
  built_in?: boolean;
  standalone?: boolean;
  enabled?: boolean;
  created_at?: string;
  updated_at?: string;
}

interface ProviderIconDescriptorPayload {
  key?: unknown;
  provider_type?: unknown;
  label?: unknown;
}

interface CustomAgentsPayload {
  agents?: CustomAgentPayload[];
  default_agent_id?: string | null;
  effective_default_agent_id?: string | null;
}

function normalizeProviderIconKey(
  value: unknown,
  path: string,
): DesktopProviderIconKey {
  if (value === "claude" || value === "codex" || value === "traex" || value === "gemini") {
    return value;
  }
  throw new GatewayContractError(path, "must be a current provider icon key");
}

function mapProviderIconDescriptor(
  value: unknown,
  path: string,
): DesktopProviderIconDescriptor | null {
  if (value === null) {
    return null;
  }
  const record = requireContractRecord(value, path);
  const providerType = requireContractField(record, "provider_type", path);
  return {
    key: normalizeProviderIconKey(
      requireContractField(record, "key", path),
      `${path}.key`,
    ),
    providerType: mapAgentProviderType(
      providerType,
      `${path}.provider_type`,
    ),
    label: requireContractString(
      requireContractField(record, "label", path),
      `${path}.label`,
    ),
  };
}

function mapAgentProviderType(value: unknown, path: string): DesktopCustomAgent["providerType"] {
  if (
    value !== "claude_code" &&
    value !== "codex_app_server" &&
    value !== "traex" &&
    value !== "antigravity"
  ) {
    throw new GatewayContractError(path, "must be a current provider type");
  }
  return normalizeDesktopProviderType(value);
}

function optionalAgentString(
  record: Record<string, unknown>,
  field: string,
  path: string,
): string {
  if (!hasContractField(record, field)) {
    return "";
  }
  return requireContractString(record[field], `${path}.${field}`);
}

function requiredNullableAgentString(
  record: Record<string, unknown>,
  field: string,
  path: string,
): string {
  const value = requireContractField(record, field, path);
  return value === null
    ? ""
    : requireContractString(value, `${path}.${field}`);
}

function requiredNullableAgentId(
  record: Record<string, unknown>,
  field: string,
  path: string,
): string | null {
  const value = requireContractField(record, field, path);
  return value === null
    ? null
    : requireContractNonEmptyString(value, `${path}.${field}`);
}

function mapProviderEnv(value: unknown, path: string): Record<string, string> {
  const record = requireContractRecord(value, path);
  return Object.fromEntries(
    Object.entries(record).map(([key, entry]) => [
      key,
      requireContractString(entry, `${path}.${key}`),
    ]),
  );
}

function mapCustomAgent(value: unknown, index?: number): DesktopCustomAgent {
  const path = index === undefined
    ? "custom agent"
    : `custom agent list.agents[${index}]`;
  const record = requireContractRecord(value, path);
  return {
    agentId: requireContractNonEmptyString(
      requireContractField(record, "agent_id", path),
      `${path}.agent_id`,
    ),
    displayName: requireContractNonEmptyString(
      requireContractField(record, "display_name", path),
      `${path}.display_name`,
    ),
    providerType: mapAgentProviderType(
      requireContractField(record, "provider_type", path),
      `${path}.provider_type`,
    ),
    model: requireContractString(
      requireContractField(record, "model", path),
      `${path}.model`,
    ),
    modelReasoningEffort: requireContractString(
      requireContractField(record, "model_reasoning_effort", path),
      `${path}.model_reasoning_effort`,
    ),
    modelServiceTier: requireContractString(
      requireContractField(record, "model_service_tier", path),
      `${path}.model_service_tier`,
    ),
    providerEnv: hasContractField(record, "provider_env")
      ? mapProviderEnv(record.provider_env, `${path}.provider_env`)
      : {},
    defaultWorkspaceDir: optionalAgentString(
      record,
      "default_workspace_dir",
      path,
    ),
    avatarDataUrl: requiredNullableAgentString(
      record,
      "avatar_data_url",
      path,
    ),
    providerIcon: mapProviderIconDescriptor(
      requireContractField(record, "provider_icon", path),
      `${path}.provider_icon`,
    ),
    systemPrompt: requireContractString(
      requireContractField(record, "system_prompt", path),
      `${path}.system_prompt`,
    ),
    builtIn: requireContractBoolean(
      requireContractField(record, "built_in", path),
      `${path}.built_in`,
    ),
    standalone: requireContractBoolean(
      requireContractField(record, "standalone", path),
      `${path}.standalone`,
    ),
    enabled: requireContractBoolean(
      requireContractField(record, "enabled", path),
      `${path}.enabled`,
    ),
    createdAt: requireContractNonEmptyString(
      requireContractField(record, "created_at", path),
      `${path}.created_at`,
    ),
    updatedAt: requireContractNonEmptyString(
      requireContractField(record, "updated_at", path),
      `${path}.updated_at`,
    ),
  };
}

export async function listCustomAgents(
  settings: DesktopSettings,
): Promise<DesktopAgentCatalog> {
  const payload = await requestJson<CustomAgentsPayload>(
    settings,
    "/api/custom-agents",
    {
      signal: AbortSignal.timeout(8000),
    },
  );

  const record = requireContractRecord(payload, "custom agent list");
  const agents = requireContractArray(
    requireContractField(record, "agents", "custom agent list"),
    "custom agent list.agents",
  ).map(mapCustomAgent);
  return {
    agents,
    defaultAgentId: requiredNullableAgentId(
      record,
      "default_agent_id",
      "custom agent list",
    ),
    effectiveDefaultAgentId: requiredNullableAgentId(
      record,
      "effective_default_agent_id",
      "custom agent list",
    ),
  };
}

export async function createCustomAgent(
  settings: DesktopSettings,
  input: CreateCustomAgentInput,
): Promise<DesktopCustomAgent> {
  const payload = await requestJson<CustomAgentPayload>(
    settings,
    "/api/custom-agents",
    {
      method: "POST",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        agent_id: input.agentId,
        display_name: input.displayName,
        provider_type: input.providerType,
        model: input.model,
        model_reasoning_effort: input.modelReasoningEffort,
        model_service_tier: input.modelServiceTier,
        provider_env: input.providerEnv ?? null,
        default_workspace_dir: input.defaultWorkspaceDir,
        avatar_data_url: input.avatarDataUrl ?? null,
        system_prompt: input.systemPrompt,
      }),
    },
  );

  return mapCustomAgent(payload);
}

export async function updateCustomAgent(
  settings: DesktopSettings,
  input: UpdateCustomAgentInput,
): Promise<DesktopCustomAgent> {
  const payload = await requestJson<CustomAgentPayload>(
    settings,
    `/api/custom-agents/${encodeURIComponent(input.currentAgentId)}`,
    {
      method: "PUT",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        agent_id: input.agentId,
        display_name: input.displayName,
        provider_type: input.providerType,
        model: input.model,
        model_reasoning_effort: input.modelReasoningEffort,
        model_service_tier: input.modelServiceTier,
        provider_env: input.providerEnv ?? null,
        default_workspace_dir: input.defaultWorkspaceDir,
        avatar_data_url: input.avatarDataUrl ?? null,
        system_prompt: input.systemPrompt,
        expected_updated_at: input.expectedUpdatedAt,
      }),
    },
  );

  return mapCustomAgent(payload);
}

export async function deleteCustomAgent(
  settings: DesktopSettings,
  input: DeleteCustomAgentInput,
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/custom-agents/${encodeURIComponent(input.agentId)}`,
    {
      method: "DELETE",
      signal: AbortSignal.timeout(8000),
    },
  );
}

export async function toggleCustomAgent(
  settings: DesktopSettings,
  input: ToggleCustomAgentInput,
): Promise<DesktopCustomAgent> {
  const payload = await requestJson<CustomAgentPayload>(
    settings,
    `/api/custom-agents/${encodeURIComponent(input.agentId)}/toggle`,
    {
      method: "PATCH",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({ enabled: input.enabled }),
    },
  );
  return mapCustomAgent(payload);
}

export async function setDefaultCustomAgent(
  settings: DesktopSettings,
  input: SetDefaultCustomAgentInput,
): Promise<DesktopCustomAgent> {
  const payload = await requestJson<CustomAgentPayload>(
    settings,
    `/api/custom-agents/${encodeURIComponent(input.agentId)}/default`,
    {
      method: "PATCH",
      signal: AbortSignal.timeout(8000),
    },
  );
  return mapCustomAgent(payload);
}
