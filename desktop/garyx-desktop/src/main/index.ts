import { isAbsolute, join, relative, resolve } from "node:path";
import { appendFileSync, mkdirSync } from "node:fs";

import {
  app,
  BrowserWindow,
  clipboard,
  dialog,
  ipcMain,
  Menu,
  Tray,
  nativeImage,
  shell,
} from "electron";

import {
  getGatewayStatus,
  setOnStatusChange,
  startGateway,
  stopGateway,
  type GatewayStatus,
} from "./gateway-process";
import {
  GARYX_PROTOCOL,
  extractProtocolUrls,
  parseDesktopDeepLink,
} from "./deep-link";
import {
  bootstrapAutoUpdater,
  registerUpdaterIpc,
  subscribeUpdateStatus,
} from "./updater";

import type {
  CreateAutoResearchRunInput,
  CreateCustomAgentInput,
  CreateTeamInput,
  CreateSkillEntryInput,
  CreateSkillInput,
  CreateAutomationInput,
  CreateThreadInput,
  DesktopDeepLinkEvent,
  DeleteSkillEntryInput,
  DeleteSkillInput,
  DeleteCustomAgentInput,
  DeleteTeamInput,
  DeleteMcpServerInput,
  DeleteAutomationInput,
  DeleteThreadInput,
  DesktopSettings,
  GetSkillEditorInput,
  GatewayConfigDocument,
  ListAutoResearchRunsInput,
  ListCandidatesInput,
  ListWorkspaceFilesInput,
  MarkAutomationSeenInput,
  ReadMemoryDocumentInput,
  PreviewWorkspaceFileInput,
  RevealWorkspaceFileInput,
  ReadSkillFileInput,
  RenameWorkspaceInput,
  RelinkWorkspaceInput,
  RenameThreadInput,
  RemoveWorkspaceInput,
  RunAutomationNowInput,
  SaveSkillFileInput,
  SaveMemoryDocumentInput,
  SelectAutomationInput,
  SelectWorkspaceInput,
  SendMessageInput,
  DeleteSlashCommandInput,
  SelectCandidateInput,
  StopAutoResearchRunInput,
  ToggleSkillInput,
  ToggleMcpServerInput,
  UploadChatAttachmentsInput,
  UploadWorkspaceFilesInput,
  UpdateAutomationInput,
  UpdateCustomAgentInput,
  UpdateTeamInput,
  UpdateMcpServerInput,
  UpdateSkillInput,
  UpdateSlashCommandInput,
  UpsertMcpServerInput,
  UpsertSlashCommandInput,
  ShowBrowserConnectionMenuInput,
} from "@shared/contracts";

import {
  createCustomAgent,
  createTeam,
  createSkill,
  createSkillEntry,
  createAutoResearchRun,
  createMcpServer,
  createSlashCommand,
  bindRemoteChannelEndpoint,
  checkConnection,
  deleteCustomAgent,
  deleteTeam,
  deleteMcpServer,
  deleteSkillEntry,
  deleteSlashCommand,
  deleteSkill,
  detachRemoteChannelEndpoint,
  fetchAutomationActivity,
  fetchChannelEndpoints,
  fetchChannelPlugins,
  startChannelAuthFlow,
  pollChannelAuthFlow,
  fetchGatewaySettings,
  fetchThreadHistory,
  fetchThreadLogs,
  getAutoResearchRun,
  interruptThread,
  listAutoResearchCandidates,
  listAutoResearchRuns,
  listAutoResearchIterations,
  listCustomAgents,
  listTeams,
  listWorkspaceFiles,
  listMcpServers,
  listSlashCommands,
  listSkills,
  getSkillEditor,
  openChatStream,
  previewWorkspaceFile,
  probeGateway,
  readSkillFile,
  saveGatewaySettings,
  saveSkillFile,
  sendStreamingInput,
  deleteAutoResearchRun,
  selectAutoResearchCandidate,
  stopAutoResearchRun,
  toggleMcpServer,
  toggleSkill,
  uploadWorkspaceFiles,
  uploadChatAttachments,
  updateCustomAgent,
  updateTeam,
  updateMcpServer,
  updateSkill,
  updateSlashCommand,
} from "./gary-client";
import {
  addChannelAccount,
  pollFeishuChannelAuth,
  pollWeixinChannelAuth,
  startFeishuChannelAuth,
  startWeixinChannelAuth,
} from "./channel-setup";
import {
  addDesktopWorkspace,
  createDesktopAutomation,
  createDesktopThread,
  deleteDesktopAutomation,
  deleteDesktopThread,
  getDesktopState,
  getLocalDesktopSettings,
  markDesktopAutomationSeen,
  relinkDesktopWorkspace,
  runDesktopAutomationNow,
  renameDesktopWorkspace,
  renameDesktopThread,
  rememberDesktopGatewayProfile,
  saveDesktopSettings,
  selectDesktopAutomation,
  selectDesktopWorkspace,
  updateDesktopAutomation,
  removeDesktopWorkspace,
  setDesktopBotBinding,
} from "./store";
import { readMemoryDocument, saveMemoryDocument } from "./memory-documents";
import {
  activateBrowserTab,
  bindBrowserWindow,
  browserGoBack,
  browserGoForward,
  browserOpenExternal,
  browserReload,
  closeBrowserTab,
  createBrowserTab,
  listBrowserState,
  navigateBrowserTab,
  subscribeBrowserState,
  unbindBrowserWindow,
  unsubscribeBrowserState,
  updateBrowserBounds,
  setBrowserOverlayPaused,
} from "./browser-runtime";

let mainWindow: BrowserWindow | null = null;
let tray: Tray | null = null;
let isQuitting = false;
const deepLinkSubscribers = new Set<Electron.WebContents>();
const pendingDeepLinks: DesktopDeepLinkEvent[] = [];
const recentDeepLinkTimestamps = new Map<string, number>();
const startupDeepLinkUrls = extractProtocolUrls(process.argv);

const userDataOverride = process.env.GARYX_DESKTOP_USER_DATA_PATH;
if (userDataOverride) {
  app.setPath("userData", userDataOverride);
}

function pruneRecentDeepLinks(now: number): void {
  for (const [url, seenAt] of recentDeepLinkTimestamps.entries()) {
    if (now - seenAt > 2_000) {
      recentDeepLinkTimestamps.delete(url);
    }
  }
}

function rememberRecentDeepLink(rawUrl: string): boolean {
  const normalized = rawUrl.trim();
  if (!normalized) {
    return false;
  }
  const now = Date.now();
  pruneRecentDeepLinks(now);
  const previous = recentDeepLinkTimestamps.get(normalized) || 0;
  if (now - previous <= 2_000) {
    return false;
  }
  recentDeepLinkTimestamps.set(normalized, now);
  return true;
}

function writeBootstrapTrace(message: string): void {
  try {
    const dir = app.getPath("userData");
    mkdirSync(dir, { recursive: true });
    appendFileSync(
      join(dir, "desktop-bootstrap.log"),
      `${new Date().toISOString()} ${message}\n`,
    );
  } catch {
    // Best-effort only.
  }
}

function preloadPath(): string {
  return join(__dirname, "../preload/index.mjs");
}

function rendererIndexPath(): string {
  return join(__dirname, "../renderer/index.html");
}

function registerDeepLinkProtocol(): void {
  const success =
    process.defaultApp && process.argv[1]
      ? app.setAsDefaultProtocolClient(GARYX_PROTOCOL, process.execPath, [
          resolve(process.argv[1]),
        ])
      : app.setAsDefaultProtocolClient(GARYX_PROTOCOL);
  writeBootstrapTrace(`deepLink:register:${success ? "ok" : "failed"}`);
}

function dispatchDeepLink(event: DesktopDeepLinkEvent): void {
  if (deepLinkSubscribers.size === 0) {
    pendingDeepLinks.push(event);
    return;
  }
  for (const subscriber of deepLinkSubscribers) {
    if (!subscriber.isDestroyed()) {
      subscriber.send("garyx:deep-link", event);
    }
  }
}

function showMainWindow(): void {
  if (!app.isReady()) {
    return;
  }
  if (mainWindow && !mainWindow.isDestroyed()) {
    mainWindow.show();
    mainWindow.focus();
    return;
  }
  mainWindow = createMainWindow();
}

function prepareForAppQuit(): void {
  isQuitting = true;
  stopGateway();
}

function queueDeepLink(rawUrl: string): void {
  if (!rememberRecentDeepLink(rawUrl)) {
    return;
  }
  const event = parseDesktopDeepLink(rawUrl);
  const traceLabel =
    event.type === "open-thread"
      ? `open-thread:${event.threadId}`
      : event.type === "resume-session"
        ? `resume-session:${event.providerHint || "auto"}`
        : `error:${event.error}`;
  writeBootstrapTrace(`deepLink:${traceLabel}`);
  showMainWindow();
  dispatchDeepLink(event);
}

function createMainWindow(): BrowserWindow {
  writeBootstrapTrace("createMainWindow:start");
  const useMacSidebarVibrancy = process.platform === "darwin";
  const window = new BrowserWindow({
    width: 1480,
    height: 940,
    minWidth: 1180,
    minHeight: 760,
    backgroundColor: useMacSidebarVibrancy ? "#00000000" : "#ffffff",
    transparent: useMacSidebarVibrancy,
    title: "Garyx",
    titleBarStyle: "hiddenInset",
    trafficLightPosition: {
      x: 18,
      y: 18,
    },
    // `menu` is lighter/brighter than `sidebar` — matches the near-white
    // native material Codex uses. `sidebar` reads noticeably cooler/greyer,
    // which made our left rail look dimmer than intended even with a thin
    // color wash on top.
    vibrancy: useMacSidebarVibrancy ? "menu" : undefined,
    visualEffectState: useMacSidebarVibrancy ? "active" : undefined,
    webPreferences: {
      preload: preloadPath(),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
    },
  });

  window.webContents.setWindowOpenHandler(({ url }) => {
    if (url) {
      void shell.openExternal(url);
    }
    return { action: "deny" };
  });

  const devServerUrl = process.env.ELECTRON_RENDERER_URL;
  if (devServerUrl) {
    void window.loadURL(devServerUrl);
  } else {
    void window.loadFile(rendererIndexPath());
  }
  writeBootstrapTrace("createMainWindow:loaded-entry");

  bindBrowserWindow(window);
  subscribeUpdateStatus(window);
  window.on("close", (event) => {
    if (!isQuitting) {
      event.preventDefault();
      window.hide();
    }
  });
  window.on("closed", () => {
    writeBootstrapTrace("createMainWindow:closed");
    unbindBrowserWindow(window);
  });

  return window;
}

async function resolveSettings(): Promise<DesktopSettings> {
  return getLocalDesktopSettings();
}

async function pickWorkspaceDirectory(
  defaultPath?: string | null,
): Promise<string | null> {
  const ownerWindow =
    mainWindow && !mainWindow.isDestroyed() ? mainWindow : undefined;
  const options: Electron.OpenDialogOptions = {
    properties: ["openDirectory", "createDirectory"],
  };
  if (defaultPath) {
    options.defaultPath = defaultPath;
  }
  const result = ownerWindow
    ? await dialog.showOpenDialog(ownerWindow, options)
    : await dialog.showOpenDialog(options);
  if (result.canceled || result.filePaths.length === 0) {
    return null;
  }
  return result.filePaths[0] || null;
}

function resolveWorkspaceFileDisplayPath(input: RevealWorkspaceFileInput): string {
  const workspaceRoot = resolve(input.workspacePath.trim());
  const filePath = input.filePath.trim();
  if (!workspaceRoot || !filePath) {
    throw new Error("workspacePath and filePath are required");
  }

  const targetPath = resolve(workspaceRoot, filePath);
  const relativePath = relative(workspaceRoot, targetPath);
  if (!relativePath || relativePath.startsWith("..") || isAbsolute(relativePath)) {
    throw new Error("filePath must stay within workspacePath");
  }
  return targetPath;
}

function messagePreviewText(input: SendMessageInput): string {
  const text = input.message.trim();
  if (text) {
    return text;
  }
  const imageCount = input.images?.length || 0;
  const fileCount = input.files?.length || 0;
  if (imageCount && fileCount) {
    return `${imageCount} image${imageCount === 1 ? "" : "s"}, ${fileCount} file${fileCount === 1 ? "" : "s"}`;
  }
  if (imageCount) {
    return `${imageCount} image${imageCount === 1 ? "" : "s"}`;
  }
  return fileCount ? `${fileCount} file${fileCount === 1 ? "" : "s"}` : "";
}

function registerIpcHandlers(): void {
  writeBootstrapTrace("registerIpcHandlers:start");
  ipcMain.handle("garyx:get-state", async () => {
    return getDesktopState();
  });

  ipcMain.handle(
    "garyx:save-settings",
    async (_event, settings: DesktopSettings) => {
      return saveDesktopSettings(settings);
    },
  );

  ipcMain.handle("garyx:remember-gateway-profile", async () => {
    return rememberDesktopGatewayProfile();
  });

  ipcMain.handle("garyx:get-gateway-settings", async () => {
    const settings = await resolveSettings();
    return fetchGatewaySettings(settings);
  });

  ipcMain.handle("garyx:fetch-channel-plugins", async () => {
    const settings = await resolveSettings();
    return fetchChannelPlugins(settings);
  });

  ipcMain.handle("garyx:open-external-url", async (_event, input: { url?: string }) => {
    const url = String(input?.url || "").trim();
    const parsed = new URL(url);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      throw new Error("only http(s) URLs can be opened externally");
    }
    await shell.openExternal(parsed.toString());
  });

  // Channel-blind auth-flow IPC. The renderer never cares which
  // plugin it's talking to; these two handlers proxy straight to
  // the gateway's `/api/channels/plugins/{id}/auth_flow/{start,poll}`
  // endpoints. A later pass deprecates the per-channel
  // `start-{weixin,feishu}-channel-auth` handlers below.
  ipcMain.handle(
    "garyx:start-channel-auth-flow",
    async (_event, input: { pluginId: string; formState?: Record<string, unknown> }) => {
      const settings = await resolveSettings();
      return startChannelAuthFlow(settings, input.pluginId, input.formState ?? {});
    },
  );

  ipcMain.handle(
    "garyx:poll-channel-auth-flow",
    async (_event, input: { pluginId: string; sessionId: string }) => {
      const settings = await resolveSettings();
      return pollChannelAuthFlow(settings, input.pluginId, input.sessionId);
    },
  );

  ipcMain.handle(
    "garyx:save-gateway-settings",
    async (_event, config: GatewayConfigDocument) => {
      const settings = await resolveSettings();
      return saveGatewaySettings(settings, config);
    },
  );

  ipcMain.handle("garyx:add-channel-account", async (_event, input) => {
    const settings = await resolveSettings();
    await addChannelAccount(settings, input);
    return getDesktopState();
  });

  ipcMain.handle("garyx:start-weixin-channel-auth", async (_event, input) => {
    return startWeixinChannelAuth(input);
  });

  ipcMain.handle("garyx:poll-weixin-channel-auth", async (_event, input) => {
    const settings = await resolveSettings();
    return pollWeixinChannelAuth(settings, input);
  });

  ipcMain.handle("garyx:start-feishu-channel-auth", async (_event, input) => {
    return startFeishuChannelAuth(input);
  });

  ipcMain.handle("garyx:poll-feishu-channel-auth", async (_event, input) => {
    const settings = await resolveSettings();
    return pollFeishuChannelAuth(settings, input);
  });

  ipcMain.handle(
    "garyx:select-workspace",
    async (_event, input: SelectWorkspaceInput) => {
      return selectDesktopWorkspace(input.workspacePath);
    },
  );

  ipcMain.handle(
    "garyx:pick-directory",
    async (_event, input?: { defaultPath?: string | null }) => {
      return pickWorkspaceDirectory(input?.defaultPath ?? null);
    },
  );

  ipcMain.handle("garyx:add-workspace", async () => {
    const selectedPath = await pickWorkspaceDirectory();
    if (!selectedPath) {
      return {
        state: await getDesktopState(),
        workspace: null,
        cancelled: true,
      };
    }
    const result = await addDesktopWorkspace(selectedPath);
    return {
      ...result,
      cancelled: false,
    };
  });

  ipcMain.handle(
    "garyx:add-workspace-by-path",
    async (_event, input: { path: string }) => {
      const result = await addDesktopWorkspace(input.path);
      return {
        ...result,
        cancelled: false,
      };
    },
  );

  ipcMain.handle(
    "garyx:relink-workspace",
    async (_event, input: RelinkWorkspaceInput) => {
      const selectedPath = await pickWorkspaceDirectory();
      if (!selectedPath) {
        return {
          state: await getDesktopState(),
          workspace: null,
          cancelled: true,
        };
      }
      const result = await relinkDesktopWorkspace(
        input.workspacePath,
        selectedPath,
      );
      return {
        ...result,
        cancelled: false,
      };
    },
  );

  ipcMain.handle(
    "garyx:rename-workspace",
    async (_event, input: RenameWorkspaceInput) => {
      return renameDesktopWorkspace(input.workspacePath, input.name);
    },
  );

  ipcMain.handle(
    "garyx:remove-workspace",
    async (_event, input: RemoveWorkspaceInput) => {
      return removeDesktopWorkspace(input.workspacePath);
    },
  );

  ipcMain.handle(
    "garyx:select-automation",
    async (_event, input: SelectAutomationInput) => {
      return selectDesktopAutomation(input.automationId);
    },
  );

  ipcMain.handle(
    "garyx:mark-automation-seen",
    async (_event, input: MarkAutomationSeenInput) => {
      return markDesktopAutomationSeen(input.automationId, input.seenAt);
    },
  );

  ipcMain.handle(
    "garyx:create-automation",
    async (_event, input: CreateAutomationInput) => {
      return createDesktopAutomation(input);
    },
  );

  ipcMain.handle(
    "garyx:update-automation",
    async (_event, input: UpdateAutomationInput) => {
      return updateDesktopAutomation(input);
    },
  );

  ipcMain.handle(
    "garyx:delete-automation",
    async (_event, input: DeleteAutomationInput) => {
      return deleteDesktopAutomation(input.automationId);
    },
  );

  ipcMain.handle("garyx:list-skills", async () => {
    const settings = await resolveSettings();
    return listSkills(settings);
  });

  ipcMain.handle("garyx:list-custom-agents", async () => {
    const settings = await resolveSettings();
    return listCustomAgents(settings);
  });

  ipcMain.handle(
    "garyx:create-custom-agent",
    async (_event, input: CreateCustomAgentInput) => {
      const settings = await resolveSettings();
      return createCustomAgent(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:update-custom-agent",
    async (_event, input: UpdateCustomAgentInput) => {
      const settings = await resolveSettings();
      return updateCustomAgent(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:delete-custom-agent",
    async (_event, input: DeleteCustomAgentInput) => {
      const settings = await resolveSettings();
      return deleteCustomAgent(settings, input);
    },
  );

  ipcMain.handle("garyx:list-teams", async () => {
    const settings = await resolveSettings();
    return listTeams(settings);
  });

  ipcMain.handle("garyx:create-team", async (_event, input: CreateTeamInput) => {
    const settings = await resolveSettings();
    return createTeam(settings, input);
  });

  ipcMain.handle("garyx:update-team", async (_event, input: UpdateTeamInput) => {
    const settings = await resolveSettings();
    return updateTeam(settings, input);
  });

  ipcMain.handle("garyx:delete-team", async (_event, input: DeleteTeamInput) => {
    const settings = await resolveSettings();
    return deleteTeam(settings, input);
  });

  ipcMain.handle(
    "garyx:create-skill",
    async (_event, input: CreateSkillInput) => {
      const settings = await resolveSettings();
      return createSkill(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:update-skill",
    async (_event, input: UpdateSkillInput) => {
      const settings = await resolveSettings();
      return updateSkill(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:toggle-skill",
    async (_event, input: ToggleSkillInput) => {
      const settings = await resolveSettings();
      return toggleSkill(settings, input.skillId);
    },
  );

  ipcMain.handle(
    "garyx:delete-skill",
    async (_event, input: DeleteSkillInput) => {
      const settings = await resolveSettings();
      return deleteSkill(settings, input.skillId);
    },
  );

  ipcMain.handle(
    "garyx:get-skill-editor",
    async (_event, input: GetSkillEditorInput) => {
      const settings = await resolveSettings();
      return getSkillEditor(settings, input.skillId);
    },
  );

  ipcMain.handle(
    "garyx:read-skill-file",
    async (_event, input: ReadSkillFileInput) => {
      const settings = await resolveSettings();
      return readSkillFile(settings, input.skillId, input.path);
    },
  );

  ipcMain.handle(
    "garyx:save-skill-file",
    async (_event, input: SaveSkillFileInput) => {
      const settings = await resolveSettings();
      return saveSkillFile(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:read-memory-document",
    async (_event, input: ReadMemoryDocumentInput) => {
      return readMemoryDocument(input);
    },
  );

  ipcMain.handle(
    "garyx:save-memory-document",
    async (_event, input: SaveMemoryDocumentInput) => {
      return saveMemoryDocument(input);
    },
  );

  ipcMain.handle(
    "garyx:list-workspace-files",
    async (_event, input: ListWorkspaceFilesInput) => {
      const settings = await resolveSettings();
      return listWorkspaceFiles(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:preview-workspace-file",
    async (_event, input: PreviewWorkspaceFileInput) => {
      const settings = await resolveSettings();
      return previewWorkspaceFile(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:reveal-workspace-file",
    async (_event, input: RevealWorkspaceFileInput) => {
      shell.showItemInFolder(resolveWorkspaceFileDisplayPath(input));
    },
  );

  ipcMain.handle(
    "garyx:upload-chat-attachments",
    async (_event, input: UploadChatAttachmentsInput) => {
      const settings = await resolveSettings();
      return uploadChatAttachments(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:upload-workspace-files",
    async (_event, input: UploadWorkspaceFilesInput) => {
      const settings = await resolveSettings();
      return uploadWorkspaceFiles(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:create-skill-entry",
    async (_event, input: CreateSkillEntryInput) => {
      const settings = await resolveSettings();
      return createSkillEntry(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:delete-skill-entry",
    async (_event, input: DeleteSkillEntryInput) => {
      const settings = await resolveSettings();
      return deleteSkillEntry(settings, input);
    },
  );

  ipcMain.handle("garyx:list-slash-commands", async () => {
    const settings = await resolveSettings();
    return listSlashCommands(settings);
  });

  ipcMain.handle(
    "garyx:create-slash-command",
    async (_event, input: UpsertSlashCommandInput) => {
      const settings = await resolveSettings();
      return createSlashCommand(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:update-slash-command",
    async (_event, input: UpdateSlashCommandInput) => {
      const settings = await resolveSettings();
      return updateSlashCommand(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:delete-slash-command",
    async (_event, input: DeleteSlashCommandInput) => {
      const settings = await resolveSettings();
      return deleteSlashCommand(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:create-auto-research-run",
    async (_event, input: CreateAutoResearchRunInput) => {
      const settings = await resolveSettings();
      return createAutoResearchRun(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:list-auto-research-runs",
    async (_event, input?: ListAutoResearchRunsInput) => {
      const settings = await resolveSettings();
      return listAutoResearchRuns(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:get-auto-research-run",
    async (_event, runId: string) => {
      const settings = await resolveSettings();
      return getAutoResearchRun(settings, runId);
    },
  );

  ipcMain.handle(
    "garyx:list-auto-research-iterations",
    async (_event, runId: string) => {
      const settings = await resolveSettings();
      return listAutoResearchIterations(settings, runId);
    },
  );

  ipcMain.handle(
    "garyx:stop-auto-research-run",
    async (_event, input: StopAutoResearchRunInput) => {
      const settings = await resolveSettings();
      return stopAutoResearchRun(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:delete-auto-research-run",
    async (_event, runId: string) => {
      const settings = await resolveSettings();
      return deleteAutoResearchRun(settings, runId);
    },
  );

  ipcMain.handle(
    "garyx:list-auto-research-candidates",
    async (_event, input: ListCandidatesInput) => {
      const settings = await resolveSettings();
      return listAutoResearchCandidates(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:select-auto-research-candidate",
    async (_event, input: SelectCandidateInput) => {
      const settings = await resolveSettings();
      return selectAutoResearchCandidate(settings, input);
    },
  );

  ipcMain.handle("garyx:list-mcp-servers", async () => {
    const settings = await resolveSettings();
    return listMcpServers(settings);
  });

  ipcMain.handle(
    "garyx:create-mcp-server",
    async (_event, input: UpsertMcpServerInput) => {
      const settings = await resolveSettings();
      return createMcpServer(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:update-mcp-server",
    async (_event, input: UpdateMcpServerInput) => {
      const settings = await resolveSettings();
      return updateMcpServer(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:delete-mcp-server",
    async (_event, input: DeleteMcpServerInput) => {
      const settings = await resolveSettings();
      return deleteMcpServer(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:toggle-mcp-server",
    async (_event, input: ToggleMcpServerInput) => {
      const settings = await resolveSettings();
      return toggleMcpServer(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:get-automation-activity",
    async (_event, automationId: string) => {
      const settings = await resolveSettings();
      return fetchAutomationActivity(settings, automationId);
    },
  );

  ipcMain.handle(
    "garyx:run-automation-now",
    async (_event, input: RunAutomationNowInput) => {
      return runDesktopAutomationNow(input.automationId);
    },
  );

  ipcMain.handle(
    "garyx:set-bot-binding",
    async (_event, input: { threadId: string; botId: string | null }) => {
      return setDesktopBotBinding(input.threadId, input.botId);
    },
  );

  ipcMain.handle("garyx:list-channel-endpoints", async () => {
    const state = await getDesktopState();
    return state.endpoints;
  });

  ipcMain.handle(
    "garyx:bind-channel-endpoint",
    async (_event, input: { endpointKey: string; threadId: string }) => {
      const settings = await resolveSettings();
      await bindRemoteChannelEndpoint(settings, input);
      return getDesktopState();
    },
  );

  ipcMain.handle(
    "garyx:detach-channel-endpoint",
    async (_event, input: { endpointKey: string }) => {
      const settings = await resolveSettings();
      await detachRemoteChannelEndpoint(settings, input);
      return getDesktopState();
    },
  );

  ipcMain.handle(
    "garyx:create-thread",
    async (_event, input?: CreateThreadInput) => {
      return createDesktopThread(input);
    },
  );

  ipcMain.handle(
    "garyx:rename-thread",
    async (_event, input: RenameThreadInput) => {
      return renameDesktopThread(
        input.threadId || input.sessionId || "",
        input.title,
      );
    },
  );

  ipcMain.handle(
    "garyx:delete-thread",
    async (_event, input: DeleteThreadInput) => {
      return deleteDesktopThread(input.threadId || input.sessionId || "");
    },
  );

  ipcMain.handle(
    "garyx:get-thread-history",
    async (_event, threadId: string) => {
      const settings = await resolveSettings();
      return fetchThreadHistory(settings, threadId);
    },
  );
  ipcMain.handle(
    "garyx:get-thread-logs",
    async (
      _event,
      input: { threadId?: string; sessionId?: string; cursor?: number },
    ) => {
      const settings = await resolveSettings();
      return fetchThreadLogs(
        settings,
        input.threadId || input.sessionId || "",
        input.cursor,
      );
    },
  );

  ipcMain.handle(
    "garyx:open-chat-stream",
    async (_event, input: SendMessageInput) => {
      const settings = await resolveSettings();
      const result = await openChatStream(settings, input, (payload) => {
        if (!mainWindow || mainWindow.isDestroyed()) {
          return;
        }
        mainWindow.webContents.send("garyx:chat-stream", payload);
      });
      const state = await getDesktopState();
      const thread = state.threads.find(
        (entry) => entry.id === result.threadId,
      ) || {
        id: result.threadId,
        title: result.threadId,
        createdAt: new Date().toISOString(),
        updatedAt: new Date().toISOString(),
        lastMessagePreview: messagePreviewText(input),
        workspacePath: "",
      };
      return {
        ...result,
        thread,
        session: thread,
      };
    },
  );

  ipcMain.handle(
    "garyx:send-streaming-input",
    async (_event, input: SendMessageInput) => {
      const settings = await resolveSettings();
      return sendStreamingInput(settings, input);
    },
  );

  ipcMain.on("garyx:deep-link-subscribe", (event) => {
    deepLinkSubscribers.add(event.sender);
    event.sender.once("destroyed", () => {
      deepLinkSubscribers.delete(event.sender);
    });
    if (pendingDeepLinks.length > 0) {
      for (const payload of pendingDeepLinks.splice(
        0,
        pendingDeepLinks.length,
      )) {
        event.sender.send("garyx:deep-link", payload);
      }
    }
  });

  ipcMain.on("garyx:deep-link-unsubscribe", (event) => {
    deepLinkSubscribers.delete(event.sender);
  });

  ipcMain.handle("garyx:interrupt-thread", async (_event, threadId: string) => {
    const settings = await resolveSettings();
    return interruptThread(settings, threadId);
  });

  ipcMain.handle(
    "garyx:check-connection",
    async (
      _event,
      input?: { gatewayUrl?: string; gatewayAuthToken?: string },
    ) => {
      const settings = await resolveSettings();
      const nextSettings = input
        ? {
            ...settings,
            gatewayUrl:
              typeof input.gatewayUrl === "string"
                ? input.gatewayUrl
                : settings.gatewayUrl,
            gatewayAuthToken:
              typeof input.gatewayAuthToken === "string"
                ? input.gatewayAuthToken
                : settings.gatewayAuthToken,
          }
        : settings;
      return checkConnection(nextSettings);
    },
  );

  ipcMain.handle(
    "garyx:probe-gateway",
    async (
      _event,
      input: { gatewayUrl: string; gatewayAuthToken: string },
    ) => {
      return probeGateway(input);
    },
  );

  ipcMain.handle("garyx:list-browser-state", async () => {
    return listBrowserState();
  });
  ipcMain.handle("garyx:create-browser-tab", createBrowserTab);
  ipcMain.handle("garyx:activate-browser-tab", activateBrowserTab);
  ipcMain.handle("garyx:close-browser-tab", closeBrowserTab);
  ipcMain.handle("garyx:navigate-browser-tab", navigateBrowserTab);
  ipcMain.handle("garyx:browser-go-back", browserGoBack);
  ipcMain.handle("garyx:browser-go-forward", browserGoForward);
  ipcMain.handle("garyx:browser-reload", browserReload);
  ipcMain.handle("garyx:browser-open-external", browserOpenExternal);
  ipcMain.handle("garyx:update-browser-bounds", updateBrowserBounds);
  ipcMain.handle("garyx:set-browser-overlay-paused", setBrowserOverlayPaused);
  ipcMain.handle("garyx:show-browser-connection-menu", (_event, input: ShowBrowserConnectionMenuInput) => {
    showBrowserConnectionMenu(input);
  });
  ipcMain.on("garyx:browser-state-subscribe", (event) => {
    const state = subscribeBrowserState(event);
    event.sender.send("garyx:browser-state", state);
  });
  ipcMain.on("garyx:browser-state-unsubscribe", (event) => {
    unsubscribeBrowserState(event);
  });
  writeBootstrapTrace("registerIpcHandlers:done");
}

function statusLabel(s: GatewayStatus): string {
  switch (s) {
    case "starting":
      return "Gateway: Starting...";
    case "running":
      return "Gateway: Running";
    case "stopped":
      return "Gateway: Stopped";
    case "error":
      return "Gateway: Error";
  }
}

function buildTrayMenu(): Menu {
  const gwStatus = getGatewayStatus();
  return Menu.buildFromTemplate([
    { label: statusLabel(gwStatus), enabled: false },
    { type: "separator" },
    {
      label: "Show Window",
      click: () => {
        if (mainWindow && !mainWindow.isDestroyed()) {
          mainWindow.show();
          mainWindow.focus();
        } else {
          mainWindow = createMainWindow();
        }
      },
    },
    { type: "separator" },
    {
      label: "Quit Garyx",
      click: () => {
        prepareForAppQuit();
        app.quit();
      },
    },
  ]);
}

function showBrowserConnectionMenu(input: ShowBrowserConnectionMenuInput): void {
  const state = listBrowserState();
  const menu = Menu.buildFromTemplate([
    { label: "CDP", enabled: false },
    { label: state.debugEndpoint.origin, enabled: false },
    { type: "separator" },
    { label: "PROFILE", enabled: false },
    { label: state.partition, enabled: false },
    { type: "separator" },
    {
      label: input.labels?.copyCdpEndpoint || "Copy CDP Endpoint",
      click: () => {
        clipboard.writeText(state.debugEndpoint.origin);
      },
    },
    {
      label: input.labels?.copyCdpListUrl || "Copy CDP List URL",
      click: () => {
        clipboard.writeText(state.debugEndpoint.listUrl);
      },
    },
  ]);

  menu.popup({
    window: mainWindow ?? undefined,
    x: Math.max(8, Math.round(input.x)),
    y: Math.max(8, Math.round(input.y)),
  });
}

function createTray(): void {
  writeBootstrapTrace("createTray:start");
  const trayIconPath = app.isPackaged
    ? join(process.resourcesPath, "trayTemplate.png")
    : join(__dirname, "../../resources/trayTemplate.png");
  const icon = nativeImage.createFromPath(trayIconPath);
  icon.setTemplateImage(true);
  tray = new Tray(icon);
  tray.setToolTip("Garyx");
  tray.setContextMenu(buildTrayMenu());

  tray.on("click", () => {
    if (mainWindow && !mainWindow.isDestroyed()) {
      mainWindow.show();
      mainWindow.focus();
    } else {
      mainWindow = createMainWindow();
    }
  });
  writeBootstrapTrace("createTray:done");
}

app.on("open-url", (event, url) => {
  event.preventDefault();
  queueDeepLink(url);
});

app
  .whenReady()
  .then(() => {
    writeBootstrapTrace("whenReady:start");
    app.setName("Garyx");
    registerDeepLinkProtocol();
    registerIpcHandlers();
    registerUpdaterIpc({ prepareForInstall: prepareForAppQuit });

    // Ensure the launchd-managed gateway owns the configured port.
    startGateway();
    writeBootstrapTrace("whenReady:gateway-started");

    mainWindow = createMainWindow();
    writeBootstrapTrace("whenReady:window-created");

    bootstrapAutoUpdater();
    writeBootstrapTrace("whenReady:updater-bootstrapped");

    try {
      createTray();
    } catch (error) {
      console.error("failed to create tray", error);
      writeBootstrapTrace(
        `createTray:error:${error instanceof Error ? error.message : String(error)}`,
      );
    }

    // Update tray when gateway status changes
    setOnStatusChange(() => {
      if (tray) {
        tray.setContextMenu(buildTrayMenu());
      }
    });

    app.on("activate", () => {
      showMainWindow();
    });

    for (const url of startupDeepLinkUrls) {
      queueDeepLink(url);
    }
    writeBootstrapTrace("whenReady:done");
  })
  .catch((error) => {
    console.error("desktop bootstrap failed", error);
    writeBootstrapTrace(
      `whenReady:error:${error instanceof Error ? error.message : String(error)}`,
    );
  });

app.on("before-quit", () => {
  prepareForAppQuit();
});

app.on("window-all-closed", () => {
  // On macOS, don't quit when windows close — tray keeps running
  if (process.platform !== "darwin") {
    stopGateway();
    app.quit();
  }
});
