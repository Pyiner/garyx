import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type FormEvent } from 'react';
import { createPortal } from 'react-dom';

import type { DesktopBrowserState } from '@shared/contracts';

import { getDesktopApi } from './platform/desktop-api';
import { BrowserBackIcon, BrowserForwardIcon, BrowserRefreshIcon, BrowserCloseTabIcon, ExternalLinkIcon, NewTabIcon, BrowserIcon, LockIcon, InfoIcon } from './app-shell/icons';
import { useI18n } from './i18n';

function isHttpsUrl(url: string) {
  return url.startsWith('https://');
}

function activeTab(state: DesktopBrowserState | null) {
  return state?.tabs.find((tab) => tab.isActive) ?? null;
}

export function BrowserPage() {
  const { t } = useI18n();
  const api = getDesktopApi();
  const hostRef = useRef<HTMLDivElement | null>(null);
  const browserInfoButtonRef = useRef<HTMLButtonElement | null>(null);
  const [connectionMenuOpen, setConnectionMenuOpen] = useState(false);
  const [connectionMenuStyle, setConnectionMenuStyle] = useState<CSSProperties | null>(null);
  const [browserState, setBrowserState] = useState<DesktopBrowserState | null>(null);
  const [addressValue, setAddressValue] = useState('');
  const active = activeTab(browserState);

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

  async function copyBrowserConnectionValue(value: string) {
    try {
      await navigator.clipboard.writeText(value);
    } catch {
      // Ignore clipboard failures in constrained environments.
    } finally {
      setConnectionMenuOpen(false);
    }
  }

  useEffect(() => {
    if (!connectionMenuOpen) {
      setConnectionMenuStyle(null);
      return;
    }

    const updatePosition = () => {
      const button = browserInfoButtonRef.current;
      if (!button) {
        return;
      }
      const rect = button.getBoundingClientRect();
      const viewportPadding = 12;
      const menuWidth = 252;
      const gap = 6;
      const nextLeft = Math.max(
        viewportPadding,
        Math.min(rect.right - menuWidth, window.innerWidth - menuWidth - viewportPadding),
      );
      const nextTop = Math.max(viewportPadding, rect.bottom + gap);
      setConnectionMenuStyle({
        left: `${nextLeft}px`,
        top: `${nextTop}px`,
      });
    };

    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Element)) {
        return;
      }
      if (target.closest('.browser-connection-popover') || target.closest('.browser-info-button')) {
        return;
      }
      setConnectionMenuOpen(false);
    };

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setConnectionMenuOpen(false);
      }
    };

    updatePosition();
    window.addEventListener('resize', updatePosition);
    window.addEventListener('scroll', updatePosition, true);
    window.addEventListener('pointerdown', handlePointerDown);
    window.addEventListener('keydown', handleKeyDown);
    return () => {
      window.removeEventListener('resize', updatePosition);
      window.removeEventListener('scroll', updatePosition, true);
      window.removeEventListener('pointerdown', handlePointerDown);
      window.removeEventListener('keydown', handleKeyDown);
    };
  }, [connectionMenuOpen]);

  return (
    <div className="browser-page">
      <div className="browser-toolbar">
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
              aria-expanded={connectionMenuOpen}
              aria-haspopup="menu"
              className="codex-icon-button browser-toolbar-icon browser-info-button"
              onClick={() => {
                setConnectionMenuOpen((current) => !current);
              }}
              ref={browserInfoButtonRef}
              type="button"
            >
              <InfoIcon />
            </button>
          </div>
        </div>

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
            <input
              className="browser-address-input"
              onChange={(event) => {
                setAddressValue(event.target.value);
              }}
              onFocus={(event) => {
                event.target.select();
              }}
              placeholder={t('Search or enter address')}
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
        </div>
      </div>

      <div className="browser-stage">
        <div className="browser-stage-shell">
          <div className="browser-stage-host" ref={hostRef} />
        </div>
      </div>
      {connectionMenuOpen && connectionMenuStyle && browserState && typeof document !== 'undefined'
        ? createPortal(
          <div
            className="browser-connection-popover"
            style={{
              position: 'fixed',
              ...connectionMenuStyle,
              zIndex: 2000,
            }}
          >
            <div className="browser-connection-menu" role="menu" aria-label={t('Browser connection details')}>
              <div className="browser-connection-info">
                <span className="browser-connection-label">CDP</span>
                <code>{browserState.debugEndpoint.origin}</code>
              </div>
              <div className="browser-connection-info">
                <span className="browser-connection-label">PROFILE</span>
                <code>{browserState.partition}</code>
              </div>
              <div className="browser-connection-actions">
                <button
                  className="browser-connection-action"
                  onClick={() => {
                    void copyBrowserConnectionValue(browserState.debugEndpoint.origin);
                  }}
                  role="menuitem"
                  type="button"
                >
                  {t('Copy CDP Endpoint')}
                </button>
                <button
                  className="browser-connection-action"
                  onClick={() => {
                    void copyBrowserConnectionValue(browserState.debugEndpoint.listUrl);
                  }}
                  role="menuitem"
                  type="button"
                >
                  {t('Copy CDP List URL')}
                </button>
              </div>
            </div>
          </div>,
          document.body,
        )
        : null}
    </div>
  );
}
