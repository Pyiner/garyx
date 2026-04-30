const PERF_LOG_THRESHOLD_MS = 80;

function markName(label: string, suffix: string): string {
  return `garyx:${label}:${suffix}:${Math.random().toString(36).slice(2)}`;
}

export async function measureUiAction<T>(
  label: string,
  task: () => Promise<T>,
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
    }
  }
}
