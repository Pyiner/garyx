import type {
  DesktopApiProviderType,
  DesktopProviderIconDescriptor,
} from "./provider.ts";

export interface DesktopCustomAgent {
  agentId: string;
  displayName: string;
  providerType: DesktopApiProviderType;
  model: string;
  modelReasoningEffort: string;
  modelServiceTier: string;
  providerEnv: Record<string, string>;
  authSource: string;
  baseUrl: string;
  codexHome: string;
  maxToolIterations: number;
  requestTimeoutSeconds: number;
  defaultWorkspaceDir: string;
  avatarDataUrl: string;
  providerIcon?: DesktopProviderIconDescriptor | null;
  systemPrompt: string;
  builtIn: boolean;
  standalone: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface DesktopTeam {
  teamId: string;
  displayName: string;
  leaderAgentId: string;
  memberAgentIds: string[];
  workflowText: string;
  avatarDataUrl: string;
  createdAt: string;
  updatedAt: string;
}

export interface CreateCustomAgentInput {
  agentId: string;
  displayName: string;
  providerType: DesktopApiProviderType;
  model: string;
  modelReasoningEffort: string;
  modelServiceTier: string;
  providerEnv?: Record<string, string> | null;
  authSource?: string | null;
  baseUrl?: string | null;
  codexHome?: string | null;
  maxToolIterations?: number | null;
  requestTimeoutSeconds?: number | null;
  defaultWorkspaceDir: string;
  avatarDataUrl?: string | null;
  systemPrompt: string;
}

export interface UpdateCustomAgentInput extends CreateCustomAgentInput {
  currentAgentId: string;
  /** Concurrency token: the `updatedAt` of the agent this edit was based on. */
  expectedUpdatedAt: string;
}

export interface DeleteCustomAgentInput {
  agentId: string;
}

export interface GenerateCustomAgentAvatarInput {
  agentId?: string | null;
  displayName: string;
  kind?: "agent" | "team";
  stylePrompt?: string | null;
}

export interface GenerateCustomAgentAvatarResult {
  avatarDataUrl: string;
  mediaType: string;
}

export interface CreateTeamInput {
  teamId: string;
  displayName: string;
  leaderAgentId: string;
  memberAgentIds: string[];
  workflowText: string;
  avatarDataUrl?: string | null;
}

export interface UpdateTeamInput extends CreateTeamInput {
  currentTeamId: string;
  /** Concurrency token: the `updatedAt` of the team this edit was based on. */
  expectedUpdatedAt: string;
}

export interface DeleteTeamInput {
  teamId: string;
}
