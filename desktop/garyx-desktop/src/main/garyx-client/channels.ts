import type {
  ChannelPluginCatalogEntry,
  DesktopChannelEndpoint,
  DesktopSettings,
} from "@shared/contracts";
import { REMOTE_STATE_FETCH_TIMEOUT_MS, requestJson } from "./http.ts";

export interface ChannelEndpointsPayload {
  endpoints?: Array<{
    endpoint_key?: string;
    channel?: string;
    account_id?: string;
    peer_id?: string;
    chat_id?: string;
    delivery_target_type?: "chat_id" | "open_id" | string;
    delivery_target_id?: string;
    thread_scope?: string | null;
    display_label?: string;
    thread_id?: string | null;
    thread_label?: string | null;
    workspace_dir?: string | null;
    thread_updated_at?: string | null;
    last_inbound_at?: string | null;
    last_delivery_at?: string | null;
    conversation_kind?: "private" | "group" | "topic" | "unknown" | string;
    conversation_label?: string | null;
  }>;
}

export function mapChannelEndpoint(
  value: NonNullable<ChannelEndpointsPayload["endpoints"]>[number],
): DesktopChannelEndpoint {
  const conversationKind = value.conversation_kind;
  return {
    endpointKey: value.endpoint_key || "",
    channel: value.channel || "",
    accountId: value.account_id || "",
    peerId: value.peer_id || "",
    chatId: value.chat_id || "",
    deliveryTargetType:
      value.delivery_target_type === "open_id" ? "open_id" : "chat_id",
    deliveryTargetId: value.delivery_target_id || value.chat_id || "",
    threadScope: value.thread_scope ?? null,
    displayLabel: value.display_label || "",
    threadId: value.thread_id ?? null,
    threadLabel: value.thread_label ?? null,
    workspacePath: value.workspace_dir ?? null,
    threadUpdatedAt: value.thread_updated_at ?? null,
    lastInboundAt: value.last_inbound_at ?? null,
    lastDeliveryAt: value.last_delivery_at ?? null,
    conversationKind:
      conversationKind === "private" ||
      conversationKind === "group" ||
      conversationKind === "topic" ||
      conversationKind === "unknown"
        ? conversationKind
        : null,
    conversationLabel: value.conversation_label ?? null,
  };
}

/**
 * `GET /api/channels/plugins` — the schema-driven catalog of ALL
 * channel plugins the gateway currently knows about, built-in +
 * subprocess, synthesised from the Rust-side `ChannelPluginManager`.
 *
 * Every entry carries a JSON Schema for its account config and a
 * `config_methods[]` array telling the UI which configuration
 * methods (form, auto_login) to render. The payload is channel-
 * blind — the Mac App iterates the list uniformly without caring
 * whether the entry came from a built-in or a subprocess plugin.
 */
export async function fetchChannelPlugins(
  settings: DesktopSettings,
): Promise<ChannelPluginCatalogEntry[]> {
  const result = await requestJson<{
    ok?: boolean;
    plugins?: ChannelPluginCatalogEntry[];
  }>(settings, "/api/channels/plugins", {
    method: "GET",
    signal: AbortSignal.timeout(5000),
  });
  return Array.isArray(result.plugins) ? result.plugins : [];
}

/**
 * Start a channel-blind auth-flow session against the gateway. The
 * desktop does NOT know whether the plugin is built-in or a
 * subprocess — it just hits `/auth_flow/start` with the plugin id
 * the catalog gave it and the current form state. The plugin
 * decides what "auto login" means (device code / QR / …).
 *
 * `form_state` may be `{}` — the plugin falls back to its own
 * defaults. If the plugin advertises `config_methods` without
 * `auto_login`, the gateway returns 404 and this call rejects.
 */
export async function startChannelAuthFlow(
  settings: DesktopSettings,
  pluginId: string,
  formState: Record<string, unknown>,
): Promise<{
  sessionId: string;
  display: Array<{ kind: string; value?: string }>;
  expiresInSecs: number;
  pollIntervalSecs: number;
}> {
  const payload = await requestJson<{
    ok?: boolean;
    session_id?: string;
    display?: Array<{ kind: string; value?: string }>;
    expires_in_secs?: number;
    poll_interval_secs?: number;
  }>(settings, `/api/channels/plugins/${encodeURIComponent(pluginId)}/auth_flow/start`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ form_state: formState }),
    signal: AbortSignal.timeout(15_000),
  });
  if (!payload.session_id) {
    throw new Error("auth_flow/start response missing session_id");
  }
  return {
    sessionId: payload.session_id,
    display: Array.isArray(payload.display) ? payload.display : [],
    expiresInSecs: Number(payload.expires_in_secs ?? 0),
    pollIntervalSecs: Math.max(1, Number(payload.poll_interval_secs ?? 5)),
  };
}

/**
 * Advance an in-flight auth-flow session. Returns the raw 3-state
 * poll result — `{status: "pending"|"confirmed"|"failed", ...}` —
 * for the caller to render.
 *
 * On a 404 (unknown session / plugin dropped) the underlying HTTP
 * call rejects; the caller's state machine should treat that as
 * terminal.
 */
export async function pollChannelAuthFlow(
  settings: DesktopSettings,
  pluginId: string,
  sessionId: string,
): Promise<{
  status: "pending" | "confirmed" | "failed" | string;
  display?: Array<{ kind: string; value?: string }>;
  next_interval_secs?: number;
  values?: Record<string, unknown>;
  reason?: string;
}> {
  return requestJson(
    settings,
    `/api/channels/plugins/${encodeURIComponent(pluginId)}/auth_flow/poll`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ session_id: sessionId }),
      signal: AbortSignal.timeout(15_000),
    },
  );
}

export async function validateChannelAccount(
  settings: DesktopSettings,
  pluginId: string,
  input: {
    accountId: string;
    enabled?: boolean;
    config: Record<string, unknown>;
  },
): Promise<{ validated: boolean; message: string }> {
  const payload = await requestJson<{
    ok?: boolean;
    validated?: boolean;
    message?: string;
  }>(
    settings,
    `/api/channels/plugins/${encodeURIComponent(pluginId)}/validate_account`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        account_id: input.accountId,
        enabled: input.enabled ?? true,
        config: input.config,
      }),
      signal: AbortSignal.timeout(12_000),
    },
  );
  return {
    validated: Boolean(payload.validated),
    message: payload.message || "Channel account configuration accepted.",
  };
}

export async function fetchChannelEndpoints(
  settings: DesktopSettings,
): Promise<DesktopChannelEndpoint[]> {
  const payload = await requestJson<ChannelEndpointsPayload>(
    settings,
    "/api/channel-endpoints",
    {
      signal: AbortSignal.timeout(REMOTE_STATE_FETCH_TIMEOUT_MS),
    },
  );

  return Array.isArray(payload.endpoints)
    ? payload.endpoints.map(mapChannelEndpoint)
    : [];
}

export async function bindRemoteChannelEndpoint(
  settings: DesktopSettings,
  input: {
    endpointKey: string;
    threadId: string;
  },
): Promise<void> {
  await requestJson<unknown>(settings, "/api/channel-bindings/bind", {
    method: "POST",
    signal: AbortSignal.timeout(8000),
    body: JSON.stringify(input),
  });
}

export async function detachRemoteChannelEndpoint(
  settings: DesktopSettings,
  input: {
    endpointKey: string;
  },
): Promise<void> {
  await requestJson<unknown>(settings, "/api/channel-bindings/detach", {
    method: "POST",
    signal: AbortSignal.timeout(8000),
    body: JSON.stringify(input),
  });
}
