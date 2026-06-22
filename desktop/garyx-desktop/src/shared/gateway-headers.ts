const HEADER_NAME_PATTERN = /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/;

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

export function parseGatewayHeadersBlock(value: unknown): Record<string, string> {
  const headers: Record<string, string> = {};
  const block = normalizeGatewayHeadersBlock(value);
  if (!block) {
    return headers;
  }

  for (const line of block.split("\n")) {
    if (!line || line.startsWith("#")) {
      continue;
    }
    const colonIndex = line.indexOf(":");
    const equalsIndex = line.indexOf("=");
    const separatorIndex =
      colonIndex >= 0 && (equalsIndex < 0 || colonIndex < equalsIndex)
        ? colonIndex
        : equalsIndex;
    if (separatorIndex <= 0) {
      continue;
    }
    const name = line.slice(0, separatorIndex).trim();
    const rawValue = line.slice(separatorIndex + 1).trim();
    if (!HEADER_NAME_PATTERN.test(name)) {
      continue;
    }
    headers[name] = rawValue;
  }

  return headers;
}
