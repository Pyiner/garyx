import { randomUUID } from 'node:crypto';

import {
  WebContentsView,
  app,
  shell,
  type BrowserWindow,
  type IpcMainEvent,
  type IpcMainInvokeEvent,
  type Rectangle,
  type WebContents,
} from 'electron';

import type {
  BrowserBoundsInput,
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
    this.ensureInitialized();
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

export function updateBrowserBounds(_event: IpcMainInvokeEvent, input: BrowserBoundsInput): void {
  browserRuntime.setHostBounds(input);
}

export function setBrowserOverlayPaused(_event: IpcMainInvokeEvent, paused: boolean): void {
  browserRuntime.setOverlayPaused(paused);
}
