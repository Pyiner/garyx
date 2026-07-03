export type DesktopWorkspaceKind = "local";

// Directory summary used by the desktop UI. The path string is the identity;
// thread/automation source of truth remains `workspace_dir`.
export interface DesktopWorkspace {
  name: string;
  path: string | null;
  kind: DesktopWorkspaceKind;
  createdAt: string;
  updatedAt: string;
  available: boolean;
  managed?: boolean;
}

export interface DesktopLocalDirectoryEntry {
  name: string;
  path: string;
}

export interface DesktopLocalDirectoryListing {
  path: string;
  parentPath: string | null;
  entries: DesktopLocalDirectoryEntry[];
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

export interface DesktopWorkspaceGitFile {
  path: string;
  status: string;
}

export interface DesktopWorkspaceGitDetails extends DesktopWorkspaceGitStatus {
  ahead: number;
  behind: number;
  changedCount: number;
  stagedCount: number;
  unstagedCount: number;
  untrackedCount: number;
  files: DesktopWorkspaceGitFile[];
}

export interface CommitWorkspaceChangesInput {
  workspacePath: string;
  message: string;
}

export interface PushWorkspaceBranchInput {
  workspacePath: string;
}

export interface WorkspaceGitMutationResult {
  status: DesktopWorkspaceGitDetails;
  output: string;
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
}
