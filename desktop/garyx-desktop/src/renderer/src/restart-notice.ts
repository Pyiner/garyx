export interface ParsedRestartNotice {
  message: string;
}

const DEFAULT_RESTART_MESSAGE = "Garyx has restarted. Continue your task.";

function stripOuterRestartNotice(text: string): string | null {
  const open = /^\s*<garyx_restarted\b([^>]*)>\s*/.exec(text);
  if (!open) {
    return null;
  }
  const closeTag = "</garyx_restarted>";
  const closeIndex = text.lastIndexOf(closeTag);
  if (closeIndex < open[0].length) {
    return null;
  }
  return text.slice(open[0].length, closeIndex).trim();
}

/**
 * Parse a `<garyx_restarted>…</garyx_restarted>` restart-notice message into a
 * card model. Returns null when the text is not a restart notice, so callers can
 * fall back to rendering the raw text. Mirrors the former task-notification
 * parser's failure behavior.
 */
export function parseRestartNoticeText(
  text: string,
): ParsedRestartNotice | null {
  const body = stripOuterRestartNotice(text);
  if (body === null) {
    return null;
  }
  return {
    message: body || DEFAULT_RESTART_MESSAGE,
  };
}
