// Pure, headless-testable helpers for the custom-agent environment-variable
// editor. Kept out of the .tsx component so `node --test` (which cannot load
// .tsx) can exercise the merge/serialize logic directly.
//
// The KV rows are the single source of truth for env. The native-provider API
// key field is a derived view over the well-known key's row (read + write); it
// is never serialized separately, so it cannot override or resurrect a row.

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

/** Whether a string is a valid POSIX-style env var name. */
export function isValidEnvKey(key: string): boolean {
  return /^[A-Za-z_][A-Za-z0-9_]*$/.test(key);
}

/** Whether any row has a non-empty but invalid key (blocks save). */
export function envRowsHaveInvalidKey(env: EnvRow[]): boolean {
  return env.some((row) => {
    const key = row.key.trim();
    return key.length > 0 && !isValidEnvKey(key);
  });
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

/** The API-key shortcut's displayed value: the well-known key's row value. */
export function apiKeyValueFromRows(env: EnvRow[], providerType: string): string {
  const name = apiKeyEnvName(providerType);
  if (!name) {
    return '';
  }
  const row = env.find((entry) => entry.key.trim() === name);
  return row ? row.value : '';
}

/**
 * Write the API-key shortcut back into the rows: upsert the well-known key's row
 * with the given value, or remove it when the value is empty. Keeps the rows the
 * single source so the shortcut and the KV list can never diverge.
 */
export function setApiKeyInRows(env: EnvRow[], providerType: string, value: string): EnvRow[] {
  const name = apiKeyEnvName(providerType);
  if (!name) {
    return env;
  }
  const trimmed = value.trim();
  const withoutKey = env.filter((entry) => entry.key.trim() !== name);
  if (!trimmed) {
    return withoutKey;
  }
  const existing = env.find((entry) => entry.key.trim() === name);
  if (existing) {
    return env.map((entry) => (entry.key.trim() === name ? { ...entry, value: trimmed } : entry));
  }
  return [...withoutKey, { key: name, value: trimmed }];
}

/**
 * Build the full `provider_env` map from the KV editor rows. Empty (trimmed)
 * keys are dropped and the last row wins on duplicate keys. The editor is seeded
 * with the agent's full env, so the returned map is authoritative: sending it
 * preserves untouched keys and an empty map clears env.
 */
export function buildProviderEnvPayload(env: EnvRow[]): Record<string, string> {
  const map: Record<string, string> = {};
  for (const row of env) {
    const key = row.key.trim();
    if (key) {
      map[key] = row.value;
    }
  }
  return map;
}
