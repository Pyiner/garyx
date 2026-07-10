import { useState } from 'react';
import { Plus, Trash } from 'lucide-react';

import type {
  SlashCommand,
  UpdateSlashCommandInput,
  UpsertSlashCommandInput,
} from '@shared/contracts';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table';
import { Textarea } from '@/components/ui/textarea';
import { MoreDotsIcon } from '../app-shell/icons';
import { useI18n } from '../i18n';
import { classNames, noopAsync } from './shared';

const SLASH_COMMAND_NAME_PATTERN = /^[a-z0-9_]{1,32}$/;

type CommandDraft = UpsertSlashCommandInput;

type CommandsSettingsPanelProps = {
  commands?: SlashCommand[];
  commandsLoading?: boolean;
  commandsSaving?: boolean;
  onCreateSlashCommand?: (input: UpsertSlashCommandInput) => Promise<void>;
  onUpdateSlashCommand?: (input: UpdateSlashCommandInput) => Promise<void>;
  onDeleteSlashCommand?: (name: string) => Promise<void>;
};

function emptyCommandDraft(): CommandDraft {
  return {
    name: '',
    description: '',
    prompt: '',
  };
}

function commandDraftFromValue(command: SlashCommand): CommandDraft {
  return {
    name: command.name,
    description: command.description,
    prompt: command.prompt || '',
  };
}

function deriveSlashCommandDescription(prompt: string, name: string): string {
  const normalized = prompt.replace(/\s+/g, ' ').trim();
  if (!normalized) {
    return `/${name}`;
  }
  if (normalized.length <= 80) {
    return normalized;
  }
  return `${normalized.slice(0, 79).trimEnd()}…`;
}

export function CommandsSettingsPanel({
  commands = [],
  commandsLoading = false,
  commandsSaving = false,
  onCreateSlashCommand = noopAsync,
  onUpdateSlashCommand = noopAsync,
  onDeleteSlashCommand = noopAsync,
}: CommandsSettingsPanelProps) {
  const { t } = useI18n();
  const [editingCommandName, setEditingCommandName] = useState<string | null>(null);
  const [commandDraft, setCommandDraft] = useState<CommandDraft>(() => emptyCommandDraft());
  const [commandDialogOpen, setCommandDialogOpen] = useState(false);

  const normalizedCommandDraftName = commandDraft.name.trim().toLowerCase();
  const commandDraftPrompt = commandDraft.prompt?.trim() || '';
  const commandNameTaken = commands.some((command) => {
    return command.name === normalizedCommandDraftName && command.name !== editingCommandName;
  });
  const commandDraftReady = Boolean(
    SLASH_COMMAND_NAME_PATTERN.test(normalizedCommandDraftName)
      && commandDraftPrompt,
  );
  const commandDraftValidationMessage = commandNameTaken
    ? t('A command with this name already exists.')
    : normalizedCommandDraftName && !SLASH_COMMAND_NAME_PATTERN.test(normalizedCommandDraftName)
      ? t('Command names only support lowercase letters, numbers, and underscores, up to 32 characters.')
      : !commandDraftPrompt
        ? t('Enter command content.')
          : t('The command will be added to the list after saving.');
  const commandPromptPreview = (command: SlashCommand) => {
    const preview = (command.prompt || command.description || '').trim();
    return preview.length > 140 ? `${preview.slice(0, 137)}…` : preview;
  };

  function resetCommandEditor() {
    setEditingCommandName(null);
    setCommandDraft(emptyCommandDraft());
  }

  function closeCommandDialog() {
    setCommandDialogOpen(false);
    resetCommandEditor();
  }

  function openCreateCommandDialog() {
    resetCommandEditor();
    setCommandDialogOpen(true);
  }

  function openEditCommandDialog(command: SlashCommand) {
    setEditingCommandName(command.name);
    setCommandDraft(commandDraftFromValue(command));
    setCommandDialogOpen(true);
  }

  async function handleSaveCommandDraft() {
    if (!commandDraftReady || commandNameTaken) {
      return;
    }

    const payload: UpsertSlashCommandInput = {
      name: normalizedCommandDraftName,
      description: deriveSlashCommandDescription(commandDraftPrompt, normalizedCommandDraftName),
      prompt: commandDraftPrompt || null,
    };

    if (editingCommandName) {
      await onUpdateSlashCommand({
        ...payload,
        currentName: editingCommandName,
      });
    } else {
      await onCreateSlashCommand(payload);
    }
    closeCommandDialog();
  }

  async function handleDeleteCommandClick(name: string) {
    await onDeleteSlashCommand(name);
    if (editingCommandName === name) {
      closeCommandDialog();
    }
  }

  return (
    <>
      <div className="codex-section">
        <div className="codex-section-header">
          <span className="codex-section-title">{t('Command List')}</span>
          <div className="codex-list-row-actions">
            <button
              className="codex-section-action"
              onClick={openCreateCommandDialog}
              type="button"
            >
              <Plus aria-hidden size={14} />
              {t('Add Command')}
            </button>
          </div>
        </div>

        {commandsLoading ? (
          <div className="commands-empty-state">
            <strong>{t('Loading shortcuts...')}</strong>
            <span>{t('Global prompt shortcuts are loaded from the current Gateway config.')}</span>
          </div>
        ) : commands.length ? (
          <div className="commands-table">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="commands-table-col-command">{t('Command')}</TableHead>
                  <TableHead className="commands-table-col-description">{t('Description')}</TableHead>
                  <TableHead className="commands-table-col-prompt">{t('Prompt')}</TableHead>
                  <TableHead className="commands-table-col-actions">{t('Actions')}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {commands.map((command) => (
                  <TableRow
                    data-testid={`slash-command-card-${command.name}`}
                    key={command.name}
                  >
                    <TableCell className="commands-table-col-command">
                      <span className="command-table-slash">/{command.name}</span>
                    </TableCell>
                    <TableCell
                      className="commands-table-col-description"
                      title={command.description || t('Prompt shortcut')}
                    >
                      {command.description || t('Prompt shortcut')}
                    </TableCell>
                    <TableCell
                      className="commands-table-col-prompt"
                      title={commandPromptPreview(command) || t('No prompt configured.')}
                    >
                      {commandPromptPreview(command) || t('No prompt configured.')}
                    </TableCell>
                    <TableCell className="commands-table-col-actions">
                      <div className="command-list-actions">
                        <button
                          className="command-row-action"
                          onClick={() => { openEditCommandDialog(command); }}
                          type="button"
                        >
                          {t('Edit')}
                        </button>
                        <DropdownMenu>
                          <DropdownMenuTrigger asChild>
                            <button
                              aria-label={t('More actions for {name}', { name: `/${command.name}` })}
                              className="bot-table-action-button"
                              disabled={commandsSaving}
                              type="button"
                            >
                              <MoreDotsIcon size={14} />
                            </button>
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end">
                            <DropdownMenuItem
                              disabled={commandsSaving}
                              onSelect={() => { void handleDeleteCommandClick(command.name); }}
                              variant="destructive"
                            >
                              <Trash aria-hidden />
                              {t('Delete')}
                            </DropdownMenuItem>
                          </DropdownMenuContent>
                        </DropdownMenu>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        ) : (
          <div className="commands-empty-state">
            <strong>{t('No shortcuts yet')}</strong>
            <span>{t('Click Add Command above to create a prompt shortcut like /summary.')}</span>
          </div>
        )}
      </div>
      <Dialog
        open={commandDialogOpen}
        onOpenChange={(open) => {
          if (!open) {
            closeCommandDialog();
          }
        }}
      >
        <DialogContent
          className="commands-dialog"
          showCloseButton={false}
          size="form"
        >
          <DialogHeader className="commands-dialog-header">
            <Badge
              variant="outline"
              className="commands-dialog-badge"
            >
              {editingCommandName ? t('Edit Command') : t('Add Command')}
            </Badge>
            <div className="commands-dialog-title-group">
              <DialogTitle className="commands-dialog-title">
                {editingCommandName ? t('Edit /{name}', { name: editingCommandName }) : t('Add Command')}
              </DialogTitle>
              <DialogDescription className="commands-dialog-description">
                {t('Only the command name and content are needed. Telegram descriptions are generated on save.')}
              </DialogDescription>
            </div>
          </DialogHeader>

          <div className="commands-dialog-body">
            <div className="commands-field">
              <div className="commands-field-header">
                <Label className="commands-field-label" htmlFor="slash-command-name">{t('Command name')}</Label>
                <span className="commands-field-hint">{t('Only a-z, 0-9, and _')}</span>
              </div>
              <div className="commands-name-input">
                <span aria-hidden>/</span>
                <Input
                  className="commands-name-control"
                  id="slash-command-name"
                  placeholder="summary"
                  value={commandDraft.name}
                  onChange={(event) => {
                    setCommandDraft((current) => ({
                      ...current,
                      name: event.target.value.toLowerCase(),
                    }));
                  }}
                />
              </div>
            </div>

            <div className="commands-field">
              <div className="commands-field-header">
                <Label className="commands-field-label" htmlFor="slash-command-prompt">{t('Content')}</Label>
                <span className="commands-field-hint">{t('This prompt runs when /command is invoked.')}</span>
              </div>
              <Textarea
                className="commands-prompt-control"
                id="slash-command-prompt"
                placeholder={t('Summarize the key points of our conversation.')}
                value={String(commandDraft.prompt || '')}
                onChange={(event) => {
                  setCommandDraft((current) => ({
                    ...current,
                    prompt: event.target.value,
                  }));
                }}
              />
            </div>

            <p className={classNames('small-note commands-modal-note', (commandNameTaken || !commandDraftReady) && 'error')}>
              {commandDraftValidationMessage}
            </p>
          </div>

          <DialogFooter className="commands-dialog-footer">
            <Button
              className="commands-dialog-button secondary"
              onClick={closeCommandDialog}
              type="button"
              variant="outline"
            >
              {t('Cancel')}
            </Button>
            <Button
              className="commands-dialog-button primary"
              disabled={!commandDraftReady || commandNameTaken || commandsSaving}
              onClick={() => {
                void handleSaveCommandDraft();
              }}
              type="button"
            >
              {commandsSaving ? t('Saving…') : t('Save Command')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
