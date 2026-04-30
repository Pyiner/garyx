import { useEffect, useRef, useState, type FormEvent, type JSX } from 'react';
import {
  IconDeviceFloppy,
  IconFilePlus,
  IconFolderPlus,
  IconX,
} from '@tabler/icons-react';
import { Settings, Trash } from 'lucide-react';

import type {
  CreateSkillInput,
  DesktopSkillEntryNode,
  DesktopSkillFileDocument,
  DesktopSkillInfo,
} from '@shared/contracts';
import type { ToastTone } from './toast';
import { useI18n } from './i18n';

const SKILL_ID_PATTERN = /^[a-z0-9-]+$/;
const TRANSIENT_STATUS_MS = 3200;
const SKILL_BODY_PLACEHOLDER = `# When to use

Explain when this skill should trigger and what it is responsible for.

## Workflow

1. Outline the key steps Garyx should follow.
2. Mention any required files, scripts, or references.
3. Call out important constraints or output expectations.`;

type SkillDraft = CreateSkillInput;

type SkillEditorSession = {
  skill: DesktopSkillInfo;
  entries: DesktopSkillEntryNode[];
  selectedPath: string | null;
  selectedDocument: DesktopSkillFileDocument | null;
  savedContent: string;
  draftContent: string;
};

type SkillsPanelProps = {
  onToast?: (message: string, tone?: ToastTone, durationMs?: number) => void;
};

function emptyDraft(): SkillDraft {
  return {
    id: '',
    name: '',
    description: '',
    body: '',
  };
}

function deriveSkillId(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .replace(/-{2,}/g, '-');
}

function sortSkills(skills: DesktopSkillInfo[]): DesktopSkillInfo[] {
  return [...skills].sort((left, right) => {
    if (left.enabled !== right.enabled) {
      return left.enabled ? -1 : 1;
    }
    return left.name.localeCompare(right.name) || left.id.localeCompare(right.id);
  });
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : 'Unexpected skills error';
}

function isProjectSkill(skill: DesktopSkillInfo): boolean {
  return skill.sourcePath.includes('/.claude/skills/') || skill.sourcePath.includes('\\.claude\\skills\\');
}

function sourceLabel(skill: DesktopSkillInfo): string {
  return isProjectSkill(skill) ? 'Project' : 'Personal';
}

function collectSkillFiles(entries: DesktopSkillEntryNode[]): string[] {
  const files: string[] = [];
  const visit = (items: DesktopSkillEntryNode[]) => {
    items.forEach((entry) => {
      if (entry.entryType === 'directory') {
        visit(entry.children);
        return;
      }
      files.push(entry.path);
    });
  };
  visit(entries);
  return files;
}

function pickPreferredFile(
  entries: DesktopSkillEntryNode[],
  preferredPath?: string | null,
): string | null {
  const files = collectSkillFiles(entries);
  if (!files.length) {
    return null;
  }
  if (preferredPath && files.includes(preferredPath)) {
    return preferredPath;
  }
  return files.includes('SKILL.md') ? 'SKILL.md' : files[0] || null;
}

function replaceSkill(skills: DesktopSkillInfo[], nextSkill: DesktopSkillInfo): DesktopSkillInfo[] {
  const next = skills.some((skill) => skill.id === nextSkill.id)
    ? skills.map((skill) => (skill.id === nextSkill.id ? nextSkill : skill))
    : [...skills, nextSkill];
  return sortSkills(next);
}

function skillPreviewBadgeLabel(document: DesktopSkillFileDocument | null): string | null {
  if (!document) {
    return null;
  }
  switch (document.previewKind) {
    case 'markdown':
      return 'Markdown';
    case 'text':
      return 'Text';
    case 'image':
      return 'Image';
    default:
      return document.mediaType || 'Binary';
  }
}

function skillPreviewMessage(document: DesktopSkillFileDocument | null): string {
  if (!document) {
    return 'Select a file to inspect this skill.';
  }
  if (document.previewKind === 'image' && !document.dataBase64) {
    return 'This image is too large to preview in the skill editor.';
  }
  if (document.previewKind === 'image') {
    return 'This image is preview-only in the skill editor.';
  }
  if (!document.editable) {
    return 'This file type is preview-only in the skill editor.';
  }
  return '';
}

/* ------------------------------------------------------------------ */
/* SVG icons                                                          */
/* ------------------------------------------------------------------ */

const PlusIcon = (
  <svg aria-hidden width="14" height="14" viewBox="0 0 20 20" fill="none">
    <path d="M9.33496 16.5V10.665H3.5C3.13273 10.665 2.83496 10.3673 2.83496 10C2.83496 9.63273 3.13273 9.33496 3.5 9.33496H9.33496V3.5C9.33496 3.13273 9.63273 2.83496 10 2.83496C10.3673 2.83496 10.665 3.13273 10.665 3.5V9.33496H16.5C16.8673 9.33496 17.165 9.63273 17.165 10C17.165 10.3673 16.8673 10.665 16.5 10.665H10.665V16.5C10.665 16.8673 10.3673 17.165 10 17.165C9.63273 17.165 9.33496 16.8673 9.33496 16.5Z" fill="currentColor"/>
  </svg>
);

export function SkillsPanel({ onToast }: SkillsPanelProps) {
  const { t } = useI18n();
  const [skills, setSkills] = useState<DesktopSkillInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [draft, setDraft] = useState<SkillDraft>(() => emptyDraft());
  const [creating, setCreating] = useState(false);
  const [mutatingSkillId, setMutatingSkillId] = useState<string | null>(null);
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [createAdvancedOpen, setCreateAdvancedOpen] = useState(false);
  const [draftIdManuallyEdited, setDraftIdManuallyEdited] = useState(false);
  const [editor, setEditor] = useState<SkillEditorSession | null>(null);
  const [editorBusy, setEditorBusy] = useState<string | null>(null);
  const [editorError, setEditorError] = useState<string | null>(null);
  const [editorStatus, setEditorStatus] = useState<string | null>(null);
  const editorLoadRequestIdRef = useRef(0);

  async function loadSkills() {
    setLoading(true);
    setError(null);

    try {
      const nextSkills = await window.garyxDesktop.listSkills();
      setSkills(sortSkills(nextSkills));
    } catch (loadError) {
      setError(errorMessage(loadError));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void loadSkills();
  }, []);

  useEffect(() => {
    if (draftIdManuallyEdited) {
      return;
    }

    const nextId = deriveSkillId(draft.name);
    setDraft((current) => {
      if (current.id === nextId) {
        return current;
      }
      return {
        ...current,
        id: nextId,
      };
    });
  }, [draft.name, draftIdManuallyEdited]);

  useEffect(() => {
    if (!createDialogOpen || draftIdManuallyEdited || !draft.name.trim() || draft.id.trim()) {
      return;
    }
    setCreateAdvancedOpen(true);
  }, [createDialogOpen, draft.id, draft.name, draftIdManuallyEdited]);

  useEffect(() => {
    if (!error) {
      return undefined;
    }
    onToast?.(error, 'error');
    setError(null);
    return undefined;
  }, [error, onToast]);

  useEffect(() => {
    if (!status) {
      return undefined;
    }
    onToast?.(status, 'success', TRANSIENT_STATUS_MS);
    setStatus(null);
    return undefined;
  }, [status, onToast]);

  useEffect(() => {
    if (!editorError) {
      return undefined;
    }
    onToast?.(editorError, 'error');
    setEditorError(null);
    return undefined;
  }, [editorError, onToast]);

  useEffect(() => {
    if (!editorStatus) {
      return undefined;
    }
    onToast?.(editorStatus, 'success', TRANSIENT_STATUS_MS);
    setEditorStatus(null);
    return undefined;
  }, [editorStatus, onToast]);

  const trimmedId = draft.id.trim();
  const trimmedName = draft.name.trim();
  const trimmedDescription = draft.description.trim();
  const trimmedBody = draft.body.trim();
  const duplicateSkill = skills.find((skill) => skill.id === trimmedId);
  const validationError =
    !trimmedName
      ? t('Skill name is required.')
      : !trimmedDescription
        ? t('Skill description is required.')
        : !trimmedBody
          ? t('Skill content is required.')
          : !trimmedId
            ? t('Skill ID is required. Open Advanced to set a slug.')
            : !SKILL_ID_PATTERN.test(trimmedId)
              ? t('Skill ID must match [a-z0-9-].')
              : duplicateSkill
                ? t('Skill ID "{id}" already exists.', { id: trimmedId })
                : null;
  const hasSkillIdValidationError = validationError?.toLowerCase().includes('skill id') ?? false;
  const editorDirty = Boolean(
    editor
    && editor.selectedPath
    && editor.selectedDocument?.editable
    && editor.draftContent !== editor.savedContent,
  );

  function updateSkillList(nextSkill: DesktopSkillInfo) {
    setSkills((current) => replaceSkill(current, nextSkill));
  }

  useEffect(() => {
    if (!createDialogOpen || !hasSkillIdValidationError) {
      return;
    }
    setCreateAdvancedOpen(true);
  }, [createDialogOpen, hasSkillIdValidationError]);

  function openCreateDialog() {
    setDraft(emptyDraft());
    setCreateAdvancedOpen(false);
    setDraftIdManuallyEdited(false);
    setError(null);
    setStatus(null);
    setCreateDialogOpen(true);
  }

  function closeCreateDialog() {
    if (creating) {
      return;
    }
    setCreateDialogOpen(false);
    setCreateAdvancedOpen(false);
    setDraftIdManuallyEdited(false);
    setDraft(emptyDraft());
  }

  function confirmDiscardEditorChanges(): boolean {
    if (!editorDirty) {
      return true;
    }
    return window.confirm('Discard unsaved skill changes?');
  }

  async function openSkillEditor(skill: DesktopSkillInfo) {
    if (editor && editor.skill.id !== skill.id && !confirmDiscardEditorChanges()) {
      return;
    }

    const requestId = editorLoadRequestIdRef.current + 1;
    editorLoadRequestIdRef.current = requestId;
    setMutatingSkillId(skill.id);
    setError(null);
    setStatus(null);
    setEditorError(null);
    setEditorStatus(null);

    try {
      const editorState = await window.garyxDesktop.getSkillEditor({ skillId: skill.id });
      let nextSkill = editorState.skill;
      let selectedPath = pickPreferredFile(editorState.entries);
      let document: DesktopSkillFileDocument | null = null;

      if (selectedPath) {
        document = await window.garyxDesktop.readSkillFile({
          skillId: skill.id,
          path: selectedPath,
        });
        if (editorLoadRequestIdRef.current !== requestId) {
          return;
        }
        nextSkill = document.skill;
        selectedPath = document.path;
      }

      if (editorLoadRequestIdRef.current !== requestId) {
        return;
      }
      updateSkillList(nextSkill);
      setEditor({
        skill: nextSkill,
        entries: editorState.entries,
        selectedPath,
        selectedDocument: document,
        savedContent: document?.content || '',
        draftContent: document?.content || '',
      });
    } catch (editorLoadError) {
      if (editorLoadRequestIdRef.current !== requestId) {
        return;
      }
      setError(errorMessage(editorLoadError));
    } finally {
      if (editorLoadRequestIdRef.current === requestId) {
        setMutatingSkillId(null);
      }
    }
  }

  async function handleCreateSkill(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();

    if (validationError) {
      setError(validationError);
      return;
    }

    setCreating(true);
    setError(null);
    setStatus(null);

    try {
      const created = await window.garyxDesktop.createSkill({
        id: trimmedId,
        name: trimmedName,
        description: trimmedDescription,
        body: draft.body,
      });
      setSkills((current) => replaceSkill(current, created));
      setStatus(`Created ${created.name}.`);
      setCreateDialogOpen(false);
      setCreateAdvancedOpen(false);
      setDraftIdManuallyEdited(false);
      setDraft(emptyDraft());
      void openSkillEditor(created);
    } catch (createError) {
      setError(errorMessage(createError));
    } finally {
      setCreating(false);
    }
  }

  async function handleToggleSkill(skill: DesktopSkillInfo) {
    setMutatingSkillId(skill.id);
    setError(null);
    setStatus(null);

    try {
      const updated = await window.garyxDesktop.toggleSkill({ skillId: skill.id });
      updateSkillList(updated);
      if (editor?.skill.id === updated.id) {
        setEditor((current) => current ? { ...current, skill: updated } : current);
      }
      setStatus(`${updated.name} ${updated.enabled ? 'enabled' : 'disabled'}.`);
    } catch (toggleError) {
      setError(errorMessage(toggleError));
    } finally {
      setMutatingSkillId(null);
    }
  }

  async function handleDeleteSkill(skill: DesktopSkillInfo) {
    if (!window.confirm(`Delete skill "${skill.name}" and its directory?`)) {
      return;
    }

    setMutatingSkillId(skill.id);
    setError(null);
    setStatus(null);

    try {
      await window.garyxDesktop.deleteSkill({ skillId: skill.id });
      setSkills((current) => current.filter((entry) => entry.id !== skill.id));
      if (editor?.skill.id === skill.id) {
        setEditor(null);
        setEditorError(null);
        setEditorStatus(null);
      }
      setStatus(`Deleted ${skill.name}.`);
    } catch (deleteError) {
      setError(errorMessage(deleteError));
    } finally {
      setMutatingSkillId(null);
    }
  }

  async function handleSelectEditorFile(path: string) {
    if (!editor || editorBusy) {
      return;
    }
    if (editor.selectedPath === path) {
      return;
    }
    if (!confirmDiscardEditorChanges()) {
      return;
    }

    setEditorBusy(`read:${path}`);
    setEditorError(null);
    setEditorStatus(null);

    try {
      const document = await window.garyxDesktop.readSkillFile({
        skillId: editor.skill.id,
        path,
      });
      updateSkillList(document.skill);
      setEditor((current) => current ? {
        ...current,
        skill: document.skill,
        selectedPath: document.path,
        selectedDocument: document,
        savedContent: document.content,
        draftContent: document.content,
      } : current);
    } catch (readError) {
      setEditorError(errorMessage(readError));
    } finally {
      setEditorBusy(null);
    }
  }

  async function handleSaveEditorFile() {
    if (!editor?.selectedPath || !editorDirty || !editor.selectedDocument?.editable) {
      return;
    }

    setEditorBusy(`save:${editor.selectedPath}`);
    setEditorError(null);
    setEditorStatus(null);

    try {
      const document = await window.garyxDesktop.saveSkillFile({
        skillId: editor.skill.id,
        path: editor.selectedPath,
        content: editor.draftContent,
      });
      updateSkillList(document.skill);
      setEditor((current) => current ? {
        ...current,
        skill: document.skill,
        selectedPath: document.path,
        selectedDocument: document,
        savedContent: document.content,
        draftContent: document.content,
      } : current);
      setEditorStatus(`Saved ${document.path}.`);
    } catch (saveError) {
      setEditorError(errorMessage(saveError));
    } finally {
      setEditorBusy(null);
    }
  }

  async function handleCreateEditorEntry(entryType: 'file' | 'directory') {
    if (!editor || editorBusy) {
      return;
    }

    const promptLabel = entryType === 'file' ? 'New file path' : 'New folder path';
    const value = window.prompt(promptLabel, entryType === 'file' ? 'scripts/example.ts' : 'references');
    const nextPath = value?.trim();
    if (!nextPath) {
      return;
    }

    setEditorBusy(`create:${entryType}`);
    setEditorError(null);
    setEditorStatus(null);

    try {
      const nextEditor = await window.garyxDesktop.createSkillEntry({
        skillId: editor.skill.id,
        path: nextPath,
        entryType,
      });
      updateSkillList(nextEditor.skill);
      setEditor((current) => current ? {
        ...current,
        skill: nextEditor.skill,
        entries: nextEditor.entries,
      } : current);
      setEditorStatus(`${entryType === 'file' ? 'Created file' : 'Created folder'} ${nextPath}.`);

      if (entryType === 'file') {
        const document = await window.garyxDesktop.readSkillFile({
          skillId: editor.skill.id,
          path: nextPath,
        });
        updateSkillList(document.skill);
        setEditor((current) => current ? {
          ...current,
          skill: document.skill,
          entries: nextEditor.entries,
          selectedPath: document.path,
          selectedDocument: document,
          savedContent: document.content,
          draftContent: document.content,
        } : current);
      }
    } catch (entryError) {
      setEditorError(errorMessage(entryError));
    } finally {
      setEditorBusy(null);
    }
  }

  async function handleDeleteEditorEntry() {
    if (!editor?.selectedPath || editorBusy) {
      return;
    }
    if (!window.confirm(`Delete ${editor.selectedPath}?`)) {
      return;
    }

    setEditorBusy(`delete:${editor.selectedPath}`);
    setEditorError(null);
    setEditorStatus(null);

    try {
      const deletedPath = editor.selectedPath;
      const nextEditor = await window.garyxDesktop.deleteSkillEntry({
        skillId: editor.skill.id,
        path: deletedPath,
      });
      updateSkillList(nextEditor.skill);
      const nextSelectedPath = pickPreferredFile(nextEditor.entries);

      if (nextSelectedPath) {
        const document = await window.garyxDesktop.readSkillFile({
          skillId: editor.skill.id,
          path: nextSelectedPath,
        });
        updateSkillList(document.skill);
        setEditor({
          skill: document.skill,
          entries: nextEditor.entries,
          selectedPath: document.path,
          selectedDocument: document,
          savedContent: document.content,
          draftContent: document.content,
        });
      } else {
        setEditor({
          skill: nextEditor.skill,
          entries: nextEditor.entries,
          selectedPath: null,
          selectedDocument: null,
          savedContent: '',
          draftContent: '',
        });
      }

      setEditorStatus(`Deleted ${deletedPath}.`);
    } catch (deleteError) {
      setEditorError(errorMessage(deleteError));
    } finally {
      setEditorBusy(null);
    }
  }

  function closeEditor() {
    if (editorBusy) {
      return;
    }
    if (!confirmDiscardEditorChanges()) {
      return;
    }
    editorLoadRequestIdRef.current += 1;
    setEditor(null);
    setEditorBusy(null);
    setEditorError(null);
    setEditorStatus(null);
  }

  function renderTree(entries: DesktopSkillEntryNode[], depth = 0): JSX.Element[] {
    return entries.flatMap((entry) => {
      if (entry.entryType === 'directory') {
        return [
          <div
            className="skills-editor-tree-folder"
            key={entry.path}
            style={{ paddingLeft: `${12 + depth * 16}px` }}
          >
            {entry.name}
          </div>,
          ...renderTree(entry.children, depth + 1),
        ];
      }

      return [
        <button
          className={`skills-editor-tree-file ${editor?.selectedPath === entry.path ? 'active' : ''}`}
          key={entry.path}
          onClick={() => {
            void handleSelectEditorFile(entry.path);
          }}
          style={{ paddingLeft: `${12 + depth * 16}px` }}
          type="button"
        >
          {entry.name}
        </button>,
      ];
    });
  }

  return (
    <>
      <div className="skills-panel-shell">
        <div className="codex-section">
          <div className="codex-section-header skills-panel-header">
            <div className="skills-panel-header-copy">
              <span className="codex-section-title">{t('Skills')}</span>
              <span className="codex-section-note">{t('{count} total', { count: skills.length })}</span>
            </div>
            <div className="skills-panel-header-actions">
              <button
                className="codex-section-action"
                disabled={loading || creating || Boolean(mutatingSkillId) || Boolean(editorBusy)}
                onClick={() => {
                  void loadSkills();
                }}
                type="button"
              >
                {loading ? t('Refreshing...') : t('Refresh')}
              </button>
              <button
                className="codex-section-action"
                disabled={creating || Boolean(mutatingSkillId) || Boolean(editorBusy)}
                onClick={openCreateDialog}
                type="button"
              >
                {PlusIcon} {t('New Skill')}
              </button>
            </div>
          </div>
        </div>

        {loading ? (
          <div className="codex-empty-state skills-panel-empty">{t('Loading skills...')}</div>
        ) : !skills.length ? (
          <div className="codex-empty-state skills-panel-empty">{t('No skills installed. Create your first skill.')}</div>
        ) : (
          <div className="codex-list-card skills-panel-list">
            {skills.map((skill) => {
              const busy = mutatingSkillId === skill.id;
              const showId = skill.id.trim() !== skill.name.trim();
              return (
                <div
                  className="codex-list-row skills-panel-row"
                  key={skill.id}
                  style={{ minHeight: 56, opacity: skill.enabled ? 1 : 0.55 }}
                >
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0, flex: 1 }}>
                    <div className="skills-panel-row-title">
                      <span className="codex-list-row-name">{skill.name}</span>
                      <span className="codex-sync-pill ok">{t(sourceLabel(skill))}</span>
                    </div>
                    {showId ? <span className="codex-command-row-desc">{skill.id}</span> : null}
                    <span className="codex-command-row-desc skills-panel-description">
                      {skill.description || t('No description provided.')}
                    </span>
                  </div>
                  <div className="codex-list-row-actions">
                    <button
                      aria-label={t('Edit')}
                      className="codex-icon-button skills-icon-button"
                      disabled={busy || creating || Boolean(editorBusy)}
                      onClick={() => {
                        void openSkillEditor(skill);
                      }}
                      title={t('Edit')}
                      type="button"
                    >
                      <Settings />
                    </button>
                    <button
                      aria-label={t('Delete')}
                      className="codex-icon-button skills-icon-button skills-icon-button-danger"
                      disabled={busy || creating || Boolean(editorBusy)}
                      onClick={() => {
                        void handleDeleteSkill(skill);
                      }}
                      title={t('Delete')}
                      type="button"
                    >
                      <Trash />
                    </button>
                    <button
                      aria-label={skill.enabled ? t('Disable skill') : t('Enable skill')}
                      aria-pressed={skill.enabled}
                      className={`settings-switch ${skill.enabled ? 'checked' : ''}`}
                      disabled={busy || creating || Boolean(editorBusy)}
                      onClick={() => {
                        void handleToggleSkill(skill);
                      }}
                      type="button"
                    >
                      <span className="settings-switch-handle" />
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {createDialogOpen ? (
        <div
          className="modal-overlay"
          onClick={() => {
            closeCreateDialog();
          }}
          role="presentation"
        >
          <div
            aria-labelledby="skills-create-dialog-title"
            aria-modal="true"
            className="modal-card skills-create-modal"
            onClick={(event) => {
              event.stopPropagation();
            }}
            role="dialog"
          >
            <div className="panel-header skills-create-modal-header">
              <div className="bot-card-copy">
                <span className="eyebrow">{t('Create Skill')}</span>
                <h3 className="panel-title" id="skills-create-dialog-title">{t('New Skill')}</h3>
              </div>
              <button
                aria-label={t('Close skill dialog')}
                className="sidebar-tool-button"
                disabled={creating}
                onClick={closeCreateDialog}
                type="button"
              >
                <IconX size={16} stroke={1.8} />
              </button>
            </div>

            <form
              className="automation-form skills-create-form"
              onSubmit={(event) => {
                void handleCreateSkill(event);
              }}
            >
              <label>
                <span>{t('Name')}</span>
                <input
                  autoFocus
                  onChange={(event) => {
                    setDraft((current) => ({
                      ...current,
                      name: event.target.value,
                    }));
                  }}
                  placeholder={t('Example Skill')}
                  value={draft.name}
                />
              </label>

              <label>
                <span>{t('Description')}</span>
                <textarea
                  onChange={(event) => {
                    setDraft((current) => ({
                      ...current,
                      description: event.target.value,
                    }));
                  }}
                  placeholder={t('What this skill should help Garyx do.')}
                  value={draft.description}
                />
              </label>

              <label>
                <span>{t('Skill Content')}</span>
                <textarea
                  className="skills-create-content-input"
                  onChange={(event) => {
                    setDraft((current) => ({
                      ...current,
                      body: event.target.value,
                    }));
                  }}
                  placeholder={SKILL_BODY_PLACEHOLDER}
                  spellCheck={false}
                  value={draft.body}
                />
              </label>

              <p className="small-note skills-form-note">
                {t('Frontmatter is generated automatically from Name and Description. Write only the markdown body for SKILL.md here.')}
              </p>

              <div className="skills-create-advanced">
                <button
                  aria-expanded={createAdvancedOpen}
                  className="ghost-button skills-advanced-toggle"
                  onClick={() => {
                    setCreateAdvancedOpen((current) => !current);
                  }}
                  type="button"
                >
                  {createAdvancedOpen ? t('Hide Advanced') : t('Advanced')}
                </button>
                <span className="small-note">
                  {t('Skill ID')}: <code>{trimmedId || t('set manually')}</code>
                </span>
              </div>

              {createAdvancedOpen ? (
                <div className="skills-advanced-panel">
                  <label>
                    <span>{t('Skill ID')}</span>
                    <input
                      onChange={(event) => {
                        setDraftIdManuallyEdited(true);
                        setDraft((current) => ({
                          ...current,
                          id: event.target.value,
                        }));
                      }}
                      placeholder="example-skill"
                      spellCheck={false}
                      value={draft.id}
                    />
                  </label>
                  <p className="small-note skills-form-note">
                    {t('Stable slug used as the skill directory name and API key under ~/.garyx/skills/<id>.')}
                  </p>
                </div>
              ) : null}

              <p className={`small-note skills-form-note ${validationError ? 'error' : ''}`}>
                {validationError || t('The skill is created immediately with a real SKILL.md body, then opened in the full directory editor.')}
              </p>

              <div className="skills-form-footer">
                <button
                  className="ghost-button"
                  disabled={creating}
                  onClick={closeCreateDialog}
                  type="button"
                >
                  {t('Cancel')}
                </button>
                <button
                  className="primary-button"
                  disabled={creating || Boolean(validationError)}
                  type="submit"
                >
                  {creating ? t('Creating...') : t('Create Skill')}
                </button>
              </div>
            </form>
          </div>
        </div>
      ) : null}

      {editor ? (
        <div
          className="modal-overlay"
          onClick={() => {
            closeEditor();
          }}
          role="presentation"
        >
          <div
            aria-labelledby="skills-editor-title"
            aria-modal="true"
            className="modal-card skills-editor-modal"
            onClick={(event) => {
              event.stopPropagation();
            }}
            role="dialog"
          >
            <div className="skills-editor-header">
              <div className="skills-editor-header-copy">
                <span className="eyebrow">{t('Skill Editor')}</span>
                <h3 className="panel-title" id="skills-editor-title">{editor.skill.name}</h3>
                <p className="small-note">{editor.skill.sourcePath}</p>
              </div>
              <div className="skills-editor-toolbar">
                <button
                  className="codex-section-action"
                  disabled={Boolean(editorBusy)}
                  onClick={() => {
                    void handleCreateEditorEntry('file');
                  }}
                  type="button"
                >
                  <IconFilePlus size={16} stroke={1.8} />
                  {t('New File')}
                </button>
                <button
                  className="codex-section-action"
                  disabled={Boolean(editorBusy)}
                  onClick={() => {
                    void handleCreateEditorEntry('directory');
                  }}
                  type="button"
                >
                  <IconFolderPlus size={16} stroke={1.8} />
                  {t('New Folder')}
                </button>
                <button
                  aria-label={t('Delete file')}
                  className="codex-icon-button skills-icon-button skills-icon-button-danger"
                  disabled={!editor.selectedPath || editor.selectedPath === 'SKILL.md' || Boolean(editorBusy)}
                  onClick={() => {
                    void handleDeleteEditorEntry();
                  }}
                  title={t('Delete file')}
                  type="button"
                >
                  <Trash />
                </button>
                <button
                  className="primary-button"
                  disabled={
                    !editorDirty
                    || Boolean(editorBusy)
                    || !editor.selectedPath
                    || !editor.selectedDocument?.editable
                  }
                  onClick={() => {
                    void handleSaveEditorFile();
                  }}
                  type="button"
                >
                  <IconDeviceFloppy size={16} stroke={1.8} />
                  {editorBusy?.startsWith('save:') ? t('Saving...') : t('Save')}
                </button>
                <button
                  aria-label={t('Close skill editor')}
                  className="codex-icon-button"
                  disabled={Boolean(editorBusy)}
                  onClick={closeEditor}
                  type="button"
                >
                  <IconX size={16} stroke={1.8} />
                </button>
              </div>
            </div>

            <div className="skills-editor-layout">
              <aside className="skills-editor-sidebar">
                <div className="skills-editor-tree">
                  {renderTree(editor.entries)}
                </div>
              </aside>

              <section className="skills-editor-main">
                <div className="skills-editor-filebar">
                  <strong>{editor.selectedPath || t('No file selected')}</strong>
                  <div className="skills-editor-filebar-status">
                    {skillPreviewBadgeLabel(editor.selectedDocument) ? (
                      <span className="codex-sync-pill ok">
                        {t(skillPreviewBadgeLabel(editor.selectedDocument)!)}
                      </span>
                    ) : null}
                    {editorDirty ? <span className="codex-sync-pill fail">{t('Unsaved')}</span> : null}
                    {editor.selectedDocument && !editor.selectedDocument.editable ? (
                      <span className="codex-sync-pill ok">{t('Read-only')}</span>
                    ) : null}
                  </div>
                </div>

                {editor.selectedPath ? (
                  editor.selectedDocument?.previewKind === 'image' && editor.selectedDocument.dataBase64 ? (
                    <div className="skills-editor-preview-shell">
                      <div className="skills-editor-preview-copy">
                        <span className="eyebrow">{t('Image Preview')}</span>
                        <p>{t(skillPreviewMessage(editor.selectedDocument))}</p>
                      </div>
                      <div className="skills-editor-image-frame">
                        <img
                          alt={editor.selectedDocument.path}
                          className="skills-editor-image-preview"
                          src={`data:${editor.selectedDocument.mediaType};base64,${editor.selectedDocument.dataBase64}`}
                        />
                      </div>
                    </div>
                  ) : editor.selectedDocument?.editable ? (
                    <textarea
                      className="skills-editor-textarea"
                      onChange={(event) => {
                        setEditor((current) => current ? {
                          ...current,
                          draftContent: event.target.value,
                        } : current);
                      }}
                      spellCheck={false}
                      value={editor.draftContent}
                    />
                  ) : (
                    <div className="workspace-empty-block skills-editor-empty skills-editor-preview-shell">
                      <span className="eyebrow">
                        {editor.selectedDocument?.previewKind === 'image' ? t('Image Preview') : t('Preview Unavailable')}
                      </span>
                      <p>{t(skillPreviewMessage(editor.selectedDocument))}</p>
                      {editor.selectedDocument?.mediaType ? (
                        <code>{editor.selectedDocument.mediaType}</code>
                      ) : null}
                    </div>
                  )
                ) : (
                  <div className="workspace-empty-block skills-editor-empty">
                    <span className="eyebrow">{t('No Files')}</span>
                    <p>{t('Create a file to start editing this skill directory.')}</p>
                  </div>
                )}
              </section>
            </div>
          </div>
        </div>
      ) : null}
    </>
  );
}
