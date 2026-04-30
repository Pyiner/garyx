export type DesktopLanguagePreference = "system" | "en" | "zh-CN";

export interface DesktopSettings {
  gatewayUrl: string;
  gatewayAuthToken: string;
  accountId: string;
  fromId: string;
  timeoutSeconds: number;
  providerClaudeEnv: string;
  providerCodexAuthMode: "cli" | "api_key";
  providerCodexApiKey: string;
  threadLogsPanelWidth: number;
  languagePreference: DesktopLanguagePreference;
}

export interface DesktopGatewayProfile {
  id: string;
  label: string;
  gatewayUrl: string;
  gatewayAuthToken: string;
  updatedAt: string;
}

export type DesktopApiProviderType =
  | "claude_code"
  | "codex_app_server"
  | "gemini_cli";

export type DesktopThreadProviderType =
  | DesktopApiProviderType
  | "agent_team";

export type DesktopWorkspaceKind = "local";

// Path-derived directory group used by the desktop UI. This is not a durable
// Garyx domain entity; the thread/automation source of truth remains
// `workspace_dir`.
export interface DesktopWorkspace {
  id: string;
  name: string;
  path: string | null;
  kind: DesktopWorkspaceKind;
  createdAt: string;
  updatedAt: string;
  available: boolean;
  managed?: boolean;
}

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

export type DesktopAutomationSchedule =
  | {
      kind: "daily";
      time: string;
      weekdays: string[];
      timezone: string;
    }
  | {
      kind: "interval";
      hours: number;
    }
  | {
      kind: "once";
      at: string;
    };

export type DesktopAutomationStatus = "success" | "failed" | "skipped";

export interface DesktopAutomationSummary {
  id: string;
  label: string;
  prompt: string;
  agentId: string;
  enabled: boolean;
  workspaceId: string;
  workspacePath: string;
  // Latest execution thread for this automation. Empty until it has run at least once.
  threadId: string;
  nextRun: string;
  lastRunAt?: string | null;
  lastStatus: DesktopAutomationStatus;
  unreadHintTimestamp?: string | null;
  schedule: DesktopAutomationSchedule;
}

export interface DesktopAutomationActivityEntry {
  runId: string;
  status: DesktopAutomationStatus;
  startedAt: string;
  finishedAt?: string | null;
  durationMs?: number | null;
  excerpt?: string | null;
  threadId: string;
}

export interface DesktopAutomationActivityFeed {
  automationId: string;
  // Latest execution thread represented by this feed page. Empty if there is no activity yet.
  threadId: string;
  count: number;
  items: DesktopAutomationActivityEntry[];
}

export interface DesktopSkillInfo {
  id: string;
  name: string;
  description: string;
  installed: boolean;
  enabled: boolean;
  sourcePath: string;
}

export interface DesktopCustomAgent {
  agentId: string;
  displayName: string;
  providerType: DesktopApiProviderType;
  model: string;
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
  createdAt: string;
  updatedAt: string;
}

export interface CreateCustomAgentInput {
  agentId: string;
  displayName: string;
  providerType: DesktopApiProviderType;
  model: string;
  systemPrompt: string;
}

export interface UpdateCustomAgentInput extends CreateCustomAgentInput {
  currentAgentId: string;
}

export interface DeleteCustomAgentInput {
  agentId: string;
}

export interface CreateTeamInput {
  teamId: string;
  displayName: string;
  leaderAgentId: string;
  memberAgentIds: string[];
  workflowText: string;
}

export interface UpdateTeamInput extends CreateTeamInput {
  currentTeamId: string;
}

export interface DeleteTeamInput {
  teamId: string;
}

export interface DesktopSkillEntryNode {
  path: string;
  name: string;
  entryType: "file" | "directory";
  children: DesktopSkillEntryNode[];
}

export interface DesktopSkillEditorState {
  skill: DesktopSkillInfo;
  entries: DesktopSkillEntryNode[];
}

export type DesktopSkillFilePreviewKind =
  | "markdown"
  | "text"
  | "image"
  | "unsupported";

export interface DesktopSkillFileDocument {
  skill: DesktopSkillInfo;
  path: string;
  content: string;
  mediaType: string;
  previewKind: DesktopSkillFilePreviewKind;
  dataBase64?: string | null;
  editable: boolean;
}

export interface DesktopWorkspaceFileEntry {
  path: string;
  name: string;
  entryType: "file" | "directory";
  size?: number | null;
  modifiedAt?: string | null;
  mediaType?: string | null;
  hasChildren: boolean;
}

export interface DesktopWorkspaceFileListing {
  workspacePath: string;
  directoryPath: string;
  entries: DesktopWorkspaceFileEntry[];
}

export type DesktopWorkspaceFilePreviewKind =
  | "markdown"
  | "html"
  | "text"
  | "pdf"
  | "image"
  | "unsupported";

export interface DesktopWorkspaceFilePreview {
  workspacePath: string;
  path: string;
  name: string;
  mediaType: string;
  previewKind: DesktopWorkspaceFilePreviewKind;
  size: number;
  modifiedAt?: string | null;
  truncated: boolean;
  text?: string | null;
  dataBase64?: string | null;
}

export type DesktopMemoryDocumentScope = "global" | "automation" | "workspace";

export interface DesktopMemoryDocument {
  scope: DesktopMemoryDocumentScope;
  automationId?: string | null;
  workspacePath?: string | null;
  path: string;
  content: string;
  exists: boolean;
  modifiedAt?: string | null;
}

export interface ReadMemoryDocumentInput {
  scope: DesktopMemoryDocumentScope;
  automationId?: string;
  workspacePath?: string;
}

export interface SaveMemoryDocumentInput extends ReadMemoryDocumentInput {
  content: string;
}

export interface SlashCommand {
  name: string;
  description: string;
  prompt?: string | null;
}

export interface UpsertSlashCommandInput {
  name: string;
  description: string;
  prompt?: string | null;
}

export interface UpdateSlashCommandInput extends UpsertSlashCommandInput {
  currentName: string;
}

export interface DeleteSlashCommandInput {
  name: string;
}

export type AutoResearchRunState =
  | "queued"
  | "researching"
  | "judging"
  | "budget_exhausted"
  | "blocked"
  | "user_stopped";

export interface CandidateVerdict {
  score: number;
  feedback: string;
}

export interface ResearchCandidate {
  candidate_id: string;
  iteration: number;
  output: string;
  verdict?: CandidateVerdict | null;
  duration_secs: number;
}

export interface DesktopAutoResearchRun {
  runId: string;
  state: AutoResearchRunState;
  stateStartedAt?: string | null;
  goal: string;
  workspaceDir?: string | null;
  maxIterations: number;
  timeBudgetSecs: number;
  iterationsUsed: number;
  createdAt: string;
  updatedAt: string;
  terminalReason?: string | null;
  candidates: ResearchCandidate[];
  selectedCandidate?: string | null;
}

export interface DesktopAutoResearchIteration {
  runId: string;
  iterationIndex: number;
  state: "researching" | "judging" | "completed";
  workThreadId?: string | null;
  verifyThreadId?: string | null;
  startedAt: string;
  completedAt?: string | null;
}

export interface DesktopAutoResearchRunDetail {
  run: DesktopAutoResearchRun;
  latestIteration?: DesktopAutoResearchIteration | null;
  activeThreadId?: string | null;
}

export interface CandidatesResponse {
  candidates: ResearchCandidate[];
  bestCandidateId: string | null;
}

export interface CreateAutoResearchRunInput {
  goal: string;
  workspaceDir?: string;
  maxIterations?: number;
  timeBudgetSecs?: number;
}

export interface SelectCandidateInput {
  runId: string;
  candidateId: string;
}

export interface ListCandidatesInput {
  runId: string;
}

export interface StopAutoResearchRunInput {
  runId: string;
  reason?: string;
}

export interface ListAutoResearchRunsInput {
  limit?: number;
}

export type McpTransportType = "stdio" | "streamable_http";

export interface DesktopMcpServer {
  name: string;
  transport: McpTransportType;
  // STDIO fields
  command: string;
  args: string[];
  env: Record<string, string>;
  workingDir?: string | null;
  // Streamable HTTP fields
  url?: string | null;
  headers?: Record<string, string>;
  // Common
  enabled: boolean;
}

export interface UpsertMcpServerInput {
  name: string;
  transport: McpTransportType;
  // STDIO fields
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  workingDir?: string | null;
  // Streamable HTTP fields
  url?: string | null;
  headers?: Record<string, string>;
  // Common
  enabled: boolean;
}

export interface UpdateMcpServerInput extends UpsertMcpServerInput {
  currentName: string;
}

export interface DeleteMcpServerInput {
  name: string;
}

export interface ToggleMcpServerInput {
  name: string;
  enabled: boolean;
}

/**
 * Team block attached to a thread when its bound agent_id resolves to a Team.
 * Mirrors the Rust response shape — field names stay in snake_case for wire
 * fidelity because they flow straight through from the gateway JSON.
 *
 * Emitted by the gateway's thread metadata endpoint (GET /api/threads/:key
 * nested under the thread object; GET /api/threads/history as a top-level
 * sibling of `thread`/`messages`) AND by the list endpoint (GET /api/threads)
 * on every team-bound summary. Absent/null when the thread isn't bound to a
 * Team. The `teamId` + `teamName` hints remain for backward compatibility
 * but the full block is now the authoritative source for team branding.
 */
export interface ThreadTeamBlock {
  team_id: string;
  display_name: string;
  leader_agent_id: string;
  member_agent_ids: string[];
  /**
   * agent_id -> child thread_id. Empty object when no sub-agent has been
   * dispatched yet. Always present when the `team` block itself is present.
   */
  child_thread_ids: Record<string, string>;
}

export interface DesktopThreadSummary {
  id: string;
  title: string;
  createdAt: string;
  updatedAt: string;
  lastMessagePreview: string;
  workspaceId: string;
  workspacePath?: string | null;
  messageCount?: number;
  agentId?: string | null;
  teamId?: string | null;
  teamName?: string | null;
  recentRunId?: string | null;
  /**
   * Full team block when this thread is bound to a Team. The gateway's list
   * endpoint (`/api/threads`) and thread metadata endpoints both supply it for
   * team threads; older snapshots cached pre-upgrade may still be missing
   * it, hence the optional typing.
   */
  team?: ThreadTeamBlock | null;
}

export interface ThreadChannelBindingInfo {
  channel: string;
  accountId: string;
  bindingKey: string;
  chatId: string;
  deliveryTargetType: string;
  deliveryTargetId: string;
  displayLabel: string;
  lastInboundAt?: string | null;
  lastDeliveryAt?: string | null;
}

export interface ThreadRuntimeInfo {
  agentId?: string | null;
  providerType?: DesktopThreadProviderType | null;
  providerLabel?: string | null;
  sdkSessionId?: string | null;
  workspacePath?: string | null;
  channelBindings: ThreadChannelBindingInfo[];
}

export interface ConfiguredBot {
  channel: string;
  accountId: string;
  displayName: string;
  enabled: boolean;
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
  workspaceDir: string | null;
  mainEndpointStatus: "resolved" | "unresolved";
  mainEndpoint: DesktopChannelEndpoint | null;
  mainThreadId: string | null;
  defaultOpenEndpoint: DesktopChannelEndpoint | null;
  defaultOpenThreadId: string | null;
  conversationNodes: DesktopBotConversationNode[];
  endpoints: DesktopChannelEndpoint[];
}

export interface DesktopRemoteStateError {
  source: "threads" | "endpoints" | "configured_bots" | "bot_consoles" | "automations";
  label: string;
  message: string;
}

export interface DesktopState {
  settings: DesktopSettings;
  gatewayProfiles: DesktopGatewayProfile[];
  workspaces: DesktopWorkspace[];
  hiddenWorkspacePaths: string[];
  selectedWorkspaceId: string | null;
  threads: DesktopThreadSummary[];
  sessions: DesktopThreadSummary[];
  endpoints: DesktopChannelEndpoint[];
  configuredBots: ConfiguredBot[];
  botConsoles: DesktopBotConsoleSummary[];
  automations: DesktopAutomationSummary[];
  selectedAutomationId: string | null;
  lastSeenRunAtByAutomation: Record<string, string>;
  botMainThreads: Record<string, string>;
  remoteErrors: DesktopRemoteStateError[];
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

export interface StartWeixinChannelAuthInput {
  accountId?: string | null;
  name?: string | null;
  workspaceDir?: string | null;
  baseUrl?: string | null;
}

export interface StartWeixinChannelAuthResult {
  sessionId: string;
  qrCodeValue: string;
  qrCodeDataUrl: string;
  status: string;
}

export interface PollWeixinChannelAuthInput {
  sessionId: string;
}

export interface PollWeixinChannelAuthResult {
  status: string;
  accountId?: string | null;
}

/**
 * Feishu / Lark OAuth 2.0 Device Authorization Grant.
 *
 * This is the same RFC 8628 flow that garyx CLI's `--auto-register` uses,
 * so the desktop and CLI both end up with the same `app_id` / `app_secret`
 * provisioning path and the user never has to hand-copy credentials out of
 * the open platform console.
 */
export interface StartFeishuChannelAuthInput {
  accountId?: string | null;
  name?: string | null;
  workspaceDir?: string | null;
  /** `feishu` for the China tenant or `lark` for the international tenant. */
  domain?: "feishu" | "lark" | null;
}

export interface StartFeishuChannelAuthResult {
  sessionId: string;
  /** Full URL to open in a browser / encode as QR for the user. */
  verificationUrl: string;
  /** Data URL of the QR rendered server-side; UI can `<img src=...>` it. */
  qrCodeDataUrl: string;
  /** Short human-readable code; show next to the QR. */
  userCode: string;
  /** Seconds until device_code expires. */
  expiresIn: number;
  /** Seconds the caller should wait between polls. */
  interval: number;
  /** Brand the flow was started against (echoes back the input for convenience). */
  domain: "feishu" | "lark";
}

export interface PollFeishuChannelAuthInput {
  sessionId: string;
}

/**
 * `pending` — keep polling.
 * `slow_down` — back off, server-requested.
 * `confirmed` — credentials written to config; `accountId` is what we stored.
 * `denied` — user declined.
 * `expired` — device_code TTL elapsed.
 */
export interface PollFeishuChannelAuthResult {
  status: "pending" | "slow_down" | "confirmed" | "denied" | "expired";
  accountId?: string | null;
  /** Populated when status === "confirmed" so UI can display a summary. */
  appId?: string | null;
  /** Echoes the tenant_brand returned by the open platform. */
  domain?: "feishu" | "lark" | null;
}

export interface ConnectionStatus {
  ok: boolean;
  bridgeReady: boolean;
  gatewayUrl: string;
  version?: string;
  uptimeSeconds?: number;
  threadCount?: number;
  sessionCount?: number;
  error?: string;
}

export interface GatewayProbeResult {
  ok: boolean;
  isGaryGateway: boolean;
  gatewayUrl: string;
  path: string;
  version?: string;
  host?: string;
  port?: number;
  error?: string;
}

export type GatewayConfigDocument = Record<string, unknown>;

export type GatewaySettingsSource = "local_file" | "gateway_api";

export interface GatewaySettingsPayload {
  config: GatewayConfigDocument;
  source: GatewaySettingsSource;
  secretsMasked: boolean;
}

export interface GatewaySettingsSaveResult {
  ok: boolean;
  message?: string;
  errors?: string[];
  settings: GatewaySettingsPayload;
}

export type TranscriptRole =
  | "assistant"
  | "system"
  | "user"
  | "tool_use"
  | "tool_result";

export interface TranscriptMessage {
  id: string;
  role: TranscriptRole;
  text: string;
  content?: unknown;
  toolUseId?: string | null;
  toolName?: string | null;
  isError?: boolean;
  metadata?: Record<string, unknown> | null;
  timestamp?: string | null;
  pending?: boolean;
  error?: boolean;
  kind?: string;
  internal?: boolean;
  internalKind?: string | null;
  loopOrigin?: string | null;
}

export interface MessageImageAttachment {
  id: string;
  name: string;
  mediaType: string;
  path?: string;
  data?: string;
}

export interface MessageFileAttachment {
  id: string;
  name: string;
  mediaType: string;
  path?: string;
  data?: string;
}

export type ChatAttachmentKind = "image" | "file";

export interface UploadChatAttachmentBlob {
  kind: ChatAttachmentKind;
  name: string;
  mediaType?: string | null;
  dataBase64: string;
}

export interface UploadedChatAttachment {
  kind: ChatAttachmentKind;
  path: string;
  name: string;
  mediaType: string;
}

export interface UploadChatAttachmentsInput {
  files: UploadChatAttachmentBlob[];
}

export interface UploadChatAttachmentsResult {
  files: UploadedChatAttachment[];
}

export interface ChatStreamToolMessage {
  role: "tool_use" | "tool_result";
  content: unknown;
  timestamp?: string | null;
  toolUseId?: string | null;
  toolName?: string | null;
  isError?: boolean;
  metadata?: Record<string, unknown> | null;
}

export type DesktopChatStreamEvent =
  // `sessionId` remains as a compatibility mirror for older stream payloads.
  | {
      type: "accepted";
      runId: string;
      threadId: string;
      sessionId?: string;
    }
  | {
      type: "assistant_delta";
      runId: string;
      threadId: string;
      sessionId?: string;
      delta: string;
      metadata?: Record<string, unknown> | null;
    }
  | {
      type: "assistant_boundary";
      runId: string;
      threadId: string;
      sessionId?: string;
    }
  | {
      type: "tool_use";
      runId: string;
      threadId: string;
      sessionId?: string;
      message: ChatStreamToolMessage;
    }
  | {
      type: "tool_result";
      runId: string;
      threadId: string;
      sessionId?: string;
      message: ChatStreamToolMessage;
    }
  | {
      type: "user_ack";
      runId: string;
      threadId: string;
      sessionId?: string;
      pendingInputId?: string;
    }
  | {
      type: "done";
      runId: string;
      threadId: string;
      sessionId?: string;
    }
  | {
      type: "error";
      runId: string;
      threadId: string;
      sessionId?: string;
      error: string;
    };

export type DesktopChatStreamListener = (event: DesktopChatStreamEvent) => void;

export type DesktopSessionProviderHint = "claude" | "codex" | "gemini";

export type DesktopDeepLinkEvent =
  | {
      type: "open-thread";
      url: string;
      threadId: string;
    }
  | {
      type: "resume-session";
      url: string;
      sessionId: string;
      providerHint?: DesktopSessionProviderHint | null;
    }
  | {
      type: "error";
      url: string;
      error: string;
    };

export type DesktopDeepLinkListener = (event: DesktopDeepLinkEvent) => void;

export interface ThreadTranscript {
  threadId: string;
  remoteFound: boolean;
  messages: TranscriptMessage[];
  pendingInputs: PendingThreadInput[];
  threadInfo?: ThreadRuntimeInfo | null;
  /**
   * Team block when this thread is bound to an AgentTeam. `null` when the
   * thread isn't a team thread. The gateway's `/api/threads/history`
   * endpoint emits this as a sibling of `thread`/`messages`.
   */
  team?: ThreadTeamBlock | null;
}

export interface PendingThreadInput {
  id: string;
  runId?: string | null;
  text: string;
  content?: unknown;
  timestamp?: string | null;
  status: "awaiting_ack" | "orphaned";
  active: boolean;
}

export interface ThreadLogChunk {
  threadId: string;
  path: string;
  text: string;
  cursor: number;
  reset: boolean;
}

export interface CreateThreadInput {
  title?: string;
  workspaceId?: string | null;
  workspacePath?: string | null;
  /** Agent or team ID. Backend resolves whether it's a team leader or custom agent. */
  agentId?: string | null;
  /** Optional Claude/Codex/Gemini provider session id to resume from. Garyx resolves the real local provider/workspace from it. */
  sdkSessionId?: string | null;
  /** Optional provider hint for sdkSessionId. Supported values are claude, codex, and gemini. */
  sdkSessionProviderHint?: DesktopSessionProviderHint | null;
}

export interface RenameThreadInput {
  threadId: string;
  // Compatibility fallback for older callers. Prefer `threadId`.
  sessionId?: string;
  title: string;
}

export interface DeleteThreadInput {
  threadId: string;
  // Compatibility fallback for older callers. Prefer `threadId`.
  sessionId?: string;
}

export interface CreateAutomationInput {
  label: string;
  prompt: string;
  agentId: string;
  workspaceId?: string;
  workspacePath?: string;
  schedule: DesktopAutomationSchedule;
}

export interface UpdateAutomationInput {
  automationId: string;
  label?: string;
  prompt?: string;
  agentId?: string;
  workspaceId?: string;
  workspacePath?: string;
  schedule?: DesktopAutomationSchedule;
  enabled?: boolean;
}

export interface DeleteAutomationInput {
  automationId: string;
}

export interface CreateSkillInput {
  id: string;
  name: string;
  description: string;
  body: string;
}

export interface UpdateSkillInput {
  skillId: string;
  name: string;
  description: string;
}

export interface ToggleSkillInput {
  skillId: string;
}

export interface DeleteSkillInput {
  skillId: string;
}

export interface GetSkillEditorInput {
  skillId: string;
}

export interface ReadSkillFileInput {
  skillId: string;
  path: string;
}

export interface SaveSkillFileInput {
  skillId: string;
  path: string;
  content: string;
}

export interface ListWorkspaceFilesInput {
  workspacePath: string;
  directoryPath?: string;
}

export interface PreviewWorkspaceFileInput {
  workspacePath: string;
  filePath: string;
}

export type RevealWorkspaceFileInput = PreviewWorkspaceFileInput;

export interface UploadWorkspaceFileBlob {
  name: string;
  mediaType?: string | null;
  dataBase64: string;
}

export interface UploadWorkspaceFilesInput {
  workspacePath: string;
  directoryPath?: string;
  files: UploadWorkspaceFileBlob[];
}

export interface UploadWorkspaceFilesResult {
  workspacePath: string;
  directoryPath: string;
  uploadedPaths: string[];
}

export interface CreateSkillEntryInput {
  skillId: string;
  path: string;
  entryType: "file" | "directory";
}

export interface DeleteSkillEntryInput {
  skillId: string;
  path: string;
}

export interface RunAutomationNowInput {
  automationId: string;
}

export interface SelectAutomationInput {
  automationId: string | null;
}

export interface MarkAutomationSeenInput {
  automationId: string;
  seenAt: string | null;
}

export interface SetBotBindingInput {
  threadId: string;
  botId: string | null;
}

export interface BindChannelEndpointInput {
  endpointKey: string;
  threadId: string;
}

export interface DetachChannelEndpointInput {
  endpointKey: string;
}

export interface SelectWorkspaceInput {
  workspaceId: string | null;
}

export interface RemoveWorkspaceInput {
  workspaceId: string;
}

export interface RelinkWorkspaceInput {
  workspaceId: string;
}

export interface RenameWorkspaceInput {
  workspaceId: string;
  name: string;
}

export interface SendMessageInput {
  threadId: string;
  // Compatibility fallback for older callers. Prefer `threadId`.
  sessionId?: string;
  message: string;
  images?: MessageImageAttachment[];
  files?: MessageFileAttachment[];
}

export interface WorkspaceMutationResult {
  state: DesktopState;
  workspace: DesktopWorkspace | null;
  cancelled: boolean;
}

export interface OpenChatStreamResult {
  runId: string;
  threadId: string;
  // Compatibility mirror for older responses. Prefer `threadId`.
  sessionId?: string;
  response: string;
  status: "completed" | "disconnected";
  thread: DesktopThreadSummary;
  // Compatibility mirror for older responses. Prefer `thread`.
  session?: DesktopThreadSummary;
}

export interface SendStreamingInputResult {
  status: string;
  threadId: string;
  // Compatibility mirror for older responses. Prefer `threadId`.
  sessionId?: string;
  pendingInputId?: string;
}

export interface InterruptResult {
  status: string;
  threadId: string;
  // Compatibility mirror for older responses. Prefer `threadId`.
  sessionId?: string;
  abortedRuns: string[];
}

export interface AddWorkspaceByPathInput {
  path: string;
}

export interface DesktopBrowserDebugEndpoint {
  origin: string;
  versionUrl: string;
  listUrl: string;
  port: number;
}

export interface DesktopBrowserTab {
  id: string;
  title: string;
  url: string;
  isActive: boolean;
  isLoading: boolean;
  canGoBack: boolean;
  canGoForward: boolean;
}

export interface DesktopBrowserState {
  tabs: DesktopBrowserTab[];
  activeTabId: string | null;
  debugEndpoint: DesktopBrowserDebugEndpoint;
  partition: string;
}

export interface CreateBrowserTabInput {
  url?: string;
}

export interface NavigateBrowserTabInput {
  tabId: string;
  url: string;
}

export interface BrowserBoundsInput {
  x: number;
  y: number;
  width: number;
  height: number;
  visible: boolean;
}

export interface ShowBrowserConnectionMenuInput {
  x: number;
  y: number;
  labels?: {
    copyCdpEndpoint?: string;
    copyCdpListUrl?: string;
  };
}

export type DesktopBrowserStateListener = (state: DesktopBrowserState) => void;

export interface GaryxDesktopApi {
  getState: () => Promise<DesktopState>;
  saveSettings: (settings: DesktopSettings) => Promise<DesktopState>;
  rememberGatewayProfile: () => Promise<DesktopState>;
  getGatewaySettings: () => Promise<GatewaySettingsPayload>;
  fetchChannelPlugins: () => Promise<ChannelPluginCatalogEntry[]>;
  openExternalUrl: (input: { url: string }) => Promise<void>;
  /**
   * Start a channel-blind auto-login flow against the gateway. The
   * renderer supplies the plugin id (canonical or alias) and the
   * current form state; the plugin decides internally what "auto
   * login" means. Returns the initial `AuthSession` with a display
   * list to render and a poll cadence.
   */
  startChannelAuthFlow: (input: {
    pluginId: string;
    formState?: Record<string, unknown>;
  }) => Promise<{
    sessionId: string;
    display: Array<{ kind: string; value?: string }>;
    expiresInSecs: number;
    pollIntervalSecs: number;
  }>;
  /**
   * Advance a running auth-flow session by one tick. Returns the
   * raw 3-state poll result — `pending` / `confirmed` / `failed` —
   * plus optional display refresh and backoff hint.
   */
  pollChannelAuthFlow: (input: {
    pluginId: string;
    sessionId: string;
  }) => Promise<{
    status: "pending" | "confirmed" | "failed" | string;
    display?: Array<{ kind: string; value?: string }>;
    next_interval_secs?: number;
    values?: Record<string, unknown>;
    reason?: string;
  }>;
  saveGatewaySettings: (
    config: GatewayConfigDocument,
  ) => Promise<GatewaySettingsSaveResult>;
  selectWorkspace: (input: SelectWorkspaceInput) => Promise<DesktopState>;
  addWorkspace: () => Promise<WorkspaceMutationResult>;
  pickDirectory: (input?: {
    defaultPath?: string | null;
  }) => Promise<string | null>;
  addWorkspaceByPath: (
    input: AddWorkspaceByPathInput,
  ) => Promise<WorkspaceMutationResult>;
  relinkWorkspace: (
    input: RelinkWorkspaceInput,
  ) => Promise<WorkspaceMutationResult>;
  renameWorkspace: (input: RenameWorkspaceInput) => Promise<DesktopState>;
  removeWorkspace: (input: RemoveWorkspaceInput) => Promise<DesktopState>;
  selectAutomation: (input: SelectAutomationInput) => Promise<DesktopState>;
  markAutomationSeen: (input: MarkAutomationSeenInput) => Promise<DesktopState>;
  createAutomation: (
    input: CreateAutomationInput,
  ) => Promise<{ state: DesktopState; automation: DesktopAutomationSummary }>;
  updateAutomation: (
    input: UpdateAutomationInput,
  ) => Promise<{ state: DesktopState; automation: DesktopAutomationSummary }>;
  deleteAutomation: (input: DeleteAutomationInput) => Promise<DesktopState>;
  listSkills: () => Promise<DesktopSkillInfo[]>;
  listCustomAgents: () => Promise<DesktopCustomAgent[]>;
  createCustomAgent: (
    input: CreateCustomAgentInput,
  ) => Promise<DesktopCustomAgent>;
  updateCustomAgent: (
    input: UpdateCustomAgentInput,
  ) => Promise<DesktopCustomAgent>;
  deleteCustomAgent: (input: DeleteCustomAgentInput) => Promise<void>;
  listTeams: () => Promise<DesktopTeam[]>;
  createTeam: (input: CreateTeamInput) => Promise<DesktopTeam>;
  updateTeam: (input: UpdateTeamInput) => Promise<DesktopTeam>;
  deleteTeam: (input: DeleteTeamInput) => Promise<void>;
  createSkill: (input: CreateSkillInput) => Promise<DesktopSkillInfo>;
  updateSkill: (input: UpdateSkillInput) => Promise<DesktopSkillInfo>;
  toggleSkill: (input: ToggleSkillInput) => Promise<DesktopSkillInfo>;
  deleteSkill: (input: DeleteSkillInput) => Promise<void>;
  getSkillEditor: (
    input: GetSkillEditorInput,
  ) => Promise<DesktopSkillEditorState>;
  readSkillFile: (
    input: ReadSkillFileInput,
  ) => Promise<DesktopSkillFileDocument>;
  saveSkillFile: (
    input: SaveSkillFileInput,
  ) => Promise<DesktopSkillFileDocument>;
  readMemoryDocument: (
    input: ReadMemoryDocumentInput,
  ) => Promise<DesktopMemoryDocument>;
  saveMemoryDocument: (
    input: SaveMemoryDocumentInput,
  ) => Promise<DesktopMemoryDocument>;
  listWorkspaceFiles: (
    input: ListWorkspaceFilesInput,
  ) => Promise<DesktopWorkspaceFileListing>;
  previewWorkspaceFile: (
    input: PreviewWorkspaceFileInput,
  ) => Promise<DesktopWorkspaceFilePreview>;
  revealWorkspaceFile: (
    input: RevealWorkspaceFileInput,
  ) => Promise<void>;
  uploadChatAttachments: (
    input: UploadChatAttachmentsInput,
  ) => Promise<UploadChatAttachmentsResult>;
  uploadWorkspaceFiles: (
    input: UploadWorkspaceFilesInput,
  ) => Promise<UploadWorkspaceFilesResult>;
  createSkillEntry: (
    input: CreateSkillEntryInput,
  ) => Promise<DesktopSkillEditorState>;
  deleteSkillEntry: (
    input: DeleteSkillEntryInput,
  ) => Promise<DesktopSkillEditorState>;
  listSlashCommands: () => Promise<SlashCommand[]>;
  createSlashCommand: (input: UpsertSlashCommandInput) => Promise<SlashCommand>;
  updateSlashCommand: (input: UpdateSlashCommandInput) => Promise<SlashCommand>;
  deleteSlashCommand: (input: DeleteSlashCommandInput) => Promise<void>;
  listAutoResearchRuns: (
    input?: ListAutoResearchRunsInput,
  ) => Promise<DesktopAutoResearchRun[]>;
  createAutoResearchRun: (
    input: CreateAutoResearchRunInput,
  ) => Promise<DesktopAutoResearchRun>;
  getAutoResearchRun: (runId: string) => Promise<DesktopAutoResearchRunDetail>;
  listAutoResearchIterations: (
    runId: string,
  ) => Promise<DesktopAutoResearchIteration[]>;
  stopAutoResearchRun: (
    input: StopAutoResearchRunInput,
  ) => Promise<DesktopAutoResearchRun>;
  deleteAutoResearchRun: (runId: string) => Promise<void>;
  listAutoResearchCandidates: (
    input: ListCandidatesInput,
  ) => Promise<CandidatesResponse>;
  selectAutoResearchCandidate: (
    input: SelectCandidateInput,
  ) => Promise<DesktopAutoResearchRun>;
  listMcpServers: () => Promise<DesktopMcpServer[]>;
  createMcpServer: (input: UpsertMcpServerInput) => Promise<DesktopMcpServer>;
  updateMcpServer: (input: UpdateMcpServerInput) => Promise<DesktopMcpServer>;
  deleteMcpServer: (input: DeleteMcpServerInput) => Promise<void>;
  toggleMcpServer: (input: ToggleMcpServerInput) => Promise<DesktopMcpServer>;
  getAutomationActivity: (
    automationId: string,
  ) => Promise<DesktopAutomationActivityFeed>;
  runAutomationNow: (input: RunAutomationNowInput) => Promise<{
    state: DesktopState;
    activity: DesktopAutomationActivityEntry;
  }>;
  addChannelAccount: (input: AddChannelAccountInput) => Promise<DesktopState>;
  startWeixinChannelAuth: (
    input: StartWeixinChannelAuthInput,
  ) => Promise<StartWeixinChannelAuthResult>;
  pollWeixinChannelAuth: (
    input: PollWeixinChannelAuthInput,
  ) => Promise<PollWeixinChannelAuthResult>;
  startFeishuChannelAuth: (
    input: StartFeishuChannelAuthInput,
  ) => Promise<StartFeishuChannelAuthResult>;
  pollFeishuChannelAuth: (
    input: PollFeishuChannelAuthInput,
  ) => Promise<PollFeishuChannelAuthResult>;
  setBotBinding: (input: SetBotBindingInput) => Promise<DesktopState>;
  listChannelEndpoints: () => Promise<DesktopChannelEndpoint[]>;
  bindChannelEndpoint: (
    input: BindChannelEndpointInput,
  ) => Promise<DesktopState>;
  detachChannelEndpoint: (
    input: DetachChannelEndpointInput,
  ) => Promise<DesktopState>;
  createThread: (input?: CreateThreadInput) => Promise<{
    state: DesktopState;
    thread: DesktopThreadSummary;
    session?: DesktopThreadSummary;
  }>;
  renameThread: (input: RenameThreadInput) => Promise<DesktopState>;
  deleteThread: (input: DeleteThreadInput) => Promise<DesktopState>;
  getThreadHistory: (threadId: string) => Promise<ThreadTranscript>;
  getThreadLogs: (threadId: string, cursor?: number) => Promise<ThreadLogChunk>;
  openChatStream: (input: SendMessageInput) => Promise<OpenChatStreamResult>;
  sendStreamingInput: (
    input: SendMessageInput,
  ) => Promise<SendStreamingInputResult>;
  subscribeChatStream: (listener: DesktopChatStreamListener) => void;
  unsubscribeChatStream: (listener: DesktopChatStreamListener) => void;
  subscribeDeepLinks: (listener: DesktopDeepLinkListener) => void;
  unsubscribeDeepLinks: (listener: DesktopDeepLinkListener) => void;
  interruptThread: (threadId: string) => Promise<InterruptResult>;
  checkConnection: (input?: {
    gatewayUrl?: string;
    gatewayAuthToken?: string;
  }) => Promise<ConnectionStatus>;
  probeGateway: (input: {
    gatewayUrl: string;
    gatewayAuthToken: string;
  }) => Promise<GatewayProbeResult>;
  listBrowserState: () => Promise<DesktopBrowserState>;
  createBrowserTab: (
    input?: CreateBrowserTabInput,
  ) => Promise<DesktopBrowserState>;
  activateBrowserTab: (tabId: string) => Promise<DesktopBrowserState>;
  closeBrowserTab: (tabId: string) => Promise<DesktopBrowserState>;
  navigateBrowserTab: (
    input: NavigateBrowserTabInput,
  ) => Promise<DesktopBrowserState>;
  browserGoBack: (tabId: string) => Promise<DesktopBrowserState>;
  browserGoForward: (tabId: string) => Promise<DesktopBrowserState>;
  browserReload: (tabId: string) => Promise<DesktopBrowserState>;
  browserOpenExternal: (tabId: string) => Promise<void>;
  updateBrowserBounds: (input: BrowserBoundsInput) => Promise<void>;
  setBrowserOverlayPaused: (paused: boolean) => Promise<void>;
  showBrowserConnectionMenu: (
    input: ShowBrowserConnectionMenuInput,
  ) => Promise<void>;
  subscribeBrowserState: (listener: DesktopBrowserStateListener) => void;
  unsubscribeBrowserState: (listener: DesktopBrowserStateListener) => void;
  getUpdateStatus: () => Promise<DesktopUpdateStatus>;
  checkForUpdatesNow: () => Promise<DesktopUpdateCheckResult>;
  installUpdate: () => Promise<DesktopUpdateInstallResult>;
  subscribeUpdateStatus: (listener: DesktopUpdateStatusListener) => void;
  unsubscribeUpdateStatus: (listener: DesktopUpdateStatusListener) => void;
}

export interface DesktopUpdateInfo {
  version: string;
  releaseNotes?: string;
  releaseName?: string;
}

export type DesktopUpdateStatus =
  | { phase: "idle" }
  | { phase: "checking" }
  | { phase: "available"; info: DesktopUpdateInfo }
  | { phase: "downloading"; percent: number }
  | { phase: "downloaded"; info: DesktopUpdateInfo }
  | { phase: "error"; message: string };

export type DesktopUpdateCheckResult =
  | { ok: true }
  | { ok: false; reason: string };

export type DesktopUpdateInstallResult =
  | { ok: true }
  | { ok: false; reason: string };

export type DesktopUpdateStatusListener = (status: DesktopUpdateStatus) => void;

export const DEFAULT_SESSION_TITLE = "Fresh Thread";
export const DEFAULT_DESKTOP_SETTINGS: DesktopSettings = {
  gatewayUrl: "http://127.0.0.1:31337",
  gatewayAuthToken: "",
  accountId: "main",
  fromId: "mac-desktop",
  timeoutSeconds: 120,
  providerClaudeEnv: "",
  providerCodexAuthMode: "cli",
  providerCodexApiKey: "",
  threadLogsPanelWidth: 360,
  languagePreference: "system",
};
