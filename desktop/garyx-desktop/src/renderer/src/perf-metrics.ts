const PERF_LOG_THRESHOLD_MS = 80;
const API_LOG_THRESHOLD_MS = 120;
const LONG_TASK_THRESHOLD_MS = 80;
const FRAME_STALL_THRESHOLD_MS = 120;
const MEMORY_SAMPLE_INTERVAL_MS = 5_000;
const MAX_PERFORMANCE_EVENTS = 180;
const RECENT_HEALTH_WINDOW_MS = 60_000;
const API_PERFORMANCE_MESSAGE_SOURCE = 'garyx-desktop-api-performance';

type PerformanceWithMemory = Performance & {
  memory?: {
    usedJSHeapSize: number;
    totalJSHeapSize: number;
    jsHeapSizeLimit: number;
  };
};

export type RendererPerformanceSeverity = 'info' | 'warning' | 'critical';

export type RendererPerformanceEventKind =
  | 'api_call'
  | 'frame_stall'
  | 'long_task'
  | 'memory_warning'
  | 'ui_action';

export type RendererMemorySample = {
  sampledAt: number;
  usedJSHeapSize: number;
  totalJSHeapSize: number;
  jsHeapSizeLimit: number;
};

export type RendererPerformanceEvent = {
  key: string;
  timestamp: string;
  recordedAt: number;
  kind: RendererPerformanceEventKind;
  label: string;
  severity: RendererPerformanceSeverity;
  durationMs?: number;
  summary: string;
  detail: string;
};

export type RendererPerformanceSnapshot = {
  startedAt: number;
  generatedAt: number;
  status: 'nominal' | 'watch' | 'critical';
  summary: string;
  desktopApiMonitorInstalled: boolean;
  events: RendererPerformanceEvent[];
  latestMemory: RendererMemorySample | null;
  totals: Record<RendererPerformanceEventKind, number>;
};

type RendererPerformanceSubscriber = (
  snapshot: RendererPerformanceSnapshot,
) => void;

const performanceEvents: RendererPerformanceEvent[] = [];
const performanceSubscribers = new Set<RendererPerformanceSubscriber>();
const performanceTotals: Record<RendererPerformanceEventKind, number> = {
  api_call: 0,
  frame_stall: 0,
  long_task: 0,
  memory_warning: 0,
  ui_action: 0,
};

let monitorStarted = false;
let monitorStartedAt = Date.now();
let performanceEventSequence = 1;
let latestMemorySample: RendererMemorySample | null = null;
let lastMemoryWarningAt = 0;
let desktopApiProxyInstalled = false;
let desktopApiMessageListenerInstalled = false;
let proxiedDesktopApi: unknown = null;

type DesktopApiPerformanceMessage = {
  source: typeof API_PERFORMANCE_MESSAGE_SOURCE;
  method: string;
  durationMs: number;
  ok: boolean;
  errorMessage?: string;
};

function markName(label: string, suffix: string): string {
  return `garyx:${label}:${suffix}:${Math.random().toString(36).slice(2)}`;
}

function formatClock(value: number): string {
  const date = new Date(value);
  const hours = String(date.getHours()).padStart(2, '0');
  const minutes = String(date.getMinutes()).padStart(2, '0');
  const seconds = String(date.getSeconds()).padStart(2, '0');
  const milliseconds = String(date.getMilliseconds()).padStart(3, '0');
  return `${hours}:${minutes}:${seconds}.${milliseconds}`;
}

function formatDuration(durationMs: number): string {
  return `${durationMs.toFixed(durationMs >= 100 ? 0 : 1)}ms`;
}

function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) {
    return '0 MB';
  }
  const mib = value / 1024 / 1024;
  if (mib < 1024) {
    return `${mib.toFixed(mib >= 100 ? 0 : 1)} MB`;
  }
  return `${(mib / 1024).toFixed(2)} GB`;
}

function detailPayload(payload: Record<string, unknown>): string {
  try {
    return JSON.stringify(payload, null, 2);
  } catch {
    return String(payload);
  }
}

function eventSeverityForDuration(
  durationMs: number,
): RendererPerformanceSeverity {
  if (durationMs >= 500) {
    return 'critical';
  }
  if (durationMs >= PERF_LOG_THRESHOLD_MS) {
    return 'warning';
  }
  return 'info';
}

function computeSnapshotStatus(events: RendererPerformanceEvent[]): {
  status: RendererPerformanceSnapshot['status'];
  summary: string;
} {
  const recentCutoff = Date.now() - RECENT_HEALTH_WINDOW_MS;
  const recentEvents = events.filter((event) => event.recordedAt >= recentCutoff);
  const criticalCount = recentEvents.filter((event) => event.severity === 'critical').length;
  const warningCount = recentEvents.filter((event) => event.severity === 'warning').length;

  if (criticalCount > 0) {
    return {
      status: 'critical',
      summary: `${criticalCount} critical renderer performance event${criticalCount === 1 ? '' : 's'} in the last minute.`,
    };
  }
  if (warningCount > 0) {
    return {
      status: 'watch',
      summary: `${warningCount} renderer performance warning${warningCount === 1 ? '' : 's'} in the last minute.`,
    };
  }
  return {
    status: 'nominal',
    summary: 'No renderer performance warnings in the last minute.',
  };
}

export function getRendererPerformanceSnapshot(): RendererPerformanceSnapshot {
  const health = computeSnapshotStatus(performanceEvents);
  return {
    startedAt: monitorStartedAt,
    generatedAt: Date.now(),
    status: health.status,
    summary: health.summary,
    desktopApiMonitorInstalled: isDesktopApiPerformanceMonitorInstalled(),
    events: [...performanceEvents],
    latestMemory: latestMemorySample ? { ...latestMemorySample } : null,
    totals: { ...performanceTotals },
  };
}

function emitPerformanceSnapshot(): void {
  if (performanceSubscribers.size === 0) {
    return;
  }
  const snapshot = getRendererPerformanceSnapshot();
  performanceSubscribers.forEach((subscriber) => {
    subscriber(snapshot);
  });
}

function recordPerformanceEvent(input: {
  detail: string;
  durationMs?: number;
  kind: RendererPerformanceEventKind;
  label: string;
  severity: RendererPerformanceSeverity;
  summary: string;
}): void {
  const recordedAt = Date.now();
  const event: RendererPerformanceEvent = {
    key: `renderer-performance-${performanceEventSequence++}`,
    timestamp: formatClock(recordedAt),
    recordedAt,
    kind: input.kind,
    label: input.label,
    severity: input.severity,
    durationMs: input.durationMs,
    summary: input.summary,
    detail: input.detail,
  };
  performanceEvents.push(event);
  if (performanceEvents.length > MAX_PERFORMANCE_EVENTS) {
    performanceEvents.splice(0, performanceEvents.length - MAX_PERFORMANCE_EVENTS);
  }
  performanceTotals[input.kind] += 1;
  emitPerformanceSnapshot();
}

function recordMemorySample(): void {
  if (typeof performance === 'undefined') {
    return;
  }
  const memory = (performance as PerformanceWithMemory).memory;
  if (!memory) {
    return;
  }

  latestMemorySample = {
    sampledAt: Date.now(),
    usedJSHeapSize: memory.usedJSHeapSize,
    totalJSHeapSize: memory.totalJSHeapSize,
    jsHeapSizeLimit: memory.jsHeapSizeLimit,
  };

  const heapRatio = memory.jsHeapSizeLimit > 0
    ? memory.usedJSHeapSize / memory.jsHeapSizeLimit
    : 0;
  const now = Date.now();
  if (
    heapRatio >= 0.7 &&
    now - lastMemoryWarningAt > RECENT_HEALTH_WINDOW_MS
  ) {
    lastMemoryWarningAt = now;
    recordPerformanceEvent({
      kind: 'memory_warning',
      label: 'js_heap',
      severity: heapRatio >= 0.85 ? 'critical' : 'warning',
      summary: `JS heap ${formatBytes(memory.usedJSHeapSize)} / ${formatBytes(memory.jsHeapSizeLimit)}`,
      detail: detailPayload({
        type: 'memory_warning',
        usedJSHeapSize: memory.usedJSHeapSize,
        totalJSHeapSize: memory.totalJSHeapSize,
        jsHeapSizeLimit: memory.jsHeapSizeLimit,
        heapRatio: Number(heapRatio.toFixed(4)),
      }),
    });
  } else {
    emitPerformanceSnapshot();
  }
}

function startLongTaskObserver(): void {
  if (typeof PerformanceObserver === 'undefined') {
    return;
  }
  try {
    const observer = new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        if (entry.duration < LONG_TASK_THRESHOLD_MS) {
          continue;
        }
        recordPerformanceEvent({
          kind: 'long_task',
          label: entry.name || 'main_thread',
          durationMs: entry.duration,
          severity: eventSeverityForDuration(entry.duration),
          summary: `Main thread blocked for ${formatDuration(entry.duration)}`,
          detail: detailPayload({
            type: 'long_task',
            name: entry.name,
            startTime: Number(entry.startTime.toFixed(2)),
            durationMs: Number(entry.duration.toFixed(2)),
          }),
        });
      }
    });
    observer.observe({ entryTypes: ['longtask'] });
  } catch {
    // Some Electron/Chromium builds do not expose longtask entries.
  }
}

function startFrameStallLoop(): void {
  if (typeof window === 'undefined' || typeof window.requestAnimationFrame !== 'function') {
    return;
  }
  let previousFrameTime = performance.now();
  const tick = (frameTime: number) => {
    const delta = frameTime - previousFrameTime;
    previousFrameTime = frameTime;
    if (
      delta >= FRAME_STALL_THRESHOLD_MS &&
      typeof document !== 'undefined' &&
      document.visibilityState !== 'hidden'
    ) {
      recordPerformanceEvent({
        kind: 'frame_stall',
        label: 'requestAnimationFrame',
        durationMs: delta,
        severity: eventSeverityForDuration(delta),
        summary: `Frame gap ${formatDuration(delta)}`,
        detail: detailPayload({
          type: 'frame_stall',
          frameGapMs: Number(delta.toFixed(2)),
          thresholdMs: FRAME_STALL_THRESHOLD_MS,
        }),
      });
    }
    window.requestAnimationFrame(tick);
  };
  window.requestAnimationFrame(tick);
}

function monitorApiResult<T>(
  methodName: string,
  start: number,
  result: Promise<T>,
): Promise<T> {
  return result.then(
    (value) => {
      recordDesktopApiTiming(methodName, performance.now() - start, true);
      return value;
    },
    (error) => {
      recordDesktopApiTiming(methodName, performance.now() - start, false, error);
      throw error;
    },
  );
}

function recordDesktopApiTiming(
  methodName: string,
  durationMs: number,
  ok: boolean,
  error?: unknown,
): void {
  if (ok && durationMs < API_LOG_THRESHOLD_MS) {
    return;
  }
  const severity = ok ? eventSeverityForDuration(durationMs) : 'warning';
  const errorMessage = error instanceof Error ? error.message : error ? String(error) : undefined;
  recordPerformanceEvent({
    kind: 'api_call',
    label: methodName,
    durationMs,
    severity,
    summary: ok
      ? `${methodName} took ${formatDuration(durationMs)}`
      : `${methodName} failed after ${formatDuration(durationMs)}`,
    detail: detailPayload({
      type: 'api_call',
      method: methodName,
      durationMs: Number(durationMs.toFixed(2)),
      ok,
      error: errorMessage,
    }),
  });
}

function isDesktopApiPerformanceMessage(
  value: unknown,
): value is DesktopApiPerformanceMessage {
  if (!value || typeof value !== 'object') {
    return false;
  }
  const candidate = value as Partial<DesktopApiPerformanceMessage>;
  return candidate.source === API_PERFORMANCE_MESSAGE_SOURCE &&
    typeof candidate.method === 'string' &&
    typeof candidate.durationMs === 'number' &&
    Number.isFinite(candidate.durationMs) &&
    typeof candidate.ok === 'boolean';
}

function startDesktopApiPerformanceMessageListener(): void {
  if (
    desktopApiMessageListenerInstalled ||
    typeof window === 'undefined' ||
    typeof window.addEventListener !== 'function'
  ) {
    return;
  }
  window.addEventListener('message', (event) => {
    if (!isDesktopApiPerformanceMessage(event.data)) {
      return;
    }
    recordDesktopApiTiming(
      event.data.method,
      event.data.durationMs,
      event.data.ok,
      event.data.errorMessage,
    );
  });
  desktopApiMessageListenerInstalled = true;
}

function installPerformanceDebugHandle(): void {
  if (typeof window === 'undefined') {
    return;
  }
  const handle = {
    getSnapshot: getRendererPerformanceSnapshot,
    isDesktopApiMonitorInstalled: isDesktopApiPerformanceMonitorInstalled,
  };
  try {
    Object.defineProperty(window, '__garyxPerformanceMonitor', {
      configurable: true,
      enumerable: false,
      value: handle,
    });
  } catch {
    window.__garyxPerformanceMonitor = handle;
  }
}

export function installDesktopApiPerformanceMonitor(): void {
  startDesktopApiPerformanceMessageListener();
  installPerformanceDebugHandle();
  if (
    desktopApiProxyInstalled ||
    typeof window === 'undefined' ||
    !window.garyxDesktop
  ) {
    return;
  }

  const descriptor = Object.getOwnPropertyDescriptor(window, 'garyxDesktop');
  if (descriptor && descriptor.writable === false) {
    return;
  }

  const sourceApi = window.garyxDesktop;
  const wrappedMethods = new Map<PropertyKey, unknown>();
  const proxy = new Proxy(sourceApi, {
    get(target, property, receiver) {
      const value = Reflect.get(target, property, receiver);
      if (typeof property !== 'string' || typeof value !== 'function') {
        return value;
      }
      if (property.startsWith('subscribe') || property.startsWith('unsubscribe')) {
        return value.bind(target);
      }
      if (wrappedMethods.has(property)) {
        return wrappedMethods.get(property);
      }
      const wrapped = (...args: unknown[]) => {
        const start = performance.now();
        try {
          const result = value.apply(target, args);
          if (result && typeof result.then === 'function') {
            return monitorApiResult(property, start, result);
          }
          recordDesktopApiTiming(property, performance.now() - start, true);
          return result;
        } catch (error) {
          recordDesktopApiTiming(property, performance.now() - start, false, error);
          throw error;
        }
      };
      wrappedMethods.set(property, wrapped);
      return wrapped;
    },
  });

  try {
    window.garyxDesktop = proxy;
    proxiedDesktopApi = proxy;
    desktopApiProxyInstalled = true;
  } catch {
    proxiedDesktopApi = null;
  }
}

export function startRendererPerformanceMonitor(): void {
  if (monitorStarted) {
    return;
  }
  monitorStarted = true;
  monitorStartedAt = Date.now();
  installPerformanceDebugHandle();
  startDesktopApiPerformanceMessageListener();
  startLongTaskObserver();
  startFrameStallLoop();
  recordMemorySample();
  if (typeof window !== 'undefined') {
    window.setInterval(recordMemorySample, MEMORY_SAMPLE_INTERVAL_MS);
  }
}

export function subscribeRendererPerformance(
  subscriber: RendererPerformanceSubscriber,
): () => void {
  performanceSubscribers.add(subscriber);
  subscriber(getRendererPerformanceSnapshot());
  return () => {
    performanceSubscribers.delete(subscriber);
  };
}

export async function measureUiAction<T>(
  label: string,
  task: () => Promise<T> | T,
): Promise<T> {
  const startMark = markName(label, "start");
  const endMark = markName(label, "end");
  const start = performance.now();
  performance.mark(startMark);
  try {
    return await task();
  } finally {
    const duration = performance.now() - start;
    performance.mark(endMark);
    try {
      performance.measure(`garyx:${label}`, startMark, endMark);
      performance.clearMarks(startMark);
      performance.clearMarks(endMark);
    } catch {
      // Performance marks are best-effort diagnostics only.
    }
    if (duration >= PERF_LOG_THRESHOLD_MS) {
      console.info(`[garyx:perf] ${label} ${duration.toFixed(1)}ms`);
      recordPerformanceEvent({
        kind: 'ui_action',
        label,
        durationMs: duration,
        severity: eventSeverityForDuration(duration),
        summary: `${label} took ${formatDuration(duration)}`,
        detail: detailPayload({
          type: 'ui_action',
          label,
          durationMs: Number(duration.toFixed(2)),
        }),
      });
    }
  }
}

export function isDesktopApiPerformanceMonitorInstalled(): boolean {
  return desktopApiMessageListenerInstalled ||
    (desktopApiProxyInstalled && window.garyxDesktop === proxiedDesktopApi);
}
