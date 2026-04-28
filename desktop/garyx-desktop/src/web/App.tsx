import { BotConsoleView } from '@renderer/BotConsoleView';
import { WebCronPage } from '@renderer/WebCronPage';
import { WebHeartbeatPage } from '@renderer/WebHeartbeatPage';
import { ThreadsListPage } from '@renderer/ThreadsListPage';
import { WebLogsPage } from '@renderer/WebLogsPage';
import { WebSettingsPage } from '@renderer/WebSettingsPage';
import { WebStatusPage } from '@renderer/WebStatusPage';
import { useBotConsoleState } from './use-bot-console-state';
import { useThreadsListState } from './use-threads-list-state';
import { useWebCronState } from './use-web-cron-state';
import { useWebHeartbeatState } from './use-web-heartbeat-state';
import { useWebLogsState } from './use-web-logs-state';
import { useWebSettingsState } from './use-web-settings-state';
import { useWebStatusState } from './use-web-status-state';
import { buildWebRouteHref, resolveWebRoute, type WebRoute } from './web-route';

export function WebBotConsoleApp() {
  const route = resolveWebRoute();

  if (route.view === 'threads') {
    return <WebThreadsListView route={route} />;
  }

  if (route.view === 'settings') {
    return <WebSettingsView route={route} />;
  }

  if (route.view === 'status') {
    return <WebStatusView route={route} />;
  }

  if (route.view === 'logs') {
    return <WebLogsView route={route} />;
  }

  if (route.view === 'cron') {
    return <WebCronView route={route} />;
  }

  if (route.view === 'heartbeat') {
    return <WebHeartbeatView route={route} />;
  }

  return <WebBotConsoleView route={route} />;
}

function WebBotConsoleView({ route }: { route: Extract<WebRoute, { view: 'bot-console' }> }) {
  const {
    groups,
    loading,
    error,
    status,
    totalEndpoints,
    refresh,
  } = useBotConsoleState(route);

  function handleOpenBot(botId: string) {
    window.location.href = buildWebRouteHref({
      view: 'bot-console',
      botId,
    });
  }

  function handleOpenSettings() {
    window.location.href = buildWebRouteHref({
      view: 'settings',
      botId: route.botId,
    });
  }

  if (loading) {
    return (
      <div className="bot-console-shell">
        <div className="empty-state">
          <span className="eyebrow">Bot Console</span>
          <h3>Loading bots</h3>
        </div>
      </div>
    );
  }

  return (
    <div className="bot-console-shell">
      {error ? (
        <div className="bot-console-error" role="alert">
          {error}
        </div>
      ) : null}
      <BotConsoleView
        focusedBotId={route.botId}
        focusedEndpointKey={route.endpointKey}
        groups={groups}
        onOpenBot={handleOpenBot}
        onOpenSettings={handleOpenSettings}
        onRefresh={() => {
          void refresh();
        }}
        status={status}
        totalEndpoints={totalEndpoints}
        toolbarNote={
          route.botId || route.endpointKey
            ? `Deep link: ${route.botId || 'any bot'}${route.endpointKey ? ` · endpoint ${route.endpointKey}` : ''}`
            : null
        }
      />
    </div>
  );
}

function WebThreadsListView({ route }: { route: Extract<WebRoute, { view: 'threads' }> }) {
  const {
    visibleThreads,
    loading,
    error,
    filter,
    setFilter,
    refresh,
    normalThreadsCount,
    heartbeatThreadsCount,
    totalThreadsCount,
  } = useThreadsListState(route);

  return (
    <ThreadsListPage
      error={error}
      filter={filter}
      heartbeatThreadsCount={heartbeatThreadsCount}
      loading={loading}
      normalThreadsCount={normalThreadsCount}
      onFilterChange={setFilter}
      onRefresh={() => {
        void refresh();
      }}
      threads={visibleThreads}
      totalThreadsCount={totalThreadsCount}
    />
  );
}

function WebSettingsView({ route }: { route: Extract<WebRoute, { view: 'settings' }> }) {
  const settingsState = useWebSettingsState(route);

  return (
    <WebSettingsPage
      focusedBotId={route.botId}
      focusedBotSummary={settingsState.focusedBotSummary}
      error={settingsState.error}
      jsonDraft={settingsState.jsonDraft}
      loading={settingsState.loading}
      onAddFeishuAccount={settingsState.addFeishuAccount}
      onAddTelegramAccount={settingsState.addTelegramAccount}
      onChangeJson={settingsState.setJsonDraft}
      onPatchFeishuAccount={settingsState.patchFeishuAccount}
      onPatchGateway={settingsState.patchGateway}
      onPatchHeartbeat={settingsState.patchHeartbeat}
      onPatchSessions={settingsState.patchSessions}
      onPatchTelegramAccount={settingsState.patchTelegramAccount}
      onRefresh={() => {
        void settingsState.refresh();
      }}
      onRemoveFeishuAccount={settingsState.removeFeishuAccount}
      onRemoveTelegramAccount={settingsState.removeTelegramAccount}
      onSave={() => {
        void settingsState.save();
      }}
      payload={settingsState.payload}
      saving={settingsState.saving}
      status={settingsState.status}
    />
  );
}

function WebStatusView({ route }: { route: Extract<WebRoute, { view: 'status' }> }) {
  const statusState = useWebStatusState(route);

  return (
    <WebStatusPage
      agentView={statusState.agentView}
      error={statusState.error}
      loading={statusState.loading}
      onRefresh={() => {
        void statusState.refresh();
      }}
      overview={statusState.overview}
    />
  );
}

function WebLogsView({ route }: { route: Extract<WebRoute, { view: 'logs' }> }) {
  const logsState = useWebLogsState(route);

  return (
    <WebLogsPage
      error={logsState.error}
      level={logsState.level}
      lines={logsState.lines}
      loading={logsState.loading}
      onLevelChange={logsState.setLevel}
      onRefresh={() => {
        void logsState.refresh();
      }}
      path={logsState.payload?.path || null}
      totalLines={logsState.payload?.total_lines ?? null}
    />
  );
}

function WebCronView({ route }: { route: Extract<WebRoute, { view: 'cron' }> }) {
  const cronState = useWebCronState(route);

  return (
    <WebCronPage
      avgDurationMs={cronState.avgDurationMs}
      error={cronState.error}
      jobs={cronState.jobsPayload?.jobs || []}
      loading={cronState.loading}
      maxDurationMs={cronState.maxDurationMs}
      onRefresh={() => {
        void cronState.refresh();
      }}
      runs={cronState.runsPayload?.runs || []}
      totalJobs={cronState.jobsPayload?.count || (cronState.jobsPayload?.jobs || []).length}
      totalRuns={cronState.runsPayload?.total || (cronState.runsPayload?.runs || []).length}
    />
  );
}

function WebHeartbeatView({ route }: { route: Extract<WebRoute, { view: 'heartbeat' }> }) {
  const heartbeatState = useWebHeartbeatState(route);

  return (
    <WebHeartbeatPage
      error={heartbeatState.error}
      loading={heartbeatState.loading}
      onRefresh={() => {
        void heartbeatState.refresh();
      }}
      onTrigger={() => {
        void heartbeatState.trigger();
      }}
      summary={heartbeatState.summary}
      triggering={heartbeatState.triggering}
    />
  );
}
