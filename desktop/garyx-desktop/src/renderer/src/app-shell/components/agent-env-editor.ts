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

/**
 * Render env rows as dotenv-style text, one `KEY=VALUE` per line.
 *
 * Values are emitted verbatim — never quoted — so what the user sees is
 * byte-for-byte what the provider subprocess receives. Two exceptions must
 * ride the quoted-escape carrier so `parseEnvText` round-trips them exactly:
 * a value containing a newline (cannot survive a line-oriented format), and
 * a value that itself starts and ends with a double quote (would otherwise
 * be mistaken for the carrier and stripped on parse).
 */
export function formatEnvText(env: EnvRow[]): string {
  return env
    .filter((row) => row.key.trim().length > 0)
    .map((row) => {
      const key = row.key.trim();
      const looksQuoted =
        row.value.length >= 2 && row.value.startsWith('"') && row.value.endsWith('"');
      // Leading/trailing whitespace would be eaten by parse's line trim, so
      // such values must also ride the quoted carrier to round-trip.
      const hasEdgeWhitespace = row.value !== row.value.trim();
      if (
        row.value.includes('\n') ||
        row.value.includes('\r') ||
        looksQuoted ||
        (row.value.length > 0 && hasEdgeWhitespace)
      ) {
        const escaped = row.value
          .replace(/\\/g, '\\\\')
          .replace(/"/g, '\\"')
          .replace(/\r/g, '\\r')
          .replace(/\n/g, '\\n');
        return `${key}="${escaped}"`;
      }
      return `${key}=${row.value}`;
    })
    .join('\n');
}

/**
 * Parse dotenv-style text into env rows. Inverse of {@link formatEnvText}.
 *
 * - Blank lines and `#` comment lines are skipped.
 * - Lines are whitespace-trimmed (stray edge whitespace is invisible and a
 *   footgun); a value that genuinely needs edge whitespace survives via the
 *   quoted carrier, which `formatEnvText` emits for such values.
 * - Each line splits on the first `=`; the value keeps everything after it
 *   verbatim (numbers stay unquoted, inner/partial quotes are preserved).
 * - Only a value fully wrapped in double quotes is treated as the escape
 *   carrier: the quotes are stripped and `\n`/`\r`/`\"`/`\\` unescaped. This
 *   matches dotenv intuition for users who habitually quote values, and
 *   `formatEnvText` re-quotes such values so the round trip stays lossless.
 * - A line with no `=` becomes a row with an empty value so the invalid-key
 *   save gate can surface it instead of silently dropping user input.
 */
export function parseEnvText(text: string): EnvRow[] {
  const rows: EnvRow[] = [];
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith('#')) {
      continue;
    }
    const eq = line.indexOf('=');
    if (eq < 0) {
      rows.push({ key: line, value: '' });
      continue;
    }
    const key = line.slice(0, eq).trim();
    let value = line.slice(eq + 1);
    if (value.length >= 2 && value.startsWith('"') && value.endsWith('"')) {
      const inner = value.slice(1, -1);
      // Only treat it as our escape carrier when the quotes wrap the whole
      // value and the inner text does not contain a bare unescaped quote.
      let unescaped = '';
      let valid = true;
      for (let i = 0; i < inner.length; i += 1) {
        const ch = inner[i];
        if (ch === '"') {
          valid = false;
          break;
        }
        if (ch === '\\' && i + 1 < inner.length) {
          const next = inner[i + 1];
          if (next === 'n') {
            unescaped += '\n';
            i += 1;
            continue;
          }
          if (next === 'r') {
            unescaped += '\r';
            i += 1;
            continue;
          }
          if (next === '"') {
            unescaped += '"';
            i += 1;
            continue;
          }
          if (next === '\\') {
            unescaped += '\\';
            i += 1;
            continue;
          }
        }
        unescaped += ch;
      }
      if (valid) {
        value = unescaped;
      }
    }
    rows.push({ key, value });
  }
  return rows;
}
