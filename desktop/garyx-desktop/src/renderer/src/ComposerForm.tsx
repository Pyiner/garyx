import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type ChangeEvent,
  type ClipboardEvent,
  type CompositionEvent,
  type DragEvent,
  type FormEvent,
  type KeyboardEvent,
  type RefObject,
} from 'react';
import { createPortal } from 'react-dom';

import {
  IconArrowsMinimize,
  IconBolt,
  IconBrain,
  IconCode,
  IconCloud,
  IconCommand,
  IconCube,
  IconFileText,
  IconGitBranch,
  IconInfoCircle,
  IconMessageCircle,
  IconPaperclip,
  IconPlayerStopFilled,
  IconPlugConnected,
  IconPlus,
  IconServer,
  IconSettings,
  IconTerminal2,
  IconUserCircle,
  IconX,
  type Icon,
} from '@tabler/icons-react';

import type {
  DesktopBotConsoleSummary,
  DesktopApiProviderType,
  MessageFileAttachment,
  MessageImageAttachment,
  SlashCommand,
} from '@shared/contracts';

import {
  DropdownMenu,
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
import { AgentsIcon } from './app-shell/icons';

export type { ComposerAgentOption };

import { ChannelLogo } from './channel-logo';
import { useChannelPluginCatalog } from './channel-plugins/useChannelPluginCatalog';
import { buildMessageImageDataUrl } from './message-rich-content';
import { useI18n, type Translate } from './i18n';

type ComposerFormProps = {
  activeQueueLength: number;
  composer: string;
  composerAttachmentInputRef: RefObject<HTMLInputElement | null>;
  composerFiles: MessageFileAttachment[];
  composerHasPayload: boolean;
  composerImages: MessageImageAttachment[];
  composerLocked: boolean;
  composerPlaceholder: string;
  composerProviderType: DesktopApiProviderType;
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
  isActiveSendingThread: boolean;
  onAppendComposerAttachments: (files: File[]) => void;
  onComposerChange: (value: string) => void;
  onComposerCompositionEnd: (value: string) => void;
  onComposerCompositionStart: () => void;
  onComposerKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => void;
  onComposerPasteFiles: (files: File[]) => void;
  onInterrupt: () => void;
  onRemoveComposerFile: (fileId: string) => void;
  onRemoveComposerImage: (imageId: string) => void;
  onSelectBotBinding?: (botId: string | null) => void;
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  slashPanelContainerRef: RefObject<HTMLDivElement | null>;
  slashCommands: SlashCommand[];
  slashCommandsLoaded: boolean;
  slashCommandsLoading: boolean;
};

function providerOptionLabel(providerType: DesktopApiProviderType): string {
  if (providerType === 'codex_app_server') {
    return 'Codex';
  }
  if (providerType === 'gemini_cli') {
    return 'Gemini';
  }
  return 'Claude';
}

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

function slashCommandIcon(command: SlashCommand): Icon {
  const key = command.name.toLowerCase();
  if (key.includes('mcp') || key.includes('server')) {
    return IconServer;
  }
  if (key.includes('status') || key.includes('info')) {
    return IconInfoCircle;
  }
  if (key.includes('branch') || key.includes('worktree')) {
    return IconGitBranch;
  }
  if (key.includes('fast') || key.includes('quick')) {
    return IconBolt;
  }
  if (key.includes('reason')) {
    return IconBrain;
  }
  if (key.includes('model')) {
    return IconCube;
  }
  if (key.includes('compact') || key.includes('compress')) {
    return IconArrowsMinimize;
  }
  if (key.includes('review') || key.includes('code')) {
    return IconCode;
  }
  if (key.includes('feedback')) {
    return IconMessageCircle;
  }
  if (key.includes('profile') || key.includes('persona')) {
    return IconUserCircle;
  }
  if (key.includes('cloud') || key.includes('sync')) {
    return IconCloud;
  }
  if (key.includes('setting') || key.includes('config')) {
    return IconSettings;
  }
  if (key.includes('terminal') || key.includes('shell')) {
    return IconTerminal2;
  }
  return IconCommand;
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
  const providerIcon = AGENT_PROVIDER_GLYPH;
  const providerLabel = agentLabel || providerOptionLabel(composerProviderType);

  if (onSelectAgent) {
    const grouped = groupAgentOptions(agentOptions ?? []);
    const hasAgents = grouped.agent.length > 0;
    const hasTeams = grouped.team.length > 0;
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
              onSelect={() => onSelectAgent(option.id)}
            >
              {option.label}
            </FloatingActionMenuItem>
          ))}
          {hasAgents || hasTeams ? <DropdownMenuSeparator /> : null}
          {hasAgents ? (
            <DropdownMenuSub>
              <FloatingActionMenuSubTrigger>{t("Agents")}</FloatingActionMenuSubTrigger>
              <FloatingActionMenuSubContent>
                {grouped.agent.map((option) => (
                  <FloatingActionMenuItem
                    data-active={
                      option.id === selectedAgentId ? '' : undefined
                    }
                    key={option.id}
                    onSelect={() => onSelectAgent(option.id)}
                  >
                    {option.detail
                      ? `${option.label} (${option.detail})`
                      : option.label}
                  </FloatingActionMenuItem>
                ))}
              </FloatingActionMenuSubContent>
            </DropdownMenuSub>
          ) : null}
          {hasTeams ? (
            <DropdownMenuSub>
              <FloatingActionMenuSubTrigger>{t("Agent Teams")}</FloatingActionMenuSubTrigger>
              <FloatingActionMenuSubContent>
                {grouped.team.map((option) => (
                  <FloatingActionMenuItem
                    data-active={
                      option.id === selectedAgentId ? '' : undefined
                    }
                    key={option.id}
                    onSelect={() => onSelectAgent(option.id)}
                  >
                    {option.label}
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
          <IconPlugConnected aria-hidden size={15} stroke={1.7} />
        )}
        <span className="composer-menu-label">{selectedBot?.title || t('Bind bot')}</span>
      </FloatingActionMenuSubTrigger>
      <FloatingActionMenuSubContent className="composer-bot-submenu">
        <FloatingActionMenuItem
          className="composer-bot-menu-item"
          data-active={!activeThreadBotId ? '' : undefined}
          onSelect={() => {
            onSelectBotBinding(null);
          }}
        >
          <IconPlugConnected aria-hidden size={16} stroke={1.7} />
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
  composerAttachmentInputRef,
  composerFiles,
  composerHasPayload,
  composerImages,
  composerLocked,
  composerPlaceholder,
  composerProviderType,
  composerTextareaRef,
  activeThreadBot,
  activeThreadBotId,
  botBindingDisabled = false,
  botGroups,
  agentLabel,
  agentOptions,
  selectedAgentId,
  onSelectAgent,
  isActiveSendingThread,
  onAppendComposerAttachments,
  onComposerChange,
  onComposerCompositionEnd,
  onComposerCompositionStart,
  onComposerKeyDown,
  onComposerPasteFiles,
  onInterrupt,
  onRemoveComposerFile,
  onRemoveComposerImage,
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
  const [highlightedSlashCommandIndex, setHighlightedSlashCommandIndex] = useState(0);
  const [dismissedSlashQuery, setDismissedSlashQuery] = useState<string | null>(null);
  const slashCommandItemRefs = useRef<Array<HTMLButtonElement | null>>([]);

  const slashCursor = Math.max(
    0,
    Math.min(
      composerTextareaRef.current?.selectionStart ?? composerCursor,
      composer.length,
    ),
  );
  const slashTrigger = resolveSlashTrigger(composer, slashCursor);
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
      ? `${composer.slice(0, slashTrigger.start)}${insertion}${composer.slice(slashTrigger.end)}`
      : insertion;
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
      if ((event.key === 'Enter' || event.key === 'Tab') && highlightedSlashCommand) {
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
                  <CommandIcon aria-hidden size={15} stroke={1.65} />
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
      {composerImages.length || composerFiles.length ? (
        <div className="composer-attachment-strip">
          {composerImages.map((image) => (
            <div
              key={image.id}
              className="composer-attachment-chip composer-image-chip"
            >
              <img
                alt={image.name}
                className="composer-image-chip-preview"
                title={image.name}
                src={buildMessageImageDataUrl(image.mediaType, image.data || '')}
              />
              <button
                className="composer-image-chip-remove"
                onClick={() => {
                  onRemoveComposerImage(image.id);
                }}
                type="button"
              >
                <IconX aria-hidden size={10} stroke={2.2} />
                <span className="sr-only">{t("Remove image attachment")}</span>
              </button>
            </div>
          ))}
          {composerFiles.map((file) => (
            <div
              key={file.id}
              className="composer-attachment-chip composer-file-chip"
              title={file.name}
            >
              <span className="composer-file-chip-icon">
                <IconFileText aria-hidden size={14} stroke={1.8} />
              </span>
              <span className="composer-file-chip-label">{file.name}</span>
              <button
                className="composer-image-chip-remove"
                onClick={() => {
                  onRemoveComposerFile(file.id);
                }}
                type="button"
              >
                <IconX aria-hidden size={10} stroke={2.2} />
                <span className="sr-only">{t("Remove file attachment")}</span>
              </button>
            </div>
          ))}
        </div>
      ) : null}
      <textarea
        className="composer-editor"
        ref={composerTextareaRef}
        disabled={composerLocked}
        value={composer}
        onChange={(event) => {
          syncComposerCursor(event.target.selectionStart);
          onComposerChange(event.target.value);
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
        <DropdownMenu>
          <DropdownMenuTrigger
            aria-label={t('Composer actions')}
            className="ghost-button composer-plus-trigger"
            disabled={composerLocked && botBindingDisabled}
            type="button"
          >
            <IconPlus aria-hidden size={18} stroke={1.8} />
          </DropdownMenuTrigger>
          <FloatingActionMenuContent
            align="start"
            side="top"
          >
            <FloatingActionMenuItem
              className="composer-menu-item"
              disabled={composerLocked}
              onSelect={() => {
                composerAttachmentInputRef.current?.click();
              }}
            >
              <IconPaperclip aria-hidden size={16} stroke={1.75} />
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
        <div className="composer-buttons">
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
              <IconPlayerStopFilled aria-hidden size={14} />
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
