import { randomUUID } from "node:crypto";

import QRCode from "qrcode";

import type {
  AddChannelAccountInput,
  DesktopSettings,
  GatewayConfigDocument,
  PollFeishuChannelAuthInput,
  PollFeishuChannelAuthResult,
  PollWeixinChannelAuthInput,
  PollWeixinChannelAuthResult,
  StartFeishuChannelAuthInput,
  StartFeishuChannelAuthResult,
  StartWeixinChannelAuthInput,
  StartWeixinChannelAuthResult,
} from "@shared/contracts";

import { fetchGatewaySettings, saveGatewaySettings, validateChannelAccount } from "./gary-client";

type WeixinQrStartResponse = {
  qrcode?: string;
  qrcode_img_content?: string;
};

type WeixinQrStatusResponse = {
  status?: string;
  bot_token?: string;
  ilink_bot_id?: string;
  baseurl?: string;
};

type PendingWeixinAuthSession = {
  accountId?: string | null;
  name?: string | null;
  workspaceDir?: string | null;
  baseUrl: string;
  qrCodeValue: string;
};

const DEFAULT_WEIXIN_BASE_URL = "https://ilinkai.weixin.qq.com";
const pendingWeixinSessions = new Map<string, PendingWeixinAuthSession>();

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
  agentId: string | null;
  config: Record<string, unknown>;
};

function prepareChannelAccount(input: AddChannelAccountInput): PreparedChannelAccount {
  const accountName = normalizeOptionalText(input.name);
  const workspaceDir = normalizeOptionalText(input.workspaceDir);
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
    agent_id: prepared.agentId || "claude",
    workspace_dir: prepared.workspaceDir,
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

export async function startWeixinChannelAuth(
  input: StartWeixinChannelAuthInput,
): Promise<StartWeixinChannelAuthResult> {
  const baseUrl = normalizeWeixinBaseUrl(input.baseUrl);
  const response = await fetch(`${baseUrl}/ilink/bot/get_bot_qrcode?bot_type=3`, {
    signal: AbortSignal.timeout(15_000),
  });
  if (!response.ok) {
    throw new Error(`Weixin QR request failed: ${response.status}`);
  }
  const payload = (await response.json()) as WeixinQrStartResponse;
  const qrCodeValue = typeof payload.qrcode === "string" ? payload.qrcode.trim() : "";
  if (!qrCodeValue) {
    throw new Error("Weixin QR response missing qrcode.");
  }
  // qrcode is the poll-side session token; qrcode_img_content is the URL the
  // WeChat client actually resolves when it scans the code. Encoding the raw
  // token into the QR makes generic scanners just read the hex back — the
  // WeChat auth flow never triggers. Prefer the server-provided URL and fall
  // back to the token only if the field is missing.
  const qrCodeImgContent =
    typeof payload.qrcode_img_content === "string"
      ? payload.qrcode_img_content.trim()
      : "";
  const qrRenderContent = qrCodeImgContent || qrCodeValue;
  const qrCodeDataUrl = await QRCode.toDataURL(qrRenderContent, {
    margin: 1,
    width: 320,
  });
  const sessionId = randomUUID();
  pendingWeixinSessions.set(sessionId, {
    accountId: normalizeOptionalText(input.accountId),
    name: normalizeOptionalText(input.name),
    workspaceDir: normalizeOptionalText(input.workspaceDir),
    baseUrl,
    qrCodeValue,
  });
  return {
    sessionId,
    qrCodeValue,
    qrCodeDataUrl,
    status: "wait",
  };
}

export async function pollWeixinChannelAuth(
  settings: DesktopSettings,
  input: PollWeixinChannelAuthInput,
): Promise<PollWeixinChannelAuthResult> {
  const session = pendingWeixinSessions.get(input.sessionId);
  if (!session) {
    throw new Error("Weixin auth session not found.");
  }
  const response = await fetch(
    `${session.baseUrl}/ilink/bot/get_qrcode_status?qrcode=${encodeURIComponent(session.qrCodeValue)}`,
    {
      headers: {
        "iLink-App-ClientVersion": "1",
      },
      signal: AbortSignal.timeout(15_000),
    },
  );
  if (!response.ok) {
    throw new Error(`Weixin QR status failed: ${response.status}`);
  }
  const payload = (await response.json()) as WeixinQrStatusResponse;
  const status = typeof payload.status === "string" && payload.status.trim()
    ? payload.status.trim()
    : "wait";

  if (status === "confirmed") {
    const token = normalizeOptionalText(payload.bot_token);
    const scannedAccountId = normalizeOptionalText(payload.ilink_bot_id);
    if (!token || !scannedAccountId) {
      throw new Error("Weixin auth confirmed but missing bot token or account id.");
    }
    const finalAccountId = session.accountId || scannedAccountId;
    await addChannelAccount(settings, {
      channel: "weixin",
      accountId: finalAccountId,
      name: session.name,
      workspaceDir: session.workspaceDir,
      token,
      baseUrl: normalizeOptionalText(payload.baseurl) || session.baseUrl,
      uin: "",
    });
    pendingWeixinSessions.delete(input.sessionId);
    return {
      status,
      accountId: finalAccountId,
    };
  }

  if (status === "expired") {
    pendingWeixinSessions.delete(input.sessionId);
  }

  return {
    status,
    accountId: session.accountId || null,
  };
}

// ---------------------------------------------------------------------------
// Feishu / Lark OAuth 2.0 Device Authorization Grant (RFC 8628).
//
// Mirrors `garyx_channels::feishu::device_auth` — same HTTP contract, same
// Lark-tenant retry quirk, same `archetype=PersonalAgent` hard-coded value.
// We duplicate the implementation (instead of shelling out to garyx) so the
// desktop stays functional when no Rust toolchain / CLI binary is installed.
// ---------------------------------------------------------------------------

type PendingFeishuAuthSession = {
  accountId?: string | null;
  name?: string | null;
  workspaceDir?: string | null;
  deviceCode: string;
  userCode: string;
  verificationUrl: string;
  expiresAt: number;
  interval: number;
  /** The accounts-server endpoint we started against; may switch to
   *  larksuite if the server returns a Lark tenant with empty secret. */
  pollDomain: "feishu" | "lark";
  /** Tracks whether we've already retried against the other brand. */
  lastDomainAttempted: "feishu" | "lark";
};

const FEISHU_ARCHETYPE = "PersonalAgent";
const FEISHU_ACCOUNTS_URL = "https://accounts.feishu.cn/oauth/v1/app/registration";
const LARK_ACCOUNTS_URL = "https://accounts.larksuite.com/oauth/v1/app/registration";
const FEISHU_MAX_SESSION_AGE_MS = 10 * 60_000;

const pendingFeishuSessions = new Map<string, PendingFeishuAuthSession>();

function getPackageVersion(): string {
  const raw = process.env.npm_package_version || process.env.GARYX_DESKTOP_VERSION;
  return raw && raw.trim() ? raw.trim() : "0.0.0-desktop";
}

function accountsEndpointForBrand(brand: "feishu" | "lark"): string {
  return brand === "lark" ? LARK_ACCOUNTS_URL : FEISHU_ACCOUNTS_URL;
}

function buildFeishuVerificationUrl(
  brand: "feishu" | "lark",
  userCode: string,
  version: string,
): string {
  const host = brand === "lark"
    ? "https://open.larksuite.com"
    : "https://open.feishu.cn";
  const params = new URLSearchParams({
    user_code: userCode,
    lpv: version,
    ocv: version,
    from: "cli",
  });
  return `${host}/page/cli?${params.toString()}`;
}

type BeginPollResponse = {
  device_code?: string;
  user_code?: string;
  expires_in?: number;
  interval?: number;
  client_id?: string;
  client_secret?: string;
  user_info?: { tenant_brand?: string | null } | null;
  error?: string;
  error_description?: string;
};

async function postForm(url: string, body: Record<string, string>): Promise<BeginPollResponse> {
  const form = new URLSearchParams(body).toString();
  const response = await fetch(url, {
    method: "POST",
    headers: {
      "Content-Type": "application/x-www-form-urlencoded",
    },
    body: form,
    signal: AbortSignal.timeout(15_000),
  });
  const text = await response.text();
  try {
    return JSON.parse(text) as BeginPollResponse;
  } catch {
    throw new Error(
      `Feishu accounts server returned non-JSON response (HTTP ${response.status}): ${text}`,
    );
  }
}

function prunePendingFeishuSessions(now: number): void {
  for (const [id, session] of pendingFeishuSessions.entries()) {
    if (session.expiresAt + FEISHU_MAX_SESSION_AGE_MS < now) {
      pendingFeishuSessions.delete(id);
    }
  }
}

export async function startFeishuChannelAuth(
  input: StartFeishuChannelAuthInput,
): Promise<StartFeishuChannelAuthResult> {
  prunePendingFeishuSessions(Date.now());
  const userFacingBrand: "feishu" | "lark" =
    input.domain === "lark" ? "lark" : "feishu";
  // The `begin` request always goes to accounts.feishu.cn (this matches
  // lark-cli's behavior — the Feishu accounts server brokers the PersonalAgent
  // creation flow for both brands). Only the post-confirmation poll may
  // need to switch to larksuite (handled in pollFeishuChannelAuth).
  const begin = await postForm(FEISHU_ACCOUNTS_URL, {
    action: "begin",
    archetype: FEISHU_ARCHETYPE,
    auth_method: "client_secret",
    request_user_info: "open_id tenant_brand",
  });
  if (begin.error) {
    throw new Error(
      `Feishu device-flow begin failed: ${begin.error_description || begin.error}`,
    );
  }
  const deviceCode = (begin.device_code || "").trim();
  const userCode = (begin.user_code || "").trim();
  if (!deviceCode || !userCode) {
    throw new Error("Feishu device-flow begin response missing device_code or user_code.");
  }
  const verificationUrl = buildFeishuVerificationUrl(
    userFacingBrand,
    userCode,
    getPackageVersion(),
  );
  const qrCodeDataUrl = await QRCode.toDataURL(verificationUrl, {
    margin: 1,
    width: 320,
  });

  const sessionId = randomUUID();
  const expiresIn = typeof begin.expires_in === "number" && begin.expires_in > 0 ? begin.expires_in : 300;
  const interval = typeof begin.interval === "number" && begin.interval > 0 ? begin.interval : 5;
  pendingFeishuSessions.set(sessionId, {
    accountId: normalizeOptionalText(input.accountId),
    name: normalizeOptionalText(input.name),
    workspaceDir: normalizeOptionalText(input.workspaceDir),
    deviceCode,
    userCode,
    verificationUrl,
    expiresAt: Date.now() + expiresIn * 1000,
    interval,
    pollDomain: userFacingBrand,
    lastDomainAttempted: userFacingBrand,
  });

  return {
    sessionId,
    verificationUrl,
    qrCodeDataUrl,
    userCode,
    expiresIn,
    interval,
    domain: userFacingBrand,
  };
}

export async function pollFeishuChannelAuth(
  settings: DesktopSettings,
  input: PollFeishuChannelAuthInput,
): Promise<PollFeishuChannelAuthResult> {
  const session = pendingFeishuSessions.get(input.sessionId);
  if (!session) {
    throw new Error("Feishu auth session not found.");
  }
  if (Date.now() > session.expiresAt) {
    pendingFeishuSessions.delete(input.sessionId);
    return { status: "expired" };
  }

  const endpoint = accountsEndpointForBrand(session.pollDomain);
  const response = await postForm(endpoint, {
    action: "poll",
    device_code: session.deviceCode,
  });

  // Success: client_id populated, no error.
  if (!response.error && response.client_id && response.client_id.trim()) {
    const tenantBrand: "feishu" | "lark" =
      (response.user_info?.tenant_brand || "").toLowerCase() === "lark"
        ? "lark"
        : "feishu";
    const appId = response.client_id.trim();
    const appSecret = (response.client_secret || "").trim();

    // Lark-tenant-on-feishu-endpoint quirk: the open platform routes
    // PersonalAgent creation through accounts.feishu.cn for every brand,
    // but if the tenant is a Lark one the client_secret only shows up on
    // accounts.larksuite.com. Detect + switch once, then keep polling.
    if (
      tenantBrand === "lark"
      && !appSecret
      && session.pollDomain !== "lark"
      && session.lastDomainAttempted !== "lark"
    ) {
      session.pollDomain = "lark";
      session.lastDomainAttempted = "lark";
      return { status: "pending" };
    }

    if (!appSecret) {
      throw new Error(
        "Feishu accounts server returned an empty app_secret; "
          + "try the manual credential flow or contact Feishu support.",
      );
    }

    const finalAccountId = session.accountId?.trim() || appId;
    await addChannelAccount(settings, {
      channel: "feishu",
      accountId: finalAccountId,
      name: session.name,
      workspaceDir: session.workspaceDir,
      appId,
      appSecret,
      domain: tenantBrand,
    });
    pendingFeishuSessions.delete(input.sessionId);
    return {
      status: "confirmed",
      accountId: finalAccountId,
      appId,
      domain: tenantBrand,
    };
  }

  switch ((response.error || "").toLowerCase()) {
    case "authorization_pending":
    case "":
      return { status: "pending" };
    case "slow_down":
      return { status: "slow_down" };
    case "access_denied":
      pendingFeishuSessions.delete(input.sessionId);
      return { status: "denied" };
    case "expired_token":
    case "invalid_grant":
      pendingFeishuSessions.delete(input.sessionId);
      return { status: "expired" };
    default:
      throw new Error(
        `Feishu accounts server rejected poll: ${response.error_description || response.error}`,
      );
  }
}

// Visible for tests — see `channel-setup.test.ts`.
export const __test__ = {
  buildFeishuVerificationUrl,
  accountsEndpointForBrand,
  pendingFeishuSessions,
};
