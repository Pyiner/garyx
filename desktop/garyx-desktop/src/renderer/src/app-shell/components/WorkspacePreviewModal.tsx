import type { DesktopWorkspaceFilePreview } from '@shared/contracts';
import { CopyIcon, FolderOpenIcon } from 'lucide-react';

import { WorkspaceFilePreview } from '../../workspace-file-preview';
import { workspaceFileAbsolutePath } from '../workspace-helpers';

type WorkspacePreviewModalProps = {
  open: boolean;
  title: string;
  preview: DesktopWorkspaceFilePreview | null;
  loading: boolean;
  error: string | null;
  onClose: () => void;
  onLocalFileLinkClick: (absolutePath: string) => void;
};

function lastPathSegment(path: string): string {
  const normalized = path.trim().replace(/[\\/]+$/g, '');
  const segments = normalized.split(/[\\/]/).filter(Boolean);
  return segments[segments.length - 1] || normalized || 'Select a file';
}

function parentPath(path: string): string {
  const normalized = path.trim().replace(/[\\/]+$/g, '');
  const lastSlash = Math.max(
    normalized.lastIndexOf('/'),
    normalized.lastIndexOf('\\'),
  );
  if (lastSlash < 0) {
    return '';
  }
  return lastSlash === 0 ? '/' : `${normalized.slice(0, lastSlash)}/`;
}

function extensionFromName(fileName: string): string {
  const dotIndex = fileName.lastIndexOf('.');
  if (dotIndex < 0 || dotIndex === fileName.length - 1) {
    return '';
  }
  return fileName.slice(dotIndex + 1).toLowerCase();
}

function fileTypeMeta(preview: DesktopWorkspaceFilePreview | null, fileName: string): {
  className: string;
  label: string;
} {
  const extension = extensionFromName(fileName);
  if (extension === 'json') {
    return { className: 'kind-json', label: 'JSON' };
  }
  if (extension === 'md' || preview?.previewKind === 'markdown') {
    return { className: 'kind-markdown', label: 'MD' };
  }
  if (extension === 'html' || extension === 'htm' || preview?.previewKind === 'html') {
    return { className: 'kind-html', label: 'HTML' };
  }
  if (preview?.previewKind === 'pdf') {
    return { className: 'kind-pdf', label: 'PDF' };
  }
  if (preview?.previewKind === 'image') {
    return {
      className: 'kind-image',
      label: extension ? extension.slice(0, 4).toUpperCase() : 'IMG',
    };
  }
  if (preview?.previewKind === 'text') {
    return {
      className: 'kind-text',
      label: extension ? extension.slice(0, 4).toUpperCase() : 'TXT',
    };
  }
  return {
    className: 'kind-file',
    label: extension ? extension.slice(0, 4).toUpperCase() : 'FILE',
  };
}

export function WorkspacePreviewModal({
  open,
  title,
  preview,
  loading,
  error,
  onClose,
  onLocalFileLinkClick,
}: WorkspacePreviewModalProps) {
  const absolutePath = preview?.workspacePath && preview.path
    ? workspaceFileAbsolutePath(preview.workspacePath, preview.path)
    : title;
  const fileName = preview?.name || lastPathSegment(title);
  const fileParentPath = parentPath(absolutePath);
  const typeMeta = fileTypeMeta(preview, fileName);
  const canCopyPath = Boolean(absolutePath && absolutePath !== 'Select a file');
  const canRevealInFinder = Boolean(preview?.workspacePath && preview?.path);

  async function handleCopyPath() {
    if (!canCopyPath) {
      return;
    }
    try {
      await navigator.clipboard.writeText(absolutePath);
    } catch {
      // Clipboard can be unavailable in constrained preview environments.
    }
  }

  async function handleRevealInFinder() {
    if (!preview?.workspacePath || !preview.path) {
      return;
    }
    try {
      await window.garyxDesktop.revealWorkspaceFile({
        workspacePath: preview.workspacePath,
        filePath: preview.path,
      });
    } catch {
      // Keep the preview modal quiet if Finder cannot reveal the file.
    }
  }

  return (
    <>
      <button
        aria-hidden={!open}
        className={`workspace-preview-backdrop ${open ? 'visible' : ''}`}
        onClick={onClose}
        tabIndex={-1}
        type="button"
      />

      {open ? (
        <div
          aria-modal="true"
          className="workspace-preview-modal"
          role="dialog"
        >
          <div className="workspace-preview-modal-shell">
            <div className="workspace-preview-modal-header">
              <div className="workspace-preview-modal-copy" title={absolutePath}>
                <span
                  className={`workspace-preview-file-type ${typeMeta.className} ${
                    typeMeta.label.length > 3 ? 'is-wide' : ''
                  }`}
                >
                  {typeMeta.label}
                </span>
                <strong className="workspace-preview-modal-filename">
                  {fileName}
                </strong>
                {fileParentPath ? (
                  <span className="workspace-preview-modal-crumbs">
                    <span>{fileParentPath}</span>
                  </span>
                ) : null}
              </div>
              <div className="workspace-preview-modal-actions">
                {preview?.truncated ? (
                  <span className="workspace-preview-modal-status">Truncated</span>
                ) : null}
                <button
                  aria-label="Copy file path"
                  className="workspace-preview-icon-button"
                  data-tip="Copy path"
                  disabled={!canCopyPath}
                  onClick={() => {
                    void handleCopyPath();
                  }}
                  title="Copy path"
                  type="button"
                >
                  <CopyIcon aria-hidden size={13} strokeWidth={1.8} />
                </button>
                <button
                  aria-label="Show in Finder"
                  className="workspace-preview-icon-button"
                  data-tip="Show in Finder"
                  disabled={!canRevealInFinder}
                  onClick={() => {
                    void handleRevealInFinder();
                  }}
                  title="Show in Finder"
                  type="button"
                >
                  <FolderOpenIcon aria-hidden size={13} strokeWidth={1.8} />
                </button>
                <span className="workspace-preview-modal-divider" />
                <button
                  className="workspace-preview-close-button"
                  onClick={onClose}
                  type="button"
                >
                  Close
                </button>
              </div>
            </div>
            {error ? (
              <div className="workspace-file-error workspace-file-preview-error">
                {error}
              </div>
            ) : null}
            <div className="workspace-preview-modal-body">
              {loading ? (
                <div className="workspace-file-empty">Loading preview…</div>
              ) : (
                <WorkspaceFilePreview
                  onLocalFileLinkClick={onLocalFileLinkClick}
                  preview={preview}
                />
              )}
            </div>
          </div>
        </div>
      ) : null}
    </>
  );
}
