import type {
  DesktopDirectoryListingErrorCode,
  DesktopLocalDirectoryEntry,
  DesktopLocalDirectoryListing,
  DesktopWorkspace,
  DesktopWorkspaceCatalog,
} from "./contracts/workspace.ts";

/**
 * Platform-neutral parsing of gateway workspace payloads. Shared by the
 * Electron main-process client and the Web entry so both adapters present
 * the identical catalog: gateway names and order are authoritative and are
 * preserved verbatim — no client re-sorting, no basename rewriting.
 */

export class WorkspacePayloadError extends Error {}

function requireRecord(value: unknown, context: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new WorkspacePayloadError(`${context} must be an object`);
  }
  return value as Record<string, unknown>;
}

function requireNonEmptyString(value: unknown, context: string): string {
  if (typeof value !== "string" || !value.trim()) {
    throw new WorkspacePayloadError(`${context} must be a non-empty string`);
  }
  return value;
}

function requireBoolean(value: unknown, context: string): boolean {
  if (typeof value !== "boolean") {
    throw new WorkspacePayloadError(`${context} must be a boolean`);
  }
  return value;
}

function requireFiniteNumber(value: unknown, context: string): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new WorkspacePayloadError(`${context} must be a number`);
  }
  return value;
}

function nullableNonEmptyString(value: unknown, context: string): string | null {
  return value === null || value === undefined
    ? null
    : requireNonEmptyString(value, context);
}

export function parseWorkspacePayload(value: unknown, index: number): DesktopWorkspace {
  const context = `workspace list.workspaces[${index}]`;
  const record = requireRecord(value, context);
  const now = new Date().toISOString();
  return {
    name: requireNonEmptyString(record.name, `${context}.name`),
    path: requireNonEmptyString(record.path, `${context}.path`),
    kind: "local",
    createdAt: now,
    updatedAt: now,
    available: true,
    pinned: requireBoolean(record.pinned, `${context}.pinned`),
    threadCount: requireFiniteNumber(record.thread_count, `${context}.thread_count`),
    lastActivityAt: nullableNonEmptyString(
      record.last_activity_at,
      `${context}.last_activity_at`,
    ),
    gitRepo: requireBoolean(record.git_repo, `${context}.git_repo`),
  };
}

export function parseWorkspaceCatalogPayload(payload: unknown): DesktopWorkspaceCatalog {
  const record = requireRecord(payload, "workspace list");
  const workspacesValue = record.workspaces;
  if (!Array.isArray(workspacesValue)) {
    throw new WorkspacePayloadError("workspace list.workspaces must be an array");
  }
  return {
    workspaces: workspacesValue.map(parseWorkspacePayload),
    gatewayHome: nullableNonEmptyString(
      record.gateway_home,
      "workspace list.gateway_home",
    ),
    workspaceStateInitialized: requireBoolean(
      record.workspace_state_initialized,
      "workspace list.workspace_state_initialized",
    ),
  };
}

export function parseDirectoryEntryPayload(
  value: unknown,
  index: number,
): DesktopLocalDirectoryEntry {
  const context = `workspace directory listing.entries[${index}]`;
  const record = requireRecord(value, context);
  return {
    name: requireNonEmptyString(record.name, `${context}.name`),
    path: requireNonEmptyString(record.path, `${context}.path`),
    gitRepo: requireBoolean(record.gitRepo, `${context}.gitRepo`),
  };
}

export function parseDirectoryListingPayload(
  payload: unknown,
): DesktopLocalDirectoryListing {
  const record = requireRecord(payload, "workspace directory listing");
  const entriesValue = record.entries;
  if (!Array.isArray(entriesValue)) {
    throw new WorkspacePayloadError(
      "workspace directory listing.entries must be an array",
    );
  }
  return {
    path: requireNonEmptyString(record.path, "workspace directory listing.path"),
    parentPath: nullableNonEmptyString(
      record.parentPath,
      "workspace directory listing.parentPath",
    ),
    entries: entriesValue.map(parseDirectoryEntryPayload),
  };
}

/**
 * Typed directory-listing errors must cross the Electron IPC boundary,
 * which flattens rejections to a message string. Encode/decode keeps the
 * gateway's typed 400 code attached so the browser can render the error
 * inline and stay on its current directory.
 */
const DIRECTORY_LISTING_ERROR_PREFIX = "garyx-directory-listing-error:";

const DIRECTORY_LISTING_ERROR_CODES = new Set([
  "invalid_path",
  "not_found",
  "not_a_directory",
  "permission_denied",
]);

export function encodeDirectoryListingError(
  code: string,
  message: string,
): string {
  return `${DIRECTORY_LISTING_ERROR_PREFIX}${code}:${message}`;
}

export function decodeDirectoryListingError(
  message: string | null | undefined,
): { code: DesktopDirectoryListingErrorCode; message: string } | null {
  if (!message || !message.includes(DIRECTORY_LISTING_ERROR_PREFIX)) {
    return null;
  }
  const start = message.indexOf(DIRECTORY_LISTING_ERROR_PREFIX);
  const encoded = message.slice(start + DIRECTORY_LISTING_ERROR_PREFIX.length);
  const separator = encoded.indexOf(":");
  if (separator <= 0) {
    return null;
  }
  const code = encoded.slice(0, separator);
  if (!DIRECTORY_LISTING_ERROR_CODES.has(code)) {
    return null;
  }
  return {
    code: code as DesktopDirectoryListingErrorCode,
    message: encoded.slice(separator + 1),
  };
}
