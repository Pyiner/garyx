export type DesktopApiProviderType =
  | "claude_code"
  | "codex_app_server"
  | "antigravity"
  | "traex"
  | "gpt"
  | "anthropic"
  | "google"
  | "claude_llm"
  | "gemini_llm";

export type DesktopProviderIconKey = "claude" | "codex" | "traex" | "gemini";

export interface DesktopProviderIconDescriptor {
  key: DesktopProviderIconKey;
  providerType?: DesktopApiProviderType | null;
  label?: string | null;
}

export interface DesktopProviderModelOption {
  id: string;
  label: string;
  description?: string | null;
  recommended?: boolean;
  defaultReasoningEffort?: string | null;
  supportedReasoningEfforts?: DesktopProviderModelOption[];
  serviceTiers?: DesktopProviderModelOption[];
}

export interface DesktopProviderModels {
  providerType: DesktopApiProviderType;
  supportsModelSelection: boolean;
  models: DesktopProviderModelOption[];
  supportsReasoningEffortSelection?: boolean;
  reasoningEfforts?: DesktopProviderModelOption[];
  supportsServiceTierSelection?: boolean;
  serviceTiers?: DesktopProviderModelOption[];
  defaultModel?: string | null;
  defaultReasoningEffort?: string | null;
  source: string;
  error?: string | null;
}

export interface DesktopUsageWindow {
  usedPercent: number;
  remainingPercent: number;
  resetsAt?: string | null;
  resetAfterSeconds?: number | null;
}

export interface DesktopModelUsage {
  id: string;
  name: string;
  remainingFraction: number;
  remainingPercent: number;
  usedPercent: number;
  resetsAt?: string | null;
  resetAfterSeconds?: number | null;
  description?: string | null;
}

export interface DesktopProviderUsage {
  id: string;
  name: string;
  available: boolean;
  stale: boolean;
  plan?: string | null;
  weekly?: DesktopUsageWindow | null;
  session?: DesktopUsageWindow | null;
  models: DesktopModelUsage[];
  error?: string | null;
}

export interface DesktopCodingUsage {
  providers: DesktopProviderUsage[];
  refreshedAt?: string | null;
}

export type DesktopThreadProviderType = DesktopApiProviderType;

export type DesktopSessionProviderHint = "claude" | "codex";

export interface DesktopProviderRecentSession {
  providerType: DesktopApiProviderType | string;
  providerHint: DesktopSessionProviderHint;
  sessionId: string;
  title: string;
  workspaceDir: string;
  updatedAt: string;
  path?: string | null;
}

export interface ListProviderRecentSessionsInput {
  provider?: DesktopSessionProviderHint | null;
  limit?: number | null;
}
