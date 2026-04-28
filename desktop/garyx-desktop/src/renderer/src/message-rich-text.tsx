import type { MouseEvent } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

type RichMessageTone = 'default' | 'assistant';

export type LocalFileLinkHandler = (absolutePath: string) => void;

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

export function RichMessageText({
  text,
  tone = 'default',
  onLocalFileLinkClick,
}: {
  text: string;
  tone?: RichMessageTone;
  onLocalFileLinkClick?: LocalFileLinkHandler;
}) {
  return (
    <div className={`message-rich ${tone === 'assistant' ? 'message-rich-assistant' : 'message-rich-default'}`}>
      <ReactMarkdown
        components={{
          a({ children, href }) {
            const rawHref = href || '';
            const localFilePath = localFilePathFromMessageLinkHref(rawHref);
            const normalizedHref = normalizeMessageLinkHref(rawHref);
            const interceptsLocalFileLink = Boolean(localFilePath && onLocalFileLinkClick);
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
                href={normalizedHref}
                onClick={handleClick}
                rel={interceptsLocalFileLink ? undefined : 'noreferrer'}
                target={interceptsLocalFileLink ? undefined : '_blank'}
              >
                {children}
              </a>
            );
          },
          table({ children }) {
            return (
              <div className="message-rich-table-wrap">
                <table>{children}</table>
              </div>
            );
          },
        }}
        remarkPlugins={[remarkGfm]}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
}
