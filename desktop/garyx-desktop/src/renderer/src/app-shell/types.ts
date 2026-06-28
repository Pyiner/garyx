import type {
  DesktopAutomationActivityFeed,
  DesktopAutomationSchedule,
  DesktopBotConsoleSummary,
  DesktopChannelEndpoint,
  DesktopWorkspaceFileListing,
  DesktopWorkspaceFilePreview,
  MessageFileAttachment,
  MessageImageAttachment,
  PendingThreadInput,
  TranscriptMessage,
} from '@shared/contracts';
import type { AgentPickerOption } from './agent-options';

// These vocabularies are part of the cross-platform conversation state
// contract (docs/agents/conversation-state.md). The runtime arrays exist so
// conformance tests can assert them against spec/conversation-state.
export const TRANSCRIPT_ENTRY_STATES = [
  'optimistic',
  'remote_partial',
  'remote_final',
  'error',
  'interrupted',
] as const;
export type TranscriptEntryState = (typeof TRANSCRIPT_ENTRY_STATES)[number];

export type UiTranscriptMessage = TranscriptMessage & {
  intentId?: string;
  remoteRunId?: string;
  localState?: TranscriptEntryState;
};

export type MessageMap = Record<string, UiTranscriptMessage[]>;
export type PendingThreadInputMap = Record<string, PendingThreadInput[]>;
export type ContentView = 'thread' | 'browser' | 'bots' | 'automation' | 'capsules' | 'agents' | 'teams' | 'skills' | 'tasks' | 'workflow' | 'dreams' | 'settings';

export const LIVE_STREAM_STATUSES = [
  'connecting',
  'streaming',
  'reconciling',
  'disconnected',
  'failed',
  'interrupted',
] as const;
export type LiveStreamStatus = (typeof LIVE_STREAM_STATUSES)[number];

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
  targetMode: 'new_thread' | 'existing_thread';
  targetThreadId: string;
  workspacePath: string;
  schedule: DesktopAutomationSchedule;
};

export type AutomationAgentOption = AgentPickerOption;

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
