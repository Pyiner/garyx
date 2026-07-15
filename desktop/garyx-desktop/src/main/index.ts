import { isAbsolute, join, relative, resolve } from "node:path";
import {
  appendFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  renameSync,
} from "node:fs";
import {
  app,
  BrowserWindow,
  clipboard,
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
import {
  bindWindowLayoutRuntime,
  registerWindowLayoutIpc,
} from "./window-layout-runtime";
import { resolveHorizontalLayoutPolicy } from "@shared/contracts";

import type {
  ArchiveThreadInput,
  CancelCustomAgentAvatarInput,
  CreateCustomAgentInput,
  CreateSkillEntryInput,
  CreateSkillInput,
  CreateTaskInput,
  CreateAutomationInput,
  CreateThreadInput,
  DeleteCapsuleInput,
  DesktopDeepLinkEvent,
  DesktopApiProviderType,
  DeleteSkillEntryInput,
  DeleteSkillInput,
  DeleteCustomAgentInput,
  DeleteMcpServerInput,
  DeleteAutomationInput,
  DeleteThreadInput,
  DesktopSettings,
  GenerateCustomAgentAvatarInput,
  GetTaskInput,
  GetThreadHistoryInput,
  GetSkillEditorInput,
  GatewayConfigDocument,
  GatewaySettingsSaveRequestOptions,
  DeleteTaskInput,
  ListTaskForestInput,
  ListRecentThreadsInput,
  ListProviderRecentSessionsInput,
  ListTasksInput,
  ListWorkspaceFilesInput,
  MarkAutomationSeenInput,
  ReadMemoryDocumentInput,
  PreviewWorkspaceFileInput,
  RevealWorkspaceFileInput,
  ReadSkillFileInput,
  RenameThreadInput,
  RemoveWorkspaceInput,
  RunAutomationNowInput,
  SaveSkillFileInput,
  SaveMemoryDocumentInput,
  SelectAutomationInput,
  SelectWorkspaceInput,
  SetCapsuleFavoriteInput,
  RenderState,
  SendMessageInput,
  StartThreadStreamInput,
  StopThreadStreamInput,
  ThreadTranscript,
  DeleteSlashCommandInput,
  StopTaskInput,
  ToggleSkillInput,
  ToggleMcpServerInput,
  UploadChatAttachmentsInput,
  UploadWorkspaceFilesInput,
  UpdateAutomationInput,
  UpdateCustomAgentInput,
  UpdateThreadRuntimeSettingsInput,
  UpdateMcpServerInput,
  UpdateSkillInput,
  UpdateSlashCommandInput,
  UpdateTaskStatusInput,
  UpdateTaskTitleInput,
  UpsertMcpServerInput,
  UpsertSlashCommandInput,
  ShowBrowserConnectionMenuInput,
  AssignTaskInput,
  CopyTextToClipboardInput,
  UnassignTaskInput,
} from "@shared/contracts";

import {
  createCustomAgent,
  createSkill,
  createSkillEntry,
  createTask,
  createMcpServer,
  createSlashCommand,
  bindRemoteChannelEndpoint,
  checkConnection,
  deleteCapsule,
  deleteCustomAgent,
  deleteMcpServer,
  deleteSkillEntry,
  deleteSlashCommand,
  deleteSkill,
  deleteTask,
  detachRemoteChannelEndpoint,
  fetchAutomationActivity,
  fetchChannelEndpoints,
  fetchChannelPlugins,
  startChannelAuthFlow,
  pollChannelAuthFlow,
  fetchGatewaySettings,
  fetchThreadHistory,
  fetchThreadLogs,
  fetchRecentThreads,
  getCodingUsage,
  getCapsule,
  getCapsuleHtml,
  getTask,
  getWorkspaceGitStatus,
  interruptThread,
  listCapsules,
  setCapsuleFavorite,
  listTaskForest,
  listTasks,
  listProviderRecentSessions,
  listCustomAgents,
  listProviderModels,
  listWorkspaceDirectories,
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
  stopTask,
  toggleMcpServer,
  toggleSkill,
  uploadWorkspaceFiles,
  uploadChatAttachments,
  updateCustomAgent,
  updateMcpServer,
  updateSkill,
  updateSlashCommand,
  updateTaskStatus,
  assignTask,
  assertRecentThreadGatewayScope,
  unassignTask,
  updateTaskTitle,
  updateRemoteThread,
  validateListRecentThreadsInput,
} from "./gary-client";
import { wireGatewayTransport } from "./gateway-transport";
import {
  bindThreadStreamSinkNavigation,
  createThreadStreamHub,
} from "./thread-stream-hub";
import {
  clearThreadTranscriptCache,
  loadThreadTranscriptCache,
  pruneThreadTranscriptCache,
  saveThreadTranscriptCache,
} from "./transcript-cache";
import { addChannelAccount } from "./channel-setup";
import {
  cancelCustomAgentAvatarGeneration,
  generateCustomAgentAvatar,
} from "./agent-avatar";
import {
  evictCapsuleThumbnails,
  renderCapsuleThumbnail,
} from "./capsule-thumbnail";
import {
  addDesktopWorkspace,
  archiveDesktopThread,
  createDesktopAutomation,
  createDesktopThread,
  deleteDesktopAutomation,
  deleteDesktopThread,
  getDesktopState,
  getDesktopStateFast,
  getLocalDesktopSettings,
  markDesktopAutomationSeen,
  runDesktopAutomationNow,
  renameDesktopThread,
  rememberDesktopGatewayProfile,
  addDesktopGatewayProfile,
  updateDesktopGatewayProfile,
  deleteDesktopGatewayProfile,
  saveDesktopSettings,
  selectDesktopAutomation,
  selectDesktopWorkspace,
  updateDesktopAutomation,
  removeDesktopWorkspace,
  setDesktopBotBinding,
  setDesktopThreadPinned,
  setDesktopThreadPinOrder,
} from "./store";
import { readMemoryDocument, saveMemoryDocument } from "./memory-documents";
import {
  activateBrowserTab,
  bindBrowserWindow,
  browserGoBack,
  browserGoForward,
  captureBrowserTab,
  browserOpenExternal,
  browserReload,
  closeBrowserTab,
  copyImageToClipboard,
  createBrowserTab,
  listBrowserState,
  navigateBrowserTab,
  setBrowserAnnotationMode,
  subscribeBrowserAnnotationComments,
  subscribeBrowserPageMouseDown,
  subscribeBrowserState,
  unbindBrowserWindow,
  unsubscribeBrowserAnnotationComments,
  unsubscribeBrowserPageMouseDown,
  unsubscribeBrowserState,
  updateBrowserBounds,
  setBrowserOverlayPaused,
} from "./browser-runtime";
import {
  activateTerminalSession,
  closeTerminalSession,
  createTerminalSession,
  listTerminalState,
  resizeTerminalSession,
  subscribeTerminalState,
  unsubscribeTerminalState,
  writeTerminalInput,
} from "./terminal-runtime";
import {
  commitWorkspaceChanges,
  getWorkspaceGitDetails,
  pushWorkspaceBranch,
} from "./workspace-git-runtime";

let mainWindow: BrowserWindow | null = null;
let tray: Tray | null = null;
let isQuitting = false;

// Chromium transport for all gateway HTTP; per-thread SSE streams ride a
// dedicated session so they cannot starve the control plane's socket pool.
// Rationale lives with the wiring in gateway-transport.ts.
wireGatewayTransport();
const deepLinkSubscribers = new Set<Electron.WebContents>();
const pendingDeepLinks: DesktopDeepLinkEvent[] = [];

const threadStreamHub = createThreadStreamHub({
  resolveSettings: () => resolveSettings(),
  isSinkAlive: () => Boolean(mainWindow && !mainWindow.isDestroyed()),
  sendEvent: (payload) => {
    if (mainWindow && !mainWindow.isDestroyed()) {
      mainWindow.webContents.send("garyx:chat-stream", payload);
    }
  },
});

function stopThreadEventForwarder(input?: StopThreadStreamInput | null): void {
  threadStreamHub.stop(input);
}

function startThreadEventForwarder(input: StartThreadStreamInput): void {
  threadStreamHub.start(input);
}

function restartThreadEventForwarders(): void {
  threadStreamHub.restartAll();
}
const recentDeepLinkTimestamps = new Map<string, number>();
const startupDeepLinkUrls = extractProtocolUrls(process.argv);
const horizontalLayoutPolicy = resolveHorizontalLayoutPolicy(
  process.env.GARYX_DESKTOP_EXPAND_V1,
);

const DEFAULT_USER_DATA_DIR_NAME = "Garyx";
const LEGACY_USER_DATA_DIR_NAME = "garyx-desktop";
const userDataOverride = process.env.GARYX_DESKTOP_USER_DATA_PATH;

function migrateLegacyUserData(legacyPath: string, targetPath: string): void {
  try {
    if (existsSync(targetPath) || !existsSync(legacyPath)) {
      return;
    }
    mkdirSync(app.getPath("appData"), { recursive: true });
    try {
      renameSync(legacyPath, targetPath);
    } catch {
      cpSync(legacyPath, targetPath, {
        errorOnExist: false,
        force: false,
        recursive: true,
      });
    }
  } catch {
    // Best-effort migration; the app can recreate defaults in the new location.
  }
}

function configureUserDataPath(): void {
  if (userDataOverride) {
    app.setPath("userData", userDataOverride);
    return;
  }

  const appDataPath = app.getPath("appData");
  const targetPath = join(appDataPath, DEFAULT_USER_DATA_DIR_NAME);
  const legacyPath = join(appDataPath, LEGACY_USER_DATA_DIR_NAME);
  app.setPath("userData", targetPath);
  migrateLegacyUserData(legacyPath, targetPath);
}

configureUserDataPath();

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
      : event.type === "new-thread"
        ? `new-thread:${event.workspacePath || "default"}`
      : event.type === "resume-session"
        ? `resume-session:${event.providerHint || "auto"}`
      : event.type === "open-capsule"
        ? `open-capsule:${event.capsuleId}`
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
    minWidth: horizontalLayoutPolicy === "expand-v1" ? 480 : 1180,
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
      additionalArguments: [
        `--garyx-horizontal-layout-policy=${horizontalLayoutPolicy}`,
      ],
    },
  });

  bindWindowLayoutRuntime(window, horizontalLayoutPolicy);

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
  bindThreadStreamSinkNavigation(threadStreamHub, window.webContents);
  window.on("close", (event) => {
    if (!isQuitting) {
      event.preventDefault();
      window.hide();
    }
  });
  window.on("closed", () => {
    writeBootstrapTrace("createMainWindow:closed");
    unbindBrowserWindow(window);
    stopThreadEventForwarder();
  });

  return window;
}

async function resolveSettings(): Promise<DesktopSettings> {
  return getLocalDesktopSettings();
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
  registerWindowLayoutIpc();
  ipcMain.handle("garyx:get-state", async () => {
    return getDesktopState();
  });

  ipcMain.handle("garyx:get-state-fast", async () => {
    return getDesktopStateFast();
  });

  ipcMain.handle(
    "garyx:save-settings",
    async (_event, settings: DesktopSettings) => {
      const state = await saveDesktopSettings(settings);
      if (mainWindow && !mainWindow.isDestroyed()) {
        restartThreadEventForwarders();
      }
      return state;
    },
  );

  ipcMain.handle("garyx:remember-gateway-profile", async () => {
    const state = await rememberDesktopGatewayProfile();
    if (mainWindow && !mainWindow.isDestroyed()) {
      restartThreadEventForwarders();
    }
    return state;
  });

  ipcMain.handle(
    "garyx:add-gateway-profile",
    async (
      _event,
      input: {
        label?: string;
        gatewayUrl?: string;
        gatewayAuthToken?: string;
        gatewayHeaders?: string;
      },
    ) => {
      return addDesktopGatewayProfile({
        label: typeof input?.label === "string" ? input.label : "",
        gatewayUrl: String(input?.gatewayUrl || ""),
        gatewayAuthToken: typeof input?.gatewayAuthToken === "string"
          ? input.gatewayAuthToken
          : "",
        gatewayHeaders: typeof input?.gatewayHeaders === "string"
          ? input.gatewayHeaders
          : "",
      });
    },
  );

  ipcMain.handle(
    "garyx:update-gateway-profile",
    async (
      _event,
      input: {
        profileId?: string;
        label?: string;
        gatewayUrl?: string;
        gatewayAuthToken?: string;
        gatewayHeaders?: string;
      },
    ) => {
      const state = await updateDesktopGatewayProfile({
        profileId: String(input?.profileId || ""),
        label: typeof input?.label === "string" ? input.label : undefined,
        gatewayUrl: String(input?.gatewayUrl || ""),
        gatewayAuthToken: typeof input?.gatewayAuthToken === "string"
          ? input.gatewayAuthToken
          : undefined,
        gatewayHeaders: typeof input?.gatewayHeaders === "string"
          ? input.gatewayHeaders
          : undefined,
      });
      if (mainWindow && !mainWindow.isDestroyed()) {
        restartThreadEventForwarders();
      }
      return state;
    },
  );

  ipcMain.handle(
    "garyx:delete-gateway-profile",
    async (_event, input: { profileId?: string }) => {
      return deleteDesktopGatewayProfile(String(input?.profileId || ""));
    },
  );

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
  // endpoints.
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
    async (
      _event,
      config: GatewayConfigDocument,
      options?: GatewaySettingsSaveRequestOptions,
    ) => {
      const settings = await resolveSettings();
      return saveGatewaySettings(settings, config, options);
    },
  );

  ipcMain.handle("garyx:add-channel-account", async (_event, input) => {
    const settings = await resolveSettings();
    await addChannelAccount(settings, input);
    return getDesktopState();
  });

  ipcMain.handle(
    "garyx:select-workspace",
    async (_event, input: SelectWorkspaceInput) => {
      return selectDesktopWorkspace(input.workspacePath);
    },
  );

  ipcMain.handle(
    "garyx:list-workspace-directories",
    async (_event, input?: { path?: string | null }) => {
      const settings = await resolveSettings();
      return listWorkspaceDirectories(settings, input);
    },
  );

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

  ipcMain.handle("garyx:list-tasks", async (_event, input?: ListTasksInput) => {
    const settings = await resolveSettings();
    return listTasks(settings, input || {});
  });

  ipcMain.handle(
    "garyx:list-task-forest",
    async (_event, input?: ListTaskForestInput) => {
      const settings = await resolveSettings();
      return listTaskForest(settings, input || {});
    },
  );

  ipcMain.handle("garyx:get-task", async (_event, input: GetTaskInput) => {
    const settings = await resolveSettings();
    return getTask(settings, input);
  });

  ipcMain.handle("garyx:list-capsules", async () => {
    const settings = await resolveSettings();
    return listCapsules(settings);
  });

  ipcMain.handle("garyx:get-capsule", async (_event, capsuleId: string) => {
    const settings = await resolveSettings();
    return getCapsule(settings, capsuleId);
  });

  ipcMain.handle("garyx:get-capsule-html", async (_event, capsuleId: string) => {
    const settings = await resolveSettings();
    return getCapsuleHtml(settings, capsuleId);
  });

  ipcMain.handle(
    "garyx:get-capsule-thumbnail",
    async (
      _event,
      capsuleId: string,
      revision: number,
      rendition: { aspectWidth: number; aspectHeight: number },
    ) => {
      const settings = await resolveSettings();
      return renderCapsuleThumbnail(settings, capsuleId, revision, rendition);
    },
  );

  ipcMain.handle(
    "garyx:set-capsule-favorite",
    async (_event, input: SetCapsuleFavoriteInput) => {
      const settings = await resolveSettings();
      return setCapsuleFavorite(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:delete-capsule",
    async (_event, input: DeleteCapsuleInput) => {
      const settings = await resolveSettings();
      await deleteCapsule(settings, input);
      // Drop the rendered-thumbnail cache for this capsule so a re-created id
      // can never serve a stale crop.
      await evictCapsuleThumbnails(input.capsuleId);
    },
  );

  ipcMain.handle(
    "garyx:create-task",
    async (_event, input: CreateTaskInput) => {
      const settings = await resolveSettings();
      return createTask(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:list-provider-recent-sessions",
    async (_event, input?: ListProviderRecentSessionsInput) => {
      const settings = await resolveSettings();
      return listProviderRecentSessions(settings, input || {});
    },
  );

  ipcMain.handle(
    "garyx:update-task-status",
    async (_event, input: UpdateTaskStatusInput) => {
      const settings = await resolveSettings();
      return updateTaskStatus(settings, input);
    },
  );

  ipcMain.handle("garyx:assign-task", async (_event, input: AssignTaskInput) => {
    const settings = await resolveSettings();
    return assignTask(settings, input);
  });

  ipcMain.handle(
    "garyx:unassign-task",
    async (_event, input: UnassignTaskInput) => {
      const settings = await resolveSettings();
      return unassignTask(settings, input);
    },
  );

  ipcMain.handle("garyx:stop-task", async (_event, input: StopTaskInput) => {
    const settings = await resolveSettings();
    return stopTask(settings, input);
  });

  ipcMain.handle("garyx:delete-task", async (_event, input: DeleteTaskInput) => {
    const settings = await resolveSettings();
    return deleteTask(settings, input);
  });

  ipcMain.handle(
    "garyx:update-task-title",
    async (_event, input: UpdateTaskTitleInput) => {
      const settings = await resolveSettings();
      return updateTaskTitle(settings, input);
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
    "garyx:list-provider-models",
    async (_event, providerType: DesktopApiProviderType) => {
      const settings = await resolveSettings();
      return listProviderModels(settings, providerType);
    },
  );

  ipcMain.handle("garyx:get-coding-usage", async () => {
    const settings = await resolveSettings();
    return getCodingUsage(settings);
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

  ipcMain.handle(
    "garyx:generate-custom-agent-avatar",
    async (_event, input: GenerateCustomAgentAvatarInput) => {
      const settings = await resolveSettings();
      return generateCustomAgentAvatar(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:cancel-custom-agent-avatar-generation",
    (_event, input: CancelCustomAgentAvatarInput) => (
      cancelCustomAgentAvatarGeneration(input.requestId)
    ),
  );

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
    "garyx:get-workspace-git-status",
    async (_event, input: { workspacePath: string }) => {
      const settings = await resolveSettings();
      return getWorkspaceGitStatus(settings, input);
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
    "garyx:update-thread-runtime-settings",
    async (_event, input: UpdateThreadRuntimeSettingsInput) => {
      const settings = await resolveSettings();
      const threadId = input.threadId || "";
      const patch: {
        model?: string | null;
        modelReasoningEffort?: string | null;
        modelServiceTier?: string | null;
      } = {};
      if (Object.prototype.hasOwnProperty.call(input, "model")) {
        patch.model = input.model;
      }
      if (
        Object.prototype.hasOwnProperty.call(input, "modelReasoningEffort")
      ) {
        patch.modelReasoningEffort = input.modelReasoningEffort;
      }
      if (Object.prototype.hasOwnProperty.call(input, "modelServiceTier")) {
        patch.modelServiceTier = input.modelServiceTier;
      }
      await updateRemoteThread(settings, threadId, patch);
      return fetchThreadHistory(settings, { threadId });
    },
  );

  ipcMain.handle(
    "garyx:list-recent-threads",
    async (_event, rawInput: unknown) => {
      const input = validateListRecentThreadsInput(rawInput);
      const settings = await resolveSettings();
      assertRecentThreadGatewayScope(settings, input.gatewayScope);
      return fetchRecentThreads(settings, input);
    },
  );

  ipcMain.handle(
    "garyx:archive-thread",
    async (_event, input: ArchiveThreadInput) => {
      return archiveDesktopThread({
        threadId: input.threadId,
        endpointKeys: input.endpointKeys || [],
      });
    },
  );

  ipcMain.handle(
    "garyx:delete-thread",
    async (_event, input: DeleteThreadInput) => {
      return deleteDesktopThread(input.threadId || input.sessionId || "");
    },
  );

  ipcMain.handle(
    "garyx:set-thread-pinned",
    async (_event, input: { threadId: string; pinned: boolean }) => {
      return setDesktopThreadPinned(input);
    },
  );

  ipcMain.handle(
    "garyx:set-thread-pin-order",
    async (_event, input: { threadIds?: string[] }) => {
      return setDesktopThreadPinOrder(
        Array.isArray(input?.threadIds) ? input.threadIds : [],
      );
    },
  );

  ipcMain.handle(
    "garyx:get-thread-history",
    async (_event, input: string | GetThreadHistoryInput) => {
      const settings = await resolveSettings();
      return fetchThreadHistory(settings, input);
    },
  );
  ipcMain.handle(
    "garyx:load-thread-transcript-cache",
    async (_event, threadId: string) => {
      return loadThreadTranscriptCache(threadId || "");
    },
  );
  ipcMain.handle(
    "garyx:save-thread-transcript-cache",
    async (
      _event,
      transcript: ThreadTranscript,
      renderState?: RenderState | null,
    ) => {
      await saveThreadTranscriptCache(transcript, renderState ?? null);
    },
  );
  ipcMain.handle(
    "garyx:clear-thread-transcript-cache",
    async (_event, threadId: string) => {
      await clearThreadTranscriptCache(threadId || "");
    },
  );
  ipcMain.handle(
    "garyx:start-thread-stream",
    async (_event, input: StartThreadStreamInput) => {
      startThreadEventForwarder(input);
    },
  );
  ipcMain.handle(
    "garyx:stop-thread-stream",
    async (_event, input?: StopThreadStreamInput) => {
      stopThreadEventForwarder(input);
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
      const result = await openChatStream(settings, input);
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
      input?: { gatewayUrl?: string; gatewayAuthToken?: string; gatewayHeaders?: string },
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
            gatewayHeaders:
              typeof input.gatewayHeaders === "string"
                ? input.gatewayHeaders
                : settings.gatewayHeaders,
          }
        : settings;
      return checkConnection(nextSettings);
    },
  );

  ipcMain.handle(
    "garyx:probe-gateway",
    async (
      _event,
      input: { gatewayUrl: string; gatewayAuthToken: string; gatewayHeaders?: string },
    ) => {
      return probeGateway(input);
    },
  );

  ipcMain.handle("garyx:list-browser-state", async () => {
    return listBrowserState();
  });
  ipcMain.handle("garyx:get-workspace-git-details", getWorkspaceGitDetails);
  ipcMain.handle("garyx:commit-workspace-changes", commitWorkspaceChanges);
  ipcMain.handle("garyx:push-workspace-branch", pushWorkspaceBranch);
  ipcMain.handle("garyx:create-browser-tab", createBrowserTab);
  ipcMain.handle("garyx:activate-browser-tab", activateBrowserTab);
  ipcMain.handle("garyx:close-browser-tab", closeBrowserTab);
  ipcMain.handle("garyx:navigate-browser-tab", navigateBrowserTab);
  ipcMain.handle("garyx:browser-go-back", browserGoBack);
  ipcMain.handle("garyx:browser-go-forward", browserGoForward);
  ipcMain.handle("garyx:browser-reload", browserReload);
  ipcMain.handle("garyx:browser-open-external", browserOpenExternal);
  ipcMain.handle("garyx:capture-browser-tab", captureBrowserTab);
  ipcMain.handle("garyx:set-browser-annotation-mode", setBrowserAnnotationMode);
  ipcMain.handle("garyx:copy-image-to-clipboard", copyImageToClipboard);
  ipcMain.handle("garyx:copy-text-to-clipboard", (_event, input: CopyTextToClipboardInput) => {
    clipboard.writeText(typeof input.text === "string" ? input.text : "");
  });
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
  ipcMain.on("garyx:browser-annotation-comment-subscribe", (event) => {
    subscribeBrowserAnnotationComments(event);
  });
  ipcMain.on("garyx:browser-annotation-comment-unsubscribe", (event) => {
    unsubscribeBrowserAnnotationComments(event);
  });
  ipcMain.on("garyx:browser-page-mouse-down-subscribe", (event) => {
    subscribeBrowserPageMouseDown(event);
  });
  ipcMain.on("garyx:browser-page-mouse-down-unsubscribe", (event) => {
    unsubscribeBrowserPageMouseDown(event);
  });
  ipcMain.handle("garyx:list-terminal-state", async () => {
    return listTerminalState();
  });
  ipcMain.handle("garyx:create-terminal-session", createTerminalSession);
  ipcMain.handle("garyx:activate-terminal-session", activateTerminalSession);
  ipcMain.handle("garyx:close-terminal-session", closeTerminalSession);
  ipcMain.handle("garyx:write-terminal-input", writeTerminalInput);
  ipcMain.handle("garyx:resize-terminal-session", resizeTerminalSession);
  ipcMain.on("garyx:terminal-event-subscribe", (event) => {
    const state = subscribeTerminalState(event);
    event.sender.send("garyx:terminal-event", {
      type: "state",
      state,
    });
  });
  ipcMain.on("garyx:terminal-event-unsubscribe", (event) => {
    unsubscribeTerminalState(event);
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

  // The renderer passes viewport coordinates (the trigger button's
  // `getBoundingClientRect().bottom + 6`). On macOS with
  // `titleBarStyle: "hiddenInset"`, calling `menu.popup({ window, x, y })`
  // visibly drops the menu about a titlebar-height (~30-40px) below the
  // intended y. Omitting `window` makes Electron use the currently focused
  // window's content area as the coordinate origin instead, which lines up
  // with the renderer's viewport reference. Don't add the window's screen
  // offset — the menu would then accumulate the offset twice, drifting
  // right by `bounds.x` (visible as "right when the window is small, left
  // when it's wide").
  //
  // Empirically the menu still lands ~90 DIPs left of the requested x in
  // this configuration — likely a leftover from Electron's macOS
  // content-area mapping. Add a fixed compensation so the menu's right
  // edge lines up with the trigger button's right edge.
  const POPUP_X_OFFSET = 96;
  menu.popup({
    x: Math.max(0, Math.round(input.x) + POPUP_X_OFFSET),
    y: Math.max(0, Math.round(input.y)),
  });
}

function createTray(): void {
  writeBootstrapTrace("createTray:start");
  const trayIconPath = app.isPackaged
    ? join(process.resourcesPath, "trayIcon.png")
    : join(__dirname, "../../resources/trayIcon.png");
  const icon = nativeImage.createFromPath(trayIconPath);
  icon.setTemplateImage(false);
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
    void pruneThreadTranscriptCache().catch(() => undefined);
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
