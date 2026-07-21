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

export type GatewayRequestSemantics =
  | "readRetryable"
  | "mutationSingleAttempt";

export interface GatewayTaggedApiError {
  kind: "garyx_api_error";
  operation: string;
  code: string;
  message?: string;
  [key: string]: unknown;
}

export type GatewayMutationResult<T> =
  | { kind: "ok"; value: T; status: number }
  | {
      kind: "definitiveEndpointResponse";
      status: number;
      error: GatewayTaggedApiError;
      value: T | null;
      body: string;
    }
  | {
      kind: "ambiguous";
      message: string;
      status?: number;
      body?: string;
    }
  | { kind: "notSent"; message: string };

let gatewayFetchImpl: GatewayFetch | null = null;
let gatewayStreamFetchImpl: GatewayFetch | null = null;

export function setGatewayFetch(fetchImpl: GatewayFetch | null): void {
  gatewayFetchImpl = fetchImpl;
}

export function gatewayFetch(input: string, init?: RequestInit): Promise<Response> {
  if (gatewayFetchImpl) {
    return gatewayFetchImpl(input, init);
  }
  return globalThis.fetch(input, init);
}

/**
 * Transport for long-lived SSE streams, kept on its own socket pool.
 *
 * Chromium's HTTP/1.1 pool allows 6 concurrent connections per host, shared
 * by every request on the same session. Live per-thread streams occupy their
 * connection for as long as they run, so enough of them starve every
 * control-plane request into its AbortSignal timeout (#TASK-1840). Streams
 * therefore go through a dedicated session injected here; without an
 * injection they share {@link gatewayFetch} (unit tests, non-Electron).
 */
export function setGatewayStreamFetch(fetchImpl: GatewayFetch | null): void {
  gatewayStreamFetchImpl = fetchImpl;
}

export function gatewayStreamFetch(
  input: string,
  semantics: "readRetryable",
  init?: RequestInit,
): Promise<Response> {
  const method = (init?.method || "GET").toUpperCase();
  if (semantics !== "readRetryable" || (method !== "GET" && method !== "HEAD")) {
    throw new TypeError("Gateway streams must use readRetryable GET semantics.");
  }
  if (gatewayStreamFetchImpl) {
    return gatewayStreamFetchImpl(input, init);
  }
  return gatewayFetch(input, init);
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
  if (
    maybeRecord.error &&
    typeof maybeRecord.error === "object" &&
    !Array.isArray(maybeRecord.error)
  ) {
    const nestedMessage = (maybeRecord.error as Record<string, unknown>).message;
    if (typeof nestedMessage === "string" && nestedMessage.trim()) {
      return nestedMessage;
    }
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

export class GatewayContractError extends Error {
  constructor(path: string, expectation: string) {
    super(`Gateway contract violation: ${path} ${expectation}`);
    this.name = "GatewayContractError";
  }
}

export function hasContractField(
  record: Record<string, unknown>,
  field: string,
): boolean {
  return Object.prototype.hasOwnProperty.call(record, field);
}

export function requireContractField(
  record: Record<string, unknown>,
  field: string,
  context: string,
): unknown {
  if (!hasContractField(record, field)) {
    throw new GatewayContractError(`${context}.${field}`, "is required");
  }
  return record[field];
}

export function requireContractRecord(
  value: unknown,
  path: string,
): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new GatewayContractError(path, "must be an object");
  }
  return value as Record<string, unknown>;
}

export function requireContractArray(
  value: unknown,
  path: string,
): unknown[] {
  if (!Array.isArray(value)) {
    throw new GatewayContractError(path, "must be an array");
  }
  return value;
}

export function requireContractString(
  value: unknown,
  path: string,
): string {
  if (typeof value !== "string") {
    throw new GatewayContractError(path, "must be a string");
  }
  return value;
}

export function requireContractNonEmptyString(
  value: unknown,
  path: string,
): string {
  const result = requireContractString(value, path).trim();
  if (!result) {
    throw new GatewayContractError(path, "must be a non-empty string");
  }
  return result;
}

export function requireContractBoolean(
  value: unknown,
  path: string,
): boolean {
  if (typeof value !== "boolean") {
    throw new GatewayContractError(path, "must be a boolean");
  }
  return value;
}

export function requireContractFiniteNumber(
  value: unknown,
  path: string,
): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new GatewayContractError(path, "must be a finite number");
  }
  return value;
}

export function requireContractNonNegativeInteger(
  value: unknown,
  path: string,
): number {
  if (!Number.isSafeInteger(value) || (value as number) < 0) {
    throw new GatewayContractError(path, "must be a non-negative integer");
  }
  return value as number;
}

export function requireContractInteger(
  value: unknown,
  path: string,
): number {
  if (!Number.isSafeInteger(value)) {
    throw new GatewayContractError(path, "must be an integer");
  }
  return value as number;
}

function prepareRequest(
  settings: DesktopSettings,
  path: string,
  semantics: GatewayRequestSemantics,
  init: RequestInit,
  accept: string,
): { url: string; init: RequestInit } {
  const method = (init.method || "GET").toUpperCase();
  const isRead = method === "GET" || method === "HEAD";
  if (isRead !== (semantics === "readRetryable")) {
    throw new TypeError(
      `${method} requests must use ${isRead ? "readRetryable" : "mutationSingleAttempt"} semantics.`,
    );
  }

  const url = buildUrl(settings, path);
  // Validate before creating the fetch task so malformed configuration is a
  // provable `notSent` outcome for classified mutations.
  new URL(url);
  if (init.signal?.aborted) {
    throw new DOMException("The request was aborted before dispatch.", "AbortError");
  }
  const headers = applyGatewayAuthHeader(
    applyGatewayCustomHeaders(new Headers(init.headers), settings.gatewayHeaders),
    settings.gatewayAuthToken,
  );
  headers.set("Accept", accept);
  if (init.body && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }
  return {
    url,
    init: {
      ...init,
      headers,
    },
  };
}

function messageFromUnknown(error: unknown): string {
  return error instanceof Error && error.message
    ? error.message
    : String(error || "Gateway request failed.");
}

function taggedErrorFromPayload(
  payload: unknown,
  expectedOperation: string,
  status: number,
): GatewayTaggedApiError | null {
  if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
    return null;
  }
  const record = payload as Record<string, unknown>;
  if (
    record.kind !== "garyx_api_error" ||
    typeof record.operation !== "string" ||
    typeof record.code !== "string"
  ) {
    return null;
  }
  const endpointMatch = record.operation === expectedOperation;
  const gatewayAuthMatch =
    record.operation === "gateway_auth" &&
    (status === 401 || status === 403) &&
    (record.code === "unauthorized" || record.code === "forbidden");
  return endpointMatch || gatewayAuthMatch
    ? (record as GatewayTaggedApiError)
    : null;
}

export async function requestJson<T>(
  settings: DesktopSettings,
  path: string,
  semantics: GatewayRequestSemantics,
  init: RequestInit = {},
): Promise<T> {
  const request = prepareRequest(
    settings,
    path,
    semantics,
    init,
    "application/json",
  );
  const response = await gatewayFetch(request.url, request.init);
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

/**
 * Executes one mutation attempt and preserves transport uncertainty for the
 * reducer that owns retry/verification. A failure is definitive only when the
 * gateway's tagged error matches the endpoint operation (or the narrow
 * gateway-auth 401/403 acceptance set).
 */
export async function requestMutationJson<T>(
  settings: DesktopSettings,
  path: string,
  semantics: "mutationSingleAttempt",
  expectedOperation: string,
  init: RequestInit,
  decodePayload: (payload: unknown) => T,
): Promise<GatewayMutationResult<T>> {
  let request: { url: string; init: RequestInit };
  try {
    request = prepareRequest(
      settings,
      path,
      semantics,
      init,
      "application/json",
    );
  } catch (error) {
    return { kind: "notSent", message: messageFromUnknown(error) };
  }

  let responsePromise: Promise<Response>;
  try {
    responsePromise = gatewayFetch(request.url, request.init);
  } catch (error) {
    // Invoking the transport crosses the dispatch boundary. A custom fetch
    // implementation could have created the request before throwing, so this
    // is not provably `notSent`.
    return { kind: "ambiguous", message: messageFromUnknown(error) };
  }

  let response: Response;
  try {
    response = await responsePromise;
  } catch (error) {
    return { kind: "ambiguous", message: messageFromUnknown(error) };
  }

  let body: string;
  try {
    body = await response.text();
  } catch (error) {
    return {
      kind: "ambiguous",
      message: messageFromUnknown(error),
      status: response.status,
    };
  }
  const payload = tryParseJson<T>(body);
  if (response.ok) {
    if (payload === null) {
      return {
        kind: "ambiguous",
        message:
          messageFromPlainTextBody(body) || "Gateway returned invalid JSON.",
        status: response.status,
        body,
      };
    }
    try {
      return {
        kind: "ok",
        value: decodePayload(payload),
        status: response.status,
      };
    } catch (error) {
      return {
        kind: "ambiguous",
        message: messageFromUnknown(error),
        status: response.status,
        body,
      };
    }
  }

  const tagged = taggedErrorFromPayload(
    payload,
    expectedOperation,
    response.status,
  );
  if (tagged) {
    let decoded: T | null = null;
    try {
      decoded = decodePayload(payload);
    } catch {
      // Tagged endpoint/auth errors remain definitive even when their payload
      // intentionally omits a success-page body (for example auth failures).
    }
    return {
      kind: "definitiveEndpointResponse",
      status: response.status,
      error: tagged,
      value: decoded,
      body,
    };
  }
  return {
    kind: "ambiguous",
    message:
      errorMessageFromPayload(payload) ||
      messageFromPlainTextBody(body) ||
      `${response.status} ${response.statusText}`,
    status: response.status,
    body,
  };
}

export async function requestText(
  settings: DesktopSettings,
  path: string,
  semantics: GatewayRequestSemantics,
  init: RequestInit = {},
): Promise<string> {
  const request = prepareRequest(
    settings,
    path,
    semantics,
    init,
    "text/html, text/plain;q=0.9, */*;q=0.1",
  );
  const response = await gatewayFetch(request.url, request.init);
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
  semantics: GatewayRequestSemantics,
  init: RequestInit = {},
): Promise<T> {
  const method = (init.method || "GET").toUpperCase();
  const isRead = method === "GET" || method === "HEAD";
  if (isRead !== (semantics === "readRetryable")) {
    throw new TypeError(
      `${method} requests must use ${isRead ? "readRetryable" : "mutationSingleAttempt"} semantics.`,
    );
  }
  const headers = applyGatewayAuthHeader(
    applyGatewayCustomHeaders(new Headers(init.headers), gatewayHeaders),
    gatewayAuthToken,
  );
  headers.set("Accept", "application/json");
  if (init.body && !headers.has("Content-Type")) {
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
