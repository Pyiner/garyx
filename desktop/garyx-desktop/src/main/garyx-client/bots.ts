import type {
  DesktopBotConsoleSummary,
  DesktopBotConversationNode,
  DesktopSettings,
} from "@shared/contracts";
import { mapChannelEndpoint } from "./channels.ts";
import type { ChannelEndpointsPayload } from "./channels.ts";
import { REMOTE_STATE_FETCH_TIMEOUT_MS, requestJson } from "./http.ts";

export interface BotConsoleSummaryPayload {
  id?: string;
  channel?: string;
  account_id?: string;
  title?: string;
  subtitle?: string;
  root_behavior?: "open_default" | "expand_only" | string;
  status?: "connected" | "idle" | string;
  latest_activity?: string | null;
  endpoint_count?: number;
  bound_endpoint_count?: number;
  workspace_dir?: string | null;
  main_endpoint_status?: "resolved" | "unresolved" | string;
  main_endpoint?: NonNullable<ChannelEndpointsPayload["endpoints"]>[number] | null;
  main_endpoint_thread_id?: string | null;
  default_open_endpoint?: NonNullable<ChannelEndpointsPayload["endpoints"]>[number] | null;
  default_open_thread_id?: string | null;
  conversation_nodes?: Array<{
    id?: string;
    endpoint?: NonNullable<ChannelEndpointsPayload["endpoints"]>[number] | null;
    kind?: string | null;
    title?: string | null;
    badge?: string | null;
    latest_activity?: string | null;
    openable?: boolean;
  }> | null;
  endpoints?: NonNullable<ChannelEndpointsPayload["endpoints"]> | null;
}

function mapBotConversationNode(
  value: NonNullable<BotConsoleSummaryPayload["conversation_nodes"]>[number],
): DesktopBotConversationNode | null {
  const id = value.id?.trim() || "";
  const endpoint = value.endpoint ? mapChannelEndpoint(value.endpoint) : null;
  if (!id || !endpoint) {
    return null;
  }
  return {
    id,
    endpoint,
    kind: value.kind?.trim() || "unknown",
    title: value.title?.trim() || endpoint.displayLabel || id,
    badge: value.badge?.trim() || null,
    latestActivity: value.latest_activity?.trim() || null,
    openable: value.openable !== false,
  };
}

function mapBotConsoleSummary(
  value: BotConsoleSummaryPayload,
): DesktopBotConsoleSummary {
  const endpoints = Array.isArray(value.endpoints)
    ? value.endpoints.map(mapChannelEndpoint)
    : [];
  const conversationNodes = Array.isArray(value.conversation_nodes)
    ? value.conversation_nodes
        .map(mapBotConversationNode)
        .filter((entry): entry is DesktopBotConversationNode => Boolean(entry))
    : [];
  return {
    id: value.id || "",
    channel: value.channel || "",
    accountId: value.account_id || "",
    title: value.title || value.id || "",
    subtitle: value.subtitle || "",
    rootBehavior:
      value.root_behavior === "expand_only" ? "expand_only" : "open_default",
    status: value.status === "connected" ? "connected" : "idle",
    latestActivity: value.latest_activity?.trim() || null,
    endpointCount: value.endpoint_count || 0,
    boundEndpointCount: value.bound_endpoint_count || 0,
    workspaceDir: value.workspace_dir?.trim() || null,
    mainEndpointStatus:
      value.main_endpoint_status === "resolved" ? "resolved" : "unresolved",
    mainEndpoint: value.main_endpoint ? mapChannelEndpoint(value.main_endpoint) : null,
    mainThreadId: value.main_endpoint_thread_id?.trim() || null,
    defaultOpenEndpoint: value.default_open_endpoint
      ? mapChannelEndpoint(value.default_open_endpoint)
      : null,
    defaultOpenThreadId: value.default_open_thread_id?.trim() || null,
    conversationNodes,
    endpoints,
  };
}

export interface ConfiguredBotPayload {
  channel: string;
  account_id: string;
  display_name?: string | null;
  displayName?: string | null;
  name?: string | null;
  enabled: boolean;
  workspace_dir?: string | null;
  root_behavior?: "open_default" | "expand_only" | string;
  main_endpoint_status?: "resolved" | "unresolved" | string;
  main_endpoint?:
    | NonNullable<ChannelEndpointsPayload["endpoints"]>[number]
    | null;
  main_endpoint_thread_id?: string | null;
  default_open_endpoint?:
    | NonNullable<ChannelEndpointsPayload["endpoints"]>[number]
    | null;
  default_open_thread_id?: string | null;
}

export async function fetchConfiguredBots(
  settings: DesktopSettings,
): Promise<ConfiguredBotPayload[]> {
  const payload = await requestJson<{ bots?: ConfiguredBotPayload[] }>(
    settings,
    "/api/configured-bots",
    { signal: AbortSignal.timeout(REMOTE_STATE_FETCH_TIMEOUT_MS) },
  );
  return Array.isArray(payload.bots) ? payload.bots : [];
}

export async function fetchBotConsoles(
  settings: DesktopSettings,
): Promise<DesktopBotConsoleSummary[]> {
  const payload = await requestJson<{ bots?: BotConsoleSummaryPayload[] }>(
    settings,
    "/api/bot-consoles",
    { signal: AbortSignal.timeout(REMOTE_STATE_FETCH_TIMEOUT_MS) },
  );
  return Array.isArray(payload.bots)
    ? payload.bots.map(mapBotConsoleSummary)
    : [];
}
