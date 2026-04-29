import { app, BrowserWindow, ipcMain } from "electron";
// `electron-updater` is CommonJS and our compiled bundle is ESM
// (`"type": "module"` in package.json), so a named import would
// trip Node's synthetic-export limit. Default-import the CJS
// namespace and destructure from it — the runtime values are the
// same, and the types come through via the `import type` below.
import electronUpdater from "electron-updater";
import type { UpdateInfo } from "electron-updater";
const { autoUpdater } = electronUpdater;

import type { DesktopUpdateStatus } from "@shared/contracts";

// Re-check every 6 hours while the app is running. The initial check fires
// 8 seconds after app ready to avoid competing with gateway startup.
const RECURRING_CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;
const INITIAL_CHECK_DELAY_MS = 8_000;

let lastStatus: DesktopUpdateStatus = { phase: "idle" };
let subscribers = new Set<BrowserWindow>();
let bootstrapped = false;

function broadcast(status: DesktopUpdateStatus): void {
  lastStatus = status;
  for (const window of subscribers) {
    if (!window.isDestroyed()) {
      window.webContents.send("garyx:update-status", status);
    }
  }
}

function toUpdateInfo(info: UpdateInfo): { version: string; releaseNotes?: string; releaseName?: string } {
  return {
    version: info.version,
    releaseNotes: typeof info.releaseNotes === "string" ? info.releaseNotes : undefined,
    releaseName: info.releaseName ?? undefined,
  };
}

function updateErrorMessage(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  if (/app-update\.yml/i.test(message) && /ENOENT|no such file/i.test(message)) {
    return "Update metadata is missing from this app bundle. Rebuild or reinstall Garyx, then try again.";
  }
  return message;
}

export function registerUpdaterIpc(): void {
  ipcMain.handle("garyx:get-update-status", () => lastStatus);
  ipcMain.handle("garyx:install-update", () => {
    if (lastStatus.phase !== "downloaded") {
      return { ok: false, reason: "update-not-downloaded" as const };
    }
    // setImmediate so the IPC response flushes before the app quits.
    setImmediate(() => {
      try {
        autoUpdater.quitAndInstall(false, true);
      } catch (error) {
        console.error("[updater] quitAndInstall failed", error);
      }
    });
    return { ok: true as const };
  });
  ipcMain.handle("garyx:check-for-updates-now", async () => {
    if (!app.isPackaged) {
      return { ok: false, reason: "dev-build" as const };
    }
    try {
      await autoUpdater.checkForUpdates();
      return { ok: true as const };
    } catch (error) {
      return { ok: false as const, reason: updateErrorMessage(error) };
    }
  });
}

export function subscribeUpdateStatus(window: BrowserWindow): void {
  subscribers.add(window);
  window.webContents.send("garyx:update-status", lastStatus);
  window.on("closed", () => {
    subscribers.delete(window);
  });
}

export function bootstrapAutoUpdater(): void {
  if (bootstrapped) return;
  bootstrapped = true;

  // Never run against dev builds — there is no signed app to replace.
  if (!app.isPackaged) {
    return;
  }

  autoUpdater.autoDownload = true;
  autoUpdater.autoInstallOnAppQuit = true;

  autoUpdater.on("checking-for-update", () => {
    broadcast({ phase: "checking" });
  });
  autoUpdater.on("update-available", (info: UpdateInfo) => {
    broadcast({ phase: "available", info: toUpdateInfo(info) });
  });
  autoUpdater.on("update-not-available", () => {
    broadcast({ phase: "idle" });
  });
  autoUpdater.on("download-progress", (progress) => {
    broadcast({
      phase: "downloading",
      percent: typeof progress.percent === "number" ? progress.percent : 0,
    });
  });
  autoUpdater.on("update-downloaded", (info: UpdateInfo) => {
    broadcast({ phase: "downloaded", info: toUpdateInfo(info) });
  });
  autoUpdater.on("error", (error) => {
    broadcast({ phase: "error", message: updateErrorMessage(error) });
  });

  setTimeout(() => {
    autoUpdater.checkForUpdates().catch((error) => {
      console.warn("[updater] initial check failed", error);
    });
  }, INITIAL_CHECK_DELAY_MS);

  setInterval(() => {
    autoUpdater.checkForUpdates().catch((error) => {
      console.warn("[updater] periodic check failed", error);
    });
  }, RECURRING_CHECK_INTERVAL_MS);
}
