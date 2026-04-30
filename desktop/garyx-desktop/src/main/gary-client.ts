import type {
  CreateCustomAgentInput,
  CreateTeamInput,
  CreateAutoResearchRunInput,
  CreateSkillInput,
  DeleteCustomAgentInput,
  DeleteTeamInput,
  DeleteMcpServerInput,
  DesktopAutomationActivityEntry,
  DesktopAutomationActivityFeed,
  DesktopAutomationSchedule,
  DesktopAutomationStatus,
  DesktopApiProviderType,
  DesktopAutoResearchIteration,
  DesktopAutoResearchRun,
  DesktopAutoResearchRunDetail,
  DesktopAutomationSummary,
  ChannelPluginCatalogEntry,
  ChatStreamToolMessage,
  ConnectionStatus,
  DesktopCustomAgent,
  DesktopBotConsoleSummary,
  DesktopBotConversationNode,
  DesktopTeam,
  DeleteSlashCommandInput,
  DesktopChatStreamEvent,
  DesktopChannelEndpoint,
  DesktopMcpServer,
  DesktopSettings,
  DesktopThreadSummary,
  DesktopThreadProviderType,
  DesktopSkillEditorState,
  DesktopSkillEntryNode,
  DesktopSkillFileDocument,
  DesktopSkillInfo,
  DesktopWorkspaceFileEntry,
  DesktopWorkspaceFileListing,
  DesktopWorkspaceFilePreview,
  GatewayConfigDocument,
  GatewayProbeResult,
  GatewaySettingsPayload,
  GatewaySettingsSaveResult,
  GatewaySettingsSource,
  InterruptResult,
  CandidatesResponse,
  CandidateVerdict,
  ListAutoResearchRunsInput,
  ListCandidatesInput,
  ListWorkspaceFilesInput,
  MessageFileAttachment,
  MessageImageAttachment,
  OpenChatStreamResult,
  PendingThreadInput,
  PreviewWorkspaceFileInput,
  SendMessageInput,
  SendStreamingInputResult,
  SlashCommand,
  ResearchCandidate,
  SelectCandidateInput,
  StopAutoResearchRunInput,
  ThreadLogChunk,
  ThreadChannelBindingInfo,
  ThreadRuntimeInfo,
  ThreadTeamBlock,
  ThreadTranscript,
  ToggleMcpServerInput,
  TranscriptMessage,
  UploadChatAttachmentsInput,
  UploadChatAttachmentsResult,
  UploadWorkspaceFilesInput,
  UploadWorkspaceFilesResult,
  UpdateCustomAgentInput,
  UpdateTeamInput,
  UpdateMcpServerInput,
  UpdateSkillInput,
  UpdateSlashCommandInput,
  UpsertMcpServerInput,
  UpsertSlashCommandInput,
} from "@shared/contracts";

interface StreamInputWaiter {
  resolve: (result: SendStreamingInputResult) => void;
  reject: (error: Error) => void;
}

interface InterruptWaiter {
  resolve: (result: InterruptResult) => void;
  reject: (error: Error) => void;
}

interface ActiveChatSocket {
  socket: WebSocket;
  threadId: string;
  runId: string;
  responseText: string;
  sawTerminal: boolean;
  capturePrimaryResponse: boolean;
  pendingInputWaiters: StreamInputWaiter[];
  pendingInterruptWaiters: InterruptWaiter[];
}

const activeStreamRequests = new Map<string, ActiveChatSocket>();
const LOCAL_GATEWAY_HOSTS = new Set([
  "127.0.0.1",
  "localhost",
  "0.0.0.0",
  "::1",
  "[::1]",
]);
const CLAUDE_ENV_METADATA_KEY = "desktop_claude_env";
const CODEX_ENV_METADATA_KEY = "desktop_codex_env";
const CODEX_API_KEY_ENV = "OPENAI_API_KEY";

type SerializedMessageAttachments = {
  attachments: Array<{
    kind: "image" | "file";
    path: string;
    name: string;
    media_type: string;
  }>;
  images: Array<{
    name: string;
    data: string;
    media_type: string;
  }>;
  files: Array<{
    name: string;
    data: string;
    media_type: string;
  }>;
};

function serializeMessageAttachments(
  images?: MessageImageAttachment[],
  files?: MessageFileAttachment[],
): SerializedMessageAttachments {
  const attachments: SerializedMessageAttachments["attachments"] = [];
  const fallbackImages: SerializedMessageAttachments["images"] = [];
  const fallbackFiles: SerializedMessageAttachments["files"] = [];

  for (const image of images || []) {
    const path = image?.path?.trim() || "";
    const mediaType = image?.mediaType?.trim() || "";
    if (path && mediaType) {
      attachments.push({
        kind: "image",
        path,
        name: image.name,
        media_type: mediaType,
      });
      continue;
    }
    const data = image?.data?.trim() || "";
    if (!data || !mediaType) {
      continue;
    }
    fallbackImages.push({
      name: image.name,
      data,
      media_type: mediaType,
    });
  }

  for (const file of files || []) {
    const path = file?.path?.trim() || "";
    if (path) {
      attachments.push({
        kind: "file",
        path,
        name: file.name,
        media_type: file?.mediaType?.trim() || "",
      });
      continue;
    }
    const data = file?.data?.trim() || "";
    if (!data) {
      continue;
    }
    fallbackFiles.push({
      name: file.name,
      data,
      media_type: file?.mediaType?.trim() || "",
    });
  }

  return {
    attachments,
    images: fallbackImages,
    files: fallbackFiles,
  };
}

function formatLocalChatTimestamp(date = new Date()): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  const hours = String(date.getHours()).padStart(2, "0");
  const minutes = String(date.getMinutes()).padStart(2, "0");
  const seconds = String(date.getSeconds()).padStart(2, "0");
  return `${year}-${month}-${day} ${hours}:${minutes}:${seconds}`;
}

function resolveInputThreadId(input: SendMessageInput): string {
  return input.threadId || input.sessionId || "";
}

interface StatusPayload {
  sessions?: {
    count?: number;
  };
}

interface RuntimePayload {
  runtime?: {
    version?: string;
  };
  gateway?: {
    host?: string;
    port?: number;
  };
}

function normalizeGatewayUrl(gatewayUrl: string): string {
  return gatewayUrl.trim().replace(/\/+$/, "");
}

interface HistoryPayload {
  ok?: boolean;
  messages?: Array<{
    index?: number;
    role?: string;
    kind?: string;
    timestamp?: string | null;
    text?: string | null;
    content?: string | null;
    message?: Record<string, unknown> | null;
  }>;
  pending_user_inputs?: Array<{
    id?: string;
    run_id?: string | null;
    timestamp?: string | null;
    status?: string;
    active?: boolean;
    text?: string | null;
    content?: unknown;
  }>;
  team?: ThreadTeamBlockPayload | null;
}

interface ThreadMetadataPayload extends ThreadSummaryPayload {
  sdk_session_id?: string | null;
  provider_type?: string | null;
}

interface ThreadTeamBlockPayload {
  team_id?: string;
  display_name?: string;
  leader_agent_id?: string;
  member_agent_ids?: string[];
  child_thread_ids?: Record<string, string>;
}

interface ThreadLogPayload {
  threadId?: string;
  thread_id?: string;
  path?: string;
  text?: string;
  cursor?: number;
  reset?: boolean;
}

interface ThreadSummaryPayload {
  thread_key?: string;
  session_key?: string;
  thread_id?: string;
  agent_id?: string | null;
  agentId?: string | null;
  label?: string | null;
  workspace_dir?: string | null;
  channel_bindings?: Array<{
    channel?: string;
    account_id?: string;
    binding_key?: string;
    peer_id?: string;
    chat_id?: string;
    thread_scope?: string | null;
    delivery_target_type?: string;
    delivery_target_id?: string;
    display_label?: string;
    last_inbound_at?: string | null;
    last_delivery_at?: string | null;
  }>;
  updated_at?: string | null;
  created_at?: string | null;
  message_count?: number;
  last_user_message?: string | null;
  last_assistant_message?: string | null;
  team_id?: string | null;
  team_display_name?: string | null;
  teamDisplayName?: string | null;
  team?: ThreadTeamBlockPayload | null;
  recent_run_id?: string | null;
  sdk_session_id?: string | null;
}

interface ThreadsPayload {
  threads?: ThreadSummaryPayload[];
  sessions?: ThreadSummaryPayload[];
}

interface ChannelEndpointsPayload {
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

interface AutomationSummaryPayload {
  id?: string;
  label?: string | null;
  prompt?: string | null;
  agent_id?: string | null;
  agentId?: string | null;
  enabled?: boolean;
  workspace_dir?: string | null;
  workspaceDir?: string | null;
  thread_id?: string | null;
  threadId?: string | null;
  next_run?: string | null;
  nextRun?: string | null;
  last_run_at?: string | null;
  lastRunAt?: string | null;
  last_status?: string | null;
  lastStatus?: string | null;
  unread_hint_timestamp?: string | null;
  unreadHintTimestamp?: string | null;
  schedule?: unknown;
}

interface AutomationsPayload {
  automations?: AutomationSummaryPayload[];
}

interface AutomationActivityPayload {
  items?: Array<{
    run_id?: string;
    status?: string | null;
    started_at?: string | null;
    finished_at?: string | null;
    duration_ms?: number | null;
    excerpt?: string | null;
    thread_id?: string | null;
  }>;
  threadId?: string;
  count?: number;
}

interface SkillPayload {
  id?: string;
  name?: string | null;
  description?: string | null;
  installed?: boolean;
  enabled?: boolean;
  source_path?: string | null;
  sourcePath?: string | null;
}

interface SkillsPayload {
  skills?: SkillPayload[];
}

interface AutoResearchRunPayload {
  run_id?: string;
  state?: string;
  state_started_at?: string | null;
  goal?: string;
  workspace_dir?: string | null;
  max_iterations?: number;
  time_budget_secs?: number;
  iterations_used?: number;
  created_at?: string;
  updated_at?: string;
  terminal_reason?: string | null;
  candidates?: CandidatePayload[];
  selected_candidate?: string | null;
}

interface CandidatePayload {
  candidate_id?: string;
  iteration?: number;
  output?: string;
  verdict?: VerdictPayload | null;
  duration_secs?: number;
}

interface VerdictPayload {
  score?: number;
  feedback?: string;
  error?: string; // verifier failure
}

interface AutoResearchIterationPayload {
  run_id?: string;
  iteration_index?: number;
  state?: string;
  work_thread_id?: string | null;
  verify_thread_id?: string | null;
  started_at?: string;
  completed_at?: string | null;
}

interface AutoResearchRunDetailPayload {
  run?: AutoResearchRunPayload;
  latest_iteration?: AutoResearchIterationPayload | null;
  active_thread_id?: string | null;
}

interface SkillEntryPayload {
  path?: string | null;
  name?: string | null;
  entry_type?: string | null;
  entryType?: string | null;
  children?: SkillEntryPayload[] | null;
}

interface SkillEditorPayload {
  skill?: SkillPayload | null;
  entries?: SkillEntryPayload[] | null;
}

interface SkillFileDocumentPayload {
  skill?: SkillPayload | null;
  path?: string | null;
  content?: string | null;
  mediaType?: string | null;
  media_type?: string | null;
  previewKind?: string | null;
  preview_kind?: string | null;
  dataBase64?: string | null;
  data_base64?: string | null;
  editable?: boolean | null;
}

interface WorkspaceFileEntryPayload {
  path?: string | null;
  name?: string | null;
  entryType?: string | null;
  entry_type?: string | null;
  size?: number | null;
  modifiedAt?: string | null;
  modified_at?: string | null;
  mediaType?: string | null;
  media_type?: string | null;
  hasChildren?: boolean;
  has_children?: boolean;
}

interface WorkspaceFileListingPayload {
  workspaceDir?: string | null;
  workspace_dir?: string | null;
  directoryPath?: string | null;
  directory_path?: string | null;
  entries?: WorkspaceFileEntryPayload[] | null;
}

interface WorkspaceFilePreviewPayload {
  workspaceDir?: string | null;
  workspace_dir?: string | null;
  path?: string | null;
  name?: string | null;
  mediaType?: string | null;
  media_type?: string | null;
  previewKind?: string | null;
  preview_kind?: string | null;
  size?: number | null;
  modifiedAt?: string | null;
  modified_at?: string | null;
  truncated?: boolean;
  text?: string | null;
  dataBase64?: string | null;
  data_base64?: string | null;
}

interface UploadWorkspaceFilesPayload {
  workspaceDir?: string | null;
  workspace_dir?: string | null;
  directoryPath?: string | null;
  directory_path?: string | null;
  uploadedPaths?: string[] | null;
  uploaded_paths?: string[] | null;
}

interface UploadedChatAttachmentPayload {
  kind?: "image" | "file" | null;
  path?: string | null;
  name?: string | null;
  mediaType?: string | null;
  media_type?: string | null;
}

interface UploadChatAttachmentsPayload {
  files?: UploadedChatAttachmentPayload[] | null;
}

interface SlashCommandPayload {
  name?: string;
  description?: string | null;
  prompt?: string | null;
}

interface SlashCommandsPayload {
  commands?: SlashCommandPayload[];
}

interface McpServerPayload {
  name?: string;
  transport?: string | null;
  command?: string | null;
  args?: unknown;
  env?: unknown;
  enabled?: boolean;
  working_dir?: string | null;
  workingDir?: string | null;
  url?: string | null;
  bearer_token_env?: string | null;
  bearerTokenEnv?: string | null;
  headers?: unknown;
}

interface McpServersPayload {
  servers?: McpServerPayload[];
}

interface CustomAgentPayload {
  agent_id?: string;
  agentId?: string;
  display_name?: string;
  displayName?: string;
  role?: string | null;
  provider_type?: string;
  providerType?: string;
  model?: string | null;
  system_prompt?: string | null;
  systemPrompt?: string | null;
  built_in?: boolean;
  builtIn?: boolean;
  standalone?: boolean;
  created_at?: string;
  createdAt?: string;
  updated_at?: string;
  updatedAt?: string;
}

interface CustomAgentsPayload {
  agents?: CustomAgentPayload[];
}

interface TeamPayload {
  team_id?: string;
  teamId?: string;
  display_name?: string;
  displayName?: string;
  leader_agent_id?: string;
  leaderAgentId?: string;
  member_agent_ids?: unknown;
  memberAgentIds?: unknown;
  workflow_text?: string | null;
  workflowText?: string | null;
  created_at?: string;
  createdAt?: string;
  updated_at?: string;
  updatedAt?: string;
}

interface TeamsPayload {
  teams?: TeamPayload[];
}

function baseUrl(settings: DesktopSettings): string {
  return normalizeGatewayUrl(settings.gatewayUrl);
}

function normalizeDesktopProviderType(
  value: unknown,
): "claude_code" | "codex_app_server" | "gemini_cli" {
  if (value === "codex_app_server") {
    return "codex_app_server";
  }
  if (value === "gemini_cli") {
    return "gemini_cli";
  }
  return "claude_code";
}

function parseThreadProviderType(
  value: unknown,
): DesktopThreadProviderType | null {
  if (
    value === "claude_code" ||
    value === "codex_app_server" ||
    value === "gemini_cli" ||
    value === "agent_team"
  ) {
    return value;
  }
  return null;
}

function providerLabelForThread(
  value: DesktopThreadProviderType | null | undefined,
): string | null {
  switch (value) {
    case "claude_code":
      return "Claude";
    case "codex_app_server":
      return "Codex";
    case "gemini_cli":
      return "Gemini";
    case "agent_team":
      return "Team";
    default:
      return null;
  }
}

function buildUrl(settings: DesktopSettings, path: string): string {
  return `${baseUrl(settings)}${path.startsWith("/") ? path : `/${path}`}`;
}

function buildUrlFromGatewayUrl(gatewayUrl: string, path: string): string {
  const normalized = normalizeGatewayUrl(gatewayUrl);
  return `${normalized}${path.startsWith("/") ? path : `/${path}`}`;
}

function applyGatewayAuthHeader(
  headers: Headers,
  gatewayAuthToken: string | null | undefined,
): Headers {
  const token = gatewayAuthToken?.trim();
  if (token) {
    headers.set("Authorization", `Bearer ${token}`);
  } else {
    headers.delete("Authorization");
  }
  return headers;
}

function buildWebSocketUrl(settings: DesktopSettings, path: string): string {
  const httpUrl = new URL(buildUrl(settings, path));
  if (httpUrl.protocol === "https:") {
    httpUrl.protocol = "wss:";
  } else if (httpUrl.protocol === "http:") {
    httpUrl.protocol = "ws:";
  }
  const token = settings.gatewayAuthToken.trim();
  if (token) {
    httpUrl.searchParams.set("token", token);
  }
  return httpUrl.toString();
}

function isLocalGatewayUrl(gatewayUrl: string): boolean {
  try {
    const parsed = new URL(normalizeGatewayUrl(gatewayUrl));
    return LOCAL_GATEWAY_HOSTS.has(parsed.hostname);
  } catch {
    return false;
  }
}

function parseJson<T>(body: string): T {
  if (!body.trim()) {
    return {} as T;
  }
  return JSON.parse(body) as T;
}

function errorMessageFromPayload(payload: unknown): string | undefined {
  if (!payload || typeof payload !== "object") {
    return undefined;
  }
  const maybeRecord = payload as Record<string, unknown>;
  const message =
    maybeRecord.message ?? maybeRecord.error ?? maybeRecord.reason;
  if (typeof message === "string" && message.trim()) {
    return message;
  }
  const errors = maybeRecord.errors;
  if (Array.isArray(errors)) {
    const messages = errors
      .map((value) => (typeof value === "string" ? value.trim() : ""))
      .filter(Boolean);
    if (messages.length > 0) {
      return messages.join("; ");
    }
  }
  return undefined;
}

function normalizeGatewaySettingsPayload(
  payload: unknown,
  meta?: {
    source?: GatewaySettingsSource;
    secretsMasked?: boolean;
  },
): GatewaySettingsPayload {
  const normalizeConfig = (value: unknown): GatewayConfigDocument => {
    const config =
      value && typeof value === "object"
        ? (value as GatewayConfigDocument)
        : {};
    return stripLegacyGatewayConfigFields(config);
  };

  if (payload && typeof payload === "object" && "config" in payload) {
    const config = (payload as { config?: unknown }).config;
    return {
      config: normalizeConfig(config),
      source: meta?.source || "gateway_api",
      secretsMasked: meta?.secretsMasked ?? false,
    };
  }

  return {
    config: normalizeConfig(payload),
    source: meta?.source || "gateway_api",
    secretsMasked: meta?.secretsMasked ?? false,
  };
}

function stripLegacyGatewayConfigFields(
  config: GatewayConfigDocument,
): GatewayConfigDocument {
  const next = { ...config };
  let mutated = false;

  if (Object.prototype.hasOwnProperty.call(next, "agent_defaults")) {
    delete next.agent_defaults;
    mutated = true;
  }

  const sessions = config.sessions;
  if (sessions && typeof sessions === "object" && !Array.isArray(sessions)) {
    const nextSessions = { ...(sessions as Record<string, unknown>) };
    if (Object.prototype.hasOwnProperty.call(nextSessions, "redis")) {
      delete nextSessions.redis;
      mutated = true;
    }
    if (Object.prototype.hasOwnProperty.call(nextSessions, "store_type")) {
      delete nextSessions.store_type;
      mutated = true;
    }
    if (Object.keys(nextSessions).length === 0) {
      delete next.sessions;
      mutated = true;
    } else if (mutated) {
      next.sessions = nextSessions;
    }
  }

  return mutated ? next : config;
}

function stripNullObjectFields(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map((entry) => stripNullObjectFields(entry));
  }

  if (!value || typeof value !== "object") {
    return value;
  }

  const entries = Object.entries(value as Record<string, unknown>)
    .filter(([key, entryValue]) => {
      if (
        key === "webhook_url" ||
        key === "webhook_path" ||
        key === "webhook_secret" ||
        key === "verification_token" ||
        key === "encrypt_key"
      ) {
        return false;
      }
      return entryValue !== null && entryValue !== undefined;
    })
    .map(([key, entryValue]) => [key, stripNullObjectFields(entryValue)]);

  return Object.fromEntries(entries);
}

async function requestJson<T>(
  settings: DesktopSettings,
  path: string,
  init?: RequestInit,
): Promise<T> {
  const headers = applyGatewayAuthHeader(
    new Headers(init?.headers),
    settings.gatewayAuthToken,
  );
  headers.set("Accept", "application/json");
  if (init?.body && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }

  const response = await fetch(buildUrl(settings, path), {
    ...init,
    headers,
  });
  const body = await response.text();
  const payload = parseJson<T>(body);

  if (!response.ok) {
    throw new Error(
      errorMessageFromPayload(payload) ||
        `${response.status} ${response.statusText}`,
    );
  }

  return payload;
}

async function requestJsonFromGatewayUrl<T>(
  gatewayUrl: string,
  gatewayAuthToken: string,
  path: string,
  init?: RequestInit,
): Promise<T> {
  const headers = applyGatewayAuthHeader(
    new Headers(init?.headers),
    gatewayAuthToken,
  );
  headers.set("Accept", "application/json");
  if (init?.body && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }

  const response = await fetch(buildUrlFromGatewayUrl(gatewayUrl, path), {
    ...init,
    headers,
  });
  const body = await response.text();
  const payload = parseJson<T>(body);

  if (!response.ok) {
    throw new Error(
      errorMessageFromPayload(payload) ||
        `${response.status} ${response.statusText}`,
    );
  }

  return payload;
}

async function readGatewaySettingsFromApi(
  settings: DesktopSettings,
): Promise<GatewaySettingsPayload> {
  const payload = await requestJson<unknown>(settings, "/api/settings", {
    signal: AbortSignal.timeout(8000),
  });
  return normalizeGatewaySettingsPayload(payload, {
    source: "gateway_api",
    secretsMasked: true,
  });
}

function parseRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object"
    ? (value as Record<string, unknown>)
    : {};
}

function stripMatchingQuotes(value: string): string {
  if (value.length >= 2) {
    if (
      (value.startsWith('"') && value.endsWith('"')) ||
      (value.startsWith("'") && value.endsWith("'"))
    ) {
      return value.slice(1, -1);
    }
  }
  return value;
}

function parseProviderEnvBlock(raw: string): Record<string, string> {
  const env: Record<string, string> = {};

  for (const line of raw.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) {
      continue;
    }

    const normalized = trimmed.startsWith("export ")
      ? trimmed.slice("export ".length).trim()
      : trimmed;
    const separator = normalized.indexOf("=");
    if (separator <= 0) {
      continue;
    }

    const key = normalized.slice(0, separator).trim();
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) {
      continue;
    }

    const value = stripMatchingQuotes(normalized.slice(separator + 1).trim());
    env[key] = value;
  }

  return env;
}

function asString(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value : undefined;
}

function asBoolean(value: unknown): boolean | undefined {
  return typeof value === "boolean" ? value : undefined;
}

function buildProviderMetadata(
  settings: DesktopSettings,
): Record<string, unknown> | undefined {
  if (!isLocalGatewayUrl(settings.gatewayUrl)) {
    return undefined;
  }

  const metadata: Record<string, unknown> = {};
  const claudeEnv = parseProviderEnvBlock(settings.providerClaudeEnv);
  const oauthToken = asString(process.env.CLAUDE_CODE_OAUTH_TOKEN);
  if (
    oauthToken &&
    !Object.prototype.hasOwnProperty.call(claudeEnv, "CLAUDE_CODE_OAUTH_TOKEN")
  ) {
    claudeEnv.CLAUDE_CODE_OAUTH_TOKEN = oauthToken;
  }

  if (Object.keys(claudeEnv).length > 0) {
    metadata[CLAUDE_ENV_METADATA_KEY] = claudeEnv;
  }

  metadata[CODEX_ENV_METADATA_KEY] = {
    [CODEX_API_KEY_ENV]:
      settings.providerCodexAuthMode === "api_key"
        ? settings.providerCodexApiKey.trim()
        : "",
  };

  return Object.keys(metadata).length > 0 ? metadata : undefined;
}

function mapStreamToolMessage(value: unknown): ChatStreamToolMessage {
  const record = parseRecord(value);
  const role = record.role === "tool_result" ? "tool_result" : "tool_use";
  const metadataValue = record.metadata;
  return {
    role,
    content: record.content,
    timestamp: typeof record.timestamp === "string" ? record.timestamp : null,
    toolUseId:
      asString(record.tool_use_id) || asString(record.toolUseId) || null,
    toolName: asString(record.tool_name) || asString(record.toolName) || null,
    isError: asBoolean(record.is_error) ?? asBoolean(record.isError),
    metadata:
      metadataValue && typeof metadataValue === "object"
        ? (metadataValue as Record<string, unknown>)
        : null,
  };
}

function appendStreamResponseSeparator(responseText: string): string {
  if (!responseText.trim()) {
    return responseText;
  }
  if (responseText.endsWith("\n\n")) {
    return responseText;
  }
  if (responseText.endsWith("\n")) {
    return `${responseText}\n`;
  }
  return `${responseText}\n\n`;
}

function mapHistoryMessage(
  sessionId: string,
  value: NonNullable<HistoryPayload["messages"]>[number],
) {
  const normalized = parseRecord(value.message);
  const isLoopContinuation =
    Boolean((value as { internal?: boolean }).internal) &&
    (value as { internal_kind?: unknown }).internal_kind ===
      "loop_continuation";
  const role = isLoopContinuation
    ? "system"
    : value.role === "assistant"
      ? "assistant"
      : value.role === "user"
        ? "user"
        : value.role === "tool_use"
          ? "tool_use"
          : value.role === "tool_result"
            ? "tool_result"
            : "system";
  const content = "content" in normalized ? normalized.content : value.content;
  const metadataValue = normalized.metadata;
  const fallbackText =
    isLoopContinuation && value.role === "user"
      ? "System triggered an automatic continuation."
      : "";
  const text =
    asString(normalized.text) ||
    (typeof value.text === "string" ? value.text.trim() : "") ||
    (typeof value.content === "string" ? value.content.trim() : "") ||
    fallbackText;
  const hasStructuredContent = content !== null && content !== undefined;

  if (!text && !hasStructuredContent) {
    return null;
  }

  const visibleKinds = new Set([
    "assistant_reply",
    "user_input",
    "tool_trace",
    "system",
    "internal",
  ]);
  if (!visibleKinds.has(value.kind || "") && role === "system") {
    return null;
  }

  const message: TranscriptMessage = {
    id: `${sessionId}:${value.index ?? Math.random().toString(16).slice(2)}`,
    role,
    text,
    content,
    toolUseId:
      asString(normalized.tool_use_id) ||
      asString(normalized.toolUseId) ||
      null,
    toolName:
      asString(normalized.tool_name) || asString(normalized.toolName) || null,
    isError: asBoolean(normalized.is_error) ?? asBoolean(normalized.isError),
    metadata:
      metadataValue && typeof metadataValue === "object"
        ? (metadataValue as Record<string, unknown>)
        : null,
    timestamp: value.timestamp,
    kind: value.kind,
    internal: Boolean((value as { internal?: boolean }).internal),
    internalKind:
      typeof (value as { internal_kind?: unknown }).internal_kind === "string"
        ? ((value as { internal_kind?: string }).internal_kind ?? null)
        : null,
    loopOrigin:
      typeof (value as { loop_origin?: unknown }).loop_origin === "string"
        ? ((value as { loop_origin?: string }).loop_origin ?? null)
        : null,
  };
  return message;
}

function mapPendingUserInput(
  value: NonNullable<HistoryPayload["pending_user_inputs"]>[number],
): PendingThreadInput | null {
  const id = asString(value.id);
  const status = value.status === "orphaned" ? "orphaned" : "awaiting_ack";
  const content = value.content;
  const text =
    asString(value.text) || (typeof content === "string" ? content.trim() : "");

  if (!id || (!text && (content === null || content === undefined))) {
    return null;
  }

  return {
    id,
    runId: asString(value.run_id) || null,
    text,
    content,
    timestamp: asString(value.timestamp) || null,
    status,
    active: value.active !== false && status === "awaiting_ack",
  };
}

function mapThreadTeamBlock(
  value: ThreadTeamBlockPayload | null | undefined,
): ThreadTeamBlock | null {
  if (!value || typeof value !== "object") {
    return null;
  }
  const teamId = typeof value.team_id === "string" ? value.team_id : "";
  if (!teamId) {
    return null;
  }
  const memberIds = Array.isArray(value.member_agent_ids)
    ? value.member_agent_ids.filter(
        (entry): entry is string => typeof entry === "string",
      )
    : [];
  const childThreadIds: Record<string, string> = {};
  if (value.child_thread_ids && typeof value.child_thread_ids === "object") {
    for (const [agentId, threadId] of Object.entries(value.child_thread_ids)) {
      if (
        typeof agentId === "string" &&
        typeof threadId === "string" &&
        threadId
      ) {
        childThreadIds[agentId] = threadId;
      }
    }
  }
  return {
    team_id: teamId,
    display_name:
      typeof value.display_name === "string" ? value.display_name : "",
    leader_agent_id:
      typeof value.leader_agent_id === "string" ? value.leader_agent_id : "",
    member_agent_ids: memberIds,
    child_thread_ids: childThreadIds,
  };
}

function mapThreadChannelBinding(
  value:
    | NonNullable<ThreadSummaryPayload["channel_bindings"]>[number]
    | null
    | undefined,
): ThreadChannelBindingInfo | null {
  if (!value || typeof value !== "object") {
    return null;
  }
  const channel = typeof value.channel === "string" ? value.channel : "";
  const accountId =
    typeof value.account_id === "string" ? value.account_id : "";
  const bindingKey =
    typeof value.binding_key === "string"
      ? value.binding_key
      : typeof value.peer_id === "string"
        ? value.peer_id
        : typeof value.thread_scope === "string"
          ? value.thread_scope
          : "";
  const chatId = typeof value.chat_id === "string" ? value.chat_id : "";
  const deliveryTargetType =
    typeof value.delivery_target_type === "string"
      ? value.delivery_target_type
      : "chat_id";
  const deliveryTargetId =
    typeof value.delivery_target_id === "string"
      ? value.delivery_target_id
      : chatId;
  return {
    channel,
    accountId,
    bindingKey,
    chatId,
    deliveryTargetType,
    deliveryTargetId,
    displayLabel:
      typeof value.display_label === "string" ? value.display_label : "",
    lastInboundAt:
      typeof value.last_inbound_at === "string" ? value.last_inbound_at : null,
    lastDeliveryAt:
      typeof value.last_delivery_at === "string"
        ? value.last_delivery_at
        : null,
  };
}

function mapThreadRuntimeInfo(
  value: ThreadMetadataPayload | null | undefined,
): ThreadRuntimeInfo | null {
  if (!value || typeof value !== "object") {
    return null;
  }
  const providerType = parseThreadProviderType(value.provider_type);
  const channelBindings = Array.isArray(value.channel_bindings)
    ? value.channel_bindings
        .map((entry) => mapThreadChannelBinding(entry))
        .filter((entry): entry is ThreadChannelBindingInfo => Boolean(entry))
    : [];
  return {
    agentId:
      typeof value.agent_id === "string" ? value.agent_id : value.agentId ?? null,
    providerType,
    providerLabel: providerLabelForThread(providerType),
    sdkSessionId:
      typeof value.sdk_session_id === "string" ? value.sdk_session_id : null,
    workspacePath:
      typeof value.workspace_dir === "string" ? value.workspace_dir : null,
    channelBindings,
  };
}

function mapThreadSummary(value: ThreadSummaryPayload): DesktopThreadSummary {
  const id = value.thread_id || value.thread_key || value.session_key || "";
  const team = mapThreadTeamBlock(value.team);
  const teamDisplayName =
    (team && team.display_name.trim()) ||
    (typeof value.team_display_name === "string"
      ? value.team_display_name.trim()
      : "") ||
    (typeof value.teamDisplayName === "string"
      ? value.teamDisplayName.trim()
      : "");
  const labelTrimmed =
    typeof value.label === "string" && value.label.trim()
      ? value.label.trim()
      : "";
  // Title fallback chain: explicit label wins; otherwise a team thread prefers
  // the team's display_name so the thread list/header renders the team name.
  // ThreadsListPage + ThreadPage both consume `DesktopThreadSummary.title`
  // directly, so this fallback is the single source of truth for that branding.
  const title = labelTrimmed || teamDisplayName || id;
  const lastMessagePreview =
    (typeof value.last_assistant_message === "string" &&
      value.last_assistant_message.trim()) ||
    (typeof value.last_user_message === "string" &&
      value.last_user_message.trim()) ||
    "";
  return {
    id,
    title,
    createdAt: value.created_at || new Date(0).toISOString(),
    updatedAt:
      value.updated_at || value.created_at || new Date(0).toISOString(),
    lastMessagePreview,
    workspacePath: value.workspace_dir ?? null,
    messageCount:
      typeof value.message_count === "number" &&
      Number.isFinite(value.message_count)
        ? value.message_count
        : undefined,
    agentId:
      typeof (value as { agent_id?: unknown }).agent_id === "string"
        ? ((value as { agent_id?: string }).agent_id ?? null)
        : null,
    teamId:
      typeof (value as { team_id?: unknown }).team_id === "string"
        ? ((value as { team_id?: string }).team_id ?? null)
        : null,
    teamName:
      (team && team.display_name) ||
      (typeof value.team_display_name === "string"
        ? value.team_display_name
        : typeof value.teamDisplayName === "string"
          ? value.teamDisplayName
          : null),
    team,
    recentRunId:
      typeof (value as { recent_run_id?: unknown }).recent_run_id === "string"
        ? ((value as { recent_run_id?: string }).recent_run_id ?? null)
        : null,
  };
}

function mapTeam(value: TeamPayload): DesktopTeam {
  const members = Array.isArray(value.member_agent_ids)
    ? value.member_agent_ids
    : Array.isArray(value.memberAgentIds)
      ? value.memberAgentIds
      : [];
  return {
    teamId: value.team_id || value.teamId || "",
    displayName: value.display_name || value.displayName || "",
    leaderAgentId: value.leader_agent_id || value.leaderAgentId || "",
    memberAgentIds: members.filter(
      (entry): entry is string => typeof entry === "string",
    ),
    workflowText: value.workflow_text || value.workflowText || "",
    createdAt: value.created_at || value.createdAt || new Date(0).toISOString(),
    updatedAt: value.updated_at || value.updatedAt || new Date(0).toISOString(),
  };
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

function normalizeAutomationStatus(value: unknown): DesktopAutomationStatus {
  return value === "failed" || value === "skipped" ? value : "success";
}

function mapAutomationSchedule(value: unknown): DesktopAutomationSchedule {
  const record = parseRecord(value);
  if (record.kind === "daily") {
    return {
      kind: "daily",
      time: asString(record.time) || "09:00",
      weekdays: Array.isArray(record.weekdays)
        ? record.weekdays.filter(
            (entry): entry is string => typeof entry === "string",
          )
        : [],
      timezone:
        asString(record.timezone) ||
        Intl.DateTimeFormat().resolvedOptions().timeZone ||
        "UTC",
    };
  }

  if (record.kind === "once") {
    return {
      kind: "once",
      at: asString(record.at) || "",
    };
  }

  const hoursValue =
    typeof record.hours === "number" && Number.isFinite(record.hours)
      ? Math.max(1, Math.round(record.hours))
      : 24;
  return {
    kind: "interval",
    hours: hoursValue,
  };
}

function mapAutomationSummary(
  value: AutomationSummaryPayload,
): DesktopAutomationSummary {
  const agentId = value.agentId ?? value.agent_id;
  return {
    id: value.id || "",
    label:
      typeof value.label === "string" && value.label.trim()
        ? value.label.trim()
        : value.id || "",
    prompt: typeof value.prompt === "string" ? value.prompt : "",
    agentId:
      typeof agentId === "string" && agentId.trim() ? agentId.trim() : "claude",
    enabled: value.enabled !== false,
    workspacePath: value.workspaceDir || value.workspace_dir || "",
    threadId: value.threadId || value.thread_id || "",
    nextRun: value.nextRun || value.next_run || new Date(0).toISOString(),
    lastRunAt: value.lastRunAt ?? value.last_run_at ?? null,
    lastStatus: normalizeAutomationStatus(
      value.lastStatus ?? value.last_status,
    ),
    unreadHintTimestamp:
      value.unreadHintTimestamp ?? value.unread_hint_timestamp ?? null,
    schedule: mapAutomationSchedule(value.schedule),
  };
}

function mapAutomationActivityEntry(
  value: NonNullable<AutomationActivityPayload["items"]>[number],
): DesktopAutomationActivityEntry {
  return {
    runId: value.run_id || "",
    status: normalizeAutomationStatus(value.status),
    startedAt: value.started_at || new Date(0).toISOString(),
    finishedAt: value.finished_at ?? null,
    durationMs:
      typeof value.duration_ms === "number" &&
      Number.isFinite(value.duration_ms)
        ? value.duration_ms
        : null,
    excerpt:
      typeof value.excerpt === "string" && value.excerpt.trim()
        ? value.excerpt.trim()
        : null,
    threadId: value.thread_id || "",
  };
}

function mapAutoResearchRun(
  value: AutoResearchRunPayload,
): DesktopAutoResearchRun {
  return {
    runId: value.run_id || "",
    state: (value.state as DesktopAutoResearchRun["state"]) || "queued",
    stateStartedAt: value.state_started_at ?? null,
    goal: value.goal || "",
    workspaceDir: value.workspace_dir ?? null,
    maxIterations:
      typeof value.max_iterations === "number" &&
      Number.isFinite(value.max_iterations)
        ? value.max_iterations
        : 0,
    timeBudgetSecs:
      typeof value.time_budget_secs === "number" &&
      Number.isFinite(value.time_budget_secs)
        ? value.time_budget_secs
        : 0,
    iterationsUsed:
      typeof value.iterations_used === "number" &&
      Number.isFinite(value.iterations_used)
        ? value.iterations_used
        : 0,
    createdAt: value.created_at || new Date(0).toISOString(),
    updatedAt: value.updated_at || new Date(0).toISOString(),
    terminalReason: value.terminal_reason ?? null,
    candidates: Array.isArray(value.candidates)
      ? value.candidates.map(mapCandidate)
      : [],
    selectedCandidate: value.selected_candidate ?? null,
  };
}

function mapVerdict(value?: VerdictPayload | null): CandidateVerdict | null {
  if (!value) return null;

  // Handle verifier error verdicts — surface the error as feedback with score 0
  if (typeof value.error === "string" && value.error) {
    return { score: 0, feedback: `Verifier error: ${value.error}` };
  }

  if (typeof value.score !== "number") return null;

  return {
    score: value.score,
    feedback: typeof value.feedback === "string" ? value.feedback : "",
  };
}

function mapCandidate(value: CandidatePayload): ResearchCandidate {
  return {
    candidate_id: value.candidate_id || "",
    iteration: typeof value.iteration === "number" ? value.iteration : 0,
    output: value.output || "",
    verdict: mapVerdict(value.verdict),
    duration_secs:
      typeof value.duration_secs === "number" ? value.duration_secs : 0,
  };
}

function mapAutoResearchIteration(
  value: AutoResearchIterationPayload,
): DesktopAutoResearchIteration {
  return {
    runId: value.run_id || "",
    iterationIndex:
      typeof value.iteration_index === "number" &&
      Number.isFinite(value.iteration_index)
        ? value.iteration_index
        : 0,
    state:
      (value.state as DesktopAutoResearchIteration["state"]) || "researching",
    workThreadId: value.work_thread_id ?? null,
    verifyThreadId: value.verify_thread_id ?? null,
    startedAt: value.started_at || new Date(0).toISOString(),
    completedAt: value.completed_at ?? null,
  };
}

function mapSkill(value: SkillPayload): DesktopSkillInfo {
  return {
    id: value.id || "",
    name:
      typeof value.name === "string" && value.name.trim()
        ? value.name.trim()
        : value.id || "",
    description:
      typeof value.description === "string" && value.description.trim()
        ? value.description.trim()
        : "",
    installed: value.installed !== false,
    enabled: value.enabled !== false,
    sourcePath:
      (typeof value.source_path === "string" && value.source_path) ||
      (typeof value.sourcePath === "string" && value.sourcePath) ||
      "",
  };
}

function mapSkillEntry(value: SkillEntryPayload): DesktopSkillEntryNode {
  const entryType =
    value.entry_type === "directory" || value.entryType === "directory"
      ? "directory"
      : "file";
  return {
    path: (typeof value.path === "string" && value.path.trim()) || "",
    name: (typeof value.name === "string" && value.name.trim()) || "",
    entryType,
    children: Array.isArray(value.children)
      ? value.children.map(mapSkillEntry)
      : [],
  };
}

function mapSkillEditorState(
  value: SkillEditorPayload,
): DesktopSkillEditorState {
  return {
    skill: mapSkill(value.skill || {}),
    entries: Array.isArray(value.entries)
      ? value.entries.map(mapSkillEntry)
      : [],
  };
}

function normalizeSkillFilePreviewKind(
  value: unknown,
): DesktopSkillFileDocument["previewKind"] {
  switch (value) {
    case "markdown":
    case "text":
    case "image":
    case "unsupported":
      return value;
    default:
      return "unsupported";
  }
}

function mapSkillFileDocument(
  value: SkillFileDocumentPayload,
): DesktopSkillFileDocument {
  return {
    skill: mapSkill(value.skill || {}),
    path: typeof value.path === "string" ? value.path : "",
    content: typeof value.content === "string" ? value.content : "",
    mediaType:
      (typeof value.mediaType === "string" && value.mediaType) ||
      (typeof value.media_type === "string" ? value.media_type : "") ||
      "text/plain",
    previewKind: normalizeSkillFilePreviewKind(
      value.previewKind || value.preview_kind,
    ),
    dataBase64:
      typeof value.dataBase64 === "string"
        ? value.dataBase64
        : typeof value.data_base64 === "string"
          ? value.data_base64
          : null,
    editable: value.editable !== false,
  };
}

function mapWorkspaceFileEntry(
  value: WorkspaceFileEntryPayload,
): DesktopWorkspaceFileEntry {
  const entryType = value.entryType || value.entry_type;
  return {
    path: typeof value.path === "string" ? value.path : "",
    name: typeof value.name === "string" ? value.name : "",
    entryType: entryType === "directory" ? "directory" : "file",
    size:
      typeof value.size === "number" && Number.isFinite(value.size)
        ? value.size
        : null,
    modifiedAt:
      typeof value.modifiedAt === "string"
        ? value.modifiedAt
        : typeof value.modified_at === "string"
          ? value.modified_at
          : null,
    mediaType:
      typeof value.mediaType === "string"
        ? value.mediaType
        : typeof value.media_type === "string"
          ? value.media_type
          : null,
    hasChildren: value.hasChildren === true || value.has_children === true,
  };
}

function mapWorkspaceFileListing(
  value: WorkspaceFileListingPayload,
): DesktopWorkspaceFileListing {
  return {
    workspacePath:
      (typeof value.workspaceDir === "string" && value.workspaceDir) ||
      (typeof value.workspace_dir === "string" && value.workspace_dir) ||
      "",
    directoryPath:
      (typeof value.directoryPath === "string" && value.directoryPath) ||
      (typeof value.directory_path === "string" && value.directory_path) ||
      "",
    entries: Array.isArray(value.entries)
      ? value.entries.map(mapWorkspaceFileEntry)
      : [],
  };
}

function normalizeWorkspaceFilePreviewKind(
  value: unknown,
): DesktopWorkspaceFilePreview["previewKind"] {
  switch (value) {
    case "markdown":
    case "html":
    case "text":
    case "pdf":
    case "image":
    case "unsupported":
      return value;
    default:
      return "unsupported";
  }
}

function mapWorkspaceFilePreview(
  value: WorkspaceFilePreviewPayload,
): DesktopWorkspaceFilePreview {
  return {
    workspacePath:
      (typeof value.workspaceDir === "string" && value.workspaceDir) ||
      (typeof value.workspace_dir === "string" && value.workspace_dir) ||
      "",
    path: typeof value.path === "string" ? value.path : "",
    name: typeof value.name === "string" ? value.name : "",
    mediaType:
      (typeof value.mediaType === "string" && value.mediaType) ||
      (typeof value.media_type === "string" ? value.media_type : "") ||
      "application/octet-stream",
    previewKind: normalizeWorkspaceFilePreviewKind(
      value.previewKind || value.preview_kind,
    ),
    size:
      typeof value.size === "number" && Number.isFinite(value.size)
        ? value.size
        : 0,
    modifiedAt:
      typeof value.modifiedAt === "string"
        ? value.modifiedAt
        : typeof value.modified_at === "string"
          ? value.modified_at
          : null,
    truncated: value.truncated === true,
    text: typeof value.text === "string" ? value.text : null,
    dataBase64:
      typeof value.dataBase64 === "string"
        ? value.dataBase64
        : typeof value.data_base64 === "string"
          ? value.data_base64
          : null,
  };
}

function mapSlashCommand(value: SlashCommandPayload): SlashCommand {
  return {
    name: value.name || "",
    description:
      typeof value.description === "string" && value.description.trim()
        ? value.description.trim()
        : "",
    prompt:
      typeof value.prompt === "string" && value.prompt.trim()
        ? value.prompt
        : null,
  };
}

function mapMcpServer(value: McpServerPayload): DesktopMcpServer {
  const envRecord = parseRecord(value.env);
  const headersRecord = parseRecord(value.headers);
  const transport =
    value.transport === "streamable_http"
      ? ("streamable_http" as const)
      : ("stdio" as const);
  return {
    name: value.name || "",
    transport,
    command:
      typeof value.command === "string" && value.command.trim()
        ? value.command.trim()
        : "",
    args: Array.isArray(value.args)
      ? value.args.filter((entry): entry is string => typeof entry === "string")
      : [],
    env: Object.fromEntries(
      Object.entries(envRecord).flatMap(([key, entryValue]) => {
        return typeof entryValue === "string" ? [[key, entryValue]] : [];
      }),
    ),
    enabled: value.enabled !== false,
    workingDir:
      (typeof value.working_dir === "string" && value.working_dir.trim()) ||
      (typeof value.workingDir === "string" && value.workingDir.trim()) ||
      null,
    url:
      typeof value.url === "string" && value.url.trim()
        ? value.url.trim()
        : null,
    headers: Object.fromEntries(
      Object.entries(headersRecord).flatMap(([key, entryValue]) => {
        return typeof entryValue === "string" ? [[key, entryValue]] : [];
      }),
    ),
  };
}

function mapCustomAgent(value: CustomAgentPayload): DesktopCustomAgent {
  const provider = normalizeDesktopProviderType(
    value.provider_type || value.providerType,
  );
  return {
    agentId: value.agent_id || value.agentId || "",
    displayName: value.display_name || value.displayName || "",
    providerType: provider,
    model: value.model || "",
    systemPrompt: value.system_prompt || value.systemPrompt || "",
    builtIn: value.built_in === true || value.builtIn === true,
    standalone: value.standalone !== false,
    createdAt: value.created_at || value.createdAt || "",
    updatedAt: value.updated_at || value.updatedAt || "",
  };
}

export async function checkConnection(
  settings: DesktopSettings,
): Promise<ConnectionStatus> {
  try {
    const [health, status, runtime] = await Promise.all([
      requestJson<{ bridge_ready?: boolean }>(settings, "/api/chat/health", {
        signal: AbortSignal.timeout(5000),
      }),
      requestJson<StatusPayload>(settings, "/api/status", {
        signal: AbortSignal.timeout(5000),
      }),
      requestJson<RuntimePayload>(settings, "/runtime", {
        signal: AbortSignal.timeout(5000),
      }),
    ]);

    return {
      ok: true,
      bridgeReady: Boolean(health.bridge_ready),
      gatewayUrl: settings.gatewayUrl,
      version: runtime.runtime?.version,
      uptimeSeconds:
        typeof status === "object"
          ? ((status as Record<string, unknown>).uptime_seconds as number)
          : undefined,
      threadCount: status.sessions?.count,
      sessionCount: status.sessions?.count,
    };
  } catch (error) {
    return {
      ok: false,
      bridgeReady: false,
      gatewayUrl: settings.gatewayUrl,
      error:
        error instanceof Error ? error.message : "Unable to reach Garyx gateway",
    };
  }
}

export async function probeGateway(
  input: { gatewayUrl: string; gatewayAuthToken: string },
): Promise<GatewayProbeResult> {
  const normalizedGatewayUrl = normalizeGatewayUrl(input.gatewayUrl);
  const path = "/runtime";

  if (!normalizedGatewayUrl) {
    return {
      ok: false,
      isGaryGateway: false,
      gatewayUrl: normalizedGatewayUrl,
      path,
      error: "Gateway URL is required.",
    };
  }

  try {
    const runtime = await requestJsonFromGatewayUrl<RuntimePayload>(
      normalizedGatewayUrl,
      input.gatewayAuthToken,
      path,
      {
        signal: AbortSignal.timeout(5000),
      },
    );

    const version = runtime.runtime?.version;
    const host = runtime.gateway?.host;
    const port = runtime.gateway?.port;
    const isGaryGateway =
      typeof version === "string" &&
      version.trim().length > 0 &&
      typeof host === "string" &&
      host.trim().length > 0 &&
      typeof port === "number" &&
      Number.isFinite(port);

    return {
      ok: isGaryGateway,
      isGaryGateway,
      gatewayUrl: normalizedGatewayUrl,
      path,
      version,
      host,
      port,
      error: isGaryGateway
        ? undefined
        : "Reached the URL, but the response does not look like a Garyx gateway.",
    };
  } catch (error) {
    return {
      ok: false,
      isGaryGateway: false,
      gatewayUrl: normalizedGatewayUrl,
      path,
      error:
        error instanceof Error ? error.message : "Unable to probe gateway URL",
    };
  }
}

export async function fetchGatewaySettings(
  settings: DesktopSettings,
): Promise<GatewaySettingsPayload> {
  return readGatewaySettingsFromApi(settings);
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

export async function saveGatewaySettings(
  settings: DesktopSettings,
  config: GatewayConfigDocument,
): Promise<GatewaySettingsSaveResult> {
  const normalizedConfig = stripNullObjectFields(
    stripLegacyGatewayConfigFields(config),
  );
  const result = await requestJson<{
    ok?: boolean;
    message?: string;
    errors?: string[];
  }>(settings, "/api/settings?merge=false", {
    method: "PUT",
    signal: AbortSignal.timeout(12000),
    body: JSON.stringify(normalizedConfig),
  });

  return {
    ok: Boolean(result.ok),
    message: result.message,
    errors: Array.isArray(result.errors)
      ? result.errors.filter(
          (value): value is string => typeof value === "string",
        )
      : undefined,
    settings: await fetchGatewaySettings(settings),
  };
}

export async function fetchThreadHistory(
  settings: DesktopSettings,
  threadId: string,
): Promise<ThreadTranscript> {
  const query = new URLSearchParams({
    thread_id: threadId,
    limit: "200",
    include_tool_messages: "true",
  });
  const [payload, detail] = await Promise.all([
    requestJson<HistoryPayload>(
      settings,
      `/api/threads/history?${query.toString()}`,
      {
        signal: AbortSignal.timeout(8000),
      },
    ),
    requestJson<ThreadMetadataPayload>(
      settings,
      `/api/threads/${encodeURIComponent(threadId)}`,
      {
        signal: AbortSignal.timeout(8000),
      },
    ).catch(() => null),
  ]);

  const messages =
    payload.messages
      ?.map((value) => mapHistoryMessage(threadId, value))
      .filter((value): value is TranscriptMessage => Boolean(value)) ?? [];
  const pendingInputs =
    payload.pending_user_inputs
      ?.map((value) => mapPendingUserInput(value))
      .filter((value): value is PendingThreadInput => Boolean(value)) ?? [];

  return {
    threadId,
    remoteFound: Boolean(payload.ok),
    messages,
    pendingInputs,
    threadInfo: mapThreadRuntimeInfo(detail),
    team: mapThreadTeamBlock(payload.team),
  };
}

export async function fetchThreadLogs(
  settings: DesktopSettings,
  threadId: string,
  cursor?: number,
): Promise<ThreadLogChunk> {
  const query = new URLSearchParams();
  if (typeof cursor === "number" && Number.isFinite(cursor) && cursor >= 0) {
    query.set("cursor", String(Math.floor(cursor)));
  }
  const suffix = query.size ? `?${query.toString()}` : "";
  const payload = await requestJson<ThreadLogPayload>(
    settings,
    `/api/threads/${encodeURIComponent(threadId)}/logs${suffix}`,
    {
      signal: AbortSignal.timeout(8000),
    },
  );

  return {
    threadId: payload.threadId || payload.thread_id || threadId,
    path: typeof payload.path === "string" ? payload.path : "",
    text: typeof payload.text === "string" ? payload.text : "",
    cursor:
      typeof payload.cursor === "number" &&
      Number.isFinite(payload.cursor) &&
      payload.cursor >= 0
        ? payload.cursor
        : 0,
    reset: payload.reset !== false,
  };
}

export async function fetchThreads(
  settings: DesktopSettings,
): Promise<DesktopThreadSummary[]> {
  const payload = await requestJson<ThreadsPayload>(settings, "/api/threads", {
    signal: AbortSignal.timeout(8000),
  });

  const threads = Array.isArray(payload.threads)
    ? payload.threads
    : Array.isArray(payload.sessions)
      ? payload.sessions
      : [];
  return threads.map(mapThreadSummary);
}

export async function createRemoteThread(
  settings: DesktopSettings,
  input?: {
    title?: string;
    workspacePath?: string | null;
    agentId?: string | null;
    sdkSessionId?: string | null;
    sdkSessionProviderHint?: "claude" | "codex" | "gemini" | null;
  },
): Promise<DesktopThreadSummary> {
  const payload = await requestJson<ThreadSummaryPayload>(
    settings,
    "/api/threads",
    {
      method: "POST",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        label: input?.title || undefined,
        workspaceDir: input?.workspacePath || undefined,
        agentId: input?.agentId || undefined,
        sdkSessionId: input?.sdkSessionId || undefined,
        sdkSessionProviderHint: input?.sdkSessionProviderHint || undefined,
      }),
    },
  );
  return mapThreadSummary(payload);
}

export async function updateRemoteThread(
  settings: DesktopSettings,
  threadId: string,
  input: {
    title?: string;
    workspacePath?: string | null;
  },
): Promise<DesktopThreadSummary> {
  const payload = await requestJson<ThreadSummaryPayload>(
    settings,
    `/api/threads/${encodeURIComponent(threadId)}`,
    {
      method: "PATCH",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        label: input.title || undefined,
        workspaceDir: input.workspacePath || undefined,
      }),
    },
  );
  return mapThreadSummary(payload);
}

export async function deleteRemoteThread(
  settings: DesktopSettings,
  threadId: string,
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/threads/${encodeURIComponent(threadId)}`,
    {
      method: "DELETE",
      signal: AbortSignal.timeout(8000),
    },
  );
}

export const fetchSessions = fetchThreads;
export const createRemoteSession = createRemoteThread;
export const updateRemoteSession = updateRemoteThread;
export const deleteRemoteSession = deleteRemoteThread;

export async function fetchChannelEndpoints(
  settings: DesktopSettings,
): Promise<DesktopChannelEndpoint[]> {
  const payload = await requestJson<ChannelEndpointsPayload>(
    settings,
    "/api/channel-endpoints",
    {
      signal: AbortSignal.timeout(8000),
    },
  );

  return Array.isArray(payload.endpoints)
    ? payload.endpoints.map(mapChannelEndpoint)
    : [];
}

export async function listSkills(
  settings: DesktopSettings,
): Promise<DesktopSkillInfo[]> {
  const payload = await requestJson<SkillsPayload>(settings, "/api/skills", {
    signal: AbortSignal.timeout(8000),
  });

  return Array.isArray(payload.skills) ? payload.skills.map(mapSkill) : [];
}

export async function createSkill(
  settings: DesktopSettings,
  input: CreateSkillInput,
): Promise<DesktopSkillInfo> {
  const payload = await requestJson<SkillPayload>(settings, "/api/skills", {
    method: "POST",
    signal: AbortSignal.timeout(8000),
    body: JSON.stringify(input),
  });

  return mapSkill(payload);
}

export async function updateSkill(
  settings: DesktopSettings,
  input: UpdateSkillInput,
): Promise<DesktopSkillInfo> {
  const payload = await requestJson<SkillPayload>(
    settings,
    `/api/skills/${encodeURIComponent(input.skillId)}`,
    {
      method: "PATCH",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        name: input.name,
        description: input.description,
      }),
    },
  );

  return mapSkill(payload);
}

export async function toggleSkill(
  settings: DesktopSettings,
  skillId: string,
): Promise<DesktopSkillInfo> {
  const payload = await requestJson<SkillPayload>(
    settings,
    `/api/skills/${encodeURIComponent(skillId)}/toggle`,
    {
      method: "PATCH",
      signal: AbortSignal.timeout(8000),
    },
  );

  return mapSkill(payload);
}

export async function deleteSkill(
  settings: DesktopSettings,
  skillId: string,
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/skills/${encodeURIComponent(skillId)}`,
    {
      method: "DELETE",
      signal: AbortSignal.timeout(8000),
    },
  );
}

export async function getSkillEditor(
  settings: DesktopSettings,
  skillId: string,
): Promise<DesktopSkillEditorState> {
  const payload = await requestJson<SkillEditorPayload>(
    settings,
    `/api/skills/${encodeURIComponent(skillId)}/tree`,
    {
      signal: AbortSignal.timeout(8000),
    },
  );

  return mapSkillEditorState(payload);
}

export async function readSkillFile(
  settings: DesktopSettings,
  skillId: string,
  path: string,
): Promise<DesktopSkillFileDocument> {
  const payload = await requestJson<SkillFileDocumentPayload>(
    settings,
    `/api/skills/${encodeURIComponent(skillId)}/file?path=${encodeURIComponent(path)}`,
    {
      signal: AbortSignal.timeout(8000),
    },
  );

  return mapSkillFileDocument(payload);
}

export async function saveSkillFile(
  settings: DesktopSettings,
  input: { skillId: string; path: string; content: string },
): Promise<DesktopSkillFileDocument> {
  const payload = await requestJson<SkillFileDocumentPayload>(
    settings,
    `/api/skills/${encodeURIComponent(input.skillId)}/file`,
    {
      method: "PUT",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        path: input.path,
        content: input.content,
      }),
    },
  );

  return mapSkillFileDocument(payload);
}

export async function listWorkspaceFiles(
  settings: DesktopSettings,
  input: ListWorkspaceFilesInput,
): Promise<DesktopWorkspaceFileListing> {
  const query = new URLSearchParams({
    workspaceDir: input.workspacePath,
  });
  if (input.directoryPath?.trim()) {
    query.set("path", input.directoryPath.trim());
  }
  const payload = await requestJson<WorkspaceFileListingPayload>(
    settings,
    `/api/workspace-files?${query.toString()}`,
    {
      signal: AbortSignal.timeout(10000),
    },
  );
  return mapWorkspaceFileListing(payload);
}

export async function previewWorkspaceFile(
  settings: DesktopSettings,
  input: PreviewWorkspaceFileInput,
): Promise<DesktopWorkspaceFilePreview> {
  const query = new URLSearchParams({
    workspaceDir: input.workspacePath,
    path: input.filePath,
  });
  const payload = await requestJson<WorkspaceFilePreviewPayload>(
    settings,
    `/api/workspace-files/preview?${query.toString()}`,
    {
      signal: AbortSignal.timeout(15000),
    },
  );
  return mapWorkspaceFilePreview(payload);
}

export async function uploadChatAttachments(
  settings: DesktopSettings,
  input: UploadChatAttachmentsInput,
): Promise<UploadChatAttachmentsResult> {
  const payload = await requestJson<UploadChatAttachmentsPayload>(
    settings,
    "/api/chat/attachments/upload",
    {
      method: "POST",
      signal: AbortSignal.timeout(30000),
      body: JSON.stringify({
        files: input.files.map((file) => ({
          kind: file.kind,
          name: file.name,
          mediaType: file.mediaType || undefined,
          dataBase64: file.dataBase64,
        })),
      }),
    },
  );

  return {
    files: Array.isArray(payload.files)
      ? payload.files
          .map((file) => {
            const path =
              (typeof file.path === "string" && file.path) || "";
            const name =
              (typeof file.name === "string" && file.name) || "";
            const mediaType =
              (typeof file.mediaType === "string" && file.mediaType) ||
              (typeof file.media_type === "string" ? file.media_type : "") ||
              "";
            if (!path || !name) {
              return null;
            }
            return {
              kind: file.kind === "image" ? "image" : "file",
              path,
              name,
              mediaType,
            };
          })
          .filter(
            (
              file,
            ): file is UploadChatAttachmentsResult["files"][number] =>
              Boolean(file),
          )
      : [],
  };
}

export async function uploadWorkspaceFiles(
  settings: DesktopSettings,
  input: UploadWorkspaceFilesInput,
): Promise<UploadWorkspaceFilesResult> {
  const payload = await requestJson<UploadWorkspaceFilesPayload>(
    settings,
    "/api/workspace-files/upload",
    {
      method: "POST",
      signal: AbortSignal.timeout(20000),
      body: JSON.stringify({
        workspaceDir: input.workspacePath,
        path: input.directoryPath || undefined,
        files: input.files.map((file) => ({
          name: file.name,
          mediaType: file.mediaType || undefined,
          dataBase64: file.dataBase64,
        })),
      }),
    },
  );

  return {
    workspacePath:
      (typeof payload.workspaceDir === "string" && payload.workspaceDir) ||
      (typeof payload.workspace_dir === "string"
        ? payload.workspace_dir
        : "") ||
      input.workspacePath,
    directoryPath:
      (typeof payload.directoryPath === "string" && payload.directoryPath) ||
      (typeof payload.directory_path === "string"
        ? payload.directory_path
        : "") ||
      "",
    uploadedPaths: Array.isArray(payload.uploadedPaths)
      ? payload.uploadedPaths
      : Array.isArray(payload.uploaded_paths)
        ? payload.uploaded_paths
        : [],
  };
}

export async function createSkillEntry(
  settings: DesktopSettings,
  input: { skillId: string; path: string; entryType: "file" | "directory" },
): Promise<DesktopSkillEditorState> {
  const payload = await requestJson<SkillEditorPayload>(
    settings,
    `/api/skills/${encodeURIComponent(input.skillId)}/entries`,
    {
      method: "POST",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        path: input.path,
        entryType: input.entryType,
      }),
    },
  );

  return mapSkillEditorState(payload);
}

export async function deleteSkillEntry(
  settings: DesktopSettings,
  input: { skillId: string; path: string },
): Promise<DesktopSkillEditorState> {
  const payload = await requestJson<SkillEditorPayload>(
    settings,
    `/api/skills/${encodeURIComponent(input.skillId)}/entries?path=${encodeURIComponent(input.path)}`,
    {
      method: "DELETE",
      signal: AbortSignal.timeout(8000),
    },
  );

  return mapSkillEditorState(payload);
}

export async function listSlashCommands(
  settings: DesktopSettings,
): Promise<SlashCommand[]> {
  const payload = await requestJson<SlashCommandsPayload>(
    settings,
    "/api/commands/shortcuts",
    {
      signal: AbortSignal.timeout(8000),
    },
  );

  return Array.isArray(payload.commands)
    ? payload.commands.map(mapSlashCommand)
    : [];
}

export async function createSlashCommand(
  settings: DesktopSettings,
  input: UpsertSlashCommandInput,
): Promise<SlashCommand> {
  const payload = await requestJson<SlashCommandPayload>(
    settings,
    "/api/commands/shortcuts",
    {
      method: "POST",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        name: input.name,
        description: input.description,
        prompt: input.prompt || null,
      }),
    },
  );

  return mapSlashCommand(payload);
}

export async function updateSlashCommand(
  settings: DesktopSettings,
  input: UpdateSlashCommandInput,
): Promise<SlashCommand> {
  const payload = await requestJson<SlashCommandPayload>(
    settings,
    `/api/commands/shortcuts/${encodeURIComponent(input.currentName)}`,
    {
      method: "PUT",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        name: input.name,
        description: input.description,
        prompt: input.prompt || null,
      }),
    },
  );

  return mapSlashCommand(payload);
}

export async function deleteSlashCommand(
  settings: DesktopSettings,
  input: DeleteSlashCommandInput,
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/commands/shortcuts/${encodeURIComponent(input.name)}`,
    {
      method: "DELETE",
      signal: AbortSignal.timeout(8000),
    },
  );
}

export async function createAutoResearchRun(
  settings: DesktopSettings,
  input: CreateAutoResearchRunInput,
): Promise<DesktopAutoResearchRun> {
  const providerMetadata = buildProviderMetadata(settings);
  const payload = await requestJson<AutoResearchRunPayload>(
    settings,
    "/api/auto-research/runs",
    {
      method: "POST",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        goal: input.goal,
        workspace_dir: input.workspaceDir,
        max_iterations: input.maxIterations,
        time_budget_secs: input.timeBudgetSecs,
        provider_metadata: providerMetadata,
      }),
    },
  );
  return mapAutoResearchRun(payload);
}

export async function listAutoResearchRuns(
  settings: DesktopSettings,
  input: ListAutoResearchRunsInput = {},
): Promise<DesktopAutoResearchRun[]> {
  const search = new URLSearchParams();
  if (input.limit) {
    search.set("limit", String(input.limit));
  }
  const suffix = search.toString() ? `?${search.toString()}` : "";
  const payload = await requestJson<{ items?: AutoResearchRunPayload[] }>(
    settings,
    `/api/auto-research/runs${suffix}`,
    {
      signal: AbortSignal.timeout(8000),
    },
  );
  return Array.isArray(payload.items)
    ? payload.items.map(mapAutoResearchRun)
    : [];
}

export async function getAutoResearchRun(
  settings: DesktopSettings,
  runId: string,
): Promise<DesktopAutoResearchRunDetail> {
  const payload = await requestJson<AutoResearchRunDetailPayload>(
    settings,
    `/api/auto-research/runs/${encodeURIComponent(runId)}`,
    {
      signal: AbortSignal.timeout(8000),
    },
  );
  return {
    run: mapAutoResearchRun(payload.run || {}),
    latestIteration: payload.latest_iteration
      ? mapAutoResearchIteration(payload.latest_iteration)
      : null,
    activeThreadId: payload.active_thread_id ?? null,
  };
}

export async function listAutoResearchIterations(
  settings: DesktopSettings,
  runId: string,
): Promise<DesktopAutoResearchIteration[]> {
  const payload = await requestJson<{ items?: AutoResearchIterationPayload[] }>(
    settings,
    `/api/auto-research/runs/${encodeURIComponent(runId)}/iterations`,
    {
      signal: AbortSignal.timeout(8000),
    },
  );
  return Array.isArray(payload.items)
    ? payload.items.map(mapAutoResearchIteration)
    : [];
}

export async function stopAutoResearchRun(
  settings: DesktopSettings,
  input: StopAutoResearchRunInput,
): Promise<DesktopAutoResearchRun> {
  const payload = await requestJson<AutoResearchRunPayload>(
    settings,
    `/api/auto-research/runs/${encodeURIComponent(input.runId)}/stop`,
    {
      method: "POST",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        reason: input.reason,
      }),
    },
  );
  return mapAutoResearchRun(payload);
}

export async function deleteAutoResearchRun(
  settings: DesktopSettings,
  runId: string,
): Promise<void> {
  await requestJson(
    settings,
    `/api/auto-research/runs/${encodeURIComponent(runId)}`,
    {
      method: "DELETE",
      signal: AbortSignal.timeout(8000),
    },
  );
}

export async function listAutoResearchCandidates(
  settings: DesktopSettings,
  input: ListCandidatesInput,
): Promise<CandidatesResponse> {
  const payload = await requestJson<{
    candidates?: CandidatePayload[];
    best_candidate_id?: string | null;
  }>(
    settings,
    `/api/auto-research/runs/${encodeURIComponent(input.runId)}/candidates`,
    { signal: AbortSignal.timeout(8000) },
  );
  return {
    candidates: Array.isArray(payload.candidates)
      ? payload.candidates.map(mapCandidate)
      : [],
    bestCandidateId: payload.best_candidate_id ?? null,
  };
}

export async function selectAutoResearchCandidate(
  settings: DesktopSettings,
  input: SelectCandidateInput,
): Promise<DesktopAutoResearchRun> {
  const payload = await requestJson<AutoResearchRunPayload>(
    settings,
    `/api/auto-research/runs/${encodeURIComponent(input.runId)}/select/${encodeURIComponent(input.candidateId)}`,
    {
      method: "POST",
      signal: AbortSignal.timeout(8000),
    },
  );
  return mapAutoResearchRun(payload);
}

export async function listMcpServers(
  settings: DesktopSettings,
): Promise<DesktopMcpServer[]> {
  const payload = await requestJson<McpServersPayload>(
    settings,
    "/api/mcp-servers",
    {
      signal: AbortSignal.timeout(8000),
    },
  );

  return Array.isArray(payload.servers)
    ? payload.servers.map(mapMcpServer)
    : [];
}

export async function listCustomAgents(
  settings: DesktopSettings,
): Promise<DesktopCustomAgent[]> {
  const payload = await requestJson<CustomAgentsPayload>(
    settings,
    "/api/custom-agents",
    {
      signal: AbortSignal.timeout(8000),
    },
  );

  return Array.isArray(payload.agents)
    ? payload.agents.map(mapCustomAgent)
    : [];
}

export async function listTeams(
  settings: DesktopSettings,
): Promise<DesktopTeam[]> {
  const payload = await requestJson<TeamsPayload>(settings, "/api/teams", {
    signal: AbortSignal.timeout(8000),
  });

  return Array.isArray(payload.teams) ? payload.teams.map(mapTeam) : [];
}

export async function createCustomAgent(
  settings: DesktopSettings,
  input: CreateCustomAgentInput,
): Promise<DesktopCustomAgent> {
  const payload = await requestJson<CustomAgentPayload>(
    settings,
    "/api/custom-agents",
    {
      method: "POST",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        agent_id: input.agentId,
        display_name: input.displayName,
        provider_type: input.providerType,
        model: input.model,
        system_prompt: input.systemPrompt,
      }),
    },
  );

  return mapCustomAgent(payload);
}

export async function updateCustomAgent(
  settings: DesktopSettings,
  input: UpdateCustomAgentInput,
): Promise<DesktopCustomAgent> {
  const payload = await requestJson<CustomAgentPayload>(
    settings,
    `/api/custom-agents/${encodeURIComponent(input.currentAgentId)}`,
    {
      method: "PUT",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        agent_id: input.agentId,
        display_name: input.displayName,
        provider_type: input.providerType,
        model: input.model,
        system_prompt: input.systemPrompt,
      }),
    },
  );

  return mapCustomAgent(payload);
}

export async function deleteCustomAgent(
  settings: DesktopSettings,
  input: DeleteCustomAgentInput,
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/custom-agents/${encodeURIComponent(input.agentId)}`,
    {
      method: "DELETE",
      signal: AbortSignal.timeout(8000),
    },
  );
}

export async function createTeam(
  settings: DesktopSettings,
  input: CreateTeamInput,
): Promise<DesktopTeam> {
  const payload = await requestJson<TeamPayload>(settings, "/api/teams", {
    method: "POST",
    signal: AbortSignal.timeout(8000),
    body: JSON.stringify({
      teamId: input.teamId,
      displayName: input.displayName,
      leaderAgentId: input.leaderAgentId,
      memberAgentIds: input.memberAgentIds,
      workflowText: input.workflowText,
    }),
  });
  return mapTeam(payload);
}

export async function updateTeam(
  settings: DesktopSettings,
  input: UpdateTeamInput,
): Promise<DesktopTeam> {
  const payload = await requestJson<TeamPayload>(
    settings,
    `/api/teams/${encodeURIComponent(input.currentTeamId)}`,
    {
      method: "PUT",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        teamId: input.teamId,
        displayName: input.displayName,
        leaderAgentId: input.leaderAgentId,
        memberAgentIds: input.memberAgentIds,
        workflowText: input.workflowText,
      }),
    },
  );
  return mapTeam(payload);
}

export async function deleteTeam(
  settings: DesktopSettings,
  input: DeleteTeamInput,
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/teams/${encodeURIComponent(input.teamId)}`,
    {
      method: "DELETE",
      signal: AbortSignal.timeout(8000),
    },
  );
}

export async function createMcpServer(
  settings: DesktopSettings,
  input: UpsertMcpServerInput,
): Promise<DesktopMcpServer> {
  const payload = await requestJson<McpServerPayload>(
    settings,
    "/api/mcp-servers",
    {
      method: "POST",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        name: input.name,
        transport: input.transport,
        command: input.command || "",
        args: input.args || [],
        env: input.env || {},
        enabled: input.enabled,
        working_dir: input.workingDir || null,
        url: input.url || null,
        headers: input.headers || {},
      }),
    },
  );

  return mapMcpServer(payload);
}

export async function updateMcpServer(
  settings: DesktopSettings,
  input: UpdateMcpServerInput,
): Promise<DesktopMcpServer> {
  const payload = await requestJson<McpServerPayload>(
    settings,
    `/api/mcp-servers/${encodeURIComponent(input.currentName)}`,
    {
      method: "PUT",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        name: input.name,
        transport: input.transport,
        command: input.command || "",
        args: input.args || [],
        env: input.env || {},
        enabled: input.enabled,
        working_dir: input.workingDir || null,
        url: input.url || null,
        headers: input.headers || {},
      }),
    },
  );

  return mapMcpServer(payload);
}

export async function deleteMcpServer(
  settings: DesktopSettings,
  input: DeleteMcpServerInput,
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/mcp-servers/${encodeURIComponent(input.name)}`,
    {
      method: "DELETE",
      signal: AbortSignal.timeout(8000),
    },
  );
}

export async function toggleMcpServer(
  settings: DesktopSettings,
  input: ToggleMcpServerInput,
): Promise<DesktopMcpServer> {
  const payload = await requestJson<McpServerPayload>(
    settings,
    `/api/mcp-servers/${encodeURIComponent(input.name)}/toggle`,
    {
      method: "PATCH",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        enabled: input.enabled,
      }),
    },
  );

  return mapMcpServer(payload);
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
    { signal: AbortSignal.timeout(8000) },
  );
  return Array.isArray(payload.bots) ? payload.bots : [];
}

export async function fetchBotConsoles(
  settings: DesktopSettings,
): Promise<DesktopBotConsoleSummary[]> {
  const payload = await requestJson<{ bots?: BotConsoleSummaryPayload[] }>(
    settings,
    "/api/bot-consoles",
    { signal: AbortSignal.timeout(8000) },
  );
  return Array.isArray(payload.bots)
    ? payload.bots.map(mapBotConsoleSummary)
    : [];
}

export async function fetchAutomations(
  settings: DesktopSettings,
): Promise<DesktopAutomationSummary[]> {
  const payload = await requestJson<AutomationsPayload>(
    settings,
    "/api/automations",
    {
      signal: AbortSignal.timeout(8000),
    },
  );

  return Array.isArray(payload.automations)
    ? payload.automations.map(mapAutomationSummary)
    : [];
}

export async function createRemoteAutomation(
  settings: DesktopSettings,
  input: {
    label: string;
    prompt: string;
    agentId: string;
    workspacePath: string;
    schedule: DesktopAutomationSchedule;
  },
): Promise<DesktopAutomationSummary> {
  const payload = await requestJson<AutomationSummaryPayload>(
    settings,
    "/api/automations",
    {
      method: "POST",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        label: input.label,
        prompt: input.prompt,
        agentId: input.agentId,
        workspaceDir: input.workspacePath,
        schedule: input.schedule,
      }),
    },
  );
  return mapAutomationSummary(payload);
}

export async function updateRemoteAutomation(
  settings: DesktopSettings,
  automationId: string,
  input: {
    label?: string;
    prompt?: string;
    agentId?: string;
    workspacePath?: string;
    schedule?: DesktopAutomationSchedule;
    enabled?: boolean;
  },
): Promise<DesktopAutomationSummary> {
  const payload = await requestJson<AutomationSummaryPayload>(
    settings,
    `/api/automations/${encodeURIComponent(automationId)}`,
    {
      method: "PATCH",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        label: input.label,
        prompt: input.prompt,
        agentId: input.agentId,
        workspaceDir: input.workspacePath,
        schedule: input.schedule,
        enabled: input.enabled,
      }),
    },
  );
  return mapAutomationSummary(payload);
}

export async function deleteRemoteAutomation(
  settings: DesktopSettings,
  automationId: string,
): Promise<void> {
  await requestJson<unknown>(
    settings,
    `/api/automations/${encodeURIComponent(automationId)}`,
    {
      method: "DELETE",
      signal: AbortSignal.timeout(8000),
    },
  );
}

export async function fetchAutomationActivity(
  settings: DesktopSettings,
  automationId: string,
): Promise<DesktopAutomationActivityFeed> {
  const payload = await requestJson<AutomationActivityPayload>(
    settings,
    `/api/automations/${encodeURIComponent(automationId)}/activity`,
    {
      signal: AbortSignal.timeout(8000),
    },
  );

  return {
    automationId,
    threadId: payload.threadId || "",
    count:
      typeof payload.count === "number" && Number.isFinite(payload.count)
        ? payload.count
        : 0,
    items: Array.isArray(payload.items)
      ? payload.items.map(mapAutomationActivityEntry)
      : [],
  };
}

export async function runRemoteAutomationNow(
  settings: DesktopSettings,
  automationId: string,
): Promise<DesktopAutomationActivityEntry> {
  const payload = await requestJson<{
    runId?: string;
    status?: string | null;
    startedAt?: string | null;
    finishedAt?: string | null;
    durationMs?: number | null;
    excerpt?: string | null;
    threadId?: string | null;
  }>(settings, `/api/automations/${encodeURIComponent(automationId)}/run-now`, {
    method: "POST",
    signal: AbortSignal.timeout(8000),
  });

  return {
    runId: payload.runId || "",
    status: normalizeAutomationStatus(payload.status),
    startedAt: payload.startedAt || new Date(0).toISOString(),
    finishedAt: payload.finishedAt ?? null,
    durationMs:
      typeof payload.durationMs === "number" &&
      Number.isFinite(payload.durationMs)
        ? payload.durationMs
        : null,
    excerpt:
      typeof payload.excerpt === "string" && payload.excerpt.trim()
        ? payload.excerpt.trim()
        : null,
    threadId: payload.threadId || "",
  };
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

export async function openChatStream(
  settings: DesktopSettings,
  input: SendMessageInput,
  onEvent: (event: DesktopChatStreamEvent) => void,
  workspacePath?: string | null,
): Promise<{
  runId: string;
  threadId: string;
  sessionId?: string;
  response: string;
  status: OpenChatStreamResult["status"];
}> {
  const threadId = resolveInputThreadId(input);
  if (activeStreamRequests.has(threadId)) {
    throw new Error("This thread already has an active stream");
  }

  const providerMetadata = buildProviderMetadata(settings);
  const serializedAttachments = serializeMessageAttachments(
    input.images,
    input.files,
  );
  const retryDelaysMs = [300, 700, 1400];
  let lastError: Error | null = null;

  for (let attempt = 0; attempt <= retryDelaysMs.length; attempt += 1) {
    let sawRemoteStreamEvent = false;
    try {
      return await new Promise((resolve, reject) => {
        const socket = new WebSocket(
          buildWebSocketUrl(settings, "/api/chat/ws"),
        );
        const active: ActiveChatSocket = {
          socket,
          threadId,
          runId: "",
          responseText: "",
          sawTerminal: false,
          capturePrimaryResponse: true,
          pendingInputWaiters: [],
          pendingInterruptWaiters: [],
        };
        activeStreamRequests.set(threadId, active);

        let settled = false;
        let closeReason: string | null = null;
        let didOpen = false;
        const settleError = (message: string) => {
          if (settled) {
            return;
          }
          settled = true;
          closeReason = message;
          try {
            socket.close();
          } catch {
            // no-op
          }
          reject(new Error(message));
        };
        const settleSuccess = () => {
          if (settled) {
            return;
          }
          settled = true;
          resolve({
            runId: active.runId,
            threadId: active.threadId,
            sessionId: active.threadId,
            response: active.responseText,
            status: active.sawTerminal ? "completed" : "disconnected",
          });
        };

        socket.addEventListener("open", () => {
          didOpen = true;
          socket.send(
            JSON.stringify({
              op: "start",
              message: input.message,
              attachments: serializedAttachments.attachments,
              images: serializedAttachments.images,
              files: serializedAttachments.files,
              threadId,
              accountId: settings.accountId,
              fromId: settings.fromId,
              waitForResponse: false,
              timeoutSeconds: settings.timeoutSeconds,
              workspacePath: workspacePath || undefined,
              metadata: {
                client_timestamp_local: formatLocalChatTimestamp(),
              },
              providerMetadata,
            }),
          );
        });

        socket.addEventListener("message", (event) => {
          const raw = typeof event.data === "string" ? event.data : "";
          if (!raw.trim()) {
            return;
          }
          let payload: Record<string, unknown>;
          try {
            payload = parseJson<Record<string, unknown>>(raw);
          } catch {
            settleError("invalid websocket payload");
            return;
          }
          const type = asString(payload.type);
          if (!type || type === "ping") {
            return;
          }

          const payloadRunId = asString(payload.runId) || active.runId;
          const payloadThreadId =
            asString(payload.threadId) ||
            asString(payload.sessionKey) ||
            active.threadId;

          active.runId = payloadRunId;
          active.threadId = payloadThreadId;

          switch (type) {
            case "accepted":
              sawRemoteStreamEvent = true;
              onEvent({
                type: "accepted",
                runId: payloadRunId,
                threadId: payloadThreadId,
                sessionId: payloadThreadId,
              });
              return;
            case "assistant_delta": {
              const delta = asString(payload.delta) || "";
              if (!delta) {
                return;
              }
              const metadata = parseRecord(payload.metadata);
              sawRemoteStreamEvent = true;
              if (active.capturePrimaryResponse) {
                active.responseText += delta;
              }
              onEvent({
                type: "assistant_delta",
                runId: payloadRunId,
                threadId: payloadThreadId,
                sessionId: payloadThreadId,
                delta,
                metadata: Object.keys(metadata).length ? metadata : null,
              });
              return;
            }
            case "assistant_boundary":
              sawRemoteStreamEvent = true;
              if (active.capturePrimaryResponse) {
                active.responseText = appendStreamResponseSeparator(
                  active.responseText,
                );
              }
              onEvent({
                type: "assistant_boundary",
                runId: payloadRunId,
                threadId: payloadThreadId,
                sessionId: payloadThreadId,
              });
              return;
            case "tool_use":
            case "tool_result":
              sawRemoteStreamEvent = true;
              onEvent({
                type,
                runId: payloadRunId,
                threadId: payloadThreadId,
                sessionId: payloadThreadId,
                message: mapStreamToolMessage(payload.message),
              });
              return;
            case "user_ack":
              sawRemoteStreamEvent = true;
              active.capturePrimaryResponse = false;
              onEvent({
                type: "user_ack",
                runId: payloadRunId,
                threadId: payloadThreadId,
                sessionId: payloadThreadId,
                pendingInputId:
                  asString(payload.pendingInputId) ||
                  asString(payload.pending_input_id) ||
                  undefined,
              });
              return;
            case "done":
              sawRemoteStreamEvent = true;
              active.sawTerminal = true;
              onEvent({
                type: "done",
                runId: payloadRunId,
                threadId: payloadThreadId,
                sessionId: payloadThreadId,
              });
              socket.close();
              return;
            case "stream_input": {
              const waiter = active.pendingInputWaiters.shift();
              if (!waiter) {
                return;
              }
              waiter.resolve({
                status: asString(payload.status) || "no_active_session",
                threadId: payloadThreadId,
                sessionId: payloadThreadId,
                pendingInputId:
                  asString(payload.pendingInputId) ||
                  asString(payload.pending_input_id),
              });
              return;
            }
            case "interrupt": {
              const waiter = active.pendingInterruptWaiters.shift();
              if (!waiter) {
                return;
              }
              waiter.resolve({
                status: asString(payload.status) || "ok",
                threadId: payloadThreadId,
                sessionId: payloadThreadId,
                abortedRuns: Array.isArray(payload.abortedRuns)
                  ? payload.abortedRuns.map((entry) => String(entry))
                  : [],
              });
              socket.close();
              return;
            }
            case "error": {
              sawRemoteStreamEvent = true;
              const error = asString(payload.error) || "agent run failed";
              onEvent({
                type: "error",
                runId: payloadRunId,
                threadId: payloadThreadId,
                sessionId: payloadThreadId,
                error,
              });
              const pendingInput = active.pendingInputWaiters.splice(0);
              for (const waiter of pendingInput) {
                waiter.reject(new Error(error));
              }
              const pendingInterrupt = active.pendingInterruptWaiters.splice(0);
              for (const waiter of pendingInterrupt) {
                waiter.reject(new Error(error));
              }
              settleError(error);
              return;
            }
            default:
              return;
          }
        });

        socket.addEventListener("error", () => {
          settleError(
            didOpen ? "stream disconnected" : "websocket connect failed",
          );
        });

        socket.addEventListener("close", () => {
          const activeEntry = activeStreamRequests.get(threadId);
          if (activeEntry?.socket === socket) {
            activeStreamRequests.delete(threadId);
          }
          const pendingInput = active.pendingInputWaiters.splice(0);
          for (const waiter of pendingInput) {
            waiter.reject(new Error(closeReason || "stream disconnected"));
          }
          const pendingInterrupt = active.pendingInterruptWaiters.splice(0);
          for (const waiter of pendingInterrupt) {
            waiter.reject(new Error(closeReason || "stream disconnected"));
          }
          if (closeReason) {
            return;
          }
          if (!didOpen && !sawRemoteStreamEvent) {
            settleError("websocket connect failed");
            return;
          }
          settleSuccess();
        });
      });
    } catch (error) {
      lastError = error instanceof Error ? error : new Error(String(error));
      if (sawRemoteStreamEvent || attempt >= retryDelaysMs.length) {
        throw lastError;
      }
      await new Promise((resolve) =>
        setTimeout(resolve, retryDelaysMs[attempt]),
      );
    }
  }
  throw lastError || new Error("websocket connect failed");
}

export async function sendStreamingInput(
  _settings: DesktopSettings,
  input: SendMessageInput,
): Promise<SendStreamingInputResult> {
  const threadId = resolveInputThreadId(input);
  const active = activeStreamRequests.get(threadId);
  if (active && active.socket.readyState === WebSocket.OPEN) {
    const serializedAttachments = serializeMessageAttachments(
      input.images,
      input.files,
    );
    return await new Promise((resolve, reject) => {
      const waiter: StreamInputWaiter = {
        resolve: (result) => {
          clearTimeout(timeout);
          resolve(result);
        },
        reject: (error) => {
          clearTimeout(timeout);
          reject(error);
        },
      };
      const timeout = setTimeout(() => {
        const index = active.pendingInputWaiters.indexOf(waiter);
        if (index >= 0) {
          active.pendingInputWaiters.splice(index, 1);
        }
        reject(new Error("stream input timed out"));
      }, 8000);
      active.pendingInputWaiters.push(waiter);
      active.socket.send(
        JSON.stringify({
          op: "input",
          threadId,
          message: input.message,
          attachments: serializedAttachments.attachments,
          images: serializedAttachments.images,
          files: serializedAttachments.files,
        }),
      );
    });
  }
  return {
    status: "no_active_session",
    threadId,
    sessionId: input.sessionId || threadId,
  };
}

export async function interruptThread(
  _settings: DesktopSettings,
  threadId: string,
): Promise<InterruptResult> {
  const active = activeStreamRequests.get(threadId);
  if (active && active.socket.readyState === WebSocket.OPEN) {
    try {
      const result = await new Promise<InterruptResult>((resolve, reject) => {
        const waiter: InterruptWaiter = {
          resolve: (payload) => {
            clearTimeout(timeout);
            resolve(payload);
          },
          reject: (error) => {
            clearTimeout(timeout);
            reject(error);
          },
        };
        const timeout = setTimeout(() => {
          const index = active.pendingInterruptWaiters.indexOf(waiter);
          if (index >= 0) {
            active.pendingInterruptWaiters.splice(index, 1);
          }
          reject(new Error("interrupt timed out"));
        }, 5000);
        active.pendingInterruptWaiters.push(waiter);
        active.socket.send(
          JSON.stringify({
            op: "interrupt",
            threadId,
          }),
        );
      });
      active.socket.close();
      activeStreamRequests.delete(threadId);
      return result;
    } catch {
      active.socket.close();
      activeStreamRequests.delete(threadId);
    }
  }
  return {
    status: "local_abort_only",
    threadId,
    sessionId: threadId,
    abortedRuns: [],
  };
}

export const interruptSession = interruptThread;
