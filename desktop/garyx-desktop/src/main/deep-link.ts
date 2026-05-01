import type {
  DesktopDeepLinkEvent,
  DesktopSessionProviderHint,
} from "@shared/contracts";

export const GARYX_PROTOCOL = "garyx";
const DEEP_LINK_USAGE =
  "Use garyx://thread/<thread-id>, garyx://new?workspace=<path>, garyx://resume/<session-id>, or garyx://resume/<provider>/<session-id>.";

function decodeLoose(value: string | null | undefined): string | null {
  if (!value) {
    return null;
  }
  let current = value.trim();
  if (!current) {
    return null;
  }
  for (let attempt = 0; attempt < 2; attempt += 1) {
    try {
      const decoded = decodeURIComponent(current);
      if (decoded === current) {
        break;
      }
      current = decoded.trim();
    } catch {
      break;
    }
  }
  return current || null;
}

function normalizeProviderHint(
  value: string | null | undefined,
): DesktopSessionProviderHint | null {
  const normalized = value?.trim().toLowerCase() || "";
  switch (normalized) {
    case "claude":
    case "codex":
    case "gemini":
      return normalized;
    default:
      return null;
  }
}

function pathnameSegments(url: URL): string[] {
  return url.pathname
    .split("/")
    .map((segment) => decodeLoose(segment))
    .filter((segment): segment is string => Boolean(segment));
}

function deepLinkError(url: string, error: string): DesktopDeepLinkEvent {
  return {
    type: "error",
    url,
    error,
  };
}

export function extractProtocolUrls(args: string[]): string[] {
  return args
    .map((value) => value.trim())
    .filter((value) => value.toLowerCase().startsWith(`${GARYX_PROTOCOL}:`));
}

export function parseDesktopDeepLink(rawUrl: string): DesktopDeepLinkEvent {
  const normalizedUrl = rawUrl.trim();
  let url: URL;
  try {
    url = new URL(normalizedUrl);
  } catch {
    return deepLinkError(normalizedUrl, "Invalid garyx:// URL.");
  }

  if (url.protocol !== `${GARYX_PROTOCOL}:`) {
    return deepLinkError(
      normalizedUrl,
      "Unsupported URL scheme. Use garyx://.",
    );
  }

  if (url.hash) {
    return deepLinkError(
      normalizedUrl,
      `Unsupported garyx:// format. ${DEEP_LINK_USAGE}`,
    );
  }

  const action = decodeLoose(url.hostname)?.toLowerCase() || "";
  const segments = pathnameSegments(url);
  const firstSegment = segments[0] || null;
  const secondSegment = segments[1] || null;

  if (action === "thread") {
    if (url.search) {
      return deepLinkError(
        normalizedUrl,
        `Unsupported garyx:// format. ${DEEP_LINK_USAGE}`,
      );
    }
    if (!firstSegment) {
      return deepLinkError(
        normalizedUrl,
        "Missing thread id. Use garyx://thread/<thread-id>.",
      );
    }
    if (segments.length > 1) {
      return deepLinkError(
        normalizedUrl,
        `Unsupported garyx:// format. ${DEEP_LINK_USAGE}`,
      );
    }
    return {
      type: "open-thread",
      url: normalizedUrl,
      threadId: firstSegment,
    };
  }

  if (action === "new") {
    if (segments.length > 1) {
      return deepLinkError(
        normalizedUrl,
        `Unsupported garyx:// format. ${DEEP_LINK_USAGE}`,
      );
    }
    return {
      type: "new-thread",
      url: normalizedUrl,
      workspacePath:
        decodeLoose(url.searchParams.get("workspace")) ||
        decodeLoose(url.searchParams.get("workspacePath")) ||
        firstSegment,
      agentId: decodeLoose(url.searchParams.get("agent")),
    };
  }

  if (action === "resume") {
    if (url.search) {
      return deepLinkError(
        normalizedUrl,
        `Unsupported garyx:// format. ${DEEP_LINK_USAGE}`,
      );
    }
    if (!firstSegment) {
      return deepLinkError(
        normalizedUrl,
        "Missing session id. Use garyx://resume/<session-id>.",
      );
    }
    if (segments.length > 2) {
      return deepLinkError(
        normalizedUrl,
        `Unsupported garyx:// format. ${DEEP_LINK_USAGE}`,
      );
    }
    if (segments.length === 2) {
      const providerHint = normalizeProviderHint(firstSegment);
      if (!providerHint) {
        return deepLinkError(
          normalizedUrl,
          "Unsupported resume provider. Use claude, codex, or gemini.",
        );
      }
      if (!secondSegment) {
        return deepLinkError(
          normalizedUrl,
          "Missing session id. Use garyx://resume/<provider>/<session-id>.",
        );
      }
      return {
        type: "resume-session",
        url: normalizedUrl,
        sessionId: secondSegment,
        providerHint,
      };
    }
    return {
      type: "resume-session",
      url: normalizedUrl,
      sessionId: firstSegment,
    };
  }

  return deepLinkError(
    normalizedUrl,
    `Unsupported garyx:// target. ${DEEP_LINK_USAGE}`,
  );
}
