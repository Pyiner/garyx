import Foundation

public struct GaryxWorkspaceSummary: Codable, Identifiable, Equatable, Sendable {
    public var id: String { path }
    public var name: String
    public var path: String
    public var pinned: Bool
    public var threadCount: Int
    public var lastActivityAt: String?
    public var gitRepo: Bool

    enum CodingKeys: String, CodingKey {
        case name
        case path
        case workspaceDir = "workspaceDir"
        case workspaceDirSnake = "workspace_dir"
        case pinned
        case threadCount
        case threadCountSnake = "thread_count"
        case lastActivityAt
        case lastActivityAtSnake = "last_activity_at"
        case gitRepo
        case gitRepoSnake = "git_repo"
    }

    public init(
        name: String,
        path: String,
        pinned: Bool = false,
        threadCount: Int = 0,
        lastActivityAt: String? = nil,
        gitRepo: Bool = false
    ) {
        self.name = name
        self.path = path
        self.pinned = pinned
        self.threadCount = threadCount
        self.lastActivityAt = lastActivityAt
        self.gitRepo = gitRepo
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        path = try container.garyxDecodeFirstString(.path, .workspaceDir, .workspaceDirSnake) ?? ""
        name = try container.garyxDecodeFirstString(.name) ?? path.garyxLastPathComponent
        pinned = try container.garyxDecodeFirstBool(.pinned) ?? false
        threadCount = try container.garyxDecodeFirstInt(.threadCount, .threadCountSnake) ?? 0
        lastActivityAt = try container.garyxDecodeFirstString(.lastActivityAt, .lastActivityAtSnake)
        gitRepo = try container.garyxDecodeFirstBool(.gitRepo, .gitRepoSnake) ?? false
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(name, forKey: .name)
        try container.encode(path, forKey: .path)
        try container.encode(pinned, forKey: .pinned)
        try container.encode(threadCount, forKey: .threadCount)
        try container.encodeIfPresent(lastActivityAt, forKey: .lastActivityAt)
        try container.encode(gitRepo, forKey: .gitRepo)
    }
}

public struct GaryxWorkspacesPage: Decodable, Equatable, Sendable {
    public var workspaces: [GaryxWorkspaceSummary]
    public var gatewayHome: String?
    public var workspaceStateInitialized: Bool

    enum CodingKeys: String, CodingKey {
        case workspaces
        case gatewayHome
        case gatewayHomeSnake = "gateway_home"
        case workspaceStateInitialized
        case workspaceStateInitializedSnake = "workspace_state_initialized"
    }

    public init(
        workspaces: [GaryxWorkspaceSummary],
        gatewayHome: String? = nil,
        workspaceStateInitialized: Bool = true
    ) {
        self.workspaces = workspaces
        self.gatewayHome = gatewayHome
        self.workspaceStateInitialized = workspaceStateInitialized
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        workspaces = try container.decodeIfPresent([GaryxWorkspaceSummary].self, forKey: .workspaces) ?? []
        gatewayHome = try container.garyxDecodeFirstString(.gatewayHome, .gatewayHomeSnake)
        workspaceStateInitialized = try container.garyxDecodeFirstBool(
            .workspaceStateInitialized, .workspaceStateInitializedSnake
        ) ?? true
    }
}

/// The gateway workspace universe as delivered: server-ordered summaries plus
/// the gateway machine's home directory for `~` abbreviation. Clients render
/// this verbatim — no re-sorting, no renaming, no path filtering.
public struct GaryxWorkspaceCatalog: Codable, Equatable, Sendable {
    public var gatewayHome: String?
    public var workspaces: [GaryxWorkspaceSummary]

    public static let empty = GaryxWorkspaceCatalog(gatewayHome: nil, workspaces: [])

    public init(gatewayHome: String?, workspaces: [GaryxWorkspaceSummary]) {
        self.gatewayHome = gatewayHome
        self.workspaces = workspaces
    }

    public init(page: GaryxWorkspacesPage) {
        self.init(gatewayHome: page.gatewayHome, workspaces: page.workspaces)
    }

    public var paths: [String] { workspaces.map(\.path) }

    public func summary(forPath path: String) -> GaryxWorkspaceSummary? {
        workspaces.first { $0.path == path }
    }
}

public struct GaryxWorkspacePinRequest: Encodable, Equatable, Sendable {
    public var path: String
    public var pinned: Bool

    public init(path: String, pinned: Bool) {
        self.path = path
        self.pinned = pinned
    }
}

public struct GaryxWorkspaceRenameRequest: Encodable, Equatable, Sendable {
    public var path: String
    public var name: String

    public init(path: String, name: String) {
        self.path = path
        self.name = name
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
    public var gitRepo: Bool

    enum CodingKeys: String, CodingKey {
        case name
        case path
        case gitRepo
        case gitRepoSnake = "git_repo"
    }

    public init(name: String, path: String, gitRepo: Bool = false) {
        self.name = name
        self.path = path
        self.gitRepo = gitRepo
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        name = try container.garyxDecodeFirstString(.name) ?? ""
        path = try container.garyxDecodeFirstString(.path) ?? ""
        gitRepo = try container.garyxDecodeFirstBool(.gitRepo, .gitRepoSnake) ?? false
    }
}

/// The typed `/api/workspaces/directories` 400 contract. The browser renders
/// these inline and stays where it was; anything else is a transport failure.
public enum GaryxWorkspaceDirectoryErrorCode: String, Equatable, Sendable {
    case invalidPath = "invalid_path"
    case notFound = "not_found"
    case notADirectory = "not_a_directory"
    case permissionDenied = "permission_denied"

    public var userMessage: String {
        switch self {
        case .invalidPath: return "Enter an absolute path."
        case .notFound: return "That directory does not exist on the gateway."
        case .notADirectory: return "That path is not a directory."
        case .permissionDenied: return "The gateway cannot read that directory."
        }
    }
}

public struct GaryxWorkspaceDirectoryError: Error, Equatable, Sendable {
    public var code: GaryxWorkspaceDirectoryErrorCode
    public var message: String

    public init(code: GaryxWorkspaceDirectoryErrorCode, message: String) {
        self.code = code
        self.message = message
    }

    /// Decodes the typed 400 body `{"error": <message>, "code": <code>}`.
    /// Returns nil for anything that is not a recognized typed failure.
    public static func decode(statusCode: Int, body: Data) -> GaryxWorkspaceDirectoryError? {
        guard statusCode == 400 else { return nil }
        guard
            let payload = try? JSONSerialization.jsonObject(with: body) as? [String: Any],
            let rawCode = payload["code"] as? String,
            let code = GaryxWorkspaceDirectoryErrorCode(rawValue: rawCode)
        else { return nil }
        let message = payload["error"] as? String ?? code.userMessage
        return GaryxWorkspaceDirectoryError(code: code, message: message)
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

    public init(path: String, parentPath: String? = nil, entries: [GaryxWorkspaceDirectoryEntry]) {
        self.path = path
        self.parentPath = parentPath
        self.entries = entries
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
