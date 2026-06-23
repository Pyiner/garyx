import { RateLimitBanner } from 'garyx-desktop';

// RenderRateLimit fields the banner reads: provider, window, resetAt, willAutoResend.
// These cells omit resetAt so the copy is deterministic (no live countdown).
const wrap: React.CSSProperties = { padding: 16, maxWidth: 460 };

export const AutoResend = () => (
  <div style={wrap}>
    <RateLimitBanner rateLimit={{ provider: 'codex', window: '5h', willAutoResend: true } as any} />
  </div>
);

export const Exhausted = () => (
  <div style={wrap}>
    <RateLimitBanner rateLimit={{ provider: 'Claude', window: 'weekly', willAutoResend: false } as any} />
  </div>
);
