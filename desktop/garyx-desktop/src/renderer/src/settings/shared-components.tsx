import type { ReactNode } from 'react';

import { classNames } from './shared';

type SettingsControlRowProps = {
  label: string;
  description?: string;
  control: ReactNode;
  stacked?: boolean;
  className?: string;
};

export function SettingsControlRow({
  label,
  description,
  control,
  stacked = false,
  className,
}: SettingsControlRowProps) {
  return (
    <div className={classNames('settings-control-row', stacked && 'stacked', className)}>
      <div className="settings-control-row-copy">
        <div className="settings-control-row-label">{label}</div>
        {description ? <p className="settings-control-row-description">{description}</p> : null}
      </div>
      <div className="settings-control-row-control">{control}</div>
    </div>
  );
}
