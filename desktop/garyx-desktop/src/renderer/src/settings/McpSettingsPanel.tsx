import { useState } from 'react';
import { Plus, Trash } from 'lucide-react';

import type {
  DesktopMcpServer,
  DesktopWorkspace,
  McpTransportType,
  UpdateMcpServerInput,
  UpsertMcpServerInput,
} from '@shared/contracts';
import { Button } from '@/components/ui/button';
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Switch } from '@/components/ui/switch';
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group';
import { WorkspacePathPicker } from '../components/WorkspacePathPicker';
import { useI18n } from '../i18n';
import { classNames, noopAsync } from './shared';

type McpServerDraft = {
  name: string;
  transport: McpTransportType;
  // STDIO
  command: string;
  args: string[];
  envEntries: Array<{ key: string; value: string }>;
  workingDir: string;
  // Streamable HTTP
  url: string;
  headerEntries: Array<{ key: string; value: string }>;
  // Common
  enabled: boolean;
};

function emptyMcpServerDraft(): McpServerDraft {
  return {
    name: '',
    transport: 'stdio',
    command: '',
    args: [''],
    envEntries: [{ key: '', value: '' }],
    workingDir: '',
    url: '',
    headerEntries: [{ key: '', value: '' }],
    enabled: true,
  };
}

function mcpServerDraftFromValue(server: DesktopMcpServer): McpServerDraft {
  const envEntries = Object.entries(server.env || {});
  const headerEntries = Object.entries(server.headers || {});
  return {
    name: server.name,
    transport: server.transport || 'stdio',
    command: server.command,
    args: server.args.length ? [...server.args] : [''],
    envEntries: envEntries.length
      ? envEntries.map(([key, value]) => ({ key, value }))
      : [{ key: '', value: '' }],
    workingDir: server.workingDir || '',
    url: server.url || '',
    headerEntries: headerEntries.length
      ? headerEntries.map(([key, value]) => ({ key, value }))
      : [{ key: '', value: '' }],
    enabled: server.enabled,
  };
}

type McpSettingsPanelProps = {
  mcpServers?: DesktopMcpServer[];
  mcpServersLoading?: boolean;
  mcpServersSaving?: boolean;
  workspaces?: DesktopWorkspace[];
  onAddWorkspace?: (path: string) => Promise<DesktopWorkspace | null>;
  onCreateMcpServer?: (input: UpsertMcpServerInput) => Promise<void>;
  onToggleMcpServer?: (name: string, enabled: boolean) => Promise<void>;
  onUpdateMcpServer?: (input: UpdateMcpServerInput) => Promise<void>;
  onDeleteMcpServer?: (name: string) => Promise<void>;
};

export function McpSettingsPanel({
  mcpServers = [],
  mcpServersLoading = false,
  mcpServersSaving = false,
  workspaces = [],
  onAddWorkspace = async () => null,
  onCreateMcpServer = noopAsync,
  onToggleMcpServer = noopAsync,
  onUpdateMcpServer = noopAsync,
  onDeleteMcpServer = noopAsync,
}: McpSettingsPanelProps) {
  const { t } = useI18n();
  const [editingMcpServerName, setEditingMcpServerName] = useState<string | null>(null);
  const [mcpServerDraft, setMcpServerDraft] = useState<McpServerDraft>(() => emptyMcpServerDraft());
  const [mcpDialogOpen, setMcpDialogOpen] = useState(false);

  const normalizedMcpServerName = mcpServerDraft.name.trim();
  const normalizedMcpServerCommand = mcpServerDraft.command.trim();
  const normalizedMcpUrl = mcpServerDraft.url.trim();
  const normalizedMcpArgs = mcpServerDraft.args
    .map((value) => value.trim())
    .filter(Boolean);
  const mcpServerNameTaken = mcpServers.some((server) => {
    return server.name === normalizedMcpServerName && server.name !== editingMcpServerName;
  });
  const mcpTransportReady = mcpServerDraft.transport === 'stdio'
    ? Boolean(normalizedMcpServerCommand)
    : Boolean(normalizedMcpUrl);
  const mcpServerDraftReady = Boolean(
    normalizedMcpServerName
      && mcpTransportReady
      && !mcpServerNameTaken,
  );
  const mcpServerDraftValidationMessage = mcpServerNameTaken
    ? t('An MCP server with this name already exists.')
    : !normalizedMcpServerName
      ? t('Enter a server name.')
      : mcpServerDraft.transport === 'stdio' && !normalizedMcpServerCommand
        ? t('Enter a start command.')
        : mcpServerDraft.transport === 'streamable_http' && !normalizedMcpUrl
          ? t('Enter a URL.')
          : t('Saving updates garyx.json on the gateway and syncs local Claude / Codex MCP config.');

  function resetMcpServerEditor() {
    setEditingMcpServerName(null);
    setMcpServerDraft(emptyMcpServerDraft());
  }

  function closeMcpDialog() {
    setMcpDialogOpen(false);
    resetMcpServerEditor();
  }

  function openCreateMcpDialog() {
    resetMcpServerEditor();
    setMcpDialogOpen(true);
  }

  function openEditMcpDialog(server: DesktopMcpServer) {
    setEditingMcpServerName(server.name);
    setMcpServerDraft(mcpServerDraftFromValue(server));
    setMcpDialogOpen(true);
  }

  async function handleSaveMcpServerDraft() {
    if (!mcpServerDraftReady) {
      return;
    }

    const payload: UpsertMcpServerInput = {
      name: normalizedMcpServerName,
      transport: mcpServerDraft.transport,
      enabled: mcpServerDraft.enabled,
      ...(mcpServerDraft.transport === 'stdio'
        ? {
            command: normalizedMcpServerCommand,
            args: normalizedMcpArgs,
            env: Object.fromEntries(
              mcpServerDraft.envEntries.flatMap(({ key, value }) => {
                const normalizedKey = key.trim();
                return normalizedKey ? [[normalizedKey, value]] : [];
              }),
            ),
            workingDir: mcpServerDraft.workingDir.trim() || null,
          }
        : {
            url: normalizedMcpUrl,
            headers: Object.fromEntries(
              mcpServerDraft.headerEntries.flatMap(({ key, value }) => {
                const normalizedKey = key.trim();
                return normalizedKey ? [[normalizedKey, value]] : [];
              }),
            ),
          }),
    };

    if (editingMcpServerName) {
      await onUpdateMcpServer({
        ...payload,
        currentName: editingMcpServerName,
      });
    } else {
      await onCreateMcpServer(payload);
    }
    closeMcpDialog();
  }

  async function handleDeleteMcpServer(name: string) {
    if (!window.confirm(t('Delete MCP server "{name}"?', { name }))) return;
    await onDeleteMcpServer(name);
    if (editingMcpServerName === name) {
      closeMcpDialog();
    }
  }

  return (
    <>
      <div className="codex-section">
        <div className="codex-section-header">
          <span className="codex-section-title">{t('Custom Servers')}</span>
          <button
            className="codex-section-action"
            disabled={mcpServersSaving}
            onClick={openCreateMcpDialog}
            type="button"
          >
            <Plus aria-hidden size={14} />
            {t('Add Server')}
          </button>
        </div>
        {mcpServersLoading ? (
          <div className="codex-empty-state">{t('Loading current config...')}</div>
        ) : mcpServers.length ? (
          <div className="codex-list-card">
            {mcpServers.map((server) => (
              <div
                className="codex-list-row"
                data-testid={`mcp-server-card-${server.name}`}
                key={server.name}
              >
                <span className="codex-list-row-name">{server.name}</span>
                <div className="codex-list-row-actions">
                  <button
                    className="codex-icon-button"
                    onClick={() => { openEditMcpDialog(server); }}
                    title={t('Configure')}
                    type="button"
                  >
                    <svg aria-hidden width="18" height="18" viewBox="0 0 21 21" fill="none">
                      <path d="M10.7228 2.53564C11.5515 2.53564 12.3183 2.97502 12.7374 3.68994L13.5587 5.09033L13.6124 5.15967C13.6736 5.22007 13.7566 5.2556 13.8448 5.25635L15.4601 5.26904L15.6144 5.27588C16.3826 5.33292 17.0775 5.76649 17.465 6.43994L17.7931 7.01123L17.8663 7.14697C18.1815 7.78943 18.1843 8.54208 17.8741 9.18701L17.8028 9.32275L17.0001 10.7446C16.9427 10.8467 16.9426 10.9717 17.0001 11.0737L17.8028 12.4946L17.8741 12.6313C18.1842 13.2763 18.1816 14.029 17.8663 14.6714L17.7931 14.8071L17.465 15.3784C17.0774 16.0517 16.3825 16.4855 15.6144 16.5425L15.4601 16.5483L13.8448 16.562C13.7565 16.5628 13.6736 16.5982 13.6124 16.6587L13.5587 16.7271L12.7374 18.1284C12.3183 18.8432 11.5514 19.2827 10.7228 19.2827H10.0763C9.29958 19.2826 8.57714 18.8964 8.14465 18.2593L8.06261 18.1284L7.24133 16.7271C7.1966 16.6509 7.12417 16.5966 7.04113 16.5737L6.95519 16.562L5.33996 16.5483C4.56297 16.542 3.84347 16.1503 3.41613 15.5093L3.33508 15.3784L3.00695 14.8071C2.59564 14.0921 2.59168 13.2129 2.99719 12.4946L3.79894 11.0737L3.83215 10.9937C3.84657 10.9383 3.84652 10.88 3.83215 10.8247L3.79894 10.7446L2.99719 9.32275C2.59184 8.60451 2.59571 7.72612 3.00695 7.01123L3.33508 6.43994L3.41613 6.30908C3.84345 5.66796 4.56288 5.27538 5.33996 5.26904L6.95519 5.25635L7.04113 5.24463C7.12427 5.22177 7.1966 5.16664 7.24133 5.09033L8.06261 3.68994L8.14465 3.55908C8.57712 2.92179 9.29949 2.5358 10.0763 2.53564H10.7228ZM10.0763 3.86572C9.76448 3.86587 9.47308 4.01039 9.28429 4.25244L9.21008 4.36279L8.38879 5.76318C8.12941 6.20571 7.68297 6.49995 7.18273 6.56982L6.96594 6.58643L5.3507 6.59912C5.03877 6.60167 4.74854 6.74903 4.56164 6.99268L4.48742 7.10303L4.15929 7.67432C3.98236 7.98202 3.98089 8.36033 4.15539 8.66943L4.95715 10.0903L5.05187 10.2856C5.21318 10.6851 5.21302 11.1323 5.05187 11.5317L4.95715 11.728L4.15539 13.1489C3.98092 13.4581 3.98228 13.8363 4.15929 14.144L4.48742 14.7144L4.56164 14.8247C4.74853 15.0686 5.03859 15.2157 5.3507 15.2183L6.96594 15.2319L7.18273 15.2476C7.68301 15.3174 8.12939 15.6126 8.38879 16.0552L9.21008 17.4556L9.28429 17.5649C9.47307 17.8072 9.76431 17.9525 10.0763 17.9526H10.7228C11.0794 17.9526 11.4096 17.7632 11.59 17.4556L12.4112 16.0552L12.5333 15.8745C12.8433 15.4758 13.3212 15.2361 13.8341 15.2319L15.4493 15.2183L15.5812 15.2085C15.8855 15.1657 16.1569 14.985 16.3126 14.7144L16.6407 14.144L16.6984 14.0259C16.7984 13.7835 16.8 13.5113 16.7023 13.2681L16.6446 13.1489L15.8419 11.728C15.5551 11.2201 15.5552 10.5983 15.8419 10.0903L16.6446 8.66943L16.7023 8.55029C16.8001 8.30708 16.7983 8.03486 16.6984 7.79248L16.6407 7.67432L16.3126 7.10303C16.1569 6.8324 15.8856 6.65166 15.5812 6.60889L15.4493 6.59912L13.8341 6.58643C13.3213 6.58224 12.8433 6.34243 12.5333 5.94385L12.4112 5.76318L11.59 4.36279C11.4096 4.05506 11.0795 3.86572 10.7228 3.86572H10.0763ZM11.9855 10.9087C11.9853 10.0336 11.2755 9.32399 10.4005 9.32373C9.52524 9.32373 8.81474 10.0335 8.81457 10.9087C8.81457 11.7841 9.52513 12.4937 10.4005 12.4937C11.2757 12.4934 11.9855 11.7839 11.9855 10.9087ZM13.3146 10.9087C13.3146 12.5184 12.0102 13.8235 10.4005 13.8237C8.7906 13.8237 7.48547 12.5186 7.48547 10.9087C7.48564 9.29893 8.7907 7.99365 10.4005 7.99365C12.0101 7.99391 13.3144 9.29909 13.3146 10.9087Z" fill="currentColor"/>
                    </svg>
                  </button>
                  <Switch
                    aria-label={`${server.name} enabled`}
                    checked={server.enabled}
                    onCheckedChange={(nextValue) => {
                      void onToggleMcpServer(server.name, nextValue);
                    }}
                  />
                  <button
                    aria-label={t('Delete {name}', { name: server.name })}
                    className="codex-icon-button codex-icon-button-danger"
                    disabled={mcpServersSaving}
                    onClick={() => { void handleDeleteMcpServer(server.name); }}
                    title={t('Delete')}
                    type="button"
                  >
                    <Trash aria-hidden />
                  </button>
                </div>
              </div>
            ))}
          </div>
        ) : (
          <div className="codex-empty-state">{t('No MCP servers yet. Click Add Server above to create one.')}</div>
        )}
      </div>
      <Dialog
        open={mcpDialogOpen}
        onOpenChange={(open) => {
          if (!open) {
            closeMcpDialog();
          }
        }}
      >
        <DialogContent
          className="max-w-[520px] rounded-[12px] border-[#e8e8e5] bg-white p-0 shadow-[0_8px_24px_rgba(0,0,0,0.08)] gap-0"
          showCloseButton={false}
          size="narrow"
        >
          <DialogHeader className="border-b border-[#efefec] px-4 py-3">
            <DialogTitle className="text-[14px] font-semibold tracking-[-0.01em] text-[#111111]">
              {editingMcpServerName ? t('Edit {name}', { name: editingMcpServerName }) : t('Add Server')}
            </DialogTitle>
          </DialogHeader>

          <div className="space-y-3 px-4 py-4">
            <div className="grid gap-3 md:grid-cols-[1fr_auto]">
              <div className="space-y-1.5">
                <Label className="text-[11px] font-medium text-[#666663]">{t('Name')}</Label>
                <Input
                  className="h-8 rounded-[8px] border-[#e7e7e5] bg-white text-[13px] shadow-none"
                  placeholder={t('MCP server name')}
                  value={mcpServerDraft.name}
                  onChange={(event) => {
                    setMcpServerDraft((current) => ({
                      ...current,
                      name: event.target.value,
                    }));
                  }}
                />
              </div>

              <div className="space-y-1.5">
                <Label className="text-[11px] font-medium text-[#666663]">{t('Transport')}</Label>
                <ToggleGroup
                  className="h-8 rounded-[8px] bg-[#f3f3f1] p-0.5"
                  type="single"
                  value={mcpServerDraft.transport}
                  onValueChange={(nextValue) => {
                    if (!nextValue) {
                      return;
                    }
                    setMcpServerDraft((current) => ({
                      ...current,
                      transport: nextValue as McpTransportType,
                    }));
                  }}
                >
                  <ToggleGroupItem
                    className="h-7 rounded-[6px] border-0 px-3 text-[11px] text-[#666663] data-[state=on]:text-[#111111]"
                    value="stdio"
                  >
                    STDIO
                  </ToggleGroupItem>
                  <ToggleGroupItem
                    className="h-7 rounded-[6px] border-0 px-3 text-[11px] text-[#666663] data-[state=on]:text-[#111111]"
                    value="streamable_http"
                  >
                    HTTP
                  </ToggleGroupItem>
                </ToggleGroup>
              </div>
            </div>

            {mcpServerDraft.transport === 'stdio' ? (
              <>
                <div className="space-y-1.5">
                  <Label className="text-[11px] font-medium text-[#666663]">{t('Start command')}</Label>
                  <Input
                    className="h-8 rounded-[8px] border-[#e7e7e5] bg-white text-[13px] shadow-none"
                    placeholder="openai-dev-mcp serve-sqlite"
                    value={mcpServerDraft.command}
                    onChange={(event) => {
                      setMcpServerDraft((current) => ({
                        ...current,
                        command: event.target.value,
                      }));
                    }}
                  />
                </div>

                <div className="space-y-1.5">
                  <div className="flex items-center justify-between gap-2">
                    <Label className="text-[11px] font-medium text-[#666663]">{t('Arguments')}</Label>
                    <button
                      className="text-[11px] text-[#666663] hover:text-[#111111]"
                      onClick={() => {
                        setMcpServerDraft((current) => ({
                          ...current,
                          args: [...current.args, ''],
                        }));
                      }}
                      type="button"
                    >
                      + {t('Add')}
                    </button>
                  </div>
                  <div className="space-y-1.5">
                    {mcpServerDraft.args.map((value, index) => (
                      <div className="grid gap-1.5 md:grid-cols-[1fr_auto]" key={`arg-${index}`}>
                        <Input
                          className="h-8 rounded-[8px] border-[#e7e7e5] bg-white text-[13px] shadow-none"
                          value={value}
                          onChange={(event) => {
                            setMcpServerDraft((current) => ({
                              ...current,
                              args: current.args.map((entry, entryIndex) => {
                                return entryIndex === index ? event.target.value : entry;
                              }),
                            }));
                          }}
                        />
                        <button
                          className="px-2 text-[11px] text-[#9b3d3d] hover:text-[#7a2f2f] disabled:cursor-not-allowed disabled:text-[#c7c7c4]"
                          disabled={mcpServerDraft.args.length <= 1}
                          onClick={() => {
                            setMcpServerDraft((current) => ({
                              ...current,
                              args: current.args.filter((_, entryIndex) => entryIndex !== index),
                            }));
                          }}
                          type="button"
                        >
                          {t('Delete')}
                        </button>
                      </div>
                    ))}
                  </div>
                </div>

                <div className="space-y-1.5">
                  <div className="flex items-center justify-between gap-2">
                    <Label className="text-[11px] font-medium text-[#666663]">{t('Environment variables')}</Label>
                    <button
                      className="text-[11px] text-[#666663] hover:text-[#111111]"
                      onClick={() => {
                        setMcpServerDraft((current) => ({
                          ...current,
                          envEntries: [...current.envEntries, { key: '', value: '' }],
                        }));
                      }}
                      type="button"
                    >
                      + {t('Add')}
                    </button>
                  </div>
                  <div className="space-y-1.5">
                    {mcpServerDraft.envEntries.map((entry, index) => (
                      <div className="grid gap-1.5 md:grid-cols-[0.9fr_1.1fr_auto]" key={`env-${index}`}>
                        <Input
                          className="h-8 rounded-[8px] border-[#e7e7e5] bg-white text-[13px] shadow-none"
                          placeholder={t('Key')}
                          value={entry.key}
                          onChange={(event) => {
                            setMcpServerDraft((current) => ({
                              ...current,
                              envEntries: current.envEntries.map((currentEntry, entryIndex) => {
                                return entryIndex === index
                                  ? { ...currentEntry, key: event.target.value }
                                  : currentEntry;
                              }),
                            }));
                          }}
                        />
                        <Input
                          className="h-8 rounded-[8px] border-[#e7e7e5] bg-white text-[13px] shadow-none"
                          placeholder={t('Value')}
                          value={entry.value}
                          onChange={(event) => {
                            setMcpServerDraft((current) => ({
                              ...current,
                              envEntries: current.envEntries.map((currentEntry, entryIndex) => {
                                return entryIndex === index
                                  ? { ...currentEntry, value: event.target.value }
                                  : currentEntry;
                              }),
                            }));
                          }}
                        />
                        <button
                          className="px-2 text-[11px] text-[#9b3d3d] hover:text-[#7a2f2f] disabled:cursor-not-allowed disabled:text-[#c7c7c4]"
                          disabled={mcpServerDraft.envEntries.length <= 1}
                          onClick={() => {
                            setMcpServerDraft((current) => ({
                              ...current,
                              envEntries: current.envEntries.filter((_, entryIndex) => entryIndex !== index),
                            }));
                          }}
                          type="button"
                        >
                          {t('Delete')}
                        </button>
                      </div>
                    ))}
                  </div>
                </div>

                <div className="space-y-1.5">
                  <Label className="text-[11px] font-medium text-[#666663]">{t('Working directory')}</Label>
                  <WorkspacePathPicker
                    allowEmpty
                    onAddWorkspace={onAddWorkspace}
                    onChange={(value) => {
                      setMcpServerDraft((current) => ({
                        ...current,
                        workingDir: value,
                      }));
                    }}
                    placeholder={t('Choose workspace')}
                    triggerClassName="min-h-8 rounded-[8px] border-[#e7e7e5] bg-white px-3 py-1.5 text-[13px] shadow-none"
                    value={mcpServerDraft.workingDir}
                    workspaces={workspaces}
                  />
                </div>
              </>
            ) : (
              <>
                <div className="space-y-1.5">
                  <Label className="text-[11px] font-medium text-[#666663]">URL</Label>
                  <Input
                    className="h-8 rounded-[8px] border-[#e7e7e5] bg-white text-[13px] shadow-none"
                    placeholder="https://mcp.example.com/mcp"
                    value={mcpServerDraft.url}
                    onChange={(event) => {
                      setMcpServerDraft((current) => ({
                        ...current,
                        url: event.target.value,
                      }));
                    }}
                  />
                </div>

                <div className="space-y-1.5">
                  <div className="flex items-center justify-between gap-2">
                    <Label className="text-[11px] font-medium text-[#666663]">{t('Headers')}</Label>
                    <button
                      className="text-[11px] text-[#666663] hover:text-[#111111]"
                      onClick={() => {
                        setMcpServerDraft((current) => ({
                          ...current,
                          headerEntries: [...current.headerEntries, { key: '', value: '' }],
                        }));
                      }}
                      type="button"
                    >
                      + {t('Add')}
                    </button>
                  </div>
                  <div className="space-y-1.5">
                    {mcpServerDraft.headerEntries.map((entry, index) => (
                      <div className="grid gap-1.5 md:grid-cols-[0.9fr_1.1fr_auto]" key={`header-${index}`}>
                        <Input
                          className="h-8 rounded-[8px] border-[#e7e7e5] bg-white text-[13px] shadow-none"
                          placeholder={t('Key')}
                          value={entry.key}
                          onChange={(event) => {
                            setMcpServerDraft((current) => ({
                              ...current,
                              headerEntries: current.headerEntries.map((currentEntry, entryIndex) => {
                                return entryIndex === index
                                  ? { ...currentEntry, key: event.target.value }
                                  : currentEntry;
                              }),
                            }));
                          }}
                        />
                        <Input
                          className="h-8 rounded-[8px] border-[#e7e7e5] bg-white text-[13px] shadow-none"
                          placeholder={t('Value')}
                          value={entry.value}
                          onChange={(event) => {
                            setMcpServerDraft((current) => ({
                              ...current,
                              headerEntries: current.headerEntries.map((currentEntry, entryIndex) => {
                                return entryIndex === index
                                  ? { ...currentEntry, value: event.target.value }
                                  : currentEntry;
                              }),
                            }));
                          }}
                        />
                        <button
                          className="px-2 text-[11px] text-[#9b3d3d] hover:text-[#7a2f2f] disabled:cursor-not-allowed disabled:text-[#c7c7c4]"
                          disabled={mcpServerDraft.headerEntries.length <= 1}
                          onClick={() => {
                            setMcpServerDraft((current) => ({
                              ...current,
                              headerEntries: current.headerEntries.filter((_, entryIndex) => entryIndex !== index),
                            }));
                          }}
                          type="button"
                        >
                          {t('Delete')}
                        </button>
                      </div>
                    ))}
                  </div>
                </div>
              </>
            )}

            <p className={classNames('text-[11px] leading-4 text-[#8a8a87]', (mcpServerNameTaken || !mcpServerDraftReady) && '!text-[#9b3d3d]')}>
              {mcpServerDraftValidationMessage}
            </p>
          </div>

          <DialogFooter className="flex !justify-between border-t border-[#efefec] px-4 py-3 sm:!justify-between">
            <div>
              {editingMcpServerName ? (
                <Button
                  className="h-8 rounded-[8px] border-[#f0d9d9] bg-white px-3 text-[12px] text-[#9b3d3d] shadow-none hover:bg-[#fdf3f3]"
                  disabled={mcpServersSaving}
                  onClick={() => { void handleDeleteMcpServer(editingMcpServerName); }}
                  type="button"
                  variant="outline"
                >
                  {t('Delete')}
                </Button>
              ) : null}
            </div>
            <div className="flex gap-2">
              <Button
                className="h-8 rounded-[8px] border-[#e7e7e5] bg-white px-3 text-[12px] text-[#111111] shadow-none hover:bg-[#f6f6f5]"
                onClick={closeMcpDialog}
                type="button"
                variant="outline"
              >
                {t('Cancel')}
              </Button>
              <Button
                className="h-8 rounded-[8px] bg-[#111111] px-3 text-[12px] text-white shadow-none hover:bg-[#1c1c1c]"
                disabled={!mcpServerDraftReady || mcpServersSaving}
                onClick={() => {
                  void handleSaveMcpServerDraft();
                }}
                type="button"
              >
                {mcpServersSaving ? t('Saving…') : t('Save')}
              </Button>
            </div>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
