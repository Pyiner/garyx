import type {
  AddChannelAccountInput,
  DesktopSettings,
  GatewayConfigDocument,
} from "@shared/contracts";

import { fetchGatewaySettings, saveGatewaySettings, validateChannelAccount } from "./gary-client.ts";

const DEFAULT_WEIXIN_BASE_URL = "https://ilinkai.weixin.qq.com";

function ensureRecord(value: unknown): Record<string, any> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, any>)
    : {};
}

function normalizeWeixinBaseUrl(input?: string | null): string {
  return (input || DEFAULT_WEIXIN_BASE_URL).trim().replace(/\/+$/, "") || DEFAULT_WEIXIN_BASE_URL;
}

function normalizeOptionalText(input?: string | null): string | null {
  const value = typeof input === "string" ? input.trim() : "";
  return value ? value : null;
}

function canonicalPluginId(channel: string): string {
  return channel.trim().toLowerCase();
}

function ensureFlatChannelBuckets(channels: Record<string, any>): Record<string, any> {
  const legacyPlugins = ensureRecord(channels.plugins);
  for (const [channelId, channelValue] of Object.entries(legacyPlugins)) {
    if (!(channelId in channels)) {
      channels[channelId] = channelValue;
    }
  }
  delete channels.plugins;
  return channels;
}

type PreparedChannelAccount = {
  pluginId: string;
  accountName: string | null;
  workspaceDir: string | null;
  workspaceMode: "local" | "worktree";
  agentId: string | null;
  config: Record<string, unknown>;
};

function prepareChannelAccount(input: AddChannelAccountInput): PreparedChannelAccount {
  const accountName = normalizeOptionalText(input.name);
  const workspaceDir = normalizeOptionalText(input.workspaceDir);
  const workspaceMode = input.workspaceMode === "worktree" ? "worktree" : "local";
  const pluginId = canonicalPluginId(input.channel);
  const pluginConfig =
    input.config && typeof input.config === "object" && !Array.isArray(input.config)
      ? { ...input.config }
      : {};

  if (pluginId === "telegram") {
    const token = normalizeOptionalText(input.token) || normalizeOptionalText(pluginConfig.token as string | null);
    if (!token) {
      throw new Error("Telegram token is required.");
    }
    pluginConfig.token = token;
  } else if (pluginId === "feishu") {
    const appId = normalizeOptionalText(input.appId) || normalizeOptionalText(pluginConfig.app_id as string | null);
    const appSecret =
      normalizeOptionalText(input.appSecret) || normalizeOptionalText(pluginConfig.app_secret as string | null);
    if (!appId || !appSecret) {
      throw new Error("Feishu app_id and app_secret are required.");
    }
    pluginConfig.app_id = appId;
    pluginConfig.app_secret = appSecret;
    pluginConfig.domain =
      (input.domain === "lark" ? "lark" : normalizeOptionalText(pluginConfig.domain as string | null))
      || "feishu";
  } else if (pluginId === "weixin") {
    const token = normalizeOptionalText(input.token) || normalizeOptionalText(pluginConfig.token as string | null);
    if (!token) {
      throw new Error("Weixin token is required.");
    }
    pluginConfig.token = token;
    pluginConfig.uin = normalizeOptionalText(input.uin) || String(pluginConfig.uin || "");
    pluginConfig.base_url =
      normalizeOptionalText(input.baseUrl)
      || normalizeOptionalText(pluginConfig.base_url as string | null)
      || normalizeWeixinBaseUrl(null);
  }

  return {
    pluginId,
    accountName,
    workspaceDir,
    workspaceMode,
    agentId: normalizeOptionalText(input.agentId),
    config: pluginConfig,
  };
}

function upsertChannelAccount(config: GatewayConfigDocument, input: AddChannelAccountInput): GatewayConfigDocument {
  const next = JSON.parse(JSON.stringify(config || {})) as GatewayConfigDocument;
  const root = ensureRecord(next);
  root.channels = ensureRecord(root.channels);
  ensureFlatChannelBuckets(root.channels);

  const prepared = prepareChannelAccount(input);
  const pluginId = prepared.pluginId;

  root.channels[pluginId] = ensureRecord(root.channels[pluginId]);
  root.channels[pluginId].accounts = ensureRecord(root.channels[pluginId].accounts);
  root.channels[pluginId].accounts[input.accountId] = {
    enabled: true,
    name: prepared.accountName,
    agent_id: prepared.agentId,
    workspace_dir: prepared.workspaceDir,
    workspace_mode: prepared.workspaceMode,
    config: prepared.config,
  };

  return root;
}

export async function addChannelAccount(
  settings: DesktopSettings,
  input: AddChannelAccountInput,
) {
  const accountId = input.accountId.trim();
  if (!accountId) {
    throw new Error("Account ID is required.");
  }
  const prepared = prepareChannelAccount({
    ...input,
    accountId,
  });
  await validateChannelAccount(settings, prepared.pluginId, {
    accountId,
    enabled: true,
    config: prepared.config,
  });
  const current = await fetchGatewaySettings(settings);
  const config = upsertChannelAccount(current.config, {
    ...input,
    accountId,
  });
  await saveGatewaySettings(settings, config);
}
