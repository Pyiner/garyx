const HEADER_NAME_PATTERN = /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/;

export interface GatewayHeaderEntry {
  name: string;
  value: string;
}

export function normalizeGatewayHeadersBlock(value: unknown): string {
  if (typeof value !== "string") {
    return "";
  }
  return value
    .replace(/\r\n?/g, "\n")
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
    .join("\n");
}

function headerSeparatorIndex(line: string): number {
  const colonIndex = line.indexOf(":");
  const equalsIndex = line.indexOf("=");
  if (colonIndex >= 0 && (equalsIndex < 0 || colonIndex < equalsIndex)) {
    return colonIndex;
  }
  return equalsIndex;
}

export function parseGatewayHeaderEntries(value: unknown): GatewayHeaderEntry[] {
  const block = normalizeGatewayHeadersBlock(value);
  if (!block) {
    return [];
  }

  const entries: GatewayHeaderEntry[] = [];
  for (const line of block.split("\n")) {
    if (!line || line.startsWith("#")) {
      continue;
    }
    const separatorIndex = headerSeparatorIndex(line);
    if (separatorIndex <= 0) {
      continue;
    }
    const name = line.slice(0, separatorIndex).trim();
    if (!name) {
      continue;
    }
    entries.push({
      name,
      value: line.slice(separatorIndex + 1).trim(),
    });
  }
  return entries;
}

export function formatGatewayHeaderEntries(entries: readonly GatewayHeaderEntry[]): string {
  return entries
    .map((entry) => ({
      name: entry.name.trim(),
      value: entry.value.trim(),
    }))
    .filter((entry) => entry.name.length > 0)
    .map((entry) => `${entry.name}: ${entry.value}`)
    .join("\n");
}

export function parseGatewayHeadersBlock(value: unknown): Record<string, string> {
  const headers: Record<string, string> = {};
  for (const { name, value: headerValue } of parseGatewayHeaderEntries(value)) {
    if (!HEADER_NAME_PATTERN.test(name)) {
      continue;
    }
    headers[name] = headerValue;
  }

  return headers;
}
