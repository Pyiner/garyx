import {
  UIButton,
  UIBadge,
  UICard,
  UICardContent,
  UICardDescription,
  UICardHeader,
  UICardTitle,
} from './ui';

type GatewayOverview = {
  status?: string;
  uptime_seconds?: number;
  version?: string;
  gateway?: {
    host?: string;
    port?: number;
    public_url?: string | null;
  };
  threads?: {
    active?: number;
  };
  providers?: {
    count?: number;
  };
  active_runs?: number;
  stream?: {
    drops?: number;
    history_size?: number;
  };
  channels?: {
    feishu_policy_blocks?: number;
  };
};

type AgentViewPayload = {
  bridge_ready?: boolean;
  total_active_runs?: number;
  providers?: Array<{
    key?: string;
    type?: string;
    ready?: boolean;
    active_runs?: number;
  }>;
};

type WebStatusPageProps = {
  overview: GatewayOverview | null;
  agentView: AgentViewPayload | null;
  loading: boolean;
  error?: string | null;
  onRefresh?: () => void;
};

function formatUptime(seconds?: number): string {
  if (seconds == null || !Number.isFinite(seconds)) {
    return '--';
  }
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (days > 0) {
    return `${days}d ${hours}h ${minutes}m`;
  }
  if (hours > 0) {
    return `${hours}h ${minutes}m`;
  }
  return `${minutes}m`;
}

function stringifyJson(value: unknown): string {
  return JSON.stringify(value || {}, null, 2);
}

export function WebStatusPage({
  overview,
  agentView,
  loading,
  error,
  onRefresh,
}: WebStatusPageProps) {
  const providers = Array.isArray(agentView?.providers) ? agentView.providers : [];
  const gatewayHost = overview?.gateway?.host || '0.0.0.0';
  const gatewayPort = overview?.gateway?.port || 31337;
  const publicUrl = overview?.gateway?.public_url || 'No public URL configured';

  return (
    <div className="thread-history-shell">
      <div className="thread-history-page shadcn-shell">
        <section className="shadcn-hero">
          <div className="shadcn-hero-copy">
            <p className="shadcn-kicker">Status</p>
            <h1>Gateway runtime overview</h1>
            <p className="shadcn-subcopy">
              {String(overview?.status || 'unknown')} · version {String(overview?.version || '--')}
            </p>
          </div>
          <div className="shadcn-hero-actions">
            {onRefresh ? (
              <UIButton onClick={onRefresh} variant="outline">
                Refresh
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
            <span className="eyebrow">Status</span>
            <h3>Loading runtime overview</h3>
          </div>
        ) : (
          <>
            <section className="thread-history-panel">
            <div className="web-settings-summary-grid">
              <UICard className="web-settings-summary-card">
                <UICardHeader>
                  <UIBadge>Gateway</UIBadge>
                  <UICardTitle>{gatewayHost}:{gatewayPort}</UICardTitle>
                  <UICardDescription>{publicUrl}</UICardDescription>
                </UICardHeader>
              </UICard>
              <UICard className="web-settings-summary-card">
                <UICardHeader>
                  <UIBadge>Uptime</UIBadge>
                  <UICardTitle>{formatUptime(overview?.uptime_seconds)}</UICardTitle>
                  <UICardDescription>status {String(overview?.status || '--')}</UICardDescription>
                </UICardHeader>
              </UICard>
              <UICard className="web-settings-summary-card">
                <UICardHeader>
                  <UIBadge>Threads</UIBadge>
                  <UICardTitle>{String(overview?.threads?.active || 0)}</UICardTitle>
                  <UICardDescription>{String(overview?.active_runs || 0)} active runs</UICardDescription>
                </UICardHeader>
              </UICard>
              <UICard className="web-settings-summary-card">
                <UICardHeader>
                  <UIBadge>Providers</UIBadge>
                  <UICardTitle>{String(overview?.providers?.count || providers.length)}</UICardTitle>
                  <UICardDescription>{agentView?.bridge_ready ? 'bridge ready' : 'bridge not ready'}</UICardDescription>
                </UICardHeader>
              </UICard>
              <UICard className="web-settings-summary-card">
                <UICardHeader>
                  <UIBadge>Stream</UIBadge>
                  <UICardTitle>{String(overview?.stream?.drops || 0)} drops</UICardTitle>
                  <UICardDescription>history {String(overview?.stream?.history_size || 0)}</UICardDescription>
                </UICardHeader>
              </UICard>
              <UICard className="web-settings-summary-card">
                <UICardHeader>
                  <UIBadge>Feishu policy</UIBadge>
                  <UICardTitle>{String(overview?.channels?.feishu_policy_blocks || 0)}</UICardTitle>
                  <UICardDescription>blocked deliveries</UICardDescription>
                </UICardHeader>
              </UICard>
            </div>
          </section>

            <section className="thread-history-panel">
            <div className="thread-history-toolbar-copy">
              <span className="eyebrow">Providers</span>
              <h3>Agent bridge providers</h3>
              <p>{providers.length} configured provider entries</p>
            </div>
            {!providers.length ? (
              <div className="empty-state">
                <span className="eyebrow">Providers</span>
                <h3>No providers reported</h3>
              </div>
            ) : (
              <div className="thread-overview-grid">
                {providers.map((provider) => (
                  <UICard className="thread-overview-card thread-overview-card-static" key={provider.key || provider.type || 'provider'}>
                    <UICardHeader className="thread-overview-card-head">
                      <strong>{provider.key || provider.type || 'unknown provider'}</strong>
                      <UIBadge className={provider.ready ? 'is-connected' : 'is-idle'}>
                        {provider.ready ? 'Ready' : 'Not ready'}
                      </UIBadge>
                    </UICardHeader>
                    <UICardContent className="thread-overview-card-body">
                      <div className="small-note">type {String(provider.type || '--')}</div>
                      <div className="small-note">active runs {String(provider.active_runs || 0)}</div>
                    </UICardContent>
                  </UICard>
                ))}
              </div>
            )}
          </section>

            <section className="thread-history-panel web-status-json-grid">
            <UICard>
              <UICardHeader>
                <UIBadge>Overview JSON</UIBadge>
                <UICardDescription>Gateway runtime snapshot from `/api/overview`.</UICardDescription>
              </UICardHeader>
              <UICardContent>
                <pre className="web-status-json-box">{stringifyJson(overview)}</pre>
              </UICardContent>
            </UICard>
            <UICard>
              <UICardHeader>
                <UIBadge>Agent View JSON</UIBadge>
                <UICardDescription>Bridge provider snapshot from `/api/agent-view`.</UICardDescription>
              </UICardHeader>
              <UICardContent>
                <pre className="web-status-json-box">{stringifyJson(agentView)}</pre>
              </UICardContent>
            </UICard>
          </section>
          </>
        )}
      </div>
    </div>
  );
}
