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
  BrowserBoundsInput,
  DesktopBrowserAnnotationElement,
  DesktopBrowserAnnotationSnapshot,
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
const MAX_BROWSER_ANNOTATION_ELEMENTS = 140;

const BROWSER_ANNOTATION_SCRIPT = `(() => {
  const MAX_ELEMENTS = ${MAX_BROWSER_ANNOTATION_ELEMENTS};
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

  const cssEscape = (value) => {
    if (window.CSS && typeof window.CSS.escape === 'function') {
      return window.CSS.escape(String(value));
    }
    return String(value).replace(/[^a-zA-Z0-9_-]/g, (char) => {
      const code = char.charCodeAt(0).toString(16);
      return '\\\\' + code + ' ';
    });
  };
  const cssString = (value) => String(value).replace(/\\\\/g, '\\\\\\\\').replace(/"/g, '\\\\"');
  const normalize = (value, max = 96) => String(value || '').replace(/\\s+/g, ' ').trim().slice(0, max);
  const attr = (element, name) => normalize(element.getAttribute(name));
  const isDisabled = (element) => {
    if (element.getAttribute('aria-disabled') === 'true') {
      return true;
    }
    return 'disabled' in element && Boolean(element.disabled);
  };
  const inferRole = (element) => {
    const explicit = attr(element, 'role');
    if (explicit) {
      return explicit;
    }
    const tag = element.tagName.toLowerCase();
    if (tag === 'a' || tag === 'area') {
      return 'link';
    }
    if (tag === 'button' || tag === 'summary') {
      return 'button';
    }
    if (tag === 'select') {
      return 'combobox';
    }
    if (tag === 'textarea') {
      return 'textbox';
    }
    if (tag === 'label') {
      return 'label';
    }
    if (element.isContentEditable) {
      return 'textbox';
    }
    if (tag === 'input') {
      const type = String(element.getAttribute('type') || 'text').toLowerCase();
      if (type === 'button' || type === 'reset' || type === 'submit') {
        return 'button';
      }
      if (type === 'checkbox') {
        return 'checkbox';
      }
      if (type === 'radio') {
        return 'radio';
      }
      if (type === 'range') {
        return 'slider';
      }
      if (type === 'search') {
        return 'searchbox';
      }
      return 'textbox';
    }
    return tag;
  };
  const labelledByText = (element) => {
    const ids = attr(element, 'aria-labelledby');
    if (!ids) {
      return '';
    }
    return normalize(ids.split(/\\s+/).map((id) => {
      const target = document.getElementById(id);
      return target ? target.innerText || target.textContent || '' : '';
    }).join(' '));
  };
  const elementText = (element) => {
    if (element instanceof HTMLInputElement) {
      const type = String(element.type || 'text').toLowerCase();
      if (type === 'button' || type === 'submit' || type === 'reset') {
        return normalize(element.value);
      }
    }
    return normalize(element.innerText || element.textContent || '');
  };
  const labelFor = (element) => {
    return attr(element, 'aria-label') ||
      labelledByText(element) ||
      attr(element, 'alt') ||
      attr(element, 'title') ||
      attr(element, 'placeholder') ||
      attr(element, 'value') ||
      elementText(element) ||
      inferRole(element);
  };
  const isUniqueSelector = (selector) => {
    try {
      return document.querySelectorAll(selector).length === 1;
    } catch {
      return false;
    }
  };
  const selectorFor = (element) => {
    const tag = element.tagName.toLowerCase();
    if (element.id) {
      const selector = '#' + cssEscape(element.id);
      if (isUniqueSelector(selector)) {
        return selector;
      }
    }
    for (const name of ['data-testid', 'data-test', 'data-cy', 'name', 'aria-label', 'title', 'placeholder']) {
      const value = element.getAttribute(name);
      if (!value) {
        continue;
      }
      const selector = tag + '[' + name + '="' + cssString(value) + '"]';
      if (isUniqueSelector(selector)) {
        return selector;
      }
    }
    const parts = [];
    let current = element;
    while (current && current.nodeType === Node.ELEMENT_NODE && current !== document.documentElement) {
      const currentTag = current.tagName.toLowerCase();
      if (current.id) {
        parts.unshift('#' + cssEscape(current.id));
        break;
      }
      let index = 1;
      let previous = current.previousElementSibling;
      while (previous) {
        if (previous.tagName === current.tagName) {
          index += 1;
        }
        previous = previous.previousElementSibling;
      }
      parts.unshift(currentTag + ':nth-of-type(' + index + ')');
      current = current.parentElement;
    }
    return parts.join(' > ');
  };
  const visibleRect = (element) => {
    const style = window.getComputedStyle(element);
    if (style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity) === 0 || style.pointerEvents === 'none') {
      return null;
    }
    const rects = Array.from(element.getClientRects());
    for (const rect of rects) {
      const left = Math.max(0, rect.left);
      const top = Math.max(0, rect.top);
      const right = Math.min(window.innerWidth, rect.right);
      const bottom = Math.min(window.innerHeight, rect.bottom);
      const width = right - left;
      const height = bottom - top;
      if (width >= 4 && height >= 4) {
        return { x: left, y: top, width, height };
      }
    }
    return null;
  };
  const hitTestVisible = (element, rect) => {
    const points = [
      [rect.x + rect.width / 2, rect.y + rect.height / 2],
      [rect.x + Math.min(rect.width - 1, 4), rect.y + Math.min(rect.height - 1, 4)],
      [rect.x + Math.max(1, rect.width - 4), rect.y + Math.max(1, rect.height - 4)],
    ];
    return points.some(([x, y]) => {
      if (x < 0 || y < 0 || x >= window.innerWidth || y >= window.innerHeight) {
        return false;
      }
      const hit = document.elementFromPoint(x, y);
      return Boolean(hit && (hit === element || element.contains(hit) || hit.contains(element)));
    });
  };
  const candidateElements = Array.from(document.querySelectorAll(INTERACTIVE_SELECTOR));
  const seenRects = new Set();
  const elements = [];
  for (const element of candidateElements) {
    if (!(element instanceof Element) || isDisabled(element)) {
      continue;
    }
    const rect = visibleRect(element);
    if (!rect || !hitTestVisible(element, rect)) {
      continue;
    }
    const rectKey = [
      Math.round(rect.x),
      Math.round(rect.y),
      Math.round(rect.width),
      Math.round(rect.height),
    ].join(':');
    if (seenRects.has(rectKey)) {
      continue;
    }
    seenRects.add(rectKey);
    elements.push({
      label: labelFor(element),
      rect,
      role: inferRole(element),
      selector: selectorFor(element),
      tagName: element.tagName.toLowerCase(),
    });
    if (elements.length >= MAX_ELEMENTS) {
      break;
    }
  }
  return {
    elements: elements.map((element, index) => ({
      ...element,
      id: index + 1,
      nodeId: String(index + 1),
    })),
    scrollX: window.scrollX,
    scrollY: window.scrollY,
    url: window.location.href,
    viewportHeight: window.innerHeight,
    viewportWidth: window.innerWidth,
  };
})()`;

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

type BrowserAnnotationScriptResult = {
  elements?: unknown[];
  scrollX?: unknown;
  scrollY?: unknown;
  url?: unknown;
  viewportHeight?: unknown;
  viewportWidth?: unknown;
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

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function finiteNumber(value: unknown, fallback = 0): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback;
}

function safeString(value: unknown, fallback = ''): string {
  return typeof value === 'string' ? value : fallback;
}

function sanitizeAnnotationElements(value: unknown): DesktopBrowserAnnotationElement[] {
  if (!Array.isArray(value)) {
    return [];
  }
  const elements: DesktopBrowserAnnotationElement[] = [];
  for (const item of value.slice(0, MAX_BROWSER_ANNOTATION_ELEMENTS)) {
    const record = asRecord(item);
    const rect = asRecord(record?.rect);
    if (!record || !rect) {
      continue;
    }
    const x = finiteNumber(rect.x);
    const y = finiteNumber(rect.y);
    const width = finiteNumber(rect.width);
    const height = finiteNumber(rect.height);
    if (width <= 0 || height <= 0) {
      continue;
    }
    elements.push({
      id: elements.length + 1,
      nodeId: safeString(record.nodeId, String(elements.length + 1)),
      tagName: safeString(record.tagName, 'element'),
      role: safeString(record.role, 'element'),
      label: safeString(record.label, ''),
      selector: safeString(record.selector, ''),
      rect: {
        x,
        y,
        width,
        height,
      },
    });
  }
  return elements;
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

  async captureAnnotations(input: CaptureBrowserTabInput): Promise<DesktopBrowserAnnotationSnapshot> {
    const record = this.requireTab(input.tabId);
    const annotationResult = asRecord(
      await record.view.webContents
        .executeJavaScript(BROWSER_ANNOTATION_SCRIPT, true)
        .catch(() => null),
    ) as BrowserAnnotationScriptResult | null;
    const image = await record.view.webContents.capturePage();
    const size = image.getSize();
    const viewportWidth = finiteNumber(annotationResult?.viewportWidth, size.width);
    const viewportHeight = finiteNumber(annotationResult?.viewportHeight, size.height);
    return {
      dataUrl: image.toDataURL(),
      elements: sanitizeAnnotationElements(annotationResult?.elements),
      height: size.height,
      mediaType: 'image/png',
      scrollX: finiteNumber(annotationResult?.scrollX),
      scrollY: finiteNumber(annotationResult?.scrollY),
      title: safeTitle(record.title || record.view.webContents.getTitle()),
      url: safeString(annotationResult?.url, record.view.webContents.getURL() || record.url),
      viewportHeight,
      viewportWidth,
      width: size.width,
    };
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

export async function captureBrowserAnnotations(
  _event: IpcMainInvokeEvent,
  input: CaptureBrowserTabInput,
): Promise<DesktopBrowserAnnotationSnapshot> {
  return browserRuntime.captureAnnotations(input);
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
