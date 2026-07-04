// Pure transcript-content parsing helpers, extracted from
// message-rich-content.tsx (endgame batch 2a-2) so React-free modules
// (gateway-mirror) can import them without loading a JSX module.
// Verbatim relocation; the .tsx re-exports the shared surface.

export type TranscriptSegment =
  | {
      kind: "text";
      key: string;
      text: string;
    }
  | {
      kind: "image";
      key: string;
      src: string;
      alt: string;
    }
  | {
      kind: "file";
      key: string;
      path?: string;
      label: string;
      mediaType?: string;
    }
  | {
      kind: "json";
      key: string;
      json: string;
    };

export function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function imageSourceFromUnknown(value: unknown): string | null {
  const record = asRecord(value);
  if (!record) {
    return null;
  }

  const directUrl = typeof record.url === "string" ? record.url.trim() : "";
  if (directUrl) {
    return directUrl;
  }

  const source = asRecord(record.source);
  if (!source) {
    return null;
  }

  const sourceData = typeof source.data === "string" ? source.data.trim() : "";
  if (!sourceData) {
    return null;
  }

  const mediaType =
    (typeof source.media_type === "string" && source.media_type.trim()) ||
    (typeof source.mediaType === "string" && source.mediaType.trim()) ||
    "image/jpeg";

  return buildMessageImageDataUrl(mediaType, sourceData);
}

function fileSegmentFromUnknown(value: unknown): TranscriptSegment | null {
  const record = asRecord(value);
  const path =
    record && typeof record.path === "string" ? record.path.trim() : "";
  const rawLabel =
    record && typeof record.name === "string" ? record.name.trim() : "";
  const label =
    rawLabel || path.split("/").filter(Boolean).at(-1) || "Attached file";
  return {
    kind: "file",
    key: `${path || label}:file`,
    path: path || undefined,
    label,
    mediaType:
      (record && typeof record.media_type === "string"
        ? record.media_type.trim()
        : "") ||
      (record && typeof record.mediaType === "string"
        ? record.mediaType.trim()
        : "") ||
      undefined,
  };
}

function imageReferenceSegmentFromUnknown(
  value: unknown,
  key: string,
): TranscriptSegment | null {
  const record = asRecord(value);
  const path =
    record && typeof record.path === "string" ? record.path.trim() : "";
  const rawLabel =
    record && typeof record.name === "string" ? record.name.trim() : "";
  const label =
    rawLabel || path.split("/").filter(Boolean).at(-1) || "Attached image";
  if (!path && !rawLabel) {
    return null;
  }
  return {
    kind: "file",
    key,
    path: path || undefined,
    label,
    mediaType:
      (record && typeof record.media_type === "string"
        ? record.media_type.trim()
        : "") ||
      (record && typeof record.mediaType === "string"
        ? record.mediaType.trim()
        : "") ||
      undefined,
  };
}

export function collectTranscriptSegments(
  content: unknown,
  altPrefix: string,
  path = "root",
): TranscriptSegment[] {
  if (typeof content === "string") {
    const trimmed = content.trim();
    return trimmed
      ? [
          {
            kind: "text",
            key: `${path}:text`,
            text: content,
          },
        ]
      : [];
  }

  if (Array.isArray(content)) {
    return content.flatMap((entry, index) =>
      collectTranscriptSegments(entry, altPrefix, `${path}:${index}`),
    );
  }

  const record = asRecord(content);
  if (!record) {
    return [];
  }

  const type =
    typeof record.type === "string" ? record.type.trim().toLowerCase() : "";
  if (type === "text") {
    const text = typeof record.text === "string" ? record.text : "";
    return text.trim()
      ? [
          {
            kind: "text",
            key: `${path}:text`,
            text,
          },
        ]
      : [];
  }

  if (type === "image") {
    const src = imageSourceFromUnknown(record);
    if (src) {
      return [
        {
          kind: "image",
          key: `${path}:image`,
          src,
          alt: `${altPrefix} image`,
        },
      ];
    }
    const fallback = imageReferenceSegmentFromUnknown(record, `${path}:image-ref`);
    return fallback ? [fallback] : [];
  }

  if (type === "file") {
    const segment = fileSegmentFromUnknown(record);
    return segment ? [segment] : [];
  }

  const directImageSrc = imageSourceFromUnknown(record);
  if (directImageSrc) {
    return [
      {
        kind: "image",
        key: `${path}:image`,
        src: directImageSrc,
        alt: `${altPrefix} image`,
      },
    ];
  }

  return [];
}

export function fallbackJsonSegment(content: unknown): TranscriptSegment[] {
  if (content === null || content === undefined) {
    return [];
  }
  if (typeof content === "string") {
    return [];
  }
  try {
    return [
      {
        kind: "json",
        key: "fallback:json",
        json: JSON.stringify(content, null, 2),
      },
    ];
  } catch {
    return [];
  }
}

function countContentBlocksByType(content: unknown, type: string): number {
  if (Array.isArray(content)) {
    return content.reduce<number>(
      (total, entry) => total + countContentBlocksByType(entry, type),
      0,
    );
  }
  const record = asRecord(content);
  if (!record) {
    return 0;
  }
  const directType =
    typeof record.type === "string" ? record.type.trim().toLowerCase() : "";
  const nestedCount = Object.values(record).reduce<number>(
    (total, entry) => total + countContentBlocksByType(entry, type),
    0,
  );
  return nestedCount + (directType === type ? 1 : 0);
}

export function countTranscriptImages(content: unknown): number {
  return countContentBlocksByType(content, "image");
}

export function countTranscriptFiles(content: unknown): number {
  return countContentBlocksByType(content, "file");
}

export function extractTranscriptText(content: unknown): string {
  return collectTranscriptSegments(content, "message")
    .filter(
      (segment): segment is Extract<TranscriptSegment, { kind: "text" }> =>
        segment.kind === "text",
    )
    .map((segment) => segment.text)
    .join("\n\n")
    .trim();
}


export function buildMessageImageDataUrl(
  mediaType: string,
  data: string,
): string {
  const normalizedType = mediaType?.trim() || "image/jpeg";
  const normalizedData = data?.trim() || "";
  return `data:${normalizedType};base64,${normalizedData}`;
}
