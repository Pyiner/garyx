import { useMemo, useState } from 'react';
import {
  CheckIcon,
  PencilIcon,
  ServerIcon,
  SettingsIcon,
} from 'lucide-react';

import type { ConnectionStatus, DesktopGatewayProfile } from '@shared/contracts';
import { cn } from '@/lib/utils';
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover';
import { useI18n } from './i18n';

type GatewaySwitcherTone = 'connected' | 'syncing' | 'offline';

type GatewayIdentityBarProps = {
  connection: ConnectionStatus | null;
  indicatorTone: 'syncing' | 'offline' | null;
  currentGatewayUrl: string;
  profiles: DesktopGatewayProfile[];
  onSwitch: (profile: DesktopGatewayProfile) => Promise<boolean>;
  onRename: (profileId: string, label: string) => Promise<void>;
  onOpenGatewaySettings: () => void;
  onOpenSettings: () => void;
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

/// Bottom-left gateway identity bar: replaces the plain Settings row with the
/// current gateway's identity (glyph + name + connection state). The bar body
/// opens an upward switcher popover (switch, inline rename, settings entries);
/// the trailing gear keeps Settings one click away. Gateway management stays
/// in Settings -> Gateway.
export function GatewayIdentityBar({
  connection,
  indicatorTone,
  currentGatewayUrl,
  profiles,
  onSwitch,
  onRename,
  onOpenGatewaySettings,
  onOpenSettings,
}: GatewayIdentityBarProps) {
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

  function resetPopoverState() {
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

  function renderGlyph(withBadge: boolean, className: string) {
    return (
      <span aria-hidden className={className}>
        <ServerIcon size={13} strokeWidth={1.8} />
        {withBadge ? (
          <span className={`gateway-glyph-badge is-${tone}`} />
        ) : null}
      </span>
    );
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
        {renderGlyph(isCurrent, 'gateway-row-glyph')}
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
    <div className="gateway-identity-bar">
      <Popover
        open={open}
        onOpenChange={(next) => {
          setOpen(next);
          if (!next) {
            resetPopoverState();
          }
        }}
      >
        <PopoverTrigger asChild>
          <button
            aria-label={t('Switch gateway')}
            className="gateway-identity-main"
            title={`${currentLabel} · ${toneLabel}`}
            type="button"
          >
            {renderGlyph(true, 'gateway-identity-glyph')}
            <span className="gateway-identity-copy">
              <span className="gateway-identity-name">{currentLabel}</span>
              <span className="gateway-identity-status">{toneLabel}</span>
            </span>
          </button>
        </PopoverTrigger>
        <PopoverContent
          align="start"
          className="gateway-switcher-popover"
          side="top"
          sideOffset={10}
        >
          <div className="gateway-switcher-list">
            {rows.current ? renderRow(rows.current, true) : null}
            {rows.others.map((profile) => renderRow(profile, false))}
          </div>

          {errorText ? (
            <p className="gateway-switcher-error" role="alert">{errorText}</p>
          ) : null}

          <div className="gateway-switcher-popdivider" />

          <button
            className="gateway-switcher-action"
            onClick={() => {
              setOpen(false);
              resetPopoverState();
              onOpenGatewaySettings();
            }}
            type="button"
          >
            <ServerIcon aria-hidden size={15} strokeWidth={1.8} />
            <span>{t('Gateway Settings…')}</span>
          </button>
          <button
            className="gateway-switcher-action"
            onClick={() => {
              setOpen(false);
              resetPopoverState();
              onOpenSettings();
            }}
            type="button"
          >
            <SettingsIcon aria-hidden size={15} strokeWidth={1.8} />
            <span>{t('Settings')}</span>
          </button>
        </PopoverContent>
      </Popover>

      <button
        aria-label={t('Settings')}
        className="gateway-identity-gear"
        onClick={onOpenSettings}
        title={t('Settings')}
        type="button"
      >
        <SettingsIcon aria-hidden size={14} strokeWidth={1.9} />
      </button>
    </div>
  );
}
