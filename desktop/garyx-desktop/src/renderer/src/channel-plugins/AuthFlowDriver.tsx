/**
 * Channel-blind auto-login driver.
 *
 * Reads `config_methods` on the catalog entry; if it contains
 * `{kind: "auto_login"}`, the caller renders this component and it
 * takes over:
 *
 *   1. On mount, call `auth_flow/start` with the current form state.
 *   2. Render the returned `display[]` (Text / Qr items) — the
 *      plugin controls layout by ordering.
 *   3. Poll `auth_flow/poll` every `poll_interval_secs`. On each
 *      Pending, optionally replace the display; honour any
 *      `next_interval_secs` backoff hint.
 *   4. On `confirmed`, invoke `onConfirmed(values)` so the parent
 *      can merge the values into its form state and let the user
 *      review before saving.
 *   5. On `failed`, surface the reason and expose "try again" so
 *      the user can restart the flow.
 *
 * The component deliberately holds NO channel-specific knowledge.
 * Any plugin — built-in or subprocess — that advertises
 * `auto_login` works end-to-end through it.
 */
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { Copy, ExternalLink, RefreshCw } from "lucide-react";
import QRCode from "qrcode";
import { isAuthFlowQrCardBoilerplateText, useI18n } from "@/i18n";

/** One renderable item, matching `plugin_host::AuthFlowDisplayItem`. */
type DisplayItem = { kind: string; value?: string; label?: string };

interface AuthFlowDriverProps {
  /**
   * Canonical plugin id or alias. Passed verbatim to the gateway
   * endpoint — alias resolution happens server-side.
   */
  pluginId: string;
  /**
   * Current form state (JSON-Schema instance) the user has filled
   * so far. Passed to `auth_flow/start` verbatim; the plugin picks
   * the keys it needs and defaults the rest from its schema.
   */
  formState: Record<string, unknown>;
  /**
   * Called with the `values` patch on `Confirmed`. The caller
   * typically merges it back into its form state and lets the user
   * click "save" to commit.
   */
  onConfirmed: (values: Record<string, unknown>) => void;
  /**
   * Called when the user clicks the "cancel" button before the
   * session terminates. Unmounting is another way to cancel; both
   * are treated identically by the gateway (the session will
   * expire on its own TTL).
   */
  onCancel?: () => void;
  /**
   * The default rendering preserves every display item. `qr-card`
   * is the denser Add Bot flow treatment: QR first with its
   * encoded link/payload always visible underneath.
   */
  presentation?: "default" | "qr-card";
  /** Short brand initials shown in the center of the QR card. */
  badge?: string;
  /** Catalog-provided brand icon shown in the center of the QR card. */
  iconDataUrl?: string | null;
}

type DriverPhase =
  | { tag: "starting" }
  | {
      tag: "polling";
      sessionId: string;
      display: DisplayItem[];
      intervalSecs: number;
    }
  | { tag: "confirmed"; values: Record<string, unknown> }
  | { tag: "failed"; reason: string };

export function AuthFlowDriver(props: AuthFlowDriverProps) {
  const { t } = useI18n();
  const {
    pluginId,
    formState,
    onConfirmed,
    onCancel,
    presentation = "default",
    badge,
    iconDataUrl,
  } = props;
  const [phase, setPhase] = useState<DriverPhase>({ tag: "starting" });
  const [startError, setStartError] = useState<string | null>(null);
  const openedExternalUrlsRef = useRef(new Set<string>());

  const start = useCallback(async () => {
    setStartError(null);
    setPhase({ tag: "starting" });
    try {
      const api = window.garyxDesktop;
      if (!api?.startChannelAuthFlow) {
        throw new Error(
          "desktop IPC missing startChannelAuthFlow — update required",
        );
      }
      const session = await api.startChannelAuthFlow({ pluginId, formState });
      setPhase({
        tag: "polling",
        sessionId: session.sessionId,
        display: session.display ?? [],
        intervalSecs: Math.max(1, session.pollIntervalSecs),
      });
    } catch (err) {
      setStartError(err instanceof Error ? err.message : String(err));
    }
  }, [pluginId, formState]);

  // Kick off on mount.
  useEffect(() => {
    void start();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Poll loop. Re-subscribes whenever the session id or interval
  // changes (e.g. after a `next_interval_secs` backoff).
  useEffect(() => {
    if (phase.tag !== "polling") return;
    let cancelled = false;

    const tick = async () => {
      if (cancelled) return;
      try {
        const api = window.garyxDesktop;
        if (!api?.pollChannelAuthFlow) {
          setPhase({
            tag: "failed",
            reason: "desktop IPC missing pollChannelAuthFlow",
          });
          return;
        }
        const result = await api.pollChannelAuthFlow({
          pluginId,
          sessionId: phase.sessionId,
        });
        if (cancelled) return;
        if (result.status === "confirmed") {
          setPhase({ tag: "confirmed", values: result.values ?? {} });
          onConfirmed(result.values ?? {});
          return;
        }
        if (result.status === "failed") {
          setPhase({ tag: "failed", reason: result.reason ?? t("unknown") });
          return;
        }
        // pending — possibly with a display refresh + backoff.
        setPhase((prev) => {
          if (prev.tag !== "polling" || prev.sessionId !== phase.sessionId) {
            return prev;
          }
          let nextDisplay = Array.isArray(result.display)
            ? result.display
            : prev.display;
          if (presentation === "qr-card" && Array.isArray(result.display)) {
            const nextHasQr = result.display.some((item) => item.kind === "qr");
            const previousQr = prev.display.filter((item) => item.kind === "qr");
            if (!nextHasQr && previousQr.length > 0) {
              nextDisplay = [...previousQr, ...result.display];
            }
          }
          const nextInterval = result.next_interval_secs
            ? Math.max(1, Number(result.next_interval_secs))
            : prev.intervalSecs;
          return {
            tag: "polling",
            sessionId: prev.sessionId,
            display: nextDisplay,
            intervalSecs: nextInterval,
          };
        });
      } catch (err) {
        if (cancelled) return;
        setPhase({
          tag: "failed",
          reason: err instanceof Error ? err.message : String(err),
        });
      }
    };

    const timer = setTimeout(() => void tick(), phase.intervalSecs * 1000);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [phase, pluginId, onConfirmed, presentation, t]);

  useEffect(() => {
    if (phase.tag !== "polling") return;
    const url = firstDisplayUrl(phase.display);
    if (!url) return;
    const key = `${phase.sessionId}:${url}`;
    if (openedExternalUrlsRef.current.has(key)) return;
    openedExternalUrlsRef.current.add(key);
    void openExternalAuthUrl(url);
  }, [phase]);

  if (phase.tag === "starting") {
    if (startError) {
      return (
        <DriverShell presentation={presentation}>
          <p className="auth-flow-error">{startError}</p>
          <div className="auth-flow-actions">
            <button
              type="button"
              onClick={() => void start()}
              className="auth-flow-primary-action"
            >
              {t("Retry")}
            </button>
            {onCancel && (
              <button
                type="button"
                onClick={onCancel}
                className="auth-flow-secondary-action"
              >
                {t("Cancel")}
              </button>
            )}
          </div>
        </DriverShell>
      );
    }
    return (
      <DriverShell presentation={presentation}>
        <p className="auth-flow-muted">{t("Starting login session...")}</p>
      </DriverShell>
    );
  }

  if (phase.tag === "polling") {
    return (
      <DriverShell presentation={presentation}>
        <DisplayList
          badge={badge}
          iconDataUrl={iconDataUrl}
          items={phase.display}
          onRefresh={() => void start()}
          presentation={presentation}
        />
        {presentation === "default" ? (
          <p className="auth-flow-muted small">
            {t("Waiting for confirmation... Refreshes about every {seconds} seconds.", {
              seconds: phase.intervalSecs,
            })}
          </p>
        ) : null}
        {onCancel && presentation === "default" ? (
          <button
            type="button"
            onClick={onCancel}
            className="auth-flow-secondary-action"
          >
            {t("Cancel")}
          </button>
        ) : null}
      </DriverShell>
    );
  }

  if (phase.tag === "confirmed") {
    return (
      <DriverShell presentation={presentation}>
        <p className="auth-flow-success">{t("Login succeeded. Account info has been filled into the form.")}</p>
        <p className="auth-flow-muted small">{t("Review the info, then click Save.")}</p>
      </DriverShell>
    );
  }

  // failed
  return (
    <DriverShell presentation={presentation}>
      <p className="auth-flow-error">{t("Login failed: {reason}", { reason: phase.reason })}</p>
      <div className="auth-flow-actions">
        <button
          type="button"
          onClick={() => void start()}
          className="auth-flow-primary-action"
        >
          {t("Try again")}
        </button>
        {onCancel && (
          <button
            type="button"
            onClick={onCancel}
            className="auth-flow-secondary-action"
          >
            {t("Close")}
          </button>
        )}
      </div>
    </DriverShell>
  );
}

/** Layout wrapper keeping spacing + border consistent across phases. */
function DriverShell(props: {
  children: ReactNode;
  presentation?: "default" | "qr-card";
}) {
  if (props.presentation === "qr-card") {
    return <div className="auth-flow-shell qr-card">{props.children}</div>;
  }
  return (
    <div className="auth-flow-shell">
      {props.children}
    </div>
  );
}

/**
 * Render the display list verbatim. Unknown `kind` values are
 * silently skipped (forward-compat with future item types). QR
 * items are rendered as inline `<img>` by data-URL-encoding the
 * text — UI thread cost is ~1ms per QR.
 */
function DisplayList(props: {
  items: DisplayItem[];
  presentation?: "default" | "qr-card";
  badge?: string;
  iconDataUrl?: string | null;
  onRefresh?: () => void;
}) {
  const { locale, t } = useI18n();
  const { items, presentation = "default", badge, iconDataUrl, onRefresh } = props;

  if (presentation === "qr-card") {
    const qrItem = items.find((item) => item.kind === "qr" && item.value);
    const urlText = items.find((item) => displayItemUrl(item));
    const linkValue = urlText?.value || qrItem?.value || "";
    const importantText = items
      .filter((item) => item.kind === "text" && item.value && !displayItemUrl(item))
      .map((item) => item.value?.trim() || "")
      .filter((text) => {
        if (!text) return false;
        return !isAuthFlowQrCardBoilerplateText(text, locale);
      });

    if (!qrItem?.value) {
      return (
        <div className="auth-flow-display-list compact">
          {linkValue ? (
            <AuthLinkCard
              importantText={importantText}
              linkValue={linkValue}
              onRefresh={onRefresh}
            />
          ) : (
            importantText.map((text, idx) => (
              <TextItem key={idx} value={text} compact />
            ))
          )}
        </div>
      );
    }

    return (
      <div className="auth-flow-display-list compact">
        {importantText.length > 0 ? (
          <div className="auth-flow-important-text">
            {importantText.map((text, idx) => (
              <TextItem key={idx} value={text} compact />
            ))}
          </div>
        ) : null}
        <QrItem
          badge={badge}
          iconDataUrl={iconDataUrl}
          presentation="qr-card"
          value={qrItem.value}
        />
        {linkValue ? (
          <QrLinkActions linkValue={linkValue} onRefresh={onRefresh} />
        ) : null}
      </div>
    );
  }

  return (
    <div className="auth-flow-display-list">
      {items.map((item, idx) => {
        if (item.kind === "text" && item.value) {
          return <TextItem key={idx} value={item.value} />;
        }
        if (item.kind === "url" && item.value && isUrl(item.value)) {
          return (
            <AuthInlineLink
              key={idx}
              label={item.label || t("Open authorization link")}
              value={item.value}
            />
          );
        }
        if (item.kind === "qr" && item.value) {
          return (
            <div className="auth-flow-qr-block" key={idx}>
              <QrItem value={item.value} />
              <QrLinkActions linkValue={item.value} />
            </div>
          );
        }
        return null;
      })}
    </div>
  );
}

function displayItemUrl(item: DisplayItem): string | null {
  const value = item.value?.trim() || "";
  if (!isUrl(value)) return null;
  if (item.kind === "url") return value;
  if (item.kind === "text") return value;
  return null;
}

function firstDisplayUrl(items: DisplayItem[]): string | null {
  for (const item of items) {
    const url = displayItemUrl(item);
    if (url) return url;
  }
  return null;
}

function isUrl(value: string): boolean {
  const trimmed = value.trim();
  return (
    (trimmed.startsWith("http://") || trimmed.startsWith("https://")) &&
    !/\s/.test(trimmed)
  );
}

async function openExternalAuthUrl(value: string): Promise<void> {
  const url = value.trim();
  if (!isUrl(url)) return;
  const api = window.garyxDesktop;
  if (api?.openExternalUrl) {
    await api.openExternalUrl({ url });
    return;
  }
  window.open(url, "_blank", "noopener,noreferrer");
}

/**
 * Render a text item. If the text parses as an http(s) URL, it's
 * shown as a clickable link; otherwise plain paragraph. The plugin
 * controls whether to emit a URL as its own Text item (convention)
 * so the UI can style it independently from surrounding prose.
 */
function TextItem(props: { value: string; compact?: boolean }) {
  const trimmed = props.value.trim();
  if (isUrl(trimmed)) {
    return (
      <AuthInlineLink value={trimmed} />
    );
  }
  return (
    <p className={props.compact ? "auth-flow-text compact" : "auth-flow-text"}>
      {props.value}
    </p>
  );
}

function AuthInlineLink(props: { value: string; label?: string }) {
  const label = props.label || props.value;
  return (
    <button
      className="auth-flow-link"
      onClick={() => void openExternalAuthUrl(props.value)}
      type="button"
    >
      <ExternalLink aria-hidden size={13} strokeWidth={1.8} />
      <span>{label}</span>
    </button>
  );
}

function AuthLinkCard(props: {
  importantText: string[];
  linkValue: string;
  onRefresh?: () => void;
}) {
  const { t } = useI18n();
  const { importantText, linkValue, onRefresh } = props;
  const [copied, setCopied] = useState(false);

  async function copyLink() {
    try {
      await navigator.clipboard.writeText(linkValue);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    } catch {
      setCopied(false);
    }
  }

  return (
    <div className="auth-link-card">
      <div className="auth-link-card-main">
        <span className="auth-link-card-icon" aria-hidden>
          <ExternalLink size={17} strokeWidth={1.9} />
        </span>
        <div className="auth-link-card-copy">
          <p>{t("Authorization link opened in the browser")}</p>
          <button
            className="auth-link-card-open"
            onClick={() => void openExternalAuthUrl(linkValue)}
            type="button"
          >
            {t("Open again")}
          </button>
        </div>
      </div>
      {importantText.length > 0 ? (
        <div className="auth-flow-important-text device">
          {importantText.map((text, idx) => (
            <TextItem key={idx} value={text} compact />
          ))}
        </div>
      ) : null}
      <div className="auth-qr-link-row device">
        <code className="auth-qr-link-url">{linkValue}</code>
        <button
          aria-label={t("Copy authorization link")}
          className="auth-qr-link-copy"
          onClick={() => void copyLink()}
          title={copied ? t("Copied") : t("Copy")}
          type="button"
        >
          <Copy aria-hidden size={13} strokeWidth={1.7} />
        </button>
      </div>
      {onRefresh ? (
        <button
          className="auth-qr-link-toggle"
          onClick={onRefresh}
          type="button"
        >
          <RefreshCw aria-hidden size={11} strokeWidth={1.8} />
          {t("Refresh")}
        </button>
      ) : null}
    </div>
  );
}

function QrLinkActions(props: {
  linkValue: string;
  onRefresh?: () => void;
}) {
  const { t } = useI18n();
  const { linkValue, onRefresh } = props;
  const [copied, setCopied] = useState(false);
  const linkIsUrl = isUrl(linkValue);

  async function copyLink() {
    try {
      await navigator.clipboard.writeText(linkValue);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    } catch {
      setCopied(false);
    }
  }

  return (
    <div className="auth-qr-link">
      {linkIsUrl || onRefresh ? (
        <div className="auth-qr-actions">
          {linkIsUrl ? (
            <button
              className="auth-qr-link-toggle"
              onClick={() => void openExternalAuthUrl(linkValue)}
              type="button"
            >
              <ExternalLink aria-hidden size={11} strokeWidth={1.8} />
              {t("Open link")}
            </button>
          ) : null}
          {onRefresh ? (
            <>
              {linkIsUrl ? <span className="auth-qr-actions-sep">·</span> : null}
              <button
                className="auth-qr-link-toggle"
                onClick={onRefresh}
                type="button"
              >
                <RefreshCw aria-hidden size={11} strokeWidth={1.8} />
                {t("Refresh")}
              </button>
            </>
          ) : null}
        </div>
      ) : null}
      <div className="auth-qr-link-body">
        <div className="auth-qr-link-row">
          {linkIsUrl ? (
            <button
              className="auth-qr-link-url clickable"
              onClick={() => void openExternalAuthUrl(linkValue)}
              title={linkValue}
              type="button"
            >
              {linkValue}
            </button>
          ) : (
            <code className="auth-qr-link-url" title={linkValue}>
              {linkValue}
            </code>
          )}
          <button
            aria-label={t("Copy QR link")}
            className="auth-qr-link-copy"
            onClick={() => void copyLink()}
            title={copied ? t("Copied") : t("Copy")}
            type="button"
          >
            <Copy aria-hidden size={13} strokeWidth={1.7} />
          </button>
        </div>
      </div>
    </div>
  );
}

/** QR item — async-render into a data URL, stash in state. */
function QrItem(props: {
  value: string;
  presentation?: "default" | "qr-card";
  badge?: string;
  iconDataUrl?: string | null;
}) {
  const { t } = useI18n();
  const [dataUrl, setDataUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const qrWidth = props.presentation === "qr-card" ? 172 : 240;

  useEffect(() => {
    let cancelled = false;
    QRCode.toDataURL(props.value, { margin: 1, width: qrWidth })
      .then((url) => {
        if (!cancelled) setDataUrl(url);
      })
      .catch((err) => {
        if (!cancelled)
          setError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [props.value, qrWidth]);

  if (error) {
    return (
      <pre className="auth-flow-raw-value">
        {props.value}
      </pre>
    );
  }
  if (!dataUrl) {
    return (
      <div className={props.presentation === "qr-card" ? "auth-qr-card loading" : "auth-qr-default loading"}>
        {t("Rendering QR code...")}
      </div>
    );
  }

  if (props.presentation === "qr-card") {
    return (
      <div className="auth-qr-card">
        <img
          src={dataUrl}
          alt={t("auth QR code")}
          width={172}
          height={172}
          className="auth-qr-image"
        />
        <span className="auth-qr-corner tl" />
        <span className="auth-qr-corner tr" />
        <span className="auth-qr-corner bl" />
        <span className="auth-qr-corner br" />
        {props.iconDataUrl ? (
          <span className="auth-qr-logo image">
            <img alt="" src={props.iconDataUrl} />
          </span>
        ) : props.badge ? (
          <span className="auth-qr-logo">
            <span>{props.badge}</span>
          </span>
        ) : null}
      </div>
    );
  }

  return (
    <img
      src={dataUrl}
      alt={t("auth QR code")}
      width={240}
      height={240}
      className="auth-qr-default"
    />
  );
}
