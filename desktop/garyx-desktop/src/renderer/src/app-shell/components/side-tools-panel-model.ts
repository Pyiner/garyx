const EMPTY_WORKSPACE_PREVIEW_TITLE = "Select a file";
const PENDING_WORKSPACE_PREVIEW_KEY = "pending-workspace-preview";

export function workspacePreviewDirectoryCollapseKey(input: {
  shouldShowWorkspacePreview: boolean;
  workspaceFilePreviewPath?: string | null;
  workspacePreviewTitle?: string | null;
}): string | null {
  if (!input.shouldShowWorkspacePreview) {
    return null;
  }

  const title = input.workspacePreviewTitle?.trim();
  if (title && title !== EMPTY_WORKSPACE_PREVIEW_TITLE) {
    return `title:${title}`;
  }

  const previewPath = input.workspaceFilePreviewPath?.trim();
  if (previewPath) {
    return `path:${previewPath}`;
  }

  return PENDING_WORKSPACE_PREVIEW_KEY;
}

export function shouldCollapseFileDirectoryForPreview(input: {
  nextPreviewKey: string | null;
  previousPreviewKey: string | null;
}): boolean {
  return Boolean(
    input.nextPreviewKey && input.nextPreviewKey !== input.previousPreviewKey,
  );
}
