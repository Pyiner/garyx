import type {
  CreateCustomAgentInput,
  CreateTeamInput,
  DeleteCustomAgentInput,
  DeleteTeamInput,
  DesktopCustomAgent,
  DesktopProviderIconDescriptor,
  DesktopProviderIconKey,
  DesktopSettings,
  DesktopTeam,
  UpdateCustomAgentInput,
  UpdateTeamInput,
} from "@shared/contracts";
import { baseUrl, requestJson } from "./http.ts";
import { normalizeDesktopProviderType } from "./provider.ts";

interface CustomAgentPayload {
  agent_id?: string;
  agentId?: string;
  display_name?: string;
  displayName?: string;
  role?: string | null;
  provider_type?: string;
  providerType?: string;
  model?: string | null;
  model_reasoning_effort?: string | null;
  modelReasoningEffort?: string | null;
  model_service_tier?: string | null;
  modelServiceTier?: string | null;
  provider_env?: Record<string, string> | null;
  providerEnv?: Record<string, string> | null;
  env?: Record<string, string> | null;
  auth_source?: string | null;
  authSource?: string | null;
  base_url?: string | null;
  baseUrl?: string | null;
  codex_home?: string | null;
  codexHome?: string | null;
  max_tool_iterations?: number | null;
  maxToolIterations?: number | null;
  request_timeout_seconds?: number | null;
  requestTimeoutSeconds?: number | null;
  default_workspace_dir?: string | null;
  defaultWorkspaceDir?: string | null;
  avatar_data_url?: string | null;
  avatarDataUrl?: string | null;
  provider_icon?: ProviderIconDescriptorPayload | null;
  providerIcon?: ProviderIconDescriptorPayload | null;
  workspace_dir?: string | null;
  workspaceDir?: string | null;
  system_prompt?: string | null;
  systemPrompt?: string | null;
  built_in?: boolean;
  builtIn?: boolean;
  standalone?: boolean;
  created_at?: string;
  createdAt?: string;
  updated_at?: string;
  updatedAt?: string;
}

interface ProviderIconDescriptorPayload {
  key?: unknown;
  provider_type?: unknown;
  providerType?: unknown;
  label?: unknown;
}

interface CustomAgentsPayload {
  agents?: CustomAgentPayload[];
}

interface TeamPayload {
  team_id?: string;
  teamId?: string;
  display_name?: string;
  displayName?: string;
  leader_agent_id?: string;
  leaderAgentId?: string;
  member_agent_ids?: unknown;
  memberAgentIds?: unknown;
  workflow_text?: string | null;
  workflowText?: string | null;
  avatar_data_url?: string | null;
  avatarDataUrl?: string | null;
  created_at?: string;
  createdAt?: string;
  updated_at?: string;
  updatedAt?: string;
}

interface TeamsPayload {
  teams?: TeamPayload[];
}

function normalizeProviderIconKey(value: unknown): DesktopProviderIconKey | null {
  if (value === "claude" || value === "codex" || value === "traex" || value === "gemini") {
    return value;
  }
  return null;
}

function mapProviderIconDescriptor(
  value: ProviderIconDescriptorPayload | null | undefined,
): DesktopProviderIconDescriptor | null {
  if (!value || typeof value !== "object") {
    return null;
  }
  const key = normalizeProviderIconKey(value.key);
  if (!key) {
    return null;
  }
  return {
    key,
    providerType:
      value.provider_type || value.providerType
        ? normalizeDesktopProviderType(value.provider_type || value.providerType)
        : null,
    label: typeof value.label === "string" ? value.label : null,
  };
}

function mapTeam(value: TeamPayload): DesktopTeam {
  const members = Array.isArray(value.member_agent_ids)
    ? value.member_agent_ids
    : Array.isArray(value.memberAgentIds)
      ? value.memberAgentIds
      : [];
  return {
    teamId: value.team_id || value.teamId || "",
    displayName: value.display_name || value.displayName || "",
    leaderAgentId: value.leader_agent_id || value.leaderAgentId || "",
    memberAgentIds: members.filter(
      (entry): entry is string => typeof entry === "string",
    ),
    workflowText: value.workflow_text || value.workflowText || "",
    avatarDataUrl: value.avatar_data_url || value.avatarDataUrl || "",
    createdAt: value.created_at || value.createdAt || new Date(0).toISOString(),
    updatedAt: value.updated_at || value.updatedAt || new Date(0).toISOString(),
  };
}

function mapCustomAgent(value: CustomAgentPayload): DesktopCustomAgent {
  const provider = normalizeDesktopProviderType(
    value.provider_type || value.providerType,
  );
  return {
    agentId: value.agent_id || value.agentId || "",
    displayName: value.display_name || value.displayName || "",
    providerType: provider,
    model: value.model || "",
    modelReasoningEffort:
      value.model_reasoning_effort || value.modelReasoningEffort || "",
    modelServiceTier: value.model_service_tier || value.modelServiceTier || "",
    providerEnv:
      value.provider_env || value.providerEnv || value.env || {},
    authSource: value.auth_source || value.authSource || "",
    baseUrl: value.base_url || value.baseUrl || "",
    codexHome: value.codex_home || value.codexHome || "",
    maxToolIterations:
      value.max_tool_iterations || value.maxToolIterations || 32,
    requestTimeoutSeconds:
      value.request_timeout_seconds || value.requestTimeoutSeconds || 300,
    defaultWorkspaceDir:
      value.default_workspace_dir ??
      value.defaultWorkspaceDir ??
      value.workspace_dir ??
      value.workspaceDir ??
      "",
    avatarDataUrl: value.avatar_data_url || value.avatarDataUrl || "",
    providerIcon: mapProviderIconDescriptor(
      value.provider_icon || value.providerIcon,
    ),
    systemPrompt: value.system_prompt || value.systemPrompt || "",
    builtIn: value.built_in === true || value.builtIn === true,
    standalone: value.standalone !== false,
    createdAt: value.created_at || value.createdAt || "",
    updatedAt: value.updated_at || value.updatedAt || "",
  };
}

export async function listCustomAgents(
  settings: DesktopSettings,
): Promise<DesktopCustomAgent[]> {
  const payload = await requestJson<CustomAgentsPayload>(
    settings,
    "/api/custom-agents",
    {
      signal: AbortSignal.timeout(8000),
    },
  );

  return Array.isArray(payload.agents)
    ? payload.agents.map(mapCustomAgent)
    : [];
}

export async function listTeams(
  settings: DesktopSettings,
): Promise<DesktopTeam[]> {
  const payload = await requestJson<TeamsPayload>(settings, "/api/teams", {
    signal: AbortSignal.timeout(8000),
  });

  return Array.isArray(payload.teams) ? payload.teams.map(mapTeam) : [];
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
        auth_source: input.authSource ?? null,
        base_url: input.baseUrl ?? null,
        codex_home: input.codexHome ?? null,
        max_tool_iterations: input.maxToolIterations ?? null,
        request_timeout_seconds: input.requestTimeoutSeconds ?? null,
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
        auth_source: input.authSource ?? null,
        base_url: input.baseUrl ?? null,
        codex_home: input.codexHome ?? null,
        max_tool_iterations: input.maxToolIterations ?? null,
        request_timeout_seconds: input.requestTimeoutSeconds ?? null,
        default_workspace_dir: input.defaultWorkspaceDir,
        avatar_data_url: input.avatarDataUrl ?? null,
        system_prompt: input.systemPrompt,
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

export async function createTeam(
  settings: DesktopSettings,
  input: CreateTeamInput,
): Promise<DesktopTeam> {
  const payload = await requestJson<TeamPayload>(settings, "/api/teams", {
    method: "POST",
    signal: AbortSignal.timeout(8000),
    body: JSON.stringify({
      teamId: input.teamId,
      displayName: input.displayName,
      leaderAgentId: input.leaderAgentId,
      memberAgentIds: input.memberAgentIds,
      workflowText: input.workflowText,
      avatarDataUrl: input.avatarDataUrl ?? null,
    }),
  });
  return mapTeam(payload);
}

export async function updateTeam(
  settings: DesktopSettings,
  input: UpdateTeamInput,
): Promise<DesktopTeam> {
  const payload = await requestJson<TeamPayload>(
    settings,
    `/api/teams/${encodeURIComponent(input.currentTeamId)}`,
    {
      method: "PUT",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        teamId: input.teamId,
        displayName: input.displayName,
        leaderAgentId: input.leaderAgentId,
        memberAgentIds: input.memberAgentIds,
        workflowText: input.workflowText,
        avatarDataUrl: input.avatarDataUrl ?? null,
      }),
    },
  );
  return mapTeam(payload);
}

export async function deleteTeam(
  settings: DesktopSettings,
  input: DeleteTeamInput,
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/teams/${encodeURIComponent(input.teamId)}`,
    {
      method: "DELETE",
      signal: AbortSignal.timeout(8000),
    },
  );
}
