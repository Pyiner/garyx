import { useEffect, useSyncExternalStore } from 'react';

import { useI18n } from '../../i18n';
import {
  capsuleHtmlCacheKey,
  capsuleHtmlStore,
  type CapsuleHtmlState,
} from '../capsule-html-store';
import {
  capsuleThumbnailCacheKey,
  capsuleThumbnailStore,
  type CapsuleThumbnailRendition,
  type CapsuleThumbnailState,
  GALLERY_RENDITION,
} from '../capsule-thumbnail-store';

/**
 * Subscribe to the shared HTML store for one capsule revision. `active` gates
 * the fetch so off-screen previews never load HTML. Used by the focused preview
 * (the only surface that still renders a live iframe).
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
  const cacheMiss = state.status === 'idle';
  useEffect(() => {
    if (active) {
      capsuleHtmlStore.request(capsuleId, revision, {});
    }
  }, [active, cacheMiss, capsuleId, revision]);
  return state;
}

/**
 * Subscribe to the shared rendered-thumbnail store for one capsule
 * revision+rendition. `active` gates the render request so off-screen cards
 * (IntersectionObserver) never render. The returned snapshot is referentially
 * stable per state, so cards for other capsules don't re-render when this loads.
 */
function useCapsuleThumbnail(
  capsuleId: string,
  revision: number,
  rendition: CapsuleThumbnailRendition,
  options: { active: boolean },
): CapsuleThumbnailState {
  const key = capsuleThumbnailCacheKey(capsuleId, revision, rendition);
  const state = useSyncExternalStore(capsuleThumbnailStore.subscribe, () =>
    capsuleThumbnailStore.getState(key),
  );
  const { active } = options;
  const cacheMiss = state.status === 'idle';
  useEffect(() => {
    if (active) {
      capsuleThumbnailStore.request(capsuleId, revision, rendition, {});
    }
  }, [
    active,
    cacheMiss,
    capsuleId,
    revision,
    rendition.aspectWidth,
    rendition.aspectHeight,
  ]);
  return state;
}

type CapsuleLivePreviewFrameProps = {
  capsuleId: string;
  revision: number;
  title: string;
  mode: 'card' | 'preview';
  /** card: driven by IntersectionObserver; preview: always true. */
  active: boolean;
  /** Card thumbnail aspect; ignored in preview mode. Defaults to gallery 16:10. */
  rendition?: CapsuleThumbnailRendition;
};

/**
 * The Capsule renderer reused by the gallery, chat cards, and the focused
 * preview. The served HTML is untrusted, so:
 *
 * - **card** mode shows a cached thumbnail PNG (`<img>`) rendered once by the
 *   main process into a hidden sandboxed window — zero live iframe/webview in
 *   steady-state browsing.
 * - **preview** mode (the focused full-screen surface) runs the HTML in an
 *   opaque-origin iframe (`sandbox="allow-scripts"`, never `allow-same-origin`,
 *   never a webview) so it stays interactive.
 */
export function CapsuleLivePreviewFrame({
  capsuleId,
  revision,
  title,
  mode,
  active,
  rendition = GALLERY_RENDITION,
}: CapsuleLivePreviewFrameProps) {
  if (mode === 'card') {
    return (
      <CapsuleCardThumbnail
        active={active}
        capsuleId={capsuleId}
        rendition={rendition}
        revision={revision}
        title={title}
      />
    );
  }
  return (
    <CapsulePreviewIframe
      active={active}
      capsuleId={capsuleId}
      revision={revision}
      title={title}
    />
  );
}

/** Card thumbnail: a cached PNG, cover-cropped top-anchored. No live iframe. */
function CapsuleCardThumbnail({
  capsuleId,
  revision,
  rendition,
  title,
  active,
}: {
  capsuleId: string;
  revision: number;
  rendition: CapsuleThumbnailRendition;
  title: string;
  active: boolean;
}) {
  const { t } = useI18n();
  const state = useCapsuleThumbnail(capsuleId, revision, rendition, { active });

  if (state.status === 'deleted') {
    return (
      <div className="capsule-frame-state capsule-frame-deleted">
        {t('Capsule deleted')}
      </div>
    );
  }
  if (state.status !== 'ready') {
    // idle / loading / error → quiet skeleton (cards never surface a red error).
    return <div className="capsule-frame-state capsule-frame-skeleton" />;
  }
  return <img alt={title} className="capsule-card-thumb" src={state.dataUrl} />;
}

/** Focused preview: live, interactive, opaque-origin sandboxed iframe. */
function CapsulePreviewIframe({
  capsuleId,
  revision,
  title,
  active,
}: {
  capsuleId: string;
  revision: number;
  title: string;
  active: boolean;
}) {
  const { t } = useI18n();
  const state = useCapsuleHtml(capsuleId, revision, { active });
  const cacheKey = capsuleHtmlCacheKey(capsuleId, revision);

  if (state.status === 'deleted') {
    return (
      <div className="capsule-frame-state capsule-frame-deleted">
        {t('Capsule deleted')}
      </div>
    );
  }
  if (state.status !== 'ready') {
    return (
      <div className="capsule-frame-state capsule-frame-skeleton">
        {state.status === 'error' ? (
          <span className="capsule-frame-error-text">{state.message}</span>
        ) : null}
      </div>
    );
  }
  return (
    <iframe
      className="capsule-live-frame"
      key={cacheKey}
      sandbox="allow-scripts"
      srcDoc={state.html}
      title={title}
    />
  );
}
