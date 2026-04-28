import { useCallback, useEffect, useState } from 'react';

import type {
  DesktopBotConsoleSummary,
  GatewaySettingsPayload,
  GatewayThreadHistoryBackend,
} from '@shared/contracts';

import { cloneJson, stringifyJsonBlock } from '@renderer/gateway-settings';

import { fetchBotConsoles, fetchGatewaySettings, saveGatewaySettings } from './web-api';
import type { WebRoute } from './web-route';

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function ensureFlatChannelBuckets(channels: Record<string, unknown>) {
  const legacyPlugins = asRecord(channels.plugins);
  for (const [channelId, channelValue] of Object.entries(legacyPlugins)) {
    if (!(channelId in channels)) {
      channels[channelId] = channelValue;
    }
  }
  delete channels.plugins;
  return channels;
}

function ensureChannelAccounts(root: Record<string, unknown>, channelId: string) {
  const channels = asRecord(root.channels);
  ensureFlatChannelBuckets(channels);
  const channel = asRecord(channels[channelId]);
  const accounts = asRecord(channel.accounts);
  channel.accounts = accounts;
  channels[channelId] = channel;
  root.channels = channels;
  return accounts;
}

function ensureChannelAccount(
  root: Record<string, unknown>,
  channelId: string,
  accountId: string,
  defaultConfig: Record<string, unknown>,
) {
  const accounts = ensureChannelAccounts(root, channelId);
  const account = asRecord(accounts[accountId]);
  account.enabled = typeof account.enabled === 'boolean' ? account.enabled : true;
  account.name = typeof account.name === 'string' ? account.name : '';
  account.agent_id = typeof account.agent_id === 'string' ? account.agent_id : 'claude';
  account.workspace_dir = typeof account.workspace_dir === 'string' ? account.workspace_dir : '';
  account.config = {
    ...defaultConfig,
    ...asRecord(account.config),
  };
  accounts[accountId] = account;
  return account;
}

export function useWebSettingsState(route: Extract<WebRoute, { view: 'settings' }>) {
  const [payload, setPayload] = useState<GatewaySettingsPayload | null>(null);
  const [focusedBotSummary, setFocusedBotSummary] = useState<DesktopBotConsoleSummary | null>(null);
  const [jsonDraft, setJsonDraft] = useState('{}');
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    setStatus(null);
    try {
      const nextPayload = await fetchGatewaySettings();
      setPayload(nextPayload);
      setJsonDraft(stringifyJsonBlock(nextPayload.config));
      if (route.botId) {
        const botSummaries = await fetchBotConsoles();
        setFocusedBotSummary(botSummaries.find((item) => item.id === route.botId) || null);
      } else {
        setFocusedBotSummary(null);
      }
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : 'failed to load gateway settings');
    } finally {
      setLoading(false);
    }
  }, [route.botId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const save = useCallback(async () => {
    setSaving(true);
    setError(null);
    setStatus(null);
    try {
      const nextConfig = cloneJson(JSON.parse(jsonDraft));
      const result = await saveGatewaySettings(nextConfig);
      setPayload(result.settings);
      setJsonDraft(stringifyJsonBlock(result.settings.config));
      setStatus(result.message || 'Settings saved.');
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : 'failed to save gateway settings');
    } finally {
      setSaving(false);
    }
  }, [jsonDraft]);

  const patchGateway = useCallback((patch: {
    host?: string;
    port?: number;
    publicUrl?: string;
  }) => {
    try {
      const current = cloneJson(JSON.parse(jsonDraft));
      const next = asRecord(current);
      const gateway = asRecord(next.gateway);
      if (typeof patch.host === 'string') {
        gateway.host = patch.host;
      }
      if (typeof patch.port === 'number' && Number.isFinite(patch.port)) {
        gateway.port = patch.port;
      }
      if (typeof patch.publicUrl === 'string') {
        const normalized = patch.publicUrl.trim();
        gateway.public_url = normalized || null;
      }
      next.gateway = gateway;
      setJsonDraft(stringifyJsonBlock(next));
      setStatus(null);
      setError(null);
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : 'failed to patch gateway settings');
    }
  }, [jsonDraft]);

  const patchHeartbeat = useCallback((patch: {
    enabled?: boolean;
    every?: string;
    target?: string;
    ackMaxChars?: number;
    activeHoursStart?: string;
    activeHoursEnd?: string;
    activeHoursTimezone?: string;
  }) => {
    try {
      const current = cloneJson(JSON.parse(jsonDraft));
      const next = asRecord(current);
      const agentDefaults = asRecord(next.agent_defaults);
      const heartbeat = asRecord(agentDefaults.heartbeat);
      const activeHours = asRecord(heartbeat.active_hours);
      if (typeof patch.enabled === 'boolean') {
        heartbeat.enabled = patch.enabled;
      }
      if (typeof patch.every === 'string') {
        heartbeat.every = patch.every;
      }
      if (typeof patch.target === 'string') {
        heartbeat.target = patch.target;
      }
      if (typeof patch.ackMaxChars === 'number' && Number.isFinite(patch.ackMaxChars)) {
        heartbeat.ack_max_chars = patch.ackMaxChars;
      }
      if (typeof patch.activeHoursStart === 'string') {
        activeHours.start = patch.activeHoursStart;
      }
      if (typeof patch.activeHoursEnd === 'string') {
        activeHours.end = patch.activeHoursEnd;
      }
      if (typeof patch.activeHoursTimezone === 'string') {
        activeHours.timezone = patch.activeHoursTimezone;
      }
      heartbeat.active_hours = activeHours;
      agentDefaults.heartbeat = heartbeat;
      next.agent_defaults = agentDefaults;
      setJsonDraft(stringifyJsonBlock(next));
      setStatus(null);
      setError(null);
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : 'failed to patch heartbeat settings');
    }
  }, [jsonDraft]);

  const patchSessions = useCallback((patch: {
    dataDir?: string;
    backend?: GatewayThreadHistoryBackend;
  }) => {
    try {
      const current = cloneJson(JSON.parse(jsonDraft));
      const next = asRecord(current);
      const sessions = asRecord(next.sessions);
      if (typeof patch.dataDir === 'string') {
        const normalized = patch.dataDir.trim();
        sessions.data_dir = normalized || null;
      }
      if (typeof patch.backend === 'string') {
        sessions.thread_history_backend = patch.backend;
      }
      next.sessions = sessions;
      setJsonDraft(stringifyJsonBlock(next));
      setStatus(null);
      setError(null);
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : 'failed to patch thread storage settings');
    }
  }, [jsonDraft]);

  const addTelegramAccount = useCallback((accountId: string) => {
    try {
      const normalizedId = accountId.trim();
      if (!normalizedId) {
        return;
      }
      const current = cloneJson(JSON.parse(jsonDraft));
      const next = asRecord(current);
      ensureChannelAccount(next, 'telegram', normalizedId, {
        token: '',
        owner_target: null,
        groups: {},
      });
      setJsonDraft(stringifyJsonBlock(next));
      setStatus(null);
      setError(null);
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : 'failed to add telegram account');
    }
  }, [jsonDraft]);

  const removeTelegramAccount = useCallback((accountId: string) => {
    try {
      const current = cloneJson(JSON.parse(jsonDraft));
      const next = asRecord(current);
      const accounts = ensureChannelAccounts(next, 'telegram');
      delete accounts[accountId];
      setJsonDraft(stringifyJsonBlock(next));
      setStatus(null);
      setError(null);
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : 'failed to remove telegram account');
    }
  }, [jsonDraft]);

  const patchTelegramAccount = useCallback((accountId: string, patch: {
    enabled?: boolean;
    token?: string;
    name?: string;
    agentId?: string;
    workspaceDir?: string;
  }) => {
    try {
      const current = cloneJson(JSON.parse(jsonDraft));
      const next = asRecord(current);
      const account = ensureChannelAccount(next, 'telegram', accountId, {
        token: '',
        owner_target: null,
        groups: {},
      });
      const config = asRecord(account.config);
      Reflect.deleteProperty(config, 'allow_from');
      Reflect.deleteProperty(config, 'reply_to_mode');
      if (typeof patch.enabled === 'boolean') {
        account.enabled = patch.enabled;
      }
      if (typeof patch.token === 'string') {
        config.token = patch.token;
      }
      if (typeof patch.name === 'string') {
        account.name = patch.name;
      }
      if (typeof patch.agentId === 'string') {
        account.agent_id = patch.agentId;
      }
      if (typeof patch.workspaceDir === 'string') {
        account.workspace_dir = patch.workspaceDir;
      }
      account.config = config;
      setJsonDraft(stringifyJsonBlock(next));
      setStatus(null);
      setError(null);
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : 'failed to patch telegram account');
    }
  }, [jsonDraft]);

  const addFeishuAccount = useCallback((accountId: string) => {
    try {
      const normalizedId = accountId.trim();
      if (!normalizedId) {
        return;
      }
      const current = cloneJson(JSON.parse(jsonDraft));
      const next = asRecord(current);
      ensureChannelAccount(next, 'feishu', normalizedId, {
        app_id: '',
        app_secret: '',
        domain: 'feishu',
        require_mention: true,
        topic_session_mode: 'disabled',
      });
      setJsonDraft(stringifyJsonBlock(next));
      setStatus(null);
      setError(null);
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : 'failed to add feishu account');
    }
  }, [jsonDraft]);

  const removeFeishuAccount = useCallback((accountId: string) => {
    try {
      const current = cloneJson(JSON.parse(jsonDraft));
      const next = asRecord(current);
      const accounts = ensureChannelAccounts(next, 'feishu');
      delete accounts[accountId];
      setJsonDraft(stringifyJsonBlock(next));
      setStatus(null);
      setError(null);
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : 'failed to remove feishu account');
    }
  }, [jsonDraft]);

  const patchFeishuAccount = useCallback((accountId: string, patch: {
    enabled?: boolean;
    appId?: string;
    appSecret?: string;
    domain?: string;
    requireMention?: boolean;
    topicSessionMode?: string;
    name?: string;
    agentId?: string;
    workspaceDir?: string;
  }) => {
    try {
      const current = cloneJson(JSON.parse(jsonDraft));
      const next = asRecord(current);
      const account = ensureChannelAccount(next, 'feishu', accountId, {
        app_id: '',
        app_secret: '',
        domain: 'feishu',
        require_mention: true,
        topic_session_mode: 'disabled',
      });
      const config = asRecord(account.config);
      Reflect.deleteProperty(config, 'dm_policy');
      Reflect.deleteProperty(config, 'allow_from');
      Reflect.deleteProperty(config, 'group_policy');
      Reflect.deleteProperty(config, 'group_allow_from');
      Reflect.deleteProperty(config, 'groups');
      Reflect.deleteProperty(config, 'history_limit');
      if (typeof patch.enabled === 'boolean') {
        account.enabled = patch.enabled;
      }
      if (typeof patch.appId === 'string') {
        config.app_id = patch.appId;
      }
      if (typeof patch.appSecret === 'string') {
        config.app_secret = patch.appSecret;
      }
      if (typeof patch.domain === 'string') {
        config.domain = patch.domain;
      }
      if (typeof patch.requireMention === 'boolean') {
        config.require_mention = patch.requireMention;
      }
      if (typeof patch.topicSessionMode === 'string') {
        config.topic_session_mode = patch.topicSessionMode === 'enabled' ? 'enabled' : 'disabled';
      }
      if (typeof patch.name === 'string') {
        account.name = patch.name;
      }
      if (typeof patch.agentId === 'string') {
        account.agent_id = patch.agentId;
      }
      if (typeof patch.workspaceDir === 'string') {
        account.workspace_dir = patch.workspaceDir;
      }
      account.config = config;
      setJsonDraft(stringifyJsonBlock(next));
      setStatus(null);
      setError(null);
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : 'failed to patch feishu account');
    }
  }, [jsonDraft]);

  return {
    payload,
    focusedBotSummary,
    jsonDraft,
    setJsonDraft,
    loading,
    saving,
    error,
    status,
    refresh,
    save,
    patchGateway,
    patchHeartbeat,
    patchSessions,
    addTelegramAccount,
    removeTelegramAccount,
    patchTelegramAccount,
    addFeishuAccount,
    removeFeishuAccount,
    patchFeishuAccount,
  };
}
