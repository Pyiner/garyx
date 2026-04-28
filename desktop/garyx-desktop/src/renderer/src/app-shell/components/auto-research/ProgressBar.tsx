import { useState } from 'react';

/* ── Small presentational components shared across AutoResearch views ── */

/* Progress bar */
export function ProgressBar({
  value,
  max,
}: {
  value: number;
  max: number;
}) {
  const pct = max > 0 ? Math.min(100, (value / max) * 100) : 0;
  const barColor = pct >= 90 ? 'var(--color-token-error-foreground)' : pct >= 70 ? 'var(--color-token-warning-foreground)' : 'var(--color-token-text-primary)';

  return (
    <div style={{ height: 6, width: '100%', overflow: 'hidden', borderRadius: 9999, background: 'var(--color-token-bg-tertiary)', marginTop: 8 }}>
      <div
        style={{ height: '100%', borderRadius: 9999, width: `${pct}%`, backgroundColor: barColor, transition: 'width 500ms' }}
      />
    </div>
  );
}

/* Section wrapper — supports collapsible mode */
export function Section({
  title,
  description,
  actions,
  children,
  collapsible = false,
  defaultOpen = true,
}: {
  title: string;
  description?: string;
  actions?: React.ReactNode;
  children: React.ReactNode;
  collapsible?: boolean;
  defaultOpen?: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);

  return (
    <section style={{ borderBottom: '1px solid var(--color-token-border-light)', padding: '16px 0' }}>
      <div
        style={{ marginBottom: open ? 12 : 0, display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', gap: 12, cursor: collapsible ? 'pointer' : undefined, userSelect: collapsible ? 'none' : undefined }}
        onClick={collapsible ? () => setOpen((v) => !v) : undefined}
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
          <h3 style={{ margin: 0, fontSize: 'var(--text-base)', fontWeight: 'var(--font-weight-semibold)', letterSpacing: '-0.01em', color: 'var(--color-token-text-primary)', display: 'flex', alignItems: 'center', gap: 6 }}>
            {collapsible ? (
              <svg style={{ width: 10, height: 10, transition: 'transform var(--duration-fast)', transform: open ? 'rotate(90deg)' : 'none', color: 'var(--color-token-description-foreground)' }} viewBox="0 0 10 10" fill="none">
                <path d="M3.5 1.5L7 5 3.5 8.5" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
            ) : null}
            {title}
          </h3>
          {description && open ? (
            <p className="codex-command-row-desc">{description}</p>
          ) : null}
        </div>
        {open ? actions : null}
      </div>
      {open ? children : null}
    </section>
  );
}

/* Detail item (label + value) */
export function DetailItem({
  label,
  value,
}: {
  label: string;
  value: React.ReactNode;
}) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
      <p style={{ fontSize: 10, fontWeight: 600, textTransform: 'uppercase', letterSpacing: '0.12em', color: 'var(--color-token-description-foreground)', opacity: 0.7 }}>
        {label}
      </p>
      <div style={{ fontSize: 13, lineHeight: 1.6, color: 'var(--color-token-text-primary)' }}>
        {value}
      </div>
    </div>
  );
}
