import { startTransition, useEffect, useRef, useState } from 'react';

import {
  DEFAULT_DESKTOP_SETTINGS,
  type ConnectionStatus,
  type DesktopMcpServer,
  type DesktopState,
  type GatewayConfigDocument,
  type GatewaySettingsPayload,
  type GatewaySettingsSource,
  type SlashCommand,
  type UpdateMcpServerInput,
  type UpdateSlashCommandInput,
  type UpsertMcpServerInput,
  type UpsertSlashCommandInput,
} from '@shared/contracts';

import { SETTINGS_TABS, type SettingsTabId } from '../settings-tabs';
import {
  cloneJson,
  ensureGatewayConfig,
} from '../gateway-settings';
import { measureUiAction } from '../perf-metrics';
import { isGatewayConfigSettingsTab, isLocalSettingsTab } from './icons';

function desktopSettingsEqual(
  left: typeof DEFAULT_DESKTOP_SETTINGS,
  right: typeof DEFAULT_DESKTOP_SETTINGS,
): boolean {
  return (
    left.gatewayUrl === right.gatewayUrl &&
    left.gatewayAuthToken === right.gatewayAuthToken &&
    left.accountId === right.accountId &&
    left.fromId === right.fromId &&
    left.timeoutSeconds === right.timeoutSeconds &&
    left.providerClaudeEnv === right.providerClaudeEnv &&
    left.providerCodexAuthMode === right.providerCodexAuthMode &&
    left.providerCodexApiKey === right.providerCodexApiKey &&
    left.providerGeminiEnv === right.providerGeminiEnv &&
    left.threadLogsPanelWidth === right.threadLogsPanelWidth &&
    left.languagePreference === right.languagePreference
  );
}

function normalizeSettingsTab(value?: SettingsTabId | null): SettingsTabId {
  if (value === 'connection') {
    return 'gateway';
  }
  return value && SETTINGS_TABS.some((tab) => tab.id === value) ? value : 'labs';
}

function isPlainRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value);
}

function mergeGatewayConfigPatch(
  base: unknown,
  patch: GatewayConfigDocument,
): GatewayConfigDocument {
  const next = ensureGatewayConfig(cloneJson(base));
  mergeRecordPatch(next, cloneJson(patch));
  return ensureGatewayConfig(next);
}

function mergeRecordPatch(
  target: Record<string, unknown>,
  patch: Record<string, unknown>,
) {
  for (const [key, value] of Object.entries(patch)) {
    const current = target[key];
    if (isPlainRecord(current) && isPlainRecord(value)) {
      mergeRecordPatch(current, value);
    } else {
      target[key] = value;
    }
  }
}

function restoreGatewayConfigPatch(
  base: unknown,
  patch: GatewayConfigDocument,
  snapshot: GatewayConfigDocument,
): GatewayConfigDocument {
  const next = ensureGatewayConfig(cloneJson(base));
  restoreRecordPatch(next, patch, snapshot);
  return ensureGatewayConfig(next);
}

function restoreRecordPatch(
  target: Record<string, unknown>,
  patch: Record<string, unknown>,
  snapshot: Record<string, unknown>,
) {
  for (const [key, value] of Object.entries(patch)) {
    const current = target[key];
    const previous = snapshot[key];
    if (isPlainRecord(current) && isPlainRecord(value) && isPlainRecord(previous)) {
      restoreRecordPatch(current, value, previous);
      continue;
    }
    if (Object.prototype.hasOwnProperty.call(snapshot, key)) {
      target[key] = cloneJson(previous);
    } else {
      delete target[key];
    }
  }
}

type UseSettingsControllerArgs = {
  desktopState: DesktopState | null;
  initialSettingsTab?: SettingsTabId | null;
  setDesktopState: React.Dispatch<React.SetStateAction<DesktopState | null>>;
  setConnection: React.Dispatch<React.SetStateAction<ConnectionStatus | null>>;
  setError: React.Dispatch<React.SetStateAction<string | null>>;
};

export type GatewaySettingsSaveOptions = {
  silent?: boolean;
  refreshDesktopState?: 'await' | 'background' | 'skip';
};

export function useSettingsController({
  desktopState,
  initialSettingsTab,
  setDesktopState,
  setConnection,
  setError,
}: UseSettingsControllerArgs) {
  const [settingsDraft, setSettingsDraft] = useState(DEFAULT_DESKTOP_SETTINGS);
  const [gatewaySettingsDraft, setGatewaySettingsDraft] = useState<any>(() =>
    ensureGatewayConfig({}),
  );
  const [gatewaySettingsDirty, setGatewaySettingsDirty] = useState(false);
  const [gatewaySettingsLoading, setGatewaySettingsLoading] = useState(false);
  const [gatewaySettingsSaving, setGatewaySettingsSaving] = useState(false);
  const [gatewaySettingsStatus, setGatewaySettingsStatus] = useState<string | null>(null);
  const [gatewaySettingsSource, setGatewaySettingsSource] =
    useState<GatewaySettingsSource>('gateway_api');
  const [commands, setCommands] = useState<SlashCommand[]>([]);
  const [commandsLoaded, setCommandsLoaded] = useState(false);
  const [commandsLoading, setCommandsLoading] = useState(false);
  const [commandsSaving, setCommandsSaving] = useState(false);
  const commandsLoadPromiseRef = useRef<Promise<SlashCommand[]> | null>(null);
  const [mcpServers, setMcpServers] = useState<DesktopMcpServer[]>([]);
  const [mcpServersLoading, setMcpServersLoading] = useState(false);
  const [mcpServersSaving, setMcpServersSaving] = useState(false);
  const [savingSettings, setSavingSettings] = useState(false);
  const [localSettingsStatus, setLocalSettingsStatus] = useState<string | null>(null);
  const [settingsActiveTab, setSettingsActiveTab] = useState<SettingsTabId>(() =>
    normalizeSettingsTab(initialSettingsTab),
  );

  const gatewaySettingsDraftRef = useRef<any>(ensureGatewayConfig({}));
  const gatewaySettingsDirtyRef = useRef(false);
  const gatewaySettingsSavingRef = useRef(false);
  const gatewaySaveGenerationRef = useRef(0);
  const gatewayAutoSaveTimerRef = useRef<number | null>(null);
  const localSettingsDirty = Boolean(
    desktopState && !desktopSettingsEqual(settingsDraft, desktopState.settings),
  );

  function replaceGatewaySettings(payload: GatewaySettingsPayload) {
    const normalized = ensureGatewayConfig(payload.config);
    gatewaySettingsDraftRef.current = normalized;
    setGatewaySettingsDraft(normalized);
    gatewaySettingsDirtyRef.current = false;
    setGatewaySettingsDirty(false);
    setGatewaySettingsSource(payload.source);
  }

  function mutateGatewaySettingsDraft(mutator: (nextConfig: any) => void) {
    const next = ensureGatewayConfig(cloneJson(gatewaySettingsDraftRef.current));
    mutator(next);
    gatewaySettingsDraftRef.current = next;
    setGatewaySettingsDraft(next);
    gatewaySettingsDirtyRef.current = true;
    setGatewaySettingsDirty(true);
    setGatewaySettingsStatus(null);
    scheduleGatewayAutoSave();
  }

  function scheduleGatewayAutoSave() {
    if (gatewayAutoSaveTimerRef.current !== null) {
      window.clearTimeout(gatewayAutoSaveTimerRef.current);
    }
    gatewayAutoSaveTimerRef.current = window.setTimeout(() => {
      gatewayAutoSaveTimerRef.current = null;
      if (gatewaySettingsSavingRef.current) {
        scheduleGatewayAutoSave();
        return;
      }
      void handleSaveGatewaySettings({ silent: true });
    }, 600);
  }

  async function flushGatewayAutoSave(): Promise<void> {
    if (gatewayAutoSaveTimerRef.current !== null) {
      window.clearTimeout(gatewayAutoSaveTimerRef.current);
      gatewayAutoSaveTimerRef.current = null;
    }
    while (gatewaySettingsSavingRef.current) {
      await new Promise((resolve) => window.setTimeout(resolve, 40));
    }
    if (gatewaySettingsDirtyRef.current) {
      await handleSaveGatewaySettings({ silent: true });
    }
  }

  async function waitForGatewaySettingsSave(): Promise<void> {
    while (gatewaySettingsSavingRef.current) {
      await new Promise((resolve) => window.setTimeout(resolve, 40));
    }
  }

  useEffect(() => {
    return () => {
      if (gatewayAutoSaveTimerRef.current !== null) {
        window.clearTimeout(gatewayAutoSaveTimerRef.current);
      }
    };
  }, []);

  async function loadGatewaySettings(options?: {
    clearStatus?: boolean;
  }) {
    if (options?.clearStatus) {
      setGatewaySettingsStatus(null);
    }

    setGatewaySettingsLoading(true);
    try {
      const payload = await window.garyxDesktop.getGatewaySettings();
      replaceGatewaySettings(payload);
    } catch (gatewayError) {
      setGatewaySettingsStatus(
        gatewayError instanceof Error
          ? gatewayError.message
          : 'Failed to load gateway config',
      );
    } finally {
      setGatewaySettingsLoading(false);
    }
  }

  async function loadSlashCommands(options: { force?: boolean; silent?: boolean } = {}) {
    if (!options.force && commandsLoaded) {
      return;
    }
    if (commandsLoadPromiseRef.current) {
      await commandsLoadPromiseRef.current;
      return;
    }

    if (!options.silent) {
      setCommandsLoading(true);
    }
    if (!options.silent) {
      setError(null);
    }
    const loadPromise = window.garyxDesktop.listSlashCommands();
    commandsLoadPromiseRef.current = loadPromise;
    try {
      const nextCommands = await loadPromise;
      startTransition(() => {
        setCommands(nextCommands);
        setCommandsLoaded(true);
      });
    } catch (loadError) {
      if (!options.silent) {
        setError(
          loadError instanceof Error
            ? loadError.message
            : 'Failed to load slash commands',
        );
      }
    } finally {
      commandsLoadPromiseRef.current = null;
      if (!options.silent) {
        setCommandsLoading(false);
      }
    }
  }

  useEffect(() => {
    const preload = () => {
      void loadSlashCommands({ silent: true });
    };
    if ('requestIdleCallback' in window) {
      const idleId = window.requestIdleCallback(preload, { timeout: 1800 });
      return () => {
        window.cancelIdleCallback(idleId);
      };
    }
    const timeoutId = globalThis.setTimeout(preload, 500);
    return () => {
      globalThis.clearTimeout(timeoutId);
    };
  }, []);

  async function loadMcpServers() {
    setMcpServersLoading(true);
    setError(null);
    try {
      const nextServers = await window.garyxDesktop.listMcpServers();
      startTransition(() => {
        setMcpServers(nextServers);
      });
    } catch (loadError) {
      setError(
        loadError instanceof Error
          ? loadError.message
          : 'Failed to load MCP servers',
      );
    } finally {
      setMcpServersLoading(false);
    }
  }

  async function refreshSettingsTabResources(tabId: SettingsTabId): Promise<void> {
    if (tabId === 'commands') {
      await loadSlashCommands({ force: true });
      return;
    }
    if (tabId === 'mcp') {
      await loadMcpServers();
      return;
    }
    if (isGatewayConfigSettingsTab(tabId)) {
      await loadGatewaySettings({ clearStatus: true });
    }
  }

  async function handleCreateSlashCommand(input: UpsertSlashCommandInput): Promise<void> {
    if (commandsSaving) {
      return;
    }

    setCommandsSaving(true);
    setError(null);
    try {
      await window.garyxDesktop.createSlashCommand(input);
      await loadSlashCommands({ force: true });
    } catch (saveError) {
      setError(
        saveError instanceof Error
          ? saveError.message
          : 'Failed to create slash command',
      );
      throw saveError;
    } finally {
      setCommandsSaving(false);
    }
  }

  async function handleUpdateSlashCommand(input: UpdateSlashCommandInput): Promise<void> {
    if (commandsSaving) {
      return;
    }

    setCommandsSaving(true);
    setError(null);
    try {
      await window.garyxDesktop.updateSlashCommand(input);
      await loadSlashCommands({ force: true });
    } catch (saveError) {
      setError(
        saveError instanceof Error
          ? saveError.message
          : 'Failed to update slash command',
      );
      throw saveError;
    } finally {
      setCommandsSaving(false);
    }
  }

  async function handleDeleteSlashCommand(name: string): Promise<void> {
    if (commandsSaving) {
      return;
    }

    setCommandsSaving(true);
    setError(null);
    try {
      await window.garyxDesktop.deleteSlashCommand({ name });
      await loadSlashCommands({ force: true });
    } catch (deleteError) {
      setError(
        deleteError instanceof Error
          ? deleteError.message
          : 'Failed to delete slash command',
      );
      throw deleteError;
    } finally {
      setCommandsSaving(false);
    }
  }

  async function handleCreateMcpServer(input: UpsertMcpServerInput): Promise<void> {
    if (mcpServersSaving) {
      return;
    }

    setMcpServersSaving(true);
    setError(null);
    try {
      await window.garyxDesktop.createMcpServer(input);
      await loadMcpServers();
    } catch (saveError) {
      setError(
        saveError instanceof Error
          ? saveError.message
          : 'Failed to create MCP server',
      );
      throw saveError;
    } finally {
      setMcpServersSaving(false);
    }
  }

  async function handleUpdateMcpServer(input: UpdateMcpServerInput): Promise<void> {
    if (mcpServersSaving) {
      return;
    }

    setMcpServersSaving(true);
    setError(null);
    try {
      await window.garyxDesktop.updateMcpServer(input);
      await loadMcpServers();
    } catch (saveError) {
      setError(
        saveError instanceof Error
          ? saveError.message
          : 'Failed to update MCP server',
      );
      throw saveError;
    } finally {
      setMcpServersSaving(false);
    }
  }

  async function handleDeleteMcpServer(name: string): Promise<void> {
    if (mcpServersSaving) {
      return;
    }

    setMcpServersSaving(true);
    setError(null);
    try {
      await window.garyxDesktop.deleteMcpServer({ name });
      await loadMcpServers();
    } catch (deleteError) {
      setError(
        deleteError instanceof Error
          ? deleteError.message
          : 'Failed to delete MCP server',
      );
      throw deleteError;
    } finally {
      setMcpServersSaving(false);
    }
  }

  async function handleToggleMcpServer(name: string, enabled: boolean): Promise<void> {
    setMcpServersSaving(true);
    setError(null);
    try {
      await window.garyxDesktop.toggleMcpServer({ name, enabled });
      await loadMcpServers();
    } catch (toggleError) {
      setError(
        toggleError instanceof Error
          ? toggleError.message
          : 'Failed to toggle MCP server',
      );
    } finally {
      setMcpServersSaving(false);
    }
  }

  async function persistLocalSettings(
    options?: {
      refreshConnection?: boolean;
      requireGatewayConnection?: boolean;
      reloadGatewaySettings?: boolean;
    },
    draftOverride?: typeof DEFAULT_DESKTOP_SETTINGS,
  ): Promise<boolean> {
    const draft = draftOverride ?? settingsDraft;
    if (savingSettings) {
      return true;
    }

    setSavingSettings(true);
    setError(null);
    setLocalSettingsStatus(null);
    try {
      if (options?.requireGatewayConnection) {
        const status = await window.garyxDesktop.checkConnection({
          gatewayUrl: draft.gatewayUrl,
          gatewayAuthToken: draft.gatewayAuthToken,
        });
        setConnection(status);
        if (!status.ok) {
          const message = status.error || 'Unable to verify gateway connection';
          setLocalSettingsStatus(message);
          setError(message);
          return false;
        }
      }

      let nextState = await window.garyxDesktop.saveSettings(draft);
      setDesktopState(nextState);

      if (options?.requireGatewayConnection) {
        nextState = await window.garyxDesktop.rememberGatewayProfile();
        setDesktopState(nextState);
      } else if (options?.refreshConnection) {
        const status = await window.garyxDesktop.checkConnection();
        setConnection(status);
      }

      if (options?.reloadGatewaySettings) {
        await loadGatewaySettings({ clearStatus: true });
      }
      return true;
    } catch (saveError) {
      const message = saveError instanceof Error ? saveError.message : 'Failed to save local settings';
      setLocalSettingsStatus(message);
      setError(message);
      return false;
    } finally {
      setSavingSettings(false);
    }
  }

  async function handleSaveSettings(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    await persistLocalSettings({
      requireGatewayConnection: settingsActiveTab === 'gateway',
      refreshConnection: true,
      reloadGatewaySettings: !isLocalSettingsTab(settingsActiveTab),
    });
  }

  async function handleSaveLocalSettingsNow(options?: {
    requireGatewayConnection?: boolean;
    reloadGatewaySettings?: boolean;
  }): Promise<boolean> {
    return persistLocalSettings({
      requireGatewayConnection: options?.requireGatewayConnection ?? false,
      refreshConnection: true,
      reloadGatewaySettings: options?.reloadGatewaySettings ?? !isLocalSettingsTab(settingsActiveTab),
    });
  }

  async function handleSaveLocalSettingsDraft(
    nextSettings: typeof DEFAULT_DESKTOP_SETTINGS,
    options?: {
      requireGatewayConnection?: boolean;
      reloadGatewaySettings?: boolean;
    },
  ): Promise<boolean> {
    setSettingsDraft(nextSettings);
    return persistLocalSettings({
      requireGatewayConnection: options?.requireGatewayConnection ?? false,
      refreshConnection: true,
      reloadGatewaySettings: options?.reloadGatewaySettings ?? !isLocalSettingsTab(settingsActiveTab),
    }, nextSettings);
  }

  async function refreshDesktopStateAfterGatewaySave(
    saveGeneration: number,
  ): Promise<void> {
    const [status, nextState] = await measureUiAction(
      "bot.modify.refresh_desktop_state",
      () =>
        Promise.all([
          window.garyxDesktop.checkConnection(),
          window.garyxDesktop.getState(),
        ]),
    );
    if (saveGeneration !== gatewaySaveGenerationRef.current) {
      return;
    }
    startTransition(() => {
      setConnection(status);
      setDesktopState(nextState);
    });
  }

  async function handleSaveGatewaySettings(
    options?: GatewaySettingsSaveOptions,
  ): Promise<boolean> {
    if (gatewaySettingsSavingRef.current) {
      return false;
    }
    const silent = options?.silent === true;
    const refreshDesktopState = options?.refreshDesktopState ?? 'await';
    const saveGeneration = gatewaySaveGenerationRef.current + 1;
    gatewaySaveGenerationRef.current = saveGeneration;
    if (gatewayAutoSaveTimerRef.current !== null) {
      window.clearTimeout(gatewayAutoSaveTimerRef.current);
      gatewayAutoSaveTimerRef.current = null;
    }
    gatewaySettingsSavingRef.current = true;
    setGatewaySettingsSaving(true);

    let nextConfig = ensureGatewayConfig(gatewaySettingsDraftRef.current);
    nextConfig.gateway.public_url = settingsDraft.gatewayUrl || '';

    try {
      const result = await measureUiAction("bot.modify.save_gateway_settings", () =>
        window.garyxDesktop.saveGatewaySettings(nextConfig),
      );
      replaceGatewaySettings(result.settings);
      if (!silent) {
        setGatewaySettingsStatus('Saved');
      }
      // saveGatewaySettings only returns the gateway config payload, so
      // configuredBots / botMainThreads in DesktopState stay stale after a
      // bot delete until the next poll. Refresh the whole state so the
      // sidebar reflects the save immediately.
      if (refreshDesktopState === 'await') {
        await refreshDesktopStateAfterGatewaySave(saveGeneration);
      } else if (refreshDesktopState === 'background') {
        void refreshDesktopStateAfterGatewaySave(saveGeneration).catch(
          (refreshError) => {
            console.warn(
              'Failed to refresh desktop state after gateway settings save.',
              refreshError,
            );
          },
        );
      }
      return true;
    } catch (gatewayError) {
      setGatewaySettingsStatus(
        gatewayError instanceof Error
          ? gatewayError.message
          : 'Failed to save gateway config',
      );
      return false;
    } finally {
      gatewaySettingsSavingRef.current = false;
      setGatewaySettingsSaving(false);
    }
  }

  async function handleSaveGatewaySettingsPatch(
    patch: GatewayConfigDocument,
    options?: GatewaySettingsSaveOptions,
  ): Promise<boolean> {
    if (gatewaySettingsSavingRef.current) {
      await waitForGatewaySettingsSave();
    }
    const silent = options?.silent === true;
    const refreshDesktopState = options?.refreshDesktopState ?? 'background';
    const saveGeneration = gatewaySaveGenerationRef.current + 1;
    const hadDirtyDraft = gatewaySettingsDirtyRef.current;
    const previousDraft = ensureGatewayConfig(
      cloneJson(gatewaySettingsDraftRef.current),
    );
    gatewaySaveGenerationRef.current = saveGeneration;
    if (gatewayAutoSaveTimerRef.current !== null) {
      window.clearTimeout(gatewayAutoSaveTimerRef.current);
      gatewayAutoSaveTimerRef.current = null;
    }
    const optimisticDraft = mergeGatewayConfigPatch(
      gatewaySettingsDraftRef.current,
      patch,
    );
    gatewaySettingsDraftRef.current = optimisticDraft;
    setGatewaySettingsDraft(optimisticDraft);
    gatewaySettingsDirtyRef.current = hadDirtyDraft;
    setGatewaySettingsDirty(hadDirtyDraft);
    setGatewaySettingsStatus(null);
    gatewaySettingsSavingRef.current = true;
    setGatewaySettingsSaving(true);

    try {
      const result = await measureUiAction("bot.modify.save_gateway_settings_patch", () =>
        window.garyxDesktop.saveGatewaySettings(patch, { merge: true }),
      );
      if (gatewaySettingsDirtyRef.current) {
        setGatewaySettingsSource(result.settings.source);
      } else {
        replaceGatewaySettings(result.settings);
      }
      if (!silent) {
        setGatewaySettingsStatus('Saved');
      }
      if (refreshDesktopState === 'await') {
        await refreshDesktopStateAfterGatewaySave(saveGeneration);
      } else if (refreshDesktopState === 'background') {
        void refreshDesktopStateAfterGatewaySave(saveGeneration).catch(
          (refreshError) => {
            console.warn(
              'Failed to refresh desktop state after gateway settings patch save.',
              refreshError,
            );
          },
        );
      }
      return true;
    } catch (gatewayError) {
      setGatewaySettingsStatus(
        gatewayError instanceof Error
          ? gatewayError.message
          : 'Failed to save gateway config',
      );
      const keepDirtyDraft = hadDirtyDraft || gatewaySettingsDirtyRef.current;
      const restoredDraft = restoreGatewayConfigPatch(
        gatewaySettingsDraftRef.current,
        patch,
        previousDraft,
      );
      gatewaySettingsDraftRef.current = restoredDraft;
      setGatewaySettingsDraft(restoredDraft);
      gatewaySettingsDirtyRef.current = keepDirtyDraft;
      setGatewaySettingsDirty(keepDirtyDraft);
      return false;
    } finally {
      gatewaySettingsSavingRef.current = false;
      setGatewaySettingsSaving(false);
      if (gatewaySettingsDirtyRef.current) {
        scheduleGatewayAutoSave();
      }
    }
  }

  function handleRetrySettingsView() {
    setError(null);
    if (isLocalSettingsTab(settingsActiveTab)) {
      return;
    }
    void refreshSettingsTabResources(settingsActiveTab);
  }

  async function handleSelectSettingsTab(nextTab: SettingsTabId): Promise<boolean> {
    const normalizedNextTab: SettingsTabId = nextTab === 'connection' ? 'gateway' : nextTab;
    const nextTabIsLocal = isLocalSettingsTab(normalizedNextTab);

    if (normalizedNextTab === settingsActiveTab) {
      if (!nextTabIsLocal) {
        if (isGatewayConfigSettingsTab(normalizedNextTab)) {
          await flushGatewayAutoSave();
        }
        await refreshSettingsTabResources(normalizedNextTab);
      }
      return true;
    }

    if (isGatewayConfigSettingsTab(settingsActiveTab)) {
      await flushGatewayAutoSave();
    }

    setSettingsActiveTab(normalizedNextTab);
    if (!nextTabIsLocal) {
      await refreshSettingsTabResources(normalizedNextTab);
    } else {
      setGatewaySettingsStatus(null);
    }
    return true;
  }

  useEffect(() => {
    if (settingsActiveTab === 'connection') {
      setSettingsActiveTab('gateway');
    }
  }, [settingsActiveTab]);

  return {
    commands,
    commandsLoaded,
    commandsLoading,
    commandsSaving,
    gatewaySettingsDirty,
    gatewaySettingsDraft,
    gatewaySettingsLoading,
    gatewaySettingsSaving,
    gatewaySettingsSource,
    gatewaySettingsStatus,
    handleCreateMcpServer,
    handleCreateSlashCommand,
    handleDeleteMcpServer,
    handleDeleteSlashCommand,
    handleRetrySettingsView,
    handleSaveGatewaySettings,
    handleSaveGatewaySettingsPatch,
    handleSaveLocalSettingsDraft,
    handleSaveLocalSettingsNow,
    handleSaveSettings,
    handleSelectSettingsTab,
    handleToggleMcpServer,
    handleUpdateMcpServer,
    handleUpdateSlashCommand,
    loadGatewaySettings,
    loadSlashCommands,
    localSettingsDirty,
    localSettingsStatus,
    mcpServers,
    mcpServersLoading,
    mcpServersSaving,
    mutateGatewaySettingsDraft,
    persistLocalSettings,
    refreshSettingsTabResources,
    savingSettings,
    setLocalSettingsStatus,
    setSettingsDraft,
    setGatewaySettingsStatus,
    settingsActiveTab,
    settingsDraft,
  };
}
