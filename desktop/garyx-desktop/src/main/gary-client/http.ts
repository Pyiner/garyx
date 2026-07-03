import type {
  DesktopSettings,
} from "@shared/contracts";
import { parseGatewayHeadersBlock } from "../../shared/gateway-headers.ts";

export const REMOTE_STATE_FETCH_TIMEOUT_MS = 30_000;

const LOCAL_GATEWAY_HOSTS = new Set([
  "127.0.0.1",
  "localhost",
  "0.0.0.0",
  "::1",
  "[::1]",
]);

export function normalizeGatewayUrl(gatewayUrl: string): string {
  return gatewayUrl.trim().replace(/\/+$/, "");
}

export function baseUrl(settings: DesktopSettings): string {
  return normalizeGatewayUrl(settings.gatewayUrl);
}

export function buildUrl(settings: DesktopSettings, path: string): string {
  return `${baseUrl(settings)}${path.startsWith("/") ? path : `/${path}`}`;
}

function buildUrlFromGatewayUrl(gatewayUrl: string, path: string): string {
  const normalized = normalizeGatewayUrl(gatewayUrl);
  return `${normalized}${path.startsWith("/") ? path : `/${path}`}`;
}

export function applyGatewayAuthHeader(
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

export function applyGatewayCustomHeaders(
  headers: Headers,
  gatewayHeaders: string | null | undefined,
): Headers {
  for (const [name, value] of Object.entries(parseGatewayHeadersBlock(gatewayHeaders))) {
    headers.set(name, value);
  }
  return headers;
}

export function isLocalGatewayUrl(gatewayUrl: string): boolean {
  try {
    const parsed = new URL(normalizeGatewayUrl(gatewayUrl));
    return LOCAL_GATEWAY_HOSTS.has(parsed.hostname);
  } catch {
    return false;
  }
}

export function tryParseJson<T>(body: string): T | null {
  if (!body.trim()) {
    return {} as T;
  }
  try {
    return JSON.parse(body) as T;
  } catch {
    return null;
  }
}

/**
 * Gateway HTTP transport.
 *
 * Defaults to the global `fetch` so this module stays free of `electron`
 * imports and unit tests can keep stubbing `globalThis.fetch`. The Electron main
 * entry injects `net.fetch` via {@link setGatewayFetch} at startup so gateway
 * requests go through Chromium's network stack and honor the macOS system proxy
 * (e.g. Surge). Node's global `fetch` (undici) ignores the system proxy and does
 * local DNS, so a remote gateway whose hostname resolves to a private/off-LAN
 * address (split-horizon) is unreachable directly; routing through the system
 * proxy lets it tunnel, while localhost gateways stay on a direct connection
 * (Chromium bypasses the proxy for loopback).
 */
export type GatewayFetch = (
  input: string,
  init?: RequestInit,
) => Promise<Response>;

let gatewayFetchImpl: GatewayFetch | null = null;

export function setGatewayFetch(fetchImpl: GatewayFetch | null): void {
  gatewayFetchImpl = fetchImpl;
}

export function gatewayFetch(input: string, init?: RequestInit): Promise<Response> {
  if (gatewayFetchImpl) {
    return gatewayFetchImpl(input, init);
  }
  return globalThis.fetch(input, init);
}

function messageFromPlainTextBody(body: string): string | undefined {
  const trimmed = body.trim().replace(/\s+/g, " ");
  if (!trimmed) {
    return undefined;
  }
  return trimmed.length > 500 ? `${trimmed.slice(0, 497)}...` : trimmed;
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

export class GatewayRequestError extends Error {
  status: number;
  statusText: string;
  body: string;

  constructor(status: number, statusText: string, message: string, body: string) {
    super(message);
    this.name = "GatewayRequestError";
    this.status = status;
    this.statusText = statusText;
    this.body = body;
  }
}

export async function requestJson<T>(
  settings: DesktopSettings,
  path: string,
  init?: RequestInit,
): Promise<T> {
  const headers = applyGatewayAuthHeader(
    applyGatewayCustomHeaders(new Headers(init?.headers), settings.gatewayHeaders),
    settings.gatewayAuthToken,
  );
  headers.set("Accept", "application/json");
  if (init?.body && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }

  const response = await gatewayFetch(buildUrl(settings, path), {
    ...init,
    headers,
  });
  const body = await response.text();
  const payload = tryParseJson<T>(body);

  if (!response.ok) {
    const message =
      errorMessageFromPayload(payload) ||
      messageFromPlainTextBody(body) ||
      `${response.status} ${response.statusText}`;
    throw new GatewayRequestError(response.status, response.statusText, message, body);
  }

  if (payload === null) {
    throw new Error(
      messageFromPlainTextBody(body) || "Gateway returned invalid JSON.",
    );
  }

  return payload;
}

export async function requestText(
  settings: DesktopSettings,
  path: string,
  init?: RequestInit,
): Promise<string> {
  const headers = applyGatewayAuthHeader(
    applyGatewayCustomHeaders(new Headers(init?.headers), settings.gatewayHeaders),
    settings.gatewayAuthToken,
  );
  headers.set("Accept", "text/html, text/plain;q=0.9, */*;q=0.1");
  if (init?.body && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }

  const response = await gatewayFetch(buildUrl(settings, path), {
    ...init,
    headers,
  });
  const body = await response.text();
  const payload = tryParseJson<unknown>(body);

  if (!response.ok) {
    const message =
      errorMessageFromPayload(payload) ||
      messageFromPlainTextBody(body) ||
      `${response.status} ${response.statusText}`;
    throw new GatewayRequestError(response.status, response.statusText, message, body);
  }

  return body;
}

export async function requestJsonFromGatewayUrl<T>(
  gatewayUrl: string,
  gatewayAuthToken: string,
  gatewayHeaders: string | null | undefined,
  path: string,
  init?: RequestInit,
): Promise<T> {
  const headers = applyGatewayAuthHeader(
    applyGatewayCustomHeaders(new Headers(init?.headers), gatewayHeaders),
    gatewayAuthToken,
  );
  headers.set("Accept", "application/json");
  if (init?.body && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }

  const response = await gatewayFetch(buildUrlFromGatewayUrl(gatewayUrl, path), {
    ...init,
    headers,
  });
  const body = await response.text();
  const payload = tryParseJson<T>(body);

  if (!response.ok) {
    throw new Error(
      errorMessageFromPayload(payload) ||
        messageFromPlainTextBody(body) ||
        `${response.status} ${response.statusText}`,
    );
  }

  if (payload === null) {
    throw new Error(
      messageFromPlainTextBody(body) || "Gateway returned invalid JSON.",
    );
  }

  return payload;
}

export function parseRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object"
    ? (value as Record<string, unknown>)
    : {};
}

export function asString(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() ? value : undefined;
}

export function asBoolean(value: unknown): boolean | undefined {
  return typeof value === "boolean" ? value : undefined;
}

export function asFiniteNumber(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}

export function asStringList(value: unknown): string[] {
  if (!Array.isArray(value)) {
    return [];
  }
  const seen = new Set<string>();
  const ids: string[] = [];
  for (const item of value) {
    const id = asString(item);
    if (!id || seen.has(id)) {
      continue;
    }
    seen.add(id);
    ids.push(id);
  }
  return ids;
}
