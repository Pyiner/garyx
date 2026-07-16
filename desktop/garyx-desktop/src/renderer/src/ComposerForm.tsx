import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type ClipboardEvent,
  type CompositionEvent,
  type DragEvent,
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
  type RefObject,
} from 'react';
import { createPortal } from 'react-dom';

import { Box, Brain, CircleUser, Cloud as CloudIcon, Code, Command, FileText, GitBranch, Info, MessageCircle, Minimize2, Paperclip, Plug, Plus, Server, Settings as SettingsIcon, Square, Terminal, X, Zap } from 'lucide-react';
import type { LucideIcon } from 'lucide-react';

import type {
  BrowserAnnotationCommentRequest,
  DesktopBotConsoleSummary,
  DesktopApiProviderType,
  DesktopProviderModels,
  MessageFileAttachment,
  MessageImageAttachment,
  SlashCommand,
} from '@shared/contracts';

import {
  Attachment,
  AttachmentAction,
  AttachmentActions,
  AttachmentContent,
  AttachmentDescription,
  AttachmentGroup,
  AttachmentMedia,
  AttachmentTitle,
  AttachmentTrigger,
} from '@/components/ui/attachment';
import {
  DropdownMenu,
  DropdownMenuGroup,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import {
  FloatingActionMenuContent,
  FloatingActionMenuItem,
  FloatingActionMenuSubContent,
  FloatingActionMenuSubTrigger,
} from '@/components/ui/floating-action-menu';
import {
  groupAgentOptions,
  type ComposerAgentOption,
} from './app-shell/agent-options';
import { AgentOptionAvatar, AgentOptionRow } from './app-shell/components/AgentOptionAvatar';
import { providerLabel as sharedProviderLabel } from './app-shell/components/agents-hub-helpers';
import { AgentsIcon } from './app-shell/icons';
import { resolveComposerModelControlState } from './composer-model-control';

export type { ComposerAgentOption };

import { ChannelLogo } from './channel-logo';
import { useChannelPluginCatalog } from './channel-plugins/useChannelPluginCatalog';
import { buildMessageImageDataUrl, ImageZoomDialog } from './message-rich-content';
import type { ComposerPendingUpload } from './app-shell/useMessageDispatchController';
import { useI18n, type Translate } from './i18n';

type ComposerFormProps = {
  activeQueueLength: number;
  composer: string;
  composerContext?: ReactNode;
  composerAttachmentInputRef: RefObject<HTMLInputElement | null>;
  composerBrowserAnnotations: BrowserAnnotationCommentRequest[];
  composerFiles: MessageFileAttachment[];
  composerHasPayload: boolean;
  composerImages: MessageImageAttachment[];
  composerPendingUploads?: ComposerPendingUpload[];
  composerEditingLocked: boolean;
  composerLocked: boolean;
  composerPlaceholder: string;
  composerProviderType: DesktopApiProviderType;
  composerResetKey: number;
  composerTextareaRef: RefObject<HTMLTextAreaElement | null>;
  activeThreadBot?: DesktopBotConsoleSummary | null;
  activeThreadBotId?: string | null;
  botBindingDisabled?: boolean;
  botGroups?: DesktopBotConsoleSummary[];
  /** Display name of the selected agent, shown in the provider pill. */
  agentLabel?: string | null;
  /** When provided, the provider pill becomes a dropdown to change the agent. */
  agentOptions?: ComposerAgentOption[];
  selectedAgentId?: string;
  onSelectAgent?: (agentId: string) => void;
  /** Provider model catalog for the pending agent; enables the new-thread model override control. */
  newThreadProviderModels?: DesktopProviderModels | null;
  /** The pending agent's configured model; filters thinking levels when no override is chosen. */
  newThreadAgentConfiguredModel?: string | null;
  newThreadSelectedModel?: string | null;
  newThreadSelectedReasoningEffort?: string | null;
  newThreadSelectedServiceTier?: string | null;
  onSelectNewThreadModel?: (model: string | null) => void;
  onSelectNewThreadReasoningEffort?: (effort: string | null) => void;
  onSelectNewThreadServiceTier?: (tier: string | null) => void;
  threadProviderModels?: DesktopProviderModels | null;
  threadEffectiveModel?: string | null;
  threadEffectiveReasoningEffort?: string | null;
  threadEffectiveServiceTier?: string | null;
  threadSelectedModel?: string | null;
  threadSelectedReasoningEffort?: string | null;
  threadSelectedServiceTier?: string | null;
  onSelectThreadModel?: (model: string | null) => void;
  onSelectThreadReasoningEffort?: (effort: string | null) => void;
  onSelectThreadServiceTier?: (tier: string | null) => void;
  isActiveSendingThread: boolean;
  onAppendComposerAttachments: (files: File[]) => void;
  onComposerChange: (value: string) => void;
  onComposerCompositionEnd: (value: string) => void;
  onComposerCompositionStart: () => void;
  onComposerKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => void;
  onComposerPasteFiles: (files: File[]) => void;
  onInterrupt: () => void;
  onRemoveComposerBrowserAnnotation: (annotationId: string) => void;
  onRemoveComposerFile: (fileId: string) => void;
  onRemoveComposerImage: (imageId: string) => void;
  onRemoveComposerPendingUpload?: (uploadId: string) => void;
  onSelectBotBinding?: (botId: string | null) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  slashPanelContainerRef: RefObject<HTMLDivElement | null>;
  slashCommands: SlashCommand[];
  slashCommandsLoaded: boolean;
  slashCommandsLoading: boolean;
};

const COMPOSER_EDITOR_MAX_LINES = 10;

type SlashTrigger = {
  end: number;
  query: string;
  start: number;
};

function resolveSlashTrigger(value: string, cursor: number): SlashTrigger | null {
  const safeCursor = Math.max(0, Math.min(cursor, value.length));
  const beforeCursor = value.slice(0, safeCursor);
  const match = beforeCursor.match(/(^|\s)\/[a-z0-9_]*$/i);
  if (!match) {
    return null;
  }

  const boundary = match[1] || '';
  const matched = match[0];
  const start = safeCursor - matched.length + boundary.length;
  let end = safeCursor;
  while (end < value.length && /[a-z0-9_]/i.test(value[end] || '')) {
    end += 1;
  }

  return {
    end,
    query: value.slice(start + 1, safeCursor).toLowerCase(),
    start,
  };
}

function slashCommandMeta(command: SlashCommand): string {
  return (command.description || command.prompt || '').trim();
}

function slashCommandSummary(command: SlashCommand): string {
  return (command.description || command.prompt || 'Prompt shortcut').trim();
}

function slashCommandPreview(command: SlashCommand): string {
  const preview = (command.prompt || command.description || '').trim();
  return preview.length > 120 ? `${preview.slice(0, 117)}…` : preview;
}

function slashCommandLabel(command: SlashCommand, t: Translate): string {
  const key = command.name.toLowerCase();
  const labels: Record<string, string> = {
    branch: t('Branch'),
    cloud: t('Cloud'),
    compact: t('Compact'),
    compress: t('Compress'),
    feedback: t('Feedback'),
    fast: t('Fast'),
    mcp: 'MCP',
    model: t('Model'),
    persona: t('Persona'),
    profile: t('Profile'),
    reasoning: t('Reasoning'),
    review: t('Review'),
    status: t('Status'),
  };
  return labels[key] || command.name;
}

function slashCommandIcon(command: SlashCommand): LucideIcon {
  const key = command.name.toLowerCase();
  if (key.includes('mcp') || key.includes('server')) {
    return Server;
  }
  if (key.includes('status') || key.includes('info')) {
    return Info;
  }
  if (key.includes('branch') || key.includes('worktree')) {
    return GitBranch;
  }
  if (key.includes('fast') || key.includes('quick')) {
    return Zap;
  }
  if (key.includes('reason')) {
    return Brain;
  }
  if (key.includes('model')) {
    return Box;
  }
  if (key.includes('compact') || key.includes('compress')) {
    return Minimize2;
  }
  if (key.includes('review') || key.includes('code')) {
    return Code;
  }
  if (key.includes('feedback')) {
    return MessageCircle;
  }
  if (key.includes('profile') || key.includes('persona')) {
    return CircleUser;
  }
  if (key.includes('cloud') || key.includes('sync')) {
    return CloudIcon;
  }
  if (key.includes('setting') || key.includes('config')) {
    return SettingsIcon;
  }
  if (key.includes('terminal') || key.includes('shell')) {
    return Terminal;
  }
  return Command;
}

const PROVIDER_CHEVRON_GLYPH = (
  <svg
    aria-hidden
    width="10"
    height="10"
    viewBox="0 0 10 10"
    fill="none"
    className="composer-provider-chevron"
  >
    <path
      d="M2.5 3.75L5 6.25L7.5 3.75"
      stroke="currentColor"
      strokeWidth="1.2"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
);

const AGENT_PROVIDER_GLYPH = (
  <span aria-hidden className="composer-provider-agent-icon">
    <AgentsIcon />
  </span>
);

function renderComposerAgentOptionIcon(option: ComposerAgentOption) {
  return (
    <AgentOptionAvatar
      agentId={option.id}
      avatarDataUrl={option.avatarDataUrl}
      className="composer-agent-option-icon"
      kind={option.kind}
      label={option.label}
      providerIcon={option.providerIcon}
      providerType={option.providerType}
    />
  );
}

function renderComposerProviderTriggerIcon(option?: ComposerAgentOption) {
  if (!option) {
    return AGENT_PROVIDER_GLYPH;
  }

  return renderComposerAgentOptionIcon(option);
}

function browserAnnotationChipLabel(
  annotation: BrowserAnnotationCommentRequest,
  t: Translate,
): string {
  return annotation.comment.trim() || t('Browser comment');
}

function browserAnnotationChipMeta(annotation: BrowserAnnotationCommentRequest): string {
  return [
    annotation.label || annotation.tagName,
    annotation.title || annotation.url,
  ]
    .filter(Boolean)
    .join(' · ');
}

function renderComposerModelControl({
  providerModels,
  agentConfiguredModel,
  effectiveModel,
  effectiveReasoningEffort,
  effectiveServiceTier,
  selectedModel,
  selectedReasoningEffort,
  selectedServiceTier,
  onSelectModel,
  onSelectReasoningEffort,
  onSelectServiceTier,
  t,
}: {
  providerModels?: DesktopProviderModels | null;
  agentConfiguredModel?: string | null;
  effectiveModel?: string | null;
  effectiveReasoningEffort?: string | null;
  effectiveServiceTier?: string | null;
  selectedModel?: string | null;
  selectedReasoningEffort?: string | null;
  selectedServiceTier?: string | null;
  onSelectModel?: (model: string | null) => void;
  onSelectReasoningEffort?: (effort: string | null) => void;
  onSelectServiceTier?: (tier: string | null) => void;
  t: Translate;
}) {
  if (!onSelectModel || !providerModels?.supportsModelSelection) {
    return null;
  }

  const {
    models,
    effectiveModelId,
    defaultModelId,
    defaultModelLabel,
    defaultModelOption,
    triggerLabel,
    reasoningEfforts,
    effectiveReasoningEffortId,
    defaultReasoningEffortId,
    defaultEffortLabel,
    serviceTiers,
    effectiveServiceTierId,
    defaultServiceTierLabel,
  } = resolveComposerModelControlState({
    providerModels,
    agentConfiguredModel,
    effectiveModel,
    effectiveReasoningEffort,
    effectiveServiceTier,
    selectedModel,
    selectedReasoningEffort,
    selectedServiceTier,
    modelFallbackLabel: t("Model"),
    thinkingLevelFallbackLabel: t("Thinking level"),
    standardServiceTierLabel: t("Standard"),
  });
  const supportsReasoning =
    Boolean(providerModels.supportsReasoningEffortSelection) &&
    reasoningEfforts.length > 0;
  const supportsServiceTier =
    Boolean(providerModels.supportsServiceTierSelection) && serviceTiers.length > 0;

  // Selecting a model also clears a service tier the target model does not
  // support, so the thread never runs with an unsupported speed tier (mirrors
  // the iOS `selectModel` sanitize).
  const selectModelSanitizingTier = (modelId: string | null) => {
    onSelectModel(modelId);
    if (!onSelectServiceTier || !effectiveServiceTierId) {
      return;
    }
    const targetOption = modelId
      ? models.find((option) => option.id === modelId)
      : defaultModelOption;
    const targetTiers = targetOption?.serviceTiers?.length
      ? targetOption.serviceTiers
      : providerModels.serviceTiers || [];
    if (!targetTiers.some((tier) => tier.id === effectiveServiceTierId)) {
      onSelectServiceTier(null);
    }
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label={t("Change model for this thread")}
        className="composer-provider-trigger"
        data-muted={!effectiveModelId && !effectiveReasoningEffortId ? "" : undefined}
        type="button"
      >
        <Box aria-hidden size={15} strokeWidth={1.75} />
        <span className="composer-provider-label">{triggerLabel}</span>
        {PROVIDER_CHEVRON_GLYPH}
      </DropdownMenuTrigger>
      <FloatingActionMenuContent align="start" side="top">
        <DropdownMenuGroup className="composer-model-menu-options">
          <FloatingActionMenuItem
            data-active={
              !effectiveModelId || effectiveModelId === defaultModelId ? '' : undefined
            }
            onSelect={() => selectModelSanitizingTier(null)}
          >
            <span className="composer-menu-label">{defaultModelLabel}</span>
          </FloatingActionMenuItem>
          {models
            .filter((option) => option.id !== defaultModelId)
            .map((option) => (
              <FloatingActionMenuItem
                data-active={option.id === effectiveModelId ? '' : undefined}
                key={option.id}
                onSelect={() => selectModelSanitizingTier(option.id)}
              >
                <span className="composer-menu-label">{option.label}</span>
              </FloatingActionMenuItem>
            ))}
        </DropdownMenuGroup>
        {supportsReasoning && onSelectReasoningEffort ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuSub>
              <FloatingActionMenuSubTrigger>
                <Brain aria-hidden size={15} strokeWidth={1.75} />
                {t("Thinking level")}
              </FloatingActionMenuSubTrigger>
              <FloatingActionMenuSubContent>
                <FloatingActionMenuItem
                  data-active={
                    !effectiveReasoningEffortId ||
                    effectiveReasoningEffortId === defaultReasoningEffortId
                      ? ''
                      : undefined
                  }
                  onSelect={() => onSelectReasoningEffort(null)}
                >
                  <span className="composer-menu-label">{defaultEffortLabel}</span>
                </FloatingActionMenuItem>
                {reasoningEfforts
                  .filter((option) => option.id !== defaultReasoningEffortId)
                  .map((option) => (
                    <FloatingActionMenuItem
                      data-active={
                        option.id === effectiveReasoningEffortId ? '' : undefined
                      }
                      key={option.id}
                      onSelect={() => onSelectReasoningEffort(option.id)}
                    >
                      <span className="composer-menu-label">{option.label}</span>
                    </FloatingActionMenuItem>
                  ))}
              </FloatingActionMenuSubContent>
            </DropdownMenuSub>
          </>
        ) : null}
        {supportsServiceTier && onSelectServiceTier ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuSub>
              <FloatingActionMenuSubTrigger>
                <Zap aria-hidden size={15} strokeWidth={1.75} />
                {t("Speed")}
              </FloatingActionMenuSubTrigger>
              <FloatingActionMenuSubContent>
                <FloatingActionMenuItem
                  data-active={!effectiveServiceTierId ? '' : undefined}
                  onSelect={() => onSelectServiceTier(null)}
                >
                  <span className="composer-menu-label">{defaultServiceTierLabel}</span>
                </FloatingActionMenuItem>
                {serviceTiers.map((option) => (
                  <FloatingActionMenuItem
                    data-active={option.id === effectiveServiceTierId ? '' : undefined}
                    key={option.id}
                    onSelect={() => onSelectServiceTier(option.id)}
                  >
                    <span className="composer-menu-label">{option.label}</span>
                  </FloatingActionMenuItem>
                ))}
              </FloatingActionMenuSubContent>
            </DropdownMenuSub>
          </>
        ) : null}
      </FloatingActionMenuContent>
    </DropdownMenu>
  );
}

function renderComposerProviderControl({
  composerProviderType,
  agentLabel,
  agentOptions,
  selectedAgentId,
  onSelectAgent,
  t,
}: {
  composerProviderType: DesktopApiProviderType;
  agentLabel?: string | null;
  agentOptions?: ComposerAgentOption[];
  selectedAgentId?: string;
  onSelectAgent?: (agentId: string) => void;
  t: Translate;
}) {
  const selectedOption = agentOptions?.find((option) => option.id === selectedAgentId);
  const providerIcon = renderComposerProviderTriggerIcon(selectedOption);
  const providerLabel = agentLabel || sharedProviderLabel(composerProviderType);

  if (onSelectAgent) {
    const grouped = groupAgentOptions(agentOptions ?? []);
    const hasAgents = grouped.agent.length > 0;
    return (
      <DropdownMenu>
        <DropdownMenuTrigger
          aria-label={t("Change agent for this thread")}
          className="composer-provider-trigger"
          type="button"
        >
          {providerIcon}
          <span className="composer-provider-label">{providerLabel}</span>
          {PROVIDER_CHEVRON_GLYPH}
        </DropdownMenuTrigger>
        <FloatingActionMenuContent
          align="start"
          side="top"
        >
          {grouped.builtin.map((option) => (
            <FloatingActionMenuItem
              data-active={option.id === selectedAgentId ? '' : undefined}
              key={option.id}
              onSelect={() => onSelectAgent?.(option.id)}
            >
              <AgentOptionRow
                option={option}
              />
            </FloatingActionMenuItem>
          ))}
          {hasAgents ? <DropdownMenuSeparator /> : null}
          {hasAgents ? (
            <DropdownMenuSub>
              <FloatingActionMenuSubTrigger>{t("Agents")}</FloatingActionMenuSubTrigger>
              <FloatingActionMenuSubContent>
                {grouped.agent.map((option) => (
                  <FloatingActionMenuItem
                    data-active={option.id === selectedAgentId ? '' : undefined}
                    key={option.id}
                    onSelect={() => onSelectAgent?.(option.id)}
                  >
                    <AgentOptionRow
                      option={option}
                    />
                  </FloatingActionMenuItem>
                ))}
              </FloatingActionMenuSubContent>
            </DropdownMenuSub>
          ) : null}
        </FloatingActionMenuContent>
      </DropdownMenu>
    );
  }

  return (
    <div className="composer-provider-control">
      {providerIcon}
      <span className="composer-provider-label">{providerLabel}</span>
      {PROVIDER_CHEVRON_GLYPH}
    </div>
  );
}

function renderComposerBotBindingSubmenu({
  activeThreadBot,
  activeThreadBotId,
  botGroups,
  iconDataUrlByChannel,
  onSelectBotBinding,
  t,
}: {
  activeThreadBot?: DesktopBotConsoleSummary | null;
  activeThreadBotId?: string | null;
  botGroups?: DesktopBotConsoleSummary[];
  iconDataUrlByChannel: Map<string, string | null>;
  onSelectBotBinding?: (botId: string | null) => void;
  t: Translate;
}) {
  if (!onSelectBotBinding) {
    return null;
  }

  const groups = botGroups ?? [];
  const selectedBot = activeThreadBotId
    ? groups.find((group) => group.id === activeThreadBotId) || activeThreadBot
    : null;
  const visibleGroups =
    selectedBot && !groups.some((group) => group.id === selectedBot.id)
      ? [selectedBot, ...groups]
      : groups;

  return (
    <DropdownMenuSub>
      <FloatingActionMenuSubTrigger
        className={`composer-menu-subtrigger ${selectedBot ? 'selected' : ''}`}
      >
        {selectedBot ? (
          <ChannelLogo
            channel={selectedBot.channel}
            className="channel-logo composer-menu-bot-logo"
            iconDataUrl={iconDataUrlByChannel.get(selectedBot.channel.toLowerCase()) || null}
            fallbackLabel={selectedBot.title}
          />
        ) : (
          <Plug aria-hidden size={15} strokeWidth={1.7} />
        )}
        <span className="composer-menu-label">{selectedBot?.title || t('Bind bot')}</span>
      </FloatingActionMenuSubTrigger>
      <FloatingActionMenuSubContent>
        <FloatingActionMenuItem
          className="composer-bot-menu-item"
          data-active={!activeThreadBotId ? '' : undefined}
          onSelect={() => {
            onSelectBotBinding(null);
          }}
        >
          <Plug aria-hidden size={16} strokeWidth={1.7} />
          <span className="composer-menu-label">{t('No bot')}</span>
        </FloatingActionMenuItem>
        {visibleGroups.length ? (
          visibleGroups.map((group) => {
            const isActive = group.id === activeThreadBotId;
            return (
              <FloatingActionMenuItem
                className="composer-bot-menu-item"
                data-active={isActive ? '' : undefined}
                key={group.id}
                onSelect={() => {
                  onSelectBotBinding(group.id);
                }}
              >
                <ChannelLogo
                  channel={group.channel}
                  className="channel-logo composer-menu-bot-logo"
                  iconDataUrl={iconDataUrlByChannel.get(group.channel.toLowerCase()) || null}
                  fallbackLabel={group.title}
                />
                <span className="composer-menu-label">{group.title}</span>
              </FloatingActionMenuItem>
            );
          })
        ) : (
          <FloatingActionMenuItem className="composer-bot-menu-item muted" disabled>
            {t('No bots configured')}
          </FloatingActionMenuItem>
        )}
      </FloatingActionMenuSubContent>
    </DropdownMenuSub>
  );
}

export function ComposerForm({
  activeQueueLength,
  composer,
  composerContext,
  composerAttachmentInputRef,
  composerBrowserAnnotations,
  composerFiles,
  composerHasPayload,
  composerImages,
  composerPendingUploads = [],
  composerEditingLocked,
  composerLocked,
  composerPlaceholder,
  composerProviderType,
  composerResetKey,
  composerTextareaRef,
  activeThreadBot,
  activeThreadBotId,
  botBindingDisabled = false,
  botGroups,
  agentLabel,
  agentOptions,
  selectedAgentId,
  onSelectAgent,
  newThreadProviderModels,
  newThreadAgentConfiguredModel,
  newThreadSelectedModel,
  newThreadSelectedReasoningEffort,
  newThreadSelectedServiceTier,
  onSelectNewThreadModel,
  onSelectNewThreadReasoningEffort,
  onSelectNewThreadServiceTier,
  threadProviderModels,
  threadEffectiveModel,
  threadEffectiveReasoningEffort,
  threadEffectiveServiceTier,
  threadSelectedModel,
  threadSelectedReasoningEffort,
  threadSelectedServiceTier,
  onSelectThreadModel,
  onSelectThreadReasoningEffort,
  onSelectThreadServiceTier,
  isActiveSendingThread,
  onAppendComposerAttachments,
  onComposerChange,
  onComposerCompositionEnd,
  onComposerCompositionStart,
  onComposerKeyDown,
  onComposerPasteFiles,
  onInterrupt,
  onRemoveComposerBrowserAnnotation,
  onRemoveComposerFile,
  onRemoveComposerImage,
  onRemoveComposerPendingUpload,
  onSelectBotBinding,
  onSubmit,
  slashPanelContainerRef,
  slashCommands,
  slashCommandsLoaded,
  slashCommandsLoading,
}: ComposerFormProps) {
  const { t } = useI18n();
  const { entries: pluginCatalog } = useChannelPluginCatalog();
  const iconDataUrlByChannel = useMemo(
    () =>
      new Map(
        (pluginCatalog || []).map((entry) => [
          entry.id.toLowerCase(),
          entry.icon_data_url || null,
        ]),
      ),
    [pluginCatalog],
  );
  const [composerCursor, setComposerCursor] = useState(composer.length);
  const [draft, setDraft] = useState(composer);
  const [highlightedSlashCommandIndex, setHighlightedSlashCommandIndex] = useState(0);
  const [dismissedSlashQuery, setDismissedSlashQuery] = useState<string | null>(null);
  const slashCommandItemRefs = useRef<Array<HTMLButtonElement | null>>([]);

  useEffect(() => {
    setDraft(composer);
    setComposerCursor(composer.length);
  }, [composer, composerResetKey]);

  const slashCursor = Math.max(
    0,
    Math.min(
      composerTextareaRef.current?.selectionStart ?? composerCursor,
      draft.length,
    ),
  );
  const slashTrigger = resolveSlashTrigger(draft, slashCursor);
  const slashQuery = slashTrigger ? slashTrigger.query : null;
  const filteredSlashCommands = slashTrigger
    ? slashCommands.filter((command) => {
      const metadata = slashCommandMeta(command).toLowerCase();
      return command.name.toLowerCase().includes(slashTrigger.query) || metadata.includes(slashTrigger.query);
    })
    : [];
  const highlightedSlashCommand = filteredSlashCommands[
    Math.min(highlightedSlashCommandIndex, Math.max(filteredSlashCommands.length - 1, 0))
  ] || null;
  const slashPanelOpen = Boolean(
    slashTrigger &&
    dismissedSlashQuery !== slashQuery &&
    (
      filteredSlashCommands.length > 0 ||
      (slashCommandsLoaded && !slashCommandsLoading && slashCommands.length === 0)
    )
  );

  useLayoutEffect(() => {
    const textarea = composerTextareaRef.current;
    if (!textarea) {
      return;
    }

    const computedStyle = window.getComputedStyle(textarea);
    const lineHeight = Number.parseFloat(computedStyle.lineHeight) || 19.5;
    const paddingTop = Number.parseFloat(computedStyle.paddingTop) || 0;
    const paddingBottom = Number.parseFloat(computedStyle.paddingBottom) || 0;
    const maxHeight =
      lineHeight * COMPOSER_EDITOR_MAX_LINES + paddingTop + paddingBottom;

    textarea.style.height = 'auto';
    const nextHeight = Math.min(textarea.scrollHeight, maxHeight);
    textarea.style.height = `${nextHeight}px`;
    textarea.style.overflowY =
      textarea.scrollHeight > maxHeight ? 'auto' : 'hidden';
  }, [draft, composerTextareaRef]);

  useEffect(() => {
    if (!slashTrigger) {
      if (dismissedSlashQuery !== null) {
        setDismissedSlashQuery(null);
      }
      if (highlightedSlashCommandIndex !== 0) {
        setHighlightedSlashCommandIndex(0);
      }
      return;
    }

    if (dismissedSlashQuery && dismissedSlashQuery !== slashQuery) {
      setDismissedSlashQuery(null);
    }

    setHighlightedSlashCommandIndex((current) => {
      if (!filteredSlashCommands.length) {
        return 0;
      }
      return Math.min(current, filteredSlashCommands.length - 1);
    });
  }, [
    dismissedSlashQuery,
    filteredSlashCommands.length,
    highlightedSlashCommandIndex,
    slashQuery,
    slashTrigger,
  ]);

  useEffect(() => {
    if (!slashPanelOpen || !filteredSlashCommands.length) {
      return;
    }
    const activeItem = slashCommandItemRefs.current[highlightedSlashCommandIndex];
    activeItem?.scrollIntoView({ block: 'nearest' });
  }, [filteredSlashCommands.length, highlightedSlashCommandIndex, slashPanelOpen]);

  function focusComposerAt(position: number) {
    window.requestAnimationFrame(() => {
      const textarea = composerTextareaRef.current;
      if (!textarea) {
        return;
      }
      setComposerCursor(position);
      textarea.focus();
      textarea.setSelectionRange(position, position);
    });
  }

  function applySlashCommand(command: SlashCommand) {
    const insertion = `/${command.name} `;
    const nextValue = slashTrigger
      ? `${draft.slice(0, slashTrigger.start)}${insertion}${draft.slice(slashTrigger.end)}`
      : insertion;
    setDraft(nextValue);
    onComposerChange(nextValue);
    setDismissedSlashQuery(null);
    setHighlightedSlashCommandIndex(0);
    focusComposerAt((slashTrigger?.start ?? 0) + insertion.length);
  }

  function syncComposerCursor(position: number | null) {
    if (typeof position === 'number') {
      setComposerCursor(position);
    }
  }

  function handleFileSelection(event: ChangeEvent<HTMLInputElement>) {
    const files = Array.from(event.target.files || []);
    if (!files.length) {
      return;
    }
    onAppendComposerAttachments(files);
  }

  function handleDrop(event: DragEvent<HTMLFormElement>) {
    const files = Array.from(event.dataTransfer.files || []);
    if (!files.length) {
      return;
    }
    event.preventDefault();
    onAppendComposerAttachments(files);
  }

  function handlePaste(event: ClipboardEvent<HTMLTextAreaElement>) {
    const files = Array.from(event.clipboardData.files || []);
    if (!files.length) {
      return;
    }
    event.preventDefault();
    onComposerPasteFiles(files);
  }

  function handleCompositionEnd(event: CompositionEvent<HTMLTextAreaElement>) {
    onComposerCompositionEnd(event.currentTarget.value);
  }

  function handleComposerKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (slashPanelOpen) {
      if (event.key === 'ArrowDown' && filteredSlashCommands.length > 0) {
        event.preventDefault();
        setHighlightedSlashCommandIndex((current) => {
          return current >= filteredSlashCommands.length - 1 ? 0 : current + 1;
        });
        return;
      }
      if (event.key === 'ArrowUp' && filteredSlashCommands.length > 0) {
        event.preventDefault();
        setHighlightedSlashCommandIndex((current) => {
          return current <= 0 ? filteredSlashCommands.length - 1 : current - 1;
        });
        return;
      }
      if (
        (event.key === 'Enter' || event.key === 'Tab') &&
        highlightedSlashCommand &&
        !event.metaKey &&
        !event.ctrlKey
      ) {
        event.preventDefault();
        applySlashCommand(highlightedSlashCommand);
        return;
      }
      if (event.key === 'Escape' && slashTrigger) {
        event.preventDefault();
        setDismissedSlashQuery(slashTrigger.query);
        return;
      }
    }

    onComposerKeyDown(event);
  }

  const slashPanel = slashPanelOpen ? (
    <div className="composer-command-panel" data-testid="slash-command-panel">
      <div
        aria-label={t("Slash command shortcuts")}
        className="composer-command-list"
        role="listbox"
      >
        {slashCommandsLoading ? (
          <div className="composer-command-empty">
            <strong>{t("Loading shortcuts…")}</strong>
            <span>{t("Reading the current command list.")}</span>
          </div>
        ) : filteredSlashCommands.length ? (
          filteredSlashCommands.map((command, index) => {
            const isHighlighted = index === highlightedSlashCommandIndex;
            const preview = slashCommandPreview(command);
            const CommandIcon = slashCommandIcon(command);
            const label = slashCommandLabel(command, t);
            return (
              <button
                aria-selected={isHighlighted}
                className={`composer-command-item ${isHighlighted ? 'active' : ''}`}
                data-testid={`slash-command-option-${command.name}`}
                key={command.name}
                onClick={() => {
                  applySlashCommand(command);
                }}
                onFocus={() => {
                  setHighlightedSlashCommandIndex(index);
                }}
                onMouseDown={(event) => {
                  event.preventDefault();
                }}
                onMouseEnter={() => {
                  setHighlightedSlashCommandIndex(index);
                }}
                ref={(node) => {
                  slashCommandItemRefs.current[index] = node;
                }}
                role="option"
                type="button"
              >
                <span className="composer-command-icon">
                  <CommandIcon aria-hidden size={15} strokeWidth={1.65} />
                </span>
                <span className="composer-command-name">{label}</span>
                <div className="composer-command-item-copy">
                  {preview ? <span>{preview}</span> : <span>{slashCommandSummary(command)}</span>}
                </div>
                <span className="composer-command-shortcut" aria-hidden />
              </button>
            );
          })
        ) : (
          <div className="composer-command-empty">
            <strong>
              {slashCommands.length ? t('No matching shortcut') : t('No shortcuts yet')}
            </strong>
            <span>
              {slashCommands.length
                ? t('Try a different command name.')
                : t('Create one in Settings → Commands.')}
            </span>
          </div>
        )}
      </div>
    </div>
  ) : null;
  return (
    <form
      className="composer"
      onDragOver={(event) => {
        if (event.dataTransfer.types.includes('Files')) {
          event.preventDefault();
          event.dataTransfer.dropEffect = 'copy';
        }
      }}
      onDrop={handleDrop}
      onSubmit={onSubmit}
    >
      {slashPanelContainerRef.current && slashPanel
        ? createPortal(slashPanel, slashPanelContainerRef.current)
        : null}
      <div aria-hidden className="composer-surface" />
      <input
        className="composer-attachment-input"
        multiple
        onChange={handleFileSelection}
        ref={composerAttachmentInputRef}
        tabIndex={-1}
        type="file"
      />
      {composerBrowserAnnotations.length ||
      composerImages.length ||
      composerFiles.length ||
      composerPendingUploads.length ? (
        <AttachmentGroup className="px-3 pt-2">
          {composerBrowserAnnotations.map((annotation) => {
            const label = browserAnnotationChipLabel(annotation, t);
            const meta = browserAnnotationChipMeta(annotation);
            return (
              <Attachment
                key={annotation.id}
                size="sm"
                title={[label, meta].filter(Boolean).join('\n')}
              >
                <AttachmentMedia>
                  <MessageCircle aria-hidden size={14} strokeWidth={1.8} />
                </AttachmentMedia>
                <AttachmentContent>
                  <AttachmentTitle>{label}</AttachmentTitle>
                  {meta ? (
                    <AttachmentDescription>{meta}</AttachmentDescription>
                  ) : null}
                </AttachmentContent>
                <AttachmentActions>
                  <AttachmentAction
                    aria-label={t("Remove browser comment")}
                    onClick={() => {
                      onRemoveComposerBrowserAnnotation(annotation.id);
                    }}
                  >
                    <X aria-hidden size={12} strokeWidth={2.2} />
                    <span className="sr-only">{t("Remove browser comment")}</span>
                  </AttachmentAction>
                </AttachmentActions>
              </Attachment>
            );
          })}
          {composerImages.map((image) => (
            <Attachment
              key={image.id}
              orientation="vertical"
              title={image.name}
            >
              <AttachmentMedia variant="image">
                <img
                  alt={image.name}
                  src={buildMessageImageDataUrl(image.mediaType, image.data || '')}
                />
              </AttachmentMedia>
              <ImageZoomDialog
                alt={image.name}
                suggestedName={image.name}
                src={buildMessageImageDataUrl(image.mediaType, image.data || '')}
                trigger={
                  <AttachmentTrigger
                    aria-label={t("Open image preview")}
                    title={t("Open image preview")}
                  />
                }
              />
              <AttachmentActions>
                <AttachmentAction
                  aria-label={t("Remove image attachment")}
                  className="size-5 rounded-full bg-foreground/55 text-background backdrop-blur-[2px] hover:bg-foreground/75 hover:text-background"
                  onClick={() => {
                    onRemoveComposerImage(image.id);
                  }}
                >
                  <X aria-hidden size={11} strokeWidth={2.4} />
                  <span className="sr-only">{t("Remove image attachment")}</span>
                </AttachmentAction>
              </AttachmentActions>
            </Attachment>
          ))}
          {composerFiles.map((file) => (
            <Attachment key={file.id} size="sm" title={file.name}>
              <AttachmentMedia>
                <FileText aria-hidden size={14} strokeWidth={1.8} />
              </AttachmentMedia>
              <AttachmentContent>
                <AttachmentTitle>{file.name}</AttachmentTitle>
                {file.mediaType ? (
                  <AttachmentDescription>{file.mediaType}</AttachmentDescription>
                ) : null}
              </AttachmentContent>
              <AttachmentActions>
                <AttachmentAction
                  aria-label={t("Remove file attachment")}
                  onClick={() => {
                    onRemoveComposerFile(file.id);
                  }}
                >
                  <X aria-hidden size={12} strokeWidth={2.2} />
                  <span className="sr-only">{t("Remove file attachment")}</span>
                </AttachmentAction>
              </AttachmentActions>
            </Attachment>
          ))}
          {composerPendingUploads.map((upload) =>
            upload.kind === "image" ? (
              <Attachment
                key={upload.id}
                orientation="vertical"
                state={upload.status === "error" ? "error" : "uploading"}
                title={upload.name}
              >
                <AttachmentMedia variant="image">
                  {upload.previewUrl ? (
                    <>
                      <img
                        alt={upload.name}
                        className="composer-upload-img-base"
                        src={upload.previewUrl}
                      />
                      {upload.status === "uploading" ? (
                        <img
                          alt=""
                          aria-hidden
                          className="composer-upload-img-reveal"
                          src={upload.previewUrl}
                        />
                      ) : null}
                    </>
                  ) : (
                    <FileText aria-hidden size={14} strokeWidth={1.8} />
                  )}
                </AttachmentMedia>
                <AttachmentActions>
                  <AttachmentAction
                    aria-label={t("Remove image attachment")}
                    className="size-5 rounded-full bg-foreground/55 text-background backdrop-blur-[2px] hover:bg-foreground/75 hover:text-background"
                    onClick={() => {
                      onRemoveComposerPendingUpload?.(upload.id);
                    }}
                  >
                    <X aria-hidden size={11} strokeWidth={2.4} />
                    <span className="sr-only">{t("Remove image attachment")}</span>
                  </AttachmentAction>
                </AttachmentActions>
              </Attachment>
            ) : (
              <Attachment
                key={upload.id}
                size="sm"
                state={upload.status === "error" ? "error" : "uploading"}
                title={upload.name}
              >
                <AttachmentMedia>
                  <FileText aria-hidden size={14} strokeWidth={1.8} />
                </AttachmentMedia>
                <AttachmentContent>
                  <AttachmentTitle>{upload.name}</AttachmentTitle>
                  <AttachmentDescription>
                    {upload.status === "error"
                      ? t("Upload failed")
                      : t("Uploading…")}
                  </AttachmentDescription>
                </AttachmentContent>
                <AttachmentActions>
                  <AttachmentAction
                    aria-label={t("Remove file attachment")}
                    onClick={() => {
                      onRemoveComposerPendingUpload?.(upload.id);
                    }}
                  >
                    <X aria-hidden size={12} strokeWidth={2.2} />
                    <span className="sr-only">{t("Remove file attachment")}</span>
                  </AttachmentAction>
                </AttachmentActions>
              </Attachment>
            ),
          )}
        </AttachmentGroup>
      ) : null}
      <textarea
        className="composer-editor"
        ref={composerTextareaRef}
        disabled={composerEditingLocked}
        value={draft}
        onChange={(event) => {
          const nextValue = event.target.value;
          setDraft(nextValue);
          syncComposerCursor(event.target.selectionStart);
          onComposerChange(nextValue);
        }}
        onClick={(event) => {
          syncComposerCursor(event.currentTarget.selectionStart);
        }}
        onCompositionStart={onComposerCompositionStart}
        onCompositionEnd={handleCompositionEnd}
        onKeyDown={handleComposerKeyDown}
        onPaste={handlePaste}
        onSelect={(event) => {
          syncComposerCursor(event.currentTarget.selectionStart);
        }}
        placeholder={composerPlaceholder}
      />
      <div className="composer-actions composer-footer">
        <div className="composer-leading-actions">
          <DropdownMenu>
            <DropdownMenuTrigger
              aria-label={t('Composer actions')}
              className="ghost-button composer-plus-trigger"
              disabled={composerLocked && botBindingDisabled}
              type="button"
            >
              <Plus aria-hidden size={18} strokeWidth={1.8} />
            </DropdownMenuTrigger>
            <FloatingActionMenuContent
              align="start"
              side="top"
            >
              <FloatingActionMenuItem
                disabled={composerEditingLocked}
                onSelect={() => {
                  composerAttachmentInputRef.current?.click();
                }}
              >
                <Paperclip aria-hidden size={16} strokeWidth={1.75} />
                <span className="composer-menu-label">
                  {t('Add photos and files')}
                </span>
              </FloatingActionMenuItem>
              {onSelectBotBinding ? <DropdownMenuSeparator /> : null}
              {renderComposerBotBindingSubmenu({
                activeThreadBot,
                activeThreadBotId,
                botGroups,
                iconDataUrlByChannel,
                onSelectBotBinding,
                t,
              })}
            </FloatingActionMenuContent>
          </DropdownMenu>
          {composerContext}
        </div>
        <div className="composer-buttons">
          {renderComposerModelControl({
            providerModels: threadProviderModels ?? newThreadProviderModels,
            agentConfiguredModel: newThreadAgentConfiguredModel,
            effectiveModel: threadEffectiveModel ?? newThreadAgentConfiguredModel,
            effectiveReasoningEffort: threadEffectiveReasoningEffort,
            effectiveServiceTier: threadEffectiveServiceTier,
            selectedModel: threadSelectedModel ?? newThreadSelectedModel,
            selectedReasoningEffort:
              threadSelectedReasoningEffort ?? newThreadSelectedReasoningEffort,
            selectedServiceTier:
              threadSelectedServiceTier ?? newThreadSelectedServiceTier,
            onSelectModel: onSelectThreadModel ?? onSelectNewThreadModel,
            onSelectReasoningEffort:
              onSelectThreadReasoningEffort ?? onSelectNewThreadReasoningEffort,
            onSelectServiceTier:
              onSelectThreadServiceTier ?? onSelectNewThreadServiceTier,
            t,
          })}
          {renderComposerProviderControl({
            composerProviderType,
            agentLabel,
            agentOptions,
            selectedAgentId,
            onSelectAgent,
            t,
          })}
          {isActiveSendingThread ? (
            <button
              aria-label={t("Interrupt")}
              className="primary-button primary-send-button primary-stop-button"
              onClick={onInterrupt}
              type="button"
            >
              <Square aria-hidden fill="currentColor" size={14} strokeWidth={0} />
              <span className="sr-only">{t("Interrupt")}</span>
            </button>
          ) : (
            <button
              aria-label={t("Send")}
              className="primary-button primary-send-button"
              disabled={
                composerLocked ||
                (!composerHasPayload && activeQueueLength === 0)
              }
              type="submit"
            >
              <svg aria-hidden width="16" height="16" viewBox="0 0 20 20" fill="none">
                <path d="M9.33467 16.6663V4.93978L4.6374 9.63704L3.4585 8.45814L9.99967 1.91697L16.5408 8.45814L15.3619 9.63704L10.6647 4.93978V16.6663H9.33467Z" fill="currentColor"/>
              </svg>
              <span className="sr-only">{t("Send")}</span>
            </button>
          )}
        </div>
      </div>
    </form>
  );
}
