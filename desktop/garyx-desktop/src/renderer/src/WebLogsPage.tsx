import {
  UIButton,
  UIBadge,
  UICard,
  UICardContent,
  UICardDescription,
  UICardHeader,
  UICardTitle,
} from './ui';
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select';

type ParsedLogLine = {
  level: string;
  timestamp: string;
  message: string;
};

type WebLogsPageProps = {
  lines: ParsedLogLine[];
  loading: boolean;
  error?: string | null;
  level: string;
  path?: string | null;
  totalLines?: number | null;
  onLevelChange: (level: string) => void;
  onRefresh?: () => void;
};

const ALL_LOG_LEVELS_VALUE = '__all_log_levels__';

function levelTone(level: string): 'status-connected' | 'status-idle' {
  const normalized = level.toUpperCase();
  return normalized === 'ERROR' || normalized === 'WARN' || normalized === 'WARNING'
    ? 'status-idle'
    : 'status-connected';
}

export function WebLogsPage({
  lines,
  loading,
  error,
  level,
  path,
  totalLines,
  onLevelChange,
  onRefresh,
}: WebLogsPageProps) {
  return (
    <div className="thread-history-shell">
      <div className="thread-history-page shadcn-shell">
        <section className="shadcn-hero">
          <div className="shadcn-hero-copy">
            <p className="shadcn-kicker">Logs</p>
            <h1>Gateway log tail</h1>
            <p className="shadcn-subcopy">
              {path || 'default log path'}
              {totalLines != null ? ` · ${totalLines} lines` : ''}
            </p>
          </div>
          <div className="shadcn-hero-actions">
            <Select
              value={level || ALL_LOG_LEVELS_VALUE}
              onValueChange={(value) => {
                onLevelChange(value === ALL_LOG_LEVELS_VALUE ? '' : value);
              }}
            >
              <SelectTrigger className="composer-provider-select">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  <SelectItem value={ALL_LOG_LEVELS_VALUE}>All levels</SelectItem>
                  <SelectItem value="ERROR">ERROR</SelectItem>
                  <SelectItem value="WARNING">WARNING</SelectItem>
                  <SelectItem value="INFO">INFO</SelectItem>
                  <SelectItem value="DEBUG">DEBUG</SelectItem>
                </SelectGroup>
              </SelectContent>
            </Select>
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
            <span className="eyebrow">Logs</span>
            <h3>Loading log tail</h3>
          </div>
        ) : !lines.length ? (
          <div className="empty-state">
            <span className="eyebrow">Logs</span>
            <h3>No log lines found</h3>
            <p>No lines matched the current filter.</p>
          </div>
        ) : (
          <section className="thread-history-panel">
            <UICard>
              <UICardHeader>
                <UIBadge>Log stream</UIBadge>
                <UICardTitle>{lines.length} rendered entries</UICardTitle>
                <UICardDescription>
                  Filter {level || 'all levels'} · source {path || 'default log path'}
                </UICardDescription>
              </UICardHeader>
              <UICardContent>
                <div className="web-log-list">
                  {lines.map((line, index) => (
                    <article className="web-log-item" key={`${line.timestamp}-${index}`}>
                      <div className="thread-history-message-meta">
                        <UIBadge className={levelTone(line.level) === 'status-connected' ? 'is-connected' : 'is-idle'}>
                          {line.level}
                        </UIBadge>
                        <span>{line.timestamp}</span>
                      </div>
                      <pre className="web-log-message">{line.message}</pre>
                    </article>
                  ))}
                </div>
              </UICardContent>
            </UICard>
          </section>
        )}
      </div>
    </div>
  );
}
