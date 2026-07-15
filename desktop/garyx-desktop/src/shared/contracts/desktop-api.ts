import type {
  CancelCustomAgentAvatarInput,
  CreateCustomAgentInput,
  DeleteCustomAgentInput,
  DesktopCustomAgent,
  GenerateCustomAgentAvatarInput,
  GenerateCustomAgentAvatarResult,
  UpdateCustomAgentInput,
} from "./agent.ts";
import type {
  CreateAutomationInput,
  DeleteAutomationInput,
  DesktopAutomationActivityEntry,
  DesktopAutomationActivityFeed,
  DesktopAutomationSummary,
  MarkAutomationSeenInput,
  RunAutomationNowInput,
  SelectAutomationInput,
  UpdateAutomationInput,
} from "./automation.ts";
import type { SetBotBindingInput } from "./bot.ts";
import type {
  BrowserAnnotationModeInput,
  BrowserBoundsInput,
  CaptureBrowserTabInput,
  CaptureBrowserTabResult,
  CopyImageToClipboardInput,
  CopyTextToClipboardInput,
  CreateBrowserTabInput,
  CreateTerminalSessionInput,
  DesktopBrowserAnnotationCommentListener,
  DesktopBrowserPageMouseDownListener,
  DesktopBrowserState,
  DesktopBrowserStateListener,
  DesktopTerminalEventListener,
  DesktopTerminalState,
  NavigateBrowserTabInput,
  ShowBrowserConnectionMenuInput,
  TerminalResizeInput,
  TerminalSessionInput,
  TerminalWriteInput,
} from "./browser-terminal.ts";
import type {
  CreateSkillEntryInput,
  CreateSkillInput,
  DeleteMcpServerInput,
  DeleteSkillEntryInput,
  DeleteSkillInput,
  DeleteSlashCommandInput,
  DesktopMcpServer,
  DesktopMemoryDocument,
  DesktopSkillEditorState,
  DesktopSkillFileDocument,
  DesktopSkillInfo,
  GetSkillEditorInput,
  ReadMemoryDocumentInput,
  ReadSkillFileInput,
  SaveMemoryDocumentInput,
  SaveSkillFileInput,
  SlashCommand,
  ToggleMcpServerInput,
  ToggleSkillInput,
  UpdateMcpServerInput,
  UpdateSkillInput,
  UpdateSlashCommandInput,
  UpsertMcpServerInput,
  UpsertSlashCommandInput,
} from "./catalog.ts";
import type {
  AddChannelAccountInput,
  BindChannelEndpointInput,
  ChannelPluginCatalogEntry,
  DesktopChannelEndpoint,
  DetachChannelEndpointInput,
} from "./channel.ts";
import type {
  DeleteCapsuleInput,
  DesktopCapsuleHtmlResult,
  DesktopCapsuleSummary,
  DesktopCapsuleThumbnailResult,
  DesktopCapsulesPage,
  SetCapsuleFavoriteInput,
  SetCapsuleFavoriteResult,
} from "./capsule.ts";
import type {
  DesktopApiProviderType,
  DesktopCodingUsage,
  DesktopProviderModels,
  DesktopProviderRecentSession,
  ListProviderRecentSessionsInput,
} from "./provider.ts";
import type {
  ConnectionStatus,
  DesktopSettings,
  GatewayConfigDocument,
  GatewayProbeResult,
  GatewaySettingsPayload,
  GatewaySettingsSaveRequestOptions,
  GatewaySettingsSaveResult,
} from "./settings.ts";
import type { DesktopState, WorkspaceMutationResult } from "./state.ts";
import type {
  AssignTaskInput,
  CreateTaskInput,
  DeleteTaskInput,
  DesktopTaskForestPage,
  DesktopTaskSummary,
  DesktopTasksPage,
  GetTaskInput,
  ListTaskForestInput,
  ListTasksInput,
  StopTaskInput,
  UnassignTaskInput,
  UpdateTaskStatusInput,
  UpdateTaskTitleInput,
} from "./task.ts";
import type {
  ArchiveThreadInput,
  CachedThreadTranscript,
  CreateThreadInput,
  DeleteThreadInput,
  DesktopDeepLinkListener,
  DesktopThreadSummary,
  GetThreadHistoryInput,
  InterruptResult,
  ListRecentThreadsInput,
  OpenChatStreamResult,
  DesktopRecentThreadsPage,
  RenameThreadInput,
  SendMessageInput,
  SendStreamingInputResult,
  SetThreadPinnedInput,
  SetThreadPinOrderInput,
  StartThreadStreamInput,
  StopThreadStreamInput,
  ThreadLogChunk,
  ThreadTranscript,
  UpdateThreadRuntimeSettingsInput,
} from "./thread.ts";
import type {
  DesktopChatStreamListener,
  RenderState,
  UploadChatAttachmentsInput,
  UploadChatAttachmentsResult,
} from "./transcript.ts";
import type {
  DesktopUpdateCheckResult,
  DesktopUpdateInstallResult,
  DesktopUpdateStatus,
  DesktopUpdateStatusListener,
} from "./update.ts";
import type {
  AddWorkspaceByPathInput,
  CommitWorkspaceChangesInput,
  DesktopLocalDirectoryListing,
  DesktopWorkspaceFileListing,
  DesktopWorkspaceFilePreview,
  DesktopWorkspaceGitDetails,
  DesktopWorkspaceGitStatus,
  ListWorkspaceFilesInput,
  PreviewWorkspaceFileInput,
  PushWorkspaceBranchInput,
  RemoveWorkspaceInput,
  RevealWorkspaceFileInput,
  SelectWorkspaceInput,
  UploadWorkspaceFilesInput,
  UploadWorkspaceFilesResult,
  WorkspaceGitMutationResult,
} from "./workspace.ts";
import type {
  HorizontalLayoutPolicyName,
  WindowLayoutBootstrap,
  WindowLayoutCommand,
  WindowLayoutCommandResult,
  WindowLayoutSnapshotListener,
} from "./window-layout.ts";

export interface GaryxDesktopApi {
  horizontalLayoutPolicy: HorizontalLayoutPolicyName;
  getWindowLayoutBootstrap: (input: {
    rendererEpoch: string;
  }) => WindowLayoutBootstrap;
  executeWindowLayoutCommand: (
    command: WindowLayoutCommand,
  ) => Promise<WindowLayoutCommandResult>;
  subscribeWindowLayoutSnapshots: (
    listener: WindowLayoutSnapshotListener,
  ) => void;
  unsubscribeWindowLayoutSnapshots: (
    listener: WindowLayoutSnapshotListener,
  ) => void;
  getState: () => Promise<DesktopState>;
  /**
   * Boot-only fast hydration: the threads slice is a recent page (plus
   * by-id repair for pinned ids outside it). Callers must follow up with
   * a full getState() to restore full-set semantics.
   */
  getStateFast: () => Promise<DesktopState>;
  saveSettings: (settings: DesktopSettings) => Promise<DesktopState>;
  rememberGatewayProfile: () => Promise<DesktopState>;
  addGatewayProfile: (input: {
    label?: string;
    gatewayUrl: string;
    gatewayAuthToken?: string;
    gatewayHeaders?: string;
  }) => Promise<DesktopState>;
  updateGatewayProfile: (input: {
    profileId: string;
    label?: string;
    gatewayUrl: string;
    gatewayAuthToken?: string;
    gatewayHeaders?: string;
  }) => Promise<DesktopState>;
  deleteGatewayProfile: (input: { profileId: string }) => Promise<DesktopState>;
  getGatewaySettings: () => Promise<GatewaySettingsPayload>;
  fetchChannelPlugins: () => Promise<ChannelPluginCatalogEntry[]>;
  openExternalUrl: (input: { url: string }) => Promise<void>;
  /**
   * Start a channel-blind auto-login flow against the gateway. The
   * renderer supplies the plugin id (canonical or alias) and the
   * current form state; the plugin decides internally what "auto
   * login" means. Returns the initial `AuthSession` with a display
   * list to render and a poll cadence.
   */
  startChannelAuthFlow: (input: {
    pluginId: string;
    formState?: Record<string, unknown>;
  }) => Promise<{
    sessionId: string;
    display: Array<{ kind: string; value?: string }>;
    expiresInSecs: number;
    pollIntervalSecs: number;
  }>;
  /**
   * Advance a running auth-flow session by one tick. Returns the
   * raw 3-state poll result — `pending` / `confirmed` / `failed` —
   * plus optional display refresh and backoff hint.
   */
  pollChannelAuthFlow: (input: {
    pluginId: string;
    sessionId: string;
  }) => Promise<{
    status: "pending" | "confirmed" | "failed" | string;
    display?: Array<{ kind: string; value?: string }>;
    next_interval_secs?: number;
    values?: Record<string, unknown>;
    reason?: string;
  }>;
  saveGatewaySettings: (
    config: GatewayConfigDocument,
    options?: GatewaySettingsSaveRequestOptions,
  ) => Promise<GatewaySettingsSaveResult>;
  selectWorkspace: (input: SelectWorkspaceInput) => Promise<DesktopState>;
  listWorkspaceDirectories: (input?: {
    path?: string | null;
  }) => Promise<DesktopLocalDirectoryListing>;
  addWorkspaceByPath: (
    input: AddWorkspaceByPathInput,
  ) => Promise<WorkspaceMutationResult>;
  removeWorkspace: (input: RemoveWorkspaceInput) => Promise<DesktopState>;
  selectAutomation: (input: SelectAutomationInput) => Promise<DesktopState>;
  markAutomationSeen: (input: MarkAutomationSeenInput) => Promise<DesktopState>;
  createAutomation: (
    input: CreateAutomationInput,
  ) => Promise<{ state: DesktopState; automation: DesktopAutomationSummary }>;
  updateAutomation: (
    input: UpdateAutomationInput,
  ) => Promise<{ state: DesktopState; automation: DesktopAutomationSummary }>;
  deleteAutomation: (input: DeleteAutomationInput) => Promise<DesktopState>;
  listTasks: (input?: ListTasksInput) => Promise<DesktopTasksPage>;
  listTaskForest: (
    input?: ListTaskForestInput,
  ) => Promise<DesktopTaskForestPage>;
  getTask: (input: GetTaskInput) => Promise<DesktopTaskSummary>;
  createTask: (input: CreateTaskInput) => Promise<DesktopTaskSummary>;
  getWorkspaceGitStatus: (input: {
    workspacePath: string;
  }) => Promise<DesktopWorkspaceGitStatus>;
  getWorkspaceGitDetails: (input: {
    workspacePath: string;
  }) => Promise<DesktopWorkspaceGitDetails>;
  commitWorkspaceChanges: (
    input: CommitWorkspaceChangesInput,
  ) => Promise<WorkspaceGitMutationResult>;
  pushWorkspaceBranch: (
    input: PushWorkspaceBranchInput,
  ) => Promise<WorkspaceGitMutationResult>;
  updateTaskStatus: (input: UpdateTaskStatusInput) => Promise<void>;
  assignTask: (input: AssignTaskInput) => Promise<void>;
  unassignTask: (input: UnassignTaskInput) => Promise<void>;
  stopTask: (input: StopTaskInput) => Promise<void>;
  deleteTask: (input: DeleteTaskInput) => Promise<void>;
  updateTaskTitle: (input: UpdateTaskTitleInput) => Promise<void>;
  listCapsules: () => Promise<DesktopCapsulesPage>;
  getCapsule: (capsuleId: string) => Promise<DesktopCapsuleSummary | null>;
  getCapsuleHtml: (capsuleId: string) => Promise<DesktopCapsuleHtmlResult>;
  getCapsuleThumbnail: (
    capsuleId: string,
    revision: number,
    rendition: { aspectWidth: number; aspectHeight: number },
  ) => Promise<DesktopCapsuleThumbnailResult>;
  deleteCapsule: (input: DeleteCapsuleInput) => Promise<void>;
  setCapsuleFavorite: (
    input: SetCapsuleFavoriteInput,
  ) => Promise<SetCapsuleFavoriteResult>;
  listSkills: () => Promise<DesktopSkillInfo[]>;
  listCustomAgents: () => Promise<DesktopCustomAgent[]>;
  listProviderModels: (
    providerType: DesktopApiProviderType,
  ) => Promise<DesktopProviderModels>;
  getCodingUsage: () => Promise<DesktopCodingUsage>;
  createCustomAgent: (
    input: CreateCustomAgentInput,
  ) => Promise<DesktopCustomAgent>;
  updateCustomAgent: (
    input: UpdateCustomAgentInput,
  ) => Promise<DesktopCustomAgent>;
  deleteCustomAgent: (input: DeleteCustomAgentInput) => Promise<void>;
  generateCustomAgentAvatar: (
    input: GenerateCustomAgentAvatarInput,
  ) => Promise<GenerateCustomAgentAvatarResult>;
  cancelCustomAgentAvatarGeneration: (
    input: CancelCustomAgentAvatarInput,
  ) => Promise<boolean>;
  createSkill: (input: CreateSkillInput) => Promise<DesktopSkillInfo>;
  updateSkill: (input: UpdateSkillInput) => Promise<DesktopSkillInfo>;
  toggleSkill: (input: ToggleSkillInput) => Promise<DesktopSkillInfo>;
  deleteSkill: (input: DeleteSkillInput) => Promise<void>;
  getSkillEditor: (
    input: GetSkillEditorInput,
  ) => Promise<DesktopSkillEditorState>;
  readSkillFile: (
    input: ReadSkillFileInput,
  ) => Promise<DesktopSkillFileDocument>;
  saveSkillFile: (
    input: SaveSkillFileInput,
  ) => Promise<DesktopSkillFileDocument>;
  readMemoryDocument: (
    input: ReadMemoryDocumentInput,
  ) => Promise<DesktopMemoryDocument>;
  saveMemoryDocument: (
    input: SaveMemoryDocumentInput,
  ) => Promise<DesktopMemoryDocument>;
  listWorkspaceFiles: (
    input: ListWorkspaceFilesInput,
  ) => Promise<DesktopWorkspaceFileListing>;
  previewWorkspaceFile: (
    input: PreviewWorkspaceFileInput,
  ) => Promise<DesktopWorkspaceFilePreview>;
  revealWorkspaceFile: (
    input: RevealWorkspaceFileInput,
  ) => Promise<void>;
  uploadChatAttachments: (
    input: UploadChatAttachmentsInput,
  ) => Promise<UploadChatAttachmentsResult>;
  uploadWorkspaceFiles: (
    input: UploadWorkspaceFilesInput,
  ) => Promise<UploadWorkspaceFilesResult>;
  createSkillEntry: (
    input: CreateSkillEntryInput,
  ) => Promise<DesktopSkillEditorState>;
  deleteSkillEntry: (
    input: DeleteSkillEntryInput,
  ) => Promise<DesktopSkillEditorState>;
  listSlashCommands: () => Promise<SlashCommand[]>;
  createSlashCommand: (input: UpsertSlashCommandInput) => Promise<SlashCommand>;
  updateSlashCommand: (input: UpdateSlashCommandInput) => Promise<SlashCommand>;
  deleteSlashCommand: (input: DeleteSlashCommandInput) => Promise<void>;
  listMcpServers: () => Promise<DesktopMcpServer[]>;
  createMcpServer: (input: UpsertMcpServerInput) => Promise<DesktopMcpServer>;
  updateMcpServer: (input: UpdateMcpServerInput) => Promise<DesktopMcpServer>;
  deleteMcpServer: (input: DeleteMcpServerInput) => Promise<void>;
  toggleMcpServer: (input: ToggleMcpServerInput) => Promise<DesktopMcpServer>;
  getAutomationActivity: (
    automationId: string,
  ) => Promise<DesktopAutomationActivityFeed>;
  runAutomationNow: (input: RunAutomationNowInput) => Promise<{
    state: DesktopState;
    activity: DesktopAutomationActivityEntry;
  }>;
  addChannelAccount: (input: AddChannelAccountInput) => Promise<DesktopState>;
  setBotBinding: (input: SetBotBindingInput) => Promise<DesktopState>;
  listChannelEndpoints: () => Promise<DesktopChannelEndpoint[]>;
  bindChannelEndpoint: (
    input: BindChannelEndpointInput,
  ) => Promise<DesktopState>;
  detachChannelEndpoint: (
    input: DetachChannelEndpointInput,
  ) => Promise<DesktopState>;
  createThread: (input?: CreateThreadInput) => Promise<{
    state: DesktopState;
    thread: DesktopThreadSummary;
    session?: DesktopThreadSummary;
  }>;
  listProviderRecentSessions: (
    input?: ListProviderRecentSessionsInput,
  ) => Promise<DesktopProviderRecentSession[]>;
  renameThread: (input: RenameThreadInput) => Promise<DesktopState>;
  updateThreadRuntimeSettings: (
    input: UpdateThreadRuntimeSettingsInput,
  ) => Promise<ThreadTranscript>;
  listRecentThreads: (
    input: ListRecentThreadsInput,
  ) => Promise<DesktopRecentThreadsPage>;
  archiveThread: (input: ArchiveThreadInput) => Promise<DesktopState>;
  deleteThread: (input: DeleteThreadInput) => Promise<DesktopState>;
  setThreadPinned: (input: SetThreadPinnedInput) => Promise<DesktopState>;
  setThreadPinOrder: (input: SetThreadPinOrderInput) => Promise<DesktopState>;
  getThreadHistory: (
    input: string | GetThreadHistoryInput,
  ) => Promise<ThreadTranscript>;
  loadThreadTranscriptCache: (
    threadId: string,
  ) => Promise<CachedThreadTranscript | null>;
  saveThreadTranscriptCache: (
    transcript: ThreadTranscript,
    renderState?: RenderState | null,
  ) => Promise<void>;
  clearThreadTranscriptCache: (threadId: string) => Promise<void>;
  startThreadStream: (input: StartThreadStreamInput) => Promise<void>;
  stopThreadStream: (input?: StopThreadStreamInput) => Promise<void>;
  getThreadLogs: (threadId: string, cursor?: number) => Promise<ThreadLogChunk>;
  openChatStream: (input: SendMessageInput) => Promise<OpenChatStreamResult>;
  sendStreamingInput: (
    input: SendMessageInput,
  ) => Promise<SendStreamingInputResult>;
  subscribeChatStream: (listener: DesktopChatStreamListener) => void;
  unsubscribeChatStream: (listener: DesktopChatStreamListener) => void;
  subscribeDeepLinks: (listener: DesktopDeepLinkListener) => void;
  unsubscribeDeepLinks: (listener: DesktopDeepLinkListener) => void;
  interruptThread: (threadId: string) => Promise<InterruptResult>;
  checkConnection: (input?: {
    gatewayUrl?: string;
    gatewayAuthToken?: string;
    gatewayHeaders?: string;
  }) => Promise<ConnectionStatus>;
  probeGateway: (input: {
    gatewayUrl: string;
    gatewayAuthToken: string;
    gatewayHeaders?: string;
  }) => Promise<GatewayProbeResult>;
  listBrowserState: () => Promise<DesktopBrowserState>;
  createBrowserTab: (
    input?: CreateBrowserTabInput,
  ) => Promise<DesktopBrowserState>;
  activateBrowserTab: (tabId: string) => Promise<DesktopBrowserState>;
  closeBrowserTab: (tabId: string) => Promise<DesktopBrowserState>;
  navigateBrowserTab: (
    input: NavigateBrowserTabInput,
  ) => Promise<DesktopBrowserState>;
  browserGoBack: (tabId: string) => Promise<DesktopBrowserState>;
  browserGoForward: (tabId: string) => Promise<DesktopBrowserState>;
  browserReload: (tabId: string) => Promise<DesktopBrowserState>;
  browserOpenExternal: (tabId: string) => Promise<void>;
  captureBrowserTab: (
    input: string | CaptureBrowserTabInput,
  ) => Promise<CaptureBrowserTabResult>;
  setBrowserAnnotationMode: (
    input: BrowserAnnotationModeInput,
  ) => Promise<void>;
  copyImageToClipboard: (input: CopyImageToClipboardInput) => Promise<void>;
  copyTextToClipboard: (input: CopyTextToClipboardInput) => Promise<void>;
  updateBrowserBounds: (input: BrowserBoundsInput) => Promise<void>;
  setBrowserOverlayPaused: (paused: boolean) => Promise<void>;
  showBrowserConnectionMenu: (
    input: ShowBrowserConnectionMenuInput,
  ) => Promise<void>;
  subscribeBrowserState: (listener: DesktopBrowserStateListener) => void;
  unsubscribeBrowserState: (listener: DesktopBrowserStateListener) => void;
  subscribeBrowserAnnotationComments: (
    listener: DesktopBrowserAnnotationCommentListener,
  ) => void;
  unsubscribeBrowserAnnotationComments: (
    listener: DesktopBrowserAnnotationCommentListener,
  ) => void;
  subscribeBrowserPageMouseDown: (
    listener: DesktopBrowserPageMouseDownListener,
  ) => void;
  unsubscribeBrowserPageMouseDown: (
    listener: DesktopBrowserPageMouseDownListener,
  ) => void;
  listTerminalState: () => Promise<DesktopTerminalState>;
  createTerminalSession: (
    input?: CreateTerminalSessionInput,
  ) => Promise<DesktopTerminalState>;
  activateTerminalSession: (
    input: TerminalSessionInput,
  ) => Promise<DesktopTerminalState>;
  closeTerminalSession: (
    input: TerminalSessionInput,
  ) => Promise<DesktopTerminalState>;
  writeTerminalInput: (input: TerminalWriteInput) => Promise<void>;
  resizeTerminalSession: (input: TerminalResizeInput) => Promise<void>;
  subscribeTerminalEvents: (listener: DesktopTerminalEventListener) => void;
  unsubscribeTerminalEvents: (listener: DesktopTerminalEventListener) => void;
  getAppVersion: () => Promise<string>;
  getUpdateStatus: () => Promise<DesktopUpdateStatus>;
  checkForUpdatesNow: () => Promise<DesktopUpdateCheckResult>;
  installUpdate: () => Promise<DesktopUpdateInstallResult>;
  subscribeUpdateStatus: (listener: DesktopUpdateStatusListener) => void;
  unsubscribeUpdateStatus: (listener: DesktopUpdateStatusListener) => void;
}
