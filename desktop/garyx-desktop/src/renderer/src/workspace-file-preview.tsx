import { Children, isValidElement, useEffect, useId, useState, type ReactNode } from 'react';
import Prism from 'prismjs';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import 'prismjs/components/prism-bash';
import 'prismjs/components/prism-clike';
import 'prismjs/components/prism-c';
import 'prismjs/components/prism-cpp';
import 'prismjs/components/prism-css';
import 'prismjs/components/prism-diff';
import 'prismjs/components/prism-go';
import 'prismjs/components/prism-ini';
import 'prismjs/components/prism-java';
import 'prismjs/components/prism-javascript';
import 'prismjs/components/prism-json';
import 'prismjs/components/prism-markdown';
import 'prismjs/components/prism-markup';
import 'prismjs/components/prism-markup-templating';
import 'prismjs/components/prism-jsx';
import 'prismjs/components/prism-python';
import 'prismjs/components/prism-rust';
import 'prismjs/components/prism-sql';
import 'prismjs/components/prism-toml';
import 'prismjs/components/prism-typescript';
import 'prismjs/components/prism-tsx';
import 'prismjs/components/prism-yaml';

import type { DesktopWorkspaceFilePreview } from '@shared/contracts';
import {
  localFilePathFromMessageLinkHref,
  type LocalFileLinkHandler,
} from './message-rich-text';

const MAX_HIGHLIGHT_CHAR_COUNT = 200_000;

const LANGUAGE_BY_EXTENSION: Record<string, string> = {
  bash: 'bash',
  c: 'c',
  cc: 'cpp',
  conf: 'ini',
  cpp: 'cpp',
  css: 'css',
  cts: 'typescript',
  cxx: 'cpp',
  diff: 'diff',
  go: 'go',
  h: 'c',
  hpp: 'cpp',
  htm: 'markup',
  html: 'markup',
  ini: 'ini',
  java: 'java',
  js: 'javascript',
  json: 'json',
  jsx: 'jsx',
  md: 'markdown',
  mjs: 'javascript',
  mts: 'typescript',
  patch: 'diff',
  py: 'python',
  rs: 'rust',
  sh: 'bash',
  sql: 'sql',
  svg: 'markup',
  toml: 'toml',
  ts: 'typescript',
  tsx: 'tsx',
  txt: 'text',
  xml: 'markup',
  yaml: 'yaml',
  yml: 'yaml',
  zsh: 'bash',
};

function normalizeLanguageAlias(language: string): string {
  const normalized = language.trim().toLowerCase();
  if (!normalized) {
    return '';
  }

  if (['cjs', 'js', 'mjs', 'node'].includes(normalized)) {
    return 'javascript';
  }
  if (['ts', 'mts', 'cts'].includes(normalized)) {
    return 'typescript';
  }
  if (['html', 'htm', 'xml', 'svg'].includes(normalized)) {
    return 'markup';
  }
  if (['shell', 'sh', 'zsh'].includes(normalized)) {
    return 'bash';
  }
  if (['yml'].includes(normalized)) {
    return 'yaml';
  }
  if (['md', 'mdx'].includes(normalized)) {
    return 'markdown';
  }
  if (['conf', 'cfg'].includes(normalized)) {
    return 'ini';
  }
  if (['text', 'plaintext', 'plain'].includes(normalized)) {
    return 'text';
  }
  return normalized;
}

function inferLanguageFromFileName(fileName: string): string {
  const normalized = fileName.trim().toLowerCase();
  if (!normalized) {
    return 'text';
  }

  if (normalized === 'dockerfile') {
    return 'bash';
  }

  const dotIndex = normalized.lastIndexOf('.');
  if (dotIndex < 0 || dotIndex === normalized.length - 1) {
    return 'text';
  }

  return LANGUAGE_BY_EXTENSION[normalized.slice(dotIndex + 1)] || 'text';
}

function CodePreview({
  source,
  fileName,
  language,
}: {
  source: string;
  fileName?: string;
  language?: string;
}) {
  const normalizedLanguage = normalizeLanguageAlias(language || '') || inferLanguageFromFileName(fileName || '');
  const prismLanguage = normalizedLanguage !== 'text' && Prism.languages[normalizedLanguage]
    ? normalizedLanguage
    : '';
  const showHighlight = Boolean(prismLanguage) && source.length <= MAX_HIGHLIGHT_CHAR_COUNT;
  const highlightedHtml = showHighlight
    ? Prism.highlight(source, Prism.languages[prismLanguage], prismLanguage)
    : '';

  return (
    <div className={`workspace-file-code-frame ${showHighlight ? 'is-highlighted' : 'is-plain'}`}>
      <pre className="workspace-file-code-block">
        {showHighlight ? (
          <code
            className={`language-${prismLanguage}`}
            dangerouslySetInnerHTML={{ __html: highlightedHtml }}
          />
        ) : (
          <code>{source}</code>
        )}
      </pre>
    </div>
  );
}

function decodeBase64(dataBase64: string): Uint8Array {
  const binary = atob(dataBase64);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

function usePreviewBlobUrl(preview: DesktopWorkspaceFilePreview | null): string | null {
  const [url, setUrl] = useState<string | null>(null);

  useEffect(() => {
    if (!preview?.dataBase64 || !preview.mediaType) {
      setUrl((current) => {
        if (current) {
          URL.revokeObjectURL(current);
        }
        return null;
      });
      return undefined;
    }

    const bytes = decodeBase64(preview.dataBase64);
    const blob = new Blob([bytes as unknown as BlobPart], {
      type: preview.mediaType,
    });
    const nextUrl = URL.createObjectURL(blob);
    setUrl((current) => {
      if (current) {
        URL.revokeObjectURL(current);
      }
      return nextUrl;
    });

    return () => {
      URL.revokeObjectURL(nextUrl);
    };
  }, [preview?.dataBase64, preview?.mediaType]);

  return url;
}

function MermaidDiagram({ code }: { code: string }) {
  const id = useId().replace(/:/g, '-');
  const [svg, setSvg] = useState('');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    void (async () => {
      try {
        const mermaidModule = await import('mermaid');
        const mermaid = mermaidModule.default;
        mermaid.initialize({
          startOnLoad: false,
          securityLevel: 'strict',
          theme: 'neutral',
        });
        const renderId = `gary-mermaid-${id}-${Date.now()}`;
        const rendered = await mermaid.render(renderId, code);
        if (!cancelled) {
          setSvg(rendered.svg);
          setError(null);
        }
      } catch (renderError) {
        if (!cancelled) {
          setSvg('');
          setError(
            renderError instanceof Error
              ? renderError.message
              : 'Failed to render Mermaid diagram.',
          );
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [code, id]);

  if (error) {
    return <div className="workspace-file-mermaid-error">{error}</div>;
  }

  if (!svg) {
    return <div className="workspace-file-mermaid-loading">Rendering diagram…</div>;
  }

  return (
    <div
      className="workspace-file-mermaid"
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  );
}

function extractTextContent(node: ReactNode): string {
  return Children.toArray(node)
    .map((child) => {
      if (typeof child === 'string' || typeof child === 'number') {
        return String(child);
      }
      return '';
    })
    .join('');
}

function MarkdownDocument({
  text,
  onLocalFileLinkClick,
}: {
  text: string;
  onLocalFileLinkClick?: LocalFileLinkHandler;
}) {
  return (
    <div className="workspace-file-markdown">
      <ReactMarkdown
        components={{
          a({ children, href }) {
            const rawHref = href || '';
            const localFilePath = localFilePathFromMessageLinkHref(rawHref);
            const interceptsLocalFileLink = Boolean(localFilePath && onLocalFileLinkClick);
            return (
              <a
                href={localFilePath ? `file://${localFilePath}` : href}
                onClick={(event) => {
                  if (!interceptsLocalFileLink || !localFilePath || !onLocalFileLinkClick) {
                    return;
                  }
                  event.preventDefault();
                  onLocalFileLinkClick(localFilePath);
                }}
                rel={interceptsLocalFileLink ? undefined : 'noreferrer'}
                target={interceptsLocalFileLink ? undefined : '_blank'}
              >
                {children}
              </a>
            );
          },
          pre({ children }) {
            const childNodes = Children.toArray(children);
            const codeChild = childNodes.length === 1 ? childNodes[0] : null;

            if (isValidElement<{ children?: ReactNode; className?: string }>(codeChild)) {
              const className = typeof codeChild.props.className === 'string'
                ? codeChild.props.className
                : '';
              const language = className.replace(/^language-/, '').trim().toLowerCase();
              const source = extractTextContent(codeChild.props.children).replace(/\n$/, '');

              if (language === 'mermaid') {
                return <MermaidDiagram code={source} />;
              }

              return <CodePreview fileName="" language={language} source={source} />;
            }

            return <pre>{children}</pre>;
          },
          code(props: any) {
            const { children, className, ...rest } = props;
            return (
              <code className={className} {...rest}>
                {children}
              </code>
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

export function WorkspaceFilePreview({
  preview,
  onLocalFileLinkClick,
}: {
  preview: DesktopWorkspaceFilePreview | null;
  onLocalFileLinkClick?: LocalFileLinkHandler;
}) {
  const blobUrl = usePreviewBlobUrl(preview);

  if (!preview) {
    return (
      <div className="workspace-file-preview-empty">
        Select a file to preview it here.
      </div>
    );
  }

  if (preview.previewKind === 'markdown') {
    return (
      <MarkdownDocument
        onLocalFileLinkClick={onLocalFileLinkClick}
        text={preview.text || ''}
      />
    );
  }

  if (preview.previewKind === 'html') {
    return (
      <iframe
        className="workspace-file-iframe"
        sandbox="allow-same-origin"
        srcDoc={preview.text || '<body></body>'}
        title={preview.name}
      />
    );
  }

  if (preview.previewKind === 'pdf' && blobUrl) {
    return (
      <iframe
        className="workspace-file-iframe workspace-file-pdf"
        src={blobUrl}
        title={preview.name}
      />
    );
  }

  if (preview.previewKind === 'image' && blobUrl) {
    return (
      <div className="workspace-file-image-frame">
        <img
          alt={preview.name}
          className="workspace-file-image"
          src={blobUrl}
        />
      </div>
    );
  }

  if (preview.previewKind === 'text') {
    return <CodePreview fileName={preview.path || preview.name} source={preview.text || ''} />;
  }

  return (
    <div className="workspace-file-preview-empty">
      Preview is not available for this file type yet.
    </div>
  );
}
