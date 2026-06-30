const EMPTY_WORKSPACE_PREVIEW_TITLE = "Select a file";
const PENDING_WORKSPACE_PREVIEW_KEY = "pending-workspace-preview";

/** Built-in side-tool tabs. Each is a singleton in the panel's tab track. */
export type ThreadSideToolId = "files" | "tasks" | "chat" | "browser" | "terminal";

/** Prefix that marks a capsule tab key, e.g. `capsule:<capsuleId>`. */
export const CAPSULE_TAB_PREFIX = "capsule:";

/**
 * A tab in the side-tools dock: either a built-in tool or a capsule instance.
 * Capsule tabs let one dock host several capsules at once (#TASK-1470), reusing
 * the same open/close/active mechanism as the built-in tools.
 */
export type SideTabKey = ThreadSideToolId | `capsule:${string}`;

export function capsuleTabKey(capsuleId: string): SideTabKey {
  return `${CAPSULE_TAB_PREFIX}${capsuleId}`;
}

export function isCapsuleTabKey(key: string): key is `capsule:${string}` {
  return key.startsWith(CAPSULE_TAB_PREFIX);
}

export function capsuleIdFromTabKey(key: string): string | null {
  return isCapsuleTabKey(key) ? key.slice(CAPSULE_TAB_PREFIX.length) : null;
}

/**
 * Remove `key` from the combined open-tab list and pick the next active tab.
 * If the closed tab was active, the last remaining tab becomes active (matching
 * the previous built-in `closeTool` behavior); an empty list clears the active
 * tab. The caller dispatches the actual removal to the right store (local
 * built-in tools vs. gateway-owned capsule tabs); this only computes the
 * resulting tab list and active key.
 */
export function closeTab(
  openTabs: SideTabKey[],
  activeKey: SideTabKey | null,
  key: SideTabKey,
): { openTabs: SideTabKey[]; activeKey: SideTabKey | null } {
  const next = openTabs.filter((tab) => tab !== key);
  if (!next.length) {
    return { openTabs: next, activeKey: null };
  }
  if (activeKey === key) {
    return { openTabs: next, activeKey: next[next.length - 1] };
  }
  return { openTabs: next, activeKey };
}

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
