import { useMemo, type ComponentProps, type MouseEvent } from 'react';
import { cjk } from '@streamdown/cjk';
import { createCodePlugin } from '@streamdown/code';
import {
  Streamdown,
  type Components,
  type StreamdownTranslations,
} from 'streamdown';

import { useI18n } from './i18n';

type RichMessageTone = 'default' | 'assistant';

export type LocalFileLinkHandler = (absolutePath: string) => void;

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

function normalizeLocalFilePath(target: string): string | null {
  const trimmed = target.trim();
  if (!trimmed) {
    return null;
  }
  const withoutQuery = trimmed.split('?')[0] || '';
  const withoutFragment = withoutQuery.split('#')[0] || '';
  return withoutFragment.startsWith('/') ? withoutFragment : null;
}

export function localFilePathFromMessageLinkHref(target: string): string | null {
  if (!target) {
    return null;
  }
  if (target.startsWith('/')) {
    return normalizeLocalFilePath(target);
  }
  if (/^file:\/\//i.test(target)) {
    try {
      const url = new URL(target);
      return normalizeLocalFilePath(decodeURIComponent(url.pathname || ''));
    } catch {
      return null;
    }
  }
  return null;
}

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

export function RichMessageText({
  text,
  tone = 'default',
  onLocalFileLinkClick,
}: {
  text: string;
  tone?: RichMessageTone;
  onLocalFileLinkClick?: LocalFileLinkHandler;
}) {
  const translations = useStreamdownTranslations();
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
    }),
    [onLocalFileLinkClick],
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
        translations={translations}
        urlTransform={streamdownUrlTransform}
      >
        {text}
      </Streamdown>
    </div>
  );
}
