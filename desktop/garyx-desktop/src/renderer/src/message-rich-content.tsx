import {
  createContext,
  memo,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
} from "react";
import { Download, FileText, Maximize2 } from "lucide-react";

import {
  Dialog,
  DialogBody,
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
  parseRestartNoticeText,
  type ParsedRestartNotice,
} from "./restart-notice";


import {
  collectTranscriptSegments,
  countTranscriptFiles,
  countTranscriptImages,
  extractTranscriptText,
  fallbackJsonSegment,
  stripTaskNotificationEnvelope,
  taskNotificationOverflows,
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

type TaskNotificationPresentation = Extract<
  RenderMessagePresentation,
  { kind: "task_notification" }
>;

type TaskNotificationSnapshot = {
  presentation: TaskNotificationPresentation;
  body: string;
  onLocalFileLinkClick?: LocalFileLinkHandler;
  renderLocalImage?: LocalMessageImageRenderer;
};

type TaskNotificationDialogContextValue = {
  open: (snapshot: TaskNotificationSnapshot, returnFocus: HTMLElement) => void;
};

const TaskNotificationDialogContext =
  createContext<TaskNotificationDialogContextValue | null>(null);

const TASK_NOTIFICATION_LINE_COUNT = 10;
const TASK_NOTIFICATION_OVERFLOW_EPSILON_PX = 0.5;
const TASK_NOTIFICATION_INTERACTIVE_SELECTOR = [
  "a",
  "button",
  "input",
  "select",
  "textarea",
  "summary",
  "[contenteditable]",
  "[role='button']",
  "[role='link']",
  "[data-task-notification-interactive]",
].join(",");

function TaskNotificationHeader({
  dialogTitle = false,
  notification,
}: {
  dialogTitle?: boolean;
  notification: TaskNotificationPresentation;
}) {
  const { t } = useI18n();
  const title = notification.title || t("Task ready for review");
  return (
    <div className="task-notification-header">
      <div className="task-notification-heading">
        <div className="task-notification-kicker">
          <span className="task-notification-task-id">
            {notification.task_id || t("Task")}
          </span>
          <span className="task-notification-status">
            {taskNotificationStatusLabel(notification.status, t)}
          </span>
        </div>
        {dialogTitle ? (
          <DialogTitle className="task-notification-title">{title}</DialogTitle>
        ) : (
          <h3 className="task-notification-title">{title}</h3>
        )}
      </div>
    </div>
  );
}

export function TaskNotificationDialogOwner({
  children,
  scopeKey,
}: {
  children: ReactNode;
  scopeKey: string;
}) {
  const { t } = useI18n();
  const [snapshot, setSnapshot] =
    useState<TaskNotificationSnapshot | null>(null);
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const open = useCallback(
    (next: TaskNotificationSnapshot, returnFocus: HTMLElement) => {
      returnFocusRef.current = returnFocus;
      setSnapshot(next);
    },
    [],
  );
  const contextValue = useMemo(() => ({ open }), [open]);

  useEffect(() => {
    returnFocusRef.current = null;
    setSnapshot(null);
  }, [scopeKey]);

  return (
    <TaskNotificationDialogContext.Provider value={contextValue}>
      {children}
      <Dialog
        open={snapshot !== null}
        onOpenChange={(isOpen) => {
          if (!isOpen) {
            setSnapshot(null);
          }
        }}
      >
        {snapshot ? (
          <DialogContent
            aria-describedby="task-notification-dialog-description"
            className="task-notification-dialog"
            onCloseAutoFocus={(event) => {
              event.preventDefault();
              const returnFocus = returnFocusRef.current;
              if (returnFocus?.isConnected) {
                returnFocus.focus();
              }
            }}
            scroll="content"
            size="large"
          >
            <TaskNotificationHeader
              dialogTitle
              notification={snapshot.presentation}
            />
            <DialogDescription
              className="sr-only"
              id="task-notification-dialog-description"
            >
              {t("Complete task notification")}
            </DialogDescription>
            <DialogBody className="task-notification-dialog-body">
              <RichMessageText
                onLocalFileLinkClick={snapshot.onLocalFileLinkClick}
                renderLocalImage={snapshot.renderLocalImage}
                text={snapshot.body}
                tone="assistant"
              />
            </DialogBody>
          </DialogContent>
        ) : null}
      </Dialog>
    </TaskNotificationDialogContext.Provider>
  );
}

function useTaskNotificationClamp(body: string) {
  const viewportRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const [measurement, setMeasurement] = useState({
    clampHeight: 0,
    overflows: false,
  });

  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    const content = contentRef.current;
    if (!viewport || !content) {
      return;
    }

    let disposed = false;
    let animationFrame = 0;
    const measure = () => {
      animationFrame = 0;
      if (disposed) {
        return;
      }
      const richText = content.querySelector<HTMLElement>(".message-rich");
      const style = window.getComputedStyle(richText ?? content);
      let lineHeight = Number.parseFloat(style.lineHeight);
      if (!Number.isFinite(lineHeight)) {
        const fontSize = Number.parseFloat(style.fontSize);
        lineHeight = Number.isFinite(fontSize) ? fontSize * 1.45 : 0;
      }
      if (!(lineHeight > 0)) {
        return;
      }
      const clampHeight = lineHeight * TASK_NOTIFICATION_LINE_COUNT;
      viewport.style.setProperty(
        "--task-notification-clamp-height",
        `${clampHeight}px`,
      );
      const naturalHeight = Math.max(
        content.scrollHeight,
        content.getBoundingClientRect().height,
      );
      const overflows = taskNotificationOverflows(
        naturalHeight,
        clampHeight,
        TASK_NOTIFICATION_OVERFLOW_EPSILON_PX,
      );
      setMeasurement((current) =>
        current.clampHeight === clampHeight && current.overflows === overflows
          ? current
          : { clampHeight, overflows },
      );
    };
    const scheduleMeasure = () => {
      if (!animationFrame) {
        animationFrame = window.requestAnimationFrame(measure);
      }
    };

    measure();
    const resizeObserver = new ResizeObserver(scheduleMeasure);
    resizeObserver.observe(viewport);
    resizeObserver.observe(content);
    const mutationObserver = new MutationObserver(scheduleMeasure);
    mutationObserver.observe(content, {
      attributes: true,
      characterData: true,
      childList: true,
      subtree: true,
    });
    content.addEventListener("load", scheduleMeasure, true);
    window.addEventListener("resize", scheduleMeasure);

    const fonts = document.fonts;
    void fonts?.ready.then(scheduleMeasure);
    fonts?.addEventListener?.("loadingdone", scheduleMeasure);

    return () => {
      disposed = true;
      if (animationFrame) {
        window.cancelAnimationFrame(animationFrame);
      }
      resizeObserver.disconnect();
      mutationObserver.disconnect();
      content.removeEventListener("load", scheduleMeasure, true);
      window.removeEventListener("resize", scheduleMeasure);
      fonts?.removeEventListener?.("loadingdone", scheduleMeasure);
    };
  }, [body]);

  return { ...measurement, contentRef, viewportRef };
}

function taskNotificationClickIsInteractive(
  card: HTMLElement,
  target: EventTarget | null,
): boolean {
  return (
    target instanceof Element &&
    target !== card &&
    target.closest(TASK_NOTIFICATION_INTERACTIVE_SELECTOR) !== null
  );
}

function taskNotificationHasActiveSelection(card: HTMLElement): boolean {
  const selection = window.getSelection();
  return Boolean(
    selection &&
      !selection.isCollapsed &&
      ((selection.anchorNode && card.contains(selection.anchorNode)) ||
        (selection.focusNode && card.contains(selection.focusNode))),
  );
}

function TaskNotificationCard({
  notification,
  body,
  onLocalFileLinkClick,
  renderLocalImage,
}: {
  notification: TaskNotificationPresentation;
  body: string;
  onLocalFileLinkClick?: LocalFileLinkHandler;
  renderLocalImage?: LocalMessageImageRenderer;
}) {
  const { t } = useI18n();
  const dialog = useContext(TaskNotificationDialogContext);
  const cardRef = useRef<HTMLElement>(null);
  const { clampHeight, contentRef, overflows, viewportRef } =
    useTaskNotificationClamp(body);
  const activate = useCallback(() => {
    const card = cardRef.current;
    if (!overflows || !card || !dialog) {
      return;
    }
    const presentation = Object.freeze({ ...notification });
    dialog.open(
      Object.freeze({
        body,
        onLocalFileLinkClick,
        presentation,
        renderLocalImage,
      }),
      card,
    );
  }, [body, dialog, notification, onLocalFileLinkClick, overflows, renderLocalImage]);
  const handleCardClick = useCallback(
    (event: ReactMouseEvent<HTMLElement>) => {
      if (
        !overflows ||
        taskNotificationClickIsInteractive(event.currentTarget, event.target) ||
        taskNotificationHasActiveSelection(event.currentTarget)
      ) {
        return;
      }
      activate();
    },
    [activate, overflows],
  );

  return (
    <>
      <section
        aria-label={t("Task ready for review")}
        className="task-notification-card"
        data-overflow={overflows ? "true" : "false"}
        onClick={handleCardClick}
        ref={cardRef}
        tabIndex={-1}
      >
        <TaskNotificationHeader notification={notification} />

        <div className="task-notification-body">
          <div
            className="task-notification-body-viewport"
            data-overflow={overflows ? "true" : "false"}
            ref={viewportRef}
            style={
              clampHeight > 0
                ? ({
                    "--task-notification-clamp-height": `${clampHeight}px`,
                  } as CSSProperties)
                : undefined
            }
          >
            <div className="task-notification-body-content" ref={contentRef}>
              <RichMessageText
                onLocalFileLinkClick={onLocalFileLinkClick}
                renderLocalImage={renderLocalImage}
                text={body}
                tone="assistant"
              />
            </div>
          </div>
          {overflows ? (
            <button
              aria-label={t("Expand task notification")}
              className="task-notification-expand"
              data-task-notification-interactive
              onClick={activate}
              type="button"
            >
              <Maximize2 aria-hidden size={14} strokeWidth={1.9} />
              <span>{t("Expand")}</span>
            </button>
          ) : null}
        </div>
      </section>
    </>
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
    () => {
      if (presentation?.kind !== "task_notification") {
        return null;
      }
      const body = stripTaskNotificationEnvelope(text);
      return body === null ? null : { presentation, body };
    },
    [presentation, text],
  );

  if (!renderableSegments.length) {
    return null;
  }
  if (taskNotification) {
    return (
      <div className="message-rich-content">
        <TaskNotificationCard
          body={taskNotification.body}
          notification={taskNotification.presentation}
          onLocalFileLinkClick={onLocalFileLinkClick}
          renderLocalImage={renderLocalMarkdownImage}
        />
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
