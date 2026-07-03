export interface DesktopUpdateInfo {
  version: string;
  releaseNotes?: string;
  releaseName?: string;
}

export type DesktopUpdateStatus =
  | { phase: "idle" }
  | { phase: "checking" }
  | { phase: "available"; info: DesktopUpdateInfo }
  | { phase: "downloading"; percent: number }
  | { phase: "downloaded"; info: DesktopUpdateInfo }
  | { phase: "installing"; info: DesktopUpdateInfo }
  | { phase: "error"; message: string };

export type DesktopUpdateCheckResult =
  | { ok: true }
  | { ok: false; reason: string };

export type DesktopUpdateInstallResult =
  | { ok: true }
  | { ok: false; reason: string };

export type DesktopUpdateStatusListener = (status: DesktopUpdateStatus) => void;
