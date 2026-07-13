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
  defaultWorkspaceDir: string;
  avatarDataUrl: string;
  providerIcon?: DesktopProviderIconDescriptor | null;
  systemPrompt: string;
  builtIn: boolean;
  standalone: boolean;
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
  requestId: string;
  agentId?: string | null;
  displayName: string;
  stylePrompt?: string | null;
}

export interface CancelCustomAgentAvatarInput {
  requestId: string;
}

export type AvatarGenerationFailureCategory =
  | "unreachable"
  | "timeout"
  | "provider"
  | "unusable";

export type GenerateCustomAgentAvatarResult =
  | {
      status: "success";
      avatarDataUrl: string;
      mediaType: string;
    }
  | {
      status: "failure";
      category: AvatarGenerationFailureCategory;
      message: string;
    }
  | {
      status: "cancelled";
    };

export interface GeneratedCustomAgentAvatar {
  avatarDataUrl: string;
  mediaType: string;
}
