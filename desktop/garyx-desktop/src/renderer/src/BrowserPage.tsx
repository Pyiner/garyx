import { useEffect, useLayoutEffect, useRef, useState, type FormEvent } from 'react';

import type {
  BrowserAnnotationCommentRequest,
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

export function BrowserPage({
  onAnnotationCommentRequest,
  variant = 'page',
}: {
  onAnnotationCommentRequest?: (request: BrowserAnnotationCommentRequest) => void;
  variant?: 'page' | 'side-panel';
}) {
  const { t } = useI18n();
  const api = getDesktopApi();
  const hostRef = useRef<HTMLDivElement | null>(null);
  const browserInfoButtonRef = useRef<HTMLButtonElement | null>(null);
  const [browserState, setBrowserState] = useState<DesktopBrowserState | null>(null);
  const [addressValue, setAddressValue] = useState('');
  const [annotationMode, setAnnotationMode] = useState(false);
  const [browserStatus, setBrowserStatus] = useState<string | null>(null);
  const active = activeTab(browserState);
  const sidePanel = variant === 'side-panel';
  const hasAnnotationCommentRequest = Boolean(onAnnotationCommentRequest);
  const annotationCommentRequestRef = useRef(onAnnotationCommentRequest);

  useEffect(() => {
    annotationCommentRequestRef.current = onAnnotationCommentRequest;
  }, [onAnnotationCommentRequest]);

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
      void api.updateBrowserBounds({
        x: 0,
        y: 0,
        width: 0,
        height: 0,
        visible: false,
      });
    };
  }, [api]);

  useEffect(() => {
    if (!annotationMode || !active?.id) {
      return;
    }
    void api.setBrowserOverlayPaused(false);
    void api.setBrowserAnnotationMode({
      tabId: active.id,
      enabled: true,
    });
    return () => {
      void api.setBrowserAnnotationMode({
        tabId: active.id,
        enabled: false,
      });
    };
  }, [active?.id, active?.url, annotationMode, api]);

  useEffect(() => {
    if (!hasAnnotationCommentRequest) {
      return;
    }
    const handleAnnotationComment = (request: BrowserAnnotationCommentRequest) => {
      setAnnotationMode(false);
      setBrowserStatus(t('Comment target selected.'));
      annotationCommentRequestRef.current?.(request);
    };
    api.subscribeBrowserAnnotationComments(handleAnnotationComment);
    return () => {
      api.unsubscribeBrowserAnnotationComments(handleAnnotationComment);
    };
  }, [api, hasAnnotationCommentRequest, t]);

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
      setBrowserStatus(null);
      return;
    }
    if (!active) {
      return;
    }
    setAnnotationMode(true);
    setBrowserStatus(t('Hover an element, click it, then use the comment marker.'));
  }

  async function copyCurrentScreenshot() {
    if (!active) {
      return;
    }
    try {
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
            aria-label={t('Open in external browser')}
            onClick={() => {
              if (active) {
                void api.browserOpenExternal(active.id);
              }
            }}
            title={t('Open in external browser')}
            type="button"
          >
            <ExternalLinkIcon />
          </button>
          {sidePanel ? (
            <>
              <button
                aria-label={t('Screenshot')}
                className="codex-icon-button browser-toolbar-icon"
                disabled={!active}
                onClick={() => {
                  void copyCurrentScreenshot();
                }}
                title={t('Screenshot')}
                type="button"
              >
                <Camera aria-hidden />
              </button>
              <button
                aria-label={t('Annotate')}
                aria-pressed={annotationMode}
                className={`codex-icon-button browser-toolbar-icon ${annotationMode ? 'is-active' : ''}`}
                disabled={!active}
                onClick={() => {
                  void toggleAnnotationMode();
                }}
                type="button"
              >
                <MousePointer2 aria-hidden />
                {annotationMode ? <span className="browser-annotation-button-label">{t('Annotating')}</span> : null}
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

      {!sidePanel ? (
        <div className={`browser-status-line ${browserStatus ? '' : 'is-empty'}`}>
          {browserStatus}
        </div>
      ) : null}

      <div className="browser-stage">
        <div className="browser-stage-shell">
          <div
            className={`browser-stage-host ${annotationMode ? 'browser-stage-host-annotating' : ''}`}
            ref={hostRef}
          />
        </div>
      </div>
    </div>
  );
}
