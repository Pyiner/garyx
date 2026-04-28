import type { ReactNode } from "react";
import { FileText } from "lucide-react";

import type {
  MessageFileAttachment,
  MessageImageAttachment,
} from "@shared/contracts";

import {
  RichMessageText,
  type LocalFileLinkHandler,
} from "./message-rich-text";

type TranscriptSegment =
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

type RichMessageLayout = "default" | "media_above";

export type RichMessageBubblePart = {
  kind: "text" | "image" | "file";
  key: string;
  content: unknown;
  text: string;
};

function resolveMessageTone(role: string): "default" | "assistant" {
  return role === "assistant" ? "assistant" : "default";
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

export function buildMessageImageDataUrl(
  mediaType: string,
  data: string,
): string {
  const normalizedType = mediaType?.trim() || "image/jpeg";
  const normalizedData = data?.trim() || "";
  return `data:${normalizedType};base64,${normalizedData}`;
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

function collectTranscriptSegments(
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

function fallbackJsonSegment(content: unknown): TranscriptSegment[] {
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

function contentFromSegments(segments: TranscriptSegment[]): unknown {
  if (segments.length === 1) {
    const [segment] = segments;
    if (!segment) {
      return "";
    }
    if (segment.kind === "text") {
      return segment.text;
    }
    if (segment.kind === "image") {
      return {
        type: "image",
        url: segment.src,
      };
    }
    if (segment.kind === "file") {
      return {
        type: "file",
        name: segment.label,
        path: segment.path,
        media_type: segment.mediaType,
      };
    }
  }

  return segments.map((segment) => {
    if (segment.kind === "text") {
      return {
        type: "text",
        text: segment.text,
      };
    }
    if (segment.kind === "image") {
      return {
        type: "image",
        url: segment.src,
      };
    }
    if (segment.kind === "file") {
      return {
        type: "file",
        name: segment.label,
        path: segment.path,
        media_type: segment.mediaType,
      };
    }
    return JSON.parse(segment.json) as unknown;
  });
}

function textFromSegments(segments: TranscriptSegment[]): string {
  return segments
    .filter(
      (segment): segment is Extract<TranscriptSegment, { kind: "text" }> =>
        segment.kind === "text",
    )
    .map((segment) => segment.text)
    .join("\n\n")
    .trim();
}

function buildBubblePart(
  key: string,
  segments: TranscriptSegment[],
  kind: RichMessageBubblePart["kind"],
  fallbackText = "",
): RichMessageBubblePart {
  return {
    kind,
    key,
    content: contentFromSegments(segments),
    text: textFromSegments(segments) || fallbackText,
  };
}

export function splitRichMessageContentIntoBubbleParts({
  text,
  content,
  altPrefix = "message",
}: {
  text: string;
  content?: unknown;
  altPrefix?: string;
}): RichMessageBubblePart[] {
  const segments = collectTranscriptSegments(content, altPrefix);
  const hasStandaloneSegment = segments.some(
    (segment) => segment.kind === "image" || segment.kind === "file",
  );
  if (!hasStandaloneSegment) {
    return [
      {
        kind: "text",
        key: "text",
        content,
        text,
      },
    ];
  }

  const parts: RichMessageBubblePart[] = [];
  let currentContentSegments: TranscriptSegment[] = [];
  let partIndex = 0;

  const flushContent = () => {
    if (!currentContentSegments.length) {
      return;
    }
    parts.push(
      buildBubblePart(
        `text:${partIndex++}`,
        currentContentSegments,
        "text",
      ),
    );
    currentContentSegments = [];
  };

  for (const segment of segments) {
    if (segment.kind === "image") {
      flushContent();
      parts.push(
        buildBubblePart(
          `image:${partIndex++}`,
          [segment],
          "image",
          segment.alt,
        ),
      );
      continue;
    }
    if (segment.kind === "file") {
      flushContent();
      parts.push(
        buildBubblePart(`file:${partIndex++}`, [segment], "file", segment.label),
      );
      continue;
    }
    currentContentSegments.push(segment);
  }
  flushContent();

  return parts.length
    ? parts
    : [
        {
          kind: "text",
          key: "text",
          content,
          text,
        },
      ];
}

function formatFileSegmentMeta(
  segment: Extract<TranscriptSegment, { kind: "file" }>,
): string {
  const mediaType = segment.mediaType?.trim();
  if (mediaType) {
    return mediaType;
  }
  return segment.path ? "Local attachment" : "Attached file";
}

function MessageFileAttachmentCard({
  segment,
  onLocalFileLinkClick,
}: {
  segment: Extract<TranscriptSegment, { kind: "file" }>;
  onLocalFileLinkClick?: LocalFileLinkHandler;
}) {
  const previewPath = segment.path;
  const canPreview = Boolean(previewPath && onLocalFileLinkClick);
  const body = (
    <>
      <span aria-hidden="true" className="message-file-card-icon">
        <FileText size={18} strokeWidth={1.8} />
      </span>
      <span className="message-file-card-copy">
        <span className="message-file-card-name" title={segment.label}>
          {segment.label}
        </span>
        <span className="message-file-card-meta">
          {formatFileSegmentMeta(segment)}
        </span>
      </span>
    </>
  );

  if (!canPreview || !previewPath || !onLocalFileLinkClick) {
    return (
      <div
        className="message-file-card"
        title={segment.path || segment.label}
      >
        {body}
      </div>
    );
  }

  return (
    <button
      aria-label={`Preview attached file ${segment.label}`}
      className="message-file-card message-file-card-clickable"
      onClick={() => onLocalFileLinkClick(previewPath)}
      title={previewPath}
      type="button"
    >
      {body}
    </button>
  );
}

export function buildOptimisticTranscriptContent(
  text: string,
  images: MessageImageAttachment[],
  files: MessageFileAttachment[] = [],
): unknown {
  if (!images.length && !files.length) {
    return text;
  }

  const blocks: Array<Record<string, unknown>> = [];
  if (text.trim()) {
    blocks.push({
      type: "text",
      text,
    });
  }
  for (const image of images) {
    const block: Record<string, unknown> = {
      type: "image",
      name: image.name,
      path: image.path,
      media_type: image.mediaType,
    };
    if (image.data?.trim()) {
      block.source = {
        type: "base64",
        media_type: image.mediaType,
        data: image.data,
      };
    }
    blocks.push(block);
  }
  for (const file of files) {
    blocks.push({
      type: "file",
      name: file.name,
      path: file.path,
      media_type: file.mediaType,
    });
  }
  return blocks;
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

export function RichMessageContent({
  text,
  content,
  altPrefix = "message",
  layout = "default",
  onLocalFileLinkClick,
}: {
  text: string;
  content?: unknown;
  altPrefix?: string;
  layout?: RichMessageLayout;
  onLocalFileLinkClick?: LocalFileLinkHandler;
}) {
  const segments = collectTranscriptSegments(content, altPrefix);
  const tone = resolveMessageTone(altPrefix);
  const renderableSegments = segments.length
    ? segments
    : text.trim()
      ? [
          {
            kind: "text" as const,
            key: "fallback:text",
            text,
          },
        ]
      : fallbackJsonSegment(content);

  if (!renderableSegments.length) {
    return null;
  }

  const renderSegment = (segment: TranscriptSegment): ReactNode => {
    if (segment.kind === "text") {
      return (
        <RichMessageText
          key={segment.key}
          onLocalFileLinkClick={onLocalFileLinkClick}
          text={segment.text}
          tone={tone}
        />
      );
    }

    if (segment.kind === "image") {
      return (
        <div
          key={segment.key}
          className={`message-image-frame ${layout === "media_above" ? "message-image-frame-compact" : ""}`}
        >
          <img
            alt={segment.alt}
            className="message-image"
            loading="lazy"
            src={segment.src}
          />
        </div>
      );
    }

    if (segment.kind === "file") {
      return (
        <MessageFileAttachmentCard
          key={segment.key}
          onLocalFileLinkClick={onLocalFileLinkClick}
          segment={segment}
        />
      );
    }

    return (
      <pre key={segment.key} className="message-rich-json">
        <code>{segment.json}</code>
      </pre>
    );
  };

  if (layout === "media_above") {
    const imageSegments = renderableSegments.filter(
      (segment): segment is Extract<TranscriptSegment, { kind: "image" }> =>
        segment.kind === "image",
    );
    const bodySegments = renderableSegments.filter(
      (segment) => segment.kind !== "image",
    );

    return (
      <div className="message-rich-content message-rich-content-media-above">
        {imageSegments.length ? (
          <div className="message-media-strip">
            {imageSegments.map(renderSegment)}
          </div>
        ) : null}
        {bodySegments.length ? (
          <div className="message-rich-body">
            {bodySegments.map(renderSegment)}
          </div>
        ) : null}
      </div>
    );
  }

  return (
    <div className="message-rich-content">
      {renderableSegments.map(renderSegment)}
    </div>
  );
}
