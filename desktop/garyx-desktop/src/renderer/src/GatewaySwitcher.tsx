import { useMemo, useState } from 'react';
import { CheckIcon, ServerIcon, SettingsIcon } from 'lucide-react';

import type { ConnectionStatus, DesktopGatewayProfile } from '@shared/contracts';
import { cn } from '@/lib/utils';
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover';
import { IconTooltip, TooltipProvider } from '@/components/ui/tooltip';
import { useI18n } from './i18n';

type GatewaySwitcherTone = 'connected' | 'syncing' | 'offline';

type GatewayIdentityBarProps = {
  connection: ConnectionStatus | null;
  indicatorTone: 'syncing' | 'offline' | null;
  currentGatewayUrl: string;
  profiles: DesktopGatewayProfile[];
  onSwitch: (profile: DesktopGatewayProfile) => Promise<boolean>;
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
/// opens an upward popover that lists saved gateways — picking one switches
/// directly. The trailing gear keeps Settings one click away; gateway
/// management lives in Settings -> Gateway.
export function GatewayIdentityBar({
  connection,
  indicatorTone,
  currentGatewayUrl,
  profiles,
  onSwitch,
  onOpenSettings,
}: GatewayIdentityBarProps) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [switchingId, setSwitchingId] = useState<string | null>(null);
  const [errorText, setErrorText] = useState<string | null>(null);

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

  // Saved order is kept as-is so rows do not jump around while switching;
  // the current gateway is only marked, never moved.
  const rows = useMemo(() => {
    const saved = profiles.filter((profile) => profile.gatewayUrl.trim().length > 0);
    const currentKey = gatewayUrlKey(currentGatewayUrl);
    const hasCurrent = saved.some(
      (profile) => gatewayUrlKey(profile.gatewayUrl) === currentKey,
    );
    if (!hasCurrent && currentKey) {
      saved.unshift({
        id: UNSAVED_CURRENT_PROFILE_ID,
        label: gatewayHostLabel(currentGatewayUrl),
        gatewayUrl: currentGatewayUrl.trim(),
        gatewayAuthToken: '',
        gatewayHeaders: '',
        updatedAt: '',
      });
    }
    return { list: saved, currentKey };
  }, [profiles, currentGatewayUrl]);

  if (!currentGatewayUrl.trim()) {
    return null;
  }

  const currentProfile = rows.list.find(
    (profile) => gatewayUrlKey(profile.gatewayUrl) === rows.currentKey,
  ) || null;
  const currentLabel = currentProfile?.label || gatewayHostLabel(currentGatewayUrl);

  function resetPopoverState() {
    setSwitchingId(null);
    setErrorText(null);
  }

  async function handleRowActivate(profile: DesktopGatewayProfile, isCurrent: boolean) {
    if (switchingId) {
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
    const switching = switchingId === profile.id;
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
          <span className="gateway-switcher-item-name">{profile.label}</span>
          <span className="gateway-switcher-item-url">{profile.gatewayUrl}</span>
        </span>
        {switching ? (
          <span className="gateway-switcher-item-state">{t('Connecting…')}</span>
        ) : null}
        {!switching && isCurrent ? (
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
    <TooltipProvider>
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
            {rows.list.map((profile) => (
              renderRow(profile, gatewayUrlKey(profile.gatewayUrl) === rows.currentKey)
            ))}
          </div>

          {errorText ? (
            <p className="gateway-switcher-error" role="alert">{errorText}</p>
          ) : null}
        </PopoverContent>
      </Popover>

      <IconTooltip label={t('Settings')} side="bottom">
        <button
          aria-label={t('Settings')}
          className="gateway-identity-gear"
          onClick={onOpenSettings}
          type="button"
        >
          <SettingsIcon aria-hidden size={14} strokeWidth={1.9} />
        </button>
      </IconTooltip>
    </div>
    </TooltipProvider>
  );
}
