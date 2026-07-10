import {
  memo,
  useMemo,
  type ComponentProps,
  type MouseEvent,
  type ReactNode,
} from 'react';
import { cjk } from '@streamdown/cjk';
import { createCodePlugin } from '@streamdown/code';
import {
  Streamdown,
  type Components,
  type StreamdownTranslations,
} from 'streamdown';

import { useI18n } from './i18n';
import { localFilePathFromMessageLinkHref } from './message-local-images';
import { prepareMessageMarkdown } from './message-markdown-preprocess';
import {
  CHAT_MESSAGE_REHYPE_PLUGINS,
  CHAT_MESSAGE_REMARK_PLUGINS,
} from './message-rich-text-plugins';

type RichMessageTone = 'default' | 'assistant';

export type LocalFileLinkHandler = (absolutePath: string) => void;
export type LocalMessageImageRenderer = (image: {
  alt: string;
  path: string;
}) => ReactNode;

const garyxCodePlugin = createCodePlugin({
  themes: ['github-light', 'github-dark'],
});

const STREAMDOWN_CONTROLS = {
  code: {
    copy: true,
    download: false,
  },
  mermaid: false,
  table: false,
} as const;

export { localFilePathFromMessageLinkHref } from './message-local-images';

function normalizeMessageLinkHref(target: string): string | null {
  if (!target) {
    return null;
  }
  if (/^(https?:\/\/|mailto:)/i.test(target)) {
    return target;
  }
  const localFilePath = localFilePathFromMessageLinkHref(target);
  if (localFilePath) {
    return `file://${localFilePath}`;
  }
  return null;
}

function streamdownUrlTransform(target: string): string | null {
  if (target === 'streamdown:incomplete-link') {
    return target;
  }
  if (/^(https?:\/\/|mailto:)/i.test(target)) {
    return target;
  }
  const localFilePath = localFilePathFromMessageLinkHref(target);
  return localFilePath || null;
}

function useStreamdownTranslations(): Partial<StreamdownTranslations> {
  const { t } = useI18n();
  return useMemo(
    () => ({
      close: t('Close'),
      copied: t('Copied'),
      copyCode: t('Copy code'),
      copyLink: t('Copy link'),
      copyTable: t('Copy table'),
      copyTableAsCsv: t('Copy table as CSV'),
      copyTableAsMarkdown: t('Copy table as Markdown'),
      copyTableAsTsv: t('Copy table as TSV'),
      downloadTable: t('Download table'),
      exitFullscreen: t('Exit fullscreen'),
      externalLinkWarning: t("You're about to visit an external website."),
      openExternalLink: t('Open external link?'),
      openLink: t('Open link'),
      tableFormatCsv: t('CSV'),
      tableFormatMarkdown: t('Markdown'),
      tableFormatTsv: t('TSV'),
      viewFullscreen: t('View fullscreen'),
    }),
    [t],
  );
}

export const RichMessageText = memo(function RichMessageText({
  text,
  tone = 'default',
  onLocalFileLinkClick,
  renderLocalImage,
  surfaceCustomXmlTags = true,
}: {
  text: string;
  tone?: RichMessageTone;
  onLocalFileLinkClick?: LocalFileLinkHandler;
  renderLocalImage?: LocalMessageImageRenderer;
  surfaceCustomXmlTags?: boolean;
}) {
  const translations = useStreamdownTranslations();
  // Hide Garyx-internal injected tags before Streamdown renders. Unknown XML
  // should remain visible text, otherwise Streamdown treats it like HTML and
  // drops the tags while keeping their children.
  const prepared = useMemo(
    () => prepareMessageMarkdown(text, { surfaceCustomXmlTags }),
    [surfaceCustomXmlTags, text],
  );
  const components = useMemo<Components>(
    () => ({
      a({
        children,
        href,
        node: _node,
        ...props
      }: ComponentProps<'a'> & { node?: unknown }) {
        const rawHref = href || '';
        const localFilePath = localFilePathFromMessageLinkHref(rawHref);
        const normalizedHref = normalizeMessageLinkHref(rawHref);
        const interceptsLocalFileLink = Boolean(
          localFilePath && onLocalFileLinkClick,
        );
        if (!normalizedHref) {
          return <>{children}</>;
        }
        const handleClick = (event: MouseEvent<HTMLAnchorElement>) => {
          if (!interceptsLocalFileLink || !localFilePath || !onLocalFileLinkClick) {
            return;
          }
          event.preventDefault();
          onLocalFileLinkClick(localFilePath);
        };
        return (
          <a
            {...props}
            href={normalizedHref}
            onClick={handleClick}
            rel={interceptsLocalFileLink ? undefined : 'noreferrer'}
            target={interceptsLocalFileLink ? undefined : '_blank'}
          >
            {children}
          </a>
        );
      },
      'garyx-local-image'({ alt, path }) {
        const localPath = typeof path === 'string' ? path : '';
        const label = typeof alt === 'string' ? alt : '';
        if (!localPath) {
          return null;
        }
        if (!renderLocalImage) {
          return (
            <span className="message-local-image-fallback" title={localPath}>
              {label || localPath.split('/').pop() || localPath}
            </span>
          );
        }
        return <>{renderLocalImage({ alt: label, path: localPath })}</>;
      },
    }),
    [onLocalFileLinkClick, renderLocalImage],
  );

  return (
    <div className={`message-rich ${tone === 'assistant' ? 'message-rich-assistant' : 'message-rich-default'}`}>
      <Streamdown
        components={components}
        controls={STREAMDOWN_CONTROLS}
        dir="auto"
        lineNumbers={false}
        mode="streaming"
        normalizeHtmlIndentation
        plugins={{ cjk, code: garyxCodePlugin }}
        rehypePlugins={CHAT_MESSAGE_REHYPE_PLUGINS}
        remarkPlugins={CHAT_MESSAGE_REMARK_PLUGINS}
        translations={translations}
        urlTransform={streamdownUrlTransform}
      >
        {prepared}
      </Streamdown>
    </div>
  );
});
