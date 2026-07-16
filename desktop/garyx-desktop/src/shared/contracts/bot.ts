import type { DesktopChannelEndpoint } from "./channel.ts";

export interface ConfiguredBot {
  channel: string;
  accountId: string;
  displayName: string;
  enabled: boolean;
  /** Persisted account override. Null means follow the global default. */
  agentId: string | null;
  /** Current resolved account agent, or null when no enabled agent exists. */
  effectiveAgentId: string | null;
  workspaceDir: string | null;
  rootBehavior: "open_default" | "expand_only";
  mainEndpointStatus: "resolved" | "unresolved";
  mainEndpoint?: DesktopChannelEndpoint | null;
  mainEndpointThreadId?: string | null;
  defaultOpenEndpoint?: DesktopChannelEndpoint | null;
  defaultOpenThreadId?: string | null;
}

export type DesktopBotConsoleStatus = "connected" | "idle";

export interface DesktopBotConversationNode {
  id: string;
  endpoint: DesktopChannelEndpoint;
  kind: string;
  title: string;
  badge: string | null;
  latestActivity: string | null;
  openable: boolean;
}

export interface DesktopBotConsoleSummary {
  id: string;
  channel: string;
  accountId: string;
  title: string;
  subtitle: string;
  rootBehavior: "open_default" | "expand_only";
  status: DesktopBotConsoleStatus;
  latestActivity: string | null;
  endpointCount: number;
  boundEndpointCount: number;
  /** Persisted account override. Null means follow the global default. */
  agentId: string | null;
  /** Current resolved account agent, or null when no enabled agent exists. */
  effectiveAgentId: string | null;
  workspaceDir: string | null;
  mainEndpointStatus: "resolved" | "unresolved";
  mainEndpoint: DesktopChannelEndpoint | null;
  mainThreadId: string | null;
  defaultOpenEndpoint: DesktopChannelEndpoint | null;
  defaultOpenThreadId: string | null;
  conversationNodes: DesktopBotConversationNode[];
  endpoints: DesktopChannelEndpoint[];
}

export interface SetBotBindingInput {
  threadId: string;
  botId: string | null;
}
