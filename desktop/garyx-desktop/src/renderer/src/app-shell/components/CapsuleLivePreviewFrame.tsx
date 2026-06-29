import { useEffect, useRef, useSyncExternalStore } from 'react';

import { useI18n } from '../../i18n';
import {
  capsuleHtmlCacheKey,
  capsuleHtmlStore,
  type CapsuleHtmlState,
} from '../capsule-html-store';

// Virtual viewport the card thumbnail renders at before being scaled down to
// the card width. Keeps every thumbnail a consistent "desktop page" crop.
const CARD_VIEWPORT_WIDTH = 1024;
const CARD_VIEWPORT_HEIGHT = 640;

/**
 * Subscribe to the shared store for one capsule revision. `active` gates the
 * fetch so off-screen cards (IntersectionObserver) and unmounted previews never
 * load HTML. The returned snapshot is referentially stable per state, so cards
 * for other capsules don't re-render when this one loads.
 */
export function useCapsuleHtml(
  capsuleId: string,
  revision: number,
  options: { active: boolean },
): CapsuleHtmlState {
  const key = capsuleHtmlCacheKey(capsuleId, revision);
  const state = useSyncExternalStore(capsuleHtmlStore.subscribe, () =>
    capsuleHtmlStore.getState(key),
  );
  const { active } = options;
  useEffect(() => {
    if (active) {
      capsuleHtmlStore.request(capsuleId, revision, {});
    }
  }, [active, capsuleId, revision]);
  return state;
}

type CapsuleLivePreviewFrameProps = {
  capsuleId: string;
  revision: number;
  title: string;
  mode: 'card' | 'preview';
  /** card: driven by IntersectionObserver; preview: always true. */
  active: boolean;
};

/**
 * The one Capsule renderer reused by the gallery, the focused preview, and chat
 * cards. The served HTML is untrusted: it is fetched through the main process
 * (auth lives there, not in the renderer) and run in an opaque-origin iframe
 * (`sandbox="allow-scripts"`, never `allow-same-origin`, never a webview).
 */
export function CapsuleLivePreviewFrame({
  capsuleId,
  revision,
  title,
  mode,
  active,
}: CapsuleLivePreviewFrameProps) {
  const { t } = useI18n();
  const state = useCapsuleHtml(capsuleId, revision, { active });
  const cacheKey = capsuleHtmlCacheKey(capsuleId, revision);
  const canvasRef = useRef<HTMLDivElement | null>(null);

  // Card thumbnails render at a fixed virtual viewport and scale to fit the
  // card width. ResizeObserver fires on layout changes (not per frame), so the
  // scale stays correct across the responsive gallery grid.
  useEffect(() => {
    if (mode !== 'card') {
      return;
    }
    const el = canvasRef.current;
    if (!el || typeof ResizeObserver === 'undefined') {
      return;
    }
    const apply = () => {
      const width = el.clientWidth;
      if (width > 0) {
        el.style.setProperty(
          '--capsule-card-scale',
          String(width / CARD_VIEWPORT_WIDTH),
        );
      }
    };
    apply();
    const observer = new ResizeObserver(apply);
    observer.observe(el);
    return () => observer.disconnect();
  }, [mode]);

  if (state.status === 'deleted') {
    return (
      <div className="capsule-frame-state capsule-frame-deleted">
        {t('Capsule deleted')}
      </div>
    );
  }

  if (state.status !== 'ready') {
    // idle / loading / error → quiet skeleton. Cards never surface a red error;
    // the focused preview shows the retryable message inline.
    return (
      <div className="capsule-frame-state capsule-frame-skeleton">
        {mode === 'preview' && state.status === 'error' ? (
          <span className="capsule-frame-error-text">{state.message}</span>
        ) : null}
      </div>
    );
  }

  const frame = (
    <iframe
      className="capsule-live-frame"
      key={cacheKey}
      sandbox="allow-scripts"
      srcDoc={state.html}
      title={title}
      tabIndex={mode === 'card' ? -1 : 0}
      style={
        mode === 'card'
          ? { width: CARD_VIEWPORT_WIDTH, height: CARD_VIEWPORT_HEIGHT }
          : undefined
      }
    />
  );

  if (mode === 'card') {
    return (
      <div className="capsule-card-frame-canvas" ref={canvasRef}>
        {frame}
      </div>
    );
  }
  return frame;
}
