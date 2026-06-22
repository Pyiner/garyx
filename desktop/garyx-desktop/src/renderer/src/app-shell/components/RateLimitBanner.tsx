import { useEffect, useState, type CSSProperties } from "react";

import type { RenderRateLimit } from "@shared/contracts";

import { useI18n } from "../../i18n";

/**
 * Quota / rate-limit banner rendered at the tail of a thread when its most
 * recent run terminated because the provider's rolling usage quota was
 * exhausted. The countdown ticks locally off the server-provided `resetAt`, so
 * no streaming updates are required. When `willAutoResend` is set the gateway
 * has scheduled an automatic resend of the original message at reset time.
 */
export function RateLimitBanner({
  rateLimit,
}: {
  rateLimit?: RenderRateLimit | null;
}) {
  const { t } = useI18n();
  const [now, setNow] = useState(() => Date.now());

  const resetMs = rateLimit?.resetAt ? Date.parse(rateLimit.resetAt) : Number.NaN;
  const hasReset = Number.isFinite(resetMs);

  useEffect(() => {
    if (!rateLimit || !hasReset) {
      return;
    }
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [rateLimit, hasReset]);

  if (!rateLimit) {
    return null;
  }

  const provider = providerLabel(rateLimit.provider);
  const windowText = windowLabel(rateLimit.window, t);
  const remaining = hasReset ? resetMs - now : Number.NaN;
  const recovered = hasReset && remaining <= 0;

  const title = windowText
    ? t("{provider} {window} reached")
        .replace("{provider}", provider)
        .replace("{window}", windowText)
    : t("{provider} usage limit reached").replace("{provider}", provider);

  let detail: string;
  if (rateLimit.willAutoResend) {
    if (!hasReset) {
      detail = t("Will auto-resend when the quota recovers.");
    } else if (recovered) {
      detail = t("Quota recovered — resending…");
    } else {
      detail = t("Auto-resend in {time}").replace(
        "{time}",
        formatRemaining(remaining),
      );
    }
  } else if (hasReset && !recovered) {
    detail = t("Resets in {time}").replace("{time}", formatRemaining(remaining));
  } else {
    detail = t("Try again shortly.");
  }

  return (
    <article
      aria-live="polite"
      className="rate-limit-banner"
      role="status"
      style={bannerStyle}
    >
      <span style={dotStyle} aria-hidden="true" />
      <span style={textColumnStyle}>
        <span style={titleStyle}>{title}</span>
        <span style={detailStyle}>{detail}</span>
      </span>
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

function formatRemaining(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = total % 60;
  const pad = (value: number): string => String(value).padStart(2, "0");
  return hours > 0
    ? `${hours}:${pad(minutes)}:${pad(seconds)}`
    : `${pad(minutes)}:${pad(seconds)}`;
}

const bannerStyle: CSSProperties = {
  display: "flex",
  alignItems: "flex-start",
  gap: 10,
  margin: "8px 0",
  padding: "10px 14px",
  borderRadius: 10,
  border: "1px solid #f0e2c4",
  background: "#fdf6e9",
  color: "#7a5d1f",
};

const dotStyle: CSSProperties = {
  flex: "0 0 auto",
  width: 8,
  height: 8,
  marginTop: 5,
  borderRadius: "50%",
  background: "#d99a2b",
};

const textColumnStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 2,
  minWidth: 0,
};

const titleStyle: CSSProperties = {
  fontSize: 13,
  fontWeight: 600,
  lineHeight: 1.4,
};

const detailStyle: CSSProperties = {
  fontSize: 12.5,
  lineHeight: 1.4,
  fontVariantNumeric: "tabular-nums",
  opacity: 0.9,
};
