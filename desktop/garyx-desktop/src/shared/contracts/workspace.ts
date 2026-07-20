export type DesktopWorkspaceKind = "local";

// Directory summary used by the desktop UI. The path string is the identity;
// thread/automation source of truth remains `workspace_dir`. The gateway is
// the single source of truth for `name`, `pinned`, aggregates, and list
// order — clients render the server list verbatim and never re-sort or
// rewrite names locally.
export interface DesktopWorkspace {
  name: string;
  path: string | null;
  kind: DesktopWorkspaceKind;
  createdAt: string;
  updatedAt: string;
  available: boolean;
  managed?: boolean;
  pinned: boolean;
  threadCount: number;
  lastActivityAt: string | null;
  gitRepo: boolean;
}

// `gatewayHome` is the gateway machine's home directory; clients use it for
// `~` path abbreviation and must never substitute the local HOME.
export interface DesktopWorkspaceCatalog {
  workspaces: DesktopWorkspace[];
  gatewayHome: string | null;
  workspaceStateInitialized: boolean;
}

export interface DesktopLocalDirectoryEntry {
  name: string;
  path: string;
  gitRepo: boolean;
}

export interface DesktopLocalDirectoryListing {
  path: string;
  parentPath: string | null;
  entries: DesktopLocalDirectoryEntry[];
}

// Typed directory-listing failure codes from the gateway. The browser
// renders these inline and stays on its current directory.
export type DesktopDirectoryListingErrorCode =
  | "invalid_path"
  | "not_found"
  | "not_a_directory"
  | "permission_denied";

/**
 * Draft workspace selection for an unsent new thread. Explicit tri-state:
 * a concrete path, or the user's explicit "No workspace" choice. The
 * default is resolved once when a draft is created (never re-resolved on
 * list refresh), so a draft holds a concrete selection from birth.
 */
export type DraftWorkspaceSelection =
  | { kind: "path"; path: string }
  | { kind: "none" };

export interface PinWorkspaceInput {
  workspacePath: string;
  pinned: boolean;
}

export interface RenameWorkspaceInput {
  workspacePath: string;
  name: string;
}

export interface DesktopWorkspaceFileEntry {
  path: string;
  name: string;
  entryType: "file" | "directory";
  size?: number | null;
  modifiedAt?: string | null;
  mediaType?: string | null;
  hasChildren: boolean;
}

export interface DesktopWorkspaceFileListing {
  workspacePath: string;
  directoryPath: string;
  entries: DesktopWorkspaceFileEntry[];
}

export type DesktopWorkspaceFilePreviewKind =
  | "markdown"
  | "html"
  | "text"
  | "pdf"
  | "image"
  | "unsupported";

export interface DesktopWorkspaceFilePreview {
  workspacePath: string;
  path: string;
  name: string;
  mediaType: string;
  previewKind: DesktopWorkspaceFilePreviewKind;
  size: number;
  modifiedAt?: string | null;
  truncated: boolean;
  text?: string | null;
  dataBase64?: string | null;
}

export type DesktopWorkspaceMode = "local" | "worktree";

export interface DesktopWorkspaceGitStatus {
  workspaceDir: string;
  isGitRepo: boolean;
  repoRoot?: string | null;
  currentBranch?: string | null;
  isDirty: boolean;
}

export interface ListWorkspaceFilesInput {
  workspacePath: string;
  directoryPath?: string;
}

export interface PreviewWorkspaceFileInput {
  workspacePath: string;
  filePath: string;
}

export type RevealWorkspaceFileInput = PreviewWorkspaceFileInput;

export interface UploadWorkspaceFileBlob {
  name: string;
  mediaType?: string | null;
  dataBase64: string;
}

export interface UploadWorkspaceFilesInput {
  workspacePath: string;
  directoryPath?: string;
  files: UploadWorkspaceFileBlob[];
}

export interface UploadWorkspaceFilesResult {
  workspacePath: string;
  directoryPath: string;
  uploadedPaths: string[];
}

export interface SelectWorkspaceInput {
  workspacePath: string | null;
}

export interface RemoveWorkspaceInput {
  workspacePath: string;
}

export interface AddWorkspaceByPathInput {
  path: string;
  name?: string | null;
}
