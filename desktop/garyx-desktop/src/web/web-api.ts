import type {
  DesktopBotConversationNode,
  DesktopThreadSummary,
  DesktopBotConsoleSummary,
  DesktopChannelEndpoint,
  GatewayConfigDocument,
  GatewaySettingsPayload,
  GatewaySettingsSaveResult,
} from '@shared/contracts';

type ChannelEndpointsResponse = {
  endpoints?: DesktopChannelEndpoint[];
};

type BotConsolesResponse = {
  bots?: Array<Record<string, unknown>>;
};

type GatewayOverviewResponse = Record<string, unknown>;

type AgentViewResponse = Record<string, unknown>;
type AgentCatalogDefaults = {
  effectiveDefaultAgentId: string | null;
};

type ThreadSummaryPayload = {
  thread_key?: string;
  session_key?: string;
  thread_id?: string;
  label?: string | null;
  workspace_dir?: string | null;
  updated_at?: string | null;
  created_at?: string | null;
  message_count?: number;
  last_user_message?: string | null;
  last_assistant_message?: string | null;
  recent_run_id?: string | null;
  recentRunId?: string | null;
  run_state?: string | null;
  runState?: string | null;
};

type ThreadsPayload = {
  threads?: ThreadSummaryPayload[];
  sessions?: ThreadSummaryPayload[];
};

export type LogTailPayload = {
  path?: string;
  total_lines?: number;
  lines?: Array<string | Record<string, unknown>>;
};

export type ParsedLogLine = {
  level: string;
  timestamp: string;
  message: string;
};

export type CronJobsPayload = {
  jobs?: Array<{
    id?: string;
    schedule?: {
      cron?: string;
      interval_secs?: number;
      at?: string;
    } | string | null;
    enabled?: boolean;
    next_run?: string | null;
    last_run_at?: string | null;
    last_status?: string | null;
    run_count?: number;
  }>;
  count?: number;
};

export type CronRunsPayload = {
  runs?: Array<{
    run_id?: string;
    job_id?: string;
    status?: string;
    started_at?: string | null;
    finished_at?: string | null;
    duration_ms?: number | null;
    error?: string | null;
  }>;
  count?: number;
  total?: number;
};

function trimTrailingSlashes(value: string): string {
  return value.trim().replace(/\/+$/, '');
}

import {
  parseDirectoryListingPayload,
  parseWorkspaceCatalogPayload,
} from '../shared/workspace-payload.ts';
import type {
  DesktopLocalDirectoryListing,
  DesktopWorkspaceCatalog,
} from '../shared/contracts/workspace.ts';

export function resolveGatewayBase(): string {
  const url = new URL(window.location.href);
  return trimTrailingSlashes(url.searchParams.get('gateway') || window.location.origin);
}

async function requestJson<T>(
  path: string,
  semantics: 'readRetryable' | 'mutationSingleAttempt',
  init: RequestInit = {},
): Promise<T> {
  const method = (init.method || 'GET').toUpperCase();
  const isRead = method === 'GET' || method === 'HEAD';
  if (isRead !== (semantics === 'readRetryable')) {
    throw new TypeError(
      `${method} requests must use ${isRead ? 'readRetryable' : 'mutationSingleAttempt'} semantics.`,
    );
  }
  const response = await fetch(`${resolveGatewayBase()}${path}`, {
    ...init,
    headers: {
      'Content-Type': 'application/json',
      ...(init?.headers || {}),
    },
  });
  if (!response.ok) {
    throw new Error(`${response.status} ${response.statusText}`);
  }
  return response.json() as Promise<T>;
}

export async function fetchWorkspaceCatalog(): Promise<DesktopWorkspaceCatalog> {
  const payload = await requestJson<unknown>('/api/workspaces', 'readRetryable');
  return parseWorkspaceCatalogPayload(payload);
}

export async function addWorkspace(input: {
  path: string;
  name?: string | null;
}): Promise<DesktopWorkspaceCatalog> {
  const payload = await requestJson<unknown>('/api/workspaces', 'mutationSingleAttempt', {
    method: 'POST',
    body: JSON.stringify({ path: input.path, name: input.name || undefined }),
  });
  return parseWorkspaceCatalogPayload(payload);
}

export async function pinWorkspace(input: {
  path: string;
  pinned: boolean;
}): Promise<DesktopWorkspaceCatalog> {
  const payload = await requestJson<unknown>('/api/workspaces/pin', 'mutationSingleAttempt', {
    method: 'POST',
    body: JSON.stringify(input),
  });
  return parseWorkspaceCatalogPayload(payload);
}

export async function renameWorkspace(input: {
  path: string;
  name: string;
}): Promise<DesktopWorkspaceCatalog> {
  const payload = await requestJson<unknown>('/api/workspaces/rename', 'mutationSingleAttempt', {
    method: 'POST',
    body: JSON.stringify(input),
  });
  return parseWorkspaceCatalogPayload(payload);
}

export async function removeWorkspace(input: { path: string }): Promise<DesktopWorkspaceCatalog> {
  const query = new URLSearchParams({ path: input.path });
  const payload = await requestJson<unknown>(
    `/api/workspaces?${query.toString()}`,
    'mutationSingleAttempt',
    { method: 'DELETE' },
  );
  return parseWorkspaceCatalogPayload(payload);
}

export async function listWorkspaceDirectories(input?: {
  path?: string | null;
}): Promise<DesktopLocalDirectoryListing> {
  const query = new URLSearchParams();
  if (input?.path?.trim()) {
    query.set('path', input.path.trim());
  }
  const suffix = query.toString() ? `?${query.toString()}` : '';
  const payload = await requestJson<unknown>(
    `/api/workspaces/directories${suffix}`,
    'readRetryable',
  );
  return parseDirectoryListingPayload(payload);
}

export async function fetchChannelEndpoints(): Promise<DesktopChannelEndpoint[]> {
  const payload = await requestJson<ChannelEndpointsResponse>(
    '/api/channel-endpoints',
    'readRetryable',
  );
  return Array.isArray(payload.endpoints) ? payload.endpoints : [];
}

export async function fetchBotConsoles(): Promise<DesktopBotConsoleSummary[]> {
  const payload = await requestJson<BotConsolesResponse>(
    '/api/bot-consoles',
    'readRetryable',
  );
  return Array.isArray(payload.bots)
    ? payload.bots
        .map(mapBotConsoleSummary)
        .filter((value): value is DesktopBotConsoleSummary => Boolean(value))
    : [];
}

function stringOrNull(value: unknown): string | null {
  return typeof value === 'string' ? value : null;
}

function stringOrEmpty(value: unknown): string {
  return typeof value === 'string' ? value : '';
}

function numberOrZero(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : 0;
}

function mapBotConsoleEndpoint(value: unknown): DesktopChannelEndpoint | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return null;
  }
  const record = value as Record<string, unknown>;
  const endpointKey = stringOrEmpty(record.endpoint_key);
  if (!endpointKey) {
    return null;
  }
  return {
    endpointKey,
    channel: stringOrEmpty(record.channel),
    accountId: stringOrEmpty(record.account_id),
    peerId: stringOrEmpty(record.peer_id),
    chatId: stringOrEmpty(record.chat_id),
    deliveryTargetType: record.delivery_target_type === 'open_id' ? 'open_id' : 'chat_id',
    deliveryTargetId: stringOrEmpty(record.delivery_target_id) || stringOrEmpty(record.chat_id),
    threadScope: stringOrNull(record.thread_scope),
    displayLabel: stringOrEmpty(record.display_label),
    threadId: stringOrNull(record.thread_id),
    threadLabel: stringOrNull(record.thread_label),
    workspacePath: stringOrNull(record.workspace_dir),
    threadUpdatedAt: stringOrNull(record.thread_updated_at),
    lastInboundAt: stringOrNull(record.last_inbound_at),
    lastDeliveryAt: stringOrNull(record.last_delivery_at),
    conversationKind:
      record.conversation_kind === 'private'
      || record.conversation_kind === 'group'
      || record.conversation_kind === 'topic'
      || record.conversation_kind === 'unknown'
        ? record.conversation_kind
        : null,
    conversationLabel: stringOrNull(record.conversation_label),
  };
}

function mapBotConversationNode(value: unknown): DesktopBotConversationNode | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return null;
  }
  const record = value as Record<string, unknown>;
  const id = stringOrEmpty(record.id);
  const endpoint = mapBotConsoleEndpoint(record.endpoint);
  if (!id || !endpoint) {
    return null;
  }
  return {
    id,
    endpoint,
    kind: stringOrEmpty(record.kind) || 'unknown',
    title: stringOrEmpty(record.title) || endpoint.displayLabel || id,
    badge: stringOrNull(record.badge),
    latestActivity: stringOrNull(record.latest_activity),
    openable: Boolean(record.openable),
  };
}

function mapBotConsoleSummary(value: unknown): DesktopBotConsoleSummary | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return null;
  }
  const record = value as Record<string, unknown>;
  const id = stringOrEmpty(record.id);
  if (!id) {
    return null;
  }
  const endpoints = Array.isArray(record.endpoints)
    ? record.endpoints
        .map(mapBotConsoleEndpoint)
        .filter((item): item is DesktopChannelEndpoint => Boolean(item))
    : [];
  const conversationNodes = Array.isArray(record.conversation_nodes)
    ? record.conversation_nodes
        .map(mapBotConversationNode)
        .filter((item): item is DesktopBotConversationNode => Boolean(item))
    : [];
  return {
    id,
    channel: stringOrEmpty(record.channel),
    accountId: stringOrEmpty(record.account_id),
    title: stringOrEmpty(record.title) || id,
    subtitle: stringOrEmpty(record.subtitle),
    rootBehavior: record.root_behavior === 'expand_only' ? 'expand_only' : 'open_default',
    status: record.status === 'connected' ? 'connected' : 'idle',
    latestActivity: stringOrNull(record.latest_activity),
    endpointCount: numberOrZero(record.endpoint_count),
    boundEndpointCount: numberOrZero(record.bound_endpoint_count),
    agentId: stringOrNull(record.agent_id)?.trim() || null,
    effectiveAgentId: stringOrNull(record.effective_agent_id)?.trim() || null,
    workspaceDir: stringOrNull(record.workspace_dir),
    mainEndpointStatus: record.main_endpoint_status === 'resolved' ? 'resolved' : 'unresolved',
    mainEndpoint: mapBotConsoleEndpoint(record.main_endpoint),
    mainThreadId: stringOrNull(record.main_endpoint_thread_id)
      || stringOrNull((record.main_endpoint as Record<string, unknown> | null)?.thread_id)
      || null,
    defaultOpenEndpoint: mapBotConsoleEndpoint(record.default_open_endpoint),
    defaultOpenThreadId: stringOrNull(record.default_open_thread_id)
      || stringOrNull((record.default_open_endpoint as Record<string, unknown> | null)?.thread_id)
      || null,
    conversationNodes,
    endpoints,
  };
}

export async function fetchOverview(): Promise<GatewayOverviewResponse> {
  return requestJson<GatewayOverviewResponse>('/api/overview', 'readRetryable');
}

export async function fetchAgentView(): Promise<AgentViewResponse> {
  return requestJson<AgentViewResponse>('/api/agent-view', 'readRetryable');
}

export async function fetchAgentCatalogDefaults(): Promise<AgentCatalogDefaults> {
  const payload = await requestJson<Record<string, unknown>>(
    '/api/custom-agents',
    'readRetryable',
  );
  return {
    effectiveDefaultAgentId:
      stringOrNull(payload.effective_default_agent_id)?.trim() || null,
  };
}

export async function fetchCronJobs(): Promise<CronJobsPayload> {
  return requestJson<CronJobsPayload>(
    '/api/cron/jobs?limit=200',
    'readRetryable',
  );
}

export async function fetchCronRuns(): Promise<CronRunsPayload> {
  return requestJson<CronRunsPayload>(
    '/api/cron/runs?limit=120',
    'readRetryable',
  );
}

function mapThreadSummary(value: ThreadSummaryPayload): DesktopThreadSummary {
  const id = value.thread_id || value.thread_key || value.session_key || '';
  const preview = value.last_assistant_message || value.last_user_message || '';
  return {
    id,
    title: value.label || id,
    createdAt: value.created_at || '',
    updatedAt: value.updated_at || value.created_at || '',
    lastMessagePreview: preview,
    workspacePath: value.workspace_dir || null,
    messageCount: value.message_count,
    recentRunId: value.recent_run_id || value.recentRunId || null,
    runState: value.run_state || value.runState || null,
  };
}

export async function fetchThreads(): Promise<DesktopThreadSummary[]> {
  const payload = await requestJson<ThreadsPayload>('/api/threads', 'readRetryable');
  const items = Array.isArray(payload.threads)
    ? payload.threads
    : Array.isArray(payload.sessions)
      ? payload.sessions
      : [];
  return items
    .map(mapThreadSummary)
    .filter((thread) => Boolean(thread.id));
}

export async function fetchGatewaySettings(): Promise<GatewaySettingsPayload> {
  const payload = await requestJson<GatewaySettingsPayload>(
    '/api/settings',
    'readRetryable',
  );
  return {
    config: payload?.config && typeof payload.config === 'object' ? payload.config : {},
    source: payload?.source || 'gateway_api',
    secretsMasked: payload?.secretsMasked !== false,
  };
}

export async function saveGatewaySettings(
  config: GatewayConfigDocument,
): Promise<GatewaySettingsSaveResult> {
  const result = await requestJson<{
    ok?: boolean;
    message?: string;
    errors?: string[];
  }>(
    '/api/settings?merge=false',
    'mutationSingleAttempt',
    {
      method: 'PUT',
      body: JSON.stringify(config || {}),
    },
  );

  return {
    ok: Boolean(result.ok),
    message: result.message,
    errors: Array.isArray(result.errors)
      ? result.errors.filter((value): value is string => typeof value === 'string')
      : undefined,
    settings: await fetchGatewaySettings(),
  };
}

function asString(value: unknown): string {
  return typeof value === 'string' ? value.trim() : '';
}

function stripAnsi(value: string): string {
  return value.replace(/\x1b\[[0-9;]*m/g, '');
}

export function parseLogLine(line: string | Record<string, unknown>): ParsedLogLine {
  if (line && typeof line === 'object' && !Array.isArray(line)) {
    return {
      level: asString(line.level) || 'INFO',
      timestamp: asString(line.timestamp) || '--',
      message: asString(line.message) || asString(line.raw) || '',
    };
  }

  const clean = stripAnsi(String(line));
  const match = clean.match(/^(\d{4}-\d{2}-\d{2}T[\d:.]+Z?)\s+(ERROR|WARN|WARNING|INFO|DEBUG|TRACE)\s+(.*)$/);
  if (match) {
    return {
      timestamp: match[1],
      level: match[2],
      message: match[3],
    };
  }
  return {
    level: 'INFO',
    timestamp: '--',
    message: clean,
  };
}

export async function fetchLogsTail(level = ''): Promise<LogTailPayload> {
  const query = new URLSearchParams({
    lines: '160',
  });
  if (level) {
    query.set('pattern', level === 'WARNING' ? 'WARN|WARNING' : level);
  }
  return requestJson<LogTailPayload>(
    `/api/logs/tail?${query.toString()}`,
    'readRetryable',
  );
}
