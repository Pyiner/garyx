import { createHash, randomUUID } from 'node:crypto';
import { access, mkdir, readFile, realpath, rename, writeFile } from 'node:fs/promises';
import { dirname, join, basename, resolve as resolvePath } from 'node:path';

import { app } from 'electron';

import {
  DEFAULT_DESKTOP_SETTINGS,
  DEFAULT_SESSION_TITLE,
  type CreateAutomationInput,
  type DesktopAutomationActivityEntry,
  type DesktopAutomationSummary,
  type DesktopBotConsoleSummary,
  type ConfiguredBot,
  type DesktopGatewayProfile,
  type DesktopLanguagePreference,
  type DesktopRemoteStateError,
  type DesktopThreadSummary,
  type DesktopSettings,
  type DesktopState,
  type DesktopWorkspace,
  type DesktopChannelEndpoint,
  type DesktopSessionProviderHint,
} from '@shared/contracts';
import {
  createRemoteAutomation,
  createRemoteThread,
  deleteRemoteAutomation,
  deleteRemoteThread,
  fetchAutomations,
  fetchBotConsoles,
  fetchChannelEndpoints,
  fetchConfiguredBots,
  fetchThreads,
  mapChannelEndpoint,
  runRemoteAutomationNow,
  updateRemoteAutomation,
  updateRemoteThread,
} from './gary-client';
const STATE_FILE_NAME = 'garyx-desktop-state.json';
const LEGACY_STATE_FILE_NAME = 'garyx-desktop-state.json';
const MAX_GATEWAY_PROFILES = 12;
const LEGACY_DEFAULT_GATEWAY_URLS = new Set([
  'http://127.0.0.1:3000',
  'http://localhost:3000',
]);

function stateFilePath(): string {
  return join(app.getPath('userData'), STATE_FILE_NAME);
}

function legacyStateFilePath(): string {
  return join(app.getPath('userData'), LEGACY_STATE_FILE_NAME);
}

async function migrateLegacyStateFile(): Promise<void> {
  const nextPath = stateFilePath();
  const legacyPath = legacyStateFilePath();

  try {
    await access(nextPath);
    return;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') {
      throw error;
    }
  }

  try {
    await access(legacyPath);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
      return;
    }
    throw error;
  }

  await mkdir(dirname(nextPath), { recursive: true });
  try {
    await rename(legacyPath, nextPath);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'EEXIST') {
      return;
    }
    throw error;
  }
}

function sortThreads(threads: DesktopThreadSummary[]): DesktopThreadSummary[] {
  return [...threads].sort((left, right) => {
    return Date.parse(right.updatedAt) - Date.parse(left.updatedAt);
  });
}

function normalizeWorkspacePathKey(path: string): string {
  return path.trim().toLowerCase();
}

function buildManagedWorkspaceId(path: string): string {
  const digest = createHash('sha1').update(normalizeWorkspacePathKey(path)).digest('hex');
  return `workspace::managed::${digest}`;
}

function sortAutomations(automations: DesktopAutomationSummary[]): DesktopAutomationSummary[] {
  return [...automations].sort((left, right) => {
    if (left.enabled !== right.enabled) {
      return left.enabled ? -1 : 1;
    }

    const leftNext = Date.parse(left.nextRun) || Number.POSITIVE_INFINITY;
    const rightNext = Date.parse(right.nextRun) || Number.POSITIVE_INFINITY;
    if (leftNext !== rightNext) {
      return leftNext - rightNext;
    }

    const rightLast = Date.parse(right.lastRunAt || '') || Number.NEGATIVE_INFINITY;
    const leftLast = Date.parse(left.lastRunAt || '') || Number.NEGATIVE_INFINITY;
    if (rightLast !== leftLast) {
      return rightLast - leftLast;
    }

    return left.label.localeCompare(right.label);
  });
}

function sortWorkspaces(
  workspaces: DesktopWorkspace[],
  threads: DesktopThreadSummary[],
): DesktopWorkspace[] {
  const latestByWorkspace = new Map<string, number>();

  for (const thread of threads) {
    const current = latestByWorkspace.get(thread.workspaceId) ?? Number.NEGATIVE_INFINITY;
    latestByWorkspace.set(
      thread.workspaceId,
      Math.max(current, Date.parse(thread.updatedAt) || Number.NEGATIVE_INFINITY),
    );
  }

  return [...workspaces].sort((left, right) => {
    const rightLatest =
      latestByWorkspace.get(right.id) ??
      (Date.parse(right.updatedAt) || Number.NEGATIVE_INFINITY);
    const leftLatest =
      latestByWorkspace.get(left.id) ??
      (Date.parse(left.updatedAt) || Number.NEGATIVE_INFINITY);

    if (rightLatest !== leftLatest) {
      return rightLatest - leftLatest;
    }

    if (left.kind !== right.kind) {
      return left.kind === 'local' ? -1 : 1;
    }

    return left.name.localeCompare(right.name);
  });
}

function summarizeText(value: string, limit = 88): string {
  const normalized = value.replace(/\s+/g, ' ').trim();
  if (normalized.length <= limit) {
    return normalized;
  }
  return `${normalized.slice(0, limit - 1).trimEnd()}…`;
}

function titleFromPrompt(value: string): string {
  const summary = summarizeText(value, 40);
  return summary || DEFAULT_SESSION_TITLE;
}

function normalizeNewThreadTitle(value?: string | null): string | undefined {
  const trimmed = typeof value === 'string' ? value.trim() : '';
  if (!trimmed || trimmed === DEFAULT_SESSION_TITLE) {
    return undefined;
  }
  return trimmed;
}

function normalizeWorkspacePathInput(value?: string | null): string | null {
  if (typeof value !== 'string') {
    return null;
  }
  const trimmed = value.trim();
  return trimmed || null;
}

function normalizeSdkSessionIdInput(value?: string | null): string | null {
  if (typeof value !== 'string') {
    return null;
  }
  const trimmed = value.trim();
  return trimmed || null;
}

function normalizeSdkSessionProviderHintInput(
  value?: DesktopSessionProviderHint | string | null,
): DesktopSessionProviderHint | null {
  if (typeof value !== 'string') {
    return null;
  }
  const normalized = value.trim().toLowerCase();
  switch (normalized) {
    case 'claude':
    case 'codex':
    case 'gemini':
      return normalized;
    default:
      return null;
  }
}

function normalizeSettings(value?: Partial<DesktopSettings>): DesktopSettings {
  const normalizeMultiline = (input: unknown): string => {
    return typeof input === 'string'
      ? input.replace(/\r\n?/g, '\n').trim()
      : '';
  };
  const normalizeThreadLogsPanelWidth = (input: unknown): number => {
    const numeric = Number(input);
    if (!Number.isFinite(numeric)) {
      return DEFAULT_DESKTOP_SETTINGS.threadLogsPanelWidth;
    }
    return Math.max(280, Math.min(760, Math.round(numeric)));
  };
  const normalizeLanguagePreference = (input: unknown): DesktopLanguagePreference => {
    return input === 'en' || input === 'zh-CN' || input === 'system'
      ? input
      : DEFAULT_DESKTOP_SETTINGS.languagePreference;
  };

  return {
    gatewayUrl:
      value?.gatewayUrl?.trim().replace(/\/+$/, '') || DEFAULT_DESKTOP_SETTINGS.gatewayUrl,
    gatewayAuthToken:
      typeof value?.gatewayAuthToken === 'string'
        ? value.gatewayAuthToken.trim()
        : DEFAULT_DESKTOP_SETTINGS.gatewayAuthToken,
    accountId: value?.accountId?.trim() || DEFAULT_DESKTOP_SETTINGS.accountId,
    fromId: value?.fromId?.trim() || DEFAULT_DESKTOP_SETTINGS.fromId,
    timeoutSeconds: Math.max(
      10,
      Math.min(600, Math.round(value?.timeoutSeconds ?? DEFAULT_DESKTOP_SETTINGS.timeoutSeconds)),
    ),
    providerClaudeEnv: normalizeMultiline(value?.providerClaudeEnv),
    providerCodexAuthMode:
      value?.providerCodexAuthMode === 'api_key' ? 'api_key' : DEFAULT_DESKTOP_SETTINGS.providerCodexAuthMode,
    providerCodexApiKey:
      typeof value?.providerCodexApiKey === 'string'
        ? value.providerCodexApiKey.trim()
        : DEFAULT_DESKTOP_SETTINGS.providerCodexApiKey,
    threadLogsPanelWidth: normalizeThreadLogsPanelWidth(value?.threadLogsPanelWidth),
    languagePreference: normalizeLanguagePreference(value?.languagePreference),
  };
}

function normalizeGatewayProfileUrl(value: unknown): string {
  return typeof value === 'string' ? value.trim().replace(/\/+$/, '') : '';
}

function gatewayProfileKey(gatewayUrl: string): string {
  return gatewayUrl.trim().toLowerCase();
}

function gatewayProfileId(gatewayUrl: string): string {
  const digest = createHash('sha1').update(gatewayProfileKey(gatewayUrl)).digest('hex');
  return `gateway::${digest.slice(0, 16)}`;
}

function gatewayProfileLabel(gatewayUrl: string): string {
  try {
    const parsed = new URL(gatewayUrl);
    return parsed.host || gatewayUrl;
  } catch {
    return gatewayUrl;
  }
}

function normalizeGatewayProfile(
  value?: Partial<DesktopGatewayProfile>,
): DesktopGatewayProfile | null {
  const gatewayUrl = normalizeGatewayProfileUrl(value?.gatewayUrl);
  if (!gatewayUrl) {
    return null;
  }
  const updatedAt = typeof value?.updatedAt === 'string' && value.updatedAt.trim()
    ? value.updatedAt
    : new Date(0).toISOString();
  return {
    id: typeof value?.id === 'string' && value.id.trim()
      ? value.id.trim()
      : gatewayProfileId(gatewayUrl),
    label: typeof value?.label === 'string' && value.label.trim()
      ? value.label.trim()
      : gatewayProfileLabel(gatewayUrl),
    gatewayUrl,
    gatewayAuthToken:
      typeof value?.gatewayAuthToken === 'string' ? value.gatewayAuthToken.trim() : '',
    updatedAt,
  };
}

function profileUpdatedAtMillis(profile: DesktopGatewayProfile): number {
  const millis = Date.parse(profile.updatedAt);
  return Number.isFinite(millis) ? millis : 0;
}

function normalizeGatewayProfiles(value: unknown): DesktopGatewayProfile[] {
  if (!Array.isArray(value)) {
    return [];
  }

  const byKey = new Map<string, DesktopGatewayProfile>();
  for (const entry of value) {
    const normalized = normalizeGatewayProfile(entry as Partial<DesktopGatewayProfile>);
    if (!normalized) {
      continue;
    }
    const key = gatewayProfileKey(normalized.gatewayUrl);
    const current = byKey.get(key);
    if (!current || profileUpdatedAtMillis(normalized) >= profileUpdatedAtMillis(current)) {
      byKey.set(key, normalized);
    }
  }

  return Array.from(byKey.values())
    .sort((left, right) => profileUpdatedAtMillis(right) - profileUpdatedAtMillis(left))
    .slice(0, MAX_GATEWAY_PROFILES);
}

function profileFromSettings(
  settings: DesktopSettings,
  updatedAt = new Date().toISOString(),
): DesktopGatewayProfile | null {
  const gatewayUrl = normalizeGatewayProfileUrl(settings.gatewayUrl);
  if (!gatewayUrl) {
    return null;
  }
  return {
    id: gatewayProfileId(gatewayUrl),
    label: gatewayProfileLabel(gatewayUrl),
    gatewayUrl,
    gatewayAuthToken: settings.gatewayAuthToken.trim(),
    updatedAt,
  };
}

function upsertGatewayProfile(
  profiles: DesktopGatewayProfile[],
  settings: DesktopSettings,
): DesktopGatewayProfile[] {
  const profile = profileFromSettings(settings);
  if (!profile) {
    return profiles;
  }

  const next = [
    profile,
    ...profiles.filter((entry) => (
      gatewayProfileKey(entry.gatewayUrl) !== gatewayProfileKey(profile.gatewayUrl)
    )),
  ];
  return normalizeGatewayProfiles(next);
}

async function resolveDefaultDesktopSettings(): Promise<DesktopSettings> {
  return normalizeSettings(DEFAULT_DESKTOP_SETTINGS);
}

function upgradeDesktopSettings(
  current: DesktopSettings,
  resolvedDefaults: DesktopSettings,
): DesktopSettings {
  if (
    LEGACY_DEFAULT_GATEWAY_URLS.has(current.gatewayUrl) &&
    current.gatewayUrl !== resolvedDefaults.gatewayUrl
  ) {
    return {
      ...current,
      gatewayUrl: resolvedDefaults.gatewayUrl,
    };
  }

  return current;
}

function workspaceNameFromPath(path: string | null | undefined): string {
  if (!path) {
    return 'Unknown Workspace';
  }
  const resolved = path.trim();
  return basename(resolved) || resolved;
}

function normalizeWorkspace(value?: Partial<DesktopWorkspace>): DesktopWorkspace {
  const now = new Date().toISOString();
  const path = value?.path?.trim() || null;
  const name = value?.name?.trim() || workspaceNameFromPath(path);
  return {
    id: value?.id?.trim() || buildWorkspaceId(),
    name,
    path,
    kind: 'local',
    createdAt: value?.createdAt || now,
    updatedAt: value?.updatedAt || value?.createdAt || now,
    available: value?.available ?? Boolean(path),
    managed: value?.managed ?? false,
  };
}

function pickSelectedWorkspaceId(
  workspaces: DesktopWorkspace[],
  selectedWorkspaceId?: string | null,
): string | null {
  if (selectedWorkspaceId && workspaces.some((workspace) => workspace.id === selectedWorkspaceId)) {
    return selectedWorkspaceId;
  }

  return (
    workspaces.find((workspace) => workspace.available)?.id ||
    workspaces[0]?.id ||
    null
  );
}

function pickSelectedAutomationId(
  automations: DesktopAutomationSummary[],
  selectedAutomationId?: string | null,
): string | null {
  if (
    selectedAutomationId &&
    automations.some((automation) => automation.id === selectedAutomationId)
  ) {
    return selectedAutomationId;
  }

  return automations[0]?.id || null;
}

function normalizeState(value?: Partial<DesktopState>): DesktopState {
  const settings = normalizeSettings(value?.settings);
  const gatewayProfiles = normalizeGatewayProfiles(value?.gatewayProfiles);
  const legacyBotBindings =
    value && typeof value === 'object' && 'botBindings' in value
      ? (value as Partial<DesktopState> & { botBindings?: Record<string, string> }).botBindings
      : undefined;
  const rawWorkspaces = Array.isArray(value?.workspaces)
    ? value.workspaces.filter((workspace) => !workspace?.managed)
    : [];
  const threads: DesktopThreadSummary[] = Array.isArray(value?.threads)
    ? value.threads
    : Array.isArray(value?.sessions)
      ? value.sessions
      : [];
  const workspaces = sortWorkspaces(
    rawWorkspaces.map((workspace) => normalizeWorkspace(workspace)),
    threads,
  );

  return {
    settings,
    gatewayProfiles,
    workspaces,
    hiddenWorkspacePaths: Array.isArray(value?.hiddenWorkspacePaths)
      ? Array.from(
          new Set(
            value.hiddenWorkspacePaths
              .filter((path): path is string => typeof path === 'string')
              .map((path) => path.trim())
              .filter((path) => path.length > 0),
          ),
        )
      : [],
    selectedWorkspaceId: value?.selectedWorkspaceId ?? null,
    threads,
    sessions: threads,
    endpoints: [],
    configuredBots: [],
    botConsoles: [],
    automations: [],
    selectedAutomationId: value?.selectedAutomationId ?? null,
    botMainThreads:
      value?.botMainThreads && typeof value.botMainThreads === 'object'
        ? Object.fromEntries(
            Object.entries(value.botMainThreads).filter(
              ([, v]) => typeof v === 'string' && v.trim().length > 0,
            ),
          )
        : legacyBotBindings && typeof legacyBotBindings === 'object'
          ? Object.fromEntries(
              Object.entries(legacyBotBindings)
                .filter(
                  ([threadId, botId]) =>
                    typeof threadId === 'string'
                    && threadId.trim().length > 0
                    && typeof botId === 'string'
                    && botId.trim().length > 0,
                )
                .map(([threadId, botId]) => [String(botId).trim(), String(threadId).trim()]),
            )
          : {},
    lastSeenRunAtByAutomation:
      value?.lastSeenRunAtByAutomation && typeof value.lastSeenRunAtByAutomation === 'object'
        ? Object.fromEntries(
            Object.entries(value.lastSeenRunAtByAutomation).filter(([, entryValue]) => {
              return typeof entryValue === 'string' && entryValue.trim().length > 0;
            }),
          )
        : {},
    remoteErrors: [],
  };
}

async function writeState(state: DesktopState): Promise<void> {
  const filePath = stateFilePath();
  await mkdir(dirname(filePath), { recursive: true });
  await writeFile(filePath, JSON.stringify(state, null, 2), 'utf8');
}

async function canonicalizeWorkspacePath(path: string): Promise<string> {
  const trimmed = path.trim();
  if (!trimmed) {
    throw new Error('Choose a folder to add as a workspace.');
  }

  const resolved = resolvePath(trimmed);
  return realpath(resolved);
}

async function resolveWorkspaceAvailability(workspace: DesktopWorkspace): Promise<DesktopWorkspace> {
  if (!workspace.path) {
    return {
      ...workspace,
      available: false,
    };
  }

  try {
    await access(workspace.path);
    const canonical = await realpath(workspace.path);
    return {
      ...workspace,
      path: canonical,
      available: true,
    };
  } catch {
    return {
      ...workspace,
      available: false,
    };
  }
}

function replaceWorkspace(
  workspaces: DesktopWorkspace[],
  nextWorkspace: DesktopWorkspace,
): DesktopWorkspace[] {
  return workspaces.map((workspace) => {
    if (workspace.id !== nextWorkspace.id) {
      return workspace;
    }
    return nextWorkspace;
  });
}

function withSortedEntities(
  state: DesktopState,
  options?: { preserveMissingSelectedWorkspace?: boolean },
): DesktopState {
  const threads = sortThreads(state.threads);
  const automations = sortAutomations(state.automations);
  const workspaces = sortWorkspaces(state.workspaces, threads);
  const preserveMissingSelectedWorkspace =
    options?.preserveMissingSelectedWorkspace &&
    Boolean(state.selectedWorkspaceId) &&
    !workspaces.some((workspace) => workspace.id === state.selectedWorkspaceId);
  const preserveAutomationUiState = automations.length === 0;
  const knownAutomationIds = new Set(automations.map((automation) => automation.id));
  const lastSeenRunAtByAutomation = preserveAutomationUiState
    ? state.lastSeenRunAtByAutomation || {}
    : Object.fromEntries(
        Object.entries(state.lastSeenRunAtByAutomation || {}).filter(([automationId, timestamp]) => {
          return knownAutomationIds.has(automationId) && typeof timestamp === 'string' && timestamp.trim().length > 0;
        }),
      );
  return {
    ...state,
    threads,
    sessions: threads,
    automations,
    workspaces,
    selectedWorkspaceId: preserveMissingSelectedWorkspace
      ? state.selectedWorkspaceId
      : pickSelectedWorkspaceId(workspaces, state.selectedWorkspaceId),
    selectedAutomationId: preserveAutomationUiState
      ? state.selectedAutomationId ?? null
      : pickSelectedAutomationId(automations, state.selectedAutomationId),
    lastSeenRunAtByAutomation,
  };
}

function reconcileBotMainThreads(
  current: Record<string, string>,
  configuredBots: ConfiguredBot[],
): Record<string, string> {
  const next = { ...current };
  for (const bot of configuredBots) {
    const botId = `${bot.channel}::${bot.accountId}`;
    const mainThreadId = bot.mainEndpointThreadId?.trim();
    if (mainThreadId) {
      next[botId] = mainThreadId;
    }
  }
  return next;
}

type RemoteFetchSource = DesktopRemoteStateError['source'];

type RemoteFetchResult<T> =
  | { ok: true; value: T; error: null }
  | { ok: false; value: T; error: DesktopRemoteStateError };

function remoteErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error || 'Unknown error');
}

async function fetchRemoteSlice<T>(
  source: RemoteFetchSource,
  label: string,
  fallback: T,
  fetcher: () => Promise<T>,
): Promise<RemoteFetchResult<T>> {
  try {
    return {
      ok: true,
      value: await fetcher(),
      error: null,
    };
  } catch (error) {
    const message = remoteErrorMessage(error);
    console.warn(`Failed to refresh remote ${label}.`, error);
    return {
      ok: false,
      value: fallback,
      error: {
        source,
        label,
        message,
      },
    };
  }
}

async function mergeRemoteDesktopState(localState: DesktopState): Promise<DesktopState> {
  const [threadsResult, endpointsResult, configuredBotsResult, botConsolesResult, automationsResult] =
    await Promise.all([
      fetchRemoteSlice('threads', 'threads', localState.threads, () => fetchThreads(localState.settings)),
      fetchRemoteSlice('endpoints', 'endpoints', localState.endpoints, () => fetchChannelEndpoints(localState.settings)),
      fetchRemoteSlice(
        'configured_bots',
        'configured bots',
        [],
        () => fetchConfiguredBots(localState.settings),
      ),
      fetchRemoteSlice('bot_consoles', 'bot consoles', localState.botConsoles, () => fetchBotConsoles(localState.settings)),
      fetchRemoteSlice('automations', 'automations', localState.automations, () => fetchAutomations(localState.settings)),
    ]);
  const remoteErrors = [
    threadsResult.error,
    endpointsResult.error,
    configuredBotsResult.error,
    botConsolesResult.error,
    automationsResult.error,
  ].filter((error): error is DesktopRemoteStateError => Boolean(error));
  const remoteThreads = threadsResult.value;
  const remoteEndpoints = endpointsResult.value;
  const remoteConfiguredBots = configuredBotsResult.value;
  const remoteBotConsoles = botConsolesResult.value;
  const remoteAutomations = automationsResult.value;

  const workspaces = [...localState.workspaces];
  const workspaceByPath = new Map<string, DesktopWorkspace>();
  const workspaceByPathLower = new Map<string, DesktopWorkspace>();
  for (const workspace of workspaces) {
    const path = workspace.path?.trim();
    if (!path) {
      continue;
    }
    workspaceByPath.set(path, workspace);
    workspaceByPathLower.set(normalizeWorkspacePathKey(path), workspace);
  }

  const hiddenWorkspacePathKeys = new Set(
    (localState.hiddenWorkspacePaths || [])
      .filter((path): path is string => typeof path === 'string' && path.trim().length > 0)
      .map(normalizeWorkspacePathKey),
  );
  const inferredWorkspacePaths = new Map<string, string>();
  const collectInferredWorkspacePath = (value?: string | null) => {
    const trimmed = value?.trim() || '';
    if (!trimmed) {
      return;
    }
    const key = normalizeWorkspacePathKey(trimmed);
    if (
      hiddenWorkspacePathKeys.has(key) ||
      workspaceByPathLower.has(key) ||
      inferredWorkspacePaths.has(key)
    ) {
      return;
    }
    inferredWorkspacePaths.set(key, trimmed);
  };

  for (const thread of remoteThreads) {
    collectInferredWorkspacePath(thread.workspacePath);
  }
  for (const endpoint of remoteEndpoints) {
    collectInferredWorkspacePath(endpoint.workspacePath);
  }
  if (configuredBotsResult.ok) {
    for (const bot of remoteConfiguredBots) {
      collectInferredWorkspacePath(bot.workspace_dir);
    }
  } else {
    for (const bot of localState.configuredBots) {
      collectInferredWorkspacePath(bot.workspaceDir);
    }
  }
  for (const automation of remoteAutomations) {
    collectInferredWorkspacePath(automation.workspacePath);
  }
  for (const bot of remoteBotConsoles) {
    collectInferredWorkspacePath(bot.workspaceDir);
  }

  if (inferredWorkspacePaths.size) {
    const inferredWorkspaces = await Promise.all(
      Array.from(inferredWorkspacePaths.values()).map(async (path) => {
        const now = new Date().toISOString();
        return resolveWorkspaceAvailability({
          id: buildManagedWorkspaceId(path),
          name: workspaceNameFromPath(path),
          path,
          kind: 'local',
          createdAt: now,
          updatedAt: now,
          available: true,
          managed: true,
        });
      }),
    );
    for (const workspace of inferredWorkspaces) {
      workspaces.push(workspace);
      if (workspace.path?.trim()) {
        const key = normalizeWorkspacePathKey(workspace.path);
        workspaceByPath.set(workspace.path, workspace);
        workspaceByPathLower.set(key, workspace);
      }
    }
  }

  const resolveWorkspace = (path: string): DesktopWorkspace | null => {
    const trimmed = path.trim();
    return workspaceByPath.get(trimmed) || workspaceByPathLower.get(normalizeWorkspacePathKey(trimmed)) || null;
  };

  const threads = remoteThreads.map((thread) => {
    const trimmedWorkspacePath = thread.workspacePath?.trim() || '';
    const workspace = trimmedWorkspacePath ? resolveWorkspace(trimmedWorkspacePath) : null;

    return {
      ...thread,
      workspaceId: workspace?.id || '',
    };
  });

  const endpoints: DesktopChannelEndpoint[] = remoteEndpoints;
  const configuredBots: ConfiguredBot[] = configuredBotsResult.ok
    ? remoteConfiguredBots.map((bot) => ({
        channel: bot.channel,
        accountId: bot.account_id,
        displayName: bot.display_name?.trim() || bot.displayName?.trim() || bot.name?.trim() || bot.account_id,
        enabled: bot.enabled,
        workspaceDir: bot.workspace_dir?.trim() || null,
        rootBehavior: bot.root_behavior === 'expand_only' ? 'expand_only' as const : 'open_default' as const,
        mainEndpointStatus: bot.main_endpoint_status === 'resolved' ? 'resolved' as const : 'unresolved' as const,
        mainEndpoint: bot.main_endpoint ? mapChannelEndpoint(bot.main_endpoint) : null,
        mainEndpointThreadId: bot.main_endpoint_thread_id?.trim() || null,
        defaultOpenEndpoint: bot.default_open_endpoint ? mapChannelEndpoint(bot.default_open_endpoint) : null,
        defaultOpenThreadId: bot.default_open_thread_id?.trim() || null,
      }))
    : localState.configuredBots;
  const botConsoles: DesktopBotConsoleSummary[] = remoteBotConsoles.map((bot) => ({
    ...bot,
    workspaceDir: bot.workspaceDir?.trim() || null,
  }));
  const automations = remoteAutomations.map((automation) => {
    const trimmedWorkspacePath = automation.workspacePath?.trim() || '';
    const workspace = trimmedWorkspacePath ? resolveWorkspace(trimmedWorkspacePath) : null;

    return {
      ...automation,
      workspaceId: workspace?.id || '',
    };
  });
  const next = withSortedEntities({
    ...localState,
    workspaces,
    threads,
    endpoints,
    configuredBots,
    botConsoles,
    automations,
    botMainThreads: reconcileBotMainThreads(localState.botMainThreads || {}, configuredBots),
    remoteErrors,
  });

  return next;
}

async function hydrateState(value?: Partial<DesktopState>): Promise<DesktopState> {
  const normalized = normalizeState(value);
  const refreshedWorkspaces = await Promise.all(
    normalized.workspaces.map((workspace) => resolveWorkspaceAvailability(workspace)),
  );
  const next = withSortedEntities({
    ...normalized,
    workspaces: refreshedWorkspaces,
  }, { preserveMissingSelectedWorkspace: true });

  if (JSON.stringify(normalized) !== JSON.stringify(next)) {
    await writeState(next);
  }

  return next;
}

function requireWorkspace(state: DesktopState, workspaceId: string): DesktopWorkspace {
  const workspace = state.workspaces.find((entry) => entry.id === workspaceId);
  if (!workspace) {
    throw new Error('Workspace not found.');
  }
  return workspace;
}

function requireThread(state: DesktopState, threadId: string): DesktopThreadSummary {
  const thread = state.threads.find((entry) => entry.id === threadId);
  if (!thread) {
    throw new Error('Thread not found.');
  }
  return thread;
}

export function buildWorkspaceId(): string {
  return `workspace::${randomUUID()}`;
}

async function getLocalDesktopState(): Promise<DesktopState> {
  const resolvedDefaults = await resolveDefaultDesktopSettings();
  await migrateLegacyStateFile();
  try {
    const filePath = stateFilePath();
    const raw = await readFile(filePath, 'utf8');
    let state: DesktopState;
    try {
      state = await hydrateState(JSON.parse(raw) as Partial<DesktopState>);
    } catch (error) {
      if (!(error instanceof SyntaxError)) {
        throw error;
      }

      const recoveredState = await hydrateState({
        settings: resolvedDefaults,
      });
      const backupPath = `${filePath}.corrupt-${new Date().toISOString().replace(/[:.]/g, '-')}`;
      try {
        await rename(filePath, backupPath);
      } catch (renameError) {
        console.warn('Failed to back up corrupt desktop state file.', renameError);
      }
      await writeState(recoveredState);
      console.warn(`Recovered corrupt desktop state from ${filePath}.`, error);
      return recoveredState;
    }
    const upgradedSettings = upgradeDesktopSettings(state.settings, resolvedDefaults);
    if (JSON.stringify(upgradedSettings) !== JSON.stringify(state.settings)) {
      const next = {
        ...state,
        settings: upgradedSettings,
      };
      await writeState(next);
      return next;
    }
    return state;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
      return hydrateState({
        settings: resolvedDefaults,
      });
    }
    throw error;
  }
}

export async function getDesktopState(): Promise<DesktopState> {
  const localState = await getLocalDesktopState();
  return mergeRemoteDesktopState(localState);
}

export async function getLocalDesktopSettings(): Promise<DesktopSettings> {
  const localState = await getLocalDesktopState();
  return localState.settings;
}

export async function saveDesktopSettings(settings: DesktopSettings): Promise<DesktopState> {
  const current = await getLocalDesktopState();
  const nextSettings = normalizeSettings(settings);
  const next = {
    ...current,
    settings: nextSettings,
  };
  await writeState(next);
  return getDesktopState();
}

export async function rememberDesktopGatewayProfile(): Promise<DesktopState> {
  const current = await getLocalDesktopState();
  const next = {
    ...current,
    gatewayProfiles: upsertGatewayProfile(current.gatewayProfiles || [], current.settings),
  };
  await writeState(next);
  return getDesktopState();
}

export async function selectDesktopWorkspace(workspaceId: string | null): Promise<DesktopState> {
  const current = await getLocalDesktopState();
  const next = withSortedEntities({
    ...current,
    selectedWorkspaceId: workspaceId,
  }, { preserveMissingSelectedWorkspace: true });
  await writeState(next);
  return getDesktopState();
}

export async function selectDesktopAutomation(automationId: string | null): Promise<DesktopState> {
  const current = await getLocalDesktopState();
  const next = withSortedEntities({
    ...current,
    selectedAutomationId: automationId,
  });
  await writeState(next);
  return getDesktopState();
}

export async function setDesktopBotBinding(
  threadId: string,
  botId: string | null,
): Promise<DesktopState> {
  const current = await getLocalDesktopState();
  const nextBindings = { ...current.botMainThreads };
  if (botId && botId.trim()) {
    const normalizedBotId = botId.trim();
    for (const [existingBotId, existingThreadId] of Object.entries(nextBindings)) {
      if (existingBotId === normalizedBotId || existingThreadId === threadId) {
        delete nextBindings[existingBotId];
      }
    }
    nextBindings[normalizedBotId] = threadId;
  } else {
    for (const [existingBotId, existingThreadId] of Object.entries(nextBindings)) {
      if (existingThreadId === threadId) {
        delete nextBindings[existingBotId];
      }
    }
  }
  const next = withSortedEntities({
    ...current,
    botMainThreads: nextBindings,
  });
  await writeState(next);
  return getDesktopState();
}

export async function markDesktopAutomationSeen(
  automationId: string,
  seenAt: string | null,
): Promise<DesktopState> {
  const current = await getLocalDesktopState();
  const nextLastSeen = {
    ...current.lastSeenRunAtByAutomation,
  };
  if (seenAt && seenAt.trim()) {
    nextLastSeen[automationId] = seenAt.trim();
  } else {
    delete nextLastSeen[automationId];
  }
  const next = withSortedEntities({
    ...current,
    lastSeenRunAtByAutomation: nextLastSeen,
  });
  await writeState(next);
  return getDesktopState();
}

export async function addDesktopWorkspace(path: string): Promise<{
  state: DesktopState;
  workspace: DesktopWorkspace;
}> {
  const current = await getLocalDesktopState();
  const canonicalPath = await canonicalizeWorkspacePath(path);
  const duplicate = current.workspaces.find((workspace) => {
    return workspace.kind === 'local' && workspace.path === canonicalPath;
  });

  if (duplicate) {
    const selectedState = withSortedEntities({
      ...current,
      selectedWorkspaceId: duplicate.id,
    });
    await writeState(selectedState);
    return {
      state: selectedState,
      workspace: duplicate,
    };
  }

  const now = new Date().toISOString();
  const workspace: DesktopWorkspace = {
    id: buildWorkspaceId(),
    name: workspaceNameFromPath(canonicalPath),
    path: canonicalPath,
    kind: 'local',
    createdAt: now,
    updatedAt: now,
    available: true,
  };

  const next = withSortedEntities({
    ...current,
    workspaces: [...current.workspaces, workspace],
    hiddenWorkspacePaths: (current.hiddenWorkspacePaths || []).filter((entry) => entry !== canonicalPath),
    selectedWorkspaceId: workspace.id,
  });
  await writeState(next);
  return { state: await getDesktopState(), workspace };
}

export async function relinkDesktopWorkspace(
  workspaceId: string,
  path: string,
): Promise<{ state: DesktopState; workspace: DesktopWorkspace }> {
  const current = await getLocalDesktopState();
  const workspace = requireWorkspace(current, workspaceId);

  const canonicalPath = await canonicalizeWorkspacePath(path);
  const duplicate = current.workspaces.find((entry) => {
    return entry.id !== workspaceId && entry.kind === 'local' && entry.path === canonicalPath;
  });
  if (duplicate) {
    throw new Error('That folder is already added as a workspace.');
  }

  const updatedAt = new Date().toISOString();
  const nextWorkspace: DesktopWorkspace = {
    ...workspace,
    name: workspaceNameFromPath(canonicalPath),
    path: canonicalPath,
    available: true,
    updatedAt,
  };

  const next = withSortedEntities({
    ...current,
    workspaces: replaceWorkspace(current.workspaces, nextWorkspace),
    hiddenWorkspacePaths: (current.hiddenWorkspacePaths || []).filter((entry) => entry !== canonicalPath),
    selectedWorkspaceId: workspaceId,
  });
  await writeState(next);
  return { state: await getDesktopState(), workspace: nextWorkspace };
}

export async function renameDesktopWorkspace(
  workspaceId: string,
  name: string,
): Promise<DesktopState> {
  const trimmedName = name.trim();
  if (!trimmedName) {
    throw new Error('Workspace name cannot be empty.');
  }

  const current = await getDesktopState();
  requireWorkspace(current, workspaceId);

  const local = await getLocalDesktopState();
  if (!local.workspaces.some((entry) => entry.id === workspaceId)) {
    throw new Error('Only local workspaces can be renamed.');
  }

  const updatedAt = new Date().toISOString();
  const next = withSortedEntities({
    ...local,
    workspaces: local.workspaces.map((entry) => {
      if (entry.id !== workspaceId) {
        return entry;
      }
      return {
        ...entry,
        name: trimmedName,
        updatedAt,
      };
    }),
  });
  await writeState(next);
  return getDesktopState();
}

export async function removeDesktopWorkspace(workspaceId: string): Promise<DesktopState> {
  const current = await getDesktopState();
  const workspace = requireWorkspace(current, workspaceId);

  const local = await getLocalDesktopState();
  if (!local.workspaces.some((entry) => entry.id === workspaceId)) {
    throw new Error('Only local workspaces can be removed.');
  }
  const nextHiddenWorkspacePaths = workspace.path?.trim()
    ? Array.from(new Set([...(local.hiddenWorkspacePaths || []), workspace.path.trim()]))
    : local.hiddenWorkspacePaths || [];

  const next = withSortedEntities({
    ...local,
    workspaces: local.workspaces.filter((entry) => entry.id !== workspaceId),
    hiddenWorkspacePaths: nextHiddenWorkspacePaths,
    selectedWorkspaceId:
      local.selectedWorkspaceId === workspaceId ? null : local.selectedWorkspaceId,
  });
  await writeState(next);
  return getDesktopState();
}

export async function createDesktopThread(input?: {
  title?: string;
  workspaceId?: string | null;
  workspacePath?: string | null;
  agentId?: string | null;
  sdkSessionId?: string | null;
  sdkSessionProviderHint?: DesktopSessionProviderHint | null;
}): Promise<{ state: DesktopState; thread: DesktopThreadSummary; session?: DesktopThreadSummary }> {
  const current = await getDesktopState();
  const sdkSessionId = normalizeSdkSessionIdInput(input?.sdkSessionId);
  const sdkSessionProviderHint = sdkSessionId
    ? normalizeSdkSessionProviderHintInput(input?.sdkSessionProviderHint)
    : null;
  const explicitWorkspacePath = sdkSessionId ? null : normalizeWorkspacePathInput(input?.workspacePath);
  let targetWorkspaceId: string | null = sdkSessionId ? null : current.selectedWorkspaceId;
  if (!sdkSessionId && input && Object.prototype.hasOwnProperty.call(input, 'workspaceId')) {
    if (input.workspaceId === null) {
      targetWorkspaceId = null;
    } else if (typeof input.workspaceId === 'string') {
      const trimmed = input.workspaceId.trim();
      targetWorkspaceId = trimmed || null;
    }
  }

  let workspacePath = explicitWorkspacePath;
  if (!workspacePath) {
    if (!targetWorkspaceId && !sdkSessionId) {
      throw new Error('Choose an available folder before creating a new thread.');
    }
    if (targetWorkspaceId) {
      const workspace = requireWorkspace(current, targetWorkspaceId);
      if (!workspace.available || !workspace.path) {
        throw new Error('Choose an available folder before creating a new thread.');
      }
      workspacePath = workspace.path;
    }
  }

  const requestedTitle = normalizeNewThreadTitle(input?.title);
  const created = await createRemoteThread(current.settings, {
    title: requestedTitle,
    workspacePath,
    agentId: input?.agentId,
    sdkSessionId,
    sdkSessionProviderHint,
  });
  let state = await getDesktopState();
  let thread = state.threads.find((entry) => entry.id === created.id) || created;

  if (!requestedTitle && thread.title === thread.id && (thread.messageCount ?? 0) === 0) {
    thread = {
      ...thread,
      title: DEFAULT_SESSION_TITLE,
    };
    state = withSortedEntities({
      ...state,
      threads: state.threads.map((entry) => {
        if (entry.id !== thread.id) {
          return entry;
        }
        return thread;
      }),
    });
  }

  return {
    state,
    thread,
    session: thread,
  };
}

function requireAutomation(state: DesktopState, automationId: string): DesktopAutomationSummary {
  const automation = state.automations.find((entry) => entry.id === automationId);
  if (!automation) {
    throw new Error('Automation not found.');
  }
  return automation;
}

function requireAvailableAutomationWorkspace(
  state: DesktopState,
  workspaceId: string,
): DesktopWorkspace {
  const workspace = requireWorkspace(state, workspaceId);
  if (!workspace.available || !workspace.path) {
    throw new Error('Choose an available local workspace for this automation.');
  }
  return workspace;
}

export async function createDesktopAutomation(
  input: CreateAutomationInput,
): Promise<{ state: DesktopState; automation: DesktopAutomationSummary }> {
  const current = await getDesktopState();
  const workspace = requireAvailableAutomationWorkspace(current, input.workspaceId);
  const created = await createRemoteAutomation(current.settings, {
    label: input.label.trim(),
    prompt: input.prompt.trim(),
    agentId: input.agentId.trim(),
    workspacePath: workspace.path!,
    schedule: input.schedule,
  });
  const local = await getLocalDesktopState();
  const nextLocal = withSortedEntities({
    ...local,
    selectedAutomationId: created.id,
  });
  await writeState(nextLocal);
  const state = await getDesktopState();
  return {
    state,
    automation: state.automations.find((entry) => entry.id === created.id) || {
      ...created,
      agentId: input.agentId.trim(),
      workspaceId: workspace.id,
    },
  };
}

export async function updateDesktopAutomation(input: {
  automationId: string;
  label?: string;
  prompt?: string;
  agentId?: string;
  workspaceId?: string;
  schedule?: CreateAutomationInput['schedule'];
  enabled?: boolean;
}): Promise<{ state: DesktopState; automation: DesktopAutomationSummary }> {
  const current = await getDesktopState();
  const existing = requireAutomation(current, input.automationId);
  const workspace = input.workspaceId
    ? requireAvailableAutomationWorkspace(current, input.workspaceId)
    : requireWorkspace(current, existing.workspaceId);
  const updated = await updateRemoteAutomation(current.settings, input.automationId, {
    label: input.label?.trim(),
    prompt: input.prompt?.trim(),
    agentId: input.agentId?.trim(),
    workspacePath: workspace.path || undefined,
    schedule: input.schedule,
    enabled: input.enabled,
  });
  const local = await getLocalDesktopState();
  const nextLocal = withSortedEntities({
    ...local,
    selectedAutomationId: input.automationId,
  });
  await writeState(nextLocal);
  const state = await getDesktopState();
  return {
    state,
    automation: state.automations.find((entry) => entry.id === updated.id) || {
      ...updated,
      agentId: input.agentId?.trim() || existing.agentId,
      workspaceId: workspace.id,
    },
  };
}

export async function deleteDesktopAutomation(automationId: string): Promise<DesktopState> {
  const current = await getDesktopState();
  requireAutomation(current, automationId);
  await deleteRemoteAutomation(current.settings, automationId);
  const local = await getLocalDesktopState();
  const nextLastSeen = {
    ...local.lastSeenRunAtByAutomation,
  };
  delete nextLastSeen[automationId];
  const next = withSortedEntities({
    ...local,
    selectedAutomationId:
      local.selectedAutomationId === automationId ? null : local.selectedAutomationId,
    lastSeenRunAtByAutomation: nextLastSeen,
  });
  await writeState(next);
  return getDesktopState();
}

export async function runDesktopAutomationNow(
  automationId: string,
): Promise<{ state: DesktopState; activity: DesktopAutomationActivityEntry }> {
  const current = await getDesktopState();
  requireAutomation(current, automationId);
  const activity = await runRemoteAutomationNow(current.settings, automationId);
  const state = await getDesktopState();
  return { state, activity };
}

export async function renameDesktopThread(
  threadId: string,
  title: string,
): Promise<DesktopState> {
  const current = await getDesktopState();
  await updateRemoteThread(current.settings, threadId, {
    title: title.trim() || DEFAULT_SESSION_TITLE,
  });
  return getDesktopState();
}

export async function deleteDesktopThread(threadId: string): Promise<DesktopState> {
  const current = await getDesktopState();
  await deleteRemoteThread(current.settings, threadId);
  return getDesktopState();
}

export async function recordOutgoingThreadPrompt(threadId: string, prompt: string): Promise<{
  state: DesktopState;
  thread: DesktopThreadSummary;
  session?: DesktopThreadSummary;
}> {
  const current = await getDesktopState();
  const existing = requireThread(current, threadId);
  const preview = summarizeText(prompt);
  const thread = {
    ...existing,
    title:
      existing.title !== DEFAULT_SESSION_TITLE ? existing.title : titleFromPrompt(prompt),
    lastMessagePreview: preview,
    updatedAt: new Date().toISOString(),
  };
  return {
    state: current,
    thread,
    session: thread,
  };
}

export async function resolveThreadWorkspace(
  threadId: string,
): Promise<{ thread: DesktopThreadSummary; session?: DesktopThreadSummary; workspace: DesktopWorkspace }> {
  const current = await getDesktopState();
  const thread = requireThread(current, threadId);
  const workspace = requireWorkspace(current, thread.workspaceId);
  return { thread, session: thread, workspace };
}

export const createDesktopSession = createDesktopThread;
export const renameDesktopSession = renameDesktopThread;
export const deleteDesktopSession = deleteDesktopThread;
export const recordOutgoingPrompt = recordOutgoingThreadPrompt;
export const resolveSessionWorkspace = resolveThreadWorkspace;
