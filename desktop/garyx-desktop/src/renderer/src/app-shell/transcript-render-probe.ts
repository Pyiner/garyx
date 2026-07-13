export type TranscriptRenderProbe = Readonly<{
  record: (surface: "AppShell" | "ThreadPage" | "ThreadPage.row", key?: string) => void;
}>;

declare global {
  interface Window {
    /** Dev-only performance oracle hook. Absent during ordinary app use. */
    __garyxTranscriptRenderProbe?: TranscriptRenderProbe;
  }
}

/**
 * Record render work only when a dev oracle explicitly installs a probe.
 * Production builds erase the branch, and normal dev sessions pay one absent
 * optional-property read without retaining counters or transcript data.
 */
export function recordTranscriptRender(
  surface: "AppShell" | "ThreadPage" | "ThreadPage.row",
  key?: string,
): void {
  if (!import.meta.env.DEV) {
    return;
  }
  window.__garyxTranscriptRenderProbe?.record(surface, key);
}

