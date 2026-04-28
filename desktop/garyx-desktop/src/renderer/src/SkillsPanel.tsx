import { useEffect, useRef, useState, type FormEvent, type JSX } from 'react';
import {
  IconDeviceFloppy,
  IconFilePlus,
  IconFolderPlus,
  IconX,
} from '@tabler/icons-react';

import type {
  CreateSkillInput,
  DesktopSkillEntryNode,
  DesktopSkillFileDocument,
  DesktopSkillInfo,
} from '@shared/contracts';
import type { ToastTone } from './toast';

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

const GearIcon = (
  <svg aria-hidden width="18" height="18" viewBox="0 0 21 21" fill="none">
    <path d="M10.7228 2.53564C11.5515 2.53564 12.3183 2.97502 12.7374 3.68994L13.5587 5.09033L13.6124 5.15967C13.6736 5.22007 13.7566 5.2556 13.8448 5.25635L15.4601 5.26904L15.6144 5.27588C16.3826 5.33292 17.0775 5.76649 17.465 6.43994L17.7931 7.01123L17.8663 7.14697C18.1815 7.78943 18.1843 8.54208 17.8741 9.18701L17.8028 9.32275L17.0001 10.7446C16.9427 10.8467 16.9426 10.9717 17.0001 11.0737L17.8028 12.4946L17.8741 12.6313C18.1842 13.2763 18.1816 14.029 17.8663 14.6714L17.7931 14.8071L17.465 15.3784C17.0774 16.0517 16.3825 16.4855 15.6144 16.5425L15.4601 16.5483L13.8448 16.562C13.7565 16.5628 13.6736 16.5982 13.6124 16.6587L13.5587 16.7271L12.7374 18.1284C12.3183 18.8432 11.5514 19.2827 10.7228 19.2827H10.0763C9.29958 19.2826 8.57714 18.8964 8.14465 18.2593L8.06261 18.1284L7.24133 16.7271C7.1966 16.6509 7.12417 16.5966 7.04113 16.5737L6.95519 16.562L5.33996 16.5483C4.56297 16.542 3.84347 16.1503 3.41613 15.5093L3.33508 15.3784L3.00695 14.8071C2.59564 14.0921 2.59168 13.2129 2.99719 12.4946L3.79894 11.0737L3.83215 10.9937C3.84657 10.9383 3.84652 10.88 3.83215 10.8247L3.79894 10.7446L2.99719 9.32275C2.59184 8.60451 2.59571 7.72612 3.00695 7.01123L3.33508 6.43994L3.41613 6.30908C3.84345 5.66796 4.56288 5.27538 5.33996 5.26904L6.95519 5.25635L7.04113 5.24463C7.12427 5.22177 7.1966 5.16664 7.24133 5.09033L8.06261 3.68994L8.14465 3.55908C8.57712 2.92179 9.29949 2.5358 10.0763 2.53564H10.7228ZM11.9855 10.9087C11.9853 10.0336 11.2755 9.32399 10.4005 9.32373C9.52524 9.32373 8.81474 10.0335 8.81457 10.9087C8.81457 11.7841 9.52513 12.4937 10.4005 12.4937C11.2757 12.4934 11.9855 11.7839 11.9855 10.9087ZM13.3146 10.9087C13.3146 12.5184 12.0102 13.8235 10.4005 13.8237C8.7906 13.8237 7.48547 12.5186 7.48547 10.9087C7.48564 9.29893 8.7907 7.99365 10.4005 7.99365C12.0101 7.99391 13.3144 9.29909 13.3146 10.9087Z" fill="currentColor"/>
  </svg>
);

const TrashIcon = (
  <svg aria-hidden width="16" height="16" viewBox="0 0 20 20" fill="none">
    <path d="M5.5 2.5H14.5V4.5H5.5V2.5ZM3.5 5.5H16.5V6.83333H15.1667V15.5C15.1667 16.2364 14.5697 16.8333 13.8333 16.8333H6.16667C5.43029 16.8333 4.83333 16.2364 4.83333 15.5V6.83333H3.5V5.5ZM6.16667 6.83333V15.5H13.8333V6.83333H6.16667ZM8.16667 8.83333H9.5V13.5H8.16667V8.83333ZM10.5 8.83333H11.8333V13.5H10.5V8.83333Z" fill="currentColor"/>
  </svg>
);

export function SkillsPanel({ onToast }: SkillsPanelProps) {
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
      ? 'Skill name is required.'
      : !trimmedDescription
        ? 'Skill description is required.'
        : !trimmedBody
          ? 'Skill content is required.'
          : !trimmedId
            ? 'Skill ID is required. Open Advanced to set a slug.'
            : !SKILL_ID_PATTERN.test(trimmedId)
              ? 'Skill ID must match [a-z0-9-].'
              : duplicateSkill
                ? `Skill ID "${trimmedId}" already exists.`
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
              <span className="codex-section-title">Skills</span>
              <span className="codex-section-note">{skills.length} total</span>
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
                {loading ? 'Refreshing...' : 'Refresh'}
              </button>
              <button
                className="codex-section-action"
                disabled={creating || Boolean(mutatingSkillId) || Boolean(editorBusy)}
                onClick={openCreateDialog}
                type="button"
              >
                {PlusIcon} New Skill
              </button>
            </div>
          </div>
        </div>

        {loading ? (
          <div className="codex-empty-state skills-panel-empty">Loading skills...</div>
        ) : !skills.length ? (
          <div className="codex-empty-state skills-panel-empty">No skills installed. Create your first skill.</div>
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
                      <span className="codex-sync-pill ok">{sourceLabel(skill)}</span>
                    </div>
                    {showId ? <span className="codex-command-row-desc">{skill.id}</span> : null}
                    <span className="codex-command-row-desc skills-panel-description">
                      {skill.description || 'No description provided.'}
                    </span>
                  </div>
                  <div className="codex-list-row-actions">
                    <button
                      className="codex-icon-button"
                      disabled={busy || creating || Boolean(editorBusy)}
                      onClick={() => {
                        void openSkillEditor(skill);
                      }}
                      title="Edit"
                      type="button"
                    >
                      {GearIcon}
                    </button>
                    <button
                      className="codex-icon-button codex-icon-button-danger"
                      disabled={busy || creating || Boolean(editorBusy)}
                      onClick={() => {
                        void handleDeleteSkill(skill);
                      }}
                      title="Delete"
                      type="button"
                    >
                      {TrashIcon}
                    </button>
                    <button
                      aria-label={skill.enabled ? 'Disable skill' : 'Enable skill'}
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
                <span className="eyebrow">Create Skill</span>
                <h3 className="panel-title" id="skills-create-dialog-title">New Skill</h3>
              </div>
              <button
                aria-label="Close skill dialog"
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
                <span>Name</span>
                <input
                  autoFocus
                  onChange={(event) => {
                    setDraft((current) => ({
                      ...current,
                      name: event.target.value,
                    }));
                  }}
                  placeholder="Example Skill"
                  value={draft.name}
                />
              </label>

              <label>
                <span>Description</span>
                <textarea
                  onChange={(event) => {
                    setDraft((current) => ({
                      ...current,
                      description: event.target.value,
                    }));
                  }}
                  placeholder="What this skill should help Garyx do."
                  value={draft.description}
                />
              </label>

              <label>
                <span>Skill Content</span>
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
                Frontmatter is generated automatically from Name and Description. Write only the markdown body for <code>SKILL.md</code> here.
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
                  {createAdvancedOpen ? 'Hide Advanced' : 'Advanced'}
                </button>
                <span className="small-note">
                  Skill ID: <code>{trimmedId || 'set manually'}</code>
                </span>
              </div>

              {createAdvancedOpen ? (
                <div className="skills-advanced-panel">
                  <label>
                    <span>Skill ID</span>
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
                    Stable slug used as the skill directory name and API key under <code>~/.garyx/skills/&lt;id&gt;</code>.
                  </p>
                </div>
              ) : null}

              <p className={`small-note skills-form-note ${validationError ? 'error' : ''}`}>
                {validationError || 'The skill is created immediately with a real SKILL.md body, then opened in the full directory editor.'}
              </p>

              <div className="skills-form-footer">
                <button
                  className="ghost-button"
                  disabled={creating}
                  onClick={closeCreateDialog}
                  type="button"
                >
                  Cancel
                </button>
                <button
                  className="primary-button"
                  disabled={creating || Boolean(validationError)}
                  type="submit"
                >
                  {creating ? 'Creating...' : 'Create Skill'}
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
                <span className="eyebrow">Skill Editor</span>
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
                  New File
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
                  New Folder
                </button>
                <button
                  className="codex-icon-button codex-icon-button-danger"
                  disabled={!editor.selectedPath || editor.selectedPath === 'SKILL.md' || Boolean(editorBusy)}
                  onClick={() => {
                    void handleDeleteEditorEntry();
                  }}
                  title="Delete file"
                  type="button"
                >
                  {TrashIcon}
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
                  {editorBusy?.startsWith('save:') ? 'Saving...' : 'Save'}
                </button>
                <button
                  aria-label="Close skill editor"
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
                  <strong>{editor.selectedPath || 'No file selected'}</strong>
                  <div className="skills-editor-filebar-status">
                    {skillPreviewBadgeLabel(editor.selectedDocument) ? (
                      <span className="codex-sync-pill ok">
                        {skillPreviewBadgeLabel(editor.selectedDocument)}
                      </span>
                    ) : null}
                    {editorDirty ? <span className="codex-sync-pill fail">Unsaved</span> : null}
                    {editor.selectedDocument && !editor.selectedDocument.editable ? (
                      <span className="codex-sync-pill ok">Read-only</span>
                    ) : null}
                  </div>
                </div>

                {editor.selectedPath ? (
                  editor.selectedDocument?.previewKind === 'image' && editor.selectedDocument.dataBase64 ? (
                    <div className="skills-editor-preview-shell">
                      <div className="skills-editor-preview-copy">
                        <span className="eyebrow">Image Preview</span>
                        <p>{skillPreviewMessage(editor.selectedDocument)}</p>
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
                        {editor.selectedDocument?.previewKind === 'image' ? 'Image Preview' : 'Preview Unavailable'}
                      </span>
                      <p>{skillPreviewMessage(editor.selectedDocument)}</p>
                      {editor.selectedDocument?.mediaType ? (
                        <code>{editor.selectedDocument.mediaType}</code>
                      ) : null}
                    </div>
                  )
                ) : (
                  <div className="workspace-empty-block skills-editor-empty">
                    <span className="eyebrow">No Files</span>
                    <p>Create a file to start editing this skill directory.</p>
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
