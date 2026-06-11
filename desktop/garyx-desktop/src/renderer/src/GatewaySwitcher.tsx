import { useMemo, useState } from 'react';
import { CheckIcon, ChevronDownIcon, PencilIcon, ServerIcon } from 'lucide-react';

import type { ConnectionStatus, DesktopGatewayProfile } from '@shared/contracts';
import { cn } from '@/lib/utils';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';
import { useI18n } from './i18n';

type GatewaySwitcherTone = 'connected' | 'syncing' | 'offline';

type GatewaySwitcherControlProps = {
  connection: ConnectionStatus | null;
  indicatorTone: 'syncing' | 'offline' | null;
  currentGatewayUrl: string;
  profiles: DesktopGatewayProfile[];
  onSwitch: (profile: DesktopGatewayProfile) => Promise<boolean>;
  onRename: (profileId: string, label: string) => Promise<void>;
  onOpenGatewaySettings: () => void;
};

function gatewayHostLabel(gatewayUrl: string): string {
  try {
    const parsed = new URL(gatewayUrl);
    return parsed.host || gatewayUrl;
  } catch {
    return gatewayUrl;
  }
}

function gatewayUrlKey(gatewayUrl: string): string {
  return gatewayUrl.trim().toLowerCase();
}

const UNSAVED_CURRENT_PROFILE_ID = 'gateway-switcher::current-unsaved';

/// Title-bar gateway identity control: shows the current gateway and its
/// connection state next to the macOS traffic lights; clicking opens the
/// switcher dialog. Gateway management stays in Settings -> Gateway.
export function GatewaySwitcherControl({
  connection,
  indicatorTone,
  currentGatewayUrl,
  profiles,
  onSwitch,
  onRename,
  onOpenGatewaySettings,
}: GatewaySwitcherControlProps) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [switchingId, setSwitchingId] = useState<string | null>(null);
  const [errorText, setErrorText] = useState<string | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editDraft, setEditDraft] = useState('');

  const tone: GatewaySwitcherTone = connection?.ok
    ? 'connected'
    : indicatorTone === 'offline'
      ? 'offline'
      : 'syncing';
  const toneLabel = tone === 'connected'
    ? t('Connected')
    : tone === 'offline'
      ? t('Gateway offline')
      : t('Connecting…');

  const rows = useMemo(() => {
    const saved = profiles.filter((profile) => profile.gatewayUrl.trim().length > 0);
    const currentKey = gatewayUrlKey(currentGatewayUrl);
    const currentIndex = saved.findIndex(
      (profile) => gatewayUrlKey(profile.gatewayUrl) === currentKey,
    );
    let current: DesktopGatewayProfile | null = null;
    if (currentIndex >= 0) {
      [current] = saved.splice(currentIndex, 1);
    } else if (currentKey) {
      current = {
        id: UNSAVED_CURRENT_PROFILE_ID,
        label: gatewayHostLabel(currentGatewayUrl),
        gatewayUrl: currentGatewayUrl.trim(),
        gatewayAuthToken: '',
        updatedAt: '',
      };
    }
    return { current, others: saved };
  }, [profiles, currentGatewayUrl]);

  if (!currentGatewayUrl.trim()) {
    return null;
  }

  const currentLabel = rows.current?.label || gatewayHostLabel(currentGatewayUrl);

  function resetDialogState() {
    setSwitchingId(null);
    setErrorText(null);
    setEditingId(null);
    setEditDraft('');
  }

  async function handleRowActivate(profile: DesktopGatewayProfile, isCurrent: boolean) {
    if (editingId || switchingId) {
      return;
    }
    if (isCurrent) {
      setOpen(false);
      return;
    }
    setErrorText(null);
    setSwitchingId(profile.id);
    try {
      const switched = await onSwitch(profile);
      if (switched) {
        setOpen(false);
        return;
      }
      setErrorText(t('Unable to connect to {label}', { label: profile.label }));
    } catch {
      setErrorText(t('Unable to connect to {label}', { label: profile.label }));
    } finally {
      setSwitchingId(null);
    }
  }

  function beginRename(profile: DesktopGatewayProfile) {
    setErrorText(null);
    setEditingId(profile.id);
    setEditDraft(profile.label);
  }

  async function commitRename(profile: DesktopGatewayProfile) {
    const draft = editDraft;
    setEditingId(null);
    setEditDraft('');
    if (draft.trim() === profile.label) {
      return;
    }
    await onRename(profile.id, draft);
  }

  function renderRow(profile: DesktopGatewayProfile, isCurrent: boolean) {
    const editing = editingId === profile.id;
    const switching = switchingId === profile.id;
    const canRename = profile.id !== UNSAVED_CURRENT_PROFILE_ID;
    return (
      <div
        className={cn(
          'gateway-switcher-item',
          isCurrent && 'is-current',
          switching && 'is-switching',
        )}
        key={profile.id}
        onClick={() => {
          void handleRowActivate(profile, isCurrent);
        }}
        onKeyDown={(event) => {
          if (editing) {
            return;
          }
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            void handleRowActivate(profile, isCurrent);
          }
        }}
        role="button"
        tabIndex={0}
      >
        <span
          aria-hidden
          className={cn(
            'gateway-switcher-dot',
            isCurrent ? `is-${tone}` : 'is-idle',
          )}
        />
        <span className="gateway-switcher-item-copy">
          {editing ? (
            <input
              autoFocus
              className="gateway-switcher-rename-input"
              onBlur={() => {
                void commitRename(profile);
              }}
              onChange={(event) => {
                setEditDraft(event.target.value);
              }}
              onClick={(event) => {
                event.stopPropagation();
              }}
              onKeyDown={(event) => {
                event.stopPropagation();
                if (event.key === 'Enter') {
                  event.preventDefault();
                  void commitRename(profile);
                }
                if (event.key === 'Escape') {
                  event.preventDefault();
                  setEditingId(null);
                  setEditDraft('');
                }
              }}
              spellCheck={false}
              value={editDraft}
            />
          ) : (
            <span className="gateway-switcher-item-name">{profile.label}</span>
          )}
          <span className="gateway-switcher-item-url">{profile.gatewayUrl}</span>
        </span>
        {switching ? (
          <span className="gateway-switcher-item-state">{t('Connecting…')}</span>
        ) : null}
        {!switching && !editing && canRename ? (
          <button
            aria-label={t('Rename gateway')}
            className="gateway-switcher-rename"
            onClick={(event) => {
              event.stopPropagation();
              beginRename(profile);
            }}
            title={t('Rename gateway')}
            type="button"
          >
            <PencilIcon aria-hidden size={12.5} strokeWidth={1.9} />
          </button>
        ) : null}
        {!switching && !editing && isCurrent ? (
          <CheckIcon
            aria-hidden
            className="gateway-switcher-check"
            size={15}
            strokeWidth={2.1}
          />
        ) : null}
      </div>
    );
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) {
          resetDialogState();
        }
      }}
    >
      <DialogTrigger asChild>
        <button
          aria-label={t('Switch gateway')}
          className="gateway-switcher-trigger"
          title={`${currentLabel} · ${toneLabel}`}
          type="button"
        >
          <span aria-hidden className={`gateway-switcher-dot is-${tone}`} />
          <span className="gateway-switcher-trigger-name">{currentLabel}</span>
          <ChevronDownIcon
            aria-hidden
            className="gateway-switcher-trigger-chevron"
            size={12}
            strokeWidth={2}
          />
        </button>
      </DialogTrigger>
      <DialogContent className="gateway-switcher-dialog" size="compact">
        <DialogHeader className="gateway-profile-dialog-header">
          <div className="gateway-profile-dialog-title-row">
            <span aria-hidden className="gateway-profile-dialog-icon">
              <ServerIcon size={15} strokeWidth={1.9} />
            </span>
            <DialogTitle className="gateway-profile-dialog-title">
              {t('Gateways')}
            </DialogTitle>
          </div>
        </DialogHeader>

        <div className="gateway-switcher-list">
          {rows.current ? renderRow(rows.current, true) : null}
          {rows.others.map((profile) => renderRow(profile, false))}
        </div>

        {errorText ? (
          <p className="gateway-switcher-error" role="alert">{errorText}</p>
        ) : null}

        <div className="gateway-switcher-foot">
          <button
            className="gateway-switcher-manage"
            onClick={() => {
              setOpen(false);
              resetDialogState();
              onOpenGatewaySettings();
            }}
            type="button"
          >
            {t('Gateway Settings…')}
          </button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
