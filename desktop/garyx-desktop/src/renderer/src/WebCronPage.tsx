type CronJob = {
  id?: string;
  schedule?: {
    cron?: string;
    interval_secs?: number;
    at?: string;
  } | string | null;
  enabled?: boolean;
  next_run?: string | null;
  last_run_at?: string | null;
  last_status?: string | null;
  run_count?: number;
};

type CronRun = {
  run_id?: string;
  job_id?: string;
  status?: string;
  started_at?: string | null;
  finished_at?: string | null;
  duration_ms?: number | null;
  error?: string | null;
};

type WebCronPageProps = {
  jobs: CronJob[];
  runs: CronRun[];
  loading: boolean;
  error?: string | null;
  totalJobs: number;
  totalRuns: number;
  avgDurationMs?: number | null;
  maxDurationMs?: number | null;
  onRefresh?: () => void;
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

function formatDuration(value?: number | null): string {
  if (value == null || !Number.isFinite(value)) {
    return '--';
  }
  if (value < 1000) {
    return `${Math.round(value)}ms`;
  }
  return `${(value / 1000).toFixed(2)}s`;
}

function formatSchedule(schedule?: CronJob['schedule']): string {
  if (!schedule) {
    return '--';
  }
  if (typeof schedule === 'string') {
    return schedule;
  }
  if (schedule.cron) {
    return `cron: ${schedule.cron}`;
  }
  if (schedule.interval_secs) {
    return `every ${schedule.interval_secs}s`;
  }
  if (schedule.at) {
    return `at: ${schedule.at}`;
  }
  return '--';
}

export function WebCronPage({
  jobs,
  runs,
  loading,
  error,
  totalJobs,
  totalRuns,
  avgDurationMs,
  maxDurationMs,
  onRefresh,
}: WebCronPageProps) {
  const enabledJobs = jobs.filter((job) => job.enabled).length;

  return (
    <div className="thread-history-shell">
      <div className="thread-history-page shadcn-shell">
        <section className="shadcn-hero">
          <div className="shadcn-hero-copy">
            <p className="shadcn-kicker">Cron</p>
            <h1>Scheduled jobs overview</h1>
            <p className="shadcn-subcopy">
              {totalJobs} jobs · {enabledJobs} enabled · {totalRuns} recent runs
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
            <span className="eyebrow">Cron</span>
            <h3>Loading cron data</h3>
          </div>
        ) : (
          <>
            <section className="thread-history-panel">
              <div className="web-settings-summary-grid">
                <UICard className="web-settings-summary-card">
                  <UICardHeader>
                    <UIBadge>Jobs</UIBadge>
                    <UICardTitle>{totalJobs}</UICardTitle>
                    <UICardDescription>{enabledJobs} enabled</UICardDescription>
                  </UICardHeader>
                </UICard>
                <UICard className="web-settings-summary-card">
                  <UICardHeader>
                    <UIBadge>Runs</UIBadge>
                    <UICardTitle>{totalRuns}</UICardTitle>
                    <UICardDescription>Recent execution records</UICardDescription>
                  </UICardHeader>
                </UICard>
                <UICard className="web-settings-summary-card">
                  <UICardHeader>
                    <UIBadge>Avg Duration</UIBadge>
                    <UICardTitle>{formatDuration(avgDurationMs)}</UICardTitle>
                    <UICardDescription>Across visible runs</UICardDescription>
                  </UICardHeader>
                </UICard>
                <UICard className="web-settings-summary-card">
                  <UICardHeader>
                    <UIBadge>Max Duration</UIBadge>
                    <UICardTitle>{formatDuration(maxDurationMs)}</UICardTitle>
                    <UICardDescription>Across visible runs</UICardDescription>
                  </UICardHeader>
                </UICard>
              </div>
            </section>

            <section className="thread-history-panel">
              <div className="thread-history-toolbar-copy">
                <span className="eyebrow">Jobs</span>
                <p>Current scheduled cron jobs from the gateway.</p>
              </div>
              {!jobs.length ? (
                <div className="empty-state">
                  <span className="eyebrow">Jobs</span>
                  <h3>No cron jobs found</h3>
                </div>
              ) : (
                <div className="web-cron-table-wrap">
                  <table className="web-cron-table">
                    <thead>
                      <tr>
                        <th>Job</th>
                        <th>Schedule</th>
                        <th>Status</th>
                        <th>Last run</th>
                        <th>Next run</th>
                      </tr>
                    </thead>
                    <tbody>
                      {jobs.map((job) => (
                        <tr key={job.id || formatSchedule(job.schedule)}>
                          <td>
                            <strong>{job.id || '--'}</strong>
                            <div className="small-note">runs {String(job.run_count || 0)}</div>
                          </td>
                          <td>{formatSchedule(job.schedule)}</td>
                          <td>
                            <span className={`bot-console-status ${job.enabled ? 'status-connected' : 'status-idle'}`}>
                              {job.enabled ? 'Enabled' : 'Disabled'}
                            </span>
                            {' '}
                            <span className="small-note">{String(job.last_status || '--')}</span>
                          </td>
                          <td>{formatTime(job.last_run_at)}</td>
                          <td>{formatTime(job.next_run)}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </section>

            <section className="thread-history-panel">
              <div className="thread-history-toolbar-copy">
                <span className="eyebrow">Runs</span>
                <p>Recent cron execution history.</p>
              </div>
              {!runs.length ? (
                <div className="empty-state">
                  <span className="eyebrow">Runs</span>
                  <h3>No cron runs found</h3>
                </div>
              ) : (
                <div className="web-cron-table-wrap">
                  <table className="web-cron-table">
                    <thead>
                      <tr>
                        <th>Started</th>
                        <th>Job</th>
                        <th>Status</th>
                        <th>Duration</th>
                        <th>Error</th>
                      </tr>
                    </thead>
                    <tbody>
                      {runs.map((run) => (
                        <tr key={run.run_id || `${run.job_id}-${run.started_at}`}>
                          <td>{formatTime(run.started_at)}</td>
                          <td>
                            <strong>{run.job_id || '--'}</strong>
                            <div className="small-note">{run.run_id || '--'}</div>
                          </td>
                          <td>
                            <span className={`bot-console-status ${run.status === 'ok' || run.status === 'success' ? 'status-connected' : 'status-idle'}`}>
                              {String(run.status || '--')}
                            </span>
                          </td>
                          <td>{formatDuration(run.duration_ms)}</td>
                          <td>{run.error || '--'}</td>
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
