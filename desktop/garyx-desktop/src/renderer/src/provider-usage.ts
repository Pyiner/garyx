export const MODEL_PROVIDER_USAGE_PROVIDER_IDS = {
  claude_code: 'claude_code',
  codex_app_server: 'codex',
  antigravity: 'antigravity',
} as const;

export type ModelProviderUsageKey = keyof typeof MODEL_PROVIDER_USAGE_PROVIDER_IDS;
export type ProviderUsageLevel = 'healthy' | 'warning' | 'critical' | 'unavailable';

export const PROVIDER_USAGE_REMAINING_THRESHOLDS = {
  healthy: 50,
  warning: 20,
} as const;

export function usageProviderIdForModelProviderKey(
  key: string,
): string | undefined {
  return MODEL_PROVIDER_USAGE_PROVIDER_IDS[key as ModelProviderUsageKey];
}

export function clampUsagePercent(value: number | null | undefined): number {
  return Number.isFinite(value) ? Math.max(0, Math.min(100, value as number)) : 0;
}

export function usageLevelForRemainingPercent(
  remainingPercent: number | null | undefined,
  available = true,
): ProviderUsageLevel {
  if (!available) {
    return 'unavailable';
  }
  const percent = clampUsagePercent(remainingPercent);
  if (percent >= PROVIDER_USAGE_REMAINING_THRESHOLDS.healthy) {
    return 'healthy';
  }
  if (percent >= PROVIDER_USAGE_REMAINING_THRESHOLDS.warning) {
    return 'warning';
  }
  return 'critical';
}

export function formatUsagePercent(value: number | null | undefined): string {
  return `${Math.round(clampUsagePercent(value))}%`;
}

export function formatUsageDuration(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const days = Math.floor(total / 86_400);
  const hours = Math.floor((total % 86_400) / 3_600);
  const minutes = Math.floor((total % 3_600) / 60);
  if (days >= 1) {
    return hours > 0 ? `${days}d ${hours}h` : `${days}d`;
  }
  if (hours >= 1) {
    return minutes > 0 ? `${hours}h ${minutes}m` : `${hours}h`;
  }
  if (minutes >= 1) {
    return `${minutes}m`;
  }
  return '<1m';
}

export function resetSecondsFromIso(
  value?: string | null,
  nowMs = Date.now(),
): number | null {
  if (!value) {
    return null;
  }
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) {
    return null;
  }
  return Math.max(0, Math.floor((timestamp - nowMs) / 1000));
}

export function usageResetSeconds(
  resetsAt?: string | null,
  resetAfterSeconds?: number | null,
  nowMs = Date.now(),
): number | null {
  const candidates: number[] = [];
  if (typeof resetAfterSeconds === 'number' && Number.isFinite(resetAfterSeconds)) {
    candidates.push(Math.max(0, Math.floor(resetAfterSeconds)));
  }
  const isoSeconds = resetSecondsFromIso(resetsAt, nowMs);
  if (isoSeconds !== null) {
    candidates.push(isoSeconds);
  }
  if (candidates.length === 0) {
    return null;
  }
  return Math.min(...candidates);
}

export function usageResetText(
  resetsAt?: string | null,
  resetAfterSeconds?: number | null,
  fallback = 'reset time unknown',
  nowMs = Date.now(),
): string {
  const seconds = usageResetSeconds(resetsAt, resetAfterSeconds, nowMs);
  if (seconds !== null) {
    return `resets in ${formatUsageDuration(seconds)}`;
  }
  return fallback;
}

export function formatUsageAge(
  refreshedAt?: string | null,
  nowMs = Date.now(),
): string | null {
  if (!refreshedAt) {
    return null;
  }
  const timestamp = Date.parse(refreshedAt);
  if (!Number.isFinite(timestamp)) {
    return null;
  }
  const ageSeconds = Math.max(0, Math.floor((nowMs - timestamp) / 1000));
  return `updated ${formatUsageDuration(Math.abs(ageSeconds))} ago`;
}
