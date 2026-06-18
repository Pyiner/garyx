import type { TranscriptMessage } from "@shared/contracts";

type GeneratedImageToolMessage = Pick<
  TranscriptMessage,
  "content" | "metadata" | "toolName" | "toolUseId"
>;

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  return value as Record<string, unknown>;
}

function recordString(
  record: Record<string, unknown> | null | undefined,
  ...keys: string[]
): string {
  for (const key of keys) {
    const value = record?.[key];
    if (typeof value === "string" && value.trim()) {
      return value.trim();
    }
  }
  return "";
}

function codexItemTypeFromToolMessage(
  message: GeneratedImageToolMessage,
): string {
  const content = asRecord(message.content);
  const metadata = asRecord(message.metadata);
  return (
    recordString(metadata, "item_type", "itemType") ||
    recordString(content, "type") ||
    message.toolName?.trim() ||
    ""
  );
}

function isImageGenerationToolMessage(
  message: GeneratedImageToolMessage,
): boolean {
  return codexItemTypeFromToolMessage(message).toLowerCase() === "imagegeneration";
}

function base64FromGeneratedImageResult(result: string): string {
  const match = result.match(/^data:[^,]*,(.*)$/is);
  return (match?.[1] || result).trim();
}

function mediaTypeFromImagePath(path: string): string {
  const ext = path.split(/[./]/).at(-1)?.toLowerCase() || "";
  switch (ext) {
    case "png":
      return "image/png";
    case "gif":
      return "image/gif";
    case "webp":
      return "image/webp";
    case "jpg":
    case "jpeg":
      return "image/jpeg";
    default:
      return "image/png";
  }
}

function explicitGeneratedImageMediaType(
  content: Record<string, unknown> | null,
): string {
  return recordString(
    content,
    "media_type",
    "mediaType",
    "mime_type",
    "mimeType",
    "contentType",
  );
}

function mediaTypeFromGeneratedImageResult(
  result: string,
  content: Record<string, unknown> | null,
): string {
  const explicit = explicitGeneratedImageMediaType(content);
  if (explicit) {
    return explicit;
  }
  const match = result.match(/^data:([^;,]+)(?:;[^,]*)?,/i);
  return match?.[1]?.trim() || "image/png";
}

function generatedImagePath(content: Record<string, unknown> | null): string {
  return recordString(content, "savedPath", "saved_path", "path", "filePath");
}

function basename(path: string): string {
  return path.split("/").filter(Boolean).at(-1) || "";
}

function generatedImageName(
  message: GeneratedImageToolMessage,
  path = "",
): string {
  const content = asRecord(message.content);
  const id = recordString(content, "id") || message.toolUseId?.trim() || "";
  return basename(path) || (id ? `${id}.png` : "generated-image.png");
}

function fileUrlFromAbsolutePath(path: string): string {
  if (!path.startsWith("/")) {
    return "";
  }
  const encodedPath = path
    .split("/")
    .map((part, index) => (index === 0 ? "" : encodeURIComponent(part)))
    .join("/");
  return `file://${encodedPath}`;
}

function hydratedSourceFromContent(
  content: Record<string, unknown> | null,
): Record<string, unknown> | null {
  const source = asRecord(content?.source);
  const data = recordString(source, "data");
  if (!data) {
    return null;
  }
  return {
    type: recordString(source, "type") || "base64",
    media_type:
      recordString(source, "media_type", "mediaType") ||
      explicitGeneratedImageMediaType(content) ||
      "image/png",
    data,
  };
}

export function extractImageGenerationImageContent(
  message: GeneratedImageToolMessage,
): unknown[] | null {
  if (!isImageGenerationToolMessage(message)) {
    return null;
  }
  const content = asRecord(message.content);
  const path = generatedImagePath(content);
  if (path) {
    const url = fileUrlFromAbsolutePath(path);
    return [
      {
        type: "image",
        name: generatedImageName(message, path),
        path,
        media_type:
          explicitGeneratedImageMediaType(content) || mediaTypeFromImagePath(path),
        ...(url ? { url } : {}),
      },
    ];
  }

  const source = hydratedSourceFromContent(content);
  if (source) {
    return [
      {
        type: "image",
        name: generatedImageName(message),
        media_type:
          recordString(source, "media_type", "mediaType") || "image/png",
        source,
      },
    ];
  }

  const result = recordString(content, "result");
  if (!result) {
    return null;
  }
  const data = base64FromGeneratedImageResult(result);
  if (!data) {
    return null;
  }
  return [
    {
      type: "image",
      name: generatedImageName(message),
      source: {
        type: "base64",
        media_type: mediaTypeFromGeneratedImageResult(result, content),
        data,
      },
    },
  ];
}
