import { contextBridge, ipcRenderer } from "electron";

import type { GaryxDesktopApi } from "@shared/contracts";

const API_PERFORMANCE_THRESHOLD_MS = 120;
const API_PERFORMANCE_MESSAGE_SOURCE = "garyx-desktop-api-performance";

type DesktopApiMethod = (...args: unknown[]) => unknown;

function getErrorMessage(error: unknown): string | undefined {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  if (error) {
    return String(error);
  }
  return undefined;
}

function postApiPerformanceTiming(
  method: string,
  durationMs: number,
  ok: boolean,
  error?: unknown,
): void {
  if (ok && durationMs < API_PERFORMANCE_THRESHOLD_MS) {
    return;
  }

  try {
    window.postMessage(
      {
        source: API_PERFORMANCE_MESSAGE_SOURCE,
        method,
        durationMs: Number(durationMs.toFixed(2)),
        ok,
        errorMessage: getErrorMessage(error),
      },
      "*",
    );
  } catch {
    // Best-effort diagnostics only; never break the preload API surface.
  }
}

function wrapApiMethod(methodName: string, method: DesktopApiMethod): DesktopApiMethod {
  return (...args: unknown[]) => {
    const start = performance.now();
    try {
      const result = method(...args);
      if (result && typeof (result as PromiseLike<unknown>).then === "function") {
        return (result as PromiseLike<unknown>).then(
          (value) => {
            postApiPerformanceTiming(methodName, performance.now() - start, true);
            return value;
          },
          (error) => {
            postApiPerformanceTiming(methodName, performance.now() - start, false, error);
            throw error;
          },
        );
      }

      postApiPerformanceTiming(methodName, performance.now() - start, true);
      return result;
    } catch (error) {
      postApiPerformanceTiming(methodName, performance.now() - start, false, error);
      throw error;
    }
  };
}

function instrumentDesktopApi(source: GaryxDesktopApi): GaryxDesktopApi {
  const wrapped: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(source)) {
    if (
      typeof value === "function" &&
      !key.startsWith("subscribe") &&
      !key.startsWith("unsubscribe")
    ) {
      wrapped[key] = wrapApiMethod(key, value as DesktopApiMethod);
    } else {
      wrapped[key] = value;
    }
  }
  return wrapped as unknown as GaryxDesktopApi;
}

const chatStreamListeners = new Map<
  Parameters<GaryxDesktopApi["subscribeChatStream"]>[0],
  (_event: Electron.IpcRendererEvent, payload: unknown) => void
>();

const browserStateListeners = new Map<
  Parameters<GaryxDesktopApi["subscribeBrowserState"]>[0],
  (_event: Electron.IpcRendererEvent, payload: unknown) => void
>();

const browserAnnotationCommentListeners = new Map<
  Parameters<GaryxDesktopApi["subscribeBrowserAnnotationComments"]>[0],
  (_event: Electron.IpcRendererEvent, payload: unknown) => void
>();

const browserPageMouseDownListeners = new Map<
  Parameters<GaryxDesktopApi["subscribeBrowserPageMouseDown"]>[0],
  (_event: Electron.IpcRendererEvent) => void
>();

function clearBrowserAnnotationCommentListeners(): void {
  for (const wrapped of browserAnnotationCommentListeners.values()) {
    ipcRenderer.removeListener("garyx:browser-annotation-comment", wrapped);
  }
  browserAnnotationCommentListeners.clear();
}

const terminalEventListeners = new Map<
  Parameters<GaryxDesktopApi["subscribeTerminalEvents"]>[0],
  (_event: Electron.IpcRendererEvent, payload: unknown) => void
>();

const deepLinkListeners = new Map<
  Parameters<GaryxDesktopApi["subscribeDeepLinks"]>[0],
  (_event: Electron.IpcRendererEvent, payload: unknown) => void
>();

const updateStatusListeners = new Map<
  Parameters<GaryxDesktopApi["subscribeUpdateStatus"]>[0],
  (_event: Electron.IpcRendererEvent, payload: unknown) => void
>();

const api: GaryxDesktopApi = {
  getState: () => ipcRenderer.invoke("garyx:get-state"),
  saveSettings: (settings) =>
    ipcRenderer.invoke("garyx:save-settings", settings),
  rememberGatewayProfile: () =>
    ipcRenderer.invoke("garyx:remember-gateway-profile"),
  addGatewayProfile: (input) =>
    ipcRenderer.invoke("garyx:add-gateway-profile", input),
  updateGatewayProfile: (input) =>
    ipcRenderer.invoke("garyx:update-gateway-profile", input),
  deleteGatewayProfile: (input) =>
    ipcRenderer.invoke("garyx:delete-gateway-profile", input),
  getGatewaySettings: () => ipcRenderer.invoke("garyx:get-gateway-settings"),
  fetchChannelPlugins: () =>
    ipcRenderer.invoke("garyx:fetch-channel-plugins"),
  openExternalUrl: (input) =>
    ipcRenderer.invoke("garyx:open-external-url", input),
  startChannelAuthFlow: (input) =>
    ipcRenderer.invoke("garyx:start-channel-auth-flow", input),
  pollChannelAuthFlow: (input) =>
    ipcRenderer.invoke("garyx:poll-channel-auth-flow", input),
  saveGatewaySettings: (config, options) =>
    ipcRenderer.invoke("garyx:save-gateway-settings", config, options),
  selectWorkspace: (input) =>
    ipcRenderer.invoke("garyx:select-workspace", input),
  listWorkspaceDirectories: (input) =>
    ipcRenderer.invoke("garyx:list-workspace-directories", input),
  addWorkspaceByPath: (input) =>
    ipcRenderer.invoke("garyx:add-workspace-by-path", input),
  removeWorkspace: (input) =>
    ipcRenderer.invoke("garyx:remove-workspace", input),
  selectAutomation: (input) =>
    ipcRenderer.invoke("garyx:select-automation", input),
  markAutomationSeen: (input) =>
    ipcRenderer.invoke("garyx:mark-automation-seen", input),
  createAutomation: (input) =>
    ipcRenderer.invoke("garyx:create-automation", input),
  updateAutomation: (input) =>
    ipcRenderer.invoke("garyx:update-automation", input),
  deleteAutomation: (input) =>
    ipcRenderer.invoke("garyx:delete-automation", input),
  listTasks: (input) => ipcRenderer.invoke("garyx:list-tasks", input),
  listDreams: (input) => ipcRenderer.invoke("garyx:list-dreams", input),
  scanDreams: (input) => ipcRenderer.invoke("garyx:scan-dreams", input),
  getDream: (dreamId) => ipcRenderer.invoke("garyx:get-dream", dreamId),
  createTask: (input) => ipcRenderer.invoke("garyx:create-task", input),
  listWorkflowDefinitions: () =>
    ipcRenderer.invoke("garyx:list-workflow-definitions"),
  listProviderRecentSessions: (input) =>
    ipcRenderer.invoke("garyx:list-provider-recent-sessions", input),
  getWorkflowDefinitionSource: (input) =>
    ipcRenderer.invoke("garyx:get-workflow-definition-source", input),
  listTaskWorkflowRuns: (input) =>
    ipcRenderer.invoke("garyx:list-task-workflow-runs", input),
  getWorkflowRun: (input) =>
    ipcRenderer.invoke("garyx:get-workflow-run", input),
  startWorkflowThread: (input) =>
    ipcRenderer.invoke("garyx:start-workflow-thread", input),
  promoteThreadToTask: (input) =>
    ipcRenderer.invoke("garyx:promote-thread-to-task", input),
  updateTaskStatus: (input) =>
    ipcRenderer.invoke("garyx:update-task-status", input),
  assignTask: (input) => ipcRenderer.invoke("garyx:assign-task", input),
  unassignTask: (input) => ipcRenderer.invoke("garyx:unassign-task", input),
  stopTask: (input) => ipcRenderer.invoke("garyx:stop-task", input),
  deleteTask: (input) => ipcRenderer.invoke("garyx:delete-task", input),
  updateTaskTitle: (input) =>
    ipcRenderer.invoke("garyx:update-task-title", input),
  listSkills: () => ipcRenderer.invoke("garyx:list-skills"),
  listCustomAgents: () => ipcRenderer.invoke("garyx:list-custom-agents"),
  listProviderModels: (providerType) =>
    ipcRenderer.invoke("garyx:list-provider-models", providerType),
  createCustomAgent: (input) =>
    ipcRenderer.invoke("garyx:create-custom-agent", input),
  updateCustomAgent: (input) =>
    ipcRenderer.invoke("garyx:update-custom-agent", input),
  deleteCustomAgent: (input) =>
    ipcRenderer.invoke("garyx:delete-custom-agent", input),
  generateCustomAgentAvatar: (input) =>
    ipcRenderer.invoke("garyx:generate-custom-agent-avatar", input),
  listTeams: () => ipcRenderer.invoke("garyx:list-teams"),
  createTeam: (input) => ipcRenderer.invoke("garyx:create-team", input),
  updateTeam: (input) => ipcRenderer.invoke("garyx:update-team", input),
  deleteTeam: (input) => ipcRenderer.invoke("garyx:delete-team", input),
  createSkill: (input) => ipcRenderer.invoke("garyx:create-skill", input),
  updateSkill: (input) => ipcRenderer.invoke("garyx:update-skill", input),
  toggleSkill: (input) => ipcRenderer.invoke("garyx:toggle-skill", input),
  deleteSkill: (input) => ipcRenderer.invoke("garyx:delete-skill", input),
  getSkillEditor: (input) => ipcRenderer.invoke("garyx:get-skill-editor", input),
  readSkillFile: (input) => ipcRenderer.invoke("garyx:read-skill-file", input),
  saveSkillFile: (input) => ipcRenderer.invoke("garyx:save-skill-file", input),
  readMemoryDocument: (input) =>
    ipcRenderer.invoke("garyx:read-memory-document", input),
  saveMemoryDocument: (input) =>
    ipcRenderer.invoke("garyx:save-memory-document", input),
  listWorkspaceFiles: (input) =>
    ipcRenderer.invoke("garyx:list-workspace-files", input),
  previewWorkspaceFile: (input) =>
    ipcRenderer.invoke("garyx:preview-workspace-file", input),
  revealWorkspaceFile: (input) =>
    ipcRenderer.invoke("garyx:reveal-workspace-file", input),
  uploadChatAttachments: (input) =>
    ipcRenderer.invoke("garyx:upload-chat-attachments", input),
  uploadWorkspaceFiles: (input) =>
    ipcRenderer.invoke("garyx:upload-workspace-files", input),
  createSkillEntry: (input) =>
    ipcRenderer.invoke("garyx:create-skill-entry", input),
  deleteSkillEntry: (input) =>
    ipcRenderer.invoke("garyx:delete-skill-entry", input),
  listSlashCommands: () => ipcRenderer.invoke("garyx:list-slash-commands"),
  createSlashCommand: (input) =>
    ipcRenderer.invoke("garyx:create-slash-command", input),
  updateSlashCommand: (input) =>
    ipcRenderer.invoke("garyx:update-slash-command", input),
  deleteSlashCommand: (input) =>
    ipcRenderer.invoke("garyx:delete-slash-command", input),
  listAutoResearchRuns: (input) =>
    ipcRenderer.invoke("garyx:list-auto-research-runs", input),
  createAutoResearchRun: (input) =>
    ipcRenderer.invoke("garyx:create-auto-research-run", input),
  getAutoResearchRun: (runId) =>
    ipcRenderer.invoke("garyx:get-auto-research-run", runId),
  listAutoResearchIterations: (runId) =>
    ipcRenderer.invoke("garyx:list-auto-research-iterations", runId),
  stopAutoResearchRun: (input) =>
    ipcRenderer.invoke("garyx:stop-auto-research-run", input),
  deleteAutoResearchRun: (runId: string) =>
    ipcRenderer.invoke("garyx:delete-auto-research-run", runId),
  listAutoResearchCandidates: (input) =>
    ipcRenderer.invoke("garyx:list-auto-research-candidates", input),
  selectAutoResearchCandidate: (input) =>
    ipcRenderer.invoke("garyx:select-auto-research-candidate", input),
  listMcpServers: () => ipcRenderer.invoke("garyx:list-mcp-servers"),
  createMcpServer: (input) =>
    ipcRenderer.invoke("garyx:create-mcp-server", input),
  updateMcpServer: (input) =>
    ipcRenderer.invoke("garyx:update-mcp-server", input),
  deleteMcpServer: (input) =>
    ipcRenderer.invoke("garyx:delete-mcp-server", input),
  toggleMcpServer: (input) =>
    ipcRenderer.invoke("garyx:toggle-mcp-server", input),
  getAutomationActivity: (automationId) =>
    ipcRenderer.invoke("garyx:get-automation-activity", automationId),
  runAutomationNow: (input) =>
    ipcRenderer.invoke("garyx:run-automation-now", input),
  addChannelAccount: (input) =>
    ipcRenderer.invoke("garyx:add-channel-account", input),
  startWeixinChannelAuth: (input) =>
    ipcRenderer.invoke("garyx:start-weixin-channel-auth", input),
  pollWeixinChannelAuth: (input) =>
    ipcRenderer.invoke("garyx:poll-weixin-channel-auth", input),
  startFeishuChannelAuth: (input) =>
    ipcRenderer.invoke("garyx:start-feishu-channel-auth", input),
  pollFeishuChannelAuth: (input) =>
    ipcRenderer.invoke("garyx:poll-feishu-channel-auth", input),
  setBotBinding: (input) => ipcRenderer.invoke("garyx:set-bot-binding", input),
  listChannelEndpoints: () => ipcRenderer.invoke("garyx:list-channel-endpoints"),
  bindChannelEndpoint: (input) =>
    ipcRenderer.invoke("garyx:bind-channel-endpoint", input),
  detachChannelEndpoint: (input) =>
    ipcRenderer.invoke("garyx:detach-channel-endpoint", input),
  createThread: (input) => ipcRenderer.invoke("garyx:create-thread", input),
  getWorkspaceGitStatus: (input) =>
    ipcRenderer.invoke("garyx:get-workspace-git-status", input),
  getWorkspaceGitDetails: (input) =>
    ipcRenderer.invoke("garyx:get-workspace-git-details", input),
  commitWorkspaceChanges: (input) =>
    ipcRenderer.invoke("garyx:commit-workspace-changes", input),
  pushWorkspaceBranch: (input) =>
    ipcRenderer.invoke("garyx:push-workspace-branch", input),
  renameThread: (input) => ipcRenderer.invoke("garyx:rename-thread", input),
  deleteThread: (input) => ipcRenderer.invoke("garyx:delete-thread", input),
  setThreadPinned: (input) =>
    ipcRenderer.invoke("garyx:set-thread-pinned", input),
  getThreadHistory: (input) =>
    ipcRenderer.invoke("garyx:get-thread-history", input),
  getThreadLogs: (threadId, cursor) =>
    ipcRenderer.invoke("garyx:get-thread-logs", { threadId, cursor }),
  openChatStream: (input) => ipcRenderer.invoke("garyx:open-chat-stream", input),
  sendStreamingInput: (input) =>
    ipcRenderer.invoke("garyx:send-streaming-input", input),
  subscribeChatStream: (listener) => {
    const wrapped = (_event: Electron.IpcRendererEvent, payload: unknown) => {
      listener(payload as Parameters<typeof listener>[0]);
    };
    chatStreamListeners.set(listener, wrapped);
    ipcRenderer.on("garyx:chat-stream", wrapped);
  },
  unsubscribeChatStream: (listener) => {
    const wrapped = chatStreamListeners.get(listener);
    if (!wrapped) {
      return;
    }
    ipcRenderer.removeListener("garyx:chat-stream", wrapped);
    chatStreamListeners.delete(listener);
  },
  subscribeDeepLinks: (listener) => {
    const wrapped = (_event: Electron.IpcRendererEvent, payload: unknown) => {
      listener(payload as Parameters<typeof listener>[0]);
    };
    const hadListeners = deepLinkListeners.size > 0;
    deepLinkListeners.set(listener, wrapped);
    ipcRenderer.on("garyx:deep-link", wrapped);
    if (!hadListeners) {
      ipcRenderer.send("garyx:deep-link-subscribe");
    }
  },
  unsubscribeDeepLinks: (listener) => {
    const wrapped = deepLinkListeners.get(listener);
    if (!wrapped) {
      return;
    }
    ipcRenderer.removeListener("garyx:deep-link", wrapped);
    deepLinkListeners.delete(listener);
    if (deepLinkListeners.size === 0) {
      ipcRenderer.send("garyx:deep-link-unsubscribe");
    }
  },
  interruptThread: (threadId) =>
    ipcRenderer.invoke("garyx:interrupt-thread", threadId),
  checkConnection: (input) => ipcRenderer.invoke("garyx:check-connection", input),
  probeGateway: (input) => ipcRenderer.invoke("garyx:probe-gateway", input),
  listBrowserState: () => ipcRenderer.invoke("garyx:list-browser-state"),
  createBrowserTab: (input) =>
    ipcRenderer.invoke("garyx:create-browser-tab", input),
  activateBrowserTab: (tabId) =>
    ipcRenderer.invoke("garyx:activate-browser-tab", tabId),
  closeBrowserTab: (tabId) =>
    ipcRenderer.invoke("garyx:close-browser-tab", tabId),
  navigateBrowserTab: (input) =>
    ipcRenderer.invoke("garyx:navigate-browser-tab", input),
  browserGoBack: (tabId) => ipcRenderer.invoke("garyx:browser-go-back", tabId),
  browserGoForward: (tabId) =>
    ipcRenderer.invoke("garyx:browser-go-forward", tabId),
  browserReload: (tabId) => ipcRenderer.invoke("garyx:browser-reload", tabId),
  browserOpenExternal: (tabId) =>
    ipcRenderer.invoke("garyx:browser-open-external", tabId),
  captureBrowserTab: (input) =>
    ipcRenderer.invoke("garyx:capture-browser-tab", input),
  setBrowserAnnotationMode: (input) =>
    ipcRenderer.invoke("garyx:set-browser-annotation-mode", input),
  copyImageToClipboard: (input) =>
    ipcRenderer.invoke("garyx:copy-image-to-clipboard", input),
  copyTextToClipboard: (input) =>
    ipcRenderer.invoke("garyx:copy-text-to-clipboard", input),
  updateBrowserBounds: (input) =>
    ipcRenderer.invoke("garyx:update-browser-bounds", input),
  setBrowserOverlayPaused: (paused) =>
    ipcRenderer.invoke("garyx:set-browser-overlay-paused", paused),
  showBrowserConnectionMenu: (input) =>
    ipcRenderer.invoke("garyx:show-browser-connection-menu", input),
  subscribeBrowserState: (listener) => {
    const wrapped = (_event: Electron.IpcRendererEvent, payload: unknown) => {
      listener(payload as Parameters<typeof listener>[0]);
    };
    browserStateListeners.set(listener, wrapped);
    ipcRenderer.on("garyx:browser-state", wrapped);
    ipcRenderer.send("garyx:browser-state-subscribe");
  },
  unsubscribeBrowserState: (listener) => {
    const wrapped = browserStateListeners.get(listener);
    if (!wrapped) {
      return;
    }
    ipcRenderer.removeListener("garyx:browser-state", wrapped);
    browserStateListeners.delete(listener);
    if (browserStateListeners.size === 0) {
      ipcRenderer.send("garyx:browser-state-unsubscribe");
    }
  },
  subscribeBrowserAnnotationComments: (listener) => {
    clearBrowserAnnotationCommentListeners();
    const wrapped = (_event: Electron.IpcRendererEvent, payload: unknown) => {
      listener(payload as Parameters<typeof listener>[0]);
    };
    browserAnnotationCommentListeners.set(listener, wrapped);
    ipcRenderer.on("garyx:browser-annotation-comment", wrapped);
    ipcRenderer.send("garyx:browser-annotation-comment-subscribe");
  },
  unsubscribeBrowserAnnotationComments: (listener) => {
    const wrapped = browserAnnotationCommentListeners.get(listener);
    if (!wrapped) {
      clearBrowserAnnotationCommentListeners();
      ipcRenderer.send("garyx:browser-annotation-comment-unsubscribe");
      return;
    }
    ipcRenderer.removeListener("garyx:browser-annotation-comment", wrapped);
    browserAnnotationCommentListeners.delete(listener);
    if (browserAnnotationCommentListeners.size === 0) {
      ipcRenderer.send("garyx:browser-annotation-comment-unsubscribe");
    }
  },
  subscribeBrowserPageMouseDown: (listener) => {
    const wrapped = () => {
      listener();
    };
    browserPageMouseDownListeners.set(listener, wrapped);
    ipcRenderer.on("garyx:browser-page-mouse-down", wrapped);
    ipcRenderer.send("garyx:browser-page-mouse-down-subscribe");
  },
  unsubscribeBrowserPageMouseDown: (listener) => {
    const wrapped = browserPageMouseDownListeners.get(listener);
    if (!wrapped) {
      return;
    }
    ipcRenderer.removeListener("garyx:browser-page-mouse-down", wrapped);
    browserPageMouseDownListeners.delete(listener);
    if (browserPageMouseDownListeners.size === 0) {
      ipcRenderer.send("garyx:browser-page-mouse-down-unsubscribe");
    }
  },
  listTerminalState: () => ipcRenderer.invoke("garyx:list-terminal-state"),
  createTerminalSession: (input) =>
    ipcRenderer.invoke("garyx:create-terminal-session", input),
  activateTerminalSession: (input) =>
    ipcRenderer.invoke("garyx:activate-terminal-session", input),
  closeTerminalSession: (input) =>
    ipcRenderer.invoke("garyx:close-terminal-session", input),
  writeTerminalInput: (input) =>
    ipcRenderer.invoke("garyx:write-terminal-input", input),
  resizeTerminalSession: (input) =>
    ipcRenderer.invoke("garyx:resize-terminal-session", input),
  subscribeTerminalEvents: (listener) => {
    const wrapped = (_event: Electron.IpcRendererEvent, payload: unknown) => {
      listener(payload as Parameters<typeof listener>[0]);
    };
    const hadListeners = terminalEventListeners.size > 0;
    terminalEventListeners.set(listener, wrapped);
    ipcRenderer.on("garyx:terminal-event", wrapped);
    if (!hadListeners) {
      ipcRenderer.send("garyx:terminal-event-subscribe");
    }
  },
  unsubscribeTerminalEvents: (listener) => {
    const wrapped = terminalEventListeners.get(listener);
    if (!wrapped) {
      return;
    }
    ipcRenderer.removeListener("garyx:terminal-event", wrapped);
    terminalEventListeners.delete(listener);
    if (terminalEventListeners.size === 0) {
      ipcRenderer.send("garyx:terminal-event-unsubscribe");
    }
  },
  getAppVersion: () => ipcRenderer.invoke("garyx:get-app-version"),
  getUpdateStatus: () => ipcRenderer.invoke("garyx:get-update-status"),
  checkForUpdatesNow: () => ipcRenderer.invoke("garyx:check-for-updates-now"),
  installUpdate: () => ipcRenderer.invoke("garyx:install-update"),
  subscribeUpdateStatus: (listener) => {
    const wrapped = (_event: Electron.IpcRendererEvent, payload: unknown) => {
      listener(payload as Parameters<typeof listener>[0]);
    };
    updateStatusListeners.set(listener, wrapped);
    ipcRenderer.on("garyx:update-status", wrapped);
  },
  unsubscribeUpdateStatus: (listener) => {
    const wrapped = updateStatusListeners.get(listener);
    if (!wrapped) {
      return;
    }
    ipcRenderer.removeListener("garyx:update-status", wrapped);
    updateStatusListeners.delete(listener);
  },
};

contextBridge.exposeInMainWorld("garyxDesktop", instrumentDesktopApi(api));
