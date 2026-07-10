type MessageHastNode = {
  type?: string;
  tagName?: string;
  properties?: Record<string, unknown>;
  children?: MessageHastNode[];
};

function normalizeLocalFilePath(target: string): string | null {
  const trimmed = target.trim();
  if (!trimmed) {
    return null;
  }
  const withoutQuery = trimmed.split('?')[0] || '';
  const withoutFragment = withoutQuery.split('#')[0] || '';
  const withoutLineSuffix = withoutFragment.replace(/:\d+(?::\d+)?$/, '');
  if (!withoutLineSuffix.startsWith('/')) {
    return null;
  }
  try {
    return decodeURIComponent(withoutLineSuffix);
  } catch {
    return withoutLineSuffix;
  }
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

function rewriteLocalMarkdownImages(node: MessageHastNode): void {
  if (node.type === 'element' && node.tagName === 'img') {
    const source = node.properties?.src;
    const path = typeof source === 'string'
      ? localFilePathFromMessageLinkHref(source)
      : null;
    if (path) {
      const alt = node.properties?.alt;
      node.tagName = 'garyx-local-image';
      node.properties = {
        alt: typeof alt === 'string' ? alt : '',
        path,
      };
      node.children = [];
      return;
    }
  }

  for (const child of node.children || []) {
    rewriteLocalMarkdownImages(child);
  }
}

/**
 * Replaces only absolute-path Markdown images with a renderer-owned sentinel.
 * It runs after Streamdown's sanitizer, so ordinary remote images retain
 * Streamdown's native image wrapper, loading state, and controls.
 */
export function rehypeLocalMessageImages() {
  return (tree: MessageHastNode) => {
    rewriteLocalMarkdownImages(tree);
  };
}
