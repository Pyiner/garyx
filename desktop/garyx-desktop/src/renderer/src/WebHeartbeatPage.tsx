type HeartbeatAgent = {
  target?: string;
  count?: number;
};

type HeartbeatSummary = {
  enabled?: boolean;
  interval?: string;
  recent_count?: number;
  successful?: number;
  skipped?: number;
  last_run?: string | null;
  service_available?: boolean;
  agents?: HeartbeatAgent[];
};

type WebHeartbeatPageProps = {
  summary: HeartbeatSummary | null;
  loading: boolean;
  error?: string | null;
  triggering?: boolean;
  onRefresh?: () => void;
  onTrigger?: () => void;
};

function formatTime(value?: string | null): string {
  if (!value) {
    return '--';
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }
  return date.toLocaleString(undefined, { hour12: false });
}

export function WebHeartbeatPage({
  summary,
  loading,
  error,
  triggering,
  onRefresh,
  onTrigger,
}: WebHeartbeatPageProps) {
  const agents = Array.isArray(summary?.agents) ? summary.agents : [];

  return (
    <div className="thread-history-shell">
      <div className="thread-history-page shadcn-shell">
        <section className="shadcn-hero">
          <div className="shadcn-hero-copy">
            <p className="shadcn-kicker">Heartbeat</p>
            <h1>Heartbeat service overview</h1>
            <p className="shadcn-subcopy">
              {summary?.service_available ? 'service available' : 'service unavailable'}
              {' · '}
              {summary?.enabled === false ? 'disabled' : 'enabled'}
            </p>
          </div>
          <div className="shadcn-hero-actions">
            {onRefresh ? (
              <UIButton onClick={onRefresh} variant="outline">
                Refresh
              </UIButton>
            ) : null}
            {onTrigger ? (
              <UIButton disabled={loading || triggering} onClick={onTrigger}>
                {triggering ? 'Triggering…' : 'Trigger'}
              </UIButton>
            ) : null}
          </div>
        </section>

        {error ? (
          <div className="bot-console-error" role="alert">
            {error}
          </div>
        ) : null}

        {loading ? (
          <div className="empty-state">
            <span className="eyebrow">Heartbeat</span>
            <h3>Loading heartbeat summary</h3>
          </div>
        ) : (
          <>
            <section className="thread-history-panel">
              <div className="web-settings-summary-grid">
                <UICard className="web-settings-summary-card">
                  <UICardHeader>
                    <UIBadge>Enabled</UIBadge>
                    <UICardTitle>{summary?.enabled === false ? 'No' : 'Yes'}</UICardTitle>
                    <UICardDescription>interval {String(summary?.interval || '--')}</UICardDescription>
                  </UICardHeader>
                </UICard>
                <UICard className="web-settings-summary-card">
                  <UICardHeader>
                    <UIBadge>Recent Count</UIBadge>
                    <UICardTitle>{String(summary?.recent_count || 0)}</UICardTitle>
                    <UICardDescription>persisted heartbeat records</UICardDescription>
                  </UICardHeader>
                </UICard>
                <UICard className="web-settings-summary-card">
                  <UICardHeader>
                    <UIBadge>Successful</UIBadge>
                    <UICardTitle>{String(summary?.successful || 0)}</UICardTitle>
                    <UICardDescription>non-skipped records</UICardDescription>
                  </UICardHeader>
                </UICard>
                <UICard className="web-settings-summary-card">
                  <UICardHeader>
                    <UIBadge>Skipped</UIBadge>
                    <UICardTitle>{String(summary?.skipped || 0)}</UICardTitle>
                    <UICardDescription>outside active window or skipped</UICardDescription>
                  </UICardHeader>
                </UICard>
                <UICard className="web-settings-summary-card">
                  <UICardHeader>
                    <UIBadge>Last Run</UIBadge>
                    <UICardTitle>{formatTime(summary?.last_run)}</UICardTitle>
                    <UICardDescription>latest persisted heartbeat record</UICardDescription>
                  </UICardHeader>
                </UICard>
                <UICard className="web-settings-summary-card">
                  <UICardHeader>
                    <UIBadge>Targets</UIBadge>
                    <UICardTitle>{agents.length}</UICardTitle>
                    <UICardDescription>targets with successful sends</UICardDescription>
                  </UICardHeader>
                </UICard>
              </div>
            </section>

            <section className="thread-history-panel">
              <div className="thread-history-toolbar-copy">
                <span className="eyebrow">Targets</span>
                <p>Aggregated successful heartbeat deliveries by target.</p>
              </div>
              {!agents.length ? (
                <div className="empty-state">
                  <span className="eyebrow">Targets</span>
                  <h3>No heartbeat targets found</h3>
                </div>
              ) : (
                <div className="web-cron-table-wrap">
                  <table className="web-cron-table">
                    <thead>
                      <tr>
                        <th>Target</th>
                        <th>Count</th>
                      </tr>
                    </thead>
                    <tbody>
                      {agents.map((agent) => (
                        <tr key={agent.target || 'target'}>
                          <td><strong>{agent.target || '--'}</strong></td>
                          <td>{String(agent.count || 0)}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </section>
          </>
        )}
      </div>
    </div>
  );
}
import {
  UIButton,
  UIBadge,
  UICard,
  UICardDescription,
  UICardHeader,
  UICardTitle,
} from './ui';
