export type DesktopApiProviderType =
  | "claude_code"
  | "codex_app_server"
  | "antigravity"
  | "traex";

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

export interface DesktopScopedUsageLimit {
  id: string;
  name: string;
  kind: string;
  window: DesktopUsageWindow;
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
  scopedLimits: DesktopScopedUsageLimit[];
  models: DesktopModelUsage[];
  error?: string | null;
}

export interface DesktopCodingUsage {
  providers: DesktopProviderUsage[];
  refreshedAt?: string | null;
}

export interface DesktopClaudeCodeAccount {
  id: string | null;
  name: string;
  systemDefault: boolean;
  selected: boolean;
  email?: string | null;
  organization?: string | null;
  plan?: string | null;
  authMethod?: string | null;
  usage: DesktopProviderUsage;
}

export interface DesktopClaudeCodeAccounts {
  activeAccountId: string | null;
  accounts: DesktopClaudeCodeAccount[];
  refreshedAt: string;
}

export type DesktopClaudeAuthStatus =
  | "starting"
  | "waiting_for_code"
  | "submitted"
  | "succeeded"
  | "failed";

export interface DesktopClaudeAuthSession {
  loginId: string;
  accountId: string | null;
  status: DesktopClaudeAuthStatus;
  authorizationUrl: string | null;
  authStatus: Record<string, unknown> | null;
  error: string | null;
  exitCode: number | null;
}

export interface StartDesktopClaudeAuthInput {
  mode?: "claudeai" | "console";
  sso?: boolean;
  email?: string | null;
  managedAccountName?: string | null;
  accountId?: string | null;
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
