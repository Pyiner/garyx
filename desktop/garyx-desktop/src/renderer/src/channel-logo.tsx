import type { CSSProperties } from 'react';

import { IconMessages } from '@tabler/icons-react';

function hashCode(str: string): number {
  let hash = 0;
  for (let index = 0; index < str.length; index += 1) {
    hash = ((hash << 5) - hash + str.charCodeAt(index)) | 0;
  }
  return Math.abs(hash);
}

function fallbackTone(seed?: string | null): CSSProperties | undefined {
  const palette = [
    { background: '#e9f2ff', color: '#2457c5' },
    { background: '#edf8ef', color: '#237a43' },
    { background: '#fff1e6', color: '#b85a14' },
    { background: '#f4ecff', color: '#7345b6' },
    { background: '#eaf7f7', color: '#146a6a' },
    { background: '#fff0f3', color: '#b23a62' },
  ];
  const source = seed?.trim();
  if (!source) return undefined;
  return palette[hashCode(source) % palette.length];
}

function initialsForChannel(channel?: string | null): string {
  const normalized = channel?.trim();
  if (!normalized) {
    return '?';
  }
  const parts = normalized
    .split(/[^a-zA-Z0-9]+/)
    .map((part) => part.trim())
    .filter(Boolean);
  if (parts.length === 0) {
    return normalized.slice(0, 1).toUpperCase();
  }
  if (parts.length === 1) {
    return parts[0].slice(0, 1).toUpperCase();
  }
  return `${parts[0][0]}${parts[1][0]}`.toUpperCase();
}

function initialsForLabel(label?: string | null): string | null {
  const normalized = label?.trim();
  if (!normalized) {
    return null;
  }
  const slashSegment = normalized.split('/').filter(Boolean).at(-1) || normalized;
  const token = slashSegment.split(/[\s_-]+/).filter(Boolean).at(-1) || slashSegment;
  const chars = Array.from(token.trim());
  if (chars.length === 0) {
    return null;
  }
  if (/^[a-z0-9]+$/i.test(token)) {
    return chars.slice(0, Math.min(2, chars.length)).join('').toUpperCase();
  }
  return chars.slice(0, Math.min(2, chars.length)).join('');
}

export function ChannelLogo({
  channel,
  className = 'channel-logo',
  iconDataUrl,
  fallbackLabel,
}: {
  channel?: string | null;
  className?: string;
  iconDataUrl?: string | null;
  fallbackLabel?: string | null;
}) {
  if (iconDataUrl) {
    return (
      <span className={`channel-logo-wrap ${className}`}>
        <img alt="" className="channel-logo-svg" src={iconDataUrl} />
      </span>
    );
  }

  const initials = initialsForLabel(fallbackLabel) || initialsForChannel(channel);
  if (initials !== '?') {
    return (
      <span className={`channel-logo-wrap ${className}`}>
        <span
          className="channel-logo-fallback"
          style={fallbackTone(fallbackLabel || channel)}
        >
          {initials}
        </span>
      </span>
    );
  }

  return <IconMessages aria-hidden className={className} size={14} stroke={1.7} />;
}
