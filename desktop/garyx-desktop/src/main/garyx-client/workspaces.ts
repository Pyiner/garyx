import type {
  DesktopLocalDirectoryListing,
  DesktopSettings,
  DesktopWorkspace,
  DesktopWorkspaceFileEntry,
  DesktopWorkspaceFileListing,
  DesktopWorkspaceFilePreview,
  ListWorkspaceFilesInput,
  PreviewWorkspaceFileInput,
  UploadChatAttachmentsInput,
  UploadChatAttachmentsResult,
  UploadWorkspaceFilesInput,
  UploadWorkspaceFilesResult,
} from "@shared/contracts";
import { REMOTE_STATE_FETCH_TIMEOUT_MS, requestJson } from "./http.ts";

interface WorkspaceFileEntryPayload {
  path?: string | null;
  name?: string | null;
  entryType?: string | null;
  entry_type?: string | null;
  size?: number | null;
  modifiedAt?: string | null;
  modified_at?: string | null;
  mediaType?: string | null;
  media_type?: string | null;
  hasChildren?: boolean;
  has_children?: boolean;
}

interface WorkspaceFileListingPayload {
  workspaceDir?: string | null;
  workspace_dir?: string | null;
  directoryPath?: string | null;
  directory_path?: string | null;
  entries?: WorkspaceFileEntryPayload[] | null;
}

interface WorkspaceFilePreviewPayload {
  workspaceDir?: string | null;
  workspace_dir?: string | null;
  path?: string | null;
  name?: string | null;
  mediaType?: string | null;
  media_type?: string | null;
  previewKind?: string | null;
  preview_kind?: string | null;
  size?: number | null;
  modifiedAt?: string | null;
  modified_at?: string | null;
  truncated?: boolean;
  text?: string | null;
  dataBase64?: string | null;
  data_base64?: string | null;
}

interface UploadWorkspaceFilesPayload {
  workspaceDir?: string | null;
  workspace_dir?: string | null;
  directoryPath?: string | null;
  directory_path?: string | null;
  uploadedPaths?: string[] | null;
  uploaded_paths?: string[] | null;
}

interface UploadedChatAttachmentPayload {
  kind?: "image" | "file" | null;
  path?: string | null;
  name?: string | null;
  mediaType?: string | null;
  media_type?: string | null;
}

interface UploadChatAttachmentsPayload {
  files?: UploadedChatAttachmentPayload[] | null;
}

function mapWorkspaceFileEntry(
  value: WorkspaceFileEntryPayload,
): DesktopWorkspaceFileEntry {
  const entryType = value.entryType || value.entry_type;
  return {
    path: typeof value.path === "string" ? value.path : "",
    name: typeof value.name === "string" ? value.name : "",
    entryType: entryType === "directory" ? "directory" : "file",
    size:
      typeof value.size === "number" && Number.isFinite(value.size)
        ? value.size
        : null,
    modifiedAt:
      typeof value.modifiedAt === "string"
        ? value.modifiedAt
        : typeof value.modified_at === "string"
          ? value.modified_at
          : null,
    mediaType:
      typeof value.mediaType === "string"
        ? value.mediaType
        : typeof value.media_type === "string"
          ? value.media_type
          : null,
    hasChildren: value.hasChildren === true || value.has_children === true,
  };
}

function mapWorkspaceFileListing(
  value: WorkspaceFileListingPayload,
): DesktopWorkspaceFileListing {
  return {
    workspacePath:
      (typeof value.workspaceDir === "string" && value.workspaceDir) ||
      (typeof value.workspace_dir === "string" && value.workspace_dir) ||
      "",
    directoryPath:
      (typeof value.directoryPath === "string" && value.directoryPath) ||
      (typeof value.directory_path === "string" && value.directory_path) ||
      "",
    entries: Array.isArray(value.entries)
      ? value.entries.map(mapWorkspaceFileEntry)
      : [],
  };
}

function normalizeWorkspaceFilePreviewKind(
  value: unknown,
): DesktopWorkspaceFilePreview["previewKind"] {
  switch (value) {
    case "markdown":
    case "html":
    case "text":
    case "pdf":
    case "image":
    case "unsupported":
      return value;
    default:
      return "unsupported";
  }
}

function mapWorkspaceFilePreview(
  value: WorkspaceFilePreviewPayload,
): DesktopWorkspaceFilePreview {
  return {
    workspacePath:
      (typeof value.workspaceDir === "string" && value.workspaceDir) ||
      (typeof value.workspace_dir === "string" && value.workspace_dir) ||
      "",
    path: typeof value.path === "string" ? value.path : "",
    name: typeof value.name === "string" ? value.name : "",
    mediaType:
      (typeof value.mediaType === "string" && value.mediaType) ||
      (typeof value.media_type === "string" ? value.media_type : "") ||
      "application/octet-stream",
    previewKind: normalizeWorkspaceFilePreviewKind(
      value.previewKind || value.preview_kind,
    ),
    size:
      typeof value.size === "number" && Number.isFinite(value.size)
        ? value.size
        : 0,
    modifiedAt:
      typeof value.modifiedAt === "string"
        ? value.modifiedAt
        : typeof value.modified_at === "string"
          ? value.modified_at
          : null,
    truncated: value.truncated === true,
    text: typeof value.text === "string" ? value.text : null,
    dataBase64:
      typeof value.dataBase64 === "string"
        ? value.dataBase64
        : typeof value.data_base64 === "string"
          ? value.data_base64
          : null,
  };
}

type WorkspaceGitStatusPayload = {
  workspace_dir?: string;
  workspaceDir?: string;
  is_git_repo?: boolean;
  isGitRepo?: boolean;
  repo_root?: string | null;
  repoRoot?: string | null;
  current_branch?: string | null;
  currentBranch?: string | null;
  is_dirty?: boolean;
  isDirty?: boolean;
};

type WorkspacePayload = {
  name?: string | null;
  path?: string | null;
  workspace_dir?: string | null;
  workspaceDir?: string | null;
};

function workspaceNameFromPathPayload(path: string): string {
  const normalized = path.trim().replace(/[\\/]+$/, "");
  if (!normalized) {
    return "Workspace";
  }
  const segments = normalized.split(/[\\/]/).filter(Boolean);
  return segments[segments.length - 1] || normalized;
}

function mapWorkspace(value: WorkspacePayload): DesktopWorkspace | null {
  const path = (
    value.path ||
    value.workspaceDir ||
    value.workspace_dir ||
    ""
  ).trim();
  if (!path) {
    return null;
  }
  const name = (value.name || "").trim() || workspaceNameFromPathPayload(path);
  const now = new Date().toISOString();
  return {
    name,
    path,
    kind: "local",
    createdAt: now,
    updatedAt: now,
    available: true,
  };
}

function mapWorkspaces(payload: { workspaces?: WorkspacePayload[] | null }): DesktopWorkspace[] {
  return Array.isArray(payload.workspaces)
    ? payload.workspaces
        .map(mapWorkspace)
        .filter((workspace): workspace is DesktopWorkspace => Boolean(workspace))
    : [];
}

export async function fetchWorkspaces(
  settings: DesktopSettings,
): Promise<DesktopWorkspace[]> {
  const payload = await requestJson<{ workspaces?: WorkspacePayload[] }>(
    settings,
    "/api/workspaces",
    { signal: AbortSignal.timeout(REMOTE_STATE_FETCH_TIMEOUT_MS) },
  );
  return mapWorkspaces(payload);
}

export async function addRemoteWorkspace(
  settings: DesktopSettings,
  input: { path: string; name?: string | null },
): Promise<DesktopWorkspace[]> {
  const payload = await requestJson<{ workspaces?: WorkspacePayload[] }>(
    settings,
    "/api/workspaces",
    {
      method: "POST",
      signal: AbortSignal.timeout(8000),
      body: JSON.stringify({
        path: input.path,
        name: input.name || undefined,
      }),
    },
  );
  return mapWorkspaces(payload);
}

export async function deleteRemoteWorkspace(
  settings: DesktopSettings,
  input: { path: string },
): Promise<DesktopWorkspace[]> {
  const query = new URLSearchParams({
    path: input.path,
  });
  const payload = await requestJson<{ workspaces?: WorkspacePayload[] }>(
    settings,
    `/api/workspaces?${query.toString()}`,
    {
      method: "DELETE",
      signal: AbortSignal.timeout(8000),
    },
  );
  return mapWorkspaces(payload);
}

export async function getWorkspaceGitStatus(
  settings: DesktopSettings,
  input: { workspacePath: string },
) {
  const query = new URLSearchParams({
    workspace_dir: input.workspacePath,
  });
  const payload = await requestJson<WorkspaceGitStatusPayload>(
    settings,
    `/api/workspaces/git-status?${query.toString()}`,
    {
      signal: AbortSignal.timeout(8000),
    },
  );
  return {
    workspaceDir: payload.workspaceDir || payload.workspace_dir || input.workspacePath,
    isGitRepo: Boolean(payload.isGitRepo ?? payload.is_git_repo),
    repoRoot: payload.repoRoot ?? payload.repo_root ?? null,
    currentBranch: payload.currentBranch ?? payload.current_branch ?? null,
    isDirty: Boolean(payload.isDirty ?? payload.is_dirty),
  };
}

export async function listWorkspaceDirectories(
  settings: DesktopSettings,
  input?: { path?: string | null },
): Promise<DesktopLocalDirectoryListing> {
  const query = new URLSearchParams();
  if (input?.path?.trim()) {
    query.set("path", input.path.trim());
  }
  const suffix = query.toString() ? `?${query.toString()}` : "";
  const payload = await requestJson<{
    path?: string;
    parentPath?: string | null;
    parent_path?: string | null;
    entries?: Array<{ name?: string | null; path?: string | null }> | null;
  }>(
    settings,
    `/api/workspaces/directories${suffix}`,
    {
      signal: AbortSignal.timeout(8000),
    },
  );
  return {
    path: payload.path || "",
    parentPath: payload.parentPath ?? payload.parent_path ?? null,
    entries: Array.isArray(payload.entries)
      ? payload.entries
          .map((entry) => ({
            name: entry.name?.trim() || entry.path?.trim() || "",
            path: entry.path?.trim() || "",
          }))
          .filter((entry) => Boolean(entry.path))
      : [],
  };
}

export async function listWorkspaceFiles(
  settings: DesktopSettings,
  input: ListWorkspaceFilesInput,
): Promise<DesktopWorkspaceFileListing> {
  const query = new URLSearchParams({
    workspaceDir: input.workspacePath,
  });
  if (input.directoryPath?.trim()) {
    query.set("path", input.directoryPath.trim());
  }
  const payload = await requestJson<WorkspaceFileListingPayload>(
    settings,
    `/api/workspace-files?${query.toString()}`,
    {
      signal: AbortSignal.timeout(10000),
    },
  );
  return mapWorkspaceFileListing(payload);
}

export async function previewWorkspaceFile(
  settings: DesktopSettings,
  input: PreviewWorkspaceFileInput,
): Promise<DesktopWorkspaceFilePreview> {
  const query = new URLSearchParams({
    workspaceDir: input.workspacePath,
    path: input.filePath,
  });
  const payload = await requestJson<WorkspaceFilePreviewPayload>(
    settings,
    `/api/workspace-files/preview?${query.toString()}`,
    {
      signal: AbortSignal.timeout(15000),
    },
  );
  return mapWorkspaceFilePreview(payload);
}

export async function uploadChatAttachments(
  settings: DesktopSettings,
  input: UploadChatAttachmentsInput,
): Promise<UploadChatAttachmentsResult> {
  const payload = await requestJson<UploadChatAttachmentsPayload>(
    settings,
    "/api/chat/attachments/upload",
    {
      method: "POST",
      signal: AbortSignal.timeout(30000),
      body: JSON.stringify({
        files: input.files.map((file) => ({
          kind: file.kind,
          name: file.name,
          mediaType: file.mediaType || undefined,
          dataBase64: file.dataBase64,
        })),
      }),
    },
  );

  return {
    files: Array.isArray(payload.files)
      ? payload.files
          .map((file) => {
            const path =
              (typeof file.path === "string" && file.path) || "";
            const name =
              (typeof file.name === "string" && file.name) || "";
            const mediaType =
              (typeof file.mediaType === "string" && file.mediaType) ||
              (typeof file.media_type === "string" ? file.media_type : "") ||
              "";
            if (!path || !name) {
              return null;
            }
            return {
              kind: file.kind === "image" ? "image" : "file",
              path,
              name,
              mediaType,
            };
          })
          .filter(
            (
              file,
            ): file is UploadChatAttachmentsResult["files"][number] =>
              Boolean(file),
          )
      : [],
  };
}

export async function uploadWorkspaceFiles(
  settings: DesktopSettings,
  input: UploadWorkspaceFilesInput,
): Promise<UploadWorkspaceFilesResult> {
  const payload = await requestJson<UploadWorkspaceFilesPayload>(
    settings,
    "/api/workspace-files/upload",
    {
      method: "POST",
      signal: AbortSignal.timeout(20000),
      body: JSON.stringify({
        workspaceDir: input.workspacePath,
        path: input.directoryPath || undefined,
        files: input.files.map((file) => ({
          name: file.name,
          mediaType: file.mediaType || undefined,
          dataBase64: file.dataBase64,
        })),
      }),
    },
  );

  return {
    workspacePath:
      (typeof payload.workspaceDir === "string" && payload.workspaceDir) ||
      (typeof payload.workspace_dir === "string"
        ? payload.workspace_dir
        : "") ||
      input.workspacePath,
    directoryPath:
      (typeof payload.directoryPath === "string" && payload.directoryPath) ||
      (typeof payload.directory_path === "string"
        ? payload.directory_path
        : "") ||
      "",
    uploadedPaths: Array.isArray(payload.uploadedPaths)
      ? payload.uploadedPaths
      : Array.isArray(payload.uploaded_paths)
        ? payload.uploaded_paths
        : [],
  };
}
