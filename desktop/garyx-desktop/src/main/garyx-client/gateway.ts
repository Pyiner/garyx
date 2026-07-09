import type {
  ConnectionStatus,
  DesktopSettings,
  GatewayConfigDocument,
  GatewayProbeResult,
  GatewaySettingsPayload,
  GatewaySettingsSaveRequestOptions,
  GatewaySettingsSaveResult,
  GatewaySettingsSource,
} from "@shared/contracts";
import { normalizeGatewayUrl, requestJson, requestJsonFromGatewayUrl } from "./http.ts";

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
  input: { gatewayUrl: string; gatewayAuthToken: string; gatewayHeaders?: string },
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
      input.gatewayHeaders,
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

export async function saveGatewaySettings(
  settings: DesktopSettings,
  config: GatewayConfigDocument,
  options?: GatewaySettingsSaveRequestOptions,
): Promise<GatewaySettingsSaveResult> {
  const normalizedConfig = stripNullObjectFields(
    stripLegacyGatewayConfigFields(config),
  );
  const merge = options?.merge === true;
  const result = await requestJson<{
    ok?: boolean;
    message?: string;
    errors?: string[];
  }>(settings, `/api/settings?merge=${merge ? "true" : "false"}`, {
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
