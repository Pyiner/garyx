/**
 * Pure presentation logic for the thread-tail rate-limit card, kept out of the
 * component so the state machine, countdown formatting, and provider-message
 * linkification are unit-testable (mirrors the iOS
 * `GaryxRateLimitBannerModel`). The component maps this onto JSX and i18n.
 */

export type RateLimitBannerStateKind =
  | "auto_resend_countdown"
  | "resending"
  | "auto_resend_pending"
  | "resets_countdown"
  | "recovered"
  | "message"
  | "plain";

export interface RateLimitBannerState {
  kind: RateLimitBannerStateKind;
  /** Manual Continue makes sense only when no automatic resend is scheduled. */
  showContinue: boolean;
}

/**
 * Derive which card variant to render.
 *
 * - Auto-resend scheduled: countdown → resending once recovered; no manual
 *   action (a manual send would just hit the limit again).
 * - No auto-resend: countdown with reset clock, "should be available again"
 *   once past the reset, provider message verbatim when no reset is known,
 *   generic fallback otherwise — all with a Continue action.
 */
export function deriveRateLimitBannerState(input: {
  resetAtMs: number;
  nowMs: number;
  willAutoResend: boolean;
  hasMessage: boolean;
}): RateLimitBannerState {
  const hasReset = Number.isFinite(input.resetAtMs);
  const recovered = hasReset && input.resetAtMs - input.nowMs <= 0;

  if (input.willAutoResend) {
    if (!hasReset) {
      return { kind: "auto_resend_pending", showContinue: false };
    }
    if (recovered) {
      return { kind: "resending", showContinue: false };
    }
    return { kind: "auto_resend_countdown", showContinue: false };
  }
  if (hasReset && !recovered) {
    return { kind: "resets_countdown", showContinue: true };
  }
  if (hasReset) {
    return { kind: "recovered", showContinue: true };
  }
  if (input.hasMessage) {
    return { kind: "message", showContinue: true };
  }
  return { kind: "plain", showContinue: true };
}

/** Local wall-clock reset time; includes the date once it is not today. */
export function formatResetClock(
  resetMs: number,
  nowMs: number,
  locale: string,
): string {
  const reset = new Date(resetMs);
  const nowDate = new Date(nowMs);
  const sameDay =
    reset.getFullYear() === nowDate.getFullYear() &&
    reset.getMonth() === nowDate.getMonth() &&
    reset.getDate() === nowDate.getDate();
  const time = reset.toLocaleTimeString(locale, {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
  if (sameDay) {
    return time;
  }
  const day = reset.toLocaleDateString(locale, {
    month: "short",
    day: "numeric",
  });
  return `${day} ${time}`;
}

export function formatRemaining(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  const pad = (value: number): string => String(value).padStart(2, "0");
  return hours > 0
    ? `${hours}:${pad(minutes)}:${pad(seconds)}`
    : `${pad(minutes)}:${pad(seconds)}`;
}

export type RateLimitMessageSegment =
  | { kind: "text"; text: string }
  | { kind: "link"; text: string; url: string };

/**
 * Split a provider message into text and bare-URL link segments. Trailing
 * sentence punctuation stays in the following text segment so links like
 * "…/usage." do not swallow the period.
 */
export function messageSegments(message: string): RateLimitMessageSegment[] {
  const segments: RateLimitMessageSegment[] = [];
  for (const part of message.split(/(https?:\/\/\S+)/g)) {
    if (!part) {
      continue;
    }
    if (!/^https?:\/\//.test(part)) {
      segments.push({ kind: "text", text: part });
      continue;
    }
    const trailing = /[.,;:)\]]+$/.exec(part)?.[0] ?? "";
    const url = trailing ? part.slice(0, -trailing.length) : part;
    segments.push({
      kind: "link",
      text: url.replace(/^https?:\/\//, ""),
      url,
    });
    if (trailing) {
      segments.push({ kind: "text", text: trailing });
    }
  }
  return segments;
}
