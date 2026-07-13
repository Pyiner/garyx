export type DesktopLanguagePreference = "system" | "en" | "zh-CN";

export type DesktopFollowUpBehavior = "queue" | "steer";

export interface DesktopSettings {
  gatewayUrl: string;
  gatewayAuthToken: string;
  gatewayHeaders: string;
  accountId: string;
  fromId: string;
  timeoutSeconds: number;
  languagePreference: DesktopLanguagePreference;
  followUpBehavior: DesktopFollowUpBehavior;
}

export interface DesktopGatewayProfile {
  id: string;
  label: string;
  gatewayUrl: string;
  gatewayAuthToken: string;
  gatewayHeaders: string;
  updatedAt: string;
}

export interface ConnectionStatus {
  ok: boolean;
  bridgeReady: boolean;
  gatewayUrl: string;
  version?: string;
  uptimeSeconds?: number;
  threadCount?: number;
  sessionCount?: number;
  error?: string;
}

export interface GatewayProbeResult {
  ok: boolean;
  isGaryGateway: boolean;
  gatewayUrl: string;
  path: string;
  version?: string;
  host?: string;
  port?: number;
  error?: string;
}

export type GatewayConfigDocument = Record<string, unknown>;

export type GatewaySettingsSource = "local_file" | "gateway_api";

export interface GatewaySettingsPayload {
  config: GatewayConfigDocument;
  source: GatewaySettingsSource;
  secretsMasked: boolean;
}

export interface GatewaySettingsSaveResult {
  ok: boolean;
  message?: string;
  errors?: string[];
  settings: GatewaySettingsPayload;
}

export interface GatewaySettingsSaveRequestOptions {
  merge?: boolean;
}

export const DEFAULT_SESSION_TITLE = "Fresh Thread";

export const DEFAULT_DESKTOP_SETTINGS: DesktopSettings = {
  gatewayUrl: "http://127.0.0.1:31337",
  gatewayAuthToken: "",
  gatewayHeaders: "",
  accountId: "main",
  fromId: "mac-desktop",
  timeoutSeconds: 120,
  languagePreference: "system",
  followUpBehavior: "queue",
};
