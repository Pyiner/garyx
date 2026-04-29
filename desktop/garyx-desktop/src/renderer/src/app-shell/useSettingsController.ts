import { startTransition, useEffect, useRef, useState } from 'react';

import {
  DEFAULT_DESKTOP_SETTINGS,
  type ConnectionStatus,
  type DesktopMcpServer,
  type DesktopState,
  type GatewaySettingsPayload,
  type GatewaySettingsSource,
  type SlashCommand,
  type UpdateMcpServerInput,
  type UpdateSlashCommandInput,
  type UpsertMcpServerInput,
  type UpsertSlashCommandInput,
} from '@shared/contracts';

import type { SettingsTabId } from '../GatewaySettingsPanel';
import {
  cloneJson,
  ensureGatewayConfig,
} from '../gateway-settings';
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
    left.threadLogsPanelWidth === right.threadLogsPanelWidth &&
    left.languagePreference === right.languagePreference
  );
}

type UseSettingsControllerArgs = {
  desktopState: DesktopState | null;
  setDesktopState: React.Dispatch<React.SetStateAction<DesktopState | null>>;
  setConnection: React.Dispatch<React.SetStateAction<ConnectionStatus | null>>;
  setError: React.Dispatch<React.SetStateAction<string | null>>;
};

export function useSettingsController({
  desktopState,
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
  const [settingsActiveTab, setSettingsActiveTab] = useState<SettingsTabId>('gateway');

  const gatewaySettingsDraftRef = useRef<any>(ensureGatewayConfig({}));
  const gatewaySettingsSavingRef = useRef(false);
  const gatewayAutoSaveTimerRef = useRef<number | null>(null);
  const localSettingsDirty = Boolean(
    desktopState && !desktopSettingsEqual(settingsDraft, desktopState.settings),
  );

  function replaceGatewaySettings(payload: GatewaySettingsPayload) {
    const normalized = ensureGatewayConfig(payload.config);
    gatewaySettingsDraftRef.current = normalized;
    setGatewaySettingsDraft(normalized);
    setGatewaySettingsDirty(false);
    setGatewaySettingsSource(payload.source);
  }

  function mutateGatewaySettingsDraft(mutator: (nextConfig: any) => void) {
    const next = ensureGatewayConfig(cloneJson(gatewaySettingsDraftRef.current));
    mutator(next);
    gatewaySettingsDraftRef.current = next;
    setGatewaySettingsDraft(next);
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
    if (gatewaySettingsDirty) {
      await handleSaveGatewaySettings({ silent: true });
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

  async function persistLocalSettings(options?: {
    refreshConnection?: boolean;
    requireGatewayConnection?: boolean;
    reloadGatewaySettings?: boolean;
  }): Promise<boolean> {
    if (savingSettings) {
      return true;
    }

    setSavingSettings(true);
    setError(null);
    try {
      if (options?.requireGatewayConnection) {
        const status = await window.garyxDesktop.checkConnection({
          gatewayUrl: settingsDraft.gatewayUrl,
          gatewayAuthToken: settingsDraft.gatewayAuthToken,
        });
        setConnection(status);
        if (!status.ok) {
          setError(status.error || 'Unable to verify gateway connection');
          return false;
        }
      }

      let nextState = await window.garyxDesktop.saveSettings(settingsDraft);
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
      setError(saveError instanceof Error ? saveError.message : 'Failed to save local settings');
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

  async function handleSaveGatewaySettings(
    options?: { silent?: boolean },
  ): Promise<boolean> {
    if (gatewaySettingsSavingRef.current) {
      return false;
    }
    const silent = options?.silent === true;
    gatewaySettingsSavingRef.current = true;
    setGatewaySettingsSaving(true);

    let nextConfig = ensureGatewayConfig(gatewaySettingsDraftRef.current);
    nextConfig.gateway.public_url = settingsDraft.gatewayUrl || '';

    try {
      const result = await window.garyxDesktop.saveGatewaySettings(nextConfig);
      replaceGatewaySettings(result.settings);
      if (!silent) {
        setGatewaySettingsStatus(result.message || 'Saved gateway config.');
      }
      const status = await window.garyxDesktop.checkConnection();
      setConnection(status);
      // saveGatewaySettings only returns the gateway config payload, so
      // configuredBots / botMainThreads in DesktopState stay stale after a
      // bot delete until the next poll. Refresh the whole state so the
      // sidebar reflects the save immediately.
      const nextState = await window.garyxDesktop.getState();
      setDesktopState(nextState);
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
    handleSaveLocalSettingsNow,
    handleSaveSettings,
    handleSelectSettingsTab,
    handleToggleMcpServer,
    handleUpdateMcpServer,
    handleUpdateSlashCommand,
    loadGatewaySettings,
    loadSlashCommands,
    localSettingsDirty,
    mcpServers,
    mcpServersLoading,
    mcpServersSaving,
    mutateGatewaySettingsDraft,
    persistLocalSettings,
    refreshSettingsTabResources,
    savingSettings,
    setSettingsDraft,
    setGatewaySettingsStatus,
    settingsActiveTab,
    settingsDraft,
  };
}
