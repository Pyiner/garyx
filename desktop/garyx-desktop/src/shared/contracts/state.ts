import type { DesktopAutomationSummary } from "./automation.ts";
import type { ConfiguredBot, DesktopBotConsoleSummary } from "./bot.ts";
import type { DesktopChannelEndpoint } from "./channel.ts";
import type { DesktopGatewayProfile, DesktopSettings } from "./settings.ts";
import type { DesktopThreadSummary } from "./thread.ts";
import type { DesktopWorkspace } from "./workspace.ts";

export interface DesktopRemoteStateError {
  source: "threads" | "thread_pins" | "endpoints" | "workspaces" | "configured_bots" | "bot_consoles" | "automations";
  label: string;
  message: string;
}

export interface DesktopState {
  settings: DesktopSettings;
  gatewayProfiles: DesktopGatewayProfile[];
  /** Gateway URL the entity slices below were loaded from. Slices from a
   *  different gateway are dropped on hydrate instead of leaking into the
   *  newly selected gateway's view. */
  entitiesGatewayUrl?: string | null;
  workspaces: DesktopWorkspace[];
  selectedWorkspacePath: string | null;
  pinnedThreadIds: string[];
  /** Highest accepted revision of the gateway's atomic thread-pins page. */
  pinsRevision: number;
  threads: DesktopThreadSummary[];
  sessions: DesktopThreadSummary[];
  endpoints: DesktopChannelEndpoint[];
  configuredBots: ConfiguredBot[];
  botConsoles: DesktopBotConsoleSummary[];
  automations: DesktopAutomationSummary[];
  selectedAutomationId: string | null;
  lastSeenRunAtByAutomation: Record<string, string>;
  botMainThreads: Record<string, string>;
  remoteErrors: DesktopRemoteStateError[];
}

export interface WorkspaceMutationResult {
  state: DesktopState;
  workspace: DesktopWorkspace | null;
  cancelled: boolean;
}
