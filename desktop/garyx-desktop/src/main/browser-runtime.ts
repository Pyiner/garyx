import { randomUUID } from 'node:crypto';

import {
  WebContentsView,
  app,
  clipboard,
  nativeImage,
  shell,
  type BrowserWindow,
  type IpcMainEvent,
  type IpcMainInvokeEvent,
  type Rectangle,
  type WebContents,
} from 'electron';

import type {
  BrowserAnnotationCommentRequest,
  BrowserAnnotationModeInput,
  BrowserBoundsInput,
  CaptureBrowserTabInput,
  CaptureBrowserTabResult,
  CopyImageToClipboardInput,
  CreateBrowserTabInput,
  DesktopBrowserDebugEndpoint,
  DesktopBrowserState,
  DesktopBrowserTab,
  NavigateBrowserTabInput,
} from '@shared/contracts';

const BROWSER_PARTITION = 'persist:gary-browser';
const DEFAULT_BROWSER_URL = 'https://www.google.com/';
const DEFAULT_REMOTE_DEBUGGING_PORT = '39222';
const configuredRemoteDebuggingPort =
  process.env.GARYX_DESKTOP_REMOTE_DEBUGGING_PORT?.trim() || DEFAULT_REMOTE_DEBUGGING_PORT;
const disableFixedRemoteDebuggingPort = process.env.GARYX_DESKTOP_DISABLE_FIXED_CDP === '1';

function browserAnnotationModeScript(enabled: boolean, commentMessagePrefix: string): string {
  return `(() => {
    const KEY = '__garyxBrowserAnnotationMode';
    const existing = window[KEY];
    if (existing && typeof existing.dispose === 'function') {
      existing.dispose();
    }
    if (!${JSON.stringify(enabled)}) {
      return { enabled: false };
    }

    const COMMENT_MESSAGE_PREFIX = ${JSON.stringify(commentMessagePrefix)};
    const COMMENT_ICON = 'data:image/svg+xml,%3Csvg%20xmlns%3D%22http://www.w3.org/2000/svg%22%20width%3D%2226%22%20height%3D%2225%22%20viewBox%3D%220%200%2026%2025%22%20fill%3D%22none%22%3E%3Cpath%20d%3D%22M12.65%20.82C6.21%20.82%201%205.48%201%2011.22c0%203.01%201.43%205.72%203.72%207.62l-.92%204.12c-.12.54.46.95.92.65l4.32-2.8c1.13.31%202.35.48%203.61.48%206.43%200%2011.65-4.66%2011.65-10.4S19.08.82%2012.65.82Z%22%20fill%3D%22%230285FF%22%20stroke%3D%22white%22%20stroke-width%3D%221.5%22/%3E%3Ccircle%20cx%3D%228.8%22%20cy%3D%2211.1%22%20r%3D%221.2%22%20fill%3D%22white%22/%3E%3Ccircle%20cx%3D%2212.8%22%20cy%3D%2211.1%22%20r%3D%221.2%22%20fill%3D%22white%22/%3E%3Ccircle%20cx%3D%2216.8%22%20cy%3D%2211.1%22%20r%3D%221.2%22%20fill%3D%22white%22/%3E%3C/svg%3E';
    const COMMENT_CURSOR = 'url("' + COMMENT_ICON + '") 13 12, crosshair';
    const INTERACTIVE_SELECTOR = [
      'a[href]',
      'area[href]',
      'button',
      'input:not([type="hidden"])',
      'select',
      'textarea',
      'summary',
      'label',
      '[contenteditable=""]',
      '[contenteditable="true"]',
      '[role="button"]',
      '[role="checkbox"]',
      '[role="combobox"]',
      '[role="link"]',
      '[role="listbox"]',
      '[role="menuitem"]',
      '[role="menuitemcheckbox"]',
      '[role="menuitemradio"]',
      '[role="option"]',
      '[role="radio"]',
      '[role="searchbox"]',
      '[role="slider"]',
      '[role="spinbutton"]',
      '[role="switch"]',
      '[role="tab"]',
      '[role="textbox"]',
      '[tabindex]:not([tabindex="-1"])',
      '[onclick]',
      '[aria-haspopup]',
    ].join(',');

    const cursorStyle = document.createElement('style');
    cursorStyle.setAttribute('data-garyx-browser-annotation-ui', 'true');
    cursorStyle.setAttribute('data-garyx-browser-annotation-cursor-style', 'true');
    cursorStyle.textContent =
      'html, body, body * { cursor: ' + COMMENT_CURSOR + ' !important; -webkit-user-select: none !important; user-select: none !important; }' +
      '[data-garyx-browser-annotation-comment-button="true"] { cursor: pointer !important; }';
    (document.head || document.documentElement).appendChild(cursorStyle);

    const overlay = document.createElement('div');
    overlay.setAttribute('data-garyx-browser-annotation-hover', 'true');
    overlay.setAttribute('data-garyx-browser-annotation-ui', 'true');
    Object.assign(overlay.style, {
      position: 'fixed',
      left: '0',
      top: '0',
      width: '0',
      height: '0',
      display: 'none',
      boxSizing: 'border-box',
      pointerEvents: 'none',
      zIndex: '2147483647',
      border: '2px solid #2563eb',
      borderRadius: '4px',
      background: 'rgba(37, 99, 235, 0.08)',
      boxShadow: '0 0 0 1px rgba(255,255,255,0.9), 0 0 0 4px rgba(37,99,235,0.16)',
    });
    (document.body || document.documentElement).appendChild(overlay);

    const commentButton = document.createElement('button');
    commentButton.type = 'button';
    commentButton.setAttribute('aria-label', 'Comment on selected element');
    commentButton.setAttribute('title', 'Comment');
    commentButton.setAttribute('data-garyx-browser-annotation-comment-button', 'true');
    commentButton.setAttribute('data-garyx-browser-annotation-ui', 'true');
    Object.assign(commentButton.style, {
      position: 'fixed',
      left: '0',
      top: '0',
      width: '28px',
      height: '28px',
      display: 'none',
      padding: '0',
      border: '0',
      borderRadius: '0',
      background: 'transparent url("' + COMMENT_ICON + '") center / 26px 25px no-repeat',
      boxShadow: 'none',
      outline: 'none',
      pointerEvents: 'auto',
      zIndex: '2147483647',
    });
    (document.body || document.documentElement).appendChild(commentButton);

    const previousCursor = document.documentElement.style.cursor;
    document.documentElement.style.cursor = COMMENT_CURSOR;
    let currentElement = null;
    let selectedElement = null;

    const isDisabled = (element) => {
      if (!(element instanceof Element)) {
        return true;
      }
      if (element.getAttribute('aria-disabled') === 'true') {
        return true;
      }
      return 'disabled' in element && Boolean(element.disabled);
    };

    const visibleRect = (element) => {
      if (!(element instanceof Element) || element === document.documentElement || element === document.body) {
        return null;
      }
      const style = window.getComputedStyle(element);
      if (style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity) === 0) {
        return null;
      }
      for (const rect of Array.from(element.getClientRects())) {
        const left = Math.max(0, rect.left);
        const top = Math.max(0, rect.top);
        const right = Math.min(window.innerWidth, rect.right);
        const bottom = Math.min(window.innerHeight, rect.bottom);
        const width = right - left;
        const height = bottom - top;
        if (width >= 4 && height >= 4) {
          return { left, top, width, height };
        }
      }
      return null;
    };

    const isAnnotationUi = (node) => {
      const element = node instanceof Element ? node : node?.parentElement || null;
      return Boolean(element?.closest('[data-garyx-browser-annotation-ui="true"]'));
    };

    const closestInteractive = (node) => {
      let element = node instanceof Element ? node : node?.parentElement || null;
      if (element?.closest('[data-garyx-browser-annotation-ui="true"]')) {
        return null;
      }
      while (element && element !== document.documentElement) {
        if (element.matches(INTERACTIVE_SELECTOR) && !isDisabled(element)) {
          return element;
        }
        element = element.parentElement;
      }
      return null;
    };

    const hide = () => {
      currentElement = null;
      overlay.style.display = 'none';
    };

    const truncate = (value, maxLength) => {
      const text = String(value || '').replace(/\\s+/g, ' ').trim();
      if (text.length <= maxLength) {
        return text;
      }
      return text.slice(0, maxLength - 1) + '…';
    };

    const escapeSelectorPart = (value) => {
      if (window.CSS && typeof window.CSS.escape === 'function') {
        return window.CSS.escape(value);
      }
      return String(value).replace(/[^a-zA-Z0-9_-]/g, (character) => '\\\\' + character);
    };

    const selectorFor = (element) => {
      if (!(element instanceof Element)) {
        return null;
      }
      if (element.id) {
        return '#' + escapeSelectorPart(element.id);
      }
      const parts = [];
      let node = element;
      while (node && node instanceof Element && node !== document.documentElement && parts.length < 5) {
        let part = node.localName || node.tagName.toLowerCase();
        const classes = Array.from(node.classList || []).filter(Boolean).slice(0, 2);
        if (classes.length) {
          part += '.' + classes.map(escapeSelectorPart).join('.');
        }
        const parent = node.parentElement;
        if (parent) {
          const siblings = Array.from(parent.children).filter((sibling) => sibling.localName === node.localName);
          if (siblings.length > 1) {
            part += ':nth-of-type(' + (siblings.indexOf(node) + 1) + ')';
          }
        }
        parts.unshift(part);
        node = parent;
      }
      return parts.join(' > ') || null;
    };

    const labelFor = (element) => {
      if (!(element instanceof Element)) {
        return '';
      }
      const formValue =
        element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement
          ? element.value || element.placeholder
          : '';
      const imageAlt = element instanceof HTMLImageElement ? element.alt : '';
      return truncate(
        element.getAttribute('aria-label') ||
          element.getAttribute('title') ||
          imageAlt ||
          formValue ||
          element.textContent ||
          element.tagName.toLowerCase(),
        160,
      );
    };

    const emitCommentRequest = () => {
      if (!(selectedElement instanceof Element)) {
        return;
      }
      const rect = visibleRect(selectedElement);
      if (!rect) {
        commentButton.style.display = 'none';
        return;
      }
      const payload = {
        tagName: selectedElement.tagName.toLowerCase(),
        label: labelFor(selectedElement),
        role: selectedElement.getAttribute('role'),
        selector: selectorFor(selectedElement),
        text: truncate(selectedElement.textContent || '', 240) || null,
        rect: {
          x: Math.round(rect.left),
          y: Math.round(rect.top),
          width: Math.round(rect.width),
          height: Math.round(rect.height),
        },
      };
      console.log(COMMENT_MESSAGE_PREFIX + JSON.stringify(payload));
    };

    const positionCommentButton = () => {
      const rect = visibleRect(selectedElement);
      if (!rect) {
        commentButton.style.display = 'none';
        return;
      }
      const size = 28;
      const gap = 6;
      const padding = 8;
      let left = rect.left + rect.width + gap;
      if (left + size > window.innerWidth - padding) {
        left = Math.max(padding, rect.left - size - gap);
      }
      let top = Math.max(padding, rect.top - 10);
      if (top + size > window.innerHeight - padding) {
        top = Math.max(padding, window.innerHeight - size - padding);
      }
      commentButton.style.display = 'block';
      commentButton.style.transform =
        'translate(' + Math.round(left) + 'px, ' + Math.round(top) + 'px)';
    };

    const update = (element) => {
      const rect = visibleRect(element);
      if (!rect) {
        hide();
        return;
      }
      currentElement = element;
      overlay.style.display = 'block';
      overlay.style.transform = 'translate(' + rect.left + 'px, ' + rect.top + 'px)';
      overlay.style.width = rect.width + 'px';
      overlay.style.height = rect.height + 'px';
      if (selectedElement === element) {
        positionCommentButton();
      }
    };

    const handlePointerMove = (event) => {
      if (selectedElement || isAnnotationUi(event.target)) {
        return;
      }
      update(closestInteractive(event.target));
    };
    const handlePointerLeave = () => {
      if (!selectedElement) {
        hide();
      }
    };
    const handleScrollOrResize = () => {
      const element = selectedElement || currentElement;
      if (element) {
        update(element);
      }
    };
    const handleClick = (event) => {
      if (isAnnotationUi(event.target)) {
        return;
      }
      const target = closestInteractive(event.target);
      if (!target) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      selectedElement = target;
      update(target);
      positionCommentButton();
    };
    const handlePointerDown = (event) => {
      if (isAnnotationUi(event.target)) {
        return;
      }
      if (!closestInteractive(event.target)) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
    };
    const handleCommentButtonClick = (event) => {
      event.preventDefault();
      event.stopPropagation();
      emitCommentRequest();
    };
    const stopCommentButtonEvent = (event) => {
      event.preventDefault();
      event.stopPropagation();
    };

    window.addEventListener('pointermove', handlePointerMove, true);
    window.addEventListener('pointerleave', handlePointerLeave, true);
    window.addEventListener('scroll', handleScrollOrResize, true);
    window.addEventListener('resize', handleScrollOrResize, true);
    window.addEventListener('pointerdown', handlePointerDown, true);
    window.addEventListener('mousedown', handlePointerDown, true);
    window.addEventListener('click', handleClick, true);
    commentButton.addEventListener('click', handleCommentButtonClick, true);
    commentButton.addEventListener('pointerdown', stopCommentButtonEvent, true);
    commentButton.addEventListener('mousedown', stopCommentButtonEvent, true);

    window[KEY] = {
      dispose() {
        window.removeEventListener('pointermove', handlePointerMove, true);
        window.removeEventListener('pointerleave', handlePointerLeave, true);
        window.removeEventListener('scroll', handleScrollOrResize, true);
        window.removeEventListener('resize', handleScrollOrResize, true);
        window.removeEventListener('pointerdown', handlePointerDown, true);
        window.removeEventListener('mousedown', handlePointerDown, true);
        window.removeEventListener('click', handleClick, true);
        commentButton.removeEventListener('click', handleCommentButtonClick, true);
        commentButton.removeEventListener('pointerdown', stopCommentButtonEvent, true);
        commentButton.removeEventListener('mousedown', stopCommentButtonEvent, true);
        document.documentElement.style.cursor = previousCursor;
        cursorStyle.remove();
        overlay.remove();
        commentButton.remove();
        delete window[KEY];
      },
    };
    return { enabled: true };
  })()`;
}

if (!disableFixedRemoteDebuggingPort && !app.commandLine.hasSwitch('remote-debugging-port')) {
  app.commandLine.appendSwitch('remote-debugging-port', configuredRemoteDebuggingPort);
}

type BrowserTabRecord = {
  id: string;
  view: WebContentsView;
  title: string;
  url: string;
  isLoading: boolean;
};

function normalizeUrl(value?: string): string {
  const candidate = value?.trim();
  if (!candidate) {
    return DEFAULT_BROWSER_URL;
  }
  if (/^[a-zA-Z][a-zA-Z\d+\-.]*:/.test(candidate)) {
    return candidate;
  }
  return `https://${candidate}`;
}

function safeTitle(value: string): string {
  const trimmed = value.trim();
  return trimmed || 'New Tab';
}

export function getBrowserDebugEndpoint(): DesktopBrowserDebugEndpoint {
  const origin = `http://127.0.0.1:${configuredRemoteDebuggingPort}`;
  return {
    origin,
    versionUrl: `${origin}/json/version`,
    listUrl: `${origin}/json/list`,
    port: Number.parseInt(configuredRemoteDebuggingPort, 10),
  };
}

class BrowserRuntime {
  private readonly tabs = new Map<string, BrowserTabRecord>();

  private readonly subscribers = new Set<WebContents>();

  private readonly annotationSubscribers = new Set<WebContents>();

  private readonly annotationMessagePrefix = `__GARYX_BROWSER_ANNOTATION_COMMENT__${randomUUID()}__`;

  private window: BrowserWindow | null = null;

  private activeTabId: string | null = null;

  private mountedTabId: string | null = null;

  private hostBounds: Rectangle | null = null;

  private hostVisible = false;

  // Renderer-DOM modals (Memory dialog, etc.) live below the
  // WebContentsView in OS-level z-order — no CSS reaches an OS view.
  // When such an overlay opens, the renderer toggles this paused
  // flag; reconcile then unmounts the view so the modal isn't
  // hidden behind it. Bounds stay owned by `BrowserPage`'s layout
  // effect, so toggling back to false re-mounts at the same rect.
  private overlayPaused = false;

  private initialized = false;

  bindWindow(window: BrowserWindow): void {
    this.window = window;
    this.reconcileMountedView();
  }

  detachWindow(window: BrowserWindow): void {
    if (this.window && this.window === window) {
      this.unmountActiveView();
      this.window = null;
    }
  }

  subscribe(event: IpcMainEvent): DesktopBrowserState {
    this.subscribers.add(event.sender);
    event.sender.once('destroyed', () => {
      this.subscribers.delete(event.sender);
    });
    return this.snapshot();
  }

  unsubscribe(event: IpcMainEvent): void {
    this.subscribers.delete(event.sender);
  }

  subscribeAnnotationComments(event: IpcMainEvent): void {
    this.annotationSubscribers.add(event.sender);
    event.sender.once('destroyed', () => {
      this.annotationSubscribers.delete(event.sender);
    });
  }

  unsubscribeAnnotationComments(event: IpcMainEvent): void {
    this.annotationSubscribers.delete(event.sender);
  }

  listState(): DesktopBrowserState {
    this.ensureInitialized();
    return this.snapshot();
  }

  createTab(input?: CreateBrowserTabInput): DesktopBrowserState {
    this.ensureInitialized();
    const record = this.createTabRecord(input?.url);
    this.tabs.set(record.id, record);
    this.activeTabId = record.id;
    this.reconcileMountedView();
    this.emitState();
    return this.snapshot();
  }

  activateTab(tabId: string): DesktopBrowserState {
    this.ensureInitialized();
    if (!this.tabs.has(tabId)) {
      throw new Error(`browser tab not found: ${tabId}`);
    }
    this.activeTabId = tabId;
    this.reconcileMountedView();
    this.emitState();
    return this.snapshot();
  }

  closeTab(tabId: string): DesktopBrowserState {
    this.ensureInitialized();
    const record = this.tabs.get(tabId);
    if (!record) {
      throw new Error(`browser tab not found: ${tabId}`);
    }
    if (this.mountedTabId === tabId) {
      this.unmountActiveView();
    }
    this.tabs.delete(tabId);
    record.view.webContents.removeAllListeners();
    record.view.webContents.close();

    if (this.activeTabId === tabId) {
      const nextTab = this.tabs.values().next().value as BrowserTabRecord | undefined;
      this.activeTabId = nextTab?.id ?? null;
    }
    if (!this.activeTabId) {
      const next = this.createTabRecord();
      this.tabs.set(next.id, next);
      this.activeTabId = next.id;
    }
    this.reconcileMountedView();
    this.emitState();
    return this.snapshot();
  }

  navigate(input: NavigateBrowserTabInput): DesktopBrowserState {
    this.ensureInitialized();
    const record = this.tabs.get(input.tabId);
    if (!record) {
      throw new Error(`browser tab not found: ${input.tabId}`);
    }
    void record.view.webContents.loadURL(normalizeUrl(input.url));
    if (input.tabId !== this.activeTabId) {
      this.activeTabId = input.tabId;
      this.reconcileMountedView();
    }
    this.emitState();
    return this.snapshot();
  }

  async goBack(tabId: string): Promise<DesktopBrowserState> {
    const record = this.requireTab(tabId);
    if (record.view.webContents.navigationHistory.canGoBack()) {
      await record.view.webContents.navigationHistory.goBack();
    }
    this.emitState();
    return this.snapshot();
  }

  async goForward(tabId: string): Promise<DesktopBrowserState> {
    const record = this.requireTab(tabId);
    if (record.view.webContents.navigationHistory.canGoForward()) {
      await record.view.webContents.navigationHistory.goForward();
    }
    this.emitState();
    return this.snapshot();
  }

  async reload(tabId: string): Promise<DesktopBrowserState> {
    const record = this.requireTab(tabId);
    record.view.webContents.reload();
    this.emitState();
    return this.snapshot();
  }

  async openExternal(tabId: string): Promise<void> {
    const record = this.requireTab(tabId);
    const url = record.view.webContents.getURL().trim();
    if (url) {
      await shell.openExternal(url);
    }
  }

  async captureTab(input: CaptureBrowserTabInput): Promise<CaptureBrowserTabResult> {
    const tabId = input.tabId;
    const record = this.requireTab(tabId);
    const image = await record.view.webContents.capturePage();
    if (input.copyToClipboard !== false) {
      clipboard.writeImage(image);
    }
    const size = image.getSize();
    return {
      dataUrl: image.toDataURL(),
      height: size.height,
      mediaType: 'image/png',
      title: safeTitle(record.title || record.view.webContents.getTitle()),
      width: size.width,
    };
  }

  async setAnnotationMode(input: BrowserAnnotationModeInput): Promise<void> {
    const record = this.requireTab(input.tabId);
    await record.view.webContents
      .executeJavaScript(
        browserAnnotationModeScript(Boolean(input.enabled), this.annotationMessagePrefix),
        true,
      )
      .catch(() => null);
  }

  setHostBounds(input: BrowserBoundsInput): void {
    this.ensureInitialized();
    this.hostVisible = input.visible;
    this.hostBounds = input.visible
      ? {
          x: Math.max(0, Math.round(input.x)),
          y: Math.max(0, Math.round(input.y)),
          width: Math.max(0, Math.round(input.width)),
          height: Math.max(0, Math.round(input.height)),
        }
      : null;
    this.reconcileMountedView();
  }

  setOverlayPaused(paused: boolean): void {
    if (this.overlayPaused === paused) {
      return;
    }
    this.overlayPaused = paused;
    this.reconcileMountedView();
  }

  private ensureInitialized(): void {
    if (this.initialized) {
      return;
    }
    this.initialized = true;
    if (!this.tabs.size) {
      const initial = this.createTabRecord();
      this.tabs.set(initial.id, initial);
      this.activeTabId = initial.id;
    }
  }

  private createTabRecord(url?: string): BrowserTabRecord {
    const view = new WebContentsView({
      webPreferences: {
        partition: BROWSER_PARTITION,
        nodeIntegration: false,
        contextIsolation: true,
        sandbox: false,
      },
    });
    const record: BrowserTabRecord = {
      id: `browser-tab-${randomUUID()}`,
      view,
      title: 'New Tab',
      url: '',
      isLoading: true,
    };
    this.attachTabObservers(record);
    void view.webContents.loadURL(normalizeUrl(url));
    return record;
  }

  private attachTabObservers(record: BrowserTabRecord): void {
    const { webContents } = record.view;
    const sync = () => {
      record.title = safeTitle(webContents.getTitle() || webContents.getURL() || record.title);
      record.url = webContents.getURL() || record.url;
      record.isLoading = webContents.isLoading();
      this.emitState();
    };

    webContents.setWindowOpenHandler(({ url }) => {
      this.createTab({ url });
      return { action: 'deny' };
    });

    webContents.on('page-title-updated', sync);
    webContents.on('did-start-loading', sync);
    webContents.on('did-stop-loading', sync);
    webContents.on('did-navigate', sync);
    webContents.on('did-navigate-in-page', sync);
    webContents.on('did-fail-load', sync);
    webContents.on('console-message', (_event, _level, message) => {
      this.handleAnnotationConsoleMessage(record, String(message || ''));
    });
  }

  private handleAnnotationConsoleMessage(record: BrowserTabRecord, message: string): void {
    if (!message.startsWith(this.annotationMessagePrefix)) {
      return;
    }
    const raw = message.slice(this.annotationMessagePrefix.length);
    let payload: unknown;
    try {
      payload = JSON.parse(raw);
    } catch {
      return;
    }
    const request = this.createAnnotationCommentRequest(record, payload);
    if (!request) {
      return;
    }
    this.emitAnnotationCommentRequest(request);
  }

  private createAnnotationCommentRequest(
    record: BrowserTabRecord,
    payload: unknown,
  ): BrowserAnnotationCommentRequest | null {
    if (!payload || typeof payload !== 'object') {
      return null;
    }
    const input = payload as Record<string, unknown>;
    const rect = input.rect;
    if (!rect || typeof rect !== 'object') {
      return null;
    }
    const rectInput = rect as Record<string, unknown>;
    const x = Number(rectInput.x);
    const y = Number(rectInput.y);
    const width = Number(rectInput.width);
    const height = Number(rectInput.height);
    if (![x, y, width, height].every(Number.isFinite) || width <= 0 || height <= 0) {
      return null;
    }
    const stringValue = (value: unknown): string | null => {
      if (typeof value !== 'string') {
        return null;
      }
      const trimmed = value.trim();
      return trimmed || null;
    };
    const tagName = stringValue(input.tagName) || 'element';
    const label = stringValue(input.label) || tagName;
    const webContents = record.view.webContents;
    return {
      id: `browser-comment-${randomUUID()}`,
      tabId: record.id,
      url: webContents.getURL() || record.url,
      title: safeTitle(webContents.getTitle() || record.title || webContents.getURL()),
      tagName,
      label,
      role: stringValue(input.role),
      selector: stringValue(input.selector),
      text: stringValue(input.text),
      rect: {
        x: Math.round(x),
        y: Math.round(y),
        width: Math.round(width),
        height: Math.round(height),
      },
    };
  }

  private requireTab(tabId: string): BrowserTabRecord {
    this.ensureInitialized();
    const record = this.tabs.get(tabId);
    if (!record) {
      throw new Error(`browser tab not found: ${tabId}`);
    }
    return record;
  }

  private snapshot(): DesktopBrowserState {
    const tabs = Array.from(this.tabs.values()).map((record): DesktopBrowserTab => {
      const { webContents } = record.view;
      return {
        id: record.id,
        title: safeTitle(record.title || webContents.getTitle() || webContents.getURL()),
        url: webContents.getURL() || record.url,
        isActive: record.id === this.activeTabId,
        isLoading: webContents.isLoading(),
        canGoBack: webContents.navigationHistory.canGoBack(),
        canGoForward: webContents.navigationHistory.canGoForward(),
      };
    });
    return {
      tabs,
      activeTabId: this.activeTabId,
      debugEndpoint: getBrowserDebugEndpoint(),
      partition: BROWSER_PARTITION,
    };
  }

  private emitState(): void {
    const state = this.snapshot();
    for (const subscriber of Array.from(this.subscribers)) {
      if (subscriber.isDestroyed()) {
        this.subscribers.delete(subscriber);
        continue;
      }
      subscriber.send('garyx:browser-state', state);
    }
  }

  private emitAnnotationCommentRequest(request: BrowserAnnotationCommentRequest): void {
    for (const subscriber of Array.from(this.annotationSubscribers)) {
      if (subscriber.isDestroyed()) {
        this.annotationSubscribers.delete(subscriber);
        continue;
      }
      subscriber.send('garyx:browser-annotation-comment', request);
    }
  }

  private reconcileMountedView(): void {
    const window = this.window;
    if (!window || window.isDestroyed()) {
      return;
    }
    if (this.overlayPaused || !this.hostVisible || !this.hostBounds || !this.activeTabId) {
      this.unmountActiveView();
      return;
    }
    const active = this.tabs.get(this.activeTabId);
    if (!active) {
      this.unmountActiveView();
      return;
    }
    if (this.mountedTabId !== active.id) {
      this.unmountActiveView();
      window.contentView.addChildView(active.view);
      this.mountedTabId = active.id;
    }
    active.view.setBounds(this.hostBounds);
    active.view.setVisible(true);
  }

  private unmountActiveView(): void {
    const window = this.window;
    if (!window || window.isDestroyed() || !this.mountedTabId) {
      this.mountedTabId = null;
      return;
    }
    const mounted = this.tabs.get(this.mountedTabId);
    if (mounted) {
      mounted.view.setVisible(false);
      window.contentView.removeChildView(mounted.view);
    }
    this.mountedTabId = null;
  }
}

const browserRuntime = new BrowserRuntime();

export function bindBrowserWindow(window: BrowserWindow): void {
  browserRuntime.bindWindow(window);
}

export function unbindBrowserWindow(window: BrowserWindow): void {
  browserRuntime.detachWindow(window);
}

export function subscribeBrowserState(event: IpcMainEvent): DesktopBrowserState {
  return browserRuntime.subscribe(event);
}

export function unsubscribeBrowserState(event: IpcMainEvent): void {
  browserRuntime.unsubscribe(event);
}

export function subscribeBrowserAnnotationComments(event: IpcMainEvent): void {
  browserRuntime.subscribeAnnotationComments(event);
}

export function unsubscribeBrowserAnnotationComments(event: IpcMainEvent): void {
  browserRuntime.unsubscribeAnnotationComments(event);
}

export function listBrowserState(): DesktopBrowserState {
  return browserRuntime.listState();
}

export function createBrowserTab(_event: IpcMainInvokeEvent, input?: CreateBrowserTabInput): DesktopBrowserState {
  return browserRuntime.createTab(input);
}

export function activateBrowserTab(_event: IpcMainInvokeEvent, tabId: string): DesktopBrowserState {
  return browserRuntime.activateTab(tabId);
}

export function closeBrowserTab(_event: IpcMainInvokeEvent, tabId: string): DesktopBrowserState {
  return browserRuntime.closeTab(tabId);
}

export function navigateBrowserTab(
  _event: IpcMainInvokeEvent,
  input: NavigateBrowserTabInput,
): DesktopBrowserState {
  return browserRuntime.navigate(input);
}

export async function browserGoBack(_event: IpcMainInvokeEvent, tabId: string): Promise<DesktopBrowserState> {
  return browserRuntime.goBack(tabId);
}

export async function browserGoForward(
  _event: IpcMainInvokeEvent,
  tabId: string,
): Promise<DesktopBrowserState> {
  return browserRuntime.goForward(tabId);
}

export async function browserReload(_event: IpcMainInvokeEvent, tabId: string): Promise<DesktopBrowserState> {
  return browserRuntime.reload(tabId);
}

export async function browserOpenExternal(_event: IpcMainInvokeEvent, tabId: string): Promise<void> {
  return browserRuntime.openExternal(tabId);
}

export async function captureBrowserTab(
  _event: IpcMainInvokeEvent,
  input: string | CaptureBrowserTabInput,
): Promise<CaptureBrowserTabResult> {
  const captureInput =
    typeof input === 'string'
      ? { tabId: input, copyToClipboard: true }
      : {
          ...input,
          copyToClipboard: input.copyToClipboard !== false,
        };
  return browserRuntime.captureTab(captureInput);
}

export async function setBrowserAnnotationMode(
  _event: IpcMainInvokeEvent,
  input: BrowserAnnotationModeInput,
): Promise<void> {
  await browserRuntime.setAnnotationMode(input);
}

export function copyImageToClipboard(
  _event: IpcMainInvokeEvent,
  input: CopyImageToClipboardInput,
): void {
  const image = nativeImage.createFromDataURL(input.dataUrl);
  if (image.isEmpty()) {
    throw new Error('Image is empty.');
  }
  clipboard.writeImage(image);
}

export function updateBrowserBounds(_event: IpcMainInvokeEvent, input: BrowserBoundsInput): void {
  browserRuntime.setHostBounds(input);
}

export function setBrowserOverlayPaused(_event: IpcMainInvokeEvent, paused: boolean): void {
  browserRuntime.setOverlayPaused(paused);
}
