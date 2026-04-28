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

function compactAgentLabel(
  targets: AgentTargetOption[],
  value: string,
): string {
  return (
    targets.find((target) => target.value === value)?.label ||
    value ||
    "Default route"
  );
}

export function AddBotDialog(props: AddBotDialogProps) {
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
    setPluginConfig((current) => ({ ...current, ...values }));
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
  }, [generatedAccountId]);

  const goToAuthStep = useCallback(() => {
    if (!selectedEntry) {
      setSaveError("请先选择一个渠道");
      return;
    }
    if (!accountId.trim() && !selectedHasAutoLogin) {
      setSaveError("请填写 account id");
      return;
    }
    setSaveError(null);
    setStep(2);
  }, [accountId, selectedEntry, selectedHasAutoLogin]);

  const handleSave = async () => {
    if (!selectedEntry) {
      setSaveError("请先选择一个渠道");
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
      setSaveError("请填写 account id");
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

  const accountDisplay =
    accountId.trim() || configAccountIdOverride(pluginConfig) || "保存时确认";
  const selectedAgentLabel = compactAgentLabel(agentTargets, agentId);
  const workspaceDisplay = workspaceDir.trim() || "默认主工作区";

  return (
    <Dialog open={open} onOpenChange={(next) => !next && onClose()}>
      <DialogContent className="add-bot-dialog">
        <DialogHeader className="add-bot-dialog-header">
          <DialogTitle className="add-bot-dialog-title">添加渠道账号</DialogTitle>
          <DialogDescription className="add-bot-dialog-description">
            {step === 1
              ? "先确认要绑定的渠道与基础信息，下一步填写认证。"
              : selectedHasAutoLogin
                ? "用手机扫码绑定，凭证会自动写入。"
                : "填写该渠道需要的认证信息。"}
          </DialogDescription>
        </DialogHeader>

        <div className="add-bot-stepper" aria-label="添加 Bot 步骤">
          <button
            className={`add-bot-step ${step === 1 ? "current" : "done"}`}
            onClick={() => setStep(1)}
            type="button"
          >
            <span className="add-bot-step-num">
              {step === 2 ? <Check aria-hidden size={11} strokeWidth={2.4} /> : "1"}
            </span>
            <span>基础信息</span>
          </button>
          <span className={`add-bot-step-line ${step === 2 ? "filled" : ""}`} />
          <button
            className={`add-bot-step ${step === 2 ? "current" : ""}`}
            disabled={!selectedEntry}
            onClick={goToAuthStep}
            type="button"
          >
            <span className="add-bot-step-num">2</span>
            <span>渠道认证</span>
          </button>
        </div>

        <div className="add-bot-dialog-body">
          {catalogError ? (
            <div className="add-bot-alert error">
              获取渠道目录失败：{catalogError}
            </div>
          ) : null}

          {step === 1 ? (
            <div className="add-bot-step-panel">
              <div className="add-bot-field-grid">
                <div className="add-bot-field">
                  <Label className="add-bot-label" htmlFor="add-bot-channel">
                    渠道
                  </Label>
                  <Select
                    value={pluginId}
                    onValueChange={handlePluginChange}
                    disabled={catalogLoading || !entries || entries.length === 0}
                  >
                    <SelectTrigger className="add-bot-control" id="add-bot-channel">
                      <SelectValue placeholder={catalogLoading ? "加载中…" : "选择渠道"} />
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
                    placeholder="唯一标识，例如 product-ship"
                  />
                </div>

                <div className="add-bot-field">
                  <Label className="add-bot-label" htmlFor="add-bot-name">
                    显示名 <span className="add-bot-optional">可选</span>
                  </Label>
                  <Input
                    className="add-bot-control"
                    id="add-bot-name"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    placeholder="用于侧边栏的名称"
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
                      <SelectValue placeholder="选择 agent" />
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
                      当前 agent `{agentId}` 已不存在，请重新选择
                    </span>
                  ) : null}
                </div>

                <div className="add-bot-field add-bot-field-wide">
                  <Label className="add-bot-label" htmlFor="add-bot-workspace">
                    工作目录 <span className="add-bot-optional">可选</span>
                  </Label>
                  <DirectoryInput
                    id="add-bot-workspace"
                    value={workspaceDir}
                    onChange={setWorkspaceDir}
                    placeholder="默认沿用主工作区"
                  />
                </div>
              </div>
            </div>
          ) : selectedEntry ? (
            <div className="add-bot-step-panel">
              <div className="add-bot-channel-context">
                {selectedEntry.icon_data_url ? (
                  <img
                    alt=""
                    className="add-bot-channel-context-icon"
                    height={26}
                    src={selectedEntry.icon_data_url}
                    width={26}
                  />
                ) : (
                  <span className="add-bot-channel-context-badge">
                    {channelInitials(selectedEntry)}
                  </span>
                )}
                <div className="add-bot-channel-context-meta">
                  <div className="add-bot-channel-context-name">
                    {selectedEntry.display_name || selectedEntry.id} · {accountDisplay}
                  </div>
                  <div className="add-bot-channel-context-sub">
                    绑定到 {selectedAgentLabel} · {workspaceDisplay}
                  </div>
                </div>
                <button
                  className="add-bot-channel-context-edit"
                  onClick={() => setStep(1)}
                  type="button"
                >
                  编辑
                </button>
              </div>

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
              请选择渠道以继续。
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
            取消
          </Button>
          {step === 2 ? (
            <Button
              className="add-bot-footer-button secondary"
              onClick={() => setStep(1)}
              disabled={saving}
              variant="secondary"
            >
              <ChevronLeft aria-hidden size={13} strokeWidth={2} />
              上一步
            </Button>
          ) : null}
          {step === 1 ? (
            <Button
              className="add-bot-footer-button primary"
              onClick={goToAuthStep}
              disabled={!selectedEntry}
            >
              下一步
              <ChevronRight aria-hidden size={13} strokeWidth={2} />
            </Button>
          ) : (
            <Button
              className="add-bot-footer-button primary"
              onClick={() => void handleSave()}
              disabled={saving || !selectedEntry}
            >
              {saving ? "保存中…" : "保存"}
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
  const { entry, methods, value, onChange, onAuthConfirmed } = props;

  if (methods === "empty") {
    return (
      <div className="add-bot-alert error">
        该插件未声明任何配置方法（<code>config_methods</code> 为空）。
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
            <details className="add-bot-manual-details">
              <summary>
                <span>手动填写凭证</span>
                <span>扫码不可用时使用</span>
              </summary>
              <div className="add-bot-manual-form">
                <JsonSchemaForm
                  schema={entry.schema as Record<string, unknown>}
                  value={value}
                  onChange={onChange}
                />
              </div>
            </details>
          ) : (
            <>
              <div className="add-bot-auth-card-header">
                <h4>手动填写</h4>
                <p>保存前请确认这些字段来自官方渠道后台。</p>
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
