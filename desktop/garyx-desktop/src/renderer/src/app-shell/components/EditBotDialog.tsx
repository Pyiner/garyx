/**
 * Schema-driven "Edit bot" dialog.
 *
 * Mirrors AddBotDialog's two-step structure:
 *   1. account metadata
 *   2. channel authentication/configuration
 */
import { useEffect, useMemo, useState } from "react";
import { Check, ChevronLeft, ChevronRight, RefreshCw } from "lucide-react";

import type {
  ChannelPluginCatalogEntry,
  ChannelPluginConfigMethod,
  DesktopWorkspace,
  DesktopWorkspaceMode,
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
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

import type { AgentTargetOption } from "../agent-options";
import { AgentOptionRow } from "./AgentOptionAvatar";
import { AuthFlowDriver } from "../../channel-plugins/AuthFlowDriver";
import { JsonSchemaForm } from "../../channel-plugins/JsonSchemaForm";
import { DirectoryInput } from "../../components/DirectoryInput";
import { useChannelPluginCatalog } from "../../channel-plugins/useChannelPluginCatalog";
import { useI18n } from "../../i18n";
import {
  channelAgentIdFromSelectValue,
  channelAgentSelectValue,
  explicitChannelAgentUnavailable,
  FOLLOW_GLOBAL_AGENT_SELECT_VALUE,
} from "../channel-agent-selection";

type EditBotStep = 1 | 2;

export type EditBotDialogContext = {
  kind: string;
  accountId: string;
  account: any;
  agentId: string | null;
};

export type EditBotPatch = {
  nextAccountId?: string | null;
  name?: string | null;
  agentId?: string | null;
  workspaceDir?: string | null;
  workspaceMode?: DesktopWorkspaceMode;
  config?: Record<string, unknown>;
};

type EditBotDialogProps = {
  open: boolean;
  context: EditBotDialogContext | null;
  agentTargets: AgentTargetOption[];
  effectiveDefaultAgentId?: string | null;
  workspaces?: DesktopWorkspace[];
  onAddWorkspace?: (path: string) => Promise<DesktopWorkspace | null>;
  saving?: boolean;
  onClose: () => void;
  onSave: (input: {
    kind: string;
    accountId: string;
    patch: EditBotPatch;
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
    if (!strip.has(key)) {
      out[key] = value;
    }
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

export function EditBotDialog(props: EditBotDialogProps) {
  const { t } = useI18n();
  const {
    open,
    context,
    agentTargets,
    effectiveDefaultAgentId = null,
    workspaces = [],
    onAddWorkspace,
    saving,
    onClose,
    onSave,
  } = props;
  const { entries, loading: catalogLoading } = useChannelPluginCatalog();

  const [step, setStep] = useState<EditBotStep>(1);
  const [name, setName] = useState("");
  const [agentId, setAgentId] = useState<string | null>(null);
  const [workspaceDir, setWorkspaceDir] = useState("");
  const [workspaceMode, setWorkspaceMode] = useState<DesktopWorkspaceMode>("local");
  const [pluginConfig, setPluginConfig] = useState<Record<string, unknown>>({});
  const [showReauthorize, setShowReauthorize] = useState(false);
  const [reauthorizedAccountId, setReauthorizedAccountId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open || !context) {
      setError(null);
      return;
    }

    const account = (context.account || {}) as Record<string, unknown>;
    setStep(1);
    setName(String(account.name || ""));
    setAgentId(context.agentId);
    setWorkspaceDir(String(account.workspace_dir || ""));
    setWorkspaceMode(account.workspace_mode === "worktree" ? "worktree" : "local");
    setPluginConfig(accountToConfig(account));
    setShowReauthorize(false);
    setReauthorizedAccountId(null);
    setError(null);
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
  const followGlobalLabel = useMemo(() => {
    const current = agentTargets.find((target) => target.value === effectiveDefaultAgentId);
    return current
      ? t("Follow global default (currently {agent})", { agent: current.label })
      : t("Follow global default (currently no enabled agent)");
  }, [agentTargets, effectiveDefaultAgentId, t]);

  if (!context) {
    return (
      <Dialog
        open={open}
        onOpenChange={(next) => {
          if (!next) onClose();
        }}
      >
        <DialogContent className="add-bot-dialog" size="form" />
      </Dialog>
    );
  }

  const { kind, accountId } = context;
  const nextAccountId =
    reauthorizedAccountId && reauthorizedAccountId !== accountId
      ? reauthorizedAccountId
      : null;
  const selectedAgentMissing = explicitChannelAgentUnavailable(agentTargets, agentId);

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
      setError(t("Channel catalog is still loading."));
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
        workspaceMode,
        config: pluginConfig,
      };
      patch.agentId = agentId;
      await onSave({ kind, accountId, patch });
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : t("Save failed."));
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <DialogContent className="add-bot-dialog" size="form">
        <DialogHeader className="add-bot-dialog-header">
          <DialogTitle className="add-bot-dialog-title">{t("Edit channel account")}</DialogTitle>
          <DialogDescription className="add-bot-dialog-description">
            {step === 1
              ? t("Confirm account basics first, then check authentication config.")
              : t("Reauthorize or manually review this channel's authentication info.")}
          </DialogDescription>
        </DialogHeader>

        <div className="add-bot-stepper" aria-label={t("Edit bot steps")}>
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
          {step === 1 ? (
            <div className="add-bot-step-panel">
              <div className="add-bot-field-grid">
                <div className="add-bot-field">
                  <Label className="add-bot-label" htmlFor="edit-bot-channel">
                    {t("Channel")}
                  </Label>
                  <Select value={kind} disabled>
                    <SelectTrigger className="add-bot-control" id="edit-bot-channel">
                      <SelectValue placeholder={catalogLoading ? t("Loading...") : kind} />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        <SelectItem value={kind}>
                          {selectedEntry?.display_name || kind}
                        </SelectItem>
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                </div>

                <div className="add-bot-field">
                  <Label className="add-bot-label" htmlFor="edit-bot-account-id">
                    {t("Account ID")}
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
                    {t("Display name")} <span className="add-bot-optional">{t("Optional")}</span>
                  </Label>
                  <Input
                    className="add-bot-control"
                    id="edit-bot-name"
                    value={name}
                    onChange={(event) => setName(event.target.value)}
                    placeholder={t("Name shown in the sidebar")}
                  />
                </div>

                <div className="add-bot-field">
                  <Label className="add-bot-label" htmlFor="edit-bot-agent">
                    {t("Agent")}
                  </Label>
                  <Select
                    value={channelAgentSelectValue(agentId)}
                    onValueChange={(value) => setAgentId(channelAgentIdFromSelectValue(value))}
                  >
                    <SelectTrigger className="add-bot-control" id="edit-bot-agent">
                      <SelectValue placeholder={t("Choose agent")} />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        <SelectLabel>{t("Agents")}</SelectLabel>
                        <SelectItem value={FOLLOW_GLOBAL_AGENT_SELECT_VALUE}>
                          {followGlobalLabel}
                        </SelectItem>
                        {selectedAgentMissing && agentId ? (
                          <SelectItem disabled value={agentId}>
                            {t('{id} (disabled or unavailable)', { id: agentId })}
                          </SelectItem>
                        ) : null}
                        {agentTargets.map((target) => (
                          <SelectItem key={target.value} value={target.value}>
                            <AgentOptionRow
                              option={target}
                            />
                          </SelectItem>
                        ))}
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                  {selectedAgentMissing ? (
                    <span className="add-bot-field-warning">
                      {t('Agent "{id}" is disabled or unavailable. Choose again or follow the global default.', { id: agentId })}
                    </span>
                  ) : null}
                </div>

                <div className="add-bot-field add-bot-field-wide">
                  <Label className="add-bot-label" htmlFor="edit-bot-workspace">
                    {t("Working directory")} <span className="add-bot-optional">{t("Optional")}</span>
                  </Label>
                  <DirectoryInput
                    id="edit-bot-workspace"
                    value={workspaceDir}
                    onChange={setWorkspaceDir}
                    onAddWorkspace={onAddWorkspace}
                    placeholder={t("Use the main workspace by default")}
                    workspaces={workspaces}
                  />
                </div>

                <div className="add-bot-field">
                  <Label className="add-bot-label" htmlFor="edit-bot-workspace-mode">
                    {t("Workspace mode")}
                  </Label>
                  <Select
                    value={workspaceMode}
                    onValueChange={(value) => setWorkspaceMode(value as DesktopWorkspaceMode)}
                  >
                    <SelectTrigger className="add-bot-control" id="edit-bot-workspace-mode">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="local">{t("Local")}</SelectItem>
                      <SelectItem value="worktree">{t("Worktree")}</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
              </div>
            </div>
          ) : selectedEntry ? (
            <div className="add-bot-step-panel">
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
                  {t("After saving, account ID changes to {id}", { id: nextAccountId })}
                </div>
              ) : null}
            </div>
          ) : catalogLoading ? (
            <div className="add-bot-alert">{t("Channel catalog is loading.")}</div>
          ) : (
            <div className="add-bot-alert">{t("Channel config for this account was not found.")}</div>
          )}
        </div>

        <DialogFooter className="add-bot-dialog-footer">
          <div className="add-bot-footer-left">
            {error ? (
              <span className="add-bot-save-error">{error}</span>
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

function EditBotAuthStep(props: {
  entry: ChannelPluginCatalogEntry;
  methods: ChannelPluginConfigMethod[] | "empty" | null;
  value: Record<string, unknown>;
  showReauthorize: boolean;
  onChange: (next: Record<string, unknown>) => void;
  onAuthConfirmed: (values: Record<string, unknown>) => void;
  onToggleReauthorize: (next: boolean) => void;
}) {
  const { t } = useI18n();
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
        {t("This plugin does not declare any config methods.")} (<code>config_methods</code> {t("is empty")}).
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
            <Button
              className="add-bot-footer-button primary add-bot-reauthorize-button"
              onClick={() => onToggleReauthorize(true)}
              type="button"
            >
              <RefreshCw aria-hidden size={13} strokeWidth={2} />
              {t("Reauthorize")}
            </Button>
          )}
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
                secretInputType="text"
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
