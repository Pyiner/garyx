/**
 * Schema-driven "Edit bot" dialog.
 *
 * This mirrors AddBotDialog's two-step structure:
 *   1. account metadata
 *   2. channel authentication/configuration
 *
 * The dialog is channel-blind. It reads the channel catalog, renders
 * the plugin JSON Schema, and uses the generic AuthFlowDriver for any
 * channel/plugin that advertises an auto-login flow.
 */
import { useEffect, useMemo, useState } from "react";
import { Check, ChevronLeft, ChevronRight, RefreshCw } from "lucide-react";

import type {
  ChannelPluginCatalogEntry,
  ChannelPluginConfigMethod,
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
import { JsonSchemaForm } from "../../channel-plugins/JsonSchemaForm";
import { DirectoryInput } from "../../components/DirectoryInput";
import { useChannelPluginCatalog } from "../../channel-plugins/useChannelPluginCatalog";

type AgentTargetOption = { value: string; label: string };
type EditBotStep = 1 | 2;

export type EditBotDialogContext = {
  kind: string;
  accountId: string;
  account: any;
  resolvedAgentId: string;
};

export type EditBotPatch = {
  nextAccountId?: string | null;
  name?: string | null;
  agentId?: string;
  workspaceDir?: string | null;
  config?: Record<string, unknown>;
};

type EditBotDialogProps = {
  open: boolean;
  context: EditBotDialogContext | null;
  agentTargets: AgentTargetOption[];
  saving?: boolean;
  onClose: () => void;
  onSave: (input: {
    kind: string;
    accountId: string;
    patch: EditBotPatch;
  }) => Promise<void> | void;
  onRemove: (input: {
    kind: string;
    accountId: string;
  }) => Promise<void> | void;
};

function accountToConfig(account: Record<string, unknown>): Record<string, unknown> {
  const nested = account.config;
  if (nested && typeof nested === "object" && !Array.isArray(nested)) {
    return { ...(nested as Record<string, unknown>) };
  }
  const strip = new Set([
    "enabled",
    "name",
    "agent_id",
    "workspace_dir",
    "owner_target",
    "groups",
  ]);
  const out: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(account)) {
    if (!strip.has(key)) out[key] = value;
  }
  return out;
}

function methodKind(method: { kind?: string }): string {
  return typeof method?.kind === "string" ? method.kind : "";
}

function resolveConfigMethods(
  entry: ChannelPluginCatalogEntry,
): ChannelPluginConfigMethod[] | "empty" {
  const raw = entry.config_methods;
  if (raw === undefined) return [{ kind: "form" }];
  if (raw.length === 0) return "empty";
  return raw;
}

function accountIdFromAuthValues(values: Record<string, unknown>): string | null {
  const value = values.account_id;
  return typeof value === "string" && value.trim() ? value.trim() : null;
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

export function EditBotDialog(props: EditBotDialogProps) {
  const { open, context, agentTargets, saving, onClose, onSave, onRemove } =
    props;
  const { entries, loading: catalogLoading } = useChannelPluginCatalog();

  const [step, setStep] = useState<EditBotStep>(1);
  const [name, setName] = useState("");
  const [agentId, setAgentId] = useState("");
  const [workspaceDir, setWorkspaceDir] = useState("");
  const [pluginConfig, setPluginConfig] = useState<Record<string, unknown>>({});
  const [showReauthorize, setShowReauthorize] = useState(false);
  const [reauthorizedAccountId, setReauthorizedAccountId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [removing, setRemoving] = useState(false);

  useEffect(() => {
    if (!open || !context) {
      setError(null);
      setRemoving(false);
      return;
    }
    const account = (context.account || {}) as Record<string, unknown>;
    setStep(1);
    setName(String(account.name || ""));
    setAgentId(context.resolvedAgentId || "");
    setWorkspaceDir(String(account.workspace_dir || ""));
    setPluginConfig(accountToConfig(account));
    setShowReauthorize(false);
    setReauthorizedAccountId(null);
    setError(null);
    setRemoving(false);
  }, [open, context]);

  const selectedEntry: ChannelPluginCatalogEntry | null = useMemo(() => {
    if (!entries || !context) return null;
    return (
      entries.find(
        (entry) =>
          entry.id === context.kind ||
          entry.id.toLowerCase() === String(context.kind).toLowerCase(),
      ) ?? null
    );
  }, [entries, context]);

  const selectedMethods = useMemo(() => {
    return selectedEntry ? resolveConfigMethods(selectedEntry) : null;
  }, [selectedEntry]);

  if (!context) {
    return (
      <Dialog
        open={open}
        onOpenChange={(next) => {
          if (!next) onClose();
        }}
      >
        <DialogContent className="add-bot-dialog" />
      </Dialog>
    );
  }

  const { kind, accountId } = context;
  const nextAccountId =
    reauthorizedAccountId && reauthorizedAccountId !== accountId
      ? reauthorizedAccountId
      : null;
  const accountDisplay = nextAccountId ? `${accountId} -> ${nextAccountId}` : accountId;
  const selectedAgentLabel = compactAgentLabel(agentTargets, agentId);
  const workspaceDisplay = workspaceDir.trim() || "默认主工作区";
  const selectedAgentMissing = Boolean(
    agentId &&
      agentTargets.length > 0 &&
      !agentTargets.find((target) => target.value === agentId),
  );

  function handleReauthorizeConfirmed(values: Record<string, unknown>) {
    setPluginConfig((current) => ({
      ...current,
      ...stripAuthIdentityHints(values, selectedEntry?.schema),
    }));
    const returnedAccountId = accountIdFromAuthValues(values);
    if (returnedAccountId) {
      setReauthorizedAccountId(returnedAccountId);
    }
  }

  function goToAuthStep() {
    if (!selectedEntry) {
      setError("渠道目录仍在加载");
      return;
    }
    setError(null);
    setStep(2);
  }

  async function handleSave() {
    if (!context) return;
    setError(null);
    try {
      const patch: EditBotPatch = {
        nextAccountId,
        name: name.trim() || null,
        workspaceDir: workspaceDir.trim() || null,
        config: pluginConfig,
      };
      if (agentId) patch.agentId = agentId;
      await onSave({ kind, accountId, patch });
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : "保存失败");
    }
  }

  async function handleRemove() {
    if (!context) return;
    if (!window.confirm(`确认删除 ${kind} 账号 "${name || accountId}"？`)) return;
    setRemoving(true);
    setError(null);
    try {
      await onRemove({ kind, accountId });
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : "删除失败");
    } finally {
      setRemoving(false);
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <DialogContent className="add-bot-dialog">
        <DialogHeader className="add-bot-dialog-header">
          <DialogTitle className="add-bot-dialog-title">编辑渠道账号</DialogTitle>
          <DialogDescription className="add-bot-dialog-description">
            {step === 1
              ? "先确认账号基础信息，下一步检查认证配置。"
              : "重新授权或手动检查该渠道的认证信息。"}
          </DialogDescription>
        </DialogHeader>

        <div className="add-bot-stepper" aria-label="编辑 Bot 步骤">
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
          {step === 1 ? (
            <div className="add-bot-step-panel">
              <div className="add-bot-field-grid">
                <div className="add-bot-field">
                  <Label className="add-bot-label" htmlFor="edit-bot-channel">
                    渠道
                  </Label>
                  <Select value={kind} disabled>
                    <SelectTrigger className="add-bot-control" id="edit-bot-channel">
                      <SelectValue placeholder={catalogLoading ? "加载中…" : kind} />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value={kind}>
                        {selectedEntry?.display_name || kind}
                      </SelectItem>
                    </SelectContent>
                  </Select>
                </div>

                <div className="add-bot-field">
                  <Label className="add-bot-label" htmlFor="edit-bot-account-id">
                    Account ID
                  </Label>
                  <Input
                    className="add-bot-control add-bot-mono"
                    disabled
                    id="edit-bot-account-id"
                    value={accountId}
                  />
                </div>

                <div className="add-bot-field">
                  <Label className="add-bot-label" htmlFor="edit-bot-name">
                    显示名 <span className="add-bot-optional">可选</span>
                  </Label>
                  <Input
                    className="add-bot-control"
                    id="edit-bot-name"
                    value={name}
                    onChange={(event) => setName(event.target.value)}
                    placeholder="用于侧边栏的名称"
                  />
                </div>

                <div className="add-bot-field">
                  <Label className="add-bot-label" htmlFor="edit-bot-agent">
                    Agent
                  </Label>
                  <Select
                    value={agentId}
                    onValueChange={setAgentId}
                    disabled={agentTargets.length === 0}
                  >
                    <SelectTrigger className="add-bot-control" id="edit-bot-agent">
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
                  <Label className="add-bot-label" htmlFor="edit-bot-workspace">
                    工作目录 <span className="add-bot-optional">可选</span>
                  </Label>
                  <DirectoryInput
                    id="edit-bot-workspace"
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

              <EditBotAuthStep
                entry={selectedEntry}
                methods={selectedMethods}
                onAuthConfirmed={handleReauthorizeConfirmed}
                onChange={setPluginConfig}
                showReauthorize={showReauthorize}
                value={pluginConfig}
                onToggleReauthorize={setShowReauthorize}
              />

              {nextAccountId ? (
                <div className="add-bot-alert">
                  保存后账号 ID 将更新为 <code>{nextAccountId}</code>
                </div>
              ) : null}
            </div>
          ) : catalogLoading ? (
            <div className="add-bot-alert">
              渠道目录加载中。
            </div>
          ) : (
            <div className="add-bot-alert">
              找不到该账号对应的渠道配置。
            </div>
          )}
        </div>

        <DialogFooter className="add-bot-dialog-footer">
          <div className="add-bot-footer-left">
            {error ? (
              <span className="add-bot-save-error">{error}</span>
            ) : (
              <>
                <span className="add-bot-step-meta">
                  <b>{step}</b> / 2
                </span>
                <button
                  className="add-bot-channel-context-edit"
                  disabled={removing || saving}
                  onClick={() => void handleRemove()}
                  type="button"
                >
                  {removing ? "删除中…" : "删除账号"}
                </button>
              </>
            )}
          </div>
          <Button
            className="add-bot-footer-button ghost"
            onClick={onClose}
            disabled={saving || removing}
            variant="ghost"
          >
            取消
          </Button>
          {step === 2 ? (
            <Button
              className="add-bot-footer-button secondary"
              onClick={() => setStep(1)}
              disabled={saving || removing}
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
              disabled={!selectedEntry || removing}
            >
              下一步
              <ChevronRight aria-hidden size={13} strokeWidth={2} />
            </Button>
          ) : (
            <Button
              className="add-bot-footer-button primary"
              onClick={() => void handleSave()}
              disabled={saving || removing || !selectedEntry}
            >
              {saving ? "保存中…" : "保存"}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function EditBotAuthStep(props: {
  entry: ChannelPluginCatalogEntry;
  methods: ChannelPluginConfigMethod[] | "empty" | null;
  value: Record<string, unknown>;
  showReauthorize: boolean;
  onChange: (next: Record<string, unknown>) => void;
  onAuthConfirmed: (values: Record<string, unknown>) => void;
  onToggleReauthorize: (next: boolean) => void;
}) {
  const {
    entry,
    methods,
    value,
    showReauthorize,
    onChange,
    onAuthConfirmed,
    onToggleReauthorize,
  } = props;

  if (methods === "empty") {
    return (
      <div className="add-bot-alert error">
        该插件未声明任何配置方法（<code>config_methods</code> 为空）。
      </div>
    );
  }

  const resolvedMethods: Array<{ kind?: string }> = methods ?? [{ kind: "form" }];
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
          {showReauthorize ? (
            <AuthFlowDriver
              badge={channelInitials(entry)}
              formState={value}
              iconDataUrl={entry.icon_data_url}
              onCancel={() => onToggleReauthorize(false)}
              onConfirmed={onAuthConfirmed}
              pluginId={entry.id}
              presentation="qr-card"
            />
          ) : (
            <>
              <div className="add-bot-auth-card-header">
                <h4>重新授权</h4>
                <p>刷新该账号的渠道凭证。</p>
              </div>
              <Button
                className="add-bot-footer-button primary"
                onClick={() => onToggleReauthorize(true)}
                type="button"
              >
                <RefreshCw aria-hidden size={13} strokeWidth={2} />
                开始授权
              </Button>
            </>
          )}
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
                <span>手动编辑凭证</span>
                <span>需要直接修正配置时使用</span>
              </summary>
              <div className="add-bot-manual-form">
                <JsonSchemaForm
                  schema={entry.schema as Record<string, unknown>}
                  secretInputType="text"
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
                secretInputType="text"
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
