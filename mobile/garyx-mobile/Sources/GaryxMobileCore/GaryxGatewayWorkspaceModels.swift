import Foundation

public struct GaryxWorkspaceSummary: Decodable, Identifiable, Equatable, Sendable {
    public var id: String { path }
    public var name: String
    public var path: String

    enum CodingKeys: String, CodingKey {
        case name
        case path
        case workspaceDir = "workspaceDir"
        case workspaceDirSnake = "workspace_dir"
    }

    public init(name: String, path: String) {
        self.name = name
        self.path = path
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        path = try container.garyxDecodeFirstString(.path, .workspaceDir, .workspaceDirSnake) ?? ""
        name = try container.garyxDecodeFirstString(.name) ?? path.garyxLastPathComponent
    }
}

public struct GaryxWorkspacesPage: Decodable, Equatable, Sendable {
    public var workspaces: [GaryxWorkspaceSummary]

    enum CodingKeys: String, CodingKey {
        case workspaces
    }

    public init(workspaces: [GaryxWorkspaceSummary]) {
        self.workspaces = workspaces
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        workspaces = try container.decodeIfPresent([GaryxWorkspaceSummary].self, forKey: .workspaces) ?? []
    }
}

public struct GaryxWorkspaceUpsertRequest: Encodable, Equatable, Sendable {
    public var path: String
    public var name: String?

    public init(path: String, name: String? = nil) {
        self.path = path
        self.name = name
    }
}

public struct GaryxWorkspaceGitStatus: Decodable, Equatable, Sendable {
    public var workspaceDir: String
    public var isGitRepo: Bool
    public var repoRoot: String?
    public var currentBranch: String?
    public var isDirty: Bool

    public var canUseWorktree: Bool {
        isGitRepo
    }

    public init(
        workspaceDir: String,
        isGitRepo: Bool,
        repoRoot: String? = nil,
        currentBranch: String? = nil,
        isDirty: Bool = false
    ) {
        self.workspaceDir = workspaceDir
        self.isGitRepo = isGitRepo
        self.repoRoot = repoRoot
        self.currentBranch = currentBranch
        self.isDirty = isDirty
    }

    enum CodingKeys: String, CodingKey {
        case workspaceDir
        case workspaceDirSnake = "workspace_dir"
        case isGitRepo
        case isGitRepoSnake = "is_git_repo"
        case repoRoot
        case repoRootSnake = "repo_root"
        case currentBranch
        case currentBranchSnake = "current_branch"
        case isDirty
        case isDirtySnake = "is_dirty"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        workspaceDir = try container.garyxDecodeFirstString(.workspaceDir, .workspaceDirSnake) ?? ""
        isGitRepo = try container.garyxDecodeFirstBool(.isGitRepo, .isGitRepoSnake) ?? false
        repoRoot = try container.garyxDecodeFirstString(.repoRoot, .repoRootSnake)
        currentBranch = try container.garyxDecodeFirstString(.currentBranch, .currentBranchSnake)
        isDirty = try container.garyxDecodeFirstBool(.isDirty, .isDirtySnake) ?? false
    }
}

/// Resolves the workspace mode for a new thread: `worktree` only when the
/// user chose a workspace, prefers worktree mode, and that workspace's git
/// status allows worktrees; otherwise `local`.
public enum GaryxNewThreadWorkspaceModePolicy {
    public static func workspaceMode(
        workspace: String,
        preferredMode: String?,
        gitStatuses: [String: GaryxWorkspaceGitStatus]
    ) -> String {
        let trimmedWorkspace = workspace.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedWorkspace.isEmpty else { return "local" }
        guard normalizedWorkspaceMode(preferredMode) == "worktree" else { return "local" }
        guard gitStatuses[trimmedWorkspace]?.canUseWorktree == true else { return "local" }
        return "worktree"
    }

    public static func normalizedWorkspaceMode(_ value: String?) -> String {
        let normalized = value?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() ?? ""
        return normalized == "worktree" ? "worktree" : "local"
    }
}

public struct GaryxWorkspaceDirectoryEntry: Decodable, Identifiable, Equatable, Sendable {
    public var id: String { path }
    public var name: String
    public var path: String

    enum CodingKeys: String, CodingKey {
        case name
        case path
    }
}

public struct GaryxWorkspaceDirectoryListing: Decodable, Equatable, Sendable {
    public var path: String
    public var parentPath: String?
    public var entries: [GaryxWorkspaceDirectoryEntry]

    enum CodingKeys: String, CodingKey {
        case path
        case parentPath
        case parentPathSnake = "parent_path"
        case entries
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        path = try container.garyxDecodeFirstString(.path) ?? ""
        parentPath = try container.garyxDecodeFirstString(.parentPath, .parentPathSnake)
        entries = try container.decodeIfPresent([GaryxWorkspaceDirectoryEntry].self, forKey: .entries) ?? []
    }
}


public struct GaryxWorkspaceFileEntry: Decodable, Identifiable, Equatable, Sendable {
    public var id: String { path }
    public var path: String
    public var name: String
    public var entryType: String
    public var size: Int?
    public var modifiedAt: String?
    public var mediaType: String?
    public var hasChildren: Bool

    enum CodingKeys: String, CodingKey {
        case path
        case name
        case entryType
        case entryTypeSnake = "entry_type"
        case size
        case modifiedAt
        case modifiedAtSnake = "modified_at"
        case mediaType
        case mediaTypeSnake = "media_type"
        case hasChildren
        case hasChildrenSnake = "has_children"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        path = try container.garyxDecodeFirstString(.path) ?? ""
        name = try container.garyxDecodeFirstString(.name) ?? path.garyxLastPathComponent
        entryType = try container.garyxDecodeFirstString(.entryType, .entryTypeSnake) ?? "file"
        size = try container.garyxDecodeFirstInt(.size)
        modifiedAt = try container.garyxDecodeFirstString(.modifiedAt, .modifiedAtSnake)
        mediaType = try container.garyxDecodeFirstString(.mediaType, .mediaTypeSnake)
        hasChildren = try container.garyxDecodeFirstBool(.hasChildren, .hasChildrenSnake) ?? false
    }
}


public struct GaryxWorkspaceFileListing: Decodable, Equatable, Sendable {
    public var workspaceDir: String
    public var directoryPath: String
    public var entries: [GaryxWorkspaceFileEntry]

    enum CodingKeys: String, CodingKey {
        case workspaceDir
        case workspaceDirSnake = "workspace_dir"
        case directoryPath
        case directoryPathSnake = "directory_path"
        case entries
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        workspaceDir = try container.garyxDecodeFirstString(.workspaceDir, .workspaceDirSnake) ?? ""
        directoryPath = try container.garyxDecodeFirstString(.directoryPath, .directoryPathSnake) ?? ""
        entries = try container.decodeIfPresent([GaryxWorkspaceFileEntry].self, forKey: .entries) ?? []
    }
}


public struct GaryxWorkspaceFilePreview: Decodable, Equatable, Sendable {
    public var workspaceDir: String
    public var path: String
    public var name: String
    public var mediaType: String
    public var previewKind: String
    public var size: Int
    public var modifiedAt: String?
    public var truncated: Bool
    public var text: String?
    public var dataBase64: String?

    enum CodingKeys: String, CodingKey {
        case workspaceDir
        case workspaceDirSnake = "workspace_dir"
        case path
        case name
        case mediaType
        case mediaTypeSnake = "media_type"
        case previewKind
        case previewKindSnake = "preview_kind"
        case size
        case modifiedAt
        case modifiedAtSnake = "modified_at"
        case truncated
        case text
        case dataBase64
        case dataBase64Snake = "data_base64"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        workspaceDir = try container.garyxDecodeFirstString(.workspaceDir, .workspaceDirSnake) ?? ""
        path = try container.garyxDecodeFirstString(.path) ?? ""
        name = try container.garyxDecodeFirstString(.name) ?? path.garyxLastPathComponent
        mediaType = try container.garyxDecodeFirstString(.mediaType, .mediaTypeSnake) ?? "application/octet-stream"
        previewKind = try container.garyxDecodeFirstString(.previewKind, .previewKindSnake) ?? "unsupported"
        size = try container.garyxDecodeFirstInt(.size) ?? 0
        modifiedAt = try container.garyxDecodeFirstString(.modifiedAt, .modifiedAtSnake)
        truncated = try container.garyxDecodeFirstBool(.truncated) ?? false
        text = try container.garyxDecodeFirstString(.text)
        dataBase64 = try container.garyxDecodeFirstString(.dataBase64, .dataBase64Snake)
    }
}


public struct GaryxUploadFileBlob: Encodable, Equatable, Sendable {
    public var name: String
    public var mediaType: String?
    public var dataBase64: String

    public init(name: String, mediaType: String? = nil, dataBase64: String) {
        self.name = name
        self.mediaType = mediaType
        self.dataBase64 = dataBase64
    }
}


public struct GaryxUploadChatAttachmentBlob: Encodable, Equatable, Sendable {
    public var kind: String
    public var name: String
    public var mediaType: String?
    public var dataBase64: String

    public init(kind: String, name: String, mediaType: String? = nil, dataBase64: String) {
        self.kind = kind
        self.name = name
        self.mediaType = mediaType
        self.dataBase64 = dataBase64
    }
}


public struct GaryxUploadWorkspaceFilesRequest: Encodable, Equatable, Sendable {
    public var workspaceDir: String
    public var path: String?
    public var files: [GaryxUploadFileBlob]

    public init(workspaceDir: String, path: String? = nil, files: [GaryxUploadFileBlob]) {
        self.workspaceDir = workspaceDir
        self.path = path
        self.files = files
    }
}


public struct GaryxUploadWorkspaceFilesResult: Decodable, Equatable, Sendable {
    public var workspaceDir: String
    public var directoryPath: String
    public var uploadedPaths: [String]

    enum CodingKeys: String, CodingKey {
        case workspaceDir
        case workspaceDirSnake = "workspace_dir"
        case directoryPath
        case directoryPathSnake = "directory_path"
        case uploadedPaths
        case uploadedPathsSnake = "uploaded_paths"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        workspaceDir = try container.garyxDecodeFirstString(.workspaceDir, .workspaceDirSnake) ?? ""
        directoryPath = try container.garyxDecodeFirstString(.directoryPath, .directoryPathSnake) ?? ""
        uploadedPaths = try container.decodeIfPresent([String].self, forKey: .uploadedPaths)
            ?? container.decodeIfPresent([String].self, forKey: .uploadedPathsSnake)
            ?? []
    }
}


public struct GaryxUploadChatAttachmentsRequest: Encodable, Equatable, Sendable {
    public var files: [GaryxUploadChatAttachmentBlob]

    public init(files: [GaryxUploadChatAttachmentBlob]) {
        self.files = files
    }
}


public struct GaryxUploadedChatAttachment: Decodable, Equatable, Sendable {
    public var kind: String
    public var path: String
    public var name: String
    public var mediaType: String

    enum CodingKeys: String, CodingKey {
        case kind
        case path
        case name
        case mediaType
        case mediaTypeSnake = "media_type"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        kind = try container.garyxDecodeFirstString(.kind) ?? "file"
        path = try container.garyxDecodeFirstString(.path) ?? ""
        name = try container.garyxDecodeFirstString(.name) ?? path.garyxLastPathComponent
        mediaType = try container.garyxDecodeFirstString(.mediaType, .mediaTypeSnake) ?? ""
    }
}


public struct GaryxUploadChatAttachmentsResult: Decodable, Equatable, Sendable {
    public var files: [GaryxUploadedChatAttachment]
}


public struct GaryxPromptAttachment: Encodable, Equatable, Sendable {
    public var kind: String
    public var path: String
    public var name: String
    public var mediaType: String

    public init(kind: String, path: String, name: String, mediaType: String) {
        self.kind = kind
        self.path = path
        self.name = name
        self.mediaType = mediaType
    }

    enum CodingKeys: String, CodingKey {
        case kind
        case path
        case name
        case mediaType = "media_type"
    }
}


public struct GaryxInlineImagePayload: Encodable, Equatable, Sendable {
    public var name: String
    public var data: String
    public var mediaType: String

    public init(name: String, data: String, mediaType: String) {
        self.name = name
        self.data = data
        self.mediaType = mediaType
    }

    enum CodingKeys: String, CodingKey {
        case name
        case data
        case mediaType = "media_type"
    }
}


public struct GaryxInlineFilePayload: Encodable, Equatable, Sendable {
    public var name: String
    public var data: String
    public var mediaType: String

    public init(name: String, data: String, mediaType: String) {
        self.name = name
        self.data = data
        self.mediaType = mediaType
    }

    enum CodingKeys: String, CodingKey {
        case name
        case data
        case mediaType = "media_type"
    }
}
