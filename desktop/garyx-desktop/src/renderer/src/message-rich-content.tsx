import { memo, useMemo, type ReactNode } from "react";
import { FileText } from "lucide-react";

import type {
  MessageFileAttachment,
  MessageImageAttachment,
} from "@shared/contracts";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import {
  RichMessageText,
  type LocalFileLinkHandler,
} from "./message-rich-text";
import { useI18n, type Translate } from "./i18n";
import {
  parseTaskNotificationText,
  type ParsedTaskNotification,
} from "./task-notification";
import {
  parseRestartNoticeText,
  type ParsedRestartNotice,
} from "./restart-notice";


import {
  collectTranscriptSegments,
  countTranscriptFiles,
  countTranscriptImages,
  extractTranscriptText,
  fallbackJsonSegment,
  type TranscriptSegment,
  buildMessageImageDataUrl,
} from "./message-rich-content-core";
export {
  buildMessageImageDataUrl,
  countTranscriptFiles,
  countTranscriptImages,
  extractTranscriptText,
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

function taskNotificationStatusLabel(status: string, t: Translate): string {
  if (status === "in_review") {
    return t("In review");
  }
  return status
    .split(/[_-]+/)
    .filter(Boolean)
    .map((part) => part.slice(0, 1).toUpperCase() + part.slice(1))
    .join(" ");
}

function TaskNotificationCard({
  notification,
}: {
  notification: ParsedTaskNotification;
}) {
  const { t } = useI18n();
  return (
    <section
      className="task-notification-card"
      aria-label={t("Task ready for review")}
    >
      <div className="task-notification-header">
        <div className="task-notification-heading">
          <div className="task-notification-kicker">
            <span className="task-notification-task-id">
              {notification.taskId || t("Task")}
            </span>
            <span className="task-notification-status">
              {taskNotificationStatusLabel(notification.status, t)}
            </span>
          </div>
          <h3 className="task-notification-title">{notification.title}</h3>
        </div>
      </div>

      <div className="task-notification-body">
        <RichMessageText
          text={notification.finalMessage}
          tone="assistant"
        />
      </div>
    </section>
  );
}

function RestartNoticeCard({
  notice,
}: {
  notice: ParsedRestartNotice;
}) {
  const { t } = useI18n();
  return (
    <section className="restart-notice-card" aria-label={t("Garyx restarted")}>
      <div className="restart-notice-header">
        <span className="restart-notice-kicker">{t("Garyx restarted")}</span>
      </div>
      <div className="restart-notice-body">
        <RichMessageText text={notice.message} tone="assistant" />
      </div>
    </section>
  );
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
  t: Translate,
): string {
  const mediaType = segment.mediaType?.trim();
  if (mediaType) {
    return mediaType;
  }
  return segment.path ? t("Local attachment") : t("Attached file");
}

function MessageFileAttachmentCard({
  segment,
  onLocalFileLinkClick,
}: {
  segment: Extract<TranscriptSegment, { kind: "file" }>;
  onLocalFileLinkClick?: LocalFileLinkHandler;
}) {
  const { t } = useI18n();
  const previewPath = segment.path;
  const canPreview = Boolean(previewPath && onLocalFileLinkClick);
  const label = segment.label === "Attached file" || segment.label === "Attached image"
    ? t(segment.label)
    : segment.label;
  const body = (
    <>
      <span aria-hidden="true" className="message-file-card-icon">
        <FileText size={18} strokeWidth={1.8} />
      </span>
      <span className="message-file-card-copy">
        <span className="message-file-card-name" title={label}>
          {label}
        </span>
        <span className="message-file-card-meta">
          {formatFileSegmentMeta(segment, t)}
        </span>
      </span>
    </>
  );

  if (!canPreview || !previewPath || !onLocalFileLinkClick) {
    return (
      <div
        className="message-file-card"
        title={segment.path || label}
      >
        {body}
      </div>
    );
  }

  return (
    <button
      aria-label={t('Preview attached file {name}', { name: label })}
      className="message-file-card message-file-card-clickable"
      onClick={() => onLocalFileLinkClick(previewPath)}
      title={previewPath}
      type="button"
    >
      {body}
    </button>
  );
}

function MessageImageAttachmentFrame({
  compact,
  segment,
}: {
  compact: boolean;
  segment: Extract<TranscriptSegment, { kind: "image" }>;
}) {
  const { t } = useI18n();
  const frameClassName = `message-image-frame ${
    compact ? "message-image-frame-compact" : ""
  }`;

  return (
    <Dialog>
      <DialogTrigger asChild>
        <button
          aria-label={t("Open image preview")}
          className={frameClassName}
          title={t("Open image preview")}
          type="button"
        >
          <img
            alt={segment.alt}
            className="message-image"
            loading="lazy"
            src={segment.src}
          />
        </button>
      </DialogTrigger>
      <DialogContent
        className="message-image-preview-dialog"
        size="viewer"
      >
        <DialogTitle className="sr-only">{t("Image preview")}</DialogTitle>
        <DialogDescription className="sr-only">
          {t("Full-size image preview")}
        </DialogDescription>
        <div className="message-image-preview-stage">
          <img
            alt={segment.alt}
            className="message-image-preview"
            src={segment.src}
          />
        </div>
      </DialogContent>
    </Dialog>
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





export const RichMessageContent = memo(function RichMessageContent({
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
  const segments = useMemo(
    () => collectTranscriptSegments(content, altPrefix),
    [altPrefix, content],
  );
  const tone = useMemo(() => resolveMessageTone(altPrefix), [altPrefix]);
  const renderableSegments = useMemo<TranscriptSegment[]>(
    () =>
      segments.length
        ? segments
        : text.trim()
          ? [
              {
                kind: "text",
                key: "fallback:text",
                text,
              },
            ]
          : fallbackJsonSegment(content),
    [content, segments, text],
  );

  if (!renderableSegments.length) {
    return null;
  }

  const renderSegment = (segment: TranscriptSegment): ReactNode => {
    if (segment.kind === "text") {
      const taskNotification = parseTaskNotificationText(segment.text);
      if (taskNotification) {
        return (
          <TaskNotificationCard
            key={segment.key}
            notification={taskNotification}
          />
        );
      }
      const restartNotice = parseRestartNoticeText(segment.text);
      if (restartNotice) {
        return <RestartNoticeCard key={segment.key} notice={restartNotice} />;
      }
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
        <MessageImageAttachmentFrame
          compact={layout === "media_above"}
          key={segment.key}
          segment={segment}
        />
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
});
