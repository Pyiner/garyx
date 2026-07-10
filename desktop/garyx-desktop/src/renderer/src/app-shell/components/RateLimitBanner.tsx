import {
  Fragment,
  useEffect,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";

import type { RenderRateLimit } from "@shared/contracts";

import { useI18n } from "../../i18n";
import {
  deriveRateLimitBannerState,
  formatRemaining,
  formatResetClock,
  messageSegments,
} from "../rate-limit-banner-model";

/**
 * Quota / rate-limit card rendered at the tail of a thread when its most
 * recent run terminated because the provider's rolling usage quota was
 * exhausted. The countdown ticks locally off the server-provided `resetAt`, so
 * no streaming updates are required.
 *
 * State derivation lives in `rate-limit-banner-model.ts`; this component maps
 * it onto JSX. When no automatic resend is scheduled, a Continue button
 * dispatches a literal "continue" prompt through the regular send pipeline via
 * `onContinue`; the card disappears once the new run starts and the server
 * clears the rate-limit state.
 */
export function RateLimitBanner({
  rateLimit,
  onContinue,
}: {
  rateLimit?: RenderRateLimit | null;
  onContinue?: () => void;
}) {
  const { t, locale } = useI18n();
  const [now, setNow] = useState(() => Date.now());
  const [sending, setSending] = useState(false);
  const [hovered, setHovered] = useState(false);

  const resetMs = rateLimit?.resetAt ? Date.parse(rateLimit.resetAt) : Number.NaN;
  const hasReset = Number.isFinite(resetMs);

  useEffect(() => {
    if (!rateLimit || !hasReset) {
      return;
    }
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [rateLimit, hasReset]);

  // A fresh rate-limit context re-arms the Continue action.
  useEffect(() => {
    setSending(false);
  }, [rateLimit?.resetAt, rateLimit?.provider, rateLimit?.message]);

  if (!rateLimit) {
    return null;
  }

  const provider = providerLabel(rateLimit.provider);
  const windowText = windowLabel(rateLimit.window, t);
  const message = rateLimit.message?.trim() ?? "";
  const state = deriveRateLimitBannerState({
    resetAtMs: resetMs,
    nowMs: now,
    willAutoResend: rateLimit.willAutoResend,
    hasMessage: message.length > 0,
  });

  const title = windowText
    ? t("{provider} {window} reached")
        .replace("{provider}", provider)
        .replace("{window}", windowText)
    : t("{provider} usage limit reached").replace("{provider}", provider);

  const clock = hasReset ? formatResetClock(resetMs, now, locale) : "";
  const remaining = hasReset ? formatRemaining(resetMs - now) : "";

  let detail: ReactNode;
  switch (state.kind) {
    case "auto_resend_countdown":
      detail = renderTemplate(t("Auto-resend at {time} · {remaining} left"), {
        time: <strong style={strongStyle}>{clock}</strong>,
        remaining: <strong style={strongStyle}>{remaining}</strong>,
      });
      break;
    case "resending":
      detail = t("Quota recovered — resending…");
      break;
    case "auto_resend_pending":
      detail = t("Will auto-resend when the quota recovers.");
      break;
    case "resets_countdown":
      detail = renderTemplate(t("Resets at {time} · {remaining} left"), {
        time: <strong style={strongStyle}>{clock}</strong>,
        remaining: <strong style={strongStyle}>{remaining}</strong>,
      });
      break;
    case "recovered":
      detail = renderTemplate(
        t("Reset at {time} — quota should be available again."),
        { time: <strong style={strongStyle}>{clock}</strong> },
      );
      break;
    case "message":
      detail = messageSegments(message).map((segment, index) =>
        segment.kind === "link" ? (
          <a
            href={segment.url}
            key={index}
            rel="noreferrer"
            style={linkStyle}
            target="_blank"
          >
            {segment.text}
          </a>
        ) : (
          <Fragment key={index}>{segment.text}</Fragment>
        ),
      );
      break;
    default:
      detail = t("Try again shortly.");
      break;
  }

  const showContinue = state.showContinue && Boolean(onContinue);
  const handleContinue = () => {
    if (sending || !onContinue) {
      return;
    }
    setSending(true);
    onContinue();
  };

  return (
    <article
      aria-live="polite"
      className="rate-limit-banner"
      role="status"
      style={bannerStyle}
    >
      <span aria-hidden="true" style={chipStyle}>
        {rateLimit.willAutoResend ? autoResendIcon : hourglassIcon}
      </span>
      <span style={textColumnStyle}>
        <span style={titleStyle}>{title}</span>
        <span style={detailStyle}>{detail}</span>
      </span>
      {showContinue ? (
        <button
          disabled={sending}
          onClick={handleContinue}
          onMouseEnter={() => setHovered(true)}
          onMouseLeave={() => setHovered(false)}
          style={{
            ...continueButtonStyle,
            ...(hovered && !sending ? continueButtonHoverStyle : null),
            ...(sending ? continueButtonSendingStyle : null),
          }}
          type="button"
        >
          {sending ? t("Sending…") : t("Continue")}
        </button>
      ) : null}
    </article>
  );
}

function providerLabel(provider?: string | null): string {
  const slug = (provider ?? "").trim().toLowerCase();
  if (slug.startsWith("codex")) {
    return "Codex";
  }
  if (slug.startsWith("trae")) {
    return "TRAE";
  }
  return provider && provider.trim() ? provider.trim() : "Provider";
}

function windowLabel(
  window: string | null | undefined,
  t: (key: string) => string,
): string | null {
  switch (window) {
    case "primary":
      return t("5-hour limit");
    case "secondary":
      return t("weekly limit");
    default:
      return null;
  }
}

/** Interleave translated template text with rich placeholder nodes. */
function renderTemplate(
  template: string,
  slots: Record<string, ReactNode>,
): ReactNode {
  return template.split(/(\{[a-zA-Z]+\})/g).map((part, index) => {
    const match = /^\{([a-zA-Z]+)\}$/.exec(part);
    if (match && match[1] in slots) {
      return <Fragment key={index}>{slots[match[1]]}</Fragment>;
    }
    return <Fragment key={index}>{part}</Fragment>;
  });
}

const hourglassIcon = (
  <svg
    fill="none"
    height="15"
    stroke="currentColor"
    strokeLinecap="round"
    strokeLinejoin="round"
    strokeWidth="2"
    viewBox="0 0 24 24"
    width="15"
  >
    <path d="M5 22h14" />
    <path d="M5 2h14" />
    <path d="M17 22v-4.172a2 2 0 0 0-.586-1.414L12 12l-4.414 4.414A2 2 0 0 0 7 17.828V22" />
    <path d="M7 2v4.172a2 2 0 0 0 .586 1.414L12 12l4.414-4.414A2 2 0 0 0 17 6.172V2" />
  </svg>
);

const autoResendIcon = (
  <svg
    fill="none"
    height="15"
    stroke="currentColor"
    strokeLinecap="round"
    strokeLinejoin="round"
    strokeWidth="2"
    viewBox="0 0 24 24"
    width="15"
  >
    <path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8" />
    <path d="M21 3v5h-5" />
  </svg>
);

const bannerStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 12,
  margin: "8px 0",
  padding: "11px 12px 11px 13px",
  borderRadius: 12,
  border: "1px solid #e7e5e0",
  background: "#ffffff",
  boxShadow: "0 1px 2px rgba(26, 28, 31, 0.04)",
};

const chipStyle: CSSProperties = {
  flex: "0 0 auto",
  width: 30,
  height: 30,
  borderRadius: 8,
  background: "#f4f4f2",
  color: "#57534e",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
};

const textColumnStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 2,
  minWidth: 0,
  flex: "1 1 auto",
};

const titleStyle: CSSProperties = {
  fontSize: 13,
  fontWeight: 600,
  lineHeight: 1.35,
  color: "#1a1c1f",
  letterSpacing: "-0.08px",
};

const detailStyle: CSSProperties = {
  fontSize: 12,
  lineHeight: 1.45,
  color: "#78746b",
  fontVariantNumeric: "tabular-nums",
};

const strongStyle: CSSProperties = {
  color: "#1a1c1f",
  fontWeight: 600,
  fontVariantNumeric: "tabular-nums",
};

const linkStyle: CSSProperties = {
  color: "#57534e",
  textDecoration: "underline",
  textUnderlineOffset: 2,
};

const continueButtonStyle: CSSProperties = {
  flex: "0 0 auto",
  marginLeft: 4,
  height: 28,
  padding: "0 14px",
  borderRadius: 9,
  background: "#ffffff",
  border: "1px solid #e0ded8",
  color: "#1a1c1f",
  fontSize: 12,
  fontWeight: 600,
  fontFamily: "inherit",
  boxShadow: "0 1px 2px rgba(26, 28, 31, 0.05)",
  cursor: "pointer",
};

const continueButtonHoverStyle: CSSProperties = {
  background: "#f6f5f1",
};

const continueButtonSendingStyle: CSSProperties = {
  opacity: 0.55,
  cursor: "default",
};
