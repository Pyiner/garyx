import { useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { Pencil, Plus } from 'lucide-react';

import type {
  DesktopApiProviderType,
  DesktopClaudeAuthSession,
  DesktopClaudeCodeAccount,
  DesktopClaudeCodeAccounts,
  DesktopCodingUsage,
  DesktopProviderModels,
  DesktopProviderUsage,
  DesktopModelUsage,
  DesktopUsageWindow,
} from '@shared/contracts';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';
import { ProviderAgentIcon } from '../app-shell/components/ProviderAgentIcon';
import { useI18n, type Translate } from '../i18n';
import { shouldRequestProviderModelCatalog } from '../provider-model-catalog';
import {
  clampUsagePercent,
  formatUsageDuration,
  formatUsagePercent,
  usageLevelForRemainingPercent,
  usageResetText,
} from '../provider-usage';
import {
  MODEL_PROVIDER_ROWS,
  applyProviderCatalogDefaults,
  applyProviderConfigDraftToGatewayConfig,
  emptyModelProviderConfigDraft,
  fixedModelProviderRow,
  highestReasoningEffort,
  modelProviderDraftFromState,
  normalizeClaudeCliMode,
  providerAgentConfig,
  providerModelOptionsWithCurrent,
  reasoningEffortOptionsForModel,
  sanitizedServiceTier,
  serviceTierOptionsForModel,
  type FixedModelProviderKey,
  type FixedModelProviderRow,
  type ModelProviderConfigDraft,
} from './provider-settings-model.ts';
import { classNames } from './shared';
import { SettingsControlRow } from './shared-components';

type DraftMutator = (mutator: (nextConfig: any) => void) => void;
type GatewaySettingsSaveOptions = {
  silent?: boolean;
  refreshDesktopState?: 'await' | 'background' | 'skip';
};

type ProviderAuthState = 'ready' | 'error';

type ProviderRowDetails = {
  status: string;
  auth: string;
  authState: ProviderAuthState;
  authTooltip?: string;
  model: string;
  reasoning: string;
  serviceTier: string;
};

const PROVIDER_DEFAULT_MODEL_VALUE = '__provider_default_model__';

const PROVIDER_DEFAULT_REASONING_VALUE = '__provider_default_reasoning__';

const PROVIDER_DEFAULT_SERVICE_TIER_VALUE = '__provider_default_service_tier__';

const PROVIDER_MODEL_TYPES = Array.from(
  new Set(MODEL_PROVIDER_ROWS.map((row) => row.providerType)),
);

function isHttpUrl(value: string): boolean {
  try {
    const parsed = new URL(value);
    return parsed.protocol === 'http:' || parsed.protocol === 'https:';
  } catch {
    return false;
  }
}

async function openExternalAuthUrl(value: string): Promise<void> {
  const url = value.trim();
  if (!isHttpUrl(url)) {
    throw new Error('Claude returned an invalid authorization URL.');
  }
  await window.garyxDesktop.openExternalUrl({ url });
}

function claudeCliModeLabel(value: 'cctty' | 'native', t: Translate): string {
  return value === 'native' ? t('Native Claude CLI') : t('cctty TTY wrapper');
}

type ProviderSettingsPanelProps = {
  gatewayDraft?: any;
  onMutateGatewayDraft?: DraftMutator;
  onSaveGatewaySettings?: (options?: GatewaySettingsSaveOptions) => Promise<boolean>;
};

export function ProviderSettingsPanel({
  gatewayDraft,
  onMutateGatewayDraft = () => {},
  onSaveGatewaySettings = async () => true,
}: ProviderSettingsPanelProps) {
  const { t } = useI18n();
  const [providerConfigKey, setProviderConfigKey] = useState<FixedModelProviderKey | null>(null);
  const [providerConfigDraft, setProviderConfigDraft] = useState<ModelProviderConfigDraft>(() =>
    emptyModelProviderConfigDraft(),
  );
  const [providerConfigSaving, setProviderConfigSaving] = useState(false);
  const [providerModelsByType, setProviderModelsByType] = useState<
    Partial<Record<DesktopApiProviderType, DesktopProviderModels>>
  >({});
  const [providerModelsLoading, setProviderModelsLoading] = useState<
    Partial<Record<DesktopApiProviderType, boolean>>
  >({});
  const providerModelRequestsRef = useRef<
    Partial<Record<DesktopApiProviderType, Promise<void>>>
  >({});
  const providerModelAttemptedRef = useRef<
    Partial<Record<DesktopApiProviderType, boolean>>
  >({});
  const [codingUsage, setCodingUsage] = useState<DesktopCodingUsage | null>(null);
  const [codingUsageLoading, setCodingUsageLoading] = useState(false);
  const [codingUsageError, setCodingUsageError] = useState<string | null>(null);
  const [claudeAccounts, setClaudeAccounts] = useState<DesktopClaudeCodeAccounts | null>(null);
  const [claudeAccountsLoading, setClaudeAccountsLoading] = useState(false);
  const [claudeAccountsError, setClaudeAccountsError] = useState<string | null>(null);
  const [accountSwitcherOpen, setAccountSwitcherOpen] = useState(false);
  const [accountMutationId, setAccountMutationId] = useState<string | null>(null);
  const [loginDialog, setLoginDialog] = useState<{
    mode: 'new' | 'reauth';
    account: DesktopClaudeCodeAccount | null;
  } | null>(null);
  const [loginAccountName, setLoginAccountName] = useState('');
  const [loginSession, setLoginSession] = useState<DesktopClaudeAuthSession | null>(null);
  const [loginCode, setLoginCode] = useState('');
  const [loginBusy, setLoginBusy] = useState(false);
  const [loginError, setLoginError] = useState<string | null>(null);
  const [renameAccount, setRenameAccount] = useState<DesktopClaudeCodeAccount | null>(null);
  const [renameValue, setRenameValue] = useState('');
  const [deleteAccount, setDeleteAccount] = useState<DesktopClaudeCodeAccount | null>(null);
  const openedLoginIdsRef = useRef(new Set<string>());
  const loginFlowIdRef = useRef(0);
  const loginCodeInputRef = useRef<HTMLInputElement | null>(null);
  const providerConfigRow = providerConfigKey ? fixedModelProviderRow(providerConfigKey) : null;
  const activeProviderModels = providerConfigRow
    ? providerModelsByType[providerConfigRow.providerType] || null
    : null;
  const activeProviderModelsLoading = providerConfigRow
    ? providerModelsLoading[providerConfigRow.providerType] === true
    : false;
  const activeProviderModelOptions = providerModelOptionsWithCurrent(
    activeProviderModels,
    providerConfigDraft.model,
  );
  const activeReasoningOptions = reasoningEffortOptionsForModel(
    activeProviderModels,
    providerConfigDraft.model,
    providerConfigDraft.modelReasoningEffort,
  );
  const activeServiceTierOptions = serviceTierOptionsForModel(
    activeProviderModels,
    providerConfigDraft.model,
    providerConfigDraft.modelServiceTier,
  );
  const activeSupportsServiceTier =
    (activeProviderModels?.supportsServiceTierSelection === true
      || providerConfigDraft.modelServiceTier.trim().length > 0)
    && activeServiceTierOptions.length > 0;
  const codingUsageByProviderId = useMemo(() => {
    const map: Record<string, DesktopProviderUsage> = {};
    for (const provider of codingUsage?.providers || []) {
      map[provider.id] = provider;
    }
    return map;
  }, [codingUsage]);

  function ensureProviderModels(
    providerType: DesktopApiProviderType,
    options: { retry?: boolean } = {},
  ) {
    if (!shouldRequestProviderModelCatalog({
      catalogs: providerModelsByType,
      requests: providerModelRequestsRef.current,
      attempted: providerModelAttemptedRef.current,
    }, providerType, options)) {
      return;
    }
    providerModelAttemptedRef.current[providerType] = true;
    setProviderModelsLoading((current) => ({
      ...current,
      [providerType]: true,
    }));
    const request = window.garyxDesktop.listProviderModels(providerType).then((models) => {
      setProviderModelsByType((current) => ({
        ...current,
        [providerType]: models,
      }));
    }).catch(() => {
      // The dialog keeps the raw model input fallback if catalog loading fails.
    }).finally(() => {
      delete providerModelRequestsRef.current[providerType];
      setProviderModelsLoading((current) => ({
        ...current,
        [providerType]: false,
      }));
    });
    providerModelRequestsRef.current[providerType] = request;
  }

  async function refreshCodingUsage() {
    setCodingUsageLoading(true);
    setCodingUsageError(null);
    try {
      const usage = await window.garyxDesktop.getCodingUsage();
      setCodingUsage(usage);
    } catch (error) {
      setCodingUsageError(error instanceof Error ? error.message : t('Failed to load usage.'));
    } finally {
      setCodingUsageLoading(false);
    }
  }

  async function refreshClaudeAccounts() {
    setClaudeAccountsLoading(true);
    setClaudeAccountsError(null);
    try {
      setClaudeAccounts(await window.garyxDesktop.listClaudeCodeAccounts());
    } catch (error) {
      setClaudeAccountsError(error instanceof Error ? error.message : t('Failed to load accounts.'));
    } finally {
      setClaudeAccountsLoading(false);
    }
  }

  useEffect(() => {
    for (const providerType of PROVIDER_MODEL_TYPES) {
      ensureProviderModels(providerType);
    }
    // Prefetch once when the provider panel mounts so Configure dropdowns are
    // backed by the gateway catalog instead of the current-value fallback.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!providerConfigRow) {
      return;
    }
    ensureProviderModels(providerConfigRow.providerType, { retry: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [providerConfigRow?.providerType]);

  useEffect(() => {
    void refreshCodingUsage();
    void refreshClaudeAccounts();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [t]);

  useEffect(() => {
    if (!loginSession || loginSession.status === 'succeeded' || loginSession.status === 'failed') {
      return;
    }
    const flowId = loginFlowIdRef.current;
    const timer = window.setInterval(() => {
      void window.garyxDesktop.getClaudeCodeAuth({ loginId: loginSession.loginId })
        .then((session) => {
          if (loginFlowIdRef.current === flowId) setLoginSession(session);
        })
        .catch((error) => {
          if (loginFlowIdRef.current === flowId) {
            setLoginError(error instanceof Error ? error.message : t('Failed to check sign-in.'));
          }
        });
    }, 800);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loginSession?.loginId, loginSession?.status]);

  useEffect(() => {
    if (loginSession?.status === 'failed') {
      setLoginError(loginSession.error || t('Claude sign-in failed.'));
      return;
    }
    if (loginSession?.status !== 'succeeded') {
      return;
    }
    const flowId = loginFlowIdRef.current;
    void Promise.all([refreshClaudeAccounts(), refreshCodingUsage()]).then(() => {
      if (loginFlowIdRef.current !== flowId) return;
      setLoginDialog(null);
      setLoginSession(null);
      setAccountSwitcherOpen(true);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loginSession?.loginId, loginSession?.status]);

  useEffect(() => {
    if (!loginSession?.authorizationUrl || openedLoginIdsRef.current.has(loginSession.loginId)) {
      return;
    }
    openedLoginIdsRef.current.add(loginSession.loginId);
    void openExternalAuthUrl(loginSession.authorizationUrl).catch((error) => {
      setLoginError(error instanceof Error ? error.message : t('Could not open the browser. Use the link below.'));
    });
  }, [loginSession?.authorizationUrl, loginSession?.loginId, t]);

  useEffect(() => {
    if (!loginSession || loginSession.status === 'starting') {
      return;
    }
    loginCodeInputRef.current?.focus();
    const refocus = () => loginCodeInputRef.current?.focus();
    window.addEventListener('focus', refocus);
    return () => window.removeEventListener('focus', refocus);
  }, [loginSession?.loginId, loginSession?.status]);

  useEffect(() => {
    if (!providerConfigRow || !activeProviderModels) {
      return;
    }
    setProviderConfigDraft((current) => {
      if (current.key !== providerConfigRow.key) {
        return current;
      }
      return applyProviderCatalogDefaults(current, activeProviderModels);
    });
  }, [providerConfigRow?.key, activeProviderModels]);
  function providerRowDetails(row: FixedModelProviderRow): ProviderRowDetails {
    const runtimeConfig = providerAgentConfig(gatewayDraft, row.key);
    const configuredDefaultModel = String(runtimeConfig.default_model || '').trim();
    const configuredReasoning = String(runtimeConfig.model_reasoning_effort || '').trim();
    const configuredServiceTier = String(runtimeConfig.model_service_tier || '').trim();
    const usage = row.usageProviderId ? codingUsageByProviderId[row.usageProviderId] : null;
    const usageAuthError = usage && !usage.available && usage.error ? usage.error : '';
    const finalize = (details: ProviderRowDetails): ProviderRowDetails => {
      if (!usageAuthError) {
        return details;
      }
      return {
        ...details,
        auth: t('Error'),
        authState: 'error',
        authTooltip: usageAuthError,
      };
    };
    if (row.key === 'claude_code') {
      const mode = normalizeClaudeCliMode(providerAgentConfig(gatewayDraft, 'claude_code').claude_cli_mode);
      return finalize({
        status: t('Default'),
        auth: claudeCliModeLabel(mode, t),
        authState: 'ready',
        model: configuredDefaultModel || row.defaultModel,
        reasoning: configuredReasoning,
        serviceTier: configuredServiceTier,
      });
    }
    if (row.key === 'codex_app_server') {
      return finalize({
        status: t('Default'),
        auth: t('CLI'),
        authState: 'ready',
        model: configuredDefaultModel || row.defaultModel,
        reasoning: configuredReasoning,
        serviceTier: configuredServiceTier,
      });
    }
    if (row.key === 'antigravity') {
      return finalize({
        status: t('Default'),
        auth: t('CLI'),
        authState: 'ready',
        model: configuredDefaultModel || row.defaultModel,
        reasoning: configuredReasoning,
        serviceTier: configuredServiceTier,
      });
    }
    if (row.key === 'traex') {
      // TRAE CLI authenticates via `traex login`; no desktop-managed auth.
      return finalize({
        status: t('Default'),
        auth: t('CLI'),
        authState: 'ready',
        model: configuredDefaultModel || row.defaultModel,
        reasoning: configuredReasoning,
        serviceTier: configuredServiceTier,
      });
    }
    return finalize({
      status: t('Default'),
      auth: t('CLI'),
      authState: 'ready',
      model: configuredDefaultModel || row.defaultModel,
      reasoning: configuredReasoning,
      serviceTier: configuredServiceTier,
    });
  }

  function providerModelLabel(value: string): string {
    return value === '(provider default)' ? t('Provider default') : value;
  }

  function renderProviderDefaultChips(details: ProviderRowDetails): ReactNode {
    const chips = [
      {
        key: 'model',
        label: providerModelLabel(details.model),
        title: t('Model: {value}', { value: providerModelLabel(details.model) }),
      },
      {
        key: 'reasoning',
        label: details.reasoning || t('Reasoning default'),
        title: t('Reasoning: {value}', { value: details.reasoning || t('Provider default') }),
      },
    ];
    const tier = details.serviceTier;
    if (tier) {
      chips.push({
        key: 'tier',
        label: tier,
        title: t('Tier: {value}', { value: tier }),
      });
    }
    const tooltip = chips.map((chip) => chip.title).join('\n');
    return (
      <div className="provider-config-default-cell" title={tooltip}>
        {chips.map((chip) => (
          <span className="provider-config-default-chip" key={chip.key}>
            {chip.label}
          </span>
        ))}
      </div>
    );
  }

  function usageUpdatedText(): string | null {
    const timestamp = Date.parse(codingUsage?.refreshedAt || '');
    if (!Number.isFinite(timestamp)) {
      return null;
    }
    const elapsedSeconds = Math.max(0, Math.floor((Date.now() - timestamp) / 1000));
    return t('updated {age} ago', { age: formatUsageDuration(elapsedSeconds) });
  }

  function usageWindowCaption(
    window: DesktopUsageWindow,
    fallback: string,
  ): string {
    return usageResetText(window.resetsAt, window.resetAfterSeconds, fallback);
  }

  function modelUsageCaption(model: DesktopModelUsage): string {
    return model.description?.trim()
      || usageResetText(model.resetsAt, model.resetAfterSeconds, t('reset time unknown'));
  }

  function providerUsageWindows(
    usage: DesktopProviderUsage,
  ): Array<{ key: string; label: string; value: DesktopUsageWindow; fallback: string }> {
    const windows: Array<{
      key: string;
      label: string;
      value: DesktopUsageWindow;
      fallback: string;
    }> = [];
    if (usage.session) {
      windows.push({
        key: 'session',
        label: t('Session'),
        value: usage.session,
        fallback: t('session window'),
      });
    }
    if (usage.weekly) {
      windows.push({
        key: 'weekly',
        label: t('Weekly'),
        value: usage.weekly,
        fallback: t('weekly window'),
      });
    }
    for (const limit of usage.scopedLimits) {
      windows.push({
        key: `scoped:${limit.id}`,
        label: limit.name,
        value: limit.window,
        fallback: limit.kind.includes('weekly') ? t('weekly window') : t('usage window'),
      });
    }
    return windows;
  }

  function sortedModelsByRemaining(usage: DesktopProviderUsage): DesktopModelUsage[] {
    return [...usage.models].sort((left, right) =>
      clampUsagePercent(left.remainingPercent) - clampUsagePercent(right.remainingPercent),
    );
  }

  function renderUsagePills(usage: DesktopProviderUsage, compact = false): ReactNode {
    const updated = usageUpdatedText();
    if (!usage.plan && !usage.stale && (!updated || compact)) {
      return null;
    }
    return (
      <div className={classNames('provider-usage-pills', compact && 'compact')}>
        {usage.plan ? <span className="provider-usage-pill">{usage.plan}</span> : null}
        {usage.stale ? <span className="provider-usage-pill stale">{t('stale')}</span> : null}
        {!compact && updated ? <span className="provider-usage-updated">{updated}</span> : null}
      </div>
    );
  }

  function renderUsageMeter(
    label: string,
    remainingPercent: number,
    caption: string,
    options?: {
      compact?: boolean;
      stale?: boolean;
      title?: string;
    },
  ): ReactNode {
    const percent = clampUsagePercent(remainingPercent);
    const level = usageLevelForRemainingPercent(percent);
    return (
      <div
        className={classNames('provider-usage-meter', options?.compact && 'compact')}
        data-level={level}
        data-stale={options?.stale ? 'true' : undefined}
        title={options?.title}
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

  function selectedClaudeAccount(): DesktopClaudeCodeAccount | null {
    return claudeAccounts?.accounts.find((account) => account.selected) || null;
  }

  async function handleSelectClaudeAccount(account: DesktopClaudeCodeAccount) {
    const mutationKey = account.id || 'system';
    setAccountMutationId(mutationKey);
    setClaudeAccountsError(null);
    try {
      await window.garyxDesktop.selectClaudeCodeAccount({ accountId: account.id });
      await Promise.all([refreshClaudeAccounts(), refreshCodingUsage()]);
      setAccountSwitcherOpen(false);
    } catch (error) {
      setClaudeAccountsError(error instanceof Error ? error.message : t('Failed to switch account.'));
    } finally {
      setAccountMutationId(null);
    }
  }

  function openLoginDialog(mode: 'new' | 'reauth', account: DesktopClaudeCodeAccount | null = null) {
    loginFlowIdRef.current += 1;
    setAccountSwitcherOpen(false);
    setLoginDialog({ mode, account });
    setLoginAccountName(mode === 'new' ? '' : account?.name || '');
    setLoginSession(null);
    setLoginCode('');
    setLoginError(null);
    setLoginBusy(false);
  }

  function closeLoginDialog() {
    const session = loginSession;
    loginFlowIdRef.current += 1;
    setLoginDialog(null);
    setLoginSession(null);
    setLoginCode('');
    setLoginError(null);
    setLoginBusy(false);
    if (session && session.status !== 'succeeded' && session.status !== 'failed') {
      void window.garyxDesktop.cancelClaudeCodeAuth({ loginId: session.loginId }).catch(() => {
        // The dialog is already closed; the gateway also cleans abandoned auth
        // sessions when their CLI exits, so cancellation remains best effort here.
      });
    }
  }

  async function handleStartClaudeLogin() {
    if (!loginDialog || loginBusy) return;
    const name = loginAccountName.trim();
    if (loginDialog.mode === 'new' && !name) {
      setLoginError(t('Give this account a name first.'));
      return;
    }
    setLoginBusy(true);
    setLoginError(null);
    const flowId = loginFlowIdRef.current;
    try {
      const session = await window.garyxDesktop.startClaudeCodeAuth({
        mode: 'claudeai',
        managedAccountName: loginDialog.mode === 'new' ? name : null,
        accountId: loginDialog.mode === 'reauth' ? loginDialog.account?.id || null : null,
      });
      if (loginFlowIdRef.current !== flowId) {
        void window.garyxDesktop.cancelClaudeCodeAuth({ loginId: session.loginId }).catch(() => {});
        return;
      }
      setLoginSession(session);
      if (session.status === 'failed') {
        setLoginError(session.error || t('Claude sign-in failed.'));
      }
    } catch (error) {
      if (loginFlowIdRef.current === flowId) {
        setLoginError(error instanceof Error ? error.message : t('Could not start Claude sign-in.'));
      }
    } finally {
      if (loginFlowIdRef.current === flowId) setLoginBusy(false);
    }
  }

  async function handleSubmitClaudeCode() {
    if (!loginSession || !loginCode.trim() || loginBusy) return;
    setLoginBusy(true);
    setLoginError(null);
    const flowId = loginFlowIdRef.current;
    try {
      const session = await window.garyxDesktop.submitClaudeCodeAuth({
        loginId: loginSession.loginId,
        code: loginCode.trim(),
      });
      if (loginFlowIdRef.current === flowId) setLoginSession(session);
    } catch (error) {
      if (loginFlowIdRef.current === flowId) {
        setLoginError(error instanceof Error ? error.message : t('Could not submit the code.'));
      }
    } finally {
      if (loginFlowIdRef.current === flowId) setLoginBusy(false);
    }
  }

  async function handleRenameClaudeAccount() {
    if (!renameAccount?.id || !renameValue.trim() || accountMutationId) return;
    setAccountMutationId(renameAccount.id);
    try {
      await window.garyxDesktop.renameClaudeCodeAccount({
        accountId: renameAccount.id,
        name: renameValue.trim(),
      });
      await refreshClaudeAccounts();
      setRenameAccount(null);
    } catch (error) {
      setClaudeAccountsError(error instanceof Error ? error.message : t('Failed to rename account.'));
    } finally {
      setAccountMutationId(null);
    }
  }

  async function handleDeleteClaudeAccount() {
    if (!deleteAccount?.id || accountMutationId) return;
    setAccountMutationId(deleteAccount.id);
    try {
      await window.garyxDesktop.deleteClaudeCodeAccount({ accountId: deleteAccount.id });
      await Promise.all([refreshClaudeAccounts(), refreshCodingUsage()]);
      setDeleteAccount(null);
    } catch (error) {
      setClaudeAccountsError(error instanceof Error ? error.message : t('Failed to delete account.'));
    } finally {
      setAccountMutationId(null);
    }
  }

  function renderProviderUsageSummary(usage: DesktopProviderUsage | null): ReactNode {
    if (!usage) {
      return (
        <span className="provider-card-empty">
          {codingUsageLoading ? t('Loading quota…') : codingUsageError || t('Quota unavailable')}
        </span>
      );
    }
    if (!usage.available) {
      return <span className="provider-card-empty">{usage.error || t('Quota unavailable')}</span>;
    }
    const windows = providerUsageWindows(usage);
    const models = sortedModelsByRemaining(usage);
    if (windows.length === 0 && models.length === 0) {
      return <span className="provider-card-empty">{t('No quota data')}</span>;
    }
    return (
      <div className="provider-summary-meter-grid">
        {windows.map((entry) => (
          <div className="provider-summary-meter" key={entry.key}>
            {renderUsageMeter(
              entry.label,
              entry.value.remainingPercent,
              usageWindowCaption(entry.value, entry.fallback),
              { compact: true, stale: usage.stale },
            )}
          </div>
        ))}
        {models.slice(0, 4).map((model) => (
          <div className="provider-summary-meter" key={model.id || model.name}>
            {renderUsageMeter(model.name, model.remainingPercent, modelUsageCaption(model), {
              compact: true,
              stale: usage.stale,
            })}
          </div>
        ))}
      </div>
    );
  }

  function renderProviderCard(row: FixedModelProviderRow): ReactNode {
    const details = providerRowDetails(row);
    const claudeAccount = row.key === 'claude_code' ? selectedClaudeAccount() : null;
    const usage = row.usageProviderId
      ? codingUsageByProviderId[row.usageProviderId]
        || (row.key === 'claude_code' ? claudeAccount?.usage : null)
        || null
      : null;
    const providerDescription = row.key === 'claude_code'
      ? t('Claude Agent SDK')
      : details.auth;
    const claudeSelectionUnavailable = Boolean(
      row.key === 'claude_code' && claudeAccounts?.activeAccountId && !claudeAccount,
    );
    const claudeAccountName = claudeSelectionUnavailable
      ? t('Account unavailable')
      : claudeAccount?.name
        || (claudeAccountsLoading ? t('Loading…') : t('System default'));
    const claudeAccountDetail = claudeSelectionUnavailable
      ? t('Choose another account before starting Claude Code.')
      : claudeAccount?.email
        || claudeAccount?.organization
        || t('Uses this Mac’s Claude Code login');
    const quotaDescription = usage?.plan
      ? usage.stale
        ? `${usage.plan} · ${t('stale')}`
        : usage.plan
      : usage?.stale
        ? t('stale')
        : usageUpdatedText() || undefined;
    return (
      <section className="codex-section provider-section" key={row.key}>
        <div className="codex-section-header provider-section-header">
          <div className="provider-section-identity">
            <span className="provider-section-icon" aria-hidden>
              <ProviderAgentIcon agentId={row.agentId} providerType={row.providerType} size={19} />
            </span>
            <div className="provider-section-title-group">
              <span className="codex-section-title">{row.label}</span>
              <span className="provider-section-description">{providerDescription}</span>
            </div>
          </div>
          <button
            className="codex-section-action"
            onClick={() => openProviderConfigDialog(row.key)}
            type="button"
          >
            <Pencil aria-hidden size={13} strokeWidth={1.8} />
            {t('Edit')}
          </button>
        </div>

        <div className="codex-list-card provider-section-rows">
          {row.key === 'claude_code' ? (
            <SettingsControlRow
              className="provider-account-row"
              control={(
                <div className="provider-account-actions">
                  <Button
                    aria-label={t('Switch account')}
                    onClick={() => setAccountSwitcherOpen(true)}
                    size="sm"
                    type="button"
                    variant="outline"
                  >
                    {t('Switch account')}
                  </Button>
                  <Button
                    aria-label={t('Add account')}
                    onClick={() => openLoginDialog('new')}
                    size="sm"
                    type="button"
                  >
                    <Plus aria-hidden size={13} strokeWidth={2} />
                    {t('Add account')}
                  </Button>
                </div>
              )}
              description={`${claudeAccountName} · ${claudeAccountDetail}`}
              label={t('Current account')}
            />
          ) : null}
          {row.usageProviderId ? (
            <SettingsControlRow
              className="provider-quota-row"
              control={renderProviderUsageSummary(usage)}
              description={quotaDescription}
              label={t('Quota remaining')}
            />
          ) : null}
          <SettingsControlRow
            className="provider-default-row"
            control={renderProviderDefaultChips(details)}
            label={t('Default model')}
          />
        </div>
      </section>
    );
  }

  function renderProviderCards(): ReactNode {
    return <>{MODEL_PROVIDER_ROWS.map(renderProviderCard)}</>;
  }

  function renderProviderConfigUsageSection(row: FixedModelProviderRow): ReactNode {
    if (!row.usageProviderId) {
      return null;
    }
    const usage = codingUsageByProviderId[row.usageProviderId];
    let content: ReactNode;
    if (!usage) {
      content = codingUsageLoading
        ? <span className="provider-usage-muted">{t('Loading')}</span>
        : (
          <span className="provider-usage-muted" title={codingUsageError || undefined}>
            {codingUsageError ? t('Unavailable') : t('No quota data')}
          </span>
        );
    } else if (!usage.available) {
      content = (
        <span className="provider-usage-muted" title={usage.error || undefined}>
          {t('Unavailable')}
        </span>
      );
    } else {
      const windows = providerUsageWindows(usage);
      const models = sortedModelsByRemaining(usage);
      content = windows.length > 0 || models.length > 0 ? (
        <>
          {windows.length > 0 ? (
            <div className="provider-usage-window-grid">
              {windows.map((entry) => (
                <div className="provider-usage-window-card" key={entry.key}>
                  {renderUsageMeter(
                    entry.label,
                    entry.value.remainingPercent,
                    usageWindowCaption(entry.value, entry.fallback),
                    { stale: usage.stale },
                  )}
                </div>
              ))}
            </div>
          ) : null}
          {models.length > 0 ? (
            <div className="provider-usage-models">
              {models.map((model) => (
                <div className="provider-usage-model-row" key={model.id || model.name}>
                  {renderUsageMeter(model.name, model.remainingPercent, modelUsageCaption(model), {
                    stale: usage.stale,
                    title: model.description || undefined,
                  })}
                </div>
              ))}
            </div>
          ) : null}
        </>
      ) : <span className="provider-usage-muted">{t('No quota data')}</span>;
    }
    return (
      <section className={classNames('provider-config-usage-section', usage?.stale && 'is-stale')}>
        <div className="provider-config-usage-header">
          <div>
            <span className="provider-config-usage-title">{t('Usage')}</span>
            <span className="provider-config-usage-note">
              {usage ? t('{name} quota windows', { name: usage.name || row.label }) : t('Quota windows')}
            </span>
          </div>
          {usage ? renderUsagePills(usage) : null}
        </div>
        {content}
      </section>
    );
  }

  function openProviderConfigDialog(key: FixedModelProviderKey) {
    const row = fixedModelProviderRow(key);
    const draft = modelProviderDraftFromState(key, gatewayDraft);
    ensureProviderModels(row.providerType, { retry: true });
    setProviderConfigDraft(applyProviderCatalogDefaults(draft, providerModelsByType[row.providerType]));
    setProviderConfigKey(key);
  }

  function closeProviderConfigDialog() {
    setProviderConfigKey(null);
    setProviderConfigDraft(emptyModelProviderConfigDraft());
  }

  async function handleSaveProviderConfig() {
    if (!providerConfigRow || providerConfigSaving) {
      return;
    }
    setProviderConfigSaving(true);
    try {
      // Optimistically updates the settings draft; the dialog stays open if
      // the subsequent gateway save fails so the user can retry. Providers
      // authenticate through their own CLI login or provider env; this
      // surface persists model defaults (and the Claude CLI mode/path).
      onMutateGatewayDraft((next) => {
        applyProviderConfigDraftToGatewayConfig(next, providerConfigRow, providerConfigDraft);
      });
      if (await onSaveGatewaySettings({ refreshDesktopState: 'background' })) {
        closeProviderConfigDialog();
      }
    } finally {
      setProviderConfigSaving(false);
    }
  }
  return (
    <div className="provider-panel">
      {renderProviderCards()}
      <Dialog open={accountSwitcherOpen} onOpenChange={setAccountSwitcherOpen}>
        <DialogContent className="provider-account-dialog" size="form">
          <DialogHeader>
            <DialogTitle>{t('Claude Code accounts')}</DialogTitle>
            <DialogDescription>
              {t('Choose the account for new Claude Code runs.')}
            </DialogDescription>
          </DialogHeader>
          <div className="provider-account-dialog-body">
            {claudeAccountsError ? (
              <div className="provider-account-error">{claudeAccountsError}</div>
            ) : null}
            {claudeAccountsLoading && !claudeAccounts ? (
              <div className="provider-account-loading">{t('Loading accounts…')}</div>
            ) : null}
            <div className="codex-list-card provider-account-list">
              {(claudeAccounts?.accounts || []).map((account) => {
                const accountKey = account.id || 'system';
                const windows = providerUsageWindows(account.usage);
                return (
                  <div
                    className={classNames('provider-account-option', account.selected && 'is-selected')}
                    key={accountKey}
                  >
                    <label className="provider-account-choice">
                      <input
                        aria-label={account.selected
                          ? t('Current account: {name}', { name: account.name })
                          : t('Use account: {name}', { name: account.name })}
                        checked={account.selected}
                        className="provider-account-radio"
                        disabled={Boolean(accountMutationId)}
                        name="claude-code-account"
                        onChange={() => {
                          if (!account.selected) void handleSelectClaudeAccount(account);
                        }}
                        type="radio"
                      />
                      <div className="provider-account-option-content">
                        <div className="provider-account-option-header">
                          <div className="provider-account-option-copy">
                            <div>
                              <strong>{account.name}</strong>
                              {account.selected ? <Badge variant="outline">{t('Current')}</Badge> : null}
                              {account.plan ? <Badge variant="outline">{account.plan}</Badge> : null}
                            </div>
                            <span>{account.email || account.organization || (account.systemDefault
                              ? t('This Mac’s default Claude Code login')
                              : t('Added to Garyx'))}</span>
                          </div>
                          {accountMutationId === accountKey ? <span>{t('Switching…')}</span> : null}
                        </div>
                        <div className="provider-account-option-meters">
                          {account.usage.available && windows.length > 0 ? windows.map((entry) => (
                            <div key={entry.key}>
                              {renderUsageMeter(
                                entry.label,
                                entry.value.remainingPercent,
                                '',
                                { compact: true, stale: account.usage.stale },
                              )}
                            </div>
                          )) : (
                            <span className="provider-card-empty">
                              {account.usage.error || t('Quota unavailable')}
                            </span>
                          )}
                        </div>
                      </div>
                    </label>
                    <div className="provider-account-option-actions">
                      <button onClick={() => openLoginDialog('reauth', account)} type="button">
                        {t('Sign in again')}
                      </button>
                      {!account.systemDefault ? (
                        <>
                          <button
                            onClick={() => {
                              setRenameAccount(account);
                              setRenameValue(account.name);
                            }}
                            type="button"
                          >
                            {t('Rename')}
                          </button>
                          <button className="destructive" onClick={() => setDeleteAccount(account)} type="button">
                            {t('Delete')}
                          </button>
                        </>
                      ) : null}
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
          <DialogFooter>
            <Button onClick={() => openLoginDialog('new')} type="button">
              <Plus aria-hidden size={14} strokeWidth={2} />
              {t('Add Claude account')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={Boolean(loginDialog)}
        onOpenChange={(open) => {
          if (!open) closeLoginDialog();
        }}
      >
        <DialogContent
          className="provider-login-dialog"
          scroll="content"
          showCloseButton={!loginBusy}
          size="form"
        >
          <DialogHeader>
            <DialogTitle>
              {loginDialog?.mode === 'new' ? t('Add Claude Code account') : t('Sign in to Claude Code')}
            </DialogTitle>
            <DialogDescription>
              {loginSession
                ? t('Finish authorization in your browser, then paste the code below.')
                : loginDialog?.mode === 'new'
                  ? t('Sign in to add another Claude Code account to Garyx.')
                  : t('Sign in again to refresh this Claude Code account.')}
            </DialogDescription>
          </DialogHeader>
          <div className="provider-login-body">
            {!loginSession && loginDialog?.mode === 'new' ? (
              <div className="commands-field">
                <Label htmlFor="provider-account-name">{t('Account name')}</Label>
                <Input
                  autoFocus
                  id="provider-account-name"
                  onChange={(event) => setLoginAccountName(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter') void handleStartClaudeLogin();
                  }}
                  placeholder={t('Work, Personal…')}
                  value={loginAccountName}
                />
              </div>
            ) : null}
            {!loginSession && loginDialog?.mode === 'reauth' ? (
              <div className="provider-login-account-summary">
                <div>
                  <strong>{loginDialog.account?.name || t('System default')}</strong>
                  <span>{loginDialog.account?.email || t('Claude Code account')}</span>
                </div>
              </div>
            ) : null}
            {loginSession ? (
              <>
                <div className="provider-browser-opened">
                  <span className="provider-browser-status-dot" aria-hidden />
                  <strong>{t('Browser opened')}</strong>
                  <span>{t('Complete sign-in there, then return to Garyx.')}</span>
                </div>
                {loginSession.authorizationUrl ? (
                  <a
                    className="provider-auth-link"
                    href={loginSession.authorizationUrl}
                    onClick={(event) => {
                      event.preventDefault();
                      void openExternalAuthUrl(loginSession.authorizationUrl || '');
                    }}
                    rel="noreferrer"
                    target="_blank"
                  >
                    {loginSession.authorizationUrl}
                  </a>
                ) : null}
                <div className="commands-field provider-code-field">
                  <Label htmlFor="provider-claude-code">{t('Authorization code')}</Label>
                  <Input
                    id="provider-claude-code"
                    onChange={(event) => setLoginCode(event.target.value)}
                    onKeyDown={(event) => {
                      if (event.key === 'Enter') void handleSubmitClaudeCode();
                    }}
                    placeholder={t('Paste code from Claude')}
                    ref={loginCodeInputRef}
                    value={loginCode}
                  />
                </div>
              </>
            ) : null}
            {loginError ? <div className="provider-account-error">{loginError}</div> : null}
          </div>
          <DialogFooter>
            <Button disabled={loginBusy} onClick={closeLoginDialog} type="button" variant="outline">
              {t('Cancel')}
            </Button>
            {!loginSession ? (
              <Button disabled={loginBusy} onClick={() => { void handleStartClaudeLogin(); }} type="button">
                {loginBusy ? t('Starting…') : t('Sign in with Claude')}
              </Button>
            ) : (
              <Button
                disabled={loginBusy || !loginCode.trim() || loginSession.status === 'submitted'}
                onClick={() => { void handleSubmitClaudeCode(); }}
                type="button"
              >
                {loginBusy || loginSession.status === 'submitted' ? t('Verifying…') : t('Continue')}
              </Button>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={Boolean(renameAccount)} onOpenChange={(open) => { if (!open) setRenameAccount(null); }}>
        <DialogContent size="narrow">
          <DialogHeader>
            <DialogTitle>{t('Rename account')}</DialogTitle>
            <DialogDescription>{t('This name is only shown inside Garyx.')}</DialogDescription>
          </DialogHeader>
          <Input
            autoFocus
            onChange={(event) => setRenameValue(event.target.value)}
            onKeyDown={(event) => { if (event.key === 'Enter') void handleRenameClaudeAccount(); }}
            value={renameValue}
          />
          <DialogFooter>
            <Button onClick={() => setRenameAccount(null)} type="button" variant="outline">{t('Cancel')}</Button>
            <Button onClick={() => { void handleRenameClaudeAccount(); }} type="button">{t('Save')}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={Boolean(deleteAccount)} onOpenChange={(open) => { if (!open) setDeleteAccount(null); }}>
        <DialogContent size="narrow">
          <DialogHeader>
            <DialogTitle>{t('Delete {name}?', { name: deleteAccount?.name || t('account') })}</DialogTitle>
            <DialogDescription>
              {t('This removes the account and its local Claude Code data from this Mac. This cannot be undone.')}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button onClick={() => setDeleteAccount(null)} type="button" variant="outline">{t('Cancel')}</Button>
            <Button onClick={() => { void handleDeleteClaudeAccount(); }} type="button" variant="destructive">
              {t('Delete account')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={Boolean(providerConfigKey)}
        onOpenChange={(open) => {
          if (!open) {
            closeProviderConfigDialog();
          }
        }}
      >
        <DialogContent
          className="provider-config-dialog"
          showCloseButton={false}
          size="form"
        >
          <DialogHeader className="commands-dialog-header">
            <Badge variant="outline" className="commands-dialog-badge">
              {t('Default Provider')}
            </Badge>
            <div className="provider-config-dialog-heading">
              {providerConfigRow ? (
                <span className="provider-config-icon large" aria-hidden>
                  <ProviderAgentIcon
                    agentId={providerConfigRow.agentId}
                    providerType={providerConfigRow.providerType}
                    size={28}
                  />
                </span>
              ) : null}
              <div className="commands-dialog-title-group">
                <DialogTitle className="commands-dialog-title">
                  {providerConfigRow ? t('Configure {name}', { name: providerConfigRow.label }) : t('Configure Provider')}
                </DialogTitle>
                <DialogDescription className="commands-dialog-description provider-config-dialog-description">
                  {providerConfigRow ? (
                    <span className="provider-config-header-meta">
                      <code>{providerConfigRow.providerType}</code>
                      <span>{t('Default CLI')}</span>
                    </span>
                  ) : null}
                  <span>{t('Provider rows are fixed. Configuration controls whether each provider is ready to use.')}</span>
                </DialogDescription>
              </div>
            </div>
          </DialogHeader>

          <div className="commands-dialog-body provider-config-dialog-body">
            {providerConfigRow ? renderProviderConfigUsageSection(providerConfigRow) : null}

            {providerConfigRow?.key === 'claude_code' ? (
              <>
                <div className="commands-field">
                  <Label className="commands-field-label">{t('Agent SDK CLI')}</Label>
                  <Select
                    value={providerConfigDraft.claudeCliMode}
                    onValueChange={(value) => {
                      setProviderConfigDraft((current) => ({
                        ...current,
                        claudeCliMode: value === 'native' ? 'native' : 'cctty',
                      }));
                    }}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectGroup>
                        <SelectItem value="native">{t('Native Claude CLI')}</SelectItem>
                        <SelectItem value="cctty">{t('cctty TTY wrapper')}</SelectItem>
                      </SelectGroup>
                    </SelectContent>
                  </Select>
                </div>
                <div className="commands-field">
                  <div className="commands-field-header">
                    <Label className="commands-field-label" htmlFor="provider-claude-cli-path">{t('CLI path')}</Label>
                    <span className="commands-field-hint">{t('Leave empty to use native Claude from PATH or embedded cctty.')}</span>
                  </div>
                  <Input
                    id="provider-claude-cli-path"
                    placeholder={providerConfigDraft.claudeCliMode === 'native' ? 'claude' : 'cctty'}
                    value={providerConfigDraft.claudeCliPath}
                    onChange={(event) => {
                      setProviderConfigDraft((current) => ({
                        ...current,
                        claudeCliPath: event.target.value,
                      }));
                    }}
                  />
                </div>
              </>
            ) : null}

            {providerConfigRow ? (
              <>
                <div className="provider-config-grid">
                  <div className="commands-field">
                    <Label className="commands-field-label" htmlFor="provider-model">{t('Model')}</Label>
                    {activeProviderModelOptions.length > 0 ? (
                      <Select
                        value={providerConfigDraft.model.trim() || PROVIDER_DEFAULT_MODEL_VALUE}
                        onValueChange={(value) => {
                          setProviderConfigDraft((current) => {
                            const nextModel = value === PROVIDER_DEFAULT_MODEL_VALUE ? '' : value;
                            const reasoningOptions = reasoningEffortOptionsForModel(
                              activeProviderModels,
                              nextModel,
                              '',
                            );
                            return {
                              ...current,
                              model: nextModel,
                              modelReasoningEffort: nextModel ? highestReasoningEffort(reasoningOptions) : '',
                              modelServiceTier: sanitizedServiceTier(
                                activeProviderModels,
                                nextModel,
                                current.modelServiceTier,
                              ),
                            };
                          });
                        }}
                      >
                        <SelectTrigger id="provider-model">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectGroup>
                            <SelectItem value={PROVIDER_DEFAULT_MODEL_VALUE}>{t('Provider default')}</SelectItem>
                            {activeProviderModelOptions.map((option) => (
                              <SelectItem key={option.id} value={option.id}>
                                {option.label}
                              </SelectItem>
                            ))}
                          </SelectGroup>
                        </SelectContent>
                      </Select>
                    ) : (
                      <Input
                        id="provider-model"
                        placeholder={activeProviderModelsLoading ? t('Loading models...') : providerConfigRow.defaultModel}
                        value={providerConfigDraft.model}
                        onChange={(event) => {
                          setProviderConfigDraft((current) => ({
                            ...current,
                            model: event.target.value,
                          }));
                        }}
                      />
                    )}
                  </div>
                  <div className="commands-field">
                    <Label className="commands-field-label">{t('Reasoning')}</Label>
                    {activeReasoningOptions.length > 0 ? (
                      <Select
                        value={providerConfigDraft.modelReasoningEffort.trim() || PROVIDER_DEFAULT_REASONING_VALUE}
                        onValueChange={(value) => {
                          setProviderConfigDraft((current) => ({
                            ...current,
                            modelReasoningEffort: value === PROVIDER_DEFAULT_REASONING_VALUE ? '' : value,
                          }));
                        }}
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectGroup>
                            <SelectItem value={PROVIDER_DEFAULT_REASONING_VALUE}>{t('Provider default')}</SelectItem>
                            {activeReasoningOptions.map((option) => (
                              <SelectItem key={option.id} value={option.id}>
                                {option.label}
                              </SelectItem>
                            ))}
                          </SelectGroup>
                        </SelectContent>
                      </Select>
                    ) : (
                      <Input
                        disabled
                        value=""
                        placeholder={activeProviderModelsLoading ? t('Loading models...') : t('Unavailable')}
                        readOnly
                      />
                    )}
                  </div>
                  {activeSupportsServiceTier ? (
                    <div className="commands-field">
                      <Label className="commands-field-label" htmlFor="provider-service-tier">{t('Speed')}</Label>
                      <Select
                        value={providerConfigDraft.modelServiceTier.trim() || PROVIDER_DEFAULT_SERVICE_TIER_VALUE}
                        onValueChange={(value) => {
                          setProviderConfigDraft((current) => ({
                            ...current,
                            modelServiceTier: value === PROVIDER_DEFAULT_SERVICE_TIER_VALUE ? '' : value,
                          }));
                        }}
                      >
                        <SelectTrigger id="provider-service-tier">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectGroup>
                            <SelectItem value={PROVIDER_DEFAULT_SERVICE_TIER_VALUE}>{t('Standard')}</SelectItem>
                            {activeServiceTierOptions.map((option) => (
                              <SelectItem key={option.id} value={option.id}>
                                {option.label}
                              </SelectItem>
                            ))}
                          </SelectGroup>
                        </SelectContent>
                      </Select>
                    </div>
                  ) : null}
                </div>
              </>
            ) : null}
          </div>

          <DialogFooter className="commands-dialog-footer">
            <div className="provider-config-footer-left" />
            <div className="provider-config-footer-actions">
              <Button
                className="commands-dialog-button secondary"
                onClick={closeProviderConfigDialog}
                type="button"
                variant="outline"
              >
                {t('Cancel')}
              </Button>
              <Button
                className="commands-dialog-button primary"
                disabled={providerConfigSaving}
                onClick={() => { void handleSaveProviderConfig(); }}
                type="button"
              >
                {providerConfigSaving ? t('Saving…') : t('Save')}
              </Button>
            </div>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
