import type { DesktopWorkspaceMode } from "./workspace.ts";

export interface DesktopChannelEndpoint {
  endpointKey: string;
  channel: string;
  accountId: string;
  peerId: string;
  chatId: string;
  deliveryTargetType: "chat_id" | "open_id";
  deliveryTargetId: string;
  threadScope?: string | null;
  displayLabel: string;
  threadId?: string | null;
  threadLabel?: string | null;
  workspacePath?: string | null;
  threadUpdatedAt?: string | null;
  lastInboundAt?: string | null;
  lastDeliveryAt?: string | null;
  conversationKind?: "private" | "group" | "topic" | "unknown" | null;
  conversationLabel?: string | null;
}

export type DesktopManagedChannel = string;

/**
 * Catalog entry for a channel, mirroring the payload gateway's
 * `GET /api/channels/plugins` returns (see
 * `garyx_channels::SubprocessPluginCatalogEntry`). Covers both
 * built-in channels (telegram / feishu / weixin, synthesized
 * server-side) and subprocess plugins — the UI treats them
 * identically.
 *
 * Drives schema-driven account configuration: the UI renders
 * `schema` as a dynamic form rather than shipping per-channel
 * hand-written panels.
 */
/**
 * One entry in a plugin's `config_methods[]` array (§11). The Mac
 * App walks the array in order and renders a UI block per entry:
 *
 *   - `form`        → render the plugin's JSON Schema as a form.
 *   - `auto_login`  → render an "Auto login" button that drives
 *                     `AuthFlowDriver`.
 *
 * Future kinds (`sso_callback`, etc.) are declared opaquely via
 * `kind: string`. UIs that don't recognise the kind MUST skip the
 * entry so newer gateways remain backward-compatible.
 */
export interface ChannelPluginConfigMethod {
  kind: string;
}

export interface ChannelPluginCatalogEntry {
  id: string;
  display_name: string;
  version: string;
  description: string;
  /** "loaded" | "initializing" | "ready" | "running" | "stopped" | "error" */
  state: string;
  last_error?: string | null;
  capabilities: {
    outbound: boolean;
    inbound: boolean;
    streaming: boolean;
    images: boolean;
    files: boolean;
    hot_reload_accounts?: boolean;
    requires_public_url?: boolean;
    needs_host_ingress?: boolean;
    delivery_model: string;
  };
  /** JSON Schema (2020-12) describing one account's config. */
  schema: Record<string, unknown>;
  auth_flows: Array<{
    id: string;
    label: string;
    prompt?: string;
  }>;
  /**
   * The configuration methods the gateway advertises for this
   * plugin. The UI MUST walk this array in order and render one
   * block per entry:
   *   - `{ kind: "form" }`      → render the `schema` above as a
   *                               JSON-Schema-driven form.
   *   - `{ kind: "auto_login" }` → render a button that invokes the
   *                               channel's auto-login flow via
   *                               `POST /api/channels/plugins/:id/
   *                               auth_flow/start` + poll. On
   *                               Confirmed, the returned values
   *                               get merged into the form above.
   *   - anything else            → unknown method; render nothing
   *                               (forward-compat with future
   *                               methods older desktops don't yet
   *                               understand).
   * Optional for backward-compat: older gateways that predate §11
   * omit this field and the desktop falls back to rendering the
   * form only.
   */
  config_methods?: ChannelPluginConfigMethod[];
  /**
   * Currently-configured accounts. `config` is projected through this entry's
   * JSON Schema by the gateway before it reaches the UI.
   */
  accounts: Array<{
    id: string;
    enabled: boolean;
    config: Record<string, unknown>;
  }>;
  /**
   * Plugin-supplied brand icon as an inline `data:` URL, ready to
   * bind to `<img src={...}>`. Populated when the plugin ships an
   * icon (`plugin.icon` in its manifest) and `garyx plugins install`
   * copied the file next to the binary. Absent when the channel does
   * not ship a branding asset.
   */
  icon_data_url?: string | null;
  account_root_behavior?: "open_default" | "expand_only";
}

export interface AddChannelAccountInput {
  /** Canonical plugin id (`telegram`, `feishu`, `weixin`, `acmechat`, ...). */
  channel: string;
  accountId: string;
  name?: string | null;
  workspaceDir?: string | null;
  workspaceMode?: DesktopWorkspaceMode;
  agentId?: string | null;
  token?: string | null;
  appId?: string | null;
  appSecret?: string | null;
  baseUrl?: string | null;
  uin?: string | null;
  /** Feishu tenant brand: `feishu` (default) | `lark`. */
  domain?: "feishu" | "lark" | null;
  /** Opaque plugin config JSON, validated by the plugin's JSON Schema on save. */
  config?: Record<string, unknown> | null;
}

export interface BindChannelEndpointInput {
  endpointKey: string;
  threadId: string;
}

export interface DetachChannelEndpointInput {
  endpointKey: string;
}
