import {
  memo,
  useCallback,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { Download, FileText } from "lucide-react";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import {
  Attachment,
  AttachmentContent,
  AttachmentDescription,
  AttachmentMedia,
  AttachmentTitle,
  AttachmentTrigger,
} from "@/components/ui/attachment";
import {
  RichMessageText,
  type LocalFileLinkHandler,
  type LocalMessageImageRenderer,
} from "./message-rich-text";
import { useI18n, type Translate } from "./i18n";
import { useToastActions } from "./toast-provider";
import type { RenderMessagePresentation } from "@shared/contracts";
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

export type MessageImagePreviewLoader = (
  path: string,
) => Promise<{ src: string; alt?: string } | null>;

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
    if (segment.kind === "image_reference") {
      return {
        type: "image",
        name: segment.label,
        path: segment.path,
        media_type: segment.mediaType,
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
    if (segment.kind === "image_reference") {
      return {
        type: "image",
        name: segment.label,
        path: segment.path,
        media_type: segment.mediaType,
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
    (segment) =>
      segment.kind === "image" ||
      segment.kind === "image_reference" ||
      segment.kind === "file",
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
    if (segment.kind === "image" || segment.kind === "image_reference") {
      flushContent();
      parts.push(
        buildBubblePart(
          `image:${partIndex++}`,
          [segment],
          "image",
          segment.kind === "image" ? segment.alt : segment.label,
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
  return (
    <Attachment
      className="message-attachment-card"
      title={segment.path || label}
    >
      <AttachmentMedia>
        <FileText aria-hidden size={18} strokeWidth={1.8} />
      </AttachmentMedia>
      <AttachmentContent>
        <AttachmentTitle title={label}>{label}</AttachmentTitle>
        <AttachmentDescription>
          {formatFileSegmentMeta(segment, t)}
        </AttachmentDescription>
      </AttachmentContent>
      {canPreview && previewPath && onLocalFileLinkClick ? (
        <AttachmentTrigger
          aria-label={t('Preview attached file {name}', { name: label })}
          onClick={() => onLocalFileLinkClick(previewPath)}
          title={previewPath}
        />
      ) : null}
    </Attachment>
  );
}

export function ImageZoomDialog({
  alt,
  suggestedName,
  src,
  trigger,
}: {
  alt: string;
  suggestedName?: string;
  src: string;
  trigger: ReactNode;
}) {
  const { t } = useI18n();
  const { pushToast } = useToastActions();
  const [saving, setSaving] = useState(false);
  const handleSaveImage = useCallback(async () => {
    if (saving) {
      return;
    }
    setSaving(true);
    try {
      const result = await window.garyxDesktop.saveImage({
        dataUrl: src,
        suggestedName,
      });
      if (!result.canceled) {
        pushToast(t("Image saved."), "success");
      }
    } catch {
      pushToast(t("Could not save image."), "error");
    } finally {
      setSaving(false);
    }
  }, [pushToast, saving, src, suggestedName, t]);

  return (
    <Dialog>
      <DialogTrigger asChild>{trigger}</DialogTrigger>
      <DialogContent
        className="message-image-preview-dialog"
        size="viewer"
      >
        <DialogTitle className="sr-only">{t("Image preview")}</DialogTitle>
        <DialogDescription className="sr-only">
          {t("Full-size image preview")}
        </DialogDescription>
        <div className="message-image-preview-stage">
          <img alt={alt} className="message-image-preview" src={src} />
        </div>
        <button
          aria-busy={saving}
          aria-label={t("Download image")}
          className="message-image-preview-download"
          disabled={saving}
          onClick={() => {
            void handleSaveImage();
          }}
          title={t("Download image")}
          type="button"
        >
          <Download aria-hidden size={15} strokeWidth={1.9} />
          <span>{saving ? t("Saving…") : t("Download image")}</span>
        </button>
      </DialogContent>
    </Dialog>
  );
}

export function MessageImageAttachmentFrame({
  compact,
  segment,
  suggestedName,
}: {
  compact: boolean;
  segment: Extract<TranscriptSegment, { kind: "image" }>;
  suggestedName?: string;
}) {
  const { t } = useI18n();
  const frameClassName = `message-image-frame ${
    compact ? "message-image-frame-compact" : ""
  }`;

  return (
    <ImageZoomDialog
      alt={segment.alt}
      suggestedName={suggestedName}
      src={segment.src}
      trigger={
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
      }
    />
  );
}

export function MessagePathImageAttachmentFrame({
  alt,
  compact,
  fallback = null,
  imageKey,
  loadImagePreview,
  path,
}: {
  alt: string;
  compact: boolean;
  fallback?: ReactNode;
  imageKey: string;
  loadImagePreview?: MessageImagePreviewLoader;
  path: string;
}) {
  const { t } = useI18n();
  const [preview, setPreview] = useState<{ src: string; alt?: string } | null>(null);
  const [loadFailed, setLoadFailed] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setPreview(null);
    setLoadFailed(false);
    if (!loadImagePreview) {
      setLoadFailed(true);
      return () => {
        cancelled = true;
      };
    }
    void loadImagePreview(path)
      .then((loaded) => {
        if (cancelled) {
          return;
        }
        if (!loaded?.src.trim()) {
          setLoadFailed(true);
          return;
        }
        setPreview(loaded);
      })
      .catch(() => {
        if (!cancelled) {
          setLoadFailed(true);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [loadImagePreview, path]);

  if (loadFailed) {
    return fallback;
  }
  if (!preview) {
    return (
      <span aria-label={t("Loading")} className="message-image-loading" role="status">
        <span aria-hidden="true" className="message-image-spinner" />
      </span>
    );
  }
  return (
    <MessageImageAttachmentFrame
      compact={compact}
      suggestedName={preview.alt || alt}
      segment={{
        kind: "image",
        key: imageKey,
        src: preview.src,
        alt: preview.alt || alt,
      }}
    />
  );
}

// Moved to message-rich-content-core.ts (endgame batch 3c-2) so the
// React-free dispatch orchestrator can build optimistic bubbles; the
// re-export keeps existing .tsx consumers working.
export { buildOptimisticTranscriptContent } from "./message-rich-content-core";





export const RichMessageContent = memo(function RichMessageContent({
  text,
  content,
  altPrefix = "message",
  layout = "default",
  loadImagePreview,
  onLocalFileLinkClick,
  presentation,
}: {
  text: string;
  content?: unknown;
  altPrefix?: string;
  layout?: RichMessageLayout;
  loadImagePreview?: MessageImagePreviewLoader;
  onLocalFileLinkClick?: LocalFileLinkHandler;
  presentation?: RenderMessagePresentation;
}) {
  const segments = useMemo(
    () => collectTranscriptSegments(content, altPrefix),
    [altPrefix, content],
  );
  const tone = useMemo(() => resolveMessageTone(altPrefix), [altPrefix]);
  const renderLocalMarkdownImage = useCallback<LocalMessageImageRenderer>(
    ({ alt, path }) => {
      const label = alt.trim() || path.split("/").pop() || path;
      const fallback = (
        <span className="message-local-image-fallback" title={path}>
          {label}
        </span>
      );
      if (!loadImagePreview) {
        return fallback;
      }
      return (
        <MessagePathImageAttachmentFrame
          alt={label}
          compact
          fallback={fallback}
          imageKey={`markdown-image:${path}`}
          loadImagePreview={loadImagePreview}
          path={path}
        />
      );
    },
    [loadImagePreview],
  );
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
  const taskNotification = useMemo(
    () =>
      presentation === "task_notification"
        ? parseTaskNotificationText(text)
        : null,
    [presentation, text],
  );

  if (!renderableSegments.length) {
    return null;
  }
  if (taskNotification) {
    return (
      <div className="message-rich-content">
        <TaskNotificationCard notification={taskNotification} />
      </div>
    );
  }

  const renderSegment = (segment: TranscriptSegment): ReactNode => {
    if (segment.kind === "text") {
      const restartNotice = parseRestartNoticeText(segment.text);
      if (restartNotice) {
        return <RestartNoticeCard key={segment.key} notice={restartNotice} />;
      }
      return (
        <RichMessageText
          key={segment.key}
          onLocalFileLinkClick={onLocalFileLinkClick}
          renderLocalImage={renderLocalMarkdownImage}
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

    if (segment.kind === "image_reference") {
      const fallbackSegment: Extract<TranscriptSegment, { kind: "file" }> = {
        kind: "file",
        key: `${segment.key}:fallback`,
        path: segment.path,
        label: segment.label,
        mediaType: segment.mediaType,
      };
      return (
        <MessagePathImageAttachmentFrame
          alt={segment.label}
          compact={layout === "media_above"}
          fallback={
            <MessageFileAttachmentCard
              onLocalFileLinkClick={onLocalFileLinkClick}
              segment={fallbackSegment}
            />
          }
          imageKey={segment.key}
          key={segment.key}
          loadImagePreview={loadImagePreview}
          path={segment.path}
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
      (segment) =>
        segment.kind === "image" || segment.kind === "image_reference",
    );
    const bodySegments = renderableSegments.filter(
      (segment) =>
        segment.kind !== "image" && segment.kind !== "image_reference",
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
