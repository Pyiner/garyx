import { useEffect, useState } from 'react';

import type { DesktopUpdateStatus } from '@shared/contracts';
import { useI18n } from '../../i18n';

const IDLE_STATUS: DesktopUpdateStatus = { phase: 'idle' };

export function UpdatePill() {
  const { t } = useI18n();
  const [status, setStatus] = useState<DesktopUpdateStatus>(IDLE_STATUS);
  const [installing, setInstalling] = useState(false);

  useEffect(() => {
    const api = window.garyxDesktop;
    let cancelled = false;
    const listener = (next: DesktopUpdateStatus) => {
      if (cancelled) return;
      setStatus(next);
    };

    void api.getUpdateStatus().then((initial) => {
      if (cancelled) return;
      setStatus(initial);
    });
    api.subscribeUpdateStatus(listener);

    return () => {
      cancelled = true;
      api.unsubscribeUpdateStatus(listener);
    };
  }, []);

  if (status.phase !== 'downloaded') {
    return null;
  }

  return (
    <button
      className="update-pill update-pill-ready"
      disabled={installing}
      onClick={() => {
        if (installing) return;
        setInstalling(true);
        void window.garyxDesktop.installUpdate().catch(() => {
          setInstalling(false);
        });
      }}
      title={t('Update to v{version} and restart', { version: status.info.version })}
      type="button"
    >
      <span className="update-pill-dot" />
      <span className="update-pill-label">{t('Update')}</span>
      <span className="update-pill-version">v{status.info.version}</span>
    </button>
  );
}
