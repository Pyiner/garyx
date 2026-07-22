import {
  Fragment,
  useEffect,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";

import type {
  DesktopClaudeCodeAccount,
  DesktopClaudeCodeAccounts,
  RenderRateLimit,
} from "@shared/contracts";

import { useI18n } from "../../i18n";
import {
  deriveRateLimitBannerState,
  formatRemaining,
  formatResetClock,
  messageSegments,
  normalizeRateLimitProvider,
} from "../rate-limit-banner-model";
import { providerLabel as sharedProviderLabel } from "./agents-hub-helpers";
import { ClaudeAccountSwitcherDialog } from "./ClaudeAccountSwitcherDialog";
import { ProviderAgentIcon } from "./ProviderAgentIcon";

/**
 * Quota / rate-limit card rendered at the tail of a thread when its most
 * recent run terminated because the provider's rolling usage quota was
 * exhausted. The countdown ticks locally off the server-provided `resetAt`, so
 * no streaming updates are required.
 *
 * State derivation lives in `rate-limit-banner-model.ts`; this component maps
 * it onto JSX. When no automatic resend is scheduled, a Continue button asks
 * the quota-recovery worker to make the same durable SQL generation due now;
 * the card disappears once the recovered run starts and the server clears the
 * rate-limit state.
 */
export function RateLimitBanner({
  rateLimit,
  onContinue,
}: {
  rateLimit?: RenderRateLimit | null;
  /**
   * Makes the current durable recovery generation due now. The button shows a
   * sending state until the request settles, so an already-claimed generation
   * or failed wake re-arms the button instead of leaving it stuck.
   */
  onContinue?: () => void | Promise<unknown>;
}) {
  const { t, locale } = useI18n();
  const [now, setNow] = useState(() => Date.now());
  const [sending, setSending] = useState(false);
  const [hovered, setHovered] = useState(false);
  const [accountHovered, setAccountHovered] = useState(false);
  const [accountSwitcherOpen, setAccountSwitcherOpen] = useState(false);
  const [accounts, setAccounts] = useState<DesktopClaudeCodeAccounts | null>(null);
  const [accountsLoading, setAccountsLoading] = useState(false);
  const [accountsError, setAccountsError] = useState<string | null>(null);
  const [accountMutationId, setAccountMutationId] = useState<string | null>(null);
  const [recoveryNotice, setRecoveryNotice] = useState<string | null>(null);

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

  const rawProvider = rateLimit?.provider?.trim() ?? "";
  const normalizedProvider = normalizeRateLimitProvider(rawProvider);
  const isClaude = normalizedProvider === "claude_code";

  useEffect(() => {
    if (isClaude) void refreshAccounts();
    // A new terminal quota context is the only time this card needs a fresh
    // selected-account snapshot; the selector refreshes again when opened.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isClaude, rateLimit?.resetAt, rateLimit?.message]);

  if (!rateLimit) {
    return null;
  }

  const provider = normalizedProvider
    ? normalizedProvider === "claude_code"
      ? "Claude Code"
      : sharedProviderLabel(normalizedProvider)
    : rawProvider || "Provider";
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
      // The gateway fires the resend a buffer after the reset, so the card
      // promises the reset time and "then", not an exact resend instant.
      detail = renderTemplate(
        t("Resets at {time} · {remaining} left · then auto-resends"),
        {
          time: <strong style={strongStyle}>{clock}</strong>,
          remaining: <strong style={strongStyle}>{remaining}</strong>,
        },
      );
      break;
    case "resending":
      detail = t("Quota recovered — auto-resend within a minute…");
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

  async function refreshAccounts() {
    setAccountsLoading(true);
    setAccountsError(null);
    try {
      setAccounts(await window.garyxDesktop.listClaudeCodeAccounts());
    } catch (error) {
      setAccountsError(
        error instanceof Error ? error.message : t("Failed to load accounts."),
      );
    } finally {
      setAccountsLoading(false);
    }
  }

  async function handleSelectAccount(account: DesktopClaudeCodeAccount) {
    const mutationId = account.id || "system";
    setAccountMutationId(mutationId);
    setAccountsError(null);
    try {
      const result = await window.garyxDesktop.selectClaudeCodeAccount({
        accountId: account.id,
      });
      await refreshAccounts();
      setAccountSwitcherOpen(false);
      if (result.recoveryWarning) {
        setRecoveryNotice(t("Account switched. Retry the paused threads manually."));
      } else if (result.recovery.matchedThreads > 0) {
        setRecoveryNotice(
          t("Resuming {count} paused threads…", {
            count: result.recovery.matchedThreads,
          }),
        );
      } else {
        setRecoveryNotice(t("Account switched."));
      }
    } catch (error) {
      setAccountsError(
        error instanceof Error ? error.message : t("Failed to switch account."),
      );
    } finally {
      setAccountMutationId(null);
    }
  }
  const handleContinue = () => {
    if (sending || !onContinue) {
      return;
    }
    setSending(true);
    // Re-arm when the wake request settles: an already-claimed generation or
    // failed request leaves the card mounted, while a successful recovery run
    // clears the rate-limit state and unmounts the card.
    void Promise.resolve()
      .then(() => onContinue())
      .catch(() => {})
      .then(() => setSending(false));
  };

  return (
    <>
      <article
        aria-live="polite"
        className="rate-limit-banner"
        role="status"
        style={bannerStyle}
      >
        <div style={headerStyle}>
          <span aria-hidden="true" style={chipStyle}>
            {normalizedProvider ? (
              <ProviderAgentIcon agentId={normalizedProvider} size={24} />
            ) : rateLimit.willAutoResend ? (
              autoResendIcon
            ) : (
              hourglassIcon
            )}
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
        </div>
        {isClaude ? (
          <div style={accountSectionStyle}>
            <button
              aria-label={t("Switch Claude Code account")}
              onClick={() => {
                setRecoveryNotice(null);
                setAccountSwitcherOpen(true);
                void refreshAccounts();
              }}
              onMouseEnter={() => setAccountHovered(true)}
              onMouseLeave={() => setAccountHovered(false)}
              style={{
                ...accountRowStyle,
                ...(accountHovered ? accountRowHoverStyle : null),
              }}
              type="button"
            >
              <span style={accountCopyStyle}>
                <span style={accountEyebrowStyle}>{t("Claude Code account")}</span>
                <strong style={accountNameStyle}>
                  {accounts?.accounts.find((account) => account.selected)?.name
                    || (accountsLoading ? t("Loading account…") : t("Choose account"))}
                </strong>
              </span>
              <span style={switchLabelStyle}>
                {t("Switch account")}
                {chevronIcon}
              </span>
            </button>
            <p style={accountHintStyle}>
              {recoveryNotice
                || t("Switching accounts resumes every Claude thread paused by quota.")}
            </p>
          </div>
        ) : null}
      </article>
      {isClaude ? (
        <ClaudeAccountSwitcherDialog
          accounts={accounts}
          description={t("Choose an account to resume every quota-paused Claude thread.")}
          error={accountsError}
          loading={accountsLoading}
          mutationId={accountMutationId}
          onOpenChange={setAccountSwitcherOpen}
          onSelect={handleSelectAccount}
          open={accountSwitcherOpen}
        />
      ) : null}
    </>
  );
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

const chevronIcon = (
  <svg
    aria-hidden="true"
    fill="none"
    height="14"
    stroke="currentColor"
    strokeLinecap="round"
    strokeLinejoin="round"
    strokeWidth="2"
    viewBox="0 0 24 24"
    width="14"
  >
    <path d="m9 18 6-6-6-6" />
  </svg>
);

const bannerStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  margin: "12px 0",
  padding: "18px",
  width: "min(100%, 620px)",
  borderRadius: 14,
  border: "1px solid #e7e5e0",
  background: "#ffffff",
  boxShadow: "0 2px 8px rgba(26, 28, 31, 0.05)",
};

const headerStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 14,
};

const chipStyle: CSSProperties = {
  flex: "0 0 auto",
  width: 38,
  height: 38,
  borderRadius: 10,
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
  fontSize: 14,
  fontWeight: 600,
  lineHeight: 1.35,
  color: "#1a1c1f",
  letterSpacing: "-0.08px",
};

const detailStyle: CSSProperties = {
  fontSize: 13,
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
  height: 32,
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

const accountSectionStyle: CSSProperties = {
  marginTop: 16,
  paddingTop: 14,
  borderTop: "1px solid #eceae6",
};

const accountRowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  gap: 16,
  width: "calc(100% + 20px)",
  minHeight: 52,
  padding: "8px 10px",
  margin: "0 -10px",
  border: 0,
  borderRadius: 10,
  background: "transparent",
  color: "#1a1c1f",
  fontFamily: "inherit",
  textAlign: "left",
  cursor: "pointer",
};

const accountRowHoverStyle: CSSProperties = { background: "#f7f7f5" };

const accountCopyStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 2,
  minWidth: 0,
};

const accountEyebrowStyle: CSSProperties = {
  fontSize: 11,
  lineHeight: 1.3,
  color: "#8b877f",
};

const accountNameStyle: CSSProperties = {
  overflow: "hidden",
  fontSize: 13,
  fontWeight: 600,
  lineHeight: 1.35,
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
};

const switchLabelStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 4,
  flex: "0 0 auto",
  fontSize: 12,
  fontWeight: 600,
};

const accountHintStyle: CSSProperties = {
  margin: "5px 0 0",
  color: "#8b877f",
  fontSize: 11,
  lineHeight: 1.45,
};

const continueButtonHoverStyle: CSSProperties = {
  background: "#f6f5f1",
};

const continueButtonSendingStyle: CSSProperties = {
  opacity: 0.55,
  cursor: "default",
};
