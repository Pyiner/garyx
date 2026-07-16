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
import {
  GatewayContractError,
  REMOTE_STATE_FETCH_TIMEOUT_MS,
  requestJson,
  requireContractArray,
  requireContractBoolean,
  requireContractField,
  requireContractNonEmptyString,
  requireContractNonNegativeInteger,
  requireContractRecord,
  requireContractString,
} from "./http.ts";

interface WorkspaceFileEntryPayload {
  path?: string | null;
  name?: string | null;
  entryType?: string | null;
  size?: number | null;
  modifiedAt?: string | null;
  mediaType?: string | null;
  hasChildren?: boolean;
}

interface WorkspaceFileListingPayload {
  workspaceDir?: string | null;
  directoryPath?: string | null;
  entries?: WorkspaceFileEntryPayload[] | null;
}

interface WorkspaceFilePreviewPayload {
  workspaceDir?: string | null;
  path?: string | null;
  name?: string | null;
  mediaType?: string | null;
  previewKind?: string | null;
  size?: number | null;
  modifiedAt?: string | null;
  truncated?: boolean;
  text?: string | null;
  dataBase64?: string | null;
}

interface UploadWorkspaceFilesPayload {
  workspaceDir?: string | null;
  directoryPath?: string | null;
  uploadedPaths?: string[] | null;
}

interface UploadedChatAttachmentPayload {
  kind?: "image" | "file" | null;
  path?: string | null;
  name?: string | null;
  mediaType?: string | null;
}

interface UploadChatAttachmentsPayload {
  files?: UploadedChatAttachmentPayload[] | null;
}

function mapWorkspaceFileEntry(
  value: unknown,
  path: string,
): DesktopWorkspaceFileEntry {
  const record = requireContractRecord(value, path);
  const entryType = requireContractString(
    requireContractField(record, "entryType", path),
    `${path}.entryType`,
  );
  if (entryType !== "directory" && entryType !== "file") {
    throw new GatewayContractError(
      `${path}.entryType`,
      "must be directory or file",
    );
  }
  const nullableString = (field: string): string | null => {
    const fieldValue = requireContractField(record, field, path);
    return fieldValue === null
      ? null
      : requireContractString(fieldValue, `${path}.${field}`);
  };
  const size = requireContractField(record, "size", path);
  return {
    path: requireContractString(
      requireContractField(record, "path", path),
      `${path}.path`,
    ),
    name: requireContractString(
      requireContractField(record, "name", path),
      `${path}.name`,
    ),
    entryType,
    size: size === null
      ? null
      : requireContractNonNegativeInteger(size, `${path}.size`),
    modifiedAt: nullableString("modifiedAt"),
    mediaType: nullableString("mediaType"),
    hasChildren: requireContractBoolean(
      requireContractField(record, "hasChildren", path),
      `${path}.hasChildren`,
    ),
  };
}

function mapWorkspaceFileListing(
  value: unknown,
): DesktopWorkspaceFileListing {
  const path = "workspace file listing";
  const record = requireContractRecord(value, path);
  return {
    workspacePath: requireContractNonEmptyString(
      requireContractField(record, "workspaceDir", path),
      `${path}.workspaceDir`,
    ),
    directoryPath: requireContractString(
      requireContractField(record, "directoryPath", path),
      `${path}.directoryPath`,
    ),
    entries: requireContractArray(
      requireContractField(record, "entries", path),
      `${path}.entries`,
    ).map((entry, index) =>
      mapWorkspaceFileEntry(entry, `${path}.entries[${index}]`),
    ),
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
      throw new GatewayContractError(
        "workspace file preview.previewKind",
        "must be a current preview kind",
      );
  }
}

function mapWorkspaceFilePreview(
  value: unknown,
): DesktopWorkspaceFilePreview {
  const path = "workspace file preview";
  const record = requireContractRecord(value, path);
  const nullableString = (field: string): string | null => {
    const fieldValue = requireContractField(record, field, path);
    return fieldValue === null
      ? null
      : requireContractString(fieldValue, `${path}.${field}`);
  };
  return {
    workspacePath: requireContractNonEmptyString(
      requireContractField(record, "workspaceDir", path),
      `${path}.workspaceDir`,
    ),
    path: requireContractString(
      requireContractField(record, "path", path),
      `${path}.path`,
    ),
    name: requireContractString(
      requireContractField(record, "name", path),
      `${path}.name`,
    ),
    mediaType: requireContractNonEmptyString(
      requireContractField(record, "mediaType", path),
      `${path}.mediaType`,
    ),
    previewKind: normalizeWorkspaceFilePreviewKind(
      requireContractField(record, "previewKind", path),
    ),
    size: requireContractNonNegativeInteger(
      requireContractField(record, "size", path),
      `${path}.size`,
    ),
    modifiedAt: nullableString("modifiedAt"),
    truncated: requireContractBoolean(
      requireContractField(record, "truncated", path),
      `${path}.truncated`,
    ),
    text: nullableString("text"),
    dataBase64: nullableString("dataBase64"),
  };
}

type WorkspaceGitStatusPayload = {
  workspace_dir?: string;
  is_git_repo?: boolean;
  repo_root?: string | null;
  current_branch?: string | null;
  is_dirty?: boolean;
};

type WorkspacePayload = {
  name?: string | null;
  path?: string | null;
};

function mapWorkspace(value: unknown, index: number): DesktopWorkspace {
  const context = `workspace list.workspaces[${index}]`;
  const record = requireContractRecord(value, context);
  const path = requireContractNonEmptyString(
    requireContractField(record, "path", context),
    `${context}.path`,
  );
  const name = requireContractNonEmptyString(
    requireContractField(record, "name", context),
    `${context}.name`,
  );
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

function mapWorkspaces(payload: unknown): DesktopWorkspace[] {
  const record = requireContractRecord(payload, "workspace list");
  requireContractBoolean(
    requireContractField(record, "workspace_state_initialized", "workspace list"),
    "workspace list.workspace_state_initialized",
  );
  return requireContractArray(
    requireContractField(record, "workspaces", "workspace list"),
    "workspace list.workspaces",
  ).map(mapWorkspace);
}

export async function fetchWorkspaces(
  settings: DesktopSettings,
): Promise<DesktopWorkspace[]> {
  const payload = await requestJson<{ workspaces?: WorkspacePayload[] }>(
    settings,
    "/api/workspaces",
    "readRetryable",
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
    "mutationSingleAttempt",
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
    "mutationSingleAttempt",
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
    "readRetryable",
    {
      signal: AbortSignal.timeout(8000),
    },
  );
  const record = requireContractRecord(payload, "workspace git status");
  const nullableString = (field: string): string | null => {
    const value = requireContractField(record, field, "workspace git status");
    return value === null
      ? null
      : requireContractString(value, `workspace git status.${field}`);
  };
  return {
    workspaceDir: requireContractNonEmptyString(
      requireContractField(record, "workspace_dir", "workspace git status"),
      "workspace git status.workspace_dir",
    ),
    isGitRepo: requireContractBoolean(
      requireContractField(record, "is_git_repo", "workspace git status"),
      "workspace git status.is_git_repo",
    ),
    repoRoot: nullableString("repo_root"),
    currentBranch: nullableString("current_branch"),
    isDirty: requireContractBoolean(
      requireContractField(record, "is_dirty", "workspace git status"),
      "workspace git status.is_dirty",
    ),
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
    entries?: Array<{ name?: string | null; path?: string | null }> | null;
  }>(
    settings,
    `/api/workspaces/directories${suffix}`,
    "readRetryable",
    {
      signal: AbortSignal.timeout(8000),
    },
  );
  const record = requireContractRecord(payload, "workspace directory listing");
  const parentPath = requireContractField(
    record,
    "parentPath",
    "workspace directory listing",
  );
  return {
    path: requireContractString(
      requireContractField(record, "path", "workspace directory listing"),
      "workspace directory listing.path",
    ),
    parentPath: parentPath === null
      ? null
      : requireContractString(
          parentPath,
          "workspace directory listing.parentPath",
        ),
    entries: requireContractArray(
      requireContractField(record, "entries", "workspace directory listing"),
      "workspace directory listing.entries",
    ).map((entry, index) => {
      const path = `workspace directory listing.entries[${index}]`;
      const entryRecord = requireContractRecord(entry, path);
      return {
        name: requireContractNonEmptyString(
          requireContractField(entryRecord, "name", path),
          `${path}.name`,
        ),
        path: requireContractNonEmptyString(
          requireContractField(entryRecord, "path", path),
          `${path}.path`,
        ),
      };
    }),
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
    "readRetryable",
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
    "readRetryable",
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
    "mutationSingleAttempt",
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

  const record = requireContractRecord(payload, "chat attachment upload");
  return {
    files: requireContractArray(
      requireContractField(record, "files", "chat attachment upload"),
      "chat attachment upload.files",
    ).map((file, index) => {
      const path = `chat attachment upload.files[${index}]`;
      const fileRecord = requireContractRecord(file, path);
      const kind = requireContractString(
        requireContractField(fileRecord, "kind", path),
        `${path}.kind`,
      );
      if (kind !== "image" && kind !== "file") {
        throw new GatewayContractError(
          `${path}.kind`,
          "must be image or file",
        );
      }
      return {
        kind,
        path: requireContractNonEmptyString(
          requireContractField(fileRecord, "path", path),
          `${path}.path`,
        ),
        name: requireContractNonEmptyString(
          requireContractField(fileRecord, "name", path),
          `${path}.name`,
        ),
        mediaType: requireContractNonEmptyString(
          requireContractField(fileRecord, "mediaType", path),
          `${path}.mediaType`,
        ),
      };
    }),
  };
}

export async function uploadWorkspaceFiles(
  settings: DesktopSettings,
  input: UploadWorkspaceFilesInput,
): Promise<UploadWorkspaceFilesResult> {
  const payload = await requestJson<UploadWorkspaceFilesPayload>(
    settings,
    "/api/workspace-files/upload",
    "mutationSingleAttempt",
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

  const record = requireContractRecord(payload, "workspace file upload");
  return {
    workspacePath: requireContractNonEmptyString(
      requireContractField(record, "workspaceDir", "workspace file upload"),
      "workspace file upload.workspaceDir",
    ),
    directoryPath: requireContractString(
      requireContractField(record, "directoryPath", "workspace file upload"),
      "workspace file upload.directoryPath",
    ),
    uploadedPaths: requireContractArray(
      requireContractField(record, "uploadedPaths", "workspace file upload"),
      "workspace file upload.uploadedPaths",
    ).map((uploadedPath, index) =>
      requireContractNonEmptyString(
        uploadedPath,
        `workspace file upload.uploadedPaths[${index}]`,
      ),
    ),
  };
}
