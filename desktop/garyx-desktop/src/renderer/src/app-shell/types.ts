import type {
  DesktopAutomationActivityFeed,
  DesktopAutomationSchedule,
  DesktopBotConsoleSummary,
  DesktopChatStreamEvent,
  DesktopChannelEndpoint,
  DesktopWorkspaceFileListing,
  DesktopWorkspaceFilePreview,
  MessageFileAttachment,
  MessageImageAttachment,
  PendingThreadInput,
  TranscriptMessage,
} from '@shared/contracts';

export type TranscriptEntryState = 'optimistic' | 'remote_partial' | 'remote_final' | 'error' | 'interrupted';

export type UiTranscriptMessage = TranscriptMessage & {
  intentId?: string;
  remoteRunId?: string;
  localState?: TranscriptEntryState;
};

export type MessageMap = Record<string, UiTranscriptMessage[]>;
export type PendingThreadInputMap = Record<string, PendingThreadInput[]>;
export type ContentView = 'thread' | 'browser' | 'bots' | 'automation' | 'auto_research' | 'agents' | 'teams' | 'skills' | 'settings';

export type LiveStreamStatus =
  | 'connecting'
  | 'streaming'
  | 'reconciling'
  | 'disconnected'
  | 'failed'
  | 'interrupted';

export type GatewayIndicatorTone = 'syncing' | 'offline' | null;

export type LiveStreamState = {
  threadId: string;
  runId?: string;
  activeIntentId?: string;
  assistantEntryId?: string | null;
  pendingAckIntentIds: string[];
  streamStatus: LiveStreamStatus;
};

export type PendingAutomationRun = {
  threadId: string;
  runId: string;
  prompt: string;
};

export type BoundBot = {
  id: string;
  channel: string;
  accountId: string;
  label: string;
  endpointCount: number;
};

export type AutomationDraft = {
  label: string;
  prompt: string;
  agentId: string;
  workspacePath: string;
  schedule: DesktopAutomationSchedule;
};

export type AutomationAgentOption = {
  id: string;
  label: string;
  kind: 'agent' | 'team';
};

export type AutomationDialogState = {
  mode: 'create' | 'edit';
  automationId?: string;
  draft: AutomationDraft;
};

export type ThreadLogLine = {
  key: string;
  timestamp?: string;
  text: string;
  level: 'default' | 'error';
};

export type ClientLogEntry = {
  key: string;
  timestamp: string;
  eventType: DesktopChatStreamEvent['type'];
  summary: string;
  detail: string;
  level: 'default' | 'error';
};

export type ThreadLogTab = 'mobile' | 'client';

export type WorkspaceDirectoryState = {
  entries: DesktopWorkspaceFileListing['entries'];
  loading: boolean;
  loaded: boolean;
  error: string | null;
};

export type WorkspacePreviewState = {
  selectedWorkspaceFile: {
    workspacePath: string;
    path: string;
  } | null;
  workspaceFilePreview: DesktopWorkspaceFilePreview | null;
  workspaceFilePreviewLoading: boolean;
  workspaceFilePreviewError: string | null;
  workspacePreviewModalOpen: boolean;
  workspaceFileUploadPending: boolean;
};

export type BotSidebarGroup = DesktopBotConsoleSummary;
export type ChannelEndpoint = DesktopChannelEndpoint;
export type ComposerImages = MessageImageAttachment[];
export type ComposerFiles = MessageFileAttachment[];
