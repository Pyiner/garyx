import { useEffect, useMemo, useState } from 'react';

import type {
  CreateCustomAgentInput,
  DesktopCustomAgent,
  UpdateCustomAgentInput,
} from '@shared/contracts';

import { Input } from '../../components/ui/input';
import { Label } from '../../components/ui/label';
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '../../components/ui/select';
import { Textarea } from '../../components/ui/textarea';
import { useI18n } from '../../i18n';

type ProviderType = 'claude_code' | 'codex_app_server' | 'gemini_cli';
type EditorMode = 'inspect' | 'create' | 'edit';

type AgentsPanelProps = {
  onToast?: (message: string, tone?: 'success' | 'error' | 'info', durationMs?: number) => void;
};

type AgentDraft = {
  agentId: string;
  displayName: string;
  providerType: ProviderType;
  model: string;
  systemPrompt: string;
};

const DEFAULT_GEMINI_MODEL = 'gemini-3-flash-preview';

function emptyDraft(): AgentDraft {
  return {
    agentId: '',
    displayName: '',
    providerType: 'claude_code',
    model: '',
    systemPrompt: '',
  };
}

function deriveAgentId(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .replace(/-{2,}/g, '-');
}

function providerLabel(value: ProviderType): string {
  if (value === 'codex_app_server') {
    return 'Codex';
  }
  if (value === 'gemini_cli') {
    return 'Gemini';
  }
  return 'Claude';
}

const plusIcon = (
  <svg aria-hidden width="14" height="14" viewBox="0 0 20 20" fill="none">
    <path d="M9.33496 16.5V10.665H3.5C3.13273 10.665 2.83496 10.3673 2.83496 10C2.83496 9.63273 3.13273 9.33496 3.5 9.33496H9.33496V3.5C9.33496 3.13273 9.63273 2.83496 10 2.83496C10.3673 2.83496 10.665 3.13273 10.665 3.5V9.33496H16.5C16.8673 9.33496 17.165 9.63273 17.165 10C17.165 10.3673 16.8673 10.665 16.5 10.665H10.665V16.5C10.665 16.8673 10.3673 17.165 10 17.165C9.63273 17.165 9.33496 16.8673 9.33496 16.5Z" fill="currentColor"/>
  </svg>
);

export function AgentsPanel({ onToast }: AgentsPanelProps) {
  const { t } = useI18n();
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [agents, setAgents] = useState<DesktopCustomAgent[]>([]);
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [editorMode, setEditorMode] = useState<EditorMode>('inspect');
  const [draft, setDraft] = useState<AgentDraft>(() => emptyDraft());
  const [draftIdTouched, setDraftIdTouched] = useState(false);

  async function loadAgents(preferredAgentId?: string | null) {
    setLoading(true);
    try {
      const nextAgents = await window.garyxDesktop.listCustomAgents();
      const visibleAgents = nextAgents
        .filter((agent) => agent.standalone)
        .sort((left, right) => {
          if (left.builtIn !== right.builtIn) {
            return left.builtIn ? -1 : 1;
          }
          return left.displayName.localeCompare(right.displayName) || left.agentId.localeCompare(right.agentId);
        });
      setAgents(visibleAgents);
      setSelectedAgentId(preferredAgentId || selectedAgentId || visibleAgents[0]?.agentId || null);
    } catch (error) {
      onToast?.(error instanceof Error ? error.message : t('Failed to load agents'), 'error');
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void loadAgents();
  }, []);

  useEffect(() => {
    if (editorMode !== 'create' || draftIdTouched) {
      return;
    }
    const nextId = deriveAgentId(draft.displayName);
    setDraft((current) => (current.agentId === nextId ? current : { ...current, agentId: nextId }));
  }, [draft.displayName, draftIdTouched, editorMode]);

  const selectedAgent = useMemo(
    () => agents.find((agent) => agent.agentId === selectedAgentId) || null,
    [agents, selectedAgentId],
  );

  function openCreateEditor() {
    setEditorMode('create');
    setDraft(emptyDraft());
    setDraftIdTouched(false);
  }

  function openEditEditor(agent: DesktopCustomAgent) {
    if (agent.builtIn) {
      return;
    }
    setEditorMode('edit');
    setDraft({
      agentId: agent.agentId,
      displayName: agent.displayName,
      providerType: agent.providerType,
      model: agent.model,
      systemPrompt: agent.systemPrompt,
    });
    setDraftIdTouched(true);
  }

  async function handleDelete(agent: DesktopCustomAgent) {
    if (agent.builtIn) {
      return;
    }
    setSaving(true);
    try {
      await window.garyxDesktop.deleteCustomAgent({ agentId: agent.agentId });
      onToast?.(t('Custom agent deleted'), 'success');
      setEditorMode('inspect');
      await loadAgents(agents.find((item) => item.agentId !== agent.agentId)?.agentId || null);
    } catch (error) {
      onToast?.(error instanceof Error ? error.message : t('Failed to delete custom agent'), 'error');
    } finally {
      setSaving(false);
    }
  }

  async function handleSubmit(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSaving(true);
    try {
      const payload: CreateCustomAgentInput = {
        agentId: draft.agentId.trim(),
        displayName: draft.displayName.trim(),
        providerType: draft.providerType,
        model: draft.model.trim(),
        systemPrompt: draft.systemPrompt.trim(),
      };
      let saved: DesktopCustomAgent;
      if (editorMode === 'create') {
        saved = await window.garyxDesktop.createCustomAgent(payload);
        onToast?.(t('Custom agent created'), 'success');
      } else {
        const updatePayload: UpdateCustomAgentInput = {
          ...payload,
          currentAgentId: selectedAgent?.agentId || payload.agentId,
        };
        saved = await window.garyxDesktop.updateCustomAgent(updatePayload);
        onToast?.(t('Custom agent updated'), 'success');
      }
      setEditorMode('inspect');
      setDraft(emptyDraft());
      setDraftIdTouched(false);
      await loadAgents(saved.agentId);
    } catch (error) {
      onToast?.(error instanceof Error ? error.message : t('Failed to save custom agent'), 'error');
    } finally {
      setSaving(false);
    }
  }

  const validationError =
    !draft.displayName.trim()
      ? t('Name is required.')
      : !draft.agentId.trim()
        ? t('Agent ID is required.')
        : !draft.systemPrompt.trim()
          ? t('System prompt is required.')
          : null;

  const showingEditor = editorMode === 'create' || (editorMode === 'edit' && selectedAgent && !selectedAgent.builtIn);

  return (
    <div className="grid h-full min-h-0 w-full gap-6" style={{ gridTemplateColumns: '340px minmax(0,1fr)' }}>
      {/* ── Left column: agent list ── */}
      <div className="flex h-full min-h-0 flex-col gap-4 overflow-hidden">
        <div className="codex-section">
          <div className="codex-section-header">
            <span className="codex-section-title">{t('Agents')}</span>
            <button className="codex-section-action" onClick={openCreateEditor} type="button">
              {plusIcon} {t('New')}
            </button>
          </div>
        </div>
        {loading ? (
          <div className="codex-empty-state">{t('Loading agents...')}</div>
        ) : agents.length ? (
          <div className="codex-list-card" style={{ flex: '1 1 0', minHeight: 0, overflowY: 'auto' }}>
            {agents.map((agent) => {
              const active = agent.agentId === selectedAgentId && editorMode === 'inspect';
              return (
                <button
                  key={agent.agentId}
                  className={`codex-list-row w-full text-left ${active ? 'codex-list-row-active' : ''}`}
                  onClick={() => {
                    setSelectedAgentId(agent.agentId);
                    setEditorMode('inspect');
                  }}
                  type="button"
                >
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
                    <span className="codex-list-row-name">{agent.displayName}</span>
                    <span className="codex-command-row-desc">{agent.agentId}</span>
                  </div>
                  <div className="codex-list-row-actions">
                    <span className="codex-sync-pill ok">
                      {agent.builtIn ? t('built-in') : providerLabel(agent.providerType)}
                    </span>
                  </div>
                </button>
              );
            })}
          </div>
        ) : (
          <div className="codex-empty-state">{t('No agents found.')}</div>
        )}
      </div>

      {/* ── Right column: inspect or edit ── */}
      {showingEditor ? (
        <div className="flex h-full min-h-0 flex-col gap-4 overflow-hidden">
          <div className="codex-section">
            <div className="codex-section-header">
              <span className="codex-section-title">
                {editorMode === 'create' ? t('New Agent') : t('Edit Agent')}
              </span>
            </div>
          </div>
          <div className="codex-list-card" style={{ flex: '1 1 0', minHeight: 0, overflowY: 'auto' }}>
            <form onSubmit={handleSubmit}>
              <div className="codex-form-field">
                <Label className="codex-form-label" htmlFor="agent-display-name">{t('Name')}</Label>
                <Input
                  id="agent-display-name"
                  onChange={(event) => {
                    setDraft((current) => ({ ...current, displayName: event.target.value }));
                  }}
                  value={draft.displayName}
                />
              </div>
              <div className="codex-form-field">
                <Label className="codex-form-label" htmlFor="agent-id">{t('Agent ID')}</Label>
                <Input
                  disabled={editorMode === 'edit'}
                  id="agent-id"
                  onChange={(event) => {
                    setDraftIdTouched(true);
                    setDraft((current) => ({ ...current, agentId: event.target.value }));
                  }}
                  value={draft.agentId}
                />
              </div>
              <div className="codex-form-field">
                <Label className="codex-form-label">{t('Provider')}</Label>
                <Select
                  onValueChange={(value: ProviderType) => {
                    setDraft((current) => ({
                      ...current,
                      providerType: value,
                      model:
                        value === 'gemini_cli' && !current.model.trim()
                          ? DEFAULT_GEMINI_MODEL
                          : current.model,
                    }));
                  }}
                  value={draft.providerType}
                >
                  <SelectTrigger>
                    <SelectValue placeholder={t('Select provider')} />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="claude_code">Claude</SelectItem>
                    <SelectItem value="codex_app_server">Codex</SelectItem>
                    <SelectItem value="gemini_cli">Gemini</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="codex-form-field">
                <Label className="codex-form-label" htmlFor="agent-model">{t('Model')}</Label>
                <Input
                  id="agent-model"
                  onChange={(event) => {
                    setDraft((current) => ({ ...current, model: event.target.value }));
                  }}
                  placeholder={draft.providerType === 'gemini_cli' ? DEFAULT_GEMINI_MODEL : t('provider default')}
                  value={draft.model}
                />
              </div>
              <div className="codex-form-field">
                <Label className="codex-form-label" htmlFor="agent-system-prompt">{t('System Prompt')}</Label>
                <Textarea
                  className="min-h-[260px]"
                  id="agent-system-prompt"
                  onChange={(event) => {
                    setDraft((current) => ({ ...current, systemPrompt: event.target.value }));
                  }}
                  value={draft.systemPrompt}
                />
              </div>
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12, padding: '12px 16px' }}>
                <span className="codex-form-hint" style={{ color: '#ef4444' }}>{validationError}</span>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <button
                    className="codex-section-action"
                    onClick={() => {
                      setEditorMode('inspect');
                      setDraft(emptyDraft());
                      setDraftIdTouched(false);
                    }}
                    type="button"
                  >
                    {t('Cancel')}
                  </button>
                  <button
                    className="codex-section-action"
                    disabled={Boolean(validationError) || saving}
                    style={{ color: 'var(--color-token-text-primary)', fontWeight: 500 }}
                    type="submit"
                  >
                    {saving ? t('Saving...') : editorMode === 'create' ? t('Create Agent') : t('Save Agent')}
                  </button>
                </div>
              </div>
            </form>
          </div>
        </div>
      ) : (
        <div className="flex h-full min-h-0 flex-col gap-4 overflow-hidden">
          <div className="codex-section">
            <div className="codex-section-header">
              <span className="codex-section-title">{selectedAgent?.displayName || t('Agent')}</span>
              {selectedAgent && !selectedAgent.builtIn ? (
                <div className="codex-list-row-actions">
                  <button className="codex-section-action" onClick={() => openEditEditor(selectedAgent)} type="button">
                    {t('Edit')}
                  </button>
                  <button
                    className="codex-section-action"
                    onClick={() => { void handleDelete(selectedAgent); }}
                    style={{ color: '#ef4444' }}
                    type="button"
                  >
                    {t('Delete')}
                  </button>
                </div>
              ) : null}
            </div>
          </div>
          {selectedAgent ? (
            <div className="codex-list-card" style={{ flex: '1 1 0', minHeight: 0, overflowY: 'auto' }}>
              <div className="codex-list-row">
                <span className="codex-list-row-name">{t('Agent ID')}</span>
                <span className="codex-command-row-desc">{selectedAgent.agentId}</span>
              </div>
              <div className="codex-list-row">
                <span className="codex-list-row-name">{t('Provider')}</span>
                <span className="codex-command-row-desc">{providerLabel(selectedAgent.providerType)}</span>
              </div>
              <div className="codex-list-row">
                <span className="codex-list-row-name">{t('Model')}</span>
                <span className="codex-command-row-desc">{selectedAgent.model || t('(provider default)')}</span>
              </div>
              <div style={{ padding: '12px 16px' }}>
                <div className="codex-list-row-name" style={{ marginBottom: 8 }}>{t('System Prompt')}</div>
                <div style={{ whiteSpace: 'pre-wrap', fontSize: 13, lineHeight: 1.6, color: 'var(--color-token-text-secondary)', fontFamily: 'var(--font-mono)' }}>
                  {selectedAgent.systemPrompt || t('(empty)')}
                </div>
              </div>
            </div>
          ) : (
            <div className="codex-empty-state">
              {t('Select an agent from the list to inspect its provider and prompt.')}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
