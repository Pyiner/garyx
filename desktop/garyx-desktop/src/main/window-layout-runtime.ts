import { BrowserWindow, ipcMain, screen, type Rectangle } from "electron";

import type {
  HorizontalLayoutPolicyName,
  WindowLayoutCommand,
  WindowLayoutSnapshotOrigin,
} from "@shared/contracts";

import {
  WindowLayoutExecutor,
  type WindowLayoutPhysicalEnvironment,
} from "./window-layout-executor";

const WINDOW_LAYOUT_SNAPSHOT_CHANNEL = "garyx:window-layout-snapshot";
const runtimes = new Map<number, WindowLayoutRuntime>();
let ipcRegistered = false;

function configuredAckDelayMs(): number {
  const parsed = Number.parseInt(
    process.env.GARYX_DESKTOP_LAYOUT_ACK_DELAY_MS || "0",
    10,
  );
  return Number.isFinite(parsed) ? Math.max(0, Math.min(10_000, parsed)) : 0;
}

function rectangle(bounds: Rectangle): Rectangle {
  return {
    x: bounds.x,
    y: bounds.y,
    width: bounds.width,
    height: bounds.height,
  };
}

class WindowLayoutRuntime {
  readonly #window: BrowserWindow;
  readonly #executor: WindowLayoutExecutor;
  #nativeInputSession = false;
  #panelMutationDepth = 0;
  #nativeInputReset: ReturnType<typeof setTimeout> | null = null;
  readonly #disposers: Array<() => void> = [];

  constructor(
    window: BrowserWindow,
    policy: HorizontalLayoutPolicyName,
  ) {
    this.#window = window;
    this.#executor = new WindowLayoutExecutor({
      policy,
      ackDelayMs: configuredAckDelayMs(),
      host: {
        windowId: window.id,
        readEnvironment: () => this.#readEnvironment(),
        setBounds: (bounds) => {
          this.#panelMutationDepth += 1;
          try {
            window.setBounds(rectangle(bounds));
          } finally {
            this.#panelMutationDepth -= 1;
          }
        },
      },
      onSnapshot: (update) => {
        if (!window.isDestroyed()) {
          window.webContents.send(WINDOW_LAYOUT_SNAPSHOT_CHANNEL, update);
        }
      },
    });
    this.#bindEnvironmentEvents();
  }

  bootstrap(rendererEpoch: string, senderWindowId: number | null) {
    return this.#executor.bootstrap(rendererEpoch, { senderWindowId });
  }

  execute(command: WindowLayoutCommand, senderWindowId: number | null) {
    return this.#executor.execute(command, { senderWindowId });
  }

  dispose(): void {
    if (this.#nativeInputReset) {
      clearTimeout(this.#nativeInputReset);
      this.#nativeInputReset = null;
    }
    for (const dispose of this.#disposers.splice(0)) {
      dispose();
    }
  }

  #readEnvironment(): WindowLayoutPhysicalEnvironment {
    const bounds = rectangle(this.#window.getBounds());
    const contentBounds = rectangle(this.#window.getContentBounds());
    const normalCandidate = rectangle(this.#window.getNormalBounds());
    const normalBounds =
      normalCandidate.width > 0 && normalCandidate.height > 0
        ? normalCandidate
        : bounds;
    const display = screen.getDisplayMatching(bounds);
    return {
      bounds,
      contentBounds,
      normalBounds,
      workArea: rectangle(display.workArea),
      mode: this.#window.isFullScreen()
        ? "fullscreen"
        : this.#window.isMaximized()
          ? "maximized"
          : "normal",
      displayId: String(display.id),
      scaleFactor: display.scaleFactor,
    };
  }

  #startNativeInputSession(): void {
    this.#nativeInputSession = true;
    if (this.#nativeInputReset) {
      clearTimeout(this.#nativeInputReset);
    }
    this.#nativeInputReset = setTimeout(() => {
      this.#nativeInputSession = false;
      this.#nativeInputReset = null;
    }, 750);
  }

  #finishNativeInputSession(): void {
    this.#nativeInputSession = false;
    if (this.#nativeInputReset) {
      clearTimeout(this.#nativeInputReset);
      this.#nativeInputReset = null;
    }
  }

  #publishEnvironment(origin: WindowLayoutSnapshotOrigin): void {
    if (this.#window.isDestroyed() || this.#panelMutationDepth > 0) {
      return;
    }
    this.#executor.syncExternalEnvironment(origin);
  }

  #onWindow(event: string, listener: (...args: never[]) => void): void {
    this.#window.on(event as never, listener as never);
    this.#disposers.push(() => {
      if (!this.#window.isDestroyed()) {
        this.#window.removeListener(event as never, listener as never);
      }
    });
  }

  #onScreen(event: string, listener: (...args: never[]) => void): void {
    screen.on(event as never, listener as never);
    this.#disposers.push(() => {
      screen.removeListener(event as never, listener as never);
    });
  }

  #bindEnvironmentEvents(): void {
    this.#onWindow("will-resize", () => this.#startNativeInputSession());
    this.#onWindow("will-move", () => this.#startNativeInputSession());
    const publishGeometry = () => {
      this.#publishEnvironment(
        this.#nativeInputSession ? "user" : "panel-machine",
      );
    };
    this.#onWindow("resize", publishGeometry);
    this.#onWindow("move", publishGeometry);
    this.#onWindow("resized", () => {
      publishGeometry();
      this.#finishNativeInputSession();
    });
    this.#onWindow("moved", () => {
      publishGeometry();
      this.#finishNativeInputSession();
    });
    for (const event of [
      "maximize",
      "unmaximize",
      "enter-full-screen",
      "leave-full-screen",
    ]) {
      this.#onWindow(event, () => this.#publishEnvironment("mode"));
    }
    const publishDisplay = () => this.#publishEnvironment("display");
    this.#onScreen("display-added", publishDisplay);
    this.#onScreen("display-removed", publishDisplay);
    this.#onScreen("display-metrics-changed", publishDisplay);
  }
}

function senderWindowId(
  event: Electron.IpcMainEvent | Electron.IpcMainInvokeEvent,
): number | null {
  return BrowserWindow.fromWebContents(event.sender)?.id ?? null;
}

function runtimeForSender(
  event: Electron.IpcMainEvent | Electron.IpcMainInvokeEvent,
): WindowLayoutRuntime | null {
  const windowId = senderWindowId(event);
  return windowId === null ? null : runtimes.get(windowId) ?? null;
}

export function registerWindowLayoutIpc(): void {
  if (ipcRegistered) {
    return;
  }
  ipcRegistered = true;
  ipcMain.on("garyx:get-window-layout-bootstrap", (event, input: unknown) => {
    const runtime = runtimeForSender(event);
    const rendererEpoch =
      typeof input === "object" &&
      input !== null &&
      "rendererEpoch" in input &&
      typeof input.rendererEpoch === "string"
        ? input.rendererEpoch
        : "";
    if (!runtime) {
      throw new Error("window layout bootstrap sender is not bound");
    }
    event.returnValue = runtime.bootstrap(rendererEpoch, senderWindowId(event));
  });
  ipcMain.handle(
    "garyx:execute-window-layout-command",
    async (event, command: WindowLayoutCommand) => {
      const runtime = runtimeForSender(event);
      if (!runtime) {
        throw new Error("window layout command sender is not bound");
      }
      return runtime.execute(command, senderWindowId(event));
    },
  );
}

export function bindWindowLayoutRuntime(
  window: BrowserWindow,
  policy: HorizontalLayoutPolicyName,
): void {
  const existing = runtimes.get(window.id);
  existing?.dispose();
  const runtime = new WindowLayoutRuntime(window, policy);
  runtimes.set(window.id, runtime);
  window.once("closed", () => {
    if (runtimes.get(window.id) === runtime) {
      runtimes.delete(window.id);
      runtime.dispose();
    }
  });
}
