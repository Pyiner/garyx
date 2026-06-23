import { RendererPerformancePanel } from 'garyx-desktop';

const MB = 1024 * 1024;
const START = 1735689600000; // fixed clock so the footnote is deterministic
const GEN = START + 184_000;

const snapshot = {
  startedAt: START,
  generatedAt: GEN,
  status: 'watch',
  summary: '2 renderer performance warnings in the last minute.',
  desktopApiMonitorInstalled: true,
  totals: { api_call: 4, frame_stall: 2, long_task: 1, memory_warning: 0, ui_action: 12 },
  latestMemory: {
    sampledAt: GEN,
    usedJSHeapSize: 186 * MB,
    totalJSHeapSize: 242 * MB,
    jsHeapSizeLimit: 2048 * MB,
  },
  events: [
    {
      key: 'evt-1', timestamp: '12:03:01', recordedAt: GEN - 8000, kind: 'long_task',
      label: 'Thread render', severity: 'warning', durationMs: 184,
      summary: 'Long task during thread switch', detail: 'Main thread blocked 184ms while mounting ThreadPage.',
    },
    {
      key: 'evt-2', timestamp: '12:02:46', recordedAt: GEN - 18000, kind: 'api_call',
      label: 'gateway /api/threads', severity: 'warning', durationMs: 920,
      summary: 'Slow gateway call (920ms)', detail: 'GET /api/threads took 920ms — above the 500ms budget.',
    },
    {
      key: 'evt-3', timestamp: '12:02:10', recordedAt: GEN - 54000, kind: 'frame_stall',
      label: 'Scroll', severity: 'info', durationMs: 64,
      summary: 'Frame stall during scroll', detail: '64ms requestAnimationFrame gap while scrolling the transcript.',
    },
  ],
} as any;

export const Watch = () => (
  <div style={{ padding: 16, maxWidth: 520 }}>
    <RendererPerformancePanel snapshot={snapshot} />
  </div>
);
