// Pure, headless-testable helpers for the custom-agent environment-variable
// editor. Kept out of the .tsx component so `node --test` (which cannot load
// .tsx) can exercise the merge/serialize logic directly.

export type EnvRow = { key: string; value: string };

/** Providers whose credentials are resolved in-process from an env var. */
export function isNativeModelProvider(value: string): boolean {
  return (
    value === 'gpt' ||
    value === 'anthropic' ||
    value === 'google' ||
    value === 'claude_llm' ||
    value === 'gemini_llm'
  );
}

/** The well-known env var that carries a native provider's API key, if any. */
export function apiKeyEnvName(value: string): string | null {
  if (value === 'gpt') {
    return 'OPENAI_API_KEY';
  }
  if (value === 'anthropic' || value === 'claude_llm') {
    return 'ANTHROPIC_API_KEY';
  }
  if (value === 'google' || value === 'gemini_llm') {
    return 'GEMINI_API_KEY';
  }
  return null;
}

/**
 * Seed the KV editor from an agent's env map, sorted by key for stable display
 * order (the map itself is unordered).
 */
export function envRowsFromEnvMap(env: Record<string, string> | null | undefined): EnvRow[] {
  const map = env || {};
  return Object.keys(map)
    .sort()
    .map((key) => ({ key, value: map[key] ?? '' }));
}

/**
 * Build the full `provider_env` map from the KV editor rows. The rows are the
 * single source of truth; the native-provider API key field is a convenience
 * writer for its well-known key. Empty keys are dropped and the last row wins on
 * duplicate keys. The editor is seeded with the agent's full env, so the
 * returned map is authoritative: sending it preserves untouched keys and an
 * empty map clears env.
 */
export function buildProviderEnvPayload(
  env: EnvRow[],
  providerType: string,
  apiKey: string,
): Record<string, string> {
  const map: Record<string, string> = {};
  for (const row of env) {
    const key = row.key.trim();
    if (key) {
      map[key] = row.value;
    }
  }
  const apiKeyEnv = apiKeyEnvName(providerType);
  if (isNativeModelProvider(providerType) && apiKeyEnv && apiKey.trim()) {
    map[apiKeyEnv] = apiKey.trim();
  }
  return map;
}
