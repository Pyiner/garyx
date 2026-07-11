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
    public var providerType: String?
    public var recentRunId: String?
    public var activeRunId: String?
    public var runState: String?
    public var worktreePath: String?
    public var automationId: String?
    public var automationThreadMode: String?
    public var excludeFromRecent: Bool
    public var threadRuntime: GaryxThreadRuntimeSummary?

    public init(
        id: String,
        title: String,
        createdAt: String?,
        updatedAt: String?,
        lastMessagePreview: String,
        workspacePath: String?,
        messageCount: Int?,
        agentId: String?,
        providerType: String?,
        recentRunId: String?,
        activeRunId: String?,
        runState: String?,
        worktreePath: String?,
        automationId: String? = nil,
        automationThreadMode: String? = nil,
        excludeFromRecent: Bool = false,
        threadRuntime: GaryxThreadRuntimeSummary? = nil
    ) {
        self.id = id
        self.title = title
        self.createdAt = createdAt
        self.updatedAt = updatedAt
        self.lastMessagePreview = lastMessagePreview
        self.workspacePath = workspacePath
        self.messageCount = messageCount
        self.agentId = agentId
        self.providerType = providerType
        self.recentRunId = recentRunId
        self.activeRunId = activeRunId
        self.runState = runState
        self.worktreePath = worktreePath
        self.automationId = automationId
        self.automationThreadMode = automationThreadMode
        self.excludeFromRecent = excludeFromRecent
        self.threadRuntime = threadRuntime
    }

    enum CodingKeys: String, CodingKey {
        case id
        case threadId = "thread_id"
        case threadIdCamel = "threadId"
        case threadKey = "thread_key"
        case title
        case label
        case createdAt = "created_at"
        case createdAtCamel = "createdAt"
        case updatedAt = "updated_at"
        case updatedAtCamel = "updatedAt"
        case lastActiveAt = "last_active_at"
        case lastActiveAtCamel = "lastActiveAt"
        case lastMessagePreview
        case lastMessagePreviewSnake = "last_message_preview"
        case lastUserMessage = "last_user_message"
        case lastUserMessageCamel = "lastUserMessage"
        case lastAssistantMessage = "last_assistant_message"
        case lastAssistantMessageCamel = "lastAssistantMessage"
        case workspacePath
        case workspaceDir = "workspace_dir"
        case workspaceDirCamel = "workspaceDir"
        case messageCount = "message_count"
        case messageCountCamel = "messageCount"
        case agentId = "agent_id"
        case agentIdCamel = "agentId"
        case providerType = "provider_type"
        case providerTypeCamel = "providerType"
        case recentRunId = "recent_run_id"
        case recentRunIdCamel = "recentRunId"
        case activeRunId = "active_run_id"
        case activeRunIdCamel = "activeRunId"
        case runState = "run_state"
        case runStateCamel = "runState"
        case worktree
        case automationId = "automation_id"
        case automationIdCamel = "automationId"
        case automationThreadMode = "automation_thread_mode"
        case automationThreadModeCamel = "automationThreadMode"
        case excludeFromRecent = "exclude_from_recent"
        case excludeFromRecentCamel = "excludeFromRecent"
        case threadRuntime = "thread_runtime"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let resolvedId = try container.garyxDecodeFirstString(.id, .threadId, .threadIdCamel, .threadKey)
        id = resolvedId ?? ""
        title = try container.garyxDecodeFirstString(.title, .label) ?? "New Thread"
        createdAt = try container.garyxDecodeFirstString(.createdAt, .createdAtCamel)
        updatedAt = try container.garyxDecodeFirstString(.updatedAt, .updatedAtCamel, .lastActiveAt, .lastActiveAtCamel)
        lastMessagePreview = try container.garyxDecodeFirstString(
            .lastMessagePreview,
            .lastMessagePreviewSnake,
            .lastUserMessage,
            .lastUserMessageCamel,
            .lastAssistantMessage,
            .lastAssistantMessageCamel
        ) ?? ""
        workspacePath = try container.garyxDecodeFirstString(.workspacePath, .workspaceDir, .workspaceDirCamel)
        messageCount = try container.decodeIfPresent(Int.self, forKey: .messageCount)
            ?? container.decodeIfPresent(Int.self, forKey: .messageCountCamel)
        agentId = try container.garyxDecodeFirstString(.agentId, .agentIdCamel)
        providerType = try container.garyxDecodeFirstString(.providerType, .providerTypeCamel)
        recentRunId = try container.garyxDecodeFirstString(.recentRunId, .recentRunIdCamel)
        activeRunId = try container.garyxDecodeFirstString(.activeRunId, .activeRunIdCamel)
        runState = try container.garyxDecodeFirstString(.runState, .runStateCamel)
        worktreePath = try container
            .decodeIfPresent(GaryxThreadWorktreeSummary.self, forKey: .worktree)?
            .visiblePath
        automationId = try container.garyxDecodeFirstString(.automationId, .automationIdCamel)
        automationThreadMode = try container.garyxDecodeFirstString(.automationThreadMode, .automationThreadModeCamel)
        excludeFromRecent = try container.decodeIfPresent(Bool.self, forKey: .excludeFromRecentCamel)
            ?? container.decodeIfPresent(Bool.self, forKey: .excludeFromRecent)
            ?? false
        threadRuntime = try container.decodeIfPresent(GaryxThreadRuntimeSummary.self, forKey: .threadRuntime)
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
    case tool
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

    /// Public memberwise init so the app target can synthesize a transcript from a
    /// cache-reconstructed committed window (cache ∪ delta) and feed it through the
    /// existing render/merge path unchanged.
    public init(
        ok: Bool,
        messages: [GaryxTranscriptMessage],
        pendingUserInputs: [GaryxPendingUserInput],
        threadRuntime: GaryxThreadRuntimeSummary?,
        pageInfo: GaryxThreadTranscriptPageInfo?
    ) {
        self.ok = ok
        self.messages = messages
        self.pendingUserInputs = pendingUserInputs
        self.threadRuntime = threadRuntime
        self.pageInfo = pageInfo
    }
}

public extension GaryxThreadTranscript {
    /// Decodes a transcript from a stream snapshot envelope of the form
    /// `{ "payload": { ...transcript... } }`; nil when the envelope carries no
    /// object payload.
    static func fromSnapshotPayload(_ payload: [String: GaryxJSONValue]) throws -> GaryxThreadTranscript? {
        guard case let .object(snapshot)? = payload["payload"] else {
            return nil
        }
        let data = try JSONEncoder().encode(GaryxJSONValue.object(snapshot))
        return try JSONDecoder().decode(GaryxThreadTranscript.self, from: data)
    }
}


public struct GaryxThreadTranscriptPageInfo: Decodable, Equatable, Sendable {
    public var returnedMessages: Int
    public var returnedStartIndex: Int?
    public var returnedEndIndex: Int?
    public var hasMoreBefore: Bool
    public var nextBeforeIndex: Int?
    /// Forward (newer) cursor mirror of `hasMoreBefore`/`nextBeforeIndex`: when a
    /// page was requested with `after_index`, whether committed messages exist
    /// beyond this page and the index to resume from. Drives incremental open.
    public var hasMoreAfter: Bool
    public var nextAfterIndex: Int?
    /// Authoritative total committed message count for the thread, independent of
    /// this page's bounds. Used to detect a server-side shrink/reset (cache cursor
    /// at or beyond this means the cache is ahead of the server). Note an empty
    /// `after_index` page reports `returned_end_index == 0`, so the totals — not
    /// the page bounds — must drive shrink detection.
    public var totalMessagesInThread: Int?
    /// The server returned the bounded newest window because the requested
    /// `after_index` cursor was older than the newest `user_query_limit` user turns;
    /// the client should overwrite its cache with this window rather than merge it.
    public var reset: Bool

    enum CodingKeys: String, CodingKey {
        case returnedMessages = "returned_messages"
        case returnedStartIndex = "returned_start_index"
        case returnedEndIndex = "returned_end_index"
        case hasMoreBefore = "has_more_before"
        case nextBeforeIndex = "next_before_index"
        case hasMoreAfter = "has_more_after"
        case nextAfterIndex = "next_after_index"
        case totalMessagesInThread = "total_messages_in_thread"
        case totalMessagesInSession = "total_messages_in_session"
        case reset
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        returnedMessages = try container.decodeIfPresent(Int.self, forKey: .returnedMessages) ?? 0
        returnedStartIndex = try container.decodeIfPresent(Int.self, forKey: .returnedStartIndex)
        returnedEndIndex = try container.decodeIfPresent(Int.self, forKey: .returnedEndIndex)
        hasMoreBefore = try container.decodeIfPresent(Bool.self, forKey: .hasMoreBefore) ?? false
        nextBeforeIndex = try container.decodeIfPresent(Int.self, forKey: .nextBeforeIndex)
        hasMoreAfter = try container.decodeIfPresent(Bool.self, forKey: .hasMoreAfter) ?? false
        nextAfterIndex = try container.decodeIfPresent(Int.self, forKey: .nextAfterIndex)
        totalMessagesInThread = try container.decodeIfPresent(Int.self, forKey: .totalMessagesInThread)
            ?? container.decodeIfPresent(Int.self, forKey: .totalMessagesInSession)
        reset = try container.decodeIfPresent(Bool.self, forKey: .reset) ?? false
    }

    public init(
        returnedMessages: Int,
        returnedStartIndex: Int?,
        returnedEndIndex: Int?,
        hasMoreBefore: Bool,
        nextBeforeIndex: Int?,
        hasMoreAfter: Bool = false,
        nextAfterIndex: Int? = nil,
        totalMessagesInThread: Int? = nil,
        reset: Bool = false
    ) {
        self.totalMessagesInThread = totalMessagesInThread
        self.returnedMessages = returnedMessages
        self.returnedStartIndex = returnedStartIndex
        self.returnedEndIndex = returnedEndIndex
        self.hasMoreBefore = hasMoreBefore
        self.nextBeforeIndex = nextBeforeIndex
        self.hasMoreAfter = hasMoreAfter
        self.nextAfterIndex = nextAfterIndex
        self.reset = reset
    }
}


public struct GaryxThreadRuntimeSummary: Decodable, Equatable, Sendable {
    public var agentId: String?
    public var providerType: String?
    public var providerLabel: String?
    public var model: String?
    public var modelReasoningEffort: String?
    public var modelServiceTier: String?
    public var modelOverride: String?
    public var modelReasoningEffortOverride: String?
    public var modelServiceTierOverride: String?
    public var sdkSessionId: String?
    public var activeRun: GaryxThreadActiveRunSummary?

    public init(
        agentId: String? = nil,
        providerType: String? = nil,
        providerLabel: String? = nil,
        model: String? = nil,
        modelReasoningEffort: String? = nil,
        modelServiceTier: String? = nil,
        modelOverride: String? = nil,
        modelReasoningEffortOverride: String? = nil,
        modelServiceTierOverride: String? = nil,
        sdkSessionId: String? = nil,
        activeRun: GaryxThreadActiveRunSummary? = nil
    ) {
        self.agentId = agentId
        self.providerType = providerType
        self.providerLabel = providerLabel
        self.model = model
        self.modelReasoningEffort = modelReasoningEffort
        self.modelServiceTier = modelServiceTier
        self.modelOverride = modelOverride
        self.modelReasoningEffortOverride = modelReasoningEffortOverride
        self.modelServiceTierOverride = modelServiceTierOverride
        self.sdkSessionId = sdkSessionId
        self.activeRun = activeRun
    }

    enum CodingKeys: String, CodingKey {
        case agentId = "agent_id"
        case providerType = "provider_type"
        case providerLabel = "provider_label"
        case model
        case modelReasoningEffort = "model_reasoning_effort"
        case modelServiceTier = "model_service_tier"
        case modelOverride = "model_override"
        case modelReasoningEffortOverride = "model_reasoning_effort_override"
        case modelServiceTierOverride = "model_service_tier_override"
        case sdkSessionId = "sdk_session_id"
        case activeRun = "active_run"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        agentId = try container.garyxDecodeFirstString(.agentId)
        providerType = try container.garyxDecodeFirstString(.providerType)
        providerLabel = try container.garyxDecodeFirstString(.providerLabel)
        model = try container.garyxDecodeFirstString(.model)
        modelReasoningEffort = try container.garyxDecodeFirstString(.modelReasoningEffort)
        modelServiceTier = try container.garyxDecodeFirstString(.modelServiceTier)
        modelOverride = try container.garyxDecodeFirstString(.modelOverride)
        modelReasoningEffortOverride = try container.garyxDecodeFirstString(.modelReasoningEffortOverride)
        modelServiceTierOverride = try container.garyxDecodeFirstString(.modelServiceTierOverride)
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


public struct GaryxTranscriptMessage: Codable, Identifiable, Equatable, Sendable {
    public var id: String
    public var index: Int?
    public var role: GaryxTranscriptRole
    public var kind: String?
    public var internalKind: String?
    public var internalMessage: Bool
    public var text: String
    public var content: GaryxJSONValue?
    public var message: GaryxJSONValue?
    public var control: GaryxJSONValue?
    public var input: GaryxJSONValue?
    public var result: GaryxJSONValue?
    public var timestamp: String?
    public var toolRelated: Bool
    public var toolName: String?
    public var toolUseResult: Bool
    public var isError: Bool?
    public var likelyUserVisible: Bool
    /// Envelope tool identity the nested `content` omits, so committed tool rows
    /// carry the same id/parent as live events. `metadata.parent_tool_use_id`
    /// marks a sub-agent's nested tool call.
    public var toolUseId: String?
    public var metadata: GaryxJSONValue?

    enum CodingKeys: String, CodingKey {
        case index
        case role
        case kind
        case internalKind = "internal_kind"
        case internalKindCamel = "internalKind"
        case internalMessage = "internal"
        case text
        case content
        case message
        case control
        case input
        case result
        case timestamp
        case toolRelated = "tool_related"
        case toolRelatedCamel = "toolRelated"
        case toolName = "tool_name"
        case toolNameCamel = "toolName"
        case toolUseResult = "tool_use_result"
        case toolUseResultCamel = "toolUseResult"
        case isError = "is_error"
        case isErrorCamel = "isError"
        case likelyUserVisible = "likely_user_visible"
        case likelyUserVisibleCamel = "likelyUserVisible"
        case toolUseId = "tool_use_id"
        case toolUseIdCamel = "toolUseId"
        case metadata
    }

    /// The parent tool call id when this is a sub-agent's nested tool call (the
    /// gateway stamps it into `metadata.parent_tool_use_id`); `nil` for top-level
    /// calls such as the `Agent` spawn itself.
    public var garyxParentToolUseId: String? {
        guard case let .object(meta)? = metadata else { return nil }
        for key in ["parent_tool_use_id", "parentToolUseId"] {
            if case let .string(value)? = meta[key] {
                let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
                if !trimmed.isEmpty { return trimmed }
            }
        }
        return nil
    }

    public var originId: String? {
        Self.originId(role: role, metadata: metadata)
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        index = try container.decodeIfPresent(Int.self, forKey: .index)
        let roleValue = try container.decodeIfPresent(String.self, forKey: .role) ?? ""
        role = GaryxTranscriptRole(rawValue: roleValue) ?? .unknown
        kind = try container.decodeIfPresent(String.self, forKey: .kind)
        internalKind = try container.garyxDecodeFirstString(.internalKind, .internalKindCamel)
        internalMessage = try container.decodeIfPresent(Bool.self, forKey: .internalMessage) ?? false
        text = try container.decodeIfPresent(String.self, forKey: .text) ?? ""
        content = try container.decodeIfPresent(GaryxJSONValue.self, forKey: .content)
        message = try container.decodeIfPresent(GaryxJSONValue.self, forKey: .message)
        control = try container.decodeIfPresent(GaryxJSONValue.self, forKey: .control)
        input = try container.decodeIfPresent(GaryxJSONValue.self, forKey: .input)
        result = try container.decodeIfPresent(GaryxJSONValue.self, forKey: .result)
        timestamp = try container.decodeIfPresent(String.self, forKey: .timestamp)
        toolRelated = try container.decodeIfPresent(Bool.self, forKey: .toolRelated)
            ?? container.decodeIfPresent(Bool.self, forKey: .toolRelatedCamel)
            ?? false
        toolName = try container.garyxDecodeFirstString(.toolName, .toolNameCamel)
        toolUseResult = try container.decodeIfPresent(Bool.self, forKey: .toolUseResult)
            ?? container.decodeIfPresent(Bool.self, forKey: .toolUseResultCamel)
            ?? false
        isError = try container.decodeIfPresent(Bool.self, forKey: .isError)
            ?? container.decodeIfPresent(Bool.self, forKey: .isErrorCamel)
        likelyUserVisible = try container.decodeIfPresent(Bool.self, forKey: .likelyUserVisible)
            ?? container.decodeIfPresent(Bool.self, forKey: .likelyUserVisibleCamel)
            ?? true
        toolUseId = try container.garyxDecodeFirstString(.toolUseId, .toolUseIdCamel)
        metadata = try container.decodeIfPresent(GaryxJSONValue.self, forKey: .metadata)
        id = Self.messageId(index: index, role: role, metadata: metadata)
    }

    /// Symmetric encoder so committed transcript rows can be cached on device and
    /// re-decoded identically (`id` is re-derived from `index` on decode, so it is
    /// intentionally not encoded). Mirrors the gateway wire shape.
    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encodeIfPresent(index, forKey: .index)
        try container.encode(role.rawValue, forKey: .role)
        try container.encodeIfPresent(kind, forKey: .kind)
        try container.encodeIfPresent(internalKind, forKey: .internalKind)
        if internalMessage {
            try container.encode(internalMessage, forKey: .internalMessage)
        }
        try container.encode(text, forKey: .text)
        try container.encodeIfPresent(content, forKey: .content)
        try container.encodeIfPresent(message, forKey: .message)
        try container.encodeIfPresent(control, forKey: .control)
        try container.encodeIfPresent(input, forKey: .input)
        try container.encodeIfPresent(result, forKey: .result)
        try container.encodeIfPresent(timestamp, forKey: .timestamp)
        try container.encode(toolRelated, forKey: .toolRelated)
        try container.encodeIfPresent(toolName, forKey: .toolName)
        if toolUseResult {
            try container.encode(toolUseResult, forKey: .toolUseResult)
        }
        try container.encodeIfPresent(isError, forKey: .isError)
        try container.encode(likelyUserVisible, forKey: .likelyUserVisible)
        try container.encodeIfPresent(toolUseId, forKey: .toolUseId)
        try container.encodeIfPresent(metadata, forKey: .metadata)
    }

    /// Direct member-wise initializer for tests and cache reconstruction.
    public init(
        index: Int?,
        role: GaryxTranscriptRole,
        kind: String? = nil,
        internalKind: String? = nil,
        internalMessage: Bool = false,
        text: String = "",
        content: GaryxJSONValue? = nil,
        message: GaryxJSONValue? = nil,
        control: GaryxJSONValue? = nil,
        input: GaryxJSONValue? = nil,
        result: GaryxJSONValue? = nil,
        timestamp: String? = nil,
        toolRelated: Bool = false,
        toolName: String? = nil,
        toolUseResult: Bool = false,
        isError: Bool? = nil,
        likelyUserVisible: Bool = true,
        toolUseId: String? = nil,
        metadata: GaryxJSONValue? = nil
    ) {
        self.index = index
        self.role = role
        self.kind = kind
        self.internalKind = internalKind
        self.internalMessage = internalMessage
        self.text = text
        self.content = content
        self.message = message
        self.control = control
        self.input = input
        self.result = result
        self.timestamp = timestamp
        self.toolRelated = toolRelated
        self.toolName = toolName
        self.toolUseResult = toolUseResult
        self.isError = isError
        self.likelyUserVisible = likelyUserVisible
        self.toolUseId = toolUseId
        self.metadata = metadata
        self.id = Self.messageId(index: index, role: role, metadata: metadata)
    }

    private static func messageId(index: Int?, role: GaryxTranscriptRole, metadata: GaryxJSONValue?) -> String {
        if let originId = originId(role: role, metadata: metadata) {
            return "origin:\(originId)"
        }
        return index.map { "history:\($0)" } ?? UUID().uuidString
    }

    private static func originId(role: GaryxTranscriptRole, metadata: GaryxJSONValue?) -> String? {
        guard role == .user,
              case let .object(meta)? = metadata else {
            return nil
        }
        return meta.garyxGatewayStringValue(forKeys: ["origin_id"])
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
    /// Per-thread model override; wins over the agent's configured model.
    public var model: String?
    /// Per-thread reasoning/thinking level override.
    public var modelReasoningEffort: String?
    /// Per-thread service tier override.
    public var modelServiceTier: String?
    public var metadata: [String: String]

    public init(
        label: String? = nil,
        workspaceDir: String? = nil,
        workspaceMode: String? = nil,
        agentId: String? = nil,
        model: String? = nil,
        modelReasoningEffort: String? = nil,
        modelServiceTier: String? = nil,
        metadata: [String: String] = [:]
    ) {
        self.label = label
        self.workspaceDir = workspaceDir
        self.workspaceMode = workspaceMode
        self.agentId = agentId
        self.model = model
        self.modelReasoningEffort = modelReasoningEffort
        self.modelServiceTier = modelServiceTier
        self.metadata = metadata
    }
}


public struct GaryxUpdateThreadRequest: Encodable, Equatable, Sendable {
    public var label: String?
    public var workspaceDir: String?
    public var model: String?
    public var modelReasoningEffort: String?
    public var modelServiceTier: String?
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
