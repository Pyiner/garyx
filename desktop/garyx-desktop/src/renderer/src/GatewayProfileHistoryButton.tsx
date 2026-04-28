import { useMemo, useState } from 'react';
import { HistoryIcon } from 'lucide-react';

import type { DesktopGatewayProfile } from '@shared/contracts';
import { cn } from '@/lib/utils';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog';

type GatewayProfileHistoryButtonProps = {
  profiles: DesktopGatewayProfile[];
  className?: string;
  onSelect: (profile: DesktopGatewayProfile) => void;
};

export function GatewayProfileHistoryButton({
  profiles,
  className,
  onSelect,
}: GatewayProfileHistoryButtonProps) {
  const [open, setOpen] = useState(false);
  const normalizedProfiles = useMemo(() => {
    return profiles.filter((profile) => profile.gatewayUrl.trim().length > 0);
  }, [profiles]);

  if (normalizedProfiles.length === 0) {
    return null;
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <button
          aria-label="Choose gateway"
          className={cn('gateway-profile-history-trigger', className)}
          title="Choose gateway"
          type="button"
        >
          <HistoryIcon aria-hidden size={17} strokeWidth={1.9} />
        </button>
      </DialogTrigger>
      <DialogContent className="gateway-profile-dialog">
        <DialogHeader className="gateway-profile-dialog-header">
          <div className="gateway-profile-dialog-title-row">
            <span className="gateway-profile-dialog-icon" aria-hidden>
              <HistoryIcon size={15} strokeWidth={1.9} />
            </span>
            <DialogTitle className="gateway-profile-dialog-title">
              Choose gateway
            </DialogTitle>
          </div>
        </DialogHeader>

        <div className="gateway-profile-list">
          {normalizedProfiles.map((profile) => (
            <button
              className="gateway-profile-item"
              key={profile.id}
              onClick={() => {
                onSelect(profile);
                setOpen(false);
              }}
              type="button"
            >
              {profile.gatewayUrl}
            </button>
          ))}
        </div>
      </DialogContent>
    </Dialog>
  );
}
