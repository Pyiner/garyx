import type { DesktopRemoteStateError } from "@shared/contracts";

export function isTransientGatewayErrorMessage(
  message: string | null | undefined,
): boolean {
  const normalized = message?.trim().toLowerCase() || "";
  if (!normalized) {
    return false;
  }
  return [
    "unable to reach gary gateway",
    "failed to fetch",
    "fetch failed",
    "networkerror",
    "network request failed",
    "network request timed out",
    "network timeout",
    "timeout",
    "timed out",
    "aborterror",
    "timeouterror",
    "operation was aborted",
    "connection refused",
    "connection reset",
    "socket hang up",
    "econnrefused",
    "econnreset",
    "enotfound",
    "ehostunreach",
  ].some((needle) => normalized.includes(needle));
}

export function summarizeRemoteStateErrors(
  errors: DesktopRemoteStateError[] | null | undefined,
): { key: string; message: string } | null {
  const activeErrors = (errors || []).filter((entry) => {
    const message = entry.message.trim();
    return message && !isTransientGatewayErrorMessage(message);
  });
  if (!activeErrors.length) {
    return null;
  }
  const labels = activeErrors.map((entry) => entry.label);
  const firstMessage = activeErrors[0]?.message.trim() || "unknown error";
  const detail =
    firstMessage.length > 96 ? `${firstMessage.slice(0, 93)}...` : firstMessage;
  return {
    key: activeErrors
      .map((entry) => `${entry.source}:${entry.message}`)
      .join("|"),
    message: `Gateway sync incomplete: ${labels.join(", ")} failed. ${detail}`,
  };
}
