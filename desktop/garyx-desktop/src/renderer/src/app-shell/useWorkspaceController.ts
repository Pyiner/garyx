import { startTransition, useCallback, useRef, useState } from 'react';

import type {
  DesktopWorkspace,
  DesktopWorkspaceFileEntry,
  DesktopWorkspaceFileListing,
  DesktopWorkspaceFilePreview,
} from '@shared/contracts';

import type { ToastTone } from '../toast';
import type { WorkspaceDirectoryState } from './types';
import {
  expandWorkspaceDirectoryState,
  findWorkspaceFileEntry,
  parentDirectoryPath,
  resolveLocalFilePreviewTarget,
  workspaceDirectoryKey,
  workspaceFileAbsolutePath,
} from './workspace-helpers';

type UseWorkspaceControllerArgs = {
  activeWorkspacePath: string;
  workspaces: DesktopWorkspace[];
  setError: React.Dispatch<React.SetStateAction<string | null>>;
  pushToast: (message: string, tone?: ToastTone, durationMs?: number) => void;
};

function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => {
      reject(new Error(`Failed to read ${file.name}`));
    };
    reader.onload = () => {
      const result = typeof reader.result === 'string' ? reader.result : '';
      const commaIndex = result.indexOf(',');
      if (commaIndex < 0) {
        reject(new Error(`Failed to decode ${file.name}`));
        return;
      }
      resolve(result.slice(commaIndex + 1));
    };
    reader.readAsDataURL(file);
  });
}

export function useWorkspaceController({
  activeWorkspacePath,
  workspaces,
  setError,
  pushToast,
}: UseWorkspaceControllerArgs) {
  const [workspaceDirectories, setWorkspaceDirectories] = useState<
    Record<string, WorkspaceDirectoryState>
  >({});
  const [expandedWorkspaceDirectories, setExpandedWorkspaceDirectories] = useState<
    Record<string, boolean>
  >({});
  const [selectedWorkspaceFile, setSelectedWorkspaceFile] = useState<{
    workspacePath: string;
    path: string;
  } | null>(null);
  const [workspaceFilePreview, setWorkspaceFilePreview] = useState<DesktopWorkspaceFilePreview | null>(null);
  const [workspaceFilePreviewLoading, setWorkspaceFilePreviewLoading] = useState(false);
  const [workspaceFilePreviewError, setWorkspaceFilePreviewError] = useState<string | null>(null);
  const [workspacePreviewModalOpen, setWorkspacePreviewModalOpen] = useState(false);
  const [workspaceFileUploadPending, setWorkspaceFileUploadPending] = useState(false);

  const workspaceUploadInputRef = useRef<HTMLInputElement | null>(null);
  const workspacePreviewRequestIdRef = useRef(0);

  const activeWorkspaceRootKey = activeWorkspacePath
    ? workspaceDirectoryKey(activeWorkspacePath, '')
    : '';
  const activeWorkspaceDirectoryState = activeWorkspaceRootKey
    ? workspaceDirectories[activeWorkspaceRootKey]
    : undefined;
  const selectedWorkspaceFileEntry = selectedWorkspaceFile
    && activeWorkspacePath
    && selectedWorkspaceFile.workspacePath === activeWorkspacePath
    ? findWorkspaceFileEntry(workspaceDirectories, activeWorkspacePath, selectedWorkspaceFile.path)
    : null;
  const workspaceUploadDirectoryPath = selectedWorkspaceFileEntry?.entryType === 'directory'
    ? selectedWorkspaceFileEntry.path
    : selectedWorkspaceFile?.workspacePath === activeWorkspacePath
      ? parentDirectoryPath(selectedWorkspaceFile.path)
      : '';
  const workspacePreviewTitle = selectedWorkspaceFile
    ? workspaceFileAbsolutePath(
        selectedWorkspaceFile.workspacePath,
        selectedWorkspaceFile.path,
      )
    : 'Select a file';

  async function loadWorkspaceDirectory(
    workspacePath: string,
    directoryPath = '',
    options?: { force?: boolean },
  ): Promise<DesktopWorkspaceFileListing | null> {
    const key = workspaceDirectoryKey(workspacePath, directoryPath);
    const current = workspaceDirectories[key];
    if (!options?.force && (current?.loading || current?.loaded)) {
      return null;
    }

    setWorkspaceDirectories((existing) => ({
      ...existing,
      [key]: {
        entries: existing[key]?.entries || [],
        loading: true,
        loaded: existing[key]?.loaded || false,
        error: null,
      },
    }));

    try {
      const listing = await window.garyxDesktop.listWorkspaceFiles({
        workspacePath,
        directoryPath,
      });
      setWorkspaceDirectories((existing) => ({
        ...existing,
        [key]: {
          entries: listing.entries,
          loading: false,
          loaded: true,
          error: null,
        },
      }));
      return listing;
    } catch (workspaceError) {
      const message =
        workspaceError instanceof Error
          ? workspaceError.message
          : 'Failed to load workspace files.';
      setWorkspaceDirectories((existing) => ({
        ...existing,
        [key]: {
          entries: existing[key]?.entries || [],
          loading: false,
          loaded: existing[key]?.loaded || false,
          error: message,
        },
      }));
      return null;
    }
  }

  function expandWorkspaceDirectoryChain(workspacePath: string, directoryPath: string) {
    setExpandedWorkspaceDirectories((current) => {
      return expandWorkspaceDirectoryState(current, workspacePath, directoryPath);
    });
  }

  async function loadWorkspaceFilePreview(
    workspacePath: string,
    filePath: string,
  ) {
    const requestId = workspacePreviewRequestIdRef.current + 1;
    workspacePreviewRequestIdRef.current = requestId;
    setSelectedWorkspaceFile({
      workspacePath,
      path: filePath,
    });
    setWorkspaceFilePreviewLoading(true);
    setWorkspaceFilePreviewError(null);
    try {
      const preview = await window.garyxDesktop.previewWorkspaceFile({
        workspacePath,
        filePath,
      });
      if (workspacePreviewRequestIdRef.current !== requestId) {
        return;
      }
      startTransition(() => {
        setWorkspaceFilePreview(preview);
      });
    } catch (previewError) {
      if (workspacePreviewRequestIdRef.current !== requestId) {
        return;
      }
      const message =
        previewError instanceof Error
          ? previewError.message
          : 'Failed to preview file.';
      setWorkspaceFilePreview(null);
      setWorkspaceFilePreviewError(message);
    } finally {
      if (workspacePreviewRequestIdRef.current === requestId) {
        setWorkspaceFilePreviewLoading(false);
      }
    }
  }

  const handleLocalWorkspaceFileLinkClick = useCallback((absolutePath: string) => {
    void (async () => {
      const target = resolveLocalFilePreviewTarget(workspaces, absolutePath);
      if (!target) {
        setError('Garyx could not resolve this local file for preview.');
        return;
      }

      setError(null);
      setWorkspacePreviewModalOpen(true);
      await loadWorkspaceFilePreview(target.workspacePath, target.filePath);
    })();
  }, [setError, workspaces]);

  async function handleWorkspaceFileEntryActivate(entry: DesktopWorkspaceFileEntry) {
    if (!activeWorkspacePath) {
      return;
    }

    if (entry.entryType === 'directory') {
      const key = workspaceDirectoryKey(activeWorkspacePath, entry.path);
      const nextExpanded = !expandedWorkspaceDirectories[key];
      setExpandedWorkspaceDirectories((current) => ({
        ...current,
        [key]: nextExpanded,
      }));
      if (nextExpanded) {
        expandWorkspaceDirectoryChain(activeWorkspacePath, entry.path);
        void loadWorkspaceDirectory(activeWorkspacePath, entry.path);
      }
      return;
    }

    setWorkspacePreviewModalOpen(true);
    await loadWorkspaceFilePreview(activeWorkspacePath, entry.path);
  }

  async function handleRefreshWorkspaceFiles() {
    if (!activeWorkspacePath) {
      return;
    }

    const rootPath = '';
    const loadedDirectories = Object.keys(workspaceDirectories)
      .filter((key) => key.startsWith(`${activeWorkspacePath}::`))
      .map((key) => key.slice(activeWorkspacePath.length + 2));
    const directoriesToRefresh = new Set<string>([rootPath, ...loadedDirectories]);

    await Promise.all(
      [...directoriesToRefresh].map((directoryPath) =>
        loadWorkspaceDirectory(activeWorkspacePath, directoryPath, { force: true })),
    );

    if (selectedWorkspaceFile?.workspacePath === activeWorkspacePath && selectedWorkspaceFile.path) {
      await loadWorkspaceFilePreview(activeWorkspacePath, selectedWorkspaceFile.path);
    }
  }

  async function uploadWorkspaceFilesToActiveWorkspace(files: File[]) {
    if (!activeWorkspacePath) {
      setError('Select an available workspace before uploading files.');
      return;
    }
    if (!files.length) {
      return;
    }

    setWorkspaceFileUploadPending(true);
    setWorkspaceFilePreviewError(null);
    try {
      const payloadFiles = await Promise.all(files.map(async (file) => ({
        name: file.name || 'file',
        mediaType: file.type || null,
        dataBase64: await fileToBase64(file),
      })));

      const result = await window.garyxDesktop.uploadWorkspaceFiles({
        workspacePath: activeWorkspacePath,
        directoryPath: workspaceUploadDirectoryPath,
        files: payloadFiles,
      });
      expandWorkspaceDirectoryChain(activeWorkspacePath, workspaceUploadDirectoryPath);
      await loadWorkspaceDirectory(activeWorkspacePath, workspaceUploadDirectoryPath, {
        force: true,
      });
      const uploadedTarget = result.uploadedPaths[0];
      if (uploadedTarget) {
        await loadWorkspaceFilePreview(activeWorkspacePath, uploadedTarget);
      }
      pushToast(
        files.length === 1
          ? `Uploaded ${files[0]?.name || 'file'}.`
          : `Uploaded ${files.length} files.`,
        'success',
      );
    } catch (uploadError) {
      setError(
        uploadError instanceof Error
          ? uploadError.message
          : 'Failed to upload workspace files.',
      );
    } finally {
      setWorkspaceFileUploadPending(false);
    }
  }

  return {
    activeWorkspaceDirectoryState,
    expandedWorkspaceDirectories,
    handleLocalWorkspaceFileLinkClick,
    handleRefreshWorkspaceFiles,
    handleWorkspaceFileEntryActivate,
    loadWorkspaceDirectory,
    loadWorkspaceFilePreview,
    selectedWorkspaceFile,
    selectedWorkspaceFileEntry,
    setExpandedWorkspaceDirectories,
    setWorkspacePreviewModalOpen,
    uploadWorkspaceFilesToActiveWorkspace,
    workspaceDirectories,
    workspaceFilePreview,
    workspaceFilePreviewError,
    workspaceFilePreviewLoading,
    workspaceFileUploadPending,
    workspacePreviewModalOpen,
    workspacePreviewTitle,
    workspaceUploadDirectoryPath,
    workspaceUploadInputRef,
  };
}
