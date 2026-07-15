import { createHash } from 'node:crypto';
import { mkdir, readFile, rename, writeFile } from 'node:fs/promises';
import { dirname, join, basename } from 'node:path';

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
  type DesktopFollowUpBehavior,
  type DesktopLanguagePreference,
  type DesktopRemoteStateError,
  type DesktopThreadSummary,
  type DesktopThreadPinOrderSnapshot,
  type DesktopThreadPinsPage,
  type DesktopSettings,
  type DesktopState,
  type DesktopWorkspace,
  type DesktopChannelEndpoint,
  type DesktopSessionProviderHint,
} from '@shared/contracts';
import { desktopStateWithoutThread } from '@shared/desktop-state';
import { normalizeGatewayHeadersBlock } from '../shared/gateway-headers.ts';
import {
  archiveRemoteThread,
  createRemoteAutomation,
  createRemoteThread,
  addRemoteWorkspace,
  deleteRemoteWorkspace,
  deleteRemoteAutomation,
  deleteRemoteThread,
  fetchAutomations,
  fetchBotConsoles,
  fetchChannelEndpoints,
  fetchConfiguredBots,
  fetchThreadPins,
  fetchThreadSummary,
  fetchThreads,
  fetchWorkspaces,
  GatewayRequestError,
  mapChannelEndpoint,
  normalizeGatewayUrl,
  reorderRemoteThreadPins,
  runRemoteAutomationNow,
  setRemoteThreadPinned,
  updateRemoteAutomation,
  updateRemoteThread,
} from './gary-client';
import {
  applyRemotePinsMergeStep,
  PinnedOrderController,
} from './pinned-order-controller.ts';
import {
  PinnedOrderState,
  type PinnedOrderOutbox,
  type PinnedOrderReorderFailure,
} from './pinned-order-state.ts';
const STATE_FILE_NAME = 'garyx-desktop-state.json';
const MAX_GATEWAY_PROFILES = 12;
const LEGACY_DEFAULT_GATEWAY_URLS = new Set([
  'http://127.0.0.1:3000',
  'http://localhost:3000',
]);

type PersistedDesktopState = Partial<DesktopState> & {
  /** Main-only durable intent; never crosses the DesktopState contract. */
  pinnedOrderOutbox?: PinnedOrderOutbox | null;
};

let persistedPinnedOrderOutbox: PinnedOrderOutbox | null = null;
let persistedPinnedOrderOutboxLoaded = false;
let latestLocalDesktopState: DesktopState | null = null;
let pinnedOrderController: PinnedOrderController | null = null;
let pinnedOrderSettings: DesktopSettings | null = null;

function stateFilePath(): string {
  return join(app.getPath('userData'), STATE_FILE_NAME);
}

function sortThreads(threads: DesktopThreadSummary[]): DesktopThreadSummary[] {
  return [...threads].sort((left, right) => {
    return Date.parse(right.updatedAt) - Date.parse(left.updatedAt);
  });
}

function normalizeWorkspacePathKey(path: string): string {
  return path.trim().toLowerCase();
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
    const pathKey = normalizeWorkspacePathKey(thread.workspacePath || '');
    if (!pathKey) {
      continue;
    }
    const current = latestByWorkspace.get(pathKey) ?? Number.NEGATIVE_INFINITY;
    latestByWorkspace.set(
      pathKey,
      Math.max(current, Date.parse(thread.updatedAt) || Number.NEGATIVE_INFINITY),
    );
  }

  return [...workspaces].sort((left, right) => {
    const leftPathKey = normalizeWorkspacePathKey(left.path || '');
    const rightPathKey = normalizeWorkspacePathKey(right.path || '');
    const rightLatest =
      latestByWorkspace.get(rightPathKey) ??
      (Date.parse(right.updatedAt) || Number.NEGATIVE_INFINITY);
    const leftLatest =
      latestByWorkspace.get(leftPathKey) ??
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

function normalizePinnedThreadIds(value: unknown): string[] {
  if (!Array.isArray(value)) {
    return [];
  }
  const seen = new Set<string>();
  const ids: string[] = [];
  for (const entry of value) {
    if (typeof entry !== 'string') {
      continue;
    }
    const id = entry.trim();
    if (!id || seen.has(id)) {
      continue;
    }
    seen.add(id);
    ids.push(id);
  }
  return ids;
}

function normalizePinsRevision(value: unknown): number {
  return Number.isSafeInteger(value) && (value as number) >= 0
    ? value as number
    : 0;
}

function normalizePinnedOrderOutbox(value: unknown): PinnedOrderOutbox | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return null;
  }
  const candidate = value as Partial<PinnedOrderOutbox>;
  const gatewayIdentity = typeof candidate.gatewayIdentity === 'string'
    ? normalizeGatewayUrl(candidate.gatewayIdentity)
    : '';
  if (!gatewayIdentity) {
    return null;
  }
  return {
    gatewayIdentity,
    desiredOrder: normalizePinnedThreadIds(candidate.desiredOrder),
    lastKnownRevision: normalizePinsRevision(candidate.lastKnownRevision),
  };
}

function loadPersistedPinnedOrderOutbox(document: PersistedDesktopState): void {
  if (persistedPinnedOrderOutboxLoaded) {
    return;
  }
  persistedPinnedOrderOutbox = normalizePinnedOrderOutbox(document.pinnedOrderOutbox);
  persistedPinnedOrderOutboxLoaded = true;
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
      return normalized;
    default:
      return null;
  }
}

function normalizeSettings(value?: Partial<DesktopSettings>): DesktopSettings {
  const normalizeLanguagePreference = (input: unknown): DesktopLanguagePreference => {
    return input === 'en' || input === 'zh-CN' || input === 'system'
      ? input
      : DEFAULT_DESKTOP_SETTINGS.languagePreference;
  };
  const normalizeFollowUpBehavior = (input: unknown): DesktopFollowUpBehavior => {
    return input === 'steer' || input === 'queue'
      ? input
      : DEFAULT_DESKTOP_SETTINGS.followUpBehavior;
  };

  return {
    gatewayUrl:
      value?.gatewayUrl?.trim().replace(/\/+$/, '') || DEFAULT_DESKTOP_SETTINGS.gatewayUrl,
    gatewayAuthToken:
      typeof value?.gatewayAuthToken === 'string'
        ? value.gatewayAuthToken.trim()
        : DEFAULT_DESKTOP_SETTINGS.gatewayAuthToken,
    gatewayHeaders: normalizeGatewayHeadersBlock(value?.gatewayHeaders),
    accountId: value?.accountId?.trim() || DEFAULT_DESKTOP_SETTINGS.accountId,
    fromId: value?.fromId?.trim() || DEFAULT_DESKTOP_SETTINGS.fromId,
    timeoutSeconds: Math.max(
      10,
      Math.min(600, Math.round(value?.timeoutSeconds ?? DEFAULT_DESKTOP_SETTINGS.timeoutSeconds)),
    ),
    languagePreference: normalizeLanguagePreference(value?.languagePreference),
    followUpBehavior: normalizeFollowUpBehavior(value?.followUpBehavior),
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
    gatewayHeaders: normalizeGatewayHeadersBlock(value?.gatewayHeaders),
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
    gatewayHeaders: normalizeGatewayHeadersBlock(settings.gatewayHeaders),
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

  // Re-saving on reconnect must not clobber a custom profile name with the
  // URL-derived default label.
  const existing = profiles.find((entry) => (
    gatewayProfileKey(entry.gatewayUrl) === gatewayProfileKey(profile.gatewayUrl)
  ));
  if (existing && existing.label.trim()) {
    profile.label = existing.label;
  }
  // Keep the saved-list order stable: reconnecting must not bump an existing
  // profile to the front of the recency-sorted list.
  if (existing) {
    profile.updatedAt = existing.updatedAt;
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
    return 'Unknown Folder';
  }
  const resolved = path.trim();
  return basename(resolved) || resolved;
}

function normalizeWorkspace(value?: Partial<DesktopWorkspace>): DesktopWorkspace {
  const now = new Date().toISOString();
  const path = value?.path?.trim() || null;
  const name = workspaceNameFromPath(path);
  return {
    name,
    path,
    kind: 'local',
    createdAt: value?.createdAt || now,
    updatedAt: value?.updatedAt || value?.createdAt || now,
    available: value?.available ?? Boolean(path),
    managed: value?.managed ?? false,
  };
}

function pickSelectedWorkspacePath(
  workspaces: DesktopWorkspace[],
  selectedWorkspacePath?: string | null,
): string | null {
  const selectedKey = normalizeWorkspacePathKey(selectedWorkspacePath || '');
  if (
    selectedKey &&
    workspaces.some((workspace) => normalizeWorkspacePathKey(workspace.path || '') === selectedKey)
  ) {
    return workspaces.find(
      (workspace) => normalizeWorkspacePathKey(workspace.path || '') === selectedKey,
    )?.path || selectedWorkspacePath || null;
  }

  return (
    workspaces.find((workspace) => workspace.available)?.path ||
    workspaces[0]?.path ||
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
  // Entity slices are gateway-scoped: state persisted while another gateway
  // was selected must not leak into this gateway's view.
  const entitiesGatewayUrl = typeof value?.entitiesGatewayUrl === 'string'
    ? normalizeGatewayUrl(value.entitiesGatewayUrl)
    : '';
  const normalizedGatewayUrl = normalizeGatewayUrl(settings.gatewayUrl || '');
  const entityScopeMatches = entitiesGatewayUrl === normalizedGatewayUrl;
  const legacyBotBindings =
    value && typeof value === 'object' && 'botBindings' in value
      ? (value as Partial<DesktopState> & { botBindings?: Record<string, string> }).botBindings
      : undefined;
  const rawWorkspaces: Partial<DesktopWorkspace>[] = [];
  const persistedThreads: DesktopThreadSummary[] = Array.isArray(value?.threads)
    ? value.threads
    : Array.isArray(value?.sessions)
      ? value.sessions
      : [];
  const threads = entityScopeMatches ? persistedThreads : [];
  const normalizedWorkspacesByPath = new Map<string, DesktopWorkspace>();
  for (const workspace of rawWorkspaces) {
    const normalizedWorkspace = normalizeWorkspace(workspace);
    const workspacePath = normalizedWorkspace.path?.trim();
    if (!workspacePath || normalizedWorkspace.managed) {
      continue;
    }
    normalizedWorkspacesByPath.set(
      normalizeWorkspacePathKey(workspacePath),
      normalizedWorkspace,
    );
  }
  const workspaces = sortWorkspaces(
    Array.from(normalizedWorkspacesByPath.values()),
    threads,
  );

  return {
    settings,
    gatewayProfiles,
    entitiesGatewayUrl: normalizedGatewayUrl,
    workspaces,
    selectedWorkspacePath: entityScopeMatches ? (value?.selectedWorkspacePath ?? null) : null,
    pinnedThreadIds: entityScopeMatches ? normalizePinnedThreadIds(value?.pinnedThreadIds) : [],
    pinsRevision: entityScopeMatches ? normalizePinsRevision(value?.pinsRevision) : 0,
    threads,
    sessions: threads,
    endpoints: [],
    configuredBots: [],
    botConsoles: [],
    automations: [],
    selectedAutomationId: entityScopeMatches ? (value?.selectedAutomationId ?? null) : null,
    botMainThreads:
      entityScopeMatches && value?.botMainThreads && typeof value.botMainThreads === 'object'
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
      entityScopeMatches && value?.lastSeenRunAtByAutomation && typeof value.lastSeenRunAtByAutomation === 'object'
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
  // Write-then-rename keeps the live file complete at all times. A plain
  // truncate-and-write let concurrent readers see partial JSON, which
  // tripped the corruption recovery below and wiped settings and gateway
  // profiles to defaults.
  const tempPath = `${filePath}.tmp-${process.pid}-${Date.now().toString(36)}`;
  const persisted: PersistedDesktopState = persistedPinnedOrderOutbox
    ? {
        ...state,
        pinnedOrderOutbox: persistedPinnedOrderOutbox,
      }
    : state;
  await writeFile(tempPath, JSON.stringify(persisted, null, 2), 'utf8');
  await rename(tempPath, filePath);
  latestLocalDesktopState = state;
}

function isAbsoluteWorkspacePath(path: string): boolean {
  return path.startsWith('/')
    || /^[A-Za-z]:[\\/]/.test(path)
    || path.startsWith('\\\\')
    || path.startsWith('//');
}

function canonicalizeWorkspacePath(path: string): string {
  const trimmed = path.trim();
  if (!trimmed) {
    throw new Error('Choose a folder.');
  }
  if (!isAbsoluteWorkspacePath(trimmed)) {
    throw new Error('Choose an absolute folder path.');
  }
  return trimmed;
}

function resolveWorkspaceAvailability(workspace: DesktopWorkspace): DesktopWorkspace {
  // Workspace paths live on the gateway host. Probing the local filesystem
  // wrongly marked every remote gateway workspace unavailable, so trust the
  // gateway's workspace list instead of a client-side fs check.
  if (!workspace.path) {
    return {
      ...workspace,
      available: false,
    };
  }

  return {
    ...workspace,
    name: workspaceNameFromPath(workspace.path),
    available: true,
  };
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
    Boolean(state.selectedWorkspacePath) &&
    !workspaces.some((workspace) => (
      normalizeWorkspacePathKey(workspace.path || '') === normalizeWorkspacePathKey(state.selectedWorkspacePath || '')
    ));
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
    selectedWorkspacePath: preserveMissingSelectedWorkspace
      ? state.selectedWorkspacePath
      : pickSelectedWorkspacePath(workspaces, state.selectedWorkspacePath),
    selectedAutomationId: preserveAutomationUiState
      ? state.selectedAutomationId ?? null
      : pickSelectedAutomationId(automations, state.selectedAutomationId),
    lastSeenRunAtByAutomation,
  };
}

function reconcileBotMainThreads(configuredBots: ConfiguredBot[]): Record<string, string> {
  const next: Record<string, string> = {};
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

const BOT_CONSOLES_REMOTE_CACHE_TTL_MS = 60_000;
const remoteSliceCache = new Map<string, {
  fetchedAt: number;
  value: unknown;
}>();
let latestHydratedDesktopState: DesktopState | null = null;

function rememberHydratedDesktopState(state: DesktopState): DesktopState {
  latestHydratedDesktopState = state;
  return state;
}

async function getHydratedDesktopStateForUiMutation(): Promise<DesktopState> {
  const hydrated = latestHydratedDesktopState;
  if (!hydrated) {
    return getDesktopState();
  }
  // The hydrated snapshot can predate recent settings/profile writes (its
  // remote merge may still have been in flight while the user switched
  // gateways). UI mutations persist a full state, so client-owned fields
  // must be re-read from disk or those writes get clobbered.
  const local = await getLocalDesktopState();
  return {
    ...hydrated,
    settings: local.settings,
    gatewayProfiles: local.gatewayProfiles,
  };
}

function remoteErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error || 'Unknown error');
}

/**
 * Failed-slice fallback: prefer the last successfully hydrated in-memory
 * entities over the persisted disk snapshot. The disk snapshot is only
 * written on UI mutations, so it can be stale or empty — falling back to
 * it used to visibly regress (or blank) the UI on a transient network
 * failure against a remote gateway. The in-memory snapshot is the newest
 * state the user has already seen; a failed refresh must never downgrade
 * it. Scoped to the current gateway so a real gateway switch still drops
 * the previous gateway's entities.
 */
function lastGoodSlice<T>(
  localState: DesktopState,
  pick: (state: DesktopState) => T,
): T | null {
  const hydrated = latestHydratedDesktopState;
  if (!hydrated) {
    return null;
  }
  const hydratedGateway = (hydrated.settings.gatewayUrl || '').trim();
  const currentGateway = (localState.settings.gatewayUrl || '').trim();
  if (hydratedGateway !== currentGateway) {
    return null;
  }
  return pick(hydrated);
}

function pinnedOrderGatewayIdentity(state: DesktopState): string {
  return normalizeGatewayUrl(state.settings.gatewayUrl || '');
}

function pinnedOrderTransportFingerprint(settings: DesktopSettings): string {
  return JSON.stringify([
    normalizeGatewayUrl(settings.gatewayUrl || ''),
    settings.gatewayAuthToken || '',
    settings.gatewayHeaders || '',
  ]);
}

function projectPinnedOrderState(
  state: DesktopState,
  controller: PinnedOrderController,
): DesktopState {
  return {
    ...state,
    pinnedThreadIds: controller.state.presentedOrder,
    pinsRevision: controller.state.highestObservedRevision,
  };
}

async function persistPinnedOrderOutbox(
  outbox: PinnedOrderOutbox | null,
  gatewayIdentity: string,
): Promise<void> {
  if (outbox) {
    persistedPinnedOrderOutbox = outbox;
  } else if (
    !persistedPinnedOrderOutbox ||
    persistedPinnedOrderOutbox.gatewayIdentity === gatewayIdentity
  ) {
    persistedPinnedOrderOutbox = null;
  }
  persistedPinnedOrderOutboxLoaded = true;
  if (latestLocalDesktopState) {
    await writeState(latestLocalDesktopState);
  }
}

const PIN_ORDER_RETRY_BASE_MS = [1_000, 2_000, 4_000, 8_000, 16_000];

function classifyPinnedOrderFailure(
  error: unknown,
  attempt: number,
): PinnedOrderReorderFailure {
  const retryDelay = () => {
    const base = PIN_ORDER_RETRY_BASE_MS[
      Math.min(Math.max(0, attempt - 1), PIN_ORDER_RETRY_BASE_MS.length - 1)
    ];
    return Math.round(base * (0.8 + Math.random() * 0.4));
  };
  if (error instanceof GatewayRequestError) {
    if (error.status === 429 || error.status >= 500) {
      return { kind: 'retryable', delay: retryDelay() };
    }
    return { kind: 'permanent', statusCode: error.status };
  }
  if (
    error instanceof TypeError ||
    (error instanceof Error && ['AbortError', 'TimeoutError'].includes(error.name))
  ) {
    return { kind: 'retryable', delay: retryDelay() };
  }
  return { kind: 'permanent', statusCode: null };
}

async function ensurePinnedOrderController(
  state: DesktopState,
): Promise<PinnedOrderController> {
  const gatewayIdentity = pinnedOrderGatewayIdentity(state);
  const previousSettings = pinnedOrderSettings;
  pinnedOrderSettings = state.settings;
  if (
    pinnedOrderController &&
    pinnedOrderController.state.gatewayIdentity === gatewayIdentity
  ) {
    if (
      previousSettings &&
      pinnedOrderTransportFingerprint(previousSettings) !==
        pinnedOrderTransportFingerprint(state.settings)
    ) {
      await pinnedOrderController.resumePausedSync();
    }
    return pinnedOrderController;
  }

  const previousIdentity = pinnedOrderController?.state.gatewayIdentity ?? null;
  let controller: PinnedOrderController;
  controller = new PinnedOrderController(
    new PinnedOrderState({
      gatewayIdentity,
      initialOrder: state.pinnedThreadIds,
      revision: state.pinsRevision,
      restoredOutbox:
        persistedPinnedOrderOutbox?.gatewayIdentity === gatewayIdentity
          ? persistedPinnedOrderOutbox
          : null,
    }),
    {
      now: () => Date.now(),
      persist: async (outbox, identity) => {
        if (
          latestLocalDesktopState &&
          pinnedOrderGatewayIdentity(latestLocalDesktopState) === controller.state.gatewayIdentity
        ) {
          latestLocalDesktopState = projectPinnedOrderState(
            latestLocalDesktopState,
            controller,
          );
        }
        await persistPinnedOrderOutbox(outbox, identity);
      },
      sendReorder: async (request) => {
        const settings = pinnedOrderSettings;
        if (
          !settings ||
          normalizeGatewayUrl(settings.gatewayUrl || '') !== request.stamp.gatewayIdentity
        ) {
          throw new DOMException('Pinned-order gateway changed', 'AbortError');
        }
        const result = await reorderRemoteThreadPins(
          settings,
          request.threadIds,
          request.expectedRevision,
        );
        return {
          threadIds: result.page.threadIds,
          revision: result.page.revision,
        };
      },
      classifyFailure: classifyPinnedOrderFailure,
      isCurrent: () => pinnedOrderController === controller,
      onPublish: (order) => {
        const hydrated = latestHydratedDesktopState;
        if (
          hydrated &&
          pinnedOrderGatewayIdentity(hydrated) === controller.state.gatewayIdentity
        ) {
          latestHydratedDesktopState = {
            ...hydrated,
            pinnedThreadIds: order,
            pinsRevision: controller.state.highestObservedRevision,
          };
        }
      },
    },
  );
  pinnedOrderController = controller;

  if (previousIdentity && previousIdentity !== gatewayIdentity) {
    await persistPinnedOrderOutbox(null, previousIdentity);
  }
  return controller;
}

/** Environment-change wake for a paused durable reorder outbox. */
export async function resumeDesktopPinnedOrderSync(): Promise<void> {
  await pinnedOrderController?.resumePausedSync();
}

export async function getDesktopThreadPinOrderSnapshot(): Promise<DesktopThreadPinOrderSnapshot> {
  if (pinnedOrderController) {
    return pinnedOrderController.snapshot();
  }
  const state = latestHydratedDesktopState ?? latestLocalDesktopState ?? await getLocalDesktopState();
  return (await ensurePinnedOrderController(state)).snapshot();
}

async function fetchRemoteSlice<T>(
  source: RemoteFetchSource,
  label: string,
  fallback: T,
  fetcher: () => Promise<T>,
  options?: { cacheTtlMs?: number; cacheScope?: string },
): Promise<RemoteFetchResult<T>> {
  const cacheTtlMs = options?.cacheTtlMs || 0;
  // Scope cached slices to the gateway they came from; an unscoped cache
  // kept serving the previous gateway's data right after a switch.
  const cacheKey = `${source}::${options?.cacheScope || ''}`;
  const cached = remoteSliceCache.get(cacheKey);
  if (cacheTtlMs > 0 && cached && Date.now() - cached.fetchedAt <= cacheTtlMs) {
    return {
      ok: true,
      value: cached.value as T,
      error: null,
    };
  }

  try {
    const value = await fetcher();
    if (cacheTtlMs > 0) {
      remoteSliceCache.set(cacheKey, {
        fetchedAt: Date.now(),
        value,
      });
    }
    return {
      ok: true,
      value,
      error: null,
    };
  } catch (error) {
    const message = remoteErrorMessage(error);
    console.warn(`Failed to refresh remote ${label}.`, error);
    return {
      ok: false,
      value: cached ? cached.value as T : fallback,
      error: {
        source,
        label,
        message,
      },
    };
  }
}

function remoteWorkspacesWithAvailability(
  workspaces: DesktopWorkspace[],
): DesktopWorkspace[] {
  return workspaces.map((workspace) => resolveWorkspaceAvailability(workspace));
}

interface MergeRemoteStateOptions {
  /**
   * Fast-hydration page size for the threads slice. When set, pinned ids
   * missing from the page are repaired by single-thread fetches so the
   * pinned rail resolves before the follow-up full state lands.
   */
  threadLimit?: number;
}

async function mergeRemoteDesktopState(
  localState: DesktopState,
  options?: MergeRemoteStateOptions,
): Promise<DesktopState> {
  const pinOrder = await ensurePinnedOrderController(localState);
  const pinsRequestStamp = pinOrder.requestStamp();
  const lastGoodPinnedState = lastGoodSlice(localState, (state) => ({
    threadIds: state.pinnedThreadIds,
    revision: state.pinsRevision,
  }));
  const pinsFallback: DesktopThreadPinsPage = lastGoodPinnedState ?? {
    threadIds: localState.pinnedThreadIds,
    revision: localState.pinsRevision,
  };
  const [threadsResult, pinsResult, endpointsResult, workspacesResult, configuredBotsResult, botConsolesResult, automationsResult] =
    await Promise.all([
      fetchRemoteSlice(
        'threads',
        'threads',
        lastGoodSlice(localState, (state) => state.threads) ?? localState.threads,
        () =>
          fetchThreads(localState.settings, options?.threadLimit ? { limit: options.threadLimit } : undefined),
      ),
      fetchRemoteSlice(
        'thread_pins',
        'thread pins',
        pinsFallback,
        () => fetchThreadPins(localState.settings),
      ),
      fetchRemoteSlice(
        'endpoints',
        'endpoints',
        lastGoodSlice(localState, (state) => state.endpoints) ?? localState.endpoints,
        () => fetchChannelEndpoints(localState.settings),
      ),
      fetchRemoteSlice(
        'workspaces',
        'workspaces',
        lastGoodSlice(localState, (state) => state.workspaces) ?? [],
        () => fetchWorkspaces(localState.settings),
      ),
      fetchRemoteSlice(
        'configured_bots',
        'configured bots',
        [],
        () => fetchConfiguredBots(localState.settings),
      ),
      fetchRemoteSlice(
        'bot_consoles',
        'bot consoles',
        lastGoodSlice(localState, (state) => state.botConsoles) ?? localState.botConsoles,
        () => fetchBotConsoles(localState.settings),
        {
          cacheTtlMs: BOT_CONSOLES_REMOTE_CACHE_TTL_MS,
          cacheScope: localState.settings.gatewayUrl || '',
        },
      ),
      fetchRemoteSlice(
        'automations',
        'automations',
        lastGoodSlice(localState, (state) => state.automations) ?? localState.automations,
        () => fetchAutomations(localState.settings),
      ),
    ]);
  const remoteErrors = [
    threadsResult.error,
    pinsResult.error,
    endpointsResult.error,
    workspacesResult.error,
    configuredBotsResult.error,
    botConsolesResult.error,
    automationsResult.error,
  ].filter((error): error is DesktopRemoteStateError => Boolean(error));
  const remoteThreads = threadsResult.value;
  const remotePinsPage = pinsResult.value;
  const remoteEndpoints = endpointsResult.value;
  const remoteWorkspaces = workspacesResult.value;
  const remoteConfiguredBots = configuredBotsResult.value;
  const remoteBotConsoles = botConsolesResult.value;
  const remoteAutomations = automationsResult.value;

  const effectivePinnedThreadIds = await applyRemotePinsMergeStep(
    pinOrder,
    {
      ok: pinsResult.ok,
      value: {
        threadIds: remotePinsPage.threadIds,
        revision: remotePinsPage.revision,
      },
    },
    pinsRequestStamp,
  );

  const workspaces = remoteWorkspacesWithAvailability(remoteWorkspaces);

  let threads = remoteThreads.map((thread) => ({
    ...thread,
    workspacePath: thread.workspacePath?.trim() || null,
  }));
  if (options?.threadLimit && threadsResult.ok && pinsResult.ok) {
    const pageIds = new Set(threads.map((thread) => thread.id));
    const missingPinnedIds = normalizePinnedThreadIds(effectivePinnedThreadIds).filter(
      (threadId) => !pageIds.has(threadId),
    );
    if (missingPinnedIds.length > 0) {
      const repaired = await Promise.all(
        missingPinnedIds.map((threadId) => fetchThreadSummary(localState.settings, threadId)),
      );
      threads = [
        ...threads,
        ...repaired
          .filter((thread): thread is DesktopThreadSummary => Boolean(thread))
          .map((thread) => ({
            ...thread,
            workspacePath: thread.workspacePath?.trim() || null,
          })),
      ];
    }
  }
  const threadIds = new Set(threads.map((thread) => thread.id));
  const pinnedThreadIds = normalizePinnedThreadIds(effectivePinnedThreadIds).filter((threadId) => {
    return threadIds.has(threadId);
  });

  const endpoints: DesktopChannelEndpoint[] = remoteEndpoints;
  const configuredBots: ConfiguredBot[] = configuredBotsResult.ok
    ? remoteConfiguredBots.map((bot) => ({
        channel: bot.channel,
        accountId: bot.account_id,
        displayName: bot.display_name.trim(),
        enabled: bot.enabled,
        workspaceDir: bot.workspace_dir?.trim() || null,
        rootBehavior: bot.root_behavior === 'expand_only' ? 'expand_only' as const : 'open_default' as const,
        mainEndpointStatus: bot.main_endpoint_status === 'resolved' ? 'resolved' as const : 'unresolved' as const,
        mainEndpoint: bot.main_endpoint ? mapChannelEndpoint(bot.main_endpoint) : null,
        mainEndpointThreadId: bot.main_endpoint_thread_id?.trim() || null,
        defaultOpenEndpoint: bot.default_open_endpoint ? mapChannelEndpoint(bot.default_open_endpoint) : null,
        defaultOpenThreadId: bot.default_open_thread_id?.trim() || null,
      }))
    : lastGoodSlice(localState, (state) => state.configuredBots) ?? localState.configuredBots;
  const botConsoles: DesktopBotConsoleSummary[] = remoteBotConsoles.map((bot) => ({
    ...bot,
    workspaceDir: bot.workspaceDir?.trim() || null,
  }));
  const automations = remoteAutomations.map((automation) => ({
    ...automation,
    workspacePath: automation.workspacePath?.trim() || '',
    targetThreadId: automation.targetThreadId?.trim() || '',
    threadId: automation.threadId?.trim() || automation.targetThreadId?.trim() || '',
  }));
  const next = withSortedEntities({
    ...localState,
    workspaces,
    threads,
    pinnedThreadIds,
    pinsRevision: pinOrder.state.highestObservedRevision,
    endpoints,
    configuredBots,
    botConsoles,
    automations,
    botMainThreads: reconcileBotMainThreads(configuredBots),
    remoteErrors,
  });

  return next;
}

async function hydrateState(value?: Partial<DesktopState>): Promise<DesktopState> {
  const normalized = normalizeState(value);
  const refreshedWorkspaces = normalized.workspaces.map((workspace) =>
    resolveWorkspaceAvailability(workspace),
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

function requireWorkspace(state: DesktopState, workspacePath: string): DesktopWorkspace {
  const key = normalizeWorkspacePathKey(workspacePath);
  const workspace = state.workspaces.find((entry) => (
    normalizeWorkspacePathKey(entry.path || '') === key
  ));
  if (!workspace) {
    throw new Error('Folder not found.');
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

async function getLocalDesktopState(): Promise<DesktopState> {
  const resolvedDefaults = await resolveDefaultDesktopSettings();
  try {
    const filePath = stateFilePath();
    const raw = await readFile(filePath, 'utf8');
    let state: DesktopState;
    try {
      const document = JSON.parse(raw) as PersistedDesktopState;
      loadPersistedPinnedOrderOutbox(document);
      state = await hydrateState(document);
    } catch (error) {
      if (!(error instanceof SyntaxError)) {
        throw error;
      }

      // Re-read once before declaring corruption: a torn read that races an
      // in-flight rewrite resolves itself as soon as the writer finishes,
      // and recovery resets settings and gateway profiles to defaults.
      await new Promise((resolve) => setTimeout(resolve, 150));
      let retryState: DesktopState | null = null;
      try {
        const retryRaw = await readFile(filePath, 'utf8');
        const retryDocument = JSON.parse(retryRaw) as PersistedDesktopState;
        loadPersistedPinnedOrderOutbox(retryDocument);
        retryState = await hydrateState(retryDocument);
      } catch (retryError) {
        if (!(retryError instanceof SyntaxError)) {
          throw retryError;
        }
      }
      if (retryState) {
        latestLocalDesktopState = retryState;
        return retryState;
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
    latestLocalDesktopState = state;
    return state;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
      persistedPinnedOrderOutbox = null;
      persistedPinnedOrderOutboxLoaded = true;
      const state = await hydrateState({
        settings: resolvedDefaults,
      });
      latestLocalDesktopState = state;
      return state;
    }
    throw error;
  }
}

export async function getDesktopState(): Promise<DesktopState> {
  // A remote merge that raced a gateway switch would deliver (and remember)
  // the previous gateway's entities; re-run until the gateway URL is stable
  // across the merge.
  for (let attempt = 0; attempt < 2; attempt += 1) {
    const localState = await getLocalDesktopState();
    const merged = await mergeRemoteDesktopState(localState);
    const settingsAfter = (await getLocalDesktopSettings()).gatewayUrl || '';
    if (settingsAfter === (localState.settings.gatewayUrl || '')) {
      return rememberHydratedDesktopState(merged);
    }
  }
  const localState = await getLocalDesktopState();
  return rememberHydratedDesktopState(await mergeRemoteDesktopState(localState));
}

const FAST_STATE_THREAD_LIMIT = 200;

/**
 * Boot-only fast hydration: the threads slice is fetched as a recent page
 * (plus by-id repair for pinned threads outside it) so the first paint does
 * not wait for the full thread set. The result is intentionally NOT
 * remembered as the hydrated mutation base — UI mutations that land before
 * the follow-up full `getDesktopState()` fall back to a fresh full merge,
 * so `requireThread` never sees the truncated page.
 */
export async function getDesktopStateFast(): Promise<DesktopState> {
  const localState = await getLocalDesktopState();
  return mergeRemoteDesktopState(localState, {
    threadLimit: FAST_STATE_THREAD_LIMIT,
  });
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

export async function addDesktopGatewayProfile(input: {
  label?: string;
  gatewayUrl: string;
  gatewayAuthToken?: string;
  gatewayHeaders?: string;
}): Promise<DesktopState> {
  const profile = normalizeGatewayProfile({
    label: input.label,
    gatewayUrl: input.gatewayUrl,
    gatewayAuthToken: input.gatewayAuthToken,
    gatewayHeaders: input.gatewayHeaders,
    updatedAt: new Date().toISOString(),
  });
  if (!profile) {
    return getDesktopState();
  }
  const current = await getLocalDesktopState();
  const next = {
    ...current,
    gatewayProfiles: normalizeGatewayProfiles([
      profile,
      ...(current.gatewayProfiles || []).filter((entry) => (
        gatewayProfileKey(entry.gatewayUrl) !== gatewayProfileKey(profile.gatewayUrl)
      )),
    ]),
  };
  await writeState(next);
  return getDesktopState();
}

export async function updateDesktopGatewayProfile(input: {
  profileId: string;
  label?: string;
  gatewayUrl: string;
  gatewayAuthToken?: string;
  gatewayHeaders?: string;
}): Promise<DesktopState> {
  const current = await getLocalDesktopState();
  const normalizedId = input.profileId.trim();
  const existing = (current.gatewayProfiles || []).find((entry) => entry.id === normalizedId);
  if (!existing) {
    return getDesktopState();
  }
  const nextProfile = normalizeGatewayProfile({
    label: typeof input.label === 'string' ? input.label : existing.label,
    gatewayUrl: input.gatewayUrl,
    gatewayAuthToken: typeof input.gatewayAuthToken === 'string'
      ? input.gatewayAuthToken
      : existing.gatewayAuthToken,
    gatewayHeaders: typeof input.gatewayHeaders === 'string'
      ? input.gatewayHeaders
      : existing.gatewayHeaders,
    // Editing keeps the row where it was; only newly added profiles take a
    // fresh timestamp.
    updatedAt: existing.updatedAt,
  });
  if (!nextProfile) {
    return getDesktopState();
  }

  // Editing the active gateway's profile also updates the live connection
  // settings so the saved list and the actual connection cannot drift apart.
  const wasCurrent = gatewayProfileKey(existing.gatewayUrl)
    === gatewayProfileKey(current.settings.gatewayUrl);
  const next = {
    ...current,
    gatewayProfiles: normalizeGatewayProfiles([
      nextProfile,
      ...(current.gatewayProfiles || []).filter((entry) => (
        entry.id !== normalizedId
        && gatewayProfileKey(entry.gatewayUrl) !== gatewayProfileKey(nextProfile.gatewayUrl)
      )),
    ]),
    settings: wasCurrent
      ? normalizeSettings({
          ...current.settings,
          gatewayUrl: nextProfile.gatewayUrl,
          gatewayAuthToken: nextProfile.gatewayAuthToken,
          gatewayHeaders: nextProfile.gatewayHeaders,
        })
      : current.settings,
  };
  await writeState(next);
  return getDesktopState();
}

export async function deleteDesktopGatewayProfile(profileId: string): Promise<DesktopState> {
  const normalizedId = profileId.trim();
  const current = await getLocalDesktopState();
  const next = {
    ...current,
    gatewayProfiles: (current.gatewayProfiles || []).filter(
      (entry) => entry.id !== normalizedId,
    ),
  };
  await writeState(next);
  return getDesktopState();
}

export async function selectDesktopWorkspace(workspacePath: string | null): Promise<DesktopState> {
  const current = await getHydratedDesktopStateForUiMutation();
  const next = withSortedEntities({
    ...current,
    selectedWorkspacePath: workspacePath,
  }, { preserveMissingSelectedWorkspace: true });
  await writeState(next);
  return rememberHydratedDesktopState(next);
}

export async function selectDesktopAutomation(automationId: string | null): Promise<DesktopState> {
  const current = await getHydratedDesktopStateForUiMutation();
  const next = withSortedEntities({
    ...current,
    selectedAutomationId: automationId,
  });
  await writeState(next);
  return rememberHydratedDesktopState(next);
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
  const current = await getHydratedDesktopStateForUiMutation();
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
  return rememberHydratedDesktopState(next);
}

export async function addDesktopWorkspace(path: string): Promise<{
  state: DesktopState;
  workspace: DesktopWorkspace;
}> {
  const current = await getDesktopState();
  const canonicalPath = canonicalizeWorkspacePath(path);
  const remoteWorkspaces = await addRemoteWorkspace(current.settings, {
    path: canonicalPath,
    name: workspaceNameFromPath(canonicalPath),
  });
  const workspaces = remoteWorkspacesWithAvailability(remoteWorkspaces);
  const workspace =
    workspaces.find((entry) => normalizeWorkspacePathKey(entry.path || '') === normalizeWorkspacePathKey(canonicalPath))
    || {
      name: workspaceNameFromPath(canonicalPath),
      path: canonicalPath,
      kind: 'local' as const,
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
      available: true,
    };
  const local = await getLocalDesktopState();
  await writeState(withSortedEntities({
    ...local,
    workspaces: [],
    selectedWorkspacePath: workspace.path,
  }, { preserveMissingSelectedWorkspace: true }));
  const next = withSortedEntities({
    ...current,
    workspaces,
    selectedWorkspacePath: workspace.path,
  }, { preserveMissingSelectedWorkspace: true });
  return { state: next, workspace };
}

export async function removeDesktopWorkspace(workspacePath: string): Promise<DesktopState> {
  const current = await getDesktopState();
  const workspace = requireWorkspace(current, workspacePath);
  const workspaceKey = normalizeWorkspacePathKey(workspace.path || workspacePath);
  const remoteWorkspaces = await deleteRemoteWorkspace(current.settings, {
    path: workspace.path || workspacePath,
  });
  const workspaces = remoteWorkspacesWithAvailability(remoteWorkspaces);
  const local = await getLocalDesktopState();
  await writeState(withSortedEntities({
    ...local,
    workspaces: [],
    selectedWorkspacePath:
      normalizeWorkspacePathKey(local.selectedWorkspacePath || '') === workspaceKey
        ? null
        : local.selectedWorkspacePath,
  }, { preserveMissingSelectedWorkspace: true }));
  return withSortedEntities({
    ...current,
    workspaces,
    selectedWorkspacePath:
      normalizeWorkspacePathKey(current.selectedWorkspacePath || '') === workspaceKey
        ? null
        : current.selectedWorkspacePath,
  });
}

export async function setDesktopThreadPinned(input: {
  threadId: string;
  pinned: boolean;
}): Promise<DesktopState> {
  const current = await getDesktopState();
  const thread = requireThread(current, input.threadId);
  const pinOrder = await ensurePinnedOrderController(current);
  const membershipRequest = await pinOrder.beginMembershipChange(
    thread.id,
    input.pinned,
  );
  if (!membershipRequest) {
    return projectPinnedOrderState(current, pinOrder);
  }
  try {
    const page = await setRemoteThreadPinned(
      current.settings,
      thread.id,
      input.pinned,
    );
    await pinOrder.completeMembership(membershipRequest, {
      threadIds: page.threadIds,
      revision: page.revision,
    });
    await pinOrder.waitForTransportIdle();
    return rememberHydratedDesktopState(projectPinnedOrderState(current, pinOrder));
  } catch (error) {
    await pinOrder.failMembership(membershipRequest);
    throw error;
  }
}

export async function setDesktopThreadPinOrder(
  threadIds: string[],
): Promise<DesktopState> {
  const normalizedOrder = normalizePinnedThreadIds(threadIds);
  if (normalizedOrder.length === 0) {
    throw new Error('Pinned thread order must be a non-empty array.');
  }
  if (normalizedOrder.length !== threadIds.length) {
    throw new Error('Pinned thread order must contain unique non-empty ids.');
  }
  const current = await getHydratedDesktopStateForUiMutation();
  const pinOrder = await ensurePinnedOrderController(current);
  await pinOrder.commitOrder(normalizedOrder);
  await pinOrder.waitForTransportIdle();
  return rememberHydratedDesktopState(projectPinnedOrderState(current, pinOrder));
}

export async function createDesktopThread(input?: {
  title?: string;
  workspacePath?: string | null;
  workspaceMode?: "local" | "worktree";
  agentId?: string | null;
  model?: string | null;
  modelReasoningEffort?: string | null;
  modelServiceTier?: string | null;
  sdkSessionId?: string | null;
  sdkSessionProviderHint?: DesktopSessionProviderHint | null;
  forkFromThreadId?: string | null;
  metadata?: Record<string, unknown> | null;
}): Promise<{ state: DesktopState; thread: DesktopThreadSummary; session?: DesktopThreadSummary }> {
  const current = await getDesktopState();
  const sdkSessionId = normalizeSdkSessionIdInput(input?.sdkSessionId);
  const sdkSessionProviderHint = sdkSessionId
    ? normalizeSdkSessionProviderHintInput(input?.sdkSessionProviderHint)
    : null;
  const forkFromThreadId = input?.forkFromThreadId?.trim() || null;
  const providerBoundSource = Boolean(sdkSessionId || forkFromThreadId);
  const explicitWorkspacePath = providerBoundSource ? null : normalizeWorkspacePathInput(input?.workspacePath);
  let targetWorkspacePath: string | null = providerBoundSource ? null : current.selectedWorkspacePath;
  let workspacePath = explicitWorkspacePath;
  if (!workspacePath) {
    workspacePath = targetWorkspacePath?.trim() || null;
  }
  if (!workspacePath) {
    if (!targetWorkspacePath && !providerBoundSource) {
      throw new Error('Choose an available folder before creating a new thread.');
    }
  }
  if (workspacePath) {
    const requestedWorkspacePath = workspacePath;
    const knownWorkspace = current.workspaces.find((workspace) => (
      normalizeWorkspacePathKey(workspace.path || '') === normalizeWorkspacePathKey(requestedWorkspacePath)
    ));
    if (knownWorkspace && (!knownWorkspace.available || !knownWorkspace.path)) {
      throw new Error('Choose an available folder before creating a new thread.');
    }
    workspacePath = knownWorkspace?.path || workspacePath;
  }

  const requestedTitle = normalizeNewThreadTitle(input?.title);
  const created = await createRemoteThread(current.settings, {
    title: requestedTitle,
    workspacePath,
    workspaceMode: providerBoundSource ? "local" : input?.workspaceMode,
    agentId: input?.agentId,
    model: input?.model,
    modelReasoningEffort: input?.modelReasoningEffort,
    modelServiceTier: input?.modelServiceTier,
    sdkSessionId,
    sdkSessionProviderHint,
    forkFromThreadId,
    metadata: input?.metadata || undefined,
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

function resolveAutomationWorkspacePath(
  input: { workspacePath?: string | null },
  fallbackPath?: string | null,
  targetThreadId?: string | null,
): string {
  const explicitPath = normalizeWorkspacePathInput(input.workspacePath);
  if (explicitPath) {
    return explicitPath;
  }
  const fallback = normalizeWorkspacePathInput(fallbackPath);
  if (fallback) {
    return fallback;
  }
  if (targetThreadId?.trim()) {
    return '';
  }
  throw new Error('Choose a directory for this automation.');
}

function normalizeAutomationTargetThreadId(value?: string | null): string {
  return value?.trim() || '';
}

export async function createDesktopAutomation(
  input: CreateAutomationInput,
): Promise<{ state: DesktopState; automation: DesktopAutomationSummary }> {
  const current = await getDesktopState();
  const targetThreadId = normalizeAutomationTargetThreadId(input.targetThreadId);
  const targetThread = targetThreadId
    ? current.threads.find((thread) => thread.id === targetThreadId)
    : null;
  const workspacePath = resolveAutomationWorkspacePath(
    input,
    targetThread?.workspacePath || null,
    targetThreadId,
  );
  // A thread-bound automation always runs with the thread's own agent and
  // workspace; the gateway rejects explicit overrides for that combination.
  const remoteWorkspacePath = targetThreadId ? undefined : workspacePath;
  const agentId = targetThreadId ? undefined : input.agentId?.trim() || undefined;
  const created = await createRemoteAutomation(current.settings, {
    label: input.label.trim(),
    prompt: input.prompt.trim(),
    agentId,
    workspacePath: remoteWorkspacePath,
    targetThreadId: targetThreadId || null,
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
      agentId: agentId || created.agentId,
      workspacePath,
      targetThreadId,
    },
  };
}

export async function updateDesktopAutomation(input: {
  automationId: string;
  label?: string;
  prompt?: string;
  agentId?: string;
  workspacePath?: string | null;
  targetThreadId?: string | null;
  schedule?: CreateAutomationInput['schedule'];
  enabled?: boolean;
}): Promise<{ state: DesktopState; automation: DesktopAutomationSummary }> {
  const current = await getDesktopState();
  const existing = requireAutomation(current, input.automationId);
  const hasTargetThreadInput = Object.prototype.hasOwnProperty.call(input, 'targetThreadId');
  const existingTargetThreadId = normalizeAutomationTargetThreadId(existing.targetThreadId);
  const targetThreadId = hasTargetThreadInput
    ? normalizeAutomationTargetThreadId(input.targetThreadId)
    : existingTargetThreadId;
  const targetThread = targetThreadId
    ? current.threads.find((thread) => thread.id === targetThreadId)
    : null;
  const targetThreadChanged = hasTargetThreadInput && targetThreadId !== existingTargetThreadId;
  const fallbackWorkspacePath =
    input.workspacePath === undefined
      ? targetThreadChanged
        ? targetThread?.workspacePath || existing.workspacePath
        : existing.workspacePath || targetThread?.workspacePath
      : targetThread?.workspacePath;
  const workspacePath = resolveAutomationWorkspacePath(
    input,
    fallbackWorkspacePath,
    targetThreadId,
  );
  // A thread-bound automation always runs with the thread's own agent and
  // workspace; the gateway rejects explicit overrides for that combination.
  const remoteWorkspacePath = targetThreadId ? undefined : workspacePath;
  const updated = await updateRemoteAutomation(current.settings, input.automationId, {
    label: input.label?.trim(),
    prompt: input.prompt?.trim(),
    agentId: targetThreadId ? undefined : input.agentId?.trim(),
    workspacePath: remoteWorkspacePath,
    targetThreadId: hasTargetThreadInput ? targetThreadId || null : undefined,
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
      workspacePath,
      targetThreadId,
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
  return withSortedEntities(desktopStateWithoutThread(await getDesktopState(), threadId));
}

export async function archiveDesktopThread(input: {
  threadId: string;
  endpointKeys?: string[];
}): Promise<DesktopState> {
  const current = await getDesktopState();
  await archiveRemoteThread(
    current.settings,
    input.threadId,
    input.endpointKeys || [],
  );
  return withSortedEntities(desktopStateWithoutThread(await getDesktopState(), input.threadId));
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
  const workspace = requireWorkspace(current, thread.workspacePath || '');
  return { thread, session: thread, workspace };
}

export const createDesktopSession = createDesktopThread;
export const renameDesktopSession = renameDesktopThread;
export const deleteDesktopSession = deleteDesktopThread;
export const recordOutgoingPrompt = recordOutgoingThreadPrompt;
export const resolveSessionWorkspace = resolveThreadWorkspace;
