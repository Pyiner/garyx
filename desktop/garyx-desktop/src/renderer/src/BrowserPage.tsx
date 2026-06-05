import { useEffect, useLayoutEffect, useRef, useState, type FormEvent } from 'react';

import type {
  DesktopBrowserAnnotationElement,
  DesktopBrowserAnnotationSnapshot,
  DesktopBrowserState,
} from '@shared/contracts';

import { Input } from '@/components/ui/input';
import { getDesktopApi } from './platform/desktop-api';
import { BrowserBackIcon, BrowserForwardIcon, BrowserRefreshIcon, BrowserCloseTabIcon, ExternalLinkIcon, NewTabIcon, BrowserIcon, LockIcon, InfoIcon } from './app-shell/icons';
import { useI18n } from './i18n';
import { Camera, MousePointer2 } from 'lucide-react';

function isHttpsUrl(url: string) {
  return url.startsWith('https://');
}

function activeTab(state: DesktopBrowserState | null) {
  return state?.tabs.find((tab) => tab.isActive) ?? null;
}

function loadCanvasImage(dataUrl: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => resolve(image);
    image.onerror = () => reject(new Error('Failed to load screenshot.'));
    image.src = dataUrl;
  });
}

function drawRoundedRect(
  context: CanvasRenderingContext2D,
  x: number,
  y: number,
  width: number,
  height: number,
  radius: number,
) {
  const nextRadius = Math.min(radius, width / 2, height / 2);
  context.beginPath();
  context.moveTo(x + nextRadius, y);
  context.lineTo(x + width - nextRadius, y);
  context.quadraticCurveTo(x + width, y, x + width, y + nextRadius);
  context.lineTo(x + width, y + height - nextRadius);
  context.quadraticCurveTo(x + width, y + height, x + width - nextRadius, y + height);
  context.lineTo(x + nextRadius, y + height);
  context.quadraticCurveTo(x, y + height, x, y + height - nextRadius);
  context.lineTo(x, y + nextRadius);
  context.quadraticCurveTo(x, y, x + nextRadius, y);
  context.closePath();
}

async function renderAnnotatedSnapshot(
  snapshot: DesktopBrowserAnnotationSnapshot,
): Promise<string> {
  const image = await loadCanvasImage(snapshot.dataUrl);
  const width = snapshot.width || image.naturalWidth;
  const height = snapshot.height || image.naturalHeight;
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext('2d');
  if (!context) {
    throw new Error('Canvas is unavailable.');
  }
  context.drawImage(image, 0, 0, width, height);

  const scaleX = width / Math.max(1, snapshot.viewportWidth || width);
  const scaleY = height / Math.max(1, snapshot.viewportHeight || height);
  const markerHeight = Math.max(18, Math.round(Math.min(width, height) * 0.027));
  context.textAlign = 'center';
  context.textBaseline = 'middle';
  context.font = `700 ${Math.max(11, Math.round(markerHeight * 0.62))}px system-ui, sans-serif`;

  snapshot.elements.forEach((element) => {
    const rectX = element.rect.x * scaleX;
    const rectY = element.rect.y * scaleY;
    const rectWidth = element.rect.width * scaleX;
    const rectHeight = element.rect.height * scaleY;
    context.fillStyle = 'rgba(239, 68, 68, 0.08)';
    context.fillRect(rectX, rectY, rectWidth, rectHeight);
    context.lineWidth = Math.max(2, Math.round(Math.min(width, height) * 0.003));
    context.strokeStyle = '#ef4444';
    context.strokeRect(rectX, rectY, rectWidth, rectHeight);

    const text = String(element.id);
    const markerWidth = Math.max(markerHeight, context.measureText(text).width + 12);
    const markerX = Math.max(0, Math.min(width - markerWidth, rectX));
    const markerY = Math.max(0, Math.min(height - markerHeight, rectY - markerHeight - 2));
    drawRoundedRect(context, markerX, markerY, markerWidth, markerHeight, markerHeight / 2);
    context.fillStyle = '#ef4444';
    context.fill();
    context.lineWidth = Math.max(1, Math.round(markerHeight * 0.1));
    context.strokeStyle = '#ffffff';
    context.stroke();
    context.fillStyle = '#ffffff';
    context.fillText(text, markerX + markerWidth / 2, markerY + markerHeight / 2 + 1);
  });

  return canvas.toDataURL('image/png');
}

function formatAnnotationLabel(element: DesktopBrowserAnnotationElement) {
  const label = element.label ? ` "${element.label}"` : '';
  return `#${element.id} ${element.role}${label}`;
}

export function BrowserPage({
  variant = 'page',
}: {
  variant?: 'page' | 'side-panel';
}) {
  const { t } = useI18n();
  const api = getDesktopApi();
  const hostRef = useRef<HTMLDivElement | null>(null);
  const browserInfoButtonRef = useRef<HTMLButtonElement | null>(null);
  const [browserState, setBrowserState] = useState<DesktopBrowserState | null>(null);
  const [addressValue, setAddressValue] = useState('');
  const [annotationMode, setAnnotationMode] = useState(false);
  const [annotationSnapshot, setAnnotationSnapshot] = useState<DesktopBrowserAnnotationSnapshot | null>(null);
  const [selectedAnnotationId, setSelectedAnnotationId] = useState<number | null>(null);
  const [browserStatus, setBrowserStatus] = useState<string | null>(null);
  const active = activeTab(browserState);
  const sidePanel = variant === 'side-panel';

  useEffect(() => {
    let disposed = false;
    const handleState = (nextState: DesktopBrowserState) => {
      if (disposed) {
        return;
      }
      setBrowserState(nextState);
      const nextActive = nextState.tabs.find((tab) => tab.isActive);
      setAddressValue(nextActive?.url ?? '');
    };
    void api.listBrowserState().then(handleState);
    api.subscribeBrowserState(handleState);
    return () => {
      disposed = true;
      api.unsubscribeBrowserState(handleState);
      void api.setBrowserOverlayPaused(false);
      void api.updateBrowserBounds({
        x: 0,
        y: 0,
        width: 0,
        height: 0,
        visible: false,
      });
    };
  }, [api]);

  useLayoutEffect(() => {
    const node = hostRef.current;
    if (!node) {
      return;
    }

    const syncBounds = () => {
      const rect = node.getBoundingClientRect();
      void api.updateBrowserBounds({
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        visible: rect.width > 0 && rect.height > 0,
      });
    };

    syncBounds();
    const observer = new ResizeObserver(syncBounds);
    observer.observe(node);
    window.addEventListener('resize', syncBounds);
    window.addEventListener('scroll', syncBounds, true);
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', syncBounds);
      window.removeEventListener('scroll', syncBounds, true);
    };
  }, [api, browserState?.activeTabId]);

  async function submitAddress(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!active || !addressValue.trim()) {
      return;
    }
    await api.navigateBrowserTab({
      tabId: active.id,
      url: addressValue,
    });
  }

  function showBrowserConnectionMenu() {
    const button = browserInfoButtonRef.current;
    if (!button) {
      return;
    }
    const rect = button.getBoundingClientRect();
    const viewportPadding = 8;
    const menuWidth = 252;
    const nextX = Math.max(
      viewportPadding,
      Math.min(rect.right - menuWidth, window.innerWidth - menuWidth - viewportPadding),
    );
    void api.showBrowserConnectionMenu({
      x: nextX,
      y: rect.bottom + 6,
      labels: {
        copyCdpEndpoint: t('Copy CDP Endpoint'),
        copyCdpListUrl: t('Copy CDP List URL'),
      },
    });
  }

  async function toggleAnnotationMode() {
    if (annotationMode) {
      setAnnotationMode(false);
      setAnnotationSnapshot(null);
      setSelectedAnnotationId(null);
      setBrowserStatus(null);
      await api.setBrowserOverlayPaused(false);
      return;
    }
    if (!active) {
      return;
    }
    const snapshot = await api.captureBrowserAnnotations({
      tabId: active.id,
      copyToClipboard: false,
    });
    setAnnotationSnapshot(snapshot);
    setSelectedAnnotationId(null);
    setAnnotationMode(true);
    setBrowserStatus(
      snapshot.elements.length
        ? t('Annotated {count} elements.', { count: String(snapshot.elements.length) })
        : t('No visible interactive elements found.'),
    );
    await api.setBrowserOverlayPaused(true);
  }

  async function copyCurrentScreenshot() {
    if (!active) {
      return;
    }
    try {
      if (annotationMode && annotationSnapshot) {
        const dataUrl = await renderAnnotatedSnapshot(annotationSnapshot);
        await api.copyImageToClipboard({ dataUrl });
        setBrowserStatus(t('Annotated screenshot copied.'));
        return;
      }
      await api.captureBrowserTab({
        tabId: active.id,
        copyToClipboard: true,
      });
      setBrowserStatus(t('Screenshot copied.'));
    } catch {
      setBrowserStatus(
        annotationMode
          ? t('Failed to copy annotated screenshot.')
          : t('Failed to copy screenshot.'),
      );
    }
  }

  function selectAnnotationElement(element: DesktopBrowserAnnotationElement) {
    setSelectedAnnotationId(element.id);
    setBrowserStatus(formatAnnotationLabel(element));
  }

  return (
    <div className={`browser-page ${sidePanel ? 'browser-page-side-panel' : ''}`}>
      <div className="browser-toolbar">
        {!sidePanel ? (
          <div className="browser-tab-strip">
          <div className="browser-tab-track" role="tablist">
            {(browserState?.tabs ?? []).map((tab) => (
              <div
                key={tab.id}
                className={`browser-tab ${tab.isActive ? 'active' : ''}`}
                role="tab"
              >
                <button
                  className="browser-tab-activate"
                  onClick={() => {
                    void api.activateBrowserTab(tab.id);
                  }}
                  type="button"
                >
                  <span className="browser-tab-title">{tab.title || tab.url || t('New Tab')}</span>
                  {tab.isLoading && <span aria-label={t('Loading')} className="browser-tab-spinner" />}
                </button>
                <button
                  className="codex-icon-button browser-tab-close"
                  onClick={(event) => {
                    event.stopPropagation();
                    void api.closeBrowserTab(tab.id);
                  }}
                  type="button"
                >
                  <BrowserCloseTabIcon />
                </button>
              </div>
            ))}
            <button
              aria-label={t('New Tab')}
              className="codex-icon-button browser-tab-add"
              onClick={() => {
                void api.createBrowserTab();
              }}
              type="button"
            >
              <NewTabIcon />
            </button>
          </div>
          <div className="workspace-more-menu-shell browser-menu-shell">
            <button
              aria-haspopup="menu"
              className="codex-icon-button browser-toolbar-icon browser-info-button"
              onClick={() => {
                showBrowserConnectionMenu();
              }}
              ref={browserInfoButtonRef}
              type="button"
            >
              <InfoIcon />
            </button>
          </div>
        </div>
        ) : null}

        <div className="browser-toolbar-main">
          <button
            className="codex-icon-button browser-toolbar-icon"
            disabled={!active?.canGoBack}
            onClick={() => {
              if (active) {
                void api.browserGoBack(active.id);
              }
            }}
            type="button"
          >
            <BrowserBackIcon />
          </button>
          <button
            className="codex-icon-button browser-toolbar-icon"
            disabled={!active?.canGoForward}
            onClick={() => {
              if (active) {
                void api.browserGoForward(active.id);
              }
            }}
            type="button"
          >
            <BrowserForwardIcon />
          </button>
          <button
            className="codex-icon-button browser-toolbar-icon"
            disabled={!active}
            onClick={() => {
              if (active) {
                void api.browserReload(active.id);
              }
            }}
            type="button"
          >
            <BrowserRefreshIcon />
          </button>
          <form className="browser-address-form" onSubmit={(event) => { void submitAddress(event); }}>
            <span className="browser-address-icon">
              {active && isHttpsUrl(active.url) ? <LockIcon /> : <BrowserIcon />}
            </span>
            <Input
              className="browser-address-input"
              onChange={(event) => {
                setAddressValue(event.target.value);
              }}
              onFocus={(event) => {
                event.target.select();
              }}
              placeholder={sidePanel ? t('Input URL') : t('Search or enter address')}
              value={addressValue}
            />
          </form>
          <button
            className="codex-icon-button browser-toolbar-icon"
            disabled={!active}
            onClick={() => {
              if (active) {
                void api.browserOpenExternal(active.id);
              }
            }}
            title={t('Open in Browser')}
            type="button"
          >
            <ExternalLinkIcon />
          </button>
          {sidePanel ? (
            <>
              <button
                className="codex-icon-button browser-toolbar-icon"
                disabled={!active}
                onClick={() => {
                  void copyCurrentScreenshot();
                }}
                title={annotationMode ? t('Copy annotated screenshot') : t('Copy Screenshot')}
                type="button"
              >
                <Camera aria-hidden />
              </button>
              <button
                aria-pressed={annotationMode}
                className={`codex-icon-button browser-toolbar-icon ${annotationMode ? 'is-active' : ''}`}
                disabled={!active}
                onClick={() => {
                  void toggleAnnotationMode();
                }}
                title={t('Annotate')}
                type="button"
              >
                <MousePointer2 aria-hidden />
              </button>
              <button
                aria-haspopup="menu"
                className="codex-icon-button browser-toolbar-icon browser-info-button"
                onClick={() => {
                  showBrowserConnectionMenu();
                }}
                ref={browserInfoButtonRef}
                type="button"
              >
                <InfoIcon />
              </button>
            </>
          ) : null}
        </div>
      </div>

      <div className={`browser-status-line ${browserStatus ? '' : 'is-empty'}`}>
        {browserStatus}
      </div>

      <div className="browser-stage">
        <div className="browser-stage-shell">
          <div
            className={`browser-stage-host ${annotationMode ? 'browser-stage-host-annotating' : ''}`}
            ref={hostRef}
          />
          {annotationMode && annotationSnapshot ? (
            <div
              aria-label={t('Annotated browser snapshot')}
              className="browser-annotation-layer"
              role="group"
            >
              <img
                alt=""
                className="browser-annotation-image"
                draggable={false}
                src={annotationSnapshot.dataUrl}
              />
              {annotationSnapshot.elements.map((element) => (
                <button
                  aria-label={formatAnnotationLabel(element)}
                  className={`browser-annotation-target ${selectedAnnotationId === element.id ? 'is-selected' : ''} ${element.rect.y < 28 ? 'is-near-top' : ''}`}
                  key={element.id}
                  onClick={() => selectAnnotationElement(element)}
                  style={{
                    height: `${(element.rect.height / Math.max(1, annotationSnapshot.viewportHeight)) * 100}%`,
                    left: `${(element.rect.x / Math.max(1, annotationSnapshot.viewportWidth)) * 100}%`,
                    top: `${(element.rect.y / Math.max(1, annotationSnapshot.viewportHeight)) * 100}%`,
                    width: `${(element.rect.width / Math.max(1, annotationSnapshot.viewportWidth)) * 100}%`,
                  }}
                  type="button"
                >
                  <span className="browser-annotation-marker">{element.id}</span>
                </button>
              ))}
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}
