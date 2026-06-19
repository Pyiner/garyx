export type DesktopLanguagePreference = "system" | "en" | "zh-CN";
export type DesktopFollowUpBehavior = "queue" | "steer";

export interface DesktopSettings {
  gatewayUrl: string;
  gatewayAuthToken: string;
  accountId: string;
  fromId: string;
  timeoutSeconds: number;
  providerClaudeEnv: string;
  providerCodexAuthMode: "cli" | "api_key";
  providerCodexApiKey: string;
  providerGeminiEnv: string;
  threadLogsPanelWidth: number;
  languagePreference: DesktopLanguagePreference;
  followUpBehavior: DesktopFollowUpBehavior;
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
  | "traex"
  | "gemini_cli"
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
  source: string;
  error?: string | null;
}

export type DesktopThreadProviderType =
  | DesktopApiProviderType
  | "agent_team";

export type DesktopWorkspaceKind = "local";

// Directory summary used by the desktop UI. The path string is the identity;
// thread/automation source of truth remains `workspace_dir`.
export interface DesktopWorkspace {
  name: string;
  path: string | null;
  kind: DesktopWorkspaceKind;
  createdAt: string;
  updatedAt: string;
  available: boolean;
  managed?: boolean;
}

export interface DesktopLocalDirectoryEntry {
  name: string;
  path: string;
}

export interface DesktopLocalDirectoryListing {
  path: string;
  parentPath: string | null;
  entries: DesktopLocalDirectoryEntry[];
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
  workspacePath: string;
  // Existing thread this automation pushes scheduled prompts into, when set.
  targetThreadId: string;
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

export type DesktopTaskStatus = "todo" | "in_progress" | "in_review" | "done";

export type DesktopTaskPrincipal =
  | {
      kind: "human";
      userId: string;
    }
  | {
      kind: "agent";
      agentId: string;
    };

export interface DesktopTaskSummary {
  threadId: string;
  taskId: string;
  number: number;
  title: string;
  status: DesktopTaskStatus;
  creator: DesktopTaskPrincipal;
  assignee?: DesktopTaskPrincipal | null;
  source?: DesktopTaskSource | null;
  executor?: DesktopTaskExecutor | null;
  updatedAt: string;
  updatedBy: DesktopTaskPrincipal;
  runtimeAgentId: string;
  replyCount: number;
}

export type DesktopTaskExecutor =
  | { type: 'agent'; agentId: string }
  | { type: 'team'; teamId: string }
  | { type: 'workflow'; workflowId: string; workflowVersion?: number | null };

export interface DesktopTaskSource {
  threadId?: string | null;
  taskId?: string | null;
  taskThreadId?: string | null;
  botId?: string | null;
  channel?: string | null;
  accountId?: string | null;
}

export interface DesktopTasksPage {
  tasks: DesktopTaskSummary[];
  total: number;
  hasMore: boolean;
}

export interface DesktopDreamSpan {
  spanId: string;
  dreamId: string;
  threadId: string;
  workspacePath?: string | null;
  startSeq: number;
  endSeq: number;
  startAt: string;
  endAt: string;
  excerpt: string;
  messageCount: number;
}

export interface DesktopDreamTopic {
  dreamId: string;
  title: string;
  summary: string;
  firstMessageAt: string;
  lastMessageAt: string;
  updatedAt: string;
  source: string;
  confidence: number;
  messageCount: number;
  spanCount: number;
  spans: DesktopDreamSpan[];
}

export interface DesktopDreamScan {
  runId: string;
  scannedFrom: string;
  scannedTo: string;
  createdAt: string;
  source: string;
  status: string;
  topicsCount: number;
  spansCount: number;
  error?: string | null;
}

export interface DesktopDreamsPage {
  dreams: DesktopDreamTopic[];
  count: number;
  from: string;
  to: string;
  latestScan?: DesktopDreamScan | null;
  scan?: DesktopDreamScan | null;
}

export interface ListDreamsInput {
  from?: string | null;
  to?: string | null;
  sinceHours?: number;
  limit?: number;
}

export interface ScanDreamsInput extends ListDreamsInput {
  mode?: "auto" | "claude" | "heuristic";
}

export interface ListTasksInput {
  status?: DesktopTaskStatus | null;
  assignee?: string | null;
  sourceThread?: string | null;
  sourceBot?: string | null;
  includeDone?: boolean;
  limit?: number;
  offset?: number;
}

export type DesktopTaskNotificationTarget =
  | { kind: "none" }
  | { kind: "bot"; channel: string; accountId: string };

export interface CreateTaskInput {
  title?: string | null;
  body?: string | null;
  executor?: CreateTaskExecutorInput | null;
  /** @deprecated New task creation should use `executor`. */
  assignee?: string | null;
  start?: boolean;
  workspaceDir?: string | null;
  workspaceMode?: DesktopWorkspaceMode;
  notificationTarget: DesktopTaskNotificationTarget;
}

export type CreateTaskExecutorInput =
  | { type: 'agent'; agentId: string }
  | { type: 'team'; teamId: string }
  | { type: 'workflow'; workflowId: string; input?: unknown };

export interface UpdateTaskStatusInput {
  taskId: string;
  status: DesktopTaskStatus;
  note?: string | null;
  force?: boolean;
}

export interface AssignTaskInput {
  taskId: string;
  principal: string;
}

export interface UnassignTaskInput {
  taskId: string;
}

export interface StopTaskInput {
  taskId: string;
}

export interface DeleteTaskInput {
  taskId: string;
}

export interface UpdateTaskTitleInput {
  taskId: string;
  title: string;
}

/**
 * Reusable workflow definition discovered by the gateway under
 * `~/.garyx/workflows`. Mirrors `workflow_definition_package_json`. The list is
 * empty until a package is installed with `garyx workflow definition upsert`.
 */
export interface DesktopWorkflowDefinition {
  workflowId: string;
  version: number;
  name: string;
  description: string;
  /** Text-input metadata for product surfaces. Not a generated form schema. */
  input?: Record<string, unknown> | null;
  defaults?: Record<string, unknown> | null;
  packageDir?: string | null;
  createdAt?: string | null;
  updatedAt?: string | null;
}

export interface DesktopWorkflowSourceDocument {
  workflowId: string;
  path: string;
  content: string;
  mediaType: string;
  language: string;
}

export type DesktopWorkflowRunStatus =
  | "running"
  | "succeeded"
  | "failed"
  | "cancelled"
  | string;

/** Mirrors `workflow_run_json`. Token/cost counters are roll-ups across children. */
export interface DesktopWorkflowRun {
  workflowRunId: string;
  threadId: string;
  /** Legacy run-id alias kept while gateway payloads migrate. */
  workflowId: string;
  taskId?: string | null;
  taskThreadId?: string | null;
  parentThreadId?: string | null;
  name?: string | null;
  description?: string | null;
  status: DesktopWorkflowRunStatus;
  currentPhaseIndex?: number | null;
  meta?: Record<string, unknown> | null;
  input?: unknown | null;
  outputText?: string | null;
  error?: string | null;
  workspaceDir?: string | null;
  totalChildren: number;
  completedChildren: number;
  failedChildren: number;
  totalInputTokens: number;
  totalOutputTokens: number;
  totalToolCalls: number;
  totalCostUsd: number;
  createdAt?: string | null;
  startedAt?: string | null;
  finishedAt?: string | null;
  updatedAt?: string | null;
}

/** Mirrors `workflow_child_json` — one dispatched child agent run. */
export interface DesktopWorkflowChild {
  workflowChildRunId: string;
  workflowRunId?: string | null;
  workflowId: string;
  threadId?: string | null;
  phaseIndex?: number | null;
  phaseTitle?: string | null;
  label?: string | null;
  agentId?: string | null;
  status: DesktopWorkflowRunStatus;
  prompt?: string | null;
  resultMode?: string | null;
  schema?: unknown | null;
  resultText?: string | null;
  result?: unknown | null;
  resultPreview?: string | null;
  error?: string | null;
  inputTokens: number;
  outputTokens: number;
  toolCalls: number;
  costUsd: number;
  queuedAt?: string | null;
  startedAt?: string | null;
  finishedAt?: string | null;
  updatedAt?: string | null;
}

/** Mirrors `workflow_event_json` — one durable run event. */
export interface DesktopWorkflowEvent {
  eventSeq: number;
  eventType: string;
  workflowRunId?: string | null;
  workflowChildRunId?: string | null;
  threadId?: string | null;
  payload?: unknown;
  createdAt?: string | null;
}

export interface DesktopWorkflowRunDrilldown {
  workflow: DesktopWorkflowRun;
  children: DesktopWorkflowChild[];
  events: DesktopWorkflowEvent[];
}

export interface DesktopWorkflowRunsPage {
  taskId: string;
  workflowRuns: DesktopWorkflowRunDrilldown[];
  count: number;
  hasMore: boolean;
}

export interface ListTaskWorkflowRunsInput {
  taskId: string;
  limit?: number;
}

export interface GetWorkflowRunInput {
  workflowRunId: string;
}

export interface GetWorkflowDefinitionSourceInput {
  workflowId: string;
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

export type DesktopMemoryDocumentScope = "agent" | "automation";

export interface DesktopMemoryDocument {
  scope: DesktopMemoryDocumentScope;
  agentId?: string | null;
  automationId?: string | null;
  path: string;
  content: string;
  exists: boolean;
  modifiedAt?: string | null;
}

export interface ReadMemoryDocumentInput {
  scope: DesktopMemoryDocumentScope;
  agentId?: string;
  automationId?: string;
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
 * Emitted by thread detail/history responses (GET /api/threads/:key nested
 * under the thread object; GET /api/threads/history as a top-level sibling of
 * `thread`/`messages`). The thread list stays lightweight and does not fetch
 * this block. Absent/null when the thread isn't bound to a Team. The `teamId`
 * + `teamName` hints remain for backward compatibility but the full block is
 * the authoritative source for team branding once details are loaded.
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
  threadType?: string | null;
  createdAt: string;
  updatedAt: string;
  lastMessagePreview: string;
  workspacePath?: string | null;
  messageCount?: number;
  agentId?: string | null;
  teamId?: string | null;
  teamName?: string | null;
  recentRunId?: string | null;
  worktree?: ThreadWorktreeInfo | null;
  /**
   * Full team block when this thread is bound to a Team. It is filled by
   * thread detail/history responses; list-only snapshots may omit it.
   */
  team?: ThreadTeamBlock | null;
}

export interface ThreadWorktreeInfo {
  mode?: string | null;
  enabled?: boolean | null;
  branch?: string | null;
  sourceBranch?: string | null;
  path?: string | null;
  worktreeDir?: string | null;
  sourceWorkspaceDir?: string | null;
  sourceRepoRoot?: string | null;
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
  model?: string | null;
  modelReasoningEffort?: string | null;
  modelServiceTier?: string | null;
  modelOverride?: string | null;
  modelReasoningEffortOverride?: string | null;
  modelServiceTierOverride?: string | null;
  sdkSessionId?: string | null;
  workspacePath?: string | null;
  worktree?: ThreadWorktreeInfo | null;
  activeRun?: ThreadActiveRunInfo | null;
  channelBindings: ThreadChannelBindingInfo[];
}

export interface ThreadActiveRunInfo {
  runId: string;
  providerType?: DesktopThreadProviderType | null;
  providerLabel?: string | null;
  assistantResponse?: string | null;
  updatedAt?: string | null;
  pendingUserInputCount?: number;
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
  source: "threads" | "thread_pins" | "endpoints" | "workspaces" | "configured_bots" | "bot_consoles" | "automations";
  label: string;
  message: string;
}

export interface DesktopState {
  settings: DesktopSettings;
  gatewayProfiles: DesktopGatewayProfile[];
  /** Gateway URL the entity slices below were loaded from. Slices from a
   *  different gateway are dropped on hydrate instead of leaking into the
   *  newly selected gateway's view. */
  entitiesGatewayUrl?: string | null;
  workspaces: DesktopWorkspace[];
  selectedWorkspacePath: string | null;
  pinnedThreadIds: string[];
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

export interface GatewaySettingsSaveRequestOptions {
  merge?: boolean;
}

export type TranscriptRole =
  | "assistant"
  | "system"
  | "user"
  | "tool"
  | "tool_use"
  | "tool_result";

export interface TranscriptMessage {
  id: string;
  role: TranscriptRole;
  text: string;
  content?: unknown;
  input?: unknown;
  result?: unknown;
  toolUseId?: string | null;
  toolName?: string | null;
  toolRelated?: boolean | null;
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

export type DesktopChatStreamEvent =
  | {
      type: "committed_message";
      runId: string;
      threadId: string;
      sessionId?: string;
      seq: number;
      message: TranscriptMessage;
    }
  | {
      type: "error";
      runId: string;
      threadId: string;
      sessionId?: string;
      error: string;
      terminal?: boolean;
    };

export type DesktopChatStreamListener = (event: DesktopChatStreamEvent) => void;

export type DesktopSessionProviderHint = "claude" | "codex" | "gemini";

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

export type DesktopDeepLinkEvent =
  | {
      type: "open-thread";
      url: string;
      threadId: string;
    }
  | {
      type: "new-thread";
      url: string;
      workspacePath?: string | null;
      agentId?: string | null;
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
  thread?: DesktopThreadSummary | null;
  threadInfo?: ThreadRuntimeInfo | null;
  pageInfo?: ThreadTranscriptPageInfo | null;
  /**
   * Team block when this thread is bound to an AgentTeam. `null` when the
   * thread isn't a team thread. The gateway's `/api/threads/history`
   * endpoint emits this as a sibling of `thread`/`messages`.
   */
  team?: ThreadTeamBlock | null;
}

export interface ThreadTranscriptPageInfo {
  totalMessages: number;
  committedMessages?: number | null;
  returnedMessages: number;
  returnedUserQueries?: number | null;
  startIndex: number;
  endIndex: number;
  hasMoreBefore: boolean;
  nextBeforeIndex?: number | null;
  hasMoreAfter?: boolean;
  nextAfterIndex?: number | null;
  reset?: boolean;
  limit: number;
  userQueryLimit?: number | null;
}

export interface GetThreadHistoryInput {
  threadId: string;
  beforeIndex?: number | null;
  afterIndex?: number | null;
  limit?: number | null;
  userQueryLimit?: number | null;
}

export interface StartThreadStreamInput {
  threadId: string;
  afterSeq?: number | null;
  consumerId?: string | null;
}

export interface StopThreadStreamInput {
  threadId?: string | null;
  consumerId?: string | null;
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
  workspacePath?: string | null;
  workspaceMode?: DesktopWorkspaceMode;
  /** Agent or team ID. Backend resolves whether it's a team leader or custom agent. */
  agentId?: string | null;
  /** Optional per-thread model override; wins over the agent's configured model. */
  model?: string | null;
  /** Optional per-thread reasoning/thinking level override. */
  modelReasoningEffort?: string | null;
  /** Optional per-thread service tier override. */
  modelServiceTier?: string | null;
  /** Optional Claude/Codex/Gemini provider session id to resume from. Garyx resolves the real local provider/workspace from it. */
  sdkSessionId?: string | null;
  /** Optional provider hint for sdkSessionId. Supported values are claude, codex, and gemini. */
  sdkSessionProviderHint?: DesktopSessionProviderHint | null;
  /** Optional Garyx thread id to fork from using the provider-native session fork. */
  forkFromThreadId?: string | null;
  /** Optional thread metadata forwarded to the gateway. */
  metadata?: Record<string, unknown> | null;
}

export interface StartWorkflowThreadInput {
  workflowId: string;
  input?: unknown;
  workspacePath?: string | null;
  workspaceMode?: DesktopWorkspaceMode;
  name?: string | null;
  description?: string | null;
}

export interface StartWorkflowThreadResult {
  state: DesktopState;
  thread: DesktopThreadSummary;
  workflowRunId: string;
  dispatch?: unknown;
  workflowDefinition?: DesktopWorkflowDefinition | null;
}

export type DesktopWorkspaceMode = "local" | "worktree";

export interface DesktopWorkspaceGitStatus {
  workspaceDir: string;
  isGitRepo: boolean;
  repoRoot?: string | null;
  currentBranch?: string | null;
  isDirty: boolean;
}

export interface DesktopWorkspaceGitFile {
  path: string;
  status: string;
}

export interface DesktopWorkspaceGitDetails extends DesktopWorkspaceGitStatus {
  ahead: number;
  behind: number;
  changedCount: number;
  stagedCount: number;
  unstagedCount: number;
  untrackedCount: number;
  files: DesktopWorkspaceGitFile[];
}

export interface CommitWorkspaceChangesInput {
  workspacePath: string;
  message: string;
}

export interface PushWorkspaceBranchInput {
  workspacePath: string;
}

export interface WorkspaceGitMutationResult {
  status: DesktopWorkspaceGitDetails;
  output: string;
}

export interface RenameThreadInput {
  threadId: string;
  // Compatibility fallback for older callers. Prefer `threadId`.
  sessionId?: string;
  title: string;
}

export interface UpdateThreadRuntimeSettingsInput {
  threadId: string;
  model?: string | null;
  modelReasoningEffort?: string | null;
  modelServiceTier?: string | null;
}

export interface DeleteThreadInput {
  threadId: string;
  // Compatibility fallback for older callers. Prefer `threadId`.
  sessionId?: string;
}

export interface SetThreadPinnedInput {
  threadId: string;
  pinned: boolean;
}

export interface CreateAutomationInput {
  label: string;
  prompt: string;
  agentId: string;
  workspacePath?: string;
  targetThreadId?: string | null;
  schedule: DesktopAutomationSchedule;
}

export interface UpdateAutomationInput {
  automationId: string;
  label?: string;
  prompt?: string;
  agentId?: string;
  workspacePath?: string;
  targetThreadId?: string | null;
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
  workspacePath: string | null;
}

export interface RemoveWorkspaceInput {
  workspacePath: string;
}

export interface SendMessageInput {
  threadId: string;
  // Compatibility fallback for older callers. Prefer `threadId`.
  sessionId?: string;
  // Stable frontend identity for queued/in-flight user intents.
  clientIntentId?: string;
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
  status: "accepted" | "completed" | "disconnected";
  thread: DesktopThreadSummary;
  // Compatibility mirror for older responses. Prefer `thread`.
  session?: DesktopThreadSummary;
}

export interface SendStreamingInputResult {
  status: string;
  threadId: string;
  // Compatibility mirror for older responses. Prefer `threadId`.
  sessionId?: string;
  clientIntentId?: string;
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

export interface CaptureBrowserTabInput {
  tabId: string;
  copyToClipboard?: boolean;
}

export interface CaptureBrowserTabResult {
  dataUrl: string;
  height: number;
  mediaType: "image/png";
  title: string;
  width: number;
}

export interface BrowserAnnotationModeInput {
  tabId: string;
  enabled: boolean;
}

export interface BrowserAnnotationCommentRequest {
  id: string;
  tabId: string;
  url: string;
  title: string;
  comment: string;
  tagName: string;
  label: string;
  markerNumber?: number | null;
  role?: string | null;
  selector?: string | null;
  text?: string | null;
  rect: {
    x: number;
    y: number;
    width: number;
    height: number;
  };
  screenshot?: CaptureBrowserTabResult | null;
}

export interface CopyImageToClipboardInput {
  dataUrl: string;
}

export interface CopyTextToClipboardInput {
  text: string;
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

export type DesktopBrowserAnnotationCommentListener = (
  request: BrowserAnnotationCommentRequest,
) => void;

export type DesktopBrowserPageMouseDownListener = () => void;

export interface DesktopTerminalSession {
  id: string;
  title: string;
  cwd: string;
  output: string;
  running: boolean;
  createdAt: string;
  updatedAt: string;
  exitCode: number | null;
  exitSignal: string | null;
}

export interface DesktopTerminalState {
  activeSessionId: string | null;
  sessions: DesktopTerminalSession[];
}

export interface CreateTerminalSessionInput {
  cwd?: string | null;
  title?: string | null;
  cols?: number | null;
  rows?: number | null;
}

export interface TerminalSessionInput {
  sessionId: string;
}

export interface TerminalWriteInput extends TerminalSessionInput {
  data: string;
}

export interface TerminalResizeInput extends TerminalSessionInput {
  cols: number;
  rows: number;
}

export type DesktopTerminalEvent =
  | {
      type: "state";
      state: DesktopTerminalState;
    }
  | {
      type: "output";
      sessionId: string;
      data: string;
    };

export type DesktopTerminalEventListener = (event: DesktopTerminalEvent) => void;

export interface GaryxDesktopApi {
  getState: () => Promise<DesktopState>;
  saveSettings: (settings: DesktopSettings) => Promise<DesktopState>;
  rememberGatewayProfile: () => Promise<DesktopState>;
  addGatewayProfile: (input: {
    label?: string;
    gatewayUrl: string;
    gatewayAuthToken?: string;
  }) => Promise<DesktopState>;
  updateGatewayProfile: (input: {
    profileId: string;
    label?: string;
    gatewayUrl: string;
    gatewayAuthToken?: string;
  }) => Promise<DesktopState>;
  deleteGatewayProfile: (input: { profileId: string }) => Promise<DesktopState>;
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
    options?: GatewaySettingsSaveRequestOptions,
  ) => Promise<GatewaySettingsSaveResult>;
  selectWorkspace: (input: SelectWorkspaceInput) => Promise<DesktopState>;
  listWorkspaceDirectories: (input?: {
    path?: string | null;
  }) => Promise<DesktopLocalDirectoryListing>;
  addWorkspaceByPath: (
    input: AddWorkspaceByPathInput,
  ) => Promise<WorkspaceMutationResult>;
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
  listTasks: (input?: ListTasksInput) => Promise<DesktopTasksPage>;
  createTask: (input: CreateTaskInput) => Promise<DesktopTaskSummary>;
  listWorkflowDefinitions: () => Promise<DesktopWorkflowDefinition[]>;
  getWorkflowDefinitionSource: (
    input: GetWorkflowDefinitionSourceInput,
  ) => Promise<DesktopWorkflowSourceDocument>;
  listTaskWorkflowRuns: (
    input: ListTaskWorkflowRunsInput,
  ) => Promise<DesktopWorkflowRunsPage>;
  getWorkflowRun: (
    input: GetWorkflowRunInput,
  ) => Promise<DesktopWorkflowRunDrilldown>;
  startWorkflowThread: (
    input: StartWorkflowThreadInput,
  ) => Promise<StartWorkflowThreadResult>;
  getWorkspaceGitStatus: (input: {
    workspacePath: string;
  }) => Promise<DesktopWorkspaceGitStatus>;
  getWorkspaceGitDetails: (input: {
    workspacePath: string;
  }) => Promise<DesktopWorkspaceGitDetails>;
  commitWorkspaceChanges: (
    input: CommitWorkspaceChangesInput,
  ) => Promise<WorkspaceGitMutationResult>;
  pushWorkspaceBranch: (
    input: PushWorkspaceBranchInput,
  ) => Promise<WorkspaceGitMutationResult>;
  updateTaskStatus: (input: UpdateTaskStatusInput) => Promise<void>;
  assignTask: (input: AssignTaskInput) => Promise<void>;
  unassignTask: (input: UnassignTaskInput) => Promise<void>;
  stopTask: (input: StopTaskInput) => Promise<void>;
  deleteTask: (input: DeleteTaskInput) => Promise<void>;
  updateTaskTitle: (input: UpdateTaskTitleInput) => Promise<void>;
  listDreams: (input?: ListDreamsInput) => Promise<DesktopDreamsPage>;
  scanDreams: (input?: ScanDreamsInput) => Promise<DesktopDreamsPage>;
  getDream: (dreamId: string) => Promise<DesktopDreamTopic | null>;
  listSkills: () => Promise<DesktopSkillInfo[]>;
  listCustomAgents: () => Promise<DesktopCustomAgent[]>;
  listProviderModels: (
    providerType: DesktopApiProviderType,
  ) => Promise<DesktopProviderModels>;
  createCustomAgent: (
    input: CreateCustomAgentInput,
  ) => Promise<DesktopCustomAgent>;
  updateCustomAgent: (
    input: UpdateCustomAgentInput,
  ) => Promise<DesktopCustomAgent>;
  deleteCustomAgent: (input: DeleteCustomAgentInput) => Promise<void>;
  generateCustomAgentAvatar: (
    input: GenerateCustomAgentAvatarInput,
  ) => Promise<GenerateCustomAgentAvatarResult>;
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
  listProviderRecentSessions: (
    input?: ListProviderRecentSessionsInput,
  ) => Promise<DesktopProviderRecentSession[]>;
  renameThread: (input: RenameThreadInput) => Promise<DesktopState>;
  updateThreadRuntimeSettings: (
    input: UpdateThreadRuntimeSettingsInput,
  ) => Promise<ThreadTranscript>;
  deleteThread: (input: DeleteThreadInput) => Promise<DesktopState>;
  setThreadPinned: (input: SetThreadPinnedInput) => Promise<DesktopState>;
  getThreadHistory: (
    input: string | GetThreadHistoryInput,
  ) => Promise<ThreadTranscript>;
  loadThreadTranscriptCache: (threadId: string) => Promise<ThreadTranscript | null>;
  saveThreadTranscriptCache: (transcript: ThreadTranscript) => Promise<void>;
  clearThreadTranscriptCache: (threadId: string) => Promise<void>;
  startThreadStream: (input: StartThreadStreamInput) => Promise<void>;
  stopThreadStream: (input?: StopThreadStreamInput) => Promise<void>;
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
  captureBrowserTab: (
    input: string | CaptureBrowserTabInput,
  ) => Promise<CaptureBrowserTabResult>;
  setBrowserAnnotationMode: (
    input: BrowserAnnotationModeInput,
  ) => Promise<void>;
  copyImageToClipboard: (input: CopyImageToClipboardInput) => Promise<void>;
  copyTextToClipboard: (input: CopyTextToClipboardInput) => Promise<void>;
  updateBrowserBounds: (input: BrowserBoundsInput) => Promise<void>;
  setBrowserOverlayPaused: (paused: boolean) => Promise<void>;
  showBrowserConnectionMenu: (
    input: ShowBrowserConnectionMenuInput,
  ) => Promise<void>;
  subscribeBrowserState: (listener: DesktopBrowserStateListener) => void;
  unsubscribeBrowserState: (listener: DesktopBrowserStateListener) => void;
  subscribeBrowserAnnotationComments: (
    listener: DesktopBrowserAnnotationCommentListener,
  ) => void;
  unsubscribeBrowserAnnotationComments: (
    listener: DesktopBrowserAnnotationCommentListener,
  ) => void;
  subscribeBrowserPageMouseDown: (
    listener: DesktopBrowserPageMouseDownListener,
  ) => void;
  unsubscribeBrowserPageMouseDown: (
    listener: DesktopBrowserPageMouseDownListener,
  ) => void;
  listTerminalState: () => Promise<DesktopTerminalState>;
  createTerminalSession: (
    input?: CreateTerminalSessionInput,
  ) => Promise<DesktopTerminalState>;
  activateTerminalSession: (
    input: TerminalSessionInput,
  ) => Promise<DesktopTerminalState>;
  closeTerminalSession: (
    input: TerminalSessionInput,
  ) => Promise<DesktopTerminalState>;
  writeTerminalInput: (input: TerminalWriteInput) => Promise<void>;
  resizeTerminalSession: (input: TerminalResizeInput) => Promise<void>;
  subscribeTerminalEvents: (listener: DesktopTerminalEventListener) => void;
  unsubscribeTerminalEvents: (listener: DesktopTerminalEventListener) => void;
  getAppVersion: () => Promise<string>;
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
  | { phase: "installing"; info: DesktopUpdateInfo }
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
  providerGeminiEnv: "",
  threadLogsPanelWidth: 360,
  languagePreference: "system",
  followUpBehavior: "queue",
};
