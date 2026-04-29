/**
 * Schema-driven "Add a bot" dialog.
 *
 * Rewritten (April 2026) from a 583-line per-channel branching
 * monster into a catalog-driven flow around `JsonSchemaForm` and
 * `AuthFlowDriver`. The dialog is channel-blind: it reads
 * `GET /api/channels/plugins`, lets the user pick a channel, then
 * renders the selected entry's JSON Schema plus optional auto-login.
 *
 * Save path: the panel emits a flat `Record<string, unknown>` of
 * plugin config values; this dialog adds the generic account-
 * metadata fields (accountId, name, workspaceDir) and calls the
 * legacy `onCreateChannel` handler with the combined payload. That
 * handler still takes channel-specific typed fields (token /
 * appId / appSecret / baseUrl / domain), so a small translator
 * maps schema keys → legacy fields per channel id. The translator
 * is temporary — once `addChannelAccount` is migrated to accept a
 * generic `config: Record<string, unknown>` payload, the mapping
 * disappears and this file drops another ~30 lines.
 *
 * The auth-flow callback props (onStartWeixinAuth, onStartFeishuAuth,
 * their poll counterparts) are retained in the props signature for
 * AppShell compatibility but are IGNORED — the new flow drives
 * auth through the generic `AuthFlowDriver` via
 * `garyx:start-channel-auth-flow` IPC.
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import { Check, ChevronLeft, ChevronRight } from "lucide-react";

import type {
  ChannelPluginCatalogEntry,
  PollFeishuChannelAuthResult,
  PollWeixinChannelAuthResult,
  StartFeishuChannelAuthResult,
} from "@shared/contracts";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

import { AuthFlowDriver } from "../../channel-plugins/AuthFlowDriver";
import { DirectoryInput } from "../../components/DirectoryInput";
import { JsonSchemaForm } from "../../channel-plugins/JsonSchemaForm";
import { useChannelPluginCatalog } from "../../channel-plugins/useChannelPluginCatalog";
import { useI18n } from "../../i18n";

type FeishuDomain = "feishu" | "lark";
type AgentTargetOption = { value: string; label: string };
type AddBotStep = 1 | 2;

type AddBotDialogProps = {
  open: boolean;
  initialValues?: {
    channel?: string;
    accountId?: string;
    name?: string;
    agentId?: string;
    token?: string;
    baseUrl?: string;
  } | null;
  agentTargets: AgentTargetOption[];
  onClose: () => void;
  onCreateChannel: (input: {
    channel: string;
    accountId: string;
    name?: string | null;
    workspaceDir?: string | null;
    agentId?: string | null;
    token?: string | null;
    appId?: string | null;
    appSecret?: string | null;
    baseUrl?: string | null;
    domain?: FeishuDomain | null;
    /** Opaque plugin config for subprocess plugins; empty for
     * legacy built-ins which use the typed fields above. */
    config?: Record<string, unknown> | null;
  }) => Promise<void>;
  /** Retained for prop compatibility; no longer called by the new
   * generic auth path. Wired through AppShell's legacy handlers. */
  onStartWeixinAuth?: (input: {
    accountId?: string | null;
    name?: string | null;
    workspaceDir?: string | null;
    baseUrl?: string | null;
  }) => Promise<{ sessionId: string; qrCodeDataUrl: string }>;
  /** Retained for prop compatibility; no longer called. */
  onPollWeixinAuth?: (input: { sessionId: string }) => Promise<PollWeixinChannelAuthResult>;
  /** Retained for prop compatibility; no longer called. */
  onStartFeishuAuth?: (input: {
    accountId?: string | null;
    name?: string | null;
    workspaceDir?: string | null;
    domain?: FeishuDomain | null;
  }) => Promise<StartFeishuChannelAuthResult>;
  /** Retained for prop compatibility; no longer called. */
  onPollFeishuAuth?: (input: { sessionId: string }) => Promise<PollFeishuChannelAuthResult>;
};

/**
 * Extract the account id from the plugin config values when the
 * plugin's auto-login flow populated an `account_id` field (weixin's
 * case — the QR scanner embeds the ilink bot id). Telegram and
 * feishu don't do this; users always type the id manually for them.
 */
function configAccountIdOverride(
  config: Record<string, unknown>,
): string | undefined {
  const v = config["account_id"];
  return typeof v === "string" && v.length > 0 ? v : undefined;
}

function schemaDeclaresField(
  schema: unknown,
  field: "account_id" | "agent_id",
): boolean {
  const properties =
    schema &&
    typeof schema === "object" &&
    !Array.isArray(schema) &&
    (schema as Record<string, unknown>).properties;
  return Boolean(
    properties &&
      typeof properties === "object" &&
      !Array.isArray(properties) &&
      field in (properties as Record<string, unknown>),
  );
}

function stripAuthIdentityHints(
  values: Record<string, unknown>,
  schema: unknown,
): Record<string, unknown> {
  const next = { ...values };
  if (!schemaDeclaresField(schema, "account_id")) {
    delete next.account_id;
  }
  if (!schemaDeclaresField(schema, "agent_id")) {
    delete next.agent_id;
  }
  return next;
}

function randomAccountSuffix(): string {
  const letters = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
  const digits = "0123456789";
  const pool = `${letters}${digits}`;
  const chars = [
    letters[Math.floor(Math.random() * letters.length)],
    digits[Math.floor(Math.random() * digits.length)],
    pool[Math.floor(Math.random() * pool.length)],
  ];
  for (let i = chars.length - 1; i > 0; i -= 1) {
    const j = Math.floor(Math.random() * (i + 1));
    [chars[i], chars[j]] = [chars[j], chars[i]];
  }
  return chars.join("");
}

function accountIdChannelSlug(channelId: string): string {
  return (
    channelId
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "") || "channel"
  );
}

function defaultAccountIdForChannel(channelId: string): string {
  return `${accountIdChannelSlug(channelId)}-${randomAccountSuffix()}`;
}

function methodKind(method: { kind?: string }): string {
  return typeof method?.kind === "string" ? method.kind : "";
}

function resolveConfigMethods(
  entry: ChannelPluginCatalogEntry,
): Array<{ kind: string }> | "empty" {
  const raw = entry.config_methods;
  if (raw === undefined) return [{ kind: "form" }];
  if (raw.length === 0) return "empty";
  return raw;
}

function entrySupportsAutoLogin(entry: ChannelPluginCatalogEntry | null): boolean {
  if (!entry) return false;
  const methods = resolveConfigMethods(entry);
  return (
    methods !== "empty" &&
    methods.some((method) => methodKind(method) === "auto_login")
  );
}

function channelInitials(entry: ChannelPluginCatalogEntry | null): string {
  const source = entry?.display_name || entry?.id || "";
  const words = source
    .replace(/[()]/g, " ")
    .split(/[\s/_-]+/)
    .map((word) => word.trim())
    .filter(Boolean);
  if (words.length >= 2) {
    return `${words[0][0]}${words[1][0]}`.toUpperCase();
  }
  return (source.slice(0, 2) || "CH").toUpperCase();
}

export function AddBotDialog(props: AddBotDialogProps) {
  const { t } = useI18n();
  const { open, initialValues, onClose, onCreateChannel, agentTargets } = props;
  const { entries: allEntries, loading: catalogLoading, error: catalogError } =
    useChannelPluginCatalog();

  // The save IPC is now channel-blind: built-ins keep their typed
  // IPC fields while subprocess plugins forward their opaque `config`
  // payload verbatim. That means every catalog entry is selectable
  // from the picker — no allow-list.
  const entries = allEntries ?? [];

  const [pluginId, setPluginId] = useState<string>("");
  const [step, setStep] = useState<AddBotStep>(1);
  const [accountId, setAccountId] = useState("");
  const [name, setName] = useState("");
  const [workspaceDir, setWorkspaceDir] = useState("");
  const [agentId, setAgentId] = useState("claude");
  const [pluginConfig, setPluginConfig] = useState<Record<string, unknown>>({});
  const [generatedAccountId, setGeneratedAccountId] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const preferredAgentId = useMemo(() => {
    return agentTargets.find((target) => target.value === "claude")?.value
      || agentTargets[0]?.value
      || "claude";
  }, [agentTargets]);

  // Apply initialValues when the dialog opens so the user sees the
  // channel they clicked "edit" on pre-selected with any typed
  // starter fields.
  useEffect(() => {
    if (!open) return;
    setSaving(false);
    setSaveError(null);
    setStep(1);
    if (initialValues?.channel) {
      setPluginId(initialValues.channel);
    }
    if (initialValues?.accountId) {
      setAccountId(initialValues.accountId);
      setGeneratedAccountId(null);
    } else {
      setAccountId("");
      setGeneratedAccountId(null);
    }
    setName(initialValues?.name ?? "");
    setWorkspaceDir("");
    setAgentId(initialValues?.agentId?.trim() || preferredAgentId);
    const seededConfig: Record<string, unknown> = {};
    if (initialValues?.token) seededConfig.token = initialValues.token;
    if (initialValues?.baseUrl) seededConfig.base_url = initialValues.baseUrl;
    setPluginConfig(seededConfig);
  }, [open, initialValues, preferredAgentId]);

  // Default to the first entry once the catalog loads if the user
  // hasn't picked anything and the initialValues didn't pre-select.
  useEffect(() => {
    if (pluginId || !entries || entries.length === 0) return;
    setPluginId(entries[0].id);
  }, [entries, pluginId]);

  const selectedEntry: ChannelPluginCatalogEntry | null = useMemo(() => {
    if (!entries || !pluginId) return null;
    return (
      entries.find((e) => e.id === pluginId || e.id.toLowerCase() === pluginId.toLowerCase()) ??
      null
    );
  }, [entries, pluginId]);
  const selectedAgentMissing = Boolean(
    agentId && agentTargets.length > 0 && !agentTargets.some((target) => target.value === agentId),
  );
  const selectedMethods = useMemo(() => {
    return selectedEntry ? resolveConfigMethods(selectedEntry) : null;
  }, [selectedEntry]);
  const selectedHasAutoLogin = entrySupportsAutoLogin(selectedEntry);

  useEffect(() => {
    if (!open || !selectedEntry || initialValues?.accountId) return;
    const generated = defaultAccountIdForChannel(selectedEntry.id);
    setAccountId(generated);
    setGeneratedAccountId(generated);
  }, [open, selectedEntry?.id, initialValues?.accountId]);

  const handlePluginChange = useCallback((nextPluginId: string) => {
    setPluginId(nextPluginId);
    setPluginConfig({});
    setGeneratedAccountId(null);
    setSaveError(null);
    setStep(1);
  }, []);

  const handleAccountIdChange = useCallback(
    (nextAccountId: string) => {
      setAccountId(nextAccountId);
      if (generatedAccountId && nextAccountId !== generatedAccountId) {
        setGeneratedAccountId(null);
      }
    },
    [generatedAccountId],
  );

  const handleAuthConfirmed = useCallback((values: Record<string, unknown>) => {
    setPluginConfig((current) => ({
      ...current,
      ...stripAuthIdentityHints(values, selectedEntry?.schema),
    }));
    const resolvedFromAuth = configAccountIdOverride(values);
    if (resolvedFromAuth) {
      setAccountId((current) => {
        if (!current.trim() || current === generatedAccountId) {
          setGeneratedAccountId(null);
          return resolvedFromAuth;
        }
        return current;
      });
    }
  }, [generatedAccountId, selectedEntry?.schema]);

  const goToAuthStep = useCallback(() => {
    if (!selectedEntry) {
      setSaveError(t("Choose a channel first."));
      return;
    }
    if (!accountId.trim() && !selectedHasAutoLogin) {
      setSaveError(t("Account ID is required."));
      return;
    }
    setSaveError(null);
    setStep(2);
  }, [accountId, selectedEntry, selectedHasAutoLogin]);

  const handleSave = async () => {
    if (!selectedEntry) {
      setSaveError(t("Choose a channel first."));
      return;
    }
    // Let auto-login-supplied account_id win if the user didn't
    // type one themselves (weixin's QR flow populates it).
    const configAccountId = configAccountIdOverride(pluginConfig);
    const typedAccountId = accountId.trim();
    const resolvedAccountId =
      configAccountId && (!typedAccountId || typedAccountId === generatedAccountId)
        ? configAccountId
        : typedAccountId || configAccountId || "";
    if (!resolvedAccountId) {
      setSaveError(t("Account ID is required."));
      return;
    }
    setSaving(true);
    setSaveError(null);
    try {
      await onCreateChannel({
        channel: selectedEntry.id,
        accountId: resolvedAccountId,
        name: name.trim() || null,
        workspaceDir: workspaceDir.trim() || null,
        agentId: agentId.trim() || null,
        config: pluginConfig,
      });
      onClose();
    } catch (err) {
      setSaveError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()}>
      <DialogContent className="add-bot-dialog">
        <DialogHeader className="add-bot-dialog-header">
          <DialogTitle className="add-bot-dialog-title">{t("Add channel account")}</DialogTitle>
          <DialogDescription className="add-bot-dialog-description">
            {step === 1
              ? t("Confirm the channel and basic info first, then fill in authentication.")
              : selectedHasAutoLogin
                ? t("Scan with your phone to bind. Credentials are written automatically.")
                : t("Fill in the authentication fields required by this channel.")}
          </DialogDescription>
        </DialogHeader>

        <div className="add-bot-stepper" aria-label={t("Add bot steps")}>
          <button
            className={`add-bot-step ${step === 1 ? "current" : "done"}`}
            onClick={() => setStep(1)}
            type="button"
          >
            <span className="add-bot-step-num">
              {step === 2 ? <Check aria-hidden size={11} strokeWidth={2.4} /> : "1"}
            </span>
            <span>{t("Basic Info")}</span>
          </button>
          <span className={`add-bot-step-line ${step === 2 ? "filled" : ""}`} />
          <button
            className={`add-bot-step ${step === 2 ? "current" : ""}`}
            disabled={!selectedEntry}
            onClick={goToAuthStep}
            type="button"
          >
            <span className="add-bot-step-num">2</span>
            <span>{t("Channel Auth")}</span>
          </button>
        </div>

        <div className="add-bot-dialog-body">
          {catalogError ? (
            <div className="add-bot-alert error">
              {t("Failed to load channel catalog: {error}", { error: catalogError })}
            </div>
          ) : null}

          {step === 1 ? (
            <div className="add-bot-step-panel">
              <div className="add-bot-field-grid">
                <div className="add-bot-field">
                  <Label className="add-bot-label" htmlFor="add-bot-channel">
                    {t("Channel")}
                  </Label>
                  <Select
                    value={pluginId}
                    onValueChange={handlePluginChange}
                    disabled={catalogLoading || !entries || entries.length === 0}
                  >
                    <SelectTrigger className="add-bot-control" id="add-bot-channel">
                      <SelectValue placeholder={catalogLoading ? t("Loading...") : t("Choose channel")} />
                    </SelectTrigger>
                    <SelectContent>
                      {(entries ?? []).map((entry) => (
                        <SelectItem key={entry.id} value={entry.id}>
                          <span className="add-bot-select-option">
                            {entry.icon_data_url ? (
                              <img
                                alt=""
                                className="add-bot-select-icon"
                                height={18}
                                src={entry.icon_data_url}
                                width={18}
                              />
                            ) : (
                              <span className="add-bot-select-badge">
                                {channelInitials(entry)}
                              </span>
                            )}
                            <span className="add-bot-select-copy">
                              <span>{entry.display_name || entry.id}</span>
                              {entry.version ? (
                                <span className="add-bot-select-version">
                                  v{entry.version}
                                </span>
                              ) : null}
                            </span>
                          </span>
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>

                <div className="add-bot-field">
                  <Label className="add-bot-label" htmlFor="add-bot-account-id">
                    Account ID <span className="add-bot-required">*</span>
                  </Label>
                  <Input
                    className="add-bot-control add-bot-mono"
                    id="add-bot-account-id"
                    value={accountId}
                    onChange={(e) => handleAccountIdChange(e.target.value)}
                    placeholder={t("Unique identifier, for example product-ship")}
                  />
                </div>

                <div className="add-bot-field">
                  <Label className="add-bot-label" htmlFor="add-bot-name">
                    {t("Display name")} <span className="add-bot-optional">{t("Optional")}</span>
                  </Label>
                  <Input
                    className="add-bot-control"
                    id="add-bot-name"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    placeholder={t("Name shown in the sidebar")}
                  />
                </div>

                <div className="add-bot-field">
                  <Label className="add-bot-label" htmlFor="add-bot-agent">
                    Agent
                  </Label>
                  <Select
                    value={agentId}
                    onValueChange={setAgentId}
                    disabled={agentTargets.length === 0}
                  >
                    <SelectTrigger className="add-bot-control" id="add-bot-agent">
                      <SelectValue placeholder={t("Choose agent")} />
                    </SelectTrigger>
                    <SelectContent>
                      {agentTargets.map((target) => (
                        <SelectItem key={target.value} value={target.value}>
                          {target.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  {selectedAgentMissing ? (
                    <span className="add-bot-field-warning">
                      {t('Agent "{id}" no longer exists. Choose again.', { id: agentId })}
                    </span>
                  ) : null}
                </div>

                <div className="add-bot-field add-bot-field-wide">
                  <Label className="add-bot-label" htmlFor="add-bot-workspace">
                    {t("Working directory")} <span className="add-bot-optional">{t("Optional")}</span>
                  </Label>
                  <DirectoryInput
                    id="add-bot-workspace"
                    value={workspaceDir}
                    onChange={setWorkspaceDir}
                    placeholder={t("Use the main workspace by default")}
                  />
                </div>
              </div>
            </div>
          ) : selectedEntry ? (
            <div className="add-bot-step-panel">
              <AddBotAuthStep
                entry={selectedEntry}
                methods={selectedMethods}
                onAuthConfirmed={handleAuthConfirmed}
                onChange={setPluginConfig}
                value={pluginConfig}
              />
            </div>
          ) : !catalogLoading ? (
            <div className="add-bot-alert">
              {t("Choose a channel to continue.")}
            </div>
          ) : null}
        </div>

        <DialogFooter className="add-bot-dialog-footer">
          <div className="add-bot-footer-left">
            {saveError ? (
              <span className="add-bot-save-error">{saveError}</span>
            ) : (
              <span className="add-bot-step-meta">
                <b>{step}</b> / 2
              </span>
            )}
          </div>
          <Button
            className="add-bot-footer-button ghost"
            onClick={onClose}
            disabled={saving}
            variant="ghost"
          >
            {t("Cancel")}
          </Button>
          {step === 2 ? (
            <Button
              className="add-bot-footer-button secondary"
              onClick={() => setStep(1)}
              disabled={saving}
              variant="secondary"
            >
              <ChevronLeft aria-hidden size={13} strokeWidth={2} />
              {t("Back")}
            </Button>
          ) : null}
          {step === 1 ? (
            <Button
              className="add-bot-footer-button primary"
              onClick={goToAuthStep}
              disabled={!selectedEntry}
            >
              {t("Next")}
              <ChevronRight aria-hidden size={13} strokeWidth={2} />
            </Button>
          ) : (
            <Button
              className="add-bot-footer-button primary"
              onClick={() => void handleSave()}
              disabled={saving || !selectedEntry}
            >
              {saving ? t("Saving…") : t("Save")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function AddBotAuthStep(props: {
  entry: ChannelPluginCatalogEntry;
  methods: Array<{ kind: string }> | "empty" | null;
  value: Record<string, unknown>;
  onChange: (next: Record<string, unknown>) => void;
  onAuthConfirmed: (values: Record<string, unknown>) => void;
}) {
  const { t } = useI18n();
  const { entry, methods, value, onChange, onAuthConfirmed } = props;

  if (methods === "empty") {
    return (
      <div className="add-bot-alert error">
        {t("This plugin does not declare any config methods.")} (<code>config_methods</code> {t("is empty")}).
      </div>
    );
  }

  const resolvedMethods = methods ?? [{ kind: "form" }];
  const autoLoginMethods = resolvedMethods.filter(
    (method) => methodKind(method) === "auto_login",
  );
  const formMethods = resolvedMethods.filter(
    (method) => methodKind(method) === "form",
  );
  const hasAutoLogin = autoLoginMethods.length > 0;

  return (
    <div className="add-bot-auth-stack">
      {autoLoginMethods.map((_, idx) => (
        <section className="add-bot-auth-card auto" key={`auto-${idx}`}>
          <AuthFlowDriver
            badge={channelInitials(entry)}
            formState={value}
            iconDataUrl={entry.icon_data_url}
            onConfirmed={onAuthConfirmed}
            pluginId={entry.id}
            presentation="qr-card"
          />
        </section>
      ))}

      {formMethods.map((_, idx) => (
        <section
          className={`add-bot-auth-card manual ${hasAutoLogin ? "fallback" : ""}`}
          key={`form-${idx}`}
        >
          {hasAutoLogin ? (
            <div className="add-bot-manual-form">
              <JsonSchemaForm
                schema={entry.schema as Record<string, unknown>}
                value={value}
                onChange={onChange}
              />
            </div>
          ) : (
            <>
              <div className="add-bot-auth-card-header">
                <h4>{t("Manual setup")}</h4>
              </div>
              <JsonSchemaForm
                schema={entry.schema as Record<string, unknown>}
                value={value}
                onChange={onChange}
              />
            </>
          )}
        </section>
      ))}
    </div>
  );
}
