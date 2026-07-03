export interface DesktopBrowserDebugEndpoint {
  origin: string;
  versionUrl: string;
  listUrl: string;
  port: number;
}

export interface DesktopBrowserTab {
  id: string;
  title: string;
  url: string;
  isActive: boolean;
  isLoading: boolean;
  canGoBack: boolean;
  canGoForward: boolean;
}

export interface DesktopBrowserState {
  tabs: DesktopBrowserTab[];
  activeTabId: string | null;
  debugEndpoint: DesktopBrowserDebugEndpoint;
  partition: string;
}

export interface CreateBrowserTabInput {
  url?: string;
}

export interface NavigateBrowserTabInput {
  tabId: string;
  url: string;
}

export interface BrowserBoundsInput {
  x: number;
  y: number;
  width: number;
  height: number;
  visible: boolean;
}

export interface CaptureBrowserTabInput {
  tabId: string;
  copyToClipboard?: boolean;
}

export interface CaptureBrowserTabResult {
  dataUrl: string;
  height: number;
  mediaType: "image/png";
  title: string;
  width: number;
}

export interface BrowserAnnotationModeInput {
  tabId: string;
  enabled: boolean;
}

export interface BrowserAnnotationCommentRequest {
  id: string;
  tabId: string;
  url: string;
  title: string;
  comment: string;
  tagName: string;
  label: string;
  markerNumber?: number | null;
  role?: string | null;
  selector?: string | null;
  text?: string | null;
  rect: {
    x: number;
    y: number;
    width: number;
    height: number;
  };
  screenshot?: CaptureBrowserTabResult | null;
}

export interface CopyImageToClipboardInput {
  dataUrl: string;
}

export interface CopyTextToClipboardInput {
  text: string;
}

export interface ShowBrowserConnectionMenuInput {
  x: number;
  y: number;
  labels?: {
    copyCdpEndpoint?: string;
    copyCdpListUrl?: string;
  };
}

export type DesktopBrowserStateListener = (state: DesktopBrowserState) => void;

export type DesktopBrowserAnnotationCommentListener = (
  request: BrowserAnnotationCommentRequest,
) => void;

export type DesktopBrowserPageMouseDownListener = () => void;

export interface DesktopTerminalSession {
  id: string;
  title: string;
  cwd: string;
  output: string;
  running: boolean;
  createdAt: string;
  updatedAt: string;
  exitCode: number | null;
  exitSignal: string | null;
}

export interface DesktopTerminalState {
  activeSessionId: string | null;
  sessions: DesktopTerminalSession[];
}

export interface CreateTerminalSessionInput {
  cwd?: string | null;
  title?: string | null;
  cols?: number | null;
  rows?: number | null;
}

export interface TerminalSessionInput {
  sessionId: string;
}

export interface TerminalWriteInput extends TerminalSessionInput {
  data: string;
}

export interface TerminalResizeInput extends TerminalSessionInput {
  cols: number;
  rows: number;
}

export type DesktopTerminalEvent =
  | {
      type: "state";
      state: DesktopTerminalState;
    }
  | {
      type: "output";
      sessionId: string;
      data: string;
    };

export type DesktopTerminalEventListener = (event: DesktopTerminalEvent) => void;
