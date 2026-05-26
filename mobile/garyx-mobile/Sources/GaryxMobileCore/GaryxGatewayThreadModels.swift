import Foundation

public struct GaryxThreadsPage: Decodable, Equatable, Sendable {
    public var threads: [GaryxThreadSummary]
    public var count: Int
    public var total: Int
    public var limit: Int
    public var offset: Int
}


public struct GaryxRecentThreadsPage: Decodable, Equatable, Sendable {
    public var threads: [GaryxThreadSummary]
    public var count: Int
    public var limit: Int
    public var offset: Int
    public var total: Int
    public var hasMore: Bool

    enum CodingKeys: String, CodingKey {
        case threads
        case count
        case limit
        case offset
        case total
        case hasMore = "has_more"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        threads = try container.decodeIfPresent([GaryxThreadSummary].self, forKey: .threads) ?? []
        count = try container.decodeIfPresent(Int.self, forKey: .count) ?? threads.count
        limit = try container.decodeIfPresent(Int.self, forKey: .limit) ?? count
        offset = try container.decodeIfPresent(Int.self, forKey: .offset) ?? 0
        total = try container.decodeIfPresent(Int.self, forKey: .total) ?? offset + count
        hasMore = try container.decodeIfPresent(Bool.self, forKey: .hasMore) ?? (offset + count < total)
    }
}


public struct GaryxThreadPinsPage: Decodable, Equatable, Sendable {
    public var threadIds: [String]

    enum CodingKeys: String, CodingKey {
        case threadIds
        case threadIdsSnake = "thread_ids"
        case pins
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let rawIds = try container.decodeIfPresent([String].self, forKey: .threadIdsSnake)
            ?? container.decodeIfPresent([String].self, forKey: .threadIds)
            ?? container.decodeIfPresent([GaryxThreadPinRecord].self, forKey: .pins)?.map(\.threadId)
            ?? []
        threadIds = Self.normalizedThreadIds(rawIds)
    }

    private static func normalizedThreadIds(_ values: [String]) -> [String] {
        var seen = Set<String>()
        var ids: [String] = []
        for value in values {
            let id = value.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !id.isEmpty, seen.insert(id).inserted else { continue }
            ids.append(id)
        }
        return ids
    }
}


private struct GaryxThreadPinRecord: Decodable, Equatable, Sendable {
    var threadId: String

    enum CodingKeys: String, CodingKey {
        case threadId
        case threadIdSnake = "thread_id"
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        threadId = try container.garyxDecodeFirstString(.threadIdSnake, .threadId) ?? ""
    }
}


public struct GaryxThreadSummary: Decodable, Identifiable, Equatable, Sendable {
    public var id: String
    public var title: String
    public var createdAt: String?
    public var updatedAt: String?
    public var lastMessagePreview: String
    public var workspacePath: String?
    public var messageCount: Int?
    public var agentId: String?
    public var teamId: String?
    public var teamName: String?
    public var providerType: String?
    public var recentRunId: String?
    public var activeRunId: String?
    public var runState: String?
    public var worktreePath: String?

    public init(
        id: String,
        title: String,
        createdAt: String?,
        updatedAt: String?,
        lastMessagePreview: String,
        workspacePath: String?,
        messageCount: Int?,
        agentId: String?,
        teamId: String?,
        teamName: String?,
        providerType: String?,
        recentRunId: String?,
        activeRunId: String?,
        runState: String?,
        worktreePath: String?
    ) {
        self.id = id
        self.title = title
        self.createdAt = createdAt
        self.updatedAt = updatedAt
        self.lastMessagePreview = lastMessagePreview
        self.workspacePath = workspacePath
        self.messageCount = messageCount
        self.agentId = agentId
        self.teamId = teamId
        self.teamName = teamName
        self.providerType = providerType
        self.recentRunId = recentRunId
        self.activeRunId = activeRunId
        self.runState = runState
        self.worktreePath = worktreePath
    }

    enum CodingKeys: String, CodingKey {
        case id
        case threadId = "thread_id"
        case threadKey = "thread_key"
        case title
        case label
        case createdAt = "created_at"
        case updatedAt = "updated_at"
        case lastActiveAt = "last_active_at"
        case lastMessagePreview
        case lastMessagePreviewSnake = "last_message_preview"
        case lastUserMessage = "last_user_message"
        case lastAssistantMessage = "last_assistant_message"
        case workspacePath
        case workspaceDir = "workspace_dir"
        case messageCount = "message_count"
        case agentId = "agent_id"
        case teamId = "team_id"
        case teamDisplayName = "team_display_name"
        case teamDisplayNameCamel = "teamDisplayName"
        case providerType = "provider_type"
        case recentRunId = "recent_run_id"
        case activeRunId = "active_run_id"
        case runState = "run_state"
        case worktree
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let resolvedId = try container.garyxDecodeFirstString(.id, .threadId, .threadKey)
        id = resolvedId ?? ""
        title = try container.garyxDecodeFirstString(.title, .label) ?? "New Thread"
        createdAt = try container.garyxDecodeFirstString(.createdAt)
        updatedAt = try container.garyxDecodeFirstString(.updatedAt, .lastActiveAt)
        lastMessagePreview = try container.garyxDecodeFirstString(
            .lastMessagePreview,
            .lastMessagePreviewSnake,
            .lastUserMessage,
            .lastAssistantMessage
        ) ?? ""
        workspacePath = try container.garyxDecodeFirstString(.workspacePath, .workspaceDir)
        messageCount = try container.decodeIfPresent(Int.self, forKey: .messageCount)
        agentId = try container.garyxDecodeFirstString(.agentId)
        teamId = try container.garyxDecodeFirstString(.teamId)
        teamName = try container.garyxDecodeFirstString(.teamDisplayName, .teamDisplayNameCamel)
        providerType = try container.garyxDecodeFirstString(.providerType)
        recentRunId = try container.garyxDecodeFirstString(.recentRunId)
        activeRunId = try container.garyxDecodeFirstString(.activeRunId)
        runState = try container.garyxDecodeFirstString(.runState)
        worktreePath = try container
            .decodeIfPresent(GaryxThreadWorktreeSummary.self, forKey: .worktree)?
            .visiblePath
    }
}


private struct GaryxThreadWorktreeSummary: Decodable, Equatable, Sendable {
    var path: String?
    var worktreeDir: String?

    var visiblePath: String? {
        let value = worktreeDir ?? path
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.isEmpty ? nil : trimmed
    }

    enum CodingKeys: String, CodingKey {
        case path
        case worktreeDir = "worktree_dir"
        case worktreeDirCamel = "worktreeDir"
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        path = try container.garyxDecodeFirstString(.path)
        worktreeDir = try container.garyxDecodeFirstString(.worktreeDir, .worktreeDirCamel)
    }
}


public enum GaryxTranscriptRole: String, Codable, Equatable, Sendable {
    case assistant
    case system
    case user
    case toolUse = "tool_use"
    case toolResult = "tool_result"
    case unknown
}


public struct GaryxThreadTranscript: Decodable, Equatable, Sendable {
    public var ok: Bool
    public var messages: [GaryxTranscriptMessage]
    public var pendingUserInputs: [GaryxPendingUserInput]
    public var threadRuntime: GaryxThreadRuntimeSummary?
    public var pageInfo: GaryxThreadTranscriptPageInfo?

    enum CodingKeys: String, CodingKey {
        case ok
        case messages
        case pendingUserInputs = "pending_user_inputs"
        case threadRuntime = "thread_runtime"
        case pageInfo = "message_stats"
    }
}


public struct GaryxThreadTranscriptPageInfo: Decodable, Equatable, Sendable {
    public var returnedMessages: Int
    public var returnedStartIndex: Int?
    public var returnedEndIndex: Int?
    public var hasMoreBefore: Bool
    public var nextBeforeIndex: Int?

    enum CodingKeys: String, CodingKey {
        case returnedMessages = "returned_messages"
        case returnedStartIndex = "returned_start_index"
        case returnedEndIndex = "returned_end_index"
        case hasMoreBefore = "has_more_before"
        case nextBeforeIndex = "next_before_index"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        returnedMessages = try container.decodeIfPresent(Int.self, forKey: .returnedMessages) ?? 0
        returnedStartIndex = try container.decodeIfPresent(Int.self, forKey: .returnedStartIndex)
        returnedEndIndex = try container.decodeIfPresent(Int.self, forKey: .returnedEndIndex)
        hasMoreBefore = try container.decodeIfPresent(Bool.self, forKey: .hasMoreBefore) ?? false
        nextBeforeIndex = try container.decodeIfPresent(Int.self, forKey: .nextBeforeIndex)
    }
}


public struct GaryxThreadRuntimeSummary: Decodable, Equatable, Sendable {
    public var providerType: String?
    public var providerLabel: String?
    public var sdkSessionId: String?
    public var activeRun: GaryxThreadActiveRunSummary?

    enum CodingKeys: String, CodingKey {
        case providerType = "provider_type"
        case providerLabel = "provider_label"
        case sdkSessionId = "sdk_session_id"
        case activeRun = "active_run"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        providerType = try container.garyxDecodeFirstString(.providerType)
        providerLabel = try container.garyxDecodeFirstString(.providerLabel)
        sdkSessionId = try container.garyxDecodeFirstString(.sdkSessionId)
        activeRun = try container.decodeIfPresent(GaryxThreadActiveRunSummary.self, forKey: .activeRun)
    }
}


public struct GaryxThreadActiveRunSummary: Decodable, Equatable, Sendable {
    public var runId: String?
    public var providerType: String?
    public var providerLabel: String?
    public var assistantResponse: String?
    public var updatedAt: String?
    public var pendingUserInputCount: Int

    enum CodingKeys: String, CodingKey {
        case runId = "run_id"
        case providerType = "provider_type"
        case providerLabel = "provider_label"
        case assistantResponse = "assistant_response"
        case updatedAt = "updated_at"
        case pendingUserInputCount = "pending_user_input_count"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        runId = try container.garyxDecodeFirstString(.runId)
        providerType = try container.garyxDecodeFirstString(.providerType)
        providerLabel = try container.garyxDecodeFirstString(.providerLabel)
        assistantResponse = try container.garyxDecodeFirstString(.assistantResponse)
        updatedAt = try container.garyxDecodeFirstString(.updatedAt)
        pendingUserInputCount = try container.decodeIfPresent(Int.self, forKey: .pendingUserInputCount) ?? 0
    }
}


public struct GaryxTranscriptMessage: Decodable, Identifiable, Equatable, Sendable {
    public var id: String
    public var index: Int?
    public var role: GaryxTranscriptRole
    public var kind: String?
    public var text: String
    public var content: GaryxJSONValue?
    public var message: GaryxJSONValue?
    public var timestamp: String?
    public var toolRelated: Bool
    public var likelyUserVisible: Bool

    enum CodingKeys: String, CodingKey {
        case index
        case role
        case kind
        case text
        case content
        case message
        case timestamp
        case toolRelated = "tool_related"
        case likelyUserVisible = "likely_user_visible"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        index = try container.decodeIfPresent(Int.self, forKey: .index)
        let roleValue = try container.decodeIfPresent(String.self, forKey: .role) ?? ""
        role = GaryxTranscriptRole(rawValue: roleValue) ?? .unknown
        kind = try container.decodeIfPresent(String.self, forKey: .kind)
        text = try container.decodeIfPresent(String.self, forKey: .text) ?? ""
        content = try container.decodeIfPresent(GaryxJSONValue.self, forKey: .content)
        message = try container.decodeIfPresent(GaryxJSONValue.self, forKey: .message)
        timestamp = try container.decodeIfPresent(String.self, forKey: .timestamp)
        toolRelated = try container.decodeIfPresent(Bool.self, forKey: .toolRelated) ?? false
        likelyUserVisible = try container.decodeIfPresent(Bool.self, forKey: .likelyUserVisible) ?? true
        id = index.map { "history:\($0)" } ?? UUID().uuidString
    }
}


public struct GaryxPendingUserInput: Decodable, Identifiable, Equatable, Sendable {
    public var id: String
    public var runId: String?
    public var text: String
    public var content: GaryxJSONValue?
    public var timestamp: String?
    public var status: String?
    public var active: Bool

    enum CodingKeys: String, CodingKey {
        case id
        case runId = "run_id"
        case text
        case content
        case timestamp
        case status
        case active
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.garyxDecodeFirstString(.id) ?? ""
        runId = try container.garyxDecodeFirstString(.runId)
        text = try container.garyxDecodeFirstString(.text) ?? ""
        content = try container.decodeIfPresent(GaryxJSONValue.self, forKey: .content)
        timestamp = try container.garyxDecodeFirstString(.timestamp)
        status = try container.garyxDecodeFirstString(.status)
        active = try container.decodeIfPresent(Bool.self, forKey: .active) ?? true
    }
}


public struct GaryxCreateThreadRequest: Encodable, Equatable, Sendable {
    public var label: String?
    public var workspaceDir: String?
    public var workspaceMode: String?
    public var agentId: String?
    public var metadata: [String: String]

    public init(
        label: String? = nil,
        workspaceDir: String? = nil,
        workspaceMode: String? = nil,
        agentId: String? = nil,
        metadata: [String: String] = [:]
    ) {
        self.label = label
        self.workspaceDir = workspaceDir
        self.workspaceMode = workspaceMode
        self.agentId = agentId
        self.metadata = metadata
    }
}


public struct GaryxUpdateThreadRequest: Encodable, Equatable, Sendable {
    public var label: String?
    public var workspaceDir: String?
}


public struct GaryxThreadLogChunk: Decodable, Equatable, Sendable {
    public var threadId: String
    public var path: String
    public var text: String
    public var cursor: Int
    public var reset: Bool

    enum CodingKeys: String, CodingKey {
        case threadId
        case threadIdSnake = "thread_id"
        case path
        case text
        case cursor
        case reset
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        threadId = try container.garyxDecodeFirstString(.threadId, .threadIdSnake) ?? ""
        path = try container.garyxDecodeFirstString(.path) ?? ""
        text = try container.garyxDecodeFirstString(.text) ?? ""
        cursor = try container.garyxDecodeFirstInt(.cursor) ?? 0
        reset = try container.garyxDecodeFirstBool(.reset) ?? true
    }
}
