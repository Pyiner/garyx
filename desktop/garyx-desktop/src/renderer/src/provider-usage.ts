export const MODEL_PROVIDER_USAGE_PROVIDER_IDS = {
  claude_code: 'claude_code',
  codex_app_server: 'codex',
  antigravity: 'antigravity',
} as const;

export type ModelProviderUsageKey = keyof typeof MODEL_PROVIDER_USAGE_PROVIDER_IDS;

export function usageProviderIdForModelProviderKey(
  key: string,
): string | undefined {
  return MODEL_PROVIDER_USAGE_PROVIDER_IDS[key as ModelProviderUsageKey];
}
