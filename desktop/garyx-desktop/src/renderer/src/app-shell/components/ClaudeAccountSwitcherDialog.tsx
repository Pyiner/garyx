import type { ReactNode } from "react";
import { Plus } from "lucide-react";

import type {
  DesktopClaudeCodeAccount,
  DesktopClaudeCodeAccounts,
  DesktopProviderUsage,
  DesktopUsageWindow,
} from "@shared/contracts";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

import { useI18n } from "../../i18n";
import {
  clampUsagePercent,
  formatUsageDuration,
  formatUsagePercent,
  usageLevelForRemainingPercent,
  usageResetText,
} from "../../provider-usage";
import { classNames } from "../../settings/shared";

type ClaudeAccountSwitcherDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  accounts: DesktopClaudeCodeAccounts | null;
  loading: boolean;
  error: string | null;
  mutationId: string | null;
  onSelect: (account: DesktopClaudeCodeAccount) => void | Promise<unknown>;
  onAdd?: () => void;
  onReauthenticate?: (account: DesktopClaudeCodeAccount) => void;
  onRename?: (account: DesktopClaudeCodeAccount) => void;
  onDelete?: (account: DesktopClaudeCodeAccount) => void;
  description?: string;
};

/**
 * Shared, app-centered Claude account selector. Provider Settings supplies
 * management callbacks; transcript recovery intentionally supplies selection
 * only, keeping the in-thread action focused while preserving identical quota
 * evidence for each candidate account.
 */
export function ClaudeAccountSwitcherDialog({
  open,
  onOpenChange,
  accounts,
  loading,
  error,
  mutationId,
  onSelect,
  onAdd,
  onReauthenticate,
  onRename,
  onDelete,
  description,
}: ClaudeAccountSwitcherDialogProps) {
  const { t } = useI18n();
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="provider-account-dialog" size="form">
        <DialogHeader>
          <DialogTitle>{t("Claude Code accounts")}</DialogTitle>
          <DialogDescription>
            {description || t("Choose the account for new Claude Code runs.")}
          </DialogDescription>
        </DialogHeader>
        <div className="provider-account-dialog-body">
          {error ? <div className="provider-account-error">{error}</div> : null}
          {loading && !accounts ? (
            <div className="provider-account-loading">{t("Loading accounts…")}</div>
          ) : null}
          <div className="codex-list-card provider-account-list">
            {(accounts?.accounts || []).map((account) => {
              const accountKey = account.id || "system";
              const windows = providerUsageWindows(account.usage, t);
              return (
                <div
                  className={classNames(
                    "provider-account-option",
                    account.selected && "is-selected",
                  )}
                  key={accountKey}
                >
                  <label className="provider-account-choice">
                    <input
                      aria-label={
                        account.selected
                          ? t("Current account: {name}", { name: account.name })
                          : t("Use account: {name}", { name: account.name })
                      }
                      checked={account.selected}
                      className="provider-account-radio"
                      disabled={Boolean(mutationId)}
                      name="claude-code-account"
                      onChange={() => {
                        if (!account.selected) void onSelect(account);
                      }}
                      type="radio"
                    />
                    <div className="provider-account-option-content">
                      <div className="provider-account-option-header">
                        <div className="provider-account-option-copy">
                          <div>
                            <strong>{account.name}</strong>
                            {account.selected ? (
                              <Badge variant="outline">{t("Current")}</Badge>
                            ) : null}
                            {account.plan ? (
                              <Badge variant="outline">{account.plan}</Badge>
                            ) : null}
                          </div>
                          <span>
                            {account.email
                              || account.organization
                              || (account.systemDefault
                                ? t("This Mac’s default Claude Code login")
                                : t("Added to Garyx"))}
                          </span>
                        </div>
                        {mutationId === accountKey ? <span>{t("Switching…")}</span> : null}
                      </div>
                      <div className="provider-account-option-meters">
                        {account.usage.available && windows.length > 0 ? (
                          windows.map((entry) => (
                            <div key={entry.key}>
                              {renderUsageMeter(
                                entry.label,
                                entry.value.remainingPercent,
                                usageResetText(
                                  entry.value.resetsAt,
                                  entry.value.resetAfterSeconds,
                                  entry.fallback,
                                ),
                                account.usage.stale,
                              )}
                            </div>
                          ))
                        ) : (
                          <span className="provider-card-empty">
                            {unavailableUsageText(account.usage, t)}
                          </span>
                        )}
                      </div>
                    </div>
                  </label>
                  {onReauthenticate || (!account.systemDefault && (onRename || onDelete)) ? (
                    <div className="provider-account-option-actions">
                      {onReauthenticate ? (
                        <button onClick={() => onReauthenticate(account)} type="button">
                          {t("Sign in again")}
                        </button>
                      ) : null}
                      {!account.systemDefault && onRename ? (
                        <button onClick={() => onRename(account)} type="button">
                          {t("Rename")}
                        </button>
                      ) : null}
                      {!account.systemDefault && onDelete ? (
                        <button
                          className="destructive"
                          onClick={() => onDelete(account)}
                          type="button"
                        >
                          {t("Delete")}
                        </button>
                      ) : null}
                    </div>
                  ) : null}
                </div>
              );
            })}
          </div>
        </div>
        {onAdd ? (
          <DialogFooter>
            <Button onClick={onAdd} type="button">
              <Plus aria-hidden size={14} strokeWidth={2} />
              {t("Add Claude account")}
            </Button>
          </DialogFooter>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

type Translate = (key: string, values?: Record<string, string | number>) => string;

function providerUsageWindows(
  usage: DesktopProviderUsage,
  t: Translate,
): Array<{ key: string; label: string; value: DesktopUsageWindow; fallback: string }> {
  const windows = [] as Array<{
    key: string;
    label: string;
    value: DesktopUsageWindow;
    fallback: string;
  }>;
  if (usage.session) {
    windows.push({
      key: "session",
      label: t("Session"),
      value: usage.session,
      fallback: t("session window"),
    });
  }
  if (usage.weekly) {
    windows.push({
      key: "weekly",
      label: t("Weekly"),
      value: usage.weekly,
      fallback: t("weekly window"),
    });
  }
  for (const limit of usage.scopedLimits) {
    windows.push({
      key: `scoped:${limit.id}`,
      label: limit.name,
      value: limit.window,
      fallback: limit.kind.includes("weekly") ? t("weekly window") : t("usage window"),
    });
  }
  return windows;
}

function unavailableUsageText(usage: DesktopProviderUsage, t: Translate): string {
  switch (usage.errorCode) {
    case "rate_limited":
      return usage.retryAfterSeconds && usage.retryAfterSeconds > 0
        ? t("Try again in {age}", { age: formatUsageDuration(usage.retryAfterSeconds) })
        : t("Quota temporarily rate limited");
    case "reauth_required":
      return t("Sign in again to refresh quota");
    case "credentials_unavailable":
      return t("Account credentials unavailable");
    case "network":
      return t("Quota refresh failed — check connection");
    default:
      return t("Quota temporarily unavailable");
  }
}

function renderUsageMeter(
  label: string,
  remainingPercent: number,
  caption: string,
  stale: boolean,
): ReactNode {
  const percent = clampUsagePercent(remainingPercent);
  return (
    <div
      className="provider-usage-meter compact"
      data-level={usageLevelForRemainingPercent(percent)}
      data-stale={stale ? "true" : undefined}
    >
      <div className="provider-usage-meter-header">
        <span className="provider-usage-meter-label">{label}</span>
        <span className="provider-usage-meter-percent">{formatUsagePercent(percent)}</span>
      </div>
      <div className="provider-usage-meter-track" aria-hidden>
        <span className="provider-usage-meter-fill" style={{ width: `${percent}%` }} />
      </div>
      {caption ? <div className="provider-usage-meter-caption">{caption}</div> : null}
    </div>
  );
}
