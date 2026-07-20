import { useEffect, useRef, useState, type FormEvent } from 'react';
import { X } from 'lucide-react';

import type { DesktopWorkspace } from '@shared/contracts';

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from './ui/dialog';
import { useI18n } from '../i18n';

type WorkspaceRenameDialogProps = {
  workspace: DesktopWorkspace | null;
  saving: boolean;
  onSubmit: (workspace: DesktopWorkspace, name: string) => void;
  onCancel: () => void;
};

/** Rename a workspace's display name. Same compact rename form factor as the
 *  conversation rename dialog; the path (identity) is shown but never
 *  editable. */
export function WorkspaceRenameDialog({
  workspace,
  saving,
  onSubmit,
  onCancel,
}: WorkspaceRenameDialogProps) {
  const { t } = useI18n();
  const inputRef = useRef<HTMLInputElement | null>(null);
  const [draft, setDraft] = useState('');

  useEffect(() => {
    if (workspace) {
      setDraft(workspace.name);
    }
  }, [workspace]);

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!workspace || saving) {
      return;
    }
    const name = draft.trim();
    if (!name || name === workspace.name) {
      onCancel();
      return;
    }
    onSubmit(workspace, name);
  };

  return (
    <Dialog
      onOpenChange={(open) => {
        if (!open) {
          onCancel();
        }
      }}
      open={Boolean(workspace)}
    >
      <DialogContent
        className="thread-rename-dialog"
        onOpenAutoFocus={(event) => {
          const input = inputRef.current;
          if (!input) {
            return;
          }
          event.preventDefault();
          input.focus();
          input.select();
        }}
        overlayClassName="thread-rename-overlay"
        showCloseButton={false}
        size="compact"
      >
        <form className="thread-rename-form" onSubmit={handleSubmit}>
          <button
            aria-label={t('Close')}
            className="thread-rename-close"
            onClick={onCancel}
            type="button"
          >
            <X aria-hidden size={16} strokeWidth={2} />
          </button>
          <div className="thread-rename-copy">
            <DialogTitle className="thread-rename-title">
              {t('Rename workspace')}
            </DialogTitle>
            <DialogDescription className="thread-rename-description">
              {workspace?.path || ''}
            </DialogDescription>
          </div>
          <input
            ref={inputRef}
            aria-label={t('Workspace name')}
            className="thread-rename-input"
            disabled={saving}
            onChange={(event) => {
              setDraft(event.target.value);
            }}
            placeholder={t('Workspace name')}
            value={draft}
          />
          <div className="thread-rename-actions">
            <button
              className="thread-rename-button thread-rename-button-secondary"
              disabled={saving}
              onClick={onCancel}
              type="button"
            >
              {t('Cancel')}
            </button>
            <button
              className="thread-rename-button thread-rename-button-primary"
              disabled={saving || !draft.trim()}
              type="submit"
            >
              {t('Save')}
            </button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
