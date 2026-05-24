import Foundation

public struct GaryxGatewayConfiguration: Equatable, Sendable {
    public var baseURL: URL
    public var authToken: String?

    public init(baseURL: URL, authToken: String? = nil) {
        self.baseURL = baseURL
        self.authToken = authToken?.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

public enum GaryxGatewayError: Error, Equatable, LocalizedError {
    case invalidURL(String)
    case invalidHTTPResponse
    case httpStatus(Int, String)
    case encodingFailed(String)

    public var errorDescription: String? {
        switch self {
        case .invalidURL(let value):
            let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
            return trimmed.isEmpty
                ? "Enter the Garyx gateway URL from the Mac app."
                : "Invalid Garyx gateway URL: \(trimmed)"
        case .invalidHTTPResponse:
            return "The Garyx gateway returned a non-HTTP response."
        case .httpStatus(let status, let body):
            let message = GaryxGatewayError.message(fromHTTPBody: body)
            return message.isEmpty ? "The Garyx gateway returned HTTP \(status)." : message
        case .encodingFailed(let message):
            return message
        }
    }

    static func message(fromHTTPBody body: String) -> String {
        let trimmed = body.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return "" }
        guard let data = trimmed.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let error = object["error"] as? String,
              !error.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return trimmed
        }
        return error
    }
}

public struct GaryxSystemStatus: Decodable, Equatable, Sendable {
    public struct ThreadCount: Decodable, Equatable, Sendable {
        public var count: Int
    }

    public struct StreamStatus: Decodable, Equatable, Sendable {
        public var drops: Int
        public var historySize: Int

        enum CodingKeys: String, CodingKey {
            case drops
            case historySize = "history_size"
        }
    }

    public var status: String
    public var uptimeSeconds: Int?
    public var threads: ThreadCount?
    public var stream: StreamStatus?
    public var version: String?

    enum CodingKeys: String, CodingKey {
        case status
        case uptimeSeconds = "uptime_seconds"
        case threads
        case stream
        case version
    }
}

public struct GaryxChatHealth: Decodable, Equatable, Sendable {
    public var status: String
    public var channel: String
    public var bridgeReady: Bool

    enum CodingKeys: String, CodingKey {
        case status
        case channel
        case bridgeReady = "bridge_ready"
    }
}

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
        threadId = try container.decodeFirstString(.threadIdSnake, .threadId) ?? ""
    }
}

public struct GaryxDreamsPage: Decodable, Equatable, Sendable {
    public var dreams: [GaryxDreamTopic]
    public var count: Int
    public var from: String
    public var to: String
    public var latestScan: GaryxDreamScan?
    public var scan: GaryxDreamScan?

    enum CodingKeys: String, CodingKey {
        case dreams
        case count
        case from
        case to
        case latestScan
        case latestScanSnake = "latest_scan"
        case scan
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        dreams = try container.decodeIfPresent([GaryxDreamTopic].self, forKey: .dreams) ?? []
        count = try container.decodeIfPresent(Int.self, forKey: .count) ?? dreams.count
        from = try container.decodeFirstString(.from) ?? ""
        to = try container.decodeFirstString(.to) ?? ""
        latestScan = try container.decodeIfPresent(GaryxDreamScan.self, forKey: .latestScanSnake)
            ?? container.decodeIfPresent(GaryxDreamScan.self, forKey: .latestScan)
        scan = try container.decodeIfPresent(GaryxDreamScan.self, forKey: .scan)
    }
}

public struct GaryxDreamTopic: Decodable, Identifiable, Equatable, Sendable {
    public var id: String { dreamId }
    public var dreamId: String
    public var title: String
    public var summary: String
    public var firstMessageAt: String
    public var lastMessageAt: String
    public var updatedAt: String
    public var source: String
    public var confidence: Double
    public var messageCount: Int
    public var spanCount: Int
    public var spans: [GaryxDreamSpan]

    enum CodingKeys: String, CodingKey {
        case dreamId
        case dreamIdSnake = "dream_id"
        case title
        case summary
        case firstMessageAt
        case firstMessageAtSnake = "first_message_at"
        case lastMessageAt
        case lastMessageAtSnake = "last_message_at"
        case updatedAt
        case updatedAtSnake = "updated_at"
        case source
        case confidence
        case messageCount
        case messageCountSnake = "message_count"
        case spanCount
        case spanCountSnake = "span_count"
        case spans
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        dreamId = try container.decodeFirstString(.dreamIdSnake, .dreamId) ?? ""
        title = try container.decodeFirstString(.title) ?? "Untitled Dream"
        summary = try container.decodeFirstString(.summary) ?? ""
        firstMessageAt = try container.decodeFirstString(.firstMessageAtSnake, .firstMessageAt) ?? ""
        lastMessageAt = try container.decodeFirstString(.lastMessageAtSnake, .lastMessageAt) ?? ""
        updatedAt = try container.decodeFirstString(.updatedAtSnake, .updatedAt) ?? ""
        source = try container.decodeFirstString(.source) ?? "unknown"
        confidence = try container.decodeIfPresent(Double.self, forKey: .confidence) ?? 0
        messageCount = try container.decodeIfPresent(Int.self, forKey: .messageCountSnake)
            ?? container.decodeIfPresent(Int.self, forKey: .messageCount)
            ?? 0
        spanCount = try container.decodeIfPresent(Int.self, forKey: .spanCountSnake)
            ?? container.decodeIfPresent(Int.self, forKey: .spanCount)
            ?? 0
        spans = try container.decodeIfPresent([GaryxDreamSpan].self, forKey: .spans) ?? []
    }
}

public struct GaryxDreamSpan: Decodable, Identifiable, Equatable, Sendable {
    public var id: String { spanId }
    public var spanId: String
    public var dreamId: String
    public var threadId: String
    public var workspacePath: String?
    public var startSeq: Int
    public var endSeq: Int
    public var startAt: String
    public var endAt: String
    public var excerpt: String
    public var messageCount: Int

    enum CodingKeys: String, CodingKey {
        case spanId
        case spanIdSnake = "span_id"
        case dreamId
        case dreamIdSnake = "dream_id"
        case threadId
        case threadIdSnake = "thread_id"
        case workspacePath
        case workspaceDir = "workspace_dir"
        case startSeq
        case startSeqSnake = "start_seq"
        case endSeq
        case endSeqSnake = "end_seq"
        case startAt
        case startAtSnake = "start_at"
        case endAt
        case endAtSnake = "end_at"
        case excerpt
        case messageCount
        case messageCountSnake = "message_count"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        spanId = try container.decodeFirstString(.spanIdSnake, .spanId) ?? ""
        dreamId = try container.decodeFirstString(.dreamIdSnake, .dreamId) ?? ""
        threadId = try container.decodeFirstString(.threadIdSnake, .threadId) ?? ""
        workspacePath = try container.decodeFirstString(.workspaceDir, .workspacePath)
        startSeq = try container.decodeIfPresent(Int.self, forKey: .startSeqSnake)
            ?? container.decodeIfPresent(Int.self, forKey: .startSeq)
            ?? 0
        endSeq = try container.decodeIfPresent(Int.self, forKey: .endSeqSnake)
            ?? container.decodeIfPresent(Int.self, forKey: .endSeq)
            ?? 0
        startAt = try container.decodeFirstString(.startAtSnake, .startAt) ?? ""
        endAt = try container.decodeFirstString(.endAtSnake, .endAt) ?? ""
        excerpt = try container.decodeFirstString(.excerpt) ?? ""
        messageCount = try container.decodeIfPresent(Int.self, forKey: .messageCountSnake)
            ?? container.decodeIfPresent(Int.self, forKey: .messageCount)
            ?? 0
    }
}

public struct GaryxDreamScan: Decodable, Equatable, Sendable {
    public var runId: String
    public var scannedFrom: String
    public var scannedTo: String
    public var createdAt: String
    public var source: String
    public var status: String
    public var topicsCount: Int
    public var spansCount: Int
    public var error: String?

    enum CodingKeys: String, CodingKey {
        case runId
        case runIdSnake = "run_id"
        case scannedFrom
        case scannedFromSnake = "scanned_from"
        case scannedTo
        case scannedToSnake = "scanned_to"
        case createdAt
        case createdAtSnake = "created_at"
        case source
        case status
        case topicsCount
        case topicsCountSnake = "topics_count"
        case spansCount
        case spansCountSnake = "spans_count"
        case error
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        runId = try container.decodeFirstString(.runIdSnake, .runId) ?? ""
        scannedFrom = try container.decodeFirstString(.scannedFromSnake, .scannedFrom) ?? ""
        scannedTo = try container.decodeFirstString(.scannedToSnake, .scannedTo) ?? ""
        createdAt = try container.decodeFirstString(.createdAtSnake, .createdAt) ?? ""
        source = try container.decodeFirstString(.source) ?? "unknown"
        status = try container.decodeFirstString(.status) ?? "unknown"
        topicsCount = try container.decodeIfPresent(Int.self, forKey: .topicsCountSnake)
            ?? container.decodeIfPresent(Int.self, forKey: .topicsCount)
            ?? 0
        spansCount = try container.decodeIfPresent(Int.self, forKey: .spansCountSnake)
            ?? container.decodeIfPresent(Int.self, forKey: .spansCount)
            ?? 0
        error = try container.decodeFirstString(.error)
    }
}

public struct GaryxDreamScanRequest: Encodable, Equatable, Sendable {
    public var sinceHours: Int
    public var mode: String
    public var limit: Int

    enum CodingKeys: String, CodingKey {
        case sinceHours = "since_hours"
        case mode
        case limit
    }

    public init(sinceHours: Int = 24, mode: String = "auto", limit: Int = 600) {
        self.sinceHours = sinceHours
        self.mode = mode
        self.limit = limit
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
        let resolvedId = try container.decodeFirstString(.id, .threadId, .threadKey)
        id = resolvedId ?? ""
        title = try container.decodeFirstString(.title, .label) ?? "New Thread"
        createdAt = try container.decodeFirstString(.createdAt)
        updatedAt = try container.decodeFirstString(.updatedAt, .lastActiveAt)
        lastMessagePreview = try container.decodeFirstString(
            .lastMessagePreview,
            .lastMessagePreviewSnake,
            .lastUserMessage,
            .lastAssistantMessage
        ) ?? ""
        workspacePath = try container.decodeFirstString(.workspacePath, .workspaceDir)
        messageCount = try container.decodeIfPresent(Int.self, forKey: .messageCount)
        agentId = try container.decodeFirstString(.agentId)
        teamId = try container.decodeFirstString(.teamId)
        teamName = try container.decodeFirstString(.teamDisplayName, .teamDisplayNameCamel)
        providerType = try container.decodeFirstString(.providerType)
        recentRunId = try container.decodeFirstString(.recentRunId)
        activeRunId = try container.decodeFirstString(.activeRunId)
        runState = try container.decodeFirstString(.runState)
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
        path = try container.decodeFirstString(.path)
        worktreeDir = try container.decodeFirstString(.worktreeDir, .worktreeDirCamel)
    }
}

public struct GaryxAgentsPage: Decodable, Equatable, Sendable {
    public var agents: [GaryxAgentSummary]
}

public struct GaryxAgentSummary: Decodable, Identifiable, Equatable, Sendable {
    public var id: String
    public var displayName: String
    public var providerType: String
    public var model: String
    public var modelReasoningEffort: String
    public var modelServiceTier: String
    public var providerEnv: [String: String]
    public var authSource: String
    public var baseUrl: String
    public var codexHome: String
    public var maxToolIterations: Int?
    public var requestTimeoutSeconds: Int?
    public var defaultWorkspaceDir: String
    public var avatarDataUrl: String
    public var systemPrompt: String
    public var builtIn: Bool
    public var standalone: Bool
    public var createdAt: String?
    public var updatedAt: String?

    enum CodingKeys: String, CodingKey {
        case agentId = "agent_id"
        case agentIdCamel = "agentId"
        case displayName = "display_name"
        case displayNameCamel = "displayName"
        case providerType = "provider_type"
        case providerTypeCamel = "providerType"
        case model
        case modelReasoningEffort = "model_reasoning_effort"
        case modelReasoningEffortCamel = "modelReasoningEffort"
        case modelServiceTier = "model_service_tier"
        case modelServiceTierCamel = "modelServiceTier"
        case providerEnv = "provider_env"
        case providerEnvCamel = "providerEnv"
        case env
        case authSource = "auth_source"
        case authSourceCamel = "authSource"
        case baseUrl = "base_url"
        case baseUrlCamel = "baseUrl"
        case codexHome = "codex_home"
        case codexHomeCamel = "codexHome"
        case maxToolIterations = "max_tool_iterations"
        case maxToolIterationsCamel = "maxToolIterations"
        case requestTimeoutSeconds = "request_timeout_seconds"
        case requestTimeoutSecondsCamel = "requestTimeoutSeconds"
        case defaultWorkspaceDir = "default_workspace_dir"
        case defaultWorkspaceDirCamel = "defaultWorkspaceDir"
        case workspaceDir = "workspace_dir"
        case workspaceDirCamel = "workspaceDir"
        case avatarDataUrl = "avatar_data_url"
        case avatarDataUrlCamel = "avatarDataUrl"
        case avatarURL
        case avatarUrl = "avatar_url"
        case systemPrompt = "system_prompt"
        case systemPromptCamel = "systemPrompt"
        case builtIn = "built_in"
        case builtInCamel = "builtIn"
        case standalone
        case createdAt = "created_at"
        case createdAtCamel = "createdAt"
        case updatedAt = "updated_at"
        case updatedAtCamel = "updatedAt"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decodeFirstString(.agentId, .agentIdCamel) ?? ""
        displayName = try container.decodeFirstString(.displayName, .displayNameCamel) ?? id
        providerType = try container.decodeFirstString(.providerType, .providerTypeCamel) ?? ""
        model = try container.decodeFirstString(.model) ?? ""
        modelReasoningEffort = try container.decodeFirstString(.modelReasoningEffort, .modelReasoningEffortCamel) ?? ""
        modelServiceTier = try container.decodeFirstString(.modelServiceTier, .modelServiceTierCamel) ?? ""
        providerEnv = try container.decodeIfPresent([String: String].self, forKey: .providerEnv)
            ?? container.decodeIfPresent([String: String].self, forKey: .providerEnvCamel)
            ?? container.decodeIfPresent([String: String].self, forKey: .env)
            ?? [:]
        authSource = try container.decodeFirstString(.authSource, .authSourceCamel) ?? ""
        baseUrl = try container.decodeFirstString(.baseUrl, .baseUrlCamel) ?? ""
        codexHome = try container.decodeFirstString(.codexHome, .codexHomeCamel) ?? ""
        maxToolIterations = try container.decodeFirstInt(.maxToolIterations, .maxToolIterationsCamel)
        requestTimeoutSeconds = try container.decodeFirstInt(.requestTimeoutSeconds, .requestTimeoutSecondsCamel)
        defaultWorkspaceDir = try container.decodeFirstString(
            .defaultWorkspaceDir,
            .defaultWorkspaceDirCamel,
            .workspaceDir,
            .workspaceDirCamel
        ) ?? ""
        avatarDataUrl = try container.decodeFirstString(
            .avatarDataUrl,
            .avatarDataUrlCamel,
            .avatarURL,
            .avatarUrl
        ) ?? ""
        systemPrompt = try container.decodeFirstString(.systemPrompt, .systemPromptCamel) ?? ""
        builtIn = try container.decodeFirstBool(.builtIn, .builtInCamel) ?? false
        standalone = try container.decodeFirstBool(.standalone) ?? true
        createdAt = try container.decodeFirstString(.createdAt, .createdAtCamel)
        updatedAt = try container.decodeFirstString(.updatedAt, .updatedAtCamel)
    }
}

public struct GaryxTeamsPage: Decodable, Equatable, Sendable {
    public var teams: [GaryxTeamSummary]
}

public struct GaryxTeamSummary: Decodable, Identifiable, Equatable, Sendable {
    public var id: String
    public var displayName: String
    public var leaderAgentId: String
    public var memberAgentIds: [String]
    public var workflowText: String
    public var avatarDataUrl: String
    public var createdAt: String?
    public var updatedAt: String?

    enum CodingKeys: String, CodingKey {
        case teamId = "team_id"
        case teamIdCamel = "teamId"
        case displayName = "display_name"
        case displayNameCamel = "displayName"
        case leaderAgentId = "leader_agent_id"
        case leaderAgentIdCamel = "leaderAgentId"
        case memberAgentIds = "member_agent_ids"
        case memberAgentIdsCamel = "memberAgentIds"
        case workflowText = "workflow_text"
        case workflowTextCamel = "workflowText"
        case avatarDataUrl = "avatar_data_url"
        case avatarDataUrlCamel = "avatarDataUrl"
        case avatarURL
        case avatarUrl = "avatar_url"
        case createdAt = "created_at"
        case createdAtCamel = "createdAt"
        case updatedAt = "updated_at"
        case updatedAtCamel = "updatedAt"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decodeFirstString(.teamId, .teamIdCamel) ?? ""
        displayName = try container.decodeFirstString(.displayName, .displayNameCamel) ?? id
        leaderAgentId = try container.decodeFirstString(.leaderAgentId, .leaderAgentIdCamel) ?? ""
        memberAgentIds = try container.decodeFirstStringArray(.memberAgentIds, .memberAgentIdsCamel) ?? []
        workflowText = try container.decodeFirstString(.workflowText, .workflowTextCamel) ?? ""
        avatarDataUrl = try container.decodeFirstString(
            .avatarDataUrl,
            .avatarDataUrlCamel,
            .avatarURL,
            .avatarUrl
        ) ?? ""
        createdAt = try container.decodeFirstString(.createdAt, .createdAtCamel)
        updatedAt = try container.decodeFirstString(.updatedAt, .updatedAtCamel)
    }
}

public struct GaryxProviderModelOption: Decodable, Identifiable, Equatable, Sendable {
    public var id: String
    public var label: String
    public var description: String?
    public var recommended: Bool
    public var defaultReasoningEffort: String?
    public var supportedReasoningEfforts: [GaryxProviderModelOption]
    public var serviceTiers: [GaryxProviderModelOption]

    enum CodingKeys: String, CodingKey {
        case id
        case label
        case description
        case recommended
        case defaultReasoningEffort
        case defaultReasoningEffortSnake = "default_reasoning_effort"
        case supportedReasoningEfforts
        case supportedReasoningEffortsSnake = "supported_reasoning_efforts"
        case serviceTiers
        case serviceTiersSnake = "service_tiers"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decodeFirstString(.id) ?? ""
        label = try container.decodeFirstString(.label) ?? id
        description = try container.decodeFirstString(.description)
        recommended = try container.decodeFirstBool(.recommended) ?? false
        defaultReasoningEffort = try container.decodeFirstString(
            .defaultReasoningEffort,
            .defaultReasoningEffortSnake
        )
        supportedReasoningEfforts = try container.decodeIfPresent(
            [GaryxProviderModelOption].self,
            forKey: .supportedReasoningEfforts
        ) ?? container.decodeIfPresent(
            [GaryxProviderModelOption].self,
            forKey: .supportedReasoningEffortsSnake
        ) ?? []
        serviceTiers = try container.decodeIfPresent([GaryxProviderModelOption].self, forKey: .serviceTiers)
            ?? container.decodeIfPresent([GaryxProviderModelOption].self, forKey: .serviceTiersSnake)
            ?? []
    }
}

public struct GaryxProviderModels: Decodable, Equatable, Sendable {
    public var providerType: String
    public var supportsModelSelection: Bool
    public var models: [GaryxProviderModelOption]
    public var supportsReasoningEffortSelection: Bool
    public var reasoningEfforts: [GaryxProviderModelOption]
    public var supportsServiceTierSelection: Bool
    public var serviceTiers: [GaryxProviderModelOption]
    public var defaultModel: String?
    public var source: String
    public var error: String?

    enum CodingKeys: String, CodingKey {
        case providerType
        case providerTypeSnake = "provider_type"
        case supportsModelSelection
        case supportsModelSelectionSnake = "supports_model_selection"
        case models
        case supportsReasoningEffortSelection
        case supportsReasoningEffortSelectionSnake = "supports_reasoning_effort_selection"
        case reasoningEfforts
        case reasoningEffortsSnake = "reasoning_efforts"
        case supportsServiceTierSelection
        case supportsServiceTierSelectionSnake = "supports_service_tier_selection"
        case serviceTiers
        case serviceTiersSnake = "service_tiers"
        case defaultModel
        case defaultModelSnake = "default_model"
        case source
        case error
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        providerType = try container.decodeFirstString(.providerType, .providerTypeSnake) ?? ""
        supportsModelSelection = try container.decodeFirstBool(
            .supportsModelSelection,
            .supportsModelSelectionSnake
        ) ?? false
        models = try container.decodeIfPresent([GaryxProviderModelOption].self, forKey: .models) ?? []
        supportsReasoningEffortSelection = try container.decodeFirstBool(
            .supportsReasoningEffortSelection,
            .supportsReasoningEffortSelectionSnake
        ) ?? false
        reasoningEfforts = try container.decodeIfPresent(
            [GaryxProviderModelOption].self,
            forKey: .reasoningEfforts
        ) ?? container.decodeIfPresent([GaryxProviderModelOption].self, forKey: .reasoningEffortsSnake) ?? []
        supportsServiceTierSelection = try container.decodeFirstBool(
            .supportsServiceTierSelection,
            .supportsServiceTierSelectionSnake
        ) ?? false
        serviceTiers = try container.decodeIfPresent([GaryxProviderModelOption].self, forKey: .serviceTiers)
            ?? container.decodeIfPresent([GaryxProviderModelOption].self, forKey: .serviceTiersSnake)
            ?? []
        defaultModel = try container.decodeFirstString(.defaultModel, .defaultModelSnake)
        source = try container.decodeFirstString(.source) ?? ""
        error = try container.decodeFirstString(.error)
    }
}

public struct GaryxGeneratedAvatar: Decodable, Equatable, Sendable {
    public var avatarDataUrl: String
    public var mediaType: String

    enum CodingKeys: String, CodingKey {
        case avatarDataUrl
        case avatarDataUrlSnake = "avatar_data_url"
        case dataBase64
        case dataBase64Snake = "data_base64"
        case mediaType
        case mediaTypeSnake = "media_type"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        mediaType = try container.decodeFirstString(.mediaType, .mediaTypeSnake) ?? "image/png"
        if let dataUrl = try container.decodeFirstString(.avatarDataUrl, .avatarDataUrlSnake) {
            avatarDataUrl = dataUrl
        } else if let encoded = try container.decodeFirstString(.dataBase64, .dataBase64Snake), !encoded.isEmpty {
            avatarDataUrl = "data:\(mediaType);base64,\(encoded)"
        } else {
            avatarDataUrl = ""
        }
    }
}

public struct GaryxGenerateAvatarRequest: Encodable, Equatable, Sendable {
    public var prompt: String
    public var timeoutSecs: Int

    public init(prompt: String, timeoutSecs: Int = 600) {
        self.prompt = prompt
        self.timeoutSecs = timeoutSecs
    }

    enum CodingKeys: String, CodingKey {
        case prompt
        case timeoutSecs = "timeout_secs"
    }
}

public struct GaryxCustomAgentRequest: Encodable, Equatable, Sendable {
    public var agentId: String
    public var displayName: String
    public var providerType: String
    public var model: String?
    public var modelReasoningEffort: String?
    public var modelServiceTier: String?
    public var providerEnv: [String: String]?
    public var authSource: String?
    public var baseUrl: String?
    public var codexHome: String?
    public var maxToolIterations: Int?
    public var requestTimeoutSeconds: Int?
    public var defaultWorkspaceDir: String?
    public var avatarDataUrl: String?
    public var systemPrompt: String?

    public init(
        agentId: String,
        displayName: String,
        providerType: String,
        model: String? = nil,
        modelReasoningEffort: String? = nil,
        modelServiceTier: String? = nil,
        providerEnv: [String: String]? = nil,
        authSource: String? = nil,
        baseUrl: String? = nil,
        codexHome: String? = nil,
        maxToolIterations: Int? = nil,
        requestTimeoutSeconds: Int? = nil,
        defaultWorkspaceDir: String? = nil,
        avatarDataUrl: String? = nil,
        systemPrompt: String? = nil
    ) {
        self.agentId = agentId
        self.displayName = displayName
        self.providerType = providerType
        self.model = model
        self.modelReasoningEffort = modelReasoningEffort
        self.modelServiceTier = modelServiceTier
        self.providerEnv = providerEnv
        self.authSource = authSource
        self.baseUrl = baseUrl
        self.codexHome = codexHome
        self.maxToolIterations = maxToolIterations
        self.requestTimeoutSeconds = requestTimeoutSeconds
        self.defaultWorkspaceDir = defaultWorkspaceDir
        self.avatarDataUrl = avatarDataUrl
        self.systemPrompt = systemPrompt
    }

    enum CodingKeys: String, CodingKey {
        case agentId = "agent_id"
        case displayName = "display_name"
        case providerType = "provider_type"
        case model
        case modelReasoningEffort = "model_reasoning_effort"
        case modelServiceTier = "model_service_tier"
        case providerEnv = "provider_env"
        case authSource = "auth_source"
        case baseUrl = "base_url"
        case codexHome = "codex_home"
        case maxToolIterations = "max_tool_iterations"
        case requestTimeoutSeconds = "request_timeout_seconds"
        case defaultWorkspaceDir = "default_workspace_dir"
        case avatarDataUrl = "avatar_data_url"
        case systemPrompt = "system_prompt"
    }
}

public struct GaryxTeamRequest: Encodable, Equatable, Sendable {
    public var teamId: String
    public var displayName: String
    public var leaderAgentId: String
    public var memberAgentIds: [String]
    public var workflowText: String
    public var avatarDataUrl: String?

    public init(
        teamId: String,
        displayName: String,
        leaderAgentId: String,
        memberAgentIds: [String],
        workflowText: String,
        avatarDataUrl: String? = nil
    ) {
        self.teamId = teamId
        self.displayName = displayName
        self.leaderAgentId = leaderAgentId
        self.memberAgentIds = memberAgentIds
        self.workflowText = workflowText
        self.avatarDataUrl = avatarDataUrl
    }
}

public enum GaryxTaskStatus: String, Codable, Equatable, Sendable, CaseIterable {
    case todo
    case inProgress = "in_progress"
    case inReview = "in_review"
    case done
}

public struct GaryxTasksPage: Decodable, Equatable, Sendable {
    public var tasks: [GaryxTaskSummary]
    public var total: Int
    public var hasMore: Bool

    enum CodingKeys: String, CodingKey {
        case tasks
        case total
        case hasMore = "has_more"
        case hasMoreCamel = "hasMore"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        tasks = try container.decodeIfPresent([GaryxTaskSummary].self, forKey: .tasks) ?? []
        total = try container.decodeIfPresent(Int.self, forKey: .total) ?? tasks.count
        hasMore = try container.decodeFirstBool(.hasMore, .hasMoreCamel) ?? false
    }
}

public struct GaryxTaskSummary: Decodable, Identifiable, Equatable, Sendable {
    public var id: String
    public var threadId: String
    public var number: Int
    public var title: String
    public var status: GaryxTaskStatus
    public var creator: GaryxTaskPrincipal?
    public var assignee: GaryxTaskPrincipal?
    public var assigneeLabel: String
    public var source: GaryxTaskSource?
    public var updatedBy: GaryxTaskPrincipal?
    public var runtimeAgentId: String
    public var replyCount: Int
    public var updatedAt: String?

    enum CodingKeys: String, CodingKey {
        case task
        case threadId = "thread_id"
        case threadIdCamel = "threadId"
        case taskId = "task_id"
        case taskIdCamel = "taskId"
        case number
        case title
        case status
        case creator
        case assignee
        case source
        case updatedBy = "updated_by"
        case updatedByCamel = "updatedBy"
        case runtimeAgentId = "runtime_agent_id"
        case runtimeAgentIdCamel = "runtimeAgentId"
        case replyCount = "reply_count"
        case replyCountCamel = "replyCount"
        case updatedAt = "updated_at"
        case updatedAtCamel = "updatedAt"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        if let nested = try container.decodeIfPresent(GaryxTaskSummary.self, forKey: .task) {
            var summary = nested
            if let id = try container.decodeFirstString(.taskId, .taskIdCamel) {
                summary.id = id
            }
            if let threadId = try container.decodeFirstString(.threadId, .threadIdCamel) {
                summary.threadId = threadId
            }
            if let runtimeAgentId = try container.decodeFirstString(.runtimeAgentId, .runtimeAgentIdCamel) {
                summary.runtimeAgentId = runtimeAgentId
            }
            self = summary
            return
        }
        number = try container.decodeIfPresent(Int.self, forKey: .number) ?? 0
        id = try container.decodeFirstString(.taskId, .taskIdCamel) ?? (number > 0 ? "#TASK-\(number)" : "")
        threadId = try container.decodeFirstString(.threadId, .threadIdCamel) ?? ""
        title = try container.decodeFirstString(.title) ?? (number > 0 ? "#TASK-\(number)" : "Untitled task")
        let rawStatus = try container.decodeFirstString(.status) ?? "todo"
        status = GaryxTaskStatus(rawValue: rawStatus) ?? .todo
        creator = try container.decodeIfPresent(GaryxTaskPrincipal.self, forKey: .creator)
        assignee = try container.decodeIfPresent(GaryxTaskPrincipal.self, forKey: .assignee)
        assigneeLabel = assignee?.label ?? ""
        source = try container.decodeIfPresent(GaryxTaskSource.self, forKey: .source)
        updatedBy = try container.decodeIfPresent(GaryxTaskPrincipal.self, forKey: .updatedBy)
            ?? container.decodeIfPresent(GaryxTaskPrincipal.self, forKey: .updatedByCamel)
        runtimeAgentId = try container.decodeFirstString(.runtimeAgentId, .runtimeAgentIdCamel) ?? ""
        replyCount = try container.decodeIfPresent(Int.self, forKey: .replyCount)
            ?? container.decodeIfPresent(Int.self, forKey: .replyCountCamel)
            ?? 0
        updatedAt = try container.decodeFirstString(.updatedAt, .updatedAtCamel)
    }
}

public struct GaryxTaskSource: Decodable, Equatable, Sendable {
    public var threadId: String?
    public var taskId: String?
    public var taskThreadId: String?
    public var botId: String?
    public var channel: String?
    public var accountId: String?

    enum CodingKeys: String, CodingKey {
        case threadId = "thread_id"
        case threadIdCamel = "threadId"
        case taskId = "task_id"
        case taskIdCamel = "taskId"
        case taskThreadId = "task_thread_id"
        case taskThreadIdCamel = "taskThreadId"
        case botId = "bot_id"
        case botIdCamel = "botId"
        case channel
        case accountId = "account_id"
        case accountIdCamel = "accountId"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        threadId = try container.decodeFirstString(.threadId, .threadIdCamel)
        taskId = try container.decodeFirstString(.taskId, .taskIdCamel)
        taskThreadId = try container.decodeFirstString(.taskThreadId, .taskThreadIdCamel)
        botId = try container.decodeFirstString(.botId, .botIdCamel)
        channel = try container.decodeFirstString(.channel)
        accountId = try container.decodeFirstString(.accountId, .accountIdCamel)
    }
}

public struct GaryxTaskPrincipal: Decodable, Equatable, Sendable {
    public var kind: String
    public var agentId: String?
    public var userId: String?

    public var label: String {
        if kind == "agent", let agentId, !agentId.isEmpty {
            return agentId
        }
        if let userId, !userId.isEmpty {
            return userId
        }
        return kind
    }

    enum CodingKeys: String, CodingKey {
        case kind
        case agentId = "agent_id"
        case agentIdCamel = "agentId"
        case userId = "user_id"
        case userIdCamel = "userId"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        kind = try container.decodeFirstString(.kind) ?? ""
        agentId = try container.decodeFirstString(.agentId, .agentIdCamel)
        userId = try container.decodeFirstString(.userId, .userIdCamel)
    }
}

public struct GaryxTaskCreateRequest: Encodable, Equatable, Sendable {
    public var title: String?
    public var body: String?
    public var assignee: GaryxTaskPrincipalRequest?
    public var start: Bool
    public var runtime: GaryxTaskRuntimeRequest?
    public var notificationTarget: GaryxTaskNotificationTargetRequest

    public init(
        title: String? = nil,
        body: String? = nil,
        assignee: GaryxTaskPrincipalRequest? = nil,
        start: Bool = false,
        runtime: GaryxTaskRuntimeRequest? = nil,
        notificationTarget: GaryxTaskNotificationTargetRequest = .none
    ) {
        self.title = title
        self.body = body
        self.assignee = assignee
        self.start = start
        self.runtime = runtime
        self.notificationTarget = notificationTarget
    }

    enum CodingKeys: String, CodingKey {
        case title
        case body
        case assignee
        case start
        case runtime
        case notificationTarget = "notification_target"
    }
}

public struct GaryxTaskPrincipalRequest: Encodable, Equatable, Sendable {
    public var kind: String
    public var agentId: String?
    public var userId: String?

    public static func agent(_ agentId: String) -> Self {
        Self(kind: "agent", agentId: agentId, userId: nil)
    }

    enum CodingKeys: String, CodingKey {
        case kind
        case agentId = "agent_id"
        case userId = "user_id"
    }
}

public struct GaryxTaskRuntimeRequest: Encodable, Equatable, Sendable {
    public var agentId: String?
    public var workspaceDir: String?
    public var workspaceMode: String

    public init(agentId: String? = nil, workspaceDir: String? = nil, workspaceMode: String = "local") {
        self.agentId = agentId
        self.workspaceDir = workspaceDir
        self.workspaceMode = workspaceMode
    }

    enum CodingKeys: String, CodingKey {
        case agentId = "agent_id"
        case workspaceDir = "workspace_dir"
        case workspaceMode = "workspace_mode"
    }
}

public enum GaryxTaskNotificationTargetRequest: Encodable, Equatable, Sendable {
    case none
    case bot(channel: String, accountId: String)

    enum CodingKeys: String, CodingKey {
        case kind
        case channel
        case accountId = "account_id"
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .none:
            try container.encode("none", forKey: .kind)
        case .bot(let channel, let accountId):
            try container.encode("bot", forKey: .kind)
            try container.encode(channel, forKey: .channel)
            try container.encode(accountId, forKey: .accountId)
        }
    }
}

public struct GaryxTaskUpdateStatusRequest: Encodable, Equatable, Sendable {
    public var to: GaryxTaskStatus
    public var note: String?
    public var force: Bool

    public init(to: GaryxTaskStatus, note: String? = nil, force: Bool = false) {
        self.to = to
        self.note = note
        self.force = force
    }
}

public struct GaryxTaskListFilter: Equatable, Sendable {
    public var status: GaryxTaskStatus?
    public var assignee: String?
    public var sourceBotId: String?
    public var includeDone: Bool
    public var limit: Int
    public var offset: Int

    public init(
        status: GaryxTaskStatus? = nil,
        assignee: String? = nil,
        sourceBotId: String? = nil,
        includeDone: Bool = true,
        limit: Int = 100,
        offset: Int = 0
    ) {
        self.status = status
        self.assignee = assignee
        self.sourceBotId = sourceBotId
        self.includeDone = includeDone
        self.limit = limit
        self.offset = offset
    }
}

public struct GaryxTaskPromoteRequest: Encodable, Equatable, Sendable {
    public var threadId: String
    public var title: String?
    public var assignee: GaryxTaskPrincipalRequest?
    public var notificationTarget: GaryxTaskNotificationTargetRequest

    public init(
        threadId: String,
        title: String? = nil,
        assignee: GaryxTaskPrincipalRequest? = nil,
        notificationTarget: GaryxTaskNotificationTargetRequest = .none
    ) {
        self.threadId = threadId
        self.title = title
        self.assignee = assignee
        self.notificationTarget = notificationTarget
    }

    enum CodingKeys: String, CodingKey {
        case threadId = "thread_id"
        case title
        case assignee
        case notificationTarget = "notification_target"
    }
}

public struct GaryxTaskAssignRequest: Encodable, Equatable, Sendable {
    public var to: GaryxTaskPrincipalRequest

    public init(to: GaryxTaskPrincipalRequest) {
        self.to = to
    }
}

public struct GaryxTaskUpdateTitleRequest: Encodable, Equatable, Sendable {
    public var title: String

    public init(title: String) {
        self.title = title
    }
}

public struct GaryxAutomationsPage: Decodable, Equatable, Sendable {
    public var automations: [GaryxAutomationSummary]
}

public struct GaryxAutomationSummary: Decodable, Identifiable, Equatable, Sendable {
    public var id: String
    public var label: String
    public var prompt: String
    public var agentId: String
    public var enabled: Bool
    public var workspacePath: String
    public var targetThreadId: String?
    public var threadId: String?
    public var nextRun: String
    public var lastRunAt: String?
    public var lastStatus: String
    public var schedule: GaryxAutomationSchedule

    enum CodingKeys: String, CodingKey {
        case id
        case label
        case prompt
        case agentId = "agent_id"
        case agentIdCamel = "agentId"
        case enabled
        case workspaceDir = "workspace_dir"
        case workspaceDirCamel = "workspaceDir"
        case targetThreadId = "target_thread_id"
        case targetThreadIdCamel = "targetThreadId"
        case threadId = "thread_id"
        case threadIdCamel = "threadId"
        case nextRun = "next_run"
        case nextRunCamel = "nextRun"
        case lastRunAt = "last_run_at"
        case lastRunAtCamel = "lastRunAt"
        case lastStatus = "last_status"
        case lastStatusCamel = "lastStatus"
        case schedule
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decodeFirstString(.id) ?? ""
        label = try container.decodeFirstString(.label) ?? id
        prompt = try container.decodeFirstString(.prompt) ?? ""
        agentId = try container.decodeFirstString(.agentId, .agentIdCamel) ?? "claude"
        enabled = try container.decodeIfPresent(Bool.self, forKey: .enabled) ?? true
        workspacePath = try container.decodeFirstString(.workspaceDir, .workspaceDirCamel) ?? ""
        targetThreadId = try container.decodeFirstString(.targetThreadId, .targetThreadIdCamel)
        threadId = try container.decodeFirstString(.threadId, .threadIdCamel)
        nextRun = try container.decodeFirstString(.nextRun, .nextRunCamel) ?? ""
        lastRunAt = try container.decodeFirstString(.lastRunAt, .lastRunAtCamel)
        lastStatus = try container.decodeFirstString(.lastStatus, .lastStatusCamel) ?? "success"
        schedule = try container.decodeIfPresent(GaryxAutomationSchedule.self, forKey: .schedule) ?? .interval(hours: 24)
    }
}

public struct GaryxAutomationSchedule: Codable, Equatable, Sendable {
    public enum Kind: String, Codable, Equatable, Sendable {
        case daily
        case interval
        case once
    }

    public var kind: Kind
    public var time: String?
    public var weekdays: [String]
    public var timezone: String?
    public var hours: Int?
    public var at: String?

    public static func daily(
        time: String = "09:00",
        weekdays: [String] = ["mo", "tu", "we", "th", "fr"],
        timezone: String = "UTC"
    ) -> Self {
        Self(kind: .daily, time: time, weekdays: weekdays, timezone: timezone, hours: nil, at: nil)
    }

    public static func interval(hours: Int = 24) -> Self {
        Self(kind: .interval, time: nil, weekdays: [], timezone: nil, hours: max(1, hours), at: nil)
    }

    public static func once(at: String) -> Self {
        Self(kind: .once, time: nil, weekdays: [], timezone: nil, hours: nil, at: at)
    }

    public init(
        kind: Kind,
        time: String? = nil,
        weekdays: [String] = [],
        timezone: String? = nil,
        hours: Int? = nil,
        at: String? = nil
    ) {
        self.kind = kind
        self.time = time
        self.weekdays = weekdays
        self.timezone = timezone
        self.hours = hours
        self.at = at
    }

    enum CodingKeys: String, CodingKey {
        case kind
        case time
        case weekdays
        case timezone
        case hours
        case at
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        kind = try container.decodeIfPresent(Kind.self, forKey: .kind) ?? .interval
        time = try container.decodeIfPresent(String.self, forKey: .time)
        weekdays = try container.decodeIfPresent([String].self, forKey: .weekdays) ?? []
        timezone = try container.decodeIfPresent(String.self, forKey: .timezone)
        hours = try container.decodeIfPresent(Int.self, forKey: .hours)
        at = try container.decodeIfPresent(String.self, forKey: .at)
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(kind, forKey: .kind)
        switch kind {
        case .daily:
            try container.encode(time ?? "09:00", forKey: .time)
            try container.encode(weekdays, forKey: .weekdays)
            try container.encode(timezone ?? "UTC", forKey: .timezone)
        case .interval:
            try container.encode(max(1, hours ?? 24), forKey: .hours)
        case .once:
            try container.encode(at ?? "", forKey: .at)
        }
    }
}

public struct GaryxAutomationCreateRequest: Encodable, Equatable, Sendable {
    public var label: String
    public var prompt: String
    public var agentId: String?
    public var workspaceDir: String?
    public var targetThreadId: String?
    public var schedule: GaryxAutomationSchedule
    public var enabled: Bool?

    public init(
        label: String,
        prompt: String,
        agentId: String? = nil,
        workspaceDir: String? = nil,
        targetThreadId: String? = nil,
        schedule: GaryxAutomationSchedule = .interval(hours: 24),
        enabled: Bool? = nil
    ) {
        self.label = label
        self.prompt = prompt
        self.agentId = agentId
        self.workspaceDir = workspaceDir
        self.targetThreadId = targetThreadId
        self.schedule = schedule
        self.enabled = enabled
    }
}

public struct GaryxAutomationUpdateRequest: Encodable, Equatable, Sendable {
    public var label: String?
    public var prompt: String?
    public var agentId: String?
    public var workspaceDir: String?
    public var targetThreadId: String?
    public var clearsTargetThreadId: Bool
    public var schedule: GaryxAutomationSchedule?
    public var enabled: Bool?

    public init(
        label: String? = nil,
        prompt: String? = nil,
        agentId: String? = nil,
        workspaceDir: String? = nil,
        targetThreadId: String? = nil,
        clearsTargetThreadId: Bool = false,
        schedule: GaryxAutomationSchedule? = nil,
        enabled: Bool? = nil
    ) {
        self.label = label
        self.prompt = prompt
        self.agentId = agentId
        self.workspaceDir = workspaceDir
        self.targetThreadId = targetThreadId
        self.clearsTargetThreadId = clearsTargetThreadId
        self.schedule = schedule
        self.enabled = enabled
    }

    enum CodingKeys: String, CodingKey {
        case label
        case prompt
        case agentId
        case workspaceDir
        case targetThreadId
        case schedule
        case enabled
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encodeIfPresent(label, forKey: .label)
        try container.encodeIfPresent(prompt, forKey: .prompt)
        try container.encodeIfPresent(agentId, forKey: .agentId)
        try container.encodeIfPresent(workspaceDir, forKey: .workspaceDir)
        if let targetThreadId {
            try container.encode(targetThreadId, forKey: .targetThreadId)
        } else if clearsTargetThreadId {
            try container.encodeNil(forKey: .targetThreadId)
        }
        try container.encodeIfPresent(schedule, forKey: .schedule)
        try container.encodeIfPresent(enabled, forKey: .enabled)
    }
}

public struct GaryxAutomationActivityEntry: Decodable, Equatable, Sendable {
    public var runId: String
    public var status: String
    public var startedAt: String
    public var excerpt: String?
    public var threadId: String

    enum CodingKeys: String, CodingKey {
        case runId = "run_id"
        case runIdCamel = "runId"
        case status
        case startedAt = "started_at"
        case startedAtCamel = "startedAt"
        case excerpt
        case threadId = "thread_id"
        case threadIdCamel = "threadId"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        runId = try container.decodeFirstString(.runId, .runIdCamel) ?? ""
        status = try container.decodeFirstString(.status) ?? "success"
        startedAt = try container.decodeFirstString(.startedAt, .startedAtCamel) ?? ""
        excerpt = try container.decodeFirstString(.excerpt)
        threadId = try container.decodeFirstString(.threadId, .threadIdCamel) ?? ""
    }
}

public struct GaryxAutomationActivityFeed: Decodable, Equatable, Sendable {
    public var items: [GaryxAutomationActivityEntry]
    public var threadId: String
    public var count: Int

    enum CodingKeys: String, CodingKey {
        case items
        case threadId
        case threadIdSnake = "thread_id"
        case count
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        items = try container.decodeIfPresent([GaryxAutomationActivityEntry].self, forKey: .items) ?? []
        threadId = try container.decodeFirstString(.threadId, .threadIdSnake) ?? ""
        count = try container.decodeIfPresent(Int.self, forKey: .count) ?? items.count
    }
}

public struct GaryxSkillsPage: Decodable, Equatable, Sendable {
    public var skills: [GaryxSkillSummary]
}

public struct GaryxSkillSummary: Decodable, Identifiable, Equatable, Sendable {
    public var id: String
    public var name: String
    public var description: String
    public var installed: Bool
    public var enabled: Bool
    public var sourcePath: String

    enum CodingKeys: String, CodingKey {
        case id
        case name
        case description
        case installed
        case enabled
        case sourcePath = "source_path"
        case sourcePathCamel = "sourcePath"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decodeFirstString(.id) ?? ""
        name = try container.decodeFirstString(.name) ?? id
        description = try container.decodeFirstString(.description) ?? ""
        installed = try container.decodeFirstBool(.installed) ?? false
        enabled = try container.decodeFirstBool(.enabled) ?? true
        sourcePath = try container.decodeFirstString(.sourcePath, .sourcePathCamel) ?? ""
    }
}

public struct GaryxCreateSkillRequest: Encodable, Equatable, Sendable {
    public var id: String
    public var name: String
    public var description: String
    public var body: String

    public init(id: String, name: String, description: String, body: String) {
        self.id = id
        self.name = name
        self.description = description
        self.body = body
    }
}

public struct GaryxUpdateSkillRequest: Encodable, Equatable, Sendable {
    public var name: String
    public var description: String

    public init(name: String, description: String) {
        self.name = name
        self.description = description
    }
}

public struct GaryxSkillEntryNode: Decodable, Identifiable, Equatable, Sendable {
    public var id: String { path }
    public var path: String
    public var name: String
    public var entryType: String
    public var children: [GaryxSkillEntryNode]

    enum CodingKeys: String, CodingKey {
        case path
        case name
        case entryType
        case entryTypeSnake = "entry_type"
        case children
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        path = try container.decodeFirstString(.path) ?? ""
        name = try container.decodeFirstString(.name) ?? path.lastPathComponent
        entryType = try container.decodeFirstString(.entryType, .entryTypeSnake) ?? "file"
        children = try container.decodeIfPresent([GaryxSkillEntryNode].self, forKey: .children) ?? []
    }
}

public struct GaryxSkillEditorState: Decodable, Equatable, Sendable {
    public var skill: GaryxSkillSummary
    public var entries: [GaryxSkillEntryNode]
}

public struct GaryxSkillFileDocument: Decodable, Equatable, Sendable {
    public var skill: GaryxSkillSummary
    public var path: String
    public var content: String
    public var mediaType: String
    public var previewKind: String
    public var dataBase64: String?
    public var editable: Bool

    enum CodingKeys: String, CodingKey {
        case skill
        case path
        case content
        case mediaType
        case mediaTypeSnake = "media_type"
        case previewKind
        case previewKindSnake = "preview_kind"
        case dataBase64
        case dataBase64Snake = "data_base64"
        case editable
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        skill = try container.decode(GaryxSkillSummary.self, forKey: .skill)
        path = try container.decodeFirstString(.path) ?? ""
        content = try container.decodeFirstString(.content) ?? ""
        mediaType = try container.decodeFirstString(.mediaType, .mediaTypeSnake) ?? "text/plain"
        previewKind = try container.decodeFirstString(.previewKind, .previewKindSnake) ?? "text"
        dataBase64 = try container.decodeFirstString(.dataBase64, .dataBase64Snake)
        editable = try container.decodeIfPresent(Bool.self, forKey: .editable) ?? false
    }
}

public struct GaryxSkillFileWriteRequest: Encodable, Equatable, Sendable {
    public var path: String
    public var content: String

    public init(path: String, content: String) {
        self.path = path
        self.content = content
    }
}

public struct GaryxSkillEntryCreateRequest: Encodable, Equatable, Sendable {
    public var path: String
    public var entryType: String

    public init(path: String, entryType: String) {
        self.path = path
        self.entryType = entryType
    }
}

public struct GaryxSlashCommandsPage: Decodable, Equatable, Sendable {
    public var commands: [GaryxSlashCommand]
}

public struct GaryxSlashCommand: Decodable, Identifiable, Equatable, Sendable {
    public var id: String { name }
    public var name: String
    public var description: String
    public var prompt: String

    enum CodingKeys: String, CodingKey {
        case name
        case description
        case prompt
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        name = try container.decodeFirstString(.name) ?? ""
        description = try container.decodeFirstString(.description) ?? ""
        prompt = try container.decodeFirstString(.prompt) ?? ""
    }
}

public struct GaryxSlashCommandRequest: Encodable, Equatable, Sendable {
    public var name: String
    public var description: String
    public var prompt: String?

    public init(name: String, description: String, prompt: String?) {
        self.name = name
        self.description = description
        self.prompt = prompt
    }
}

public struct GaryxMcpServersPage: Decodable, Equatable, Sendable {
    public var servers: [GaryxMcpServer]
}

public struct GaryxMcpServer: Decodable, Identifiable, Equatable, Sendable {
    public var id: String { name }
    public var name: String
    public var transport: String
    public var command: String
    public var args: [String]
    public var env: [String: String]
    public var enabled: Bool
    public var workingDir: String?
    public var url: String?
    public var bearerTokenEnv: String?
    public var headers: [String: String]

    enum CodingKeys: String, CodingKey {
        case name
        case transport
        case command
        case args
        case env
        case enabled
        case workingDir
        case workingDirSnake = "working_dir"
        case url
        case bearerTokenEnv
        case bearerTokenEnvSnake = "bearer_token_env"
        case headers
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        name = try container.decodeFirstString(.name) ?? ""
        transport = try container.decodeFirstString(.transport) ?? "stdio"
        command = try container.decodeFirstString(.command) ?? ""
        args = try container.decodeIfPresent([String].self, forKey: .args) ?? []
        env = try container.decodeIfPresent([String: String].self, forKey: .env) ?? [:]
        enabled = try container.decodeIfPresent(Bool.self, forKey: .enabled) ?? true
        workingDir = try container.decodeFirstString(.workingDir, .workingDirSnake)
        url = try container.decodeFirstString(.url)
        bearerTokenEnv = try container.decodeFirstString(.bearerTokenEnv, .bearerTokenEnvSnake)
        headers = try container.decodeIfPresent([String: String].self, forKey: .headers) ?? [:]
    }
}

public struct GaryxMcpServerRequest: Encodable, Equatable, Sendable {
    public var name: String
    public var transport: String
    public var command: String
    public var args: [String]
    public var env: [String: String]
    public var enabled: Bool
    public var workingDir: String?
    public var url: String?
    public var bearerTokenEnv: String?
    public var headers: [String: String]

    public init(
        name: String,
        transport: String = "stdio",
        command: String = "",
        args: [String] = [],
        env: [String: String] = [:],
        enabled: Bool = true,
        workingDir: String? = nil,
        url: String? = nil,
        bearerTokenEnv: String? = nil,
        headers: [String: String] = [:]
    ) {
        self.name = name
        self.transport = transport
        self.command = command
        self.args = args
        self.env = env
        self.enabled = enabled
        self.workingDir = workingDir
        self.url = url
        self.bearerTokenEnv = bearerTokenEnv
        self.headers = headers
    }

    enum CodingKeys: String, CodingKey {
        case name
        case transport
        case command
        case args
        case env
        case enabled
        case workingDir = "working_dir"
        case url
        case bearerTokenEnv = "bearer_token_env"
        case headers
    }
}

public struct GaryxMcpServerToggleRequest: Encodable, Equatable, Sendable {
    public var enabled: Bool

    public init(enabled: Bool) {
        self.enabled = enabled
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
        workspaceDir = try container.decodeFirstString(.workspaceDir, .workspaceDirSnake) ?? ""
        isGitRepo = try container.decodeFirstBool(.isGitRepo, .isGitRepoSnake) ?? false
        repoRoot = try container.decodeFirstString(.repoRoot, .repoRootSnake)
        currentBranch = try container.decodeFirstString(.currentBranch, .currentBranchSnake)
        isDirty = try container.decodeFirstBool(.isDirty, .isDirtySnake) ?? false
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
        path = try container.decodeFirstString(.path) ?? ""
        name = try container.decodeFirstString(.name) ?? path.lastPathComponent
        entryType = try container.decodeFirstString(.entryType, .entryTypeSnake) ?? "file"
        size = try container.decodeFirstInt(.size)
        modifiedAt = try container.decodeFirstString(.modifiedAt, .modifiedAtSnake)
        mediaType = try container.decodeFirstString(.mediaType, .mediaTypeSnake)
        hasChildren = try container.decodeFirstBool(.hasChildren, .hasChildrenSnake) ?? false
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
        workspaceDir = try container.decodeFirstString(.workspaceDir, .workspaceDirSnake) ?? ""
        directoryPath = try container.decodeFirstString(.directoryPath, .directoryPathSnake) ?? ""
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
        workspaceDir = try container.decodeFirstString(.workspaceDir, .workspaceDirSnake) ?? ""
        path = try container.decodeFirstString(.path) ?? ""
        name = try container.decodeFirstString(.name) ?? path.lastPathComponent
        mediaType = try container.decodeFirstString(.mediaType, .mediaTypeSnake) ?? "application/octet-stream"
        previewKind = try container.decodeFirstString(.previewKind, .previewKindSnake) ?? "unsupported"
        size = try container.decodeFirstInt(.size) ?? 0
        modifiedAt = try container.decodeFirstString(.modifiedAt, .modifiedAtSnake)
        truncated = try container.decodeFirstBool(.truncated) ?? false
        text = try container.decodeFirstString(.text)
        dataBase64 = try container.decodeFirstString(.dataBase64, .dataBase64Snake)
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
        workspaceDir = try container.decodeFirstString(.workspaceDir, .workspaceDirSnake) ?? ""
        directoryPath = try container.decodeFirstString(.directoryPath, .directoryPathSnake) ?? ""
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
        kind = try container.decodeFirstString(.kind) ?? "file"
        path = try container.decodeFirstString(.path) ?? ""
        name = try container.decodeFirstString(.name) ?? path.lastPathComponent
        mediaType = try container.decodeFirstString(.mediaType, .mediaTypeSnake) ?? ""
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

public struct GaryxAutoResearchRunsPage: Decodable, Equatable, Sendable {
    public var items: [GaryxAutoResearchRun]
}

public struct GaryxAutoResearchRun: Decodable, Identifiable, Equatable, Sendable {
    public var id: String { runId }
    public var runId: String
    public var state: String
    public var goal: String
    public var workspaceDir: String?
    public var maxIterations: Int
    public var timeBudgetSecs: Int?
    public var iterationsUsed: Int
    public var createdAt: String
    public var updatedAt: String
    public var terminalReason: String?
    public var selectedCandidate: String?

    enum CodingKeys: String, CodingKey {
        case runId = "run_id"
        case runIdCamel = "runId"
        case state
        case goal
        case workspaceDir = "workspace_dir"
        case workspaceDirCamel = "workspaceDir"
        case maxIterations = "max_iterations"
        case maxIterationsCamel = "maxIterations"
        case timeBudgetSecs = "time_budget_secs"
        case timeBudgetSecsCamel = "timeBudgetSecs"
        case iterationsUsed = "iterations_used"
        case iterationsUsedCamel = "iterationsUsed"
        case createdAt = "created_at"
        case createdAtCamel = "createdAt"
        case updatedAt = "updated_at"
        case updatedAtCamel = "updatedAt"
        case terminalReason = "terminal_reason"
        case terminalReasonCamel = "terminalReason"
        case selectedCandidate = "selected_candidate"
        case selectedCandidateCamel = "selectedCandidate"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        runId = try container.decodeFirstString(.runId, .runIdCamel) ?? ""
        state = try container.decodeFirstString(.state) ?? "queued"
        goal = try container.decodeFirstString(.goal) ?? ""
        workspaceDir = try container.decodeFirstString(.workspaceDir, .workspaceDirCamel)
        maxIterations = try container.decodeFirstInt(.maxIterations, .maxIterationsCamel) ?? 0
        timeBudgetSecs = try container.decodeFirstInt(.timeBudgetSecs, .timeBudgetSecsCamel)
        iterationsUsed = try container.decodeFirstInt(.iterationsUsed, .iterationsUsedCamel) ?? 0
        createdAt = try container.decodeFirstString(.createdAt, .createdAtCamel) ?? ""
        updatedAt = try container.decodeFirstString(.updatedAt, .updatedAtCamel) ?? ""
        terminalReason = try container.decodeFirstString(.terminalReason, .terminalReasonCamel)
        selectedCandidate = try container.decodeFirstString(.selectedCandidate, .selectedCandidateCamel)
    }
}

public struct GaryxAutoResearchCreateRequest: Encodable, Equatable, Sendable {
    public var goal: String
    public var workspaceDir: String?
    public var maxIterations: Int
    public var timeBudgetSecs: Int?
    public var providerMetadata: [String: String]

    public init(
        goal: String,
        workspaceDir: String? = nil,
        maxIterations: Int = 3,
        timeBudgetSecs: Int? = nil,
        providerMetadata: [String: String] = [:]
    ) {
        self.goal = goal
        self.workspaceDir = workspaceDir
        self.maxIterations = maxIterations
        self.timeBudgetSecs = timeBudgetSecs
        self.providerMetadata = providerMetadata
    }

    enum CodingKeys: String, CodingKey {
        case goal
        case workspaceDir = "workspace_dir"
        case maxIterations = "max_iterations"
        case timeBudgetSecs = "time_budget_secs"
        case providerMetadata = "provider_metadata"
    }
}

public struct GaryxAutoResearchStopRequest: Encodable, Equatable, Sendable {
    public var reason: String?

    public init(reason: String? = nil) {
        self.reason = reason
    }
}

public struct GaryxResearchCandidateVerdict: Decodable, Equatable, Sendable {
    public var score: Double
    public var feedback: String

    enum CodingKeys: String, CodingKey {
        case score
        case feedback
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        if let doubleScore = try? container.decode(Double.self, forKey: .score) {
            score = doubleScore
        } else if let intScore = try? container.decode(Int.self, forKey: .score) {
            score = Double(intScore)
        } else {
            score = 0
        }
        feedback = try container.decodeFirstString(.feedback) ?? ""
    }
}

public struct GaryxResearchCandidate: Decodable, Identifiable, Equatable, Sendable {
    public var id: String { candidateId }
    public var candidateId: String
    public var iteration: Int
    public var output: String
    public var verdict: GaryxResearchCandidateVerdict?
    public var durationSecs: Double

    enum CodingKeys: String, CodingKey {
        case candidateId = "candidate_id"
        case candidateIdCamel = "candidateId"
        case iteration
        case output
        case verdict
        case durationSecs = "duration_secs"
        case durationSecsCamel = "durationSecs"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        candidateId = try container.decodeFirstString(.candidateId, .candidateIdCamel) ?? ""
        iteration = try container.decodeFirstInt(.iteration) ?? 0
        output = try container.decodeFirstString(.output) ?? ""
        verdict = try container.decodeIfPresent(GaryxResearchCandidateVerdict.self, forKey: .verdict)
        if let value = try? container.decode(Double.self, forKey: .durationSecs) {
            durationSecs = value
        } else if let value = try? container.decode(Double.self, forKey: .durationSecsCamel) {
            durationSecs = value
        } else if let value = try? container.decode(Int.self, forKey: .durationSecs) {
            durationSecs = Double(value)
        } else {
            durationSecs = 0
        }
    }
}

public struct GaryxAutoResearchCandidatesPage: Decodable, Equatable, Sendable {
    public var candidates: [GaryxResearchCandidate]
    public var bestCandidateId: String?

    enum CodingKeys: String, CodingKey {
        case candidates
        case bestCandidateId
        case bestCandidateIdSnake = "best_candidate_id"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        candidates = try container.decodeIfPresent([GaryxResearchCandidate].self, forKey: .candidates) ?? []
        bestCandidateId = try container.decodeFirstString(.bestCandidateId, .bestCandidateIdSnake)
    }
}

public struct GaryxAutoResearchFeedbackRequest: Encodable, Equatable, Sendable {
    public var message: String

    public init(message: String) {
        self.message = message
    }

    enum CodingKeys: String, CodingKey {
        case message
    }
}

public struct GaryxAutoResearchReverifyRequest: Encodable, Equatable, Sendable {
    public var candidateId: String

    public init(candidateId: String) {
        self.candidateId = candidateId
    }

    enum CodingKeys: String, CodingKey {
        case candidateId = "candidate_id"
    }
}

public struct GaryxAutoResearchIteration: Decodable, Identifiable, Equatable, Sendable {
    public var id: String { "\(runId):\(iterationIndex)" }
    public var runId: String
    public var iterationIndex: Int
    public var state: String
    public var workThreadId: String?
    public var verifyThreadId: String?
    public var startedAt: String
    public var completedAt: String?

    enum CodingKeys: String, CodingKey {
        case runId = "run_id"
        case runIdCamel = "runId"
        case iterationIndex = "iteration_index"
        case iterationIndexCamel = "iterationIndex"
        case state
        case workThreadId = "work_thread_id"
        case workThreadIdCamel = "workThreadId"
        case verifyThreadId = "verify_thread_id"
        case verifyThreadIdCamel = "verifyThreadId"
        case startedAt = "started_at"
        case startedAtCamel = "startedAt"
        case completedAt = "completed_at"
        case completedAtCamel = "completedAt"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        runId = try container.decodeFirstString(.runId, .runIdCamel) ?? ""
        iterationIndex = try container.decodeFirstInt(.iterationIndex, .iterationIndexCamel) ?? 0
        state = try container.decodeFirstString(.state) ?? ""
        workThreadId = try container.decodeFirstString(.workThreadId, .workThreadIdCamel)
        verifyThreadId = try container.decodeFirstString(.verifyThreadId, .verifyThreadIdCamel)
        startedAt = try container.decodeFirstString(.startedAt, .startedAtCamel) ?? ""
        completedAt = try container.decodeFirstString(.completedAt, .completedAtCamel)
    }
}

public struct GaryxAutoResearchIterationsPage: Decodable, Equatable, Sendable {
    public var items: [GaryxAutoResearchIteration]
}

public struct GaryxAutoResearchDetail: Decodable, Equatable, Sendable {
    public var run: GaryxAutoResearchRun
    public var latestIteration: GaryxAutoResearchIteration?
    public var activeThreadId: String?

    enum CodingKeys: String, CodingKey {
        case run
        case latestIteration = "latest_iteration"
        case latestIterationCamel = "latestIteration"
        case activeThreadId = "active_thread_id"
        case activeThreadIdCamel = "activeThreadId"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        run = try container.decode(GaryxAutoResearchRun.self, forKey: .run)
        latestIteration = try container.decodeIfPresent(GaryxAutoResearchIteration.self, forKey: .latestIteration)
            ?? container.decodeIfPresent(GaryxAutoResearchIteration.self, forKey: .latestIterationCamel)
        activeThreadId = try container.decodeFirstString(.activeThreadId, .activeThreadIdCamel)
    }
}

public struct GaryxChannelEndpointsPage: Decodable, Equatable, Sendable {
    public var endpoints: [GaryxChannelEndpoint]
}

public struct GaryxChannelEndpoint: Decodable, Identifiable, Equatable, Sendable {
    public var id: String { endpointKey }
    public var endpointKey: String
    public var channel: String
    public var accountId: String
    public var displayLabel: String
    public var threadId: String?
    public var threadLabel: String?
    public var workspaceDir: String?
    public var lastInboundAt: String?
    public var lastDeliveryAt: String?
    public var conversationKind: String?
    public var conversationLabel: String?

    enum CodingKeys: String, CodingKey {
        case endpointKey = "endpoint_key"
        case endpointKeyCamel = "endpointKey"
        case channel
        case accountId = "account_id"
        case accountIdCamel = "accountId"
        case displayLabel = "display_label"
        case displayLabelCamel = "displayLabel"
        case threadId = "thread_id"
        case threadIdCamel = "threadId"
        case threadLabel = "thread_label"
        case threadLabelCamel = "threadLabel"
        case workspaceDir = "workspace_dir"
        case workspaceDirCamel = "workspaceDir"
        case lastInboundAt = "last_inbound_at"
        case lastInboundAtCamel = "lastInboundAt"
        case lastDeliveryAt = "last_delivery_at"
        case lastDeliveryAtCamel = "lastDeliveryAt"
        case conversationKind = "conversation_kind"
        case conversationKindCamel = "conversationKind"
        case conversationLabel = "conversation_label"
        case conversationLabelCamel = "conversationLabel"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        endpointKey = try container.decodeFirstString(.endpointKey, .endpointKeyCamel) ?? ""
        channel = try container.decodeFirstString(.channel) ?? ""
        accountId = try container.decodeFirstString(.accountId, .accountIdCamel) ?? ""
        displayLabel = try container.decodeFirstString(.displayLabel, .displayLabelCamel) ?? endpointKey
        threadId = try container.decodeFirstString(.threadId, .threadIdCamel)
        threadLabel = try container.decodeFirstString(.threadLabel, .threadLabelCamel)
        workspaceDir = try container.decodeFirstString(.workspaceDir, .workspaceDirCamel)
        lastInboundAt = try container.decodeFirstString(.lastInboundAt, .lastInboundAtCamel)
        lastDeliveryAt = try container.decodeFirstString(.lastDeliveryAt, .lastDeliveryAtCamel)
        conversationKind = try container.decodeFirstString(.conversationKind, .conversationKindCamel)
        conversationLabel = try container.decodeFirstString(.conversationLabel, .conversationLabelCamel)
    }
}

public struct GaryxConfiguredBotsPage: Decodable, Equatable, Sendable {
    public var bots: [GaryxConfiguredBot]
}

public struct GaryxConfiguredBot: Decodable, Identifiable, Equatable, Sendable {
    public var id: String { "\(channel):\(accountId)" }
    public var channel: String
    public var accountId: String
    public var displayName: String
    public var enabled: Bool
    public var agentId: String?
    public var workspaceDir: String?
    public var rootBehavior: String
    public var mainEndpointStatus: String
    public var mainThreadId: String?
    public var defaultOpenThreadId: String?

    enum CodingKeys: String, CodingKey {
        case channel
        case accountId = "account_id"
        case accountIdCamel = "accountId"
        case displayName = "display_name"
        case displayNameCamel = "displayName"
        case name
        case enabled
        case agentId = "agent_id"
        case agentIdCamel = "agentId"
        case workspaceDir = "workspace_dir"
        case workspaceDirCamel = "workspaceDir"
        case rootBehavior = "root_behavior"
        case rootBehaviorCamel = "rootBehavior"
        case mainEndpointStatus = "main_endpoint_status"
        case mainEndpointStatusCamel = "mainEndpointStatus"
        case mainThreadId = "main_endpoint_thread_id"
        case mainThreadIdCamel = "mainEndpointThreadId"
        case defaultOpenThreadId = "default_open_thread_id"
        case defaultOpenThreadIdCamel = "defaultOpenThreadId"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        channel = try container.decodeFirstString(.channel) ?? ""
        accountId = try container.decodeFirstString(.accountId, .accountIdCamel) ?? ""
        displayName = try container.decodeFirstString(.displayName, .displayNameCamel, .name) ?? "\(channel):\(accountId)"
        enabled = try container.decodeIfPresent(Bool.self, forKey: .enabled) ?? true
        agentId = try container.decodeFirstString(.agentId, .agentIdCamel)
        workspaceDir = try container.decodeFirstString(.workspaceDir, .workspaceDirCamel)
        rootBehavior = try container.decodeFirstString(.rootBehavior, .rootBehaviorCamel) ?? "open_default"
        mainEndpointStatus = try container.decodeFirstString(.mainEndpointStatus, .mainEndpointStatusCamel) ?? "unresolved"
        mainThreadId = try container.decodeFirstString(.mainThreadId, .mainThreadIdCamel)
        defaultOpenThreadId = try container.decodeFirstString(.defaultOpenThreadId, .defaultOpenThreadIdCamel)
    }
}

public struct GaryxBotConsolesPage: Decodable, Equatable, Sendable {
    public var bots: [GaryxBotConsoleSummary]
}

public struct GaryxBotConversationNode: Decodable, Identifiable, Equatable, Sendable {
    public var id: String
    public var endpoint: GaryxChannelEndpoint
    public var kind: String
    public var title: String
    public var badge: String?
    public var latestActivity: String?
    public var openable: Bool

    enum CodingKeys: String, CodingKey {
        case id
        case endpoint
        case kind
        case title
        case badge
        case latestActivity = "latest_activity"
        case latestActivityCamel = "latestActivity"
        case openable
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decodeFirstString(.id) ?? ""
        endpoint = try container.decode(GaryxChannelEndpoint.self, forKey: .endpoint)
        kind = try container.decodeFirstString(.kind) ?? ""
        title = try container.decodeFirstString(.title) ?? endpoint.displayLabel
        badge = try container.decodeFirstString(.badge)
        latestActivity = try container.decodeFirstString(.latestActivity, .latestActivityCamel)
        openable = try container.decodeIfPresent(Bool.self, forKey: .openable) ?? false
    }
}

public struct GaryxBotConsoleSummary: Decodable, Identifiable, Equatable, Sendable {
    public var id: String
    public var channel: String
    public var accountId: String
    public var title: String
    public var subtitle: String
    public var agentId: String?
    public var rootBehavior: String
    public var status: String
    public var latestActivity: String?
    public var endpointCount: Int
    public var boundEndpointCount: Int
    public var workspaceDir: String?
    public var mainThreadId: String?
    public var defaultOpenThreadId: String?
    public var conversationNodes: [GaryxBotConversationNode]

    enum CodingKeys: String, CodingKey {
        case id
        case channel
        case accountId = "account_id"
        case accountIdCamel = "accountId"
        case title
        case subtitle
        case agentId = "agent_id"
        case agentIdCamel = "agentId"
        case rootBehavior = "root_behavior"
        case rootBehaviorCamel = "rootBehavior"
        case status
        case latestActivity = "latest_activity"
        case latestActivityCamel = "latestActivity"
        case endpointCount = "endpoint_count"
        case endpointCountCamel = "endpointCount"
        case boundEndpointCount = "bound_endpoint_count"
        case boundEndpointCountCamel = "boundEndpointCount"
        case workspaceDir = "workspace_dir"
        case workspaceDirCamel = "workspaceDir"
        case mainThreadId = "main_endpoint_thread_id"
        case mainThreadIdCamel = "mainEndpointThreadId"
        case defaultOpenThreadId = "default_open_thread_id"
        case defaultOpenThreadIdCamel = "defaultOpenThreadId"
        case conversationNodes = "conversation_nodes"
        case conversationNodesCamel = "conversationNodes"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decodeFirstString(.id) ?? ""
        channel = try container.decodeFirstString(.channel) ?? ""
        accountId = try container.decodeFirstString(.accountId, .accountIdCamel) ?? ""
        title = try container.decodeFirstString(.title) ?? id
        subtitle = try container.decodeFirstString(.subtitle) ?? ""
        agentId = try container.decodeFirstString(.agentId, .agentIdCamel)
        rootBehavior = try container.decodeFirstString(.rootBehavior, .rootBehaviorCamel) ?? "open_default"
        status = try container.decodeFirstString(.status) ?? "idle"
        latestActivity = try container.decodeFirstString(.latestActivity, .latestActivityCamel)
        endpointCount = try container.decodeFirstInt(.endpointCount, .endpointCountCamel) ?? 0
        boundEndpointCount = try container.decodeFirstInt(.boundEndpointCount, .boundEndpointCountCamel) ?? 0
        workspaceDir = try container.decodeFirstString(.workspaceDir, .workspaceDirCamel)
        mainThreadId = try container.decodeFirstString(.mainThreadId, .mainThreadIdCamel)
        defaultOpenThreadId = try container.decodeFirstString(.defaultOpenThreadId, .defaultOpenThreadIdCamel)
        let snakeConversationNodes = try container.decodeIfPresent(
            [GaryxBotConversationNode].self,
            forKey: .conversationNodes
        )
        let camelConversationNodes = try container.decodeIfPresent(
            [GaryxBotConversationNode].self,
            forKey: .conversationNodesCamel
        )
        conversationNodes = snakeConversationNodes ?? camelConversationNodes ?? []
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
        providerType = try container.decodeFirstString(.providerType)
        providerLabel = try container.decodeFirstString(.providerLabel)
        sdkSessionId = try container.decodeFirstString(.sdkSessionId)
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
        runId = try container.decodeFirstString(.runId)
        providerType = try container.decodeFirstString(.providerType)
        providerLabel = try container.decodeFirstString(.providerLabel)
        assistantResponse = try container.decodeFirstString(.assistantResponse)
        updatedAt = try container.decodeFirstString(.updatedAt)
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
        id = try container.decodeFirstString(.id) ?? ""
        runId = try container.decodeFirstString(.runId)
        text = try container.decodeFirstString(.text) ?? ""
        content = try container.decodeIfPresent(GaryxJSONValue.self, forKey: .content)
        timestamp = try container.decodeFirstString(.timestamp)
        status = try container.decodeFirstString(.status)
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

public struct GaryxDeleteResult: Decodable, Equatable, Sendable {
    public var deleted: Bool?
    public var id: String?
    public var taskId: String?
    public var threadId: String?

    enum CodingKeys: String, CodingKey {
        case deleted
        case id
        case taskId = "task_id"
        case taskIdCamel = "taskId"
        case threadId = "thread_id"
        case threadIdCamel = "threadId"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        deleted = try container.decodeIfPresent(Bool.self, forKey: .deleted)
        id = try container.decodeFirstString(.id)
        taskId = try container.decodeFirstString(.taskId, .taskIdCamel)
        threadId = try container.decodeFirstString(.threadId, .threadIdCamel)
    }
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
        threadId = try container.decodeFirstString(.threadId, .threadIdSnake) ?? ""
        path = try container.decodeFirstString(.path) ?? ""
        text = try container.decodeFirstString(.text) ?? ""
        cursor = try container.decodeFirstInt(.cursor) ?? 0
        reset = try container.decodeFirstBool(.reset) ?? true
    }
}

public struct GaryxEmptyResponse: Decodable, Equatable, Sendable {
    public init() {}
    public init(from decoder: Decoder) throws {
        self.init()
    }
}

public struct GaryxTaskEnvelope: Decodable, Equatable, Sendable {
    public var task: GaryxTaskSummary?
}

public struct GaryxAutomationEnabledPatch: Encodable, Equatable, Sendable {
    public var enabled: Bool
}

public struct GaryxEmptyBody: Encodable, Equatable, Sendable {
    public init() {}
}

public struct GaryxInterruptRequest: Encodable, Equatable, Sendable {
    public var threadId: String

    public init(threadId: String) {
        self.threadId = threadId
    }
}

public struct GaryxStreamInputRequest: Encodable, Equatable, Sendable {
    public var threadId: String
    public var clientIntentId: String?
    public var message: String
    public var attachments: [GaryxPromptAttachment]

    public init(
        threadId: String,
        clientIntentId: String? = nil,
        message: String,
        attachments: [GaryxPromptAttachment] = []
    ) {
        self.threadId = threadId
        self.clientIntentId = clientIntentId
        self.message = message
        self.attachments = attachments
    }
}

public struct GaryxStreamInputResult: Decodable, Equatable, Sendable {
    public var status: String
    public var threadStatus: String?
    public var clientIntentId: String?
    public var pendingInputId: String?
    public var threadId: String

    enum CodingKeys: String, CodingKey {
        case status
        case threadStatus
        case threadStatusSnake = "thread_status"
        case clientIntentId
        case clientIntentIdSnake = "client_intent_id"
        case pendingInputId
        case pendingInputIdSnake = "pending_input_id"
        case threadId
        case threadIdSnake = "thread_id"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        status = try container.decodeFirstString(.status) ?? ""
        threadStatus = try container.decodeFirstString(.threadStatus, .threadStatusSnake)
        clientIntentId = try container.decodeFirstString(.clientIntentId, .clientIntentIdSnake)
        pendingInputId = try container.decodeFirstString(.pendingInputId, .pendingInputIdSnake)
        threadId = try container.decodeFirstString(.threadId, .threadIdSnake) ?? ""
    }
}

public struct GaryxInterruptResult: Decodable, Equatable, Sendable {
    public var status: String
    public var threadId: String
    public var abortedRuns: [String]

    enum CodingKeys: String, CodingKey {
        case status
        case threadId
        case threadIdSnake = "thread_id"
        case abortedRuns
        case abortedRunsSnake = "aborted_runs"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        status = try container.decodeFirstString(.status) ?? ""
        threadId = try container.decodeFirstString(.threadId, .threadIdSnake) ?? ""
        abortedRuns = try container.decodeIfPresent([String].self, forKey: .abortedRuns)
            ?? container.decodeIfPresent([String].self, forKey: .abortedRunsSnake)
            ?? []
    }
}

public struct GaryxGatewaySettingsSaveResult: Decodable, Equatable, Sendable {
    public var ok: Bool
    public var message: String?
    public var warnings: [String]
    public var errors: [String]

    enum CodingKeys: String, CodingKey {
        case ok
        case message
        case warnings
        case errors
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        ok = try container.decodeFirstBool(.ok) ?? false
        message = try container.decodeFirstString(.message)
        warnings = try container.decodeIfPresent([String].self, forKey: .warnings) ?? []
        errors = try container.decodeIfPresent([String].self, forKey: .errors) ?? []
    }
}

public struct GaryxChannelPluginCatalogPage: Decodable, Equatable, Sendable {
    public var plugins: [GaryxChannelPluginCatalogEntry]
}

public struct GaryxChannelPluginConfigMethod: Decodable, Equatable, Sendable {
    public var kind: String
    public var title: String?
    public var description: String?

    enum CodingKeys: String, CodingKey {
        case kind
        case title
        case description
    }
}

public struct GaryxChannelPluginCatalogEntry: Decodable, Identifiable, Equatable, Sendable {
    public var id: String
    public var displayName: String
    public var description: String?
    public var iconDataUrl: String?
    public var schema: [String: GaryxJSONValue]
    public var configMethods: [GaryxChannelPluginConfigMethod]

    enum CodingKeys: String, CodingKey {
        case id
        case displayName = "display_name"
        case displayNameCamel = "displayName"
        case description
        case iconDataUrl = "icon_data_url"
        case iconDataUrlCamel = "iconDataUrl"
        case schema
        case configMethods = "config_methods"
        case configMethodsCamel = "configMethods"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decodeFirstString(.id) ?? ""
        displayName = try container.decodeFirstString(.displayName, .displayNameCamel) ?? id
        description = try container.decodeFirstString(.description)
        iconDataUrl = try container.decodeFirstString(.iconDataUrl, .iconDataUrlCamel)
        schema = try container.decodeIfPresent([String: GaryxJSONValue].self, forKey: .schema) ?? [:]
        configMethods = try container.decodeIfPresent([GaryxChannelPluginConfigMethod].self, forKey: .configMethods)
            ?? container.decodeIfPresent([GaryxChannelPluginConfigMethod].self, forKey: .configMethodsCamel)
            ?? []
    }
}

public struct GaryxChannelAuthStartRequest: Encodable, Equatable, Sendable {
    public var formState: [String: GaryxJSONValue]

    public init(formState: [String: GaryxJSONValue] = [:]) {
        self.formState = formState
    }

    enum CodingKeys: String, CodingKey {
        case formState = "form_state"
    }
}

public struct GaryxChannelAuthSession: Decodable, Equatable, Sendable {
    public var sessionId: String
    public var display: [GaryxJSONValue]
    public var expiresInSecs: Int
    public var pollIntervalSecs: Int

    enum CodingKeys: String, CodingKey {
        case sessionId = "session_id"
        case sessionIdCamel = "sessionId"
        case display
        case expiresInSecs = "expires_in_secs"
        case expiresInSecsCamel = "expiresInSecs"
        case pollIntervalSecs = "poll_interval_secs"
        case pollIntervalSecsCamel = "pollIntervalSecs"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        sessionId = try container.decodeFirstString(.sessionId, .sessionIdCamel) ?? ""
        display = try container.decodeIfPresent([GaryxJSONValue].self, forKey: .display) ?? []
        expiresInSecs = try container.decodeFirstInt(.expiresInSecs, .expiresInSecsCamel) ?? 0
        pollIntervalSecs = max(1, try container.decodeFirstInt(.pollIntervalSecs, .pollIntervalSecsCamel) ?? 5)
    }
}

public struct GaryxChannelAuthPollRequest: Encodable, Equatable, Sendable {
    public var sessionId: String

    public init(sessionId: String) {
        self.sessionId = sessionId
    }

    enum CodingKeys: String, CodingKey {
        case sessionId = "session_id"
    }
}

public struct GaryxChannelAuthPollResult: Decodable, Equatable, Sendable {
    public var status: String
    public var display: [GaryxJSONValue]
    public var nextIntervalSecs: Int?
    public var values: [String: GaryxJSONValue]?
    public var reason: String?

    enum CodingKeys: String, CodingKey {
        case status
        case display
        case nextIntervalSecs = "next_interval_secs"
        case nextIntervalSecsCamel = "nextIntervalSecs"
        case values
        case reason
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        status = try container.decodeFirstString(.status) ?? ""
        display = try container.decodeIfPresent([GaryxJSONValue].self, forKey: .display) ?? []
        nextIntervalSecs = try container.decodeFirstInt(.nextIntervalSecs, .nextIntervalSecsCamel)
        values = try container.decodeIfPresent([String: GaryxJSONValue].self, forKey: .values)
        reason = try container.decodeFirstString(.reason)
    }
}

public struct GaryxChannelAccountValidationRequest: Encodable, Equatable, Sendable {
    public var accountId: String
    public var enabled: Bool
    public var config: [String: GaryxJSONValue]

    public init(accountId: String, enabled: Bool = true, config: [String: GaryxJSONValue]) {
        self.accountId = accountId
        self.enabled = enabled
        self.config = config
    }

    enum CodingKeys: String, CodingKey {
        case accountId = "account_id"
        case enabled
        case config
    }
}

public struct GaryxChannelAccountValidationResult: Decodable, Equatable, Sendable {
    public var validated: Bool
    public var message: String
}

public struct GaryxBotBindingRequest: Encodable, Equatable, Sendable {
    public var botId: String
    public var threadId: String?

    public init(botId: String, threadId: String? = nil) {
        self.botId = botId
        self.threadId = threadId
    }
}

public struct GaryxBotBindingResult: Decodable, Equatable, Sendable {
    public var ok: Bool
    public var botId: String
    public var channel: String
    public var accountId: String
    public var workspaceMode: String?
    public var mainEndpointStatus: String
    public var currentThreadStatus: String
    public var currentThreadId: String?
    public var action: String?
    public var threadId: String?
    public var previousThreadId: String?
    public var endpointKey: String?
    public var error: String?
    public var reason: String?

    enum CodingKeys: String, CodingKey {
        case ok
        case botId = "bot_id"
        case botIdCamel = "botId"
        case channel
        case accountId = "account_id"
        case accountIdCamel = "accountId"
        case workspaceMode = "workspace_mode"
        case workspaceModeCamel = "workspaceMode"
        case mainEndpointStatus = "main_endpoint_status"
        case mainEndpointStatusCamel = "mainEndpointStatus"
        case currentThreadStatus = "current_thread_status"
        case currentThreadStatusCamel = "currentThreadStatus"
        case currentThreadId = "current_thread_id"
        case currentThreadIdCamel = "currentThreadId"
        case action
        case threadId = "thread_id"
        case threadIdCamel = "threadId"
        case previousThreadId = "previous_thread_id"
        case previousThreadIdCamel = "previousThreadId"
        case endpointKey = "endpoint_key"
        case endpointKeyCamel = "endpointKey"
        case error
        case reason
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        ok = try container.decodeFirstBool(.ok) ?? false
        botId = try container.decodeFirstString(.botId, .botIdCamel) ?? ""
        channel = try container.decodeFirstString(.channel) ?? ""
        accountId = try container.decodeFirstString(.accountId, .accountIdCamel) ?? ""
        workspaceMode = try container.decodeFirstString(.workspaceMode, .workspaceModeCamel)
        mainEndpointStatus = try container.decodeFirstString(.mainEndpointStatus, .mainEndpointStatusCamel) ?? "unknown"
        currentThreadStatus = try container.decodeFirstString(.currentThreadStatus, .currentThreadStatusCamel) ?? "unknown"
        currentThreadId = try container.decodeFirstString(.currentThreadId, .currentThreadIdCamel)
        action = try container.decodeFirstString(.action)
        threadId = try container.decodeFirstString(.threadId, .threadIdCamel)
        previousThreadId = try container.decodeFirstString(.previousThreadId, .previousThreadIdCamel)
        endpointKey = try container.decodeFirstString(.endpointKey, .endpointKeyCamel)
        error = try container.decodeFirstString(.error)
        reason = try container.decodeFirstString(.reason)
    }
}

public struct GaryxChannelEndpointBindRequest: Encodable, Equatable, Sendable {
    public var endpointKey: String
    public var threadId: String

    public init(endpointKey: String, threadId: String) {
        self.endpointKey = endpointKey
        self.threadId = threadId
    }
}

public struct GaryxChannelEndpointDetachRequest: Encodable, Equatable, Sendable {
    public var endpointKey: String

    public init(endpointKey: String) {
        self.endpointKey = endpointKey
    }
}

public struct GaryxChatWebSocketCommand: Encodable, Equatable, Sendable {
    public var op: String
    public var threadId: String?
    public var message: String?
    public var clientIntentId: String?
    public var attachments: [GaryxPromptAttachment]?
    public var images: [GaryxInlineImagePayload]?
    public var files: [GaryxInlineFilePayload]?
    public var accountId: String?
    public var fromId: String?
    public var waitForResponse: Bool?
    public var workspacePath: String?
    public var limit: Int?
    public var metadata: [String: String]

    public static func start(
        threadId: String,
        message: String,
        accountId: String = "main",
        fromId: String = "mobile",
        waitForResponse: Bool = false,
        workspacePath: String? = nil,
        attachments: [GaryxPromptAttachment] = [],
        images: [GaryxInlineImagePayload] = [],
        files: [GaryxInlineFilePayload] = [],
        metadata: [String: String] = [:]
    ) -> Self {
        Self(
            op: "start",
            threadId: threadId,
            message: message,
            attachments: attachments.isEmpty ? nil : attachments,
            images: images.isEmpty ? nil : images,
            files: files.isEmpty ? nil : files,
            accountId: accountId,
            fromId: fromId,
            waitForResponse: waitForResponse,
            workspacePath: workspacePath,
            metadata: metadata
        )
    }

    public static func input(
        threadId: String,
        message: String,
        clientIntentId: String? = nil,
        attachments: [GaryxPromptAttachment] = [],
        images: [GaryxInlineImagePayload] = [],
        files: [GaryxInlineFilePayload] = []
    ) -> Self {
        Self(
            op: "input",
            threadId: threadId,
            message: message,
            clientIntentId: clientIntentId,
            attachments: attachments.isEmpty ? nil : attachments,
            images: images.isEmpty ? nil : images,
            files: files.isEmpty ? nil : files,
            metadata: [:]
        )
    }

    public static func recover(threadId: String, limit: Int = 200) -> Self {
        Self(
            op: "recover",
            threadId: threadId,
            limit: limit,
            metadata: [:]
        )
    }

    public static func interrupt(threadId: String) -> Self {
        Self(
            op: "interrupt",
            threadId: threadId,
            metadata: [:]
        )
    }
}

public enum GaryxJSONValue: Codable, Equatable, Sendable {
    case string(String)
    case number(Double)
    case bool(Bool)
    case object([String: GaryxJSONValue])
    case array([GaryxJSONValue])
    case null

    public init(from decoder: Decoder) throws {
        let single = try decoder.singleValueContainer()
        if single.decodeNil() {
            self = .null
        } else if let value = try? single.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? single.decode(Double.self) {
            self = .number(value)
        } else if let value = try? single.decode(String.self) {
            self = .string(value)
        } else if let value = try? single.decode([GaryxJSONValue].self) {
            self = .array(value)
        } else {
            self = .object(try single.decode([String: GaryxJSONValue].self))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var single = encoder.singleValueContainer()
        switch self {
        case .string(let value):
            try single.encode(value)
        case .number(let value):
            try single.encode(value)
        case .bool(let value):
            try single.encode(value)
        case .object(let value):
            try single.encode(value)
        case .array(let value):
            try single.encode(value)
        case .null:
            try single.encodeNil()
        }
    }
}

public struct GaryxContentAttachmentDescriptor: Equatable, Sendable {
    public var id: String
    public var kind: String
    public var name: String
    public var mediaType: String
    public var path: String?
    public var dataUrl: String?
    public var remoteUrl: String?

    public init(
        id: String,
        kind: String,
        name: String,
        mediaType: String,
        path: String? = nil,
        dataUrl: String? = nil,
        remoteUrl: String? = nil
    ) {
        self.id = id
        self.kind = kind
        self.name = name
        self.mediaType = mediaType
        self.path = path
        self.dataUrl = dataUrl
        self.remoteUrl = remoteUrl
    }

    public var isImage: Bool {
        kind.caseInsensitiveCompare("image") == .orderedSame || mediaType.hasPrefix("image/")
    }
}

public enum GaryxStructuredContentRenderer {
    public static func attachments(from content: GaryxJSONValue?) -> [GaryxContentAttachmentDescriptor] {
        guard let content else { return [] }
        var attachments: [GaryxContentAttachmentDescriptor] = []

        @discardableResult
        func appendAttachment(from object: [String: GaryxJSONValue], fallbackIndex: Int) -> Bool {
            let type = object.stringValue(forKeys: ["type", "kind"])?.lowercased() ?? ""
            let mediaType = object.stringValue(forKeys: ["media_type", "mediaType"])
                ?? object.objectValue(forKeys: ["source"])?.stringValue(forKeys: ["media_type", "mediaType"])
                ?? ""
            let path = object.stringValue(forKeys: ["path", "file_path", "filePath"])
            let name = object.stringValue(forKeys: ["name", "filename", "file_name"])
                ?? path?.lastPathComponent
                ?? (type.contains("image") || mediaType.hasPrefix("image/") ? "Image" : "Attachment")
            let source = object.objectValue(forKeys: ["source"])
            let base64 = source?.stringValue(forKeys: ["data"])
                ?? object.stringValue(forKeys: ["data", "base64"])
            let attachmentDataUrl: String?
            if let base64 {
                attachmentDataUrl = base64.hasPrefix("data:")
                    ? base64
                    : makeDataUrl(mediaType: mediaType.isEmpty ? "image/jpeg" : mediaType, base64: base64)
            } else {
                attachmentDataUrl = nil
            }
            let remoteUrl = object.stringValue(forKeys: ["url", "image_url", "imageUrl"])
                ?? source?.stringValue(forKeys: ["url"])
            let isImage = type.contains("image")
                || mediaType.hasPrefix("image/")
                || attachmentDataUrl != nil
                || remoteUrl != nil
            guard isImage || type == "file" || type == "attachment" || path != nil else { return false }
            let attachmentIdBase = object.stringValue(forKeys: ["id"])
                ?? path
                ?? remoteUrl
                ?? (type.isEmpty ? "attachment" : type)
            attachments.append(
                GaryxContentAttachmentDescriptor(
                    id: "\(attachmentIdBase)-\(fallbackIndex)",
                    kind: isImage ? "image" : "file",
                    name: name,
                    mediaType: mediaType,
                    path: path,
                    dataUrl: attachmentDataUrl,
                    remoteUrl: remoteUrl
                )
            )
            return true
        }

        func inspect(_ value: GaryxJSONValue) {
            switch value.jsonStringDecodedIfNeeded {
            case .array(let items):
                items.forEach(inspect)
            case .object(let object):
                if appendAttachment(from: object, fallbackIndex: attachments.count) {
                    return
                }
                if let nested = object["content"]?.jsonStringDecodedIfNeeded {
                    inspect(nested)
                }
            case .string, .number, .bool, .null:
                break
            }
        }

        inspect(content)
        return attachments
    }

    public static func text(from value: GaryxJSONValue) -> String? {
        var parts: [String] = []

        func inspect(_ value: GaryxJSONValue) {
            switch value.jsonStringDecodedIfNeeded {
            case .string(let text):
                let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
                if !trimmed.isEmpty {
                    parts.append(trimmed)
                }
            case .array(let items):
                items.forEach(inspect)
            case .object(let object):
                let type = object.stringValue(forKeys: ["type", "kind"])?.lowercased() ?? ""
                if type == "text" || type == "input_text" {
                    if let text = object.stringValue(forKeys: ["text", "content"]) {
                        parts.append(text.trimmingCharacters(in: .whitespacesAndNewlines))
                    }
                    return
                }
                if let nested = object["content"]?.jsonStringDecodedIfNeeded,
                   type != "image",
                   type != "file" {
                    inspect(nested)
                }
            case .number, .bool, .null:
                break
            }
        }

        inspect(value)
        let text = parts.joined(separator: "\n\n").trimmingCharacters(in: .whitespacesAndNewlines)
        return text.isEmpty ? nil : text
    }

    public static func summaryText(from value: GaryxJSONValue) -> String? {
        let text = text(from: value)
        let attachments = attachments(from: value)
        guard let summary = attachmentSummary(from: attachments) else {
            return text
        }
        if let text, !text.isEmpty {
            return "\(text)\n\n\(summary)"
        }
        return summary
    }

    public static func userMergeKey(
        text: String,
        attachments: [GaryxContentAttachmentDescriptor]
    ) -> String {
        let normalizedText = normalizedMergeText(text)
        guard !attachments.isEmpty,
              let attachmentSummary = attachmentSummary(from: attachments) else {
            return normalizedText
        }
        if normalizedText.isEmpty || normalizedText == attachmentSummary {
            return attachmentSummary
        }
        return normalizedText
    }

    public static func attachmentSummary(from attachments: [GaryxContentAttachmentDescriptor]) -> String? {
        let imageCount = attachments.filter(\.isImage).count
        let fileCount = max(attachments.count - imageCount, 0)
        return attachmentSummary(imageCount: imageCount, fileCount: fileCount)
    }

    public static func attachmentSummary(imageCount: Int, fileCount: Int) -> String? {
        var parts: [String] = []
        if imageCount > 0 {
            parts.append("\(imageCount) image\(imageCount == 1 ? "" : "s")")
        }
        if fileCount > 0 {
            parts.append("\(fileCount) file\(fileCount == 1 ? "" : "s")")
        }
        return parts.isEmpty ? nil : "[\(parts.joined(separator: ", "))]"
    }

    private static func normalizedMergeText(_ text: String) -> String {
        text.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "\r\n", with: "\n")
    }

    private static func makeDataUrl(mediaType: String, base64: String) -> String {
        let normalizedType = mediaType.trimmingCharacters(in: .whitespacesAndNewlines)
        let type = normalizedType.isEmpty ? "application/octet-stream" : normalizedType
        return "data:\(type);base64,\(base64)"
    }
}

public enum GaryxChatStreamEvent: Decodable, Equatable, Sendable {
    case ping
    case accepted(runId: String, threadId: String)
    case assistantDelta(runId: String, threadId: String, delta: String, metadata: [String: GaryxJSONValue]?)
    case assistantBoundary(runId: String, threadId: String)
    case toolUse(runId: String, threadId: String, message: GaryxJSONValue?)
    case toolResult(runId: String, threadId: String, message: GaryxJSONValue?)
    case userMessage(runId: String, threadId: String, text: String, imageCount: Int)
    case userAck(runId: String, threadId: String, pendingInputId: String?)
    case threadTitleUpdated(runId: String, threadId: String, title: String)
    case done(runId: String, threadId: String)
    case runComplete(runId: String, threadId: String)
    case streamInput(status: String, threadId: String, clientIntentId: String?, pendingInputId: String?)
    case interrupt(status: String, threadId: String, abortedRuns: [String])
    case snapshot(threadId: String, payload: [String: GaryxJSONValue])
    case error(runId: String, threadId: String, error: String)
    case unknown(type: String, payload: [String: GaryxJSONValue])

    enum CodingKeys: String, CodingKey {
        case type
        case runId
        case run_id
        case threadId
        case thread_id
        case sessionKey
        case delta
        case metadata
        case message
        case text
        case imageCount = "imageCount"
        case imageCountSnake = "image_count"
        case pendingInputId
        case pending_input_id
        case clientIntentId
        case client_intent_id
        case status
        case abortedRuns
        case aborted_runs
        case error
        case title
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let type = try container.decodeFirstString(.type) ?? ""
        let payload = (try? GaryxJSONValue(from: decoder).objectValue) ?? [:]
        let runId = try container.decodeFirstString(.runId, .run_id) ?? ""
        let threadId = try container.decodeFirstString(.threadId, .thread_id, .sessionKey) ?? ""

        switch type {
        case "", "ping":
            self = .ping
        case "accepted":
            self = .accepted(runId: runId, threadId: threadId)
        case "assistant_delta":
            let metadata = try container.decodeIfPresent([String: GaryxJSONValue].self, forKey: .metadata)
            self = .assistantDelta(
                runId: runId,
                threadId: threadId,
                delta: try container.decodeFirstString(.delta) ?? "",
                metadata: metadata
            )
        case "assistant_boundary":
            self = .assistantBoundary(runId: runId, threadId: threadId)
        case "tool_use":
            self = .toolUse(
                runId: runId,
                threadId: threadId,
                message: try container.decodeIfPresent(GaryxJSONValue.self, forKey: .message)
            )
        case "tool_result":
            self = .toolResult(
                runId: runId,
                threadId: threadId,
                message: try container.decodeIfPresent(GaryxJSONValue.self, forKey: .message)
            )
        case "user_message":
            self = .userMessage(
                runId: runId,
                threadId: threadId,
                text: try container.decodeFirstString(.text, .message) ?? "",
                imageCount: try container.decodeIfPresent(Int.self, forKey: .imageCount)
                    ?? container.decodeIfPresent(Int.self, forKey: .imageCountSnake)
                    ?? 0
            )
        case "user_ack":
            self = .userAck(
                runId: runId,
                threadId: threadId,
                pendingInputId: try container.decodeFirstString(.pendingInputId, .pending_input_id)
            )
        case "thread_title_updated":
            self = .threadTitleUpdated(
                runId: runId,
                threadId: threadId,
                title: try container.decodeFirstString(.title) ?? ""
            )
        case "done":
            self = .done(runId: runId, threadId: threadId)
        case "run_complete":
            self = .runComplete(runId: runId, threadId: threadId)
        case "stream_input":
            self = .streamInput(
                status: try container.decodeFirstString(.status) ?? "",
                threadId: threadId,
                clientIntentId: try container.decodeFirstString(.clientIntentId, .client_intent_id),
                pendingInputId: try container.decodeFirstString(.pendingInputId, .pending_input_id)
            )
        case "interrupt":
            let abortedRuns = try container.decodeIfPresent([String].self, forKey: .abortedRuns)
                ?? container.decodeIfPresent([String].self, forKey: .aborted_runs)
                ?? []
            self = .interrupt(
                status: try container.decodeFirstString(.status) ?? "",
                threadId: threadId,
                abortedRuns: abortedRuns
            )
        case "snapshot":
            self = .snapshot(threadId: threadId, payload: payload)
        case "error":
            self = .error(
                runId: runId,
                threadId: threadId,
                error: try container.decodeFirstString(.error) ?? "agent run failed"
            )
        default:
            self = .unknown(type: type, payload: payload)
        }
    }
}

public final class GaryxGatewayClient {
    public let configuration: GaryxGatewayConfiguration

    private let session: URLSession
    private let encoder: JSONEncoder
    private let decoder: JSONDecoder

    public init(
        configuration: GaryxGatewayConfiguration,
        session: URLSession = .shared,
        encoder: JSONEncoder = JSONEncoder(),
        decoder: JSONDecoder = JSONDecoder()
    ) {
        self.configuration = configuration
        self.session = session
        self.encoder = encoder
        self.decoder = decoder
    }

    static func encodePathSegment(_ value: String) -> String {
        value.addingPercentEncoding(withAllowedCharacters: garyxURLPathSegmentAllowed) ?? value
    }

    public func status() async throws -> GaryxSystemStatus {
        try await get("/api/status")
    }

    public func chatHealth() async throws -> GaryxChatHealth {
        try await get("/api/chat/health")
    }

    public func gatewaySettings() async throws -> [String: GaryxJSONValue] {
        try await get("/api/settings")
    }

    public func saveGatewaySettings(
        _ config: [String: GaryxJSONValue],
        merge: Bool = true
    ) async throws -> GaryxGatewaySettingsSaveResult {
        try await put("/api/settings?merge=\(merge ? "true" : "false")", body: config)
    }

    public func listThreads(limit: Int = 100, offset: Int = 0) async throws -> GaryxThreadsPage {
        try await get(
            "/api/threads",
            queryItems: [
                URLQueryItem(name: "limit", value: String(limit)),
                URLQueryItem(name: "offset", value: String(offset)),
            ]
        )
    }

    public func listRecentThreads(limit: Int = 30, offset: Int = 0) async throws -> GaryxRecentThreadsPage {
        try await get(
            "/api/recent-threads",
            queryItems: [
                URLQueryItem(name: "limit", value: String(limit)),
                URLQueryItem(name: "offset", value: String(offset)),
            ]
        )
    }

    public func getThread(threadId: String) async throws -> GaryxThreadSummary {
        try await get("/api/threads/\(threadId.urlPathEncoded)")
    }

    public func listThreadPins() async throws -> GaryxThreadPinsPage {
        try await get("/api/thread-pins")
    }

    public func setThreadPinned(threadId: String, pinned: Bool) async throws -> GaryxThreadPinsPage {
        if pinned {
            return try await put(
                "/api/thread-pins/\(threadId.urlPathEncoded)",
                body: GaryxEmptyBody()
            )
        }
        return try await delete("/api/thread-pins/\(threadId.urlPathEncoded)")
    }

    public func listDreams(sinceHours: Int = 24, limit: Int = 80) async throws -> GaryxDreamsPage {
        try await get(
            "/api/dreams",
            queryItems: [
                URLQueryItem(name: "since_hours", value: String(max(1, sinceHours))),
                URLQueryItem(name: "limit", value: String(max(1, limit))),
            ]
        )
    }

    public func scanDreams(
        request: GaryxDreamScanRequest = GaryxDreamScanRequest()
    ) async throws -> GaryxDreamsPage {
        try await post("/api/dreams/scan", body: request)
    }

    public func threadHistory(
        threadId: String,
        limit: Int = 100,
        beforeIndex: Int? = nil,
        userQueryLimit: Int? = nil,
        includeToolMessages: Bool = true
    ) async throws -> GaryxThreadTranscript {
        var queryItems = [
            URLQueryItem(name: "thread_id", value: threadId),
            URLQueryItem(name: "limit", value: String(limit)),
            URLQueryItem(
                name: "include_tool_messages",
                value: includeToolMessages ? "true" : "false"
            ),
        ]
        if let beforeIndex {
            queryItems.append(URLQueryItem(name: "before_index", value: String(beforeIndex)))
        }
        if let userQueryLimit {
            queryItems.append(URLQueryItem(name: "user_query_limit", value: String(userQueryLimit)))
        }
        return try await get(
            "/api/threads/history",
            queryItems: queryItems
        )
    }

    public func threadLogs(threadId: String, cursor: Int? = nil) async throws -> GaryxThreadLogChunk {
        var queryItems: [URLQueryItem] = []
        if let cursor {
            queryItems.append(URLQueryItem(name: "cursor", value: String(cursor)))
        }
        return try await get("/api/threads/\(threadId.urlPathEncoded)/logs", queryItems: queryItems)
    }

    public func createThread(_ request: GaryxCreateThreadRequest) async throws -> GaryxThreadSummary {
        try await post("/api/threads", body: request)
    }

    public func updateThread(
        threadId: String,
        label: String? = nil,
        workspaceDir: String? = nil
    ) async throws -> GaryxThreadSummary {
        try await patch(
            "/api/threads/\(threadId.urlPathEncoded)",
            body: GaryxUpdateThreadRequest(label: label, workspaceDir: workspaceDir)
        )
    }

    public func deleteThread(threadId: String) async throws -> GaryxDeleteResult {
        try await delete("/api/threads/\(threadId.urlPathEncoded)")
    }

    public func interruptThread(threadId: String) async throws -> GaryxInterruptResult {
        try await post("/api/chat/interrupt", body: GaryxInterruptRequest(threadId: threadId))
    }

    public func streamInput(_ request: GaryxStreamInputRequest) async throws -> GaryxStreamInputResult {
        try await post("/api/chat/stream-input", body: request)
    }

    public func listAgents() async throws -> [GaryxAgentSummary] {
        let page: GaryxAgentsPage = try await get("/api/custom-agents")
        return page.agents
    }

    public func providerModels(providerType: String) async throws -> GaryxProviderModels {
        try await get("/api/provider-models/\(providerType.urlPathEncoded)")
    }

    public func generateAvatar(prompt: String, timeoutSecs: Int = 600) async throws -> GaryxGeneratedAvatar {
        try await post("/api/tools/image", body: GaryxGenerateAvatarRequest(prompt: prompt, timeoutSecs: timeoutSecs))
    }

    public func createAgent(_ request: GaryxCustomAgentRequest) async throws -> GaryxAgentSummary {
        try await post("/api/custom-agents", body: request)
    }

    public func updateAgent(
        agentId: String,
        request: GaryxCustomAgentRequest
    ) async throws -> GaryxAgentSummary {
        try await put("/api/custom-agents/\(agentId.urlPathEncoded)", body: request)
    }

    public func deleteAgent(agentId: String) async throws -> GaryxEmptyResponse {
        try await delete("/api/custom-agents/\(agentId.urlPathEncoded)")
    }

    public func listTeams() async throws -> [GaryxTeamSummary] {
        let page: GaryxTeamsPage = try await get("/api/teams")
        return page.teams
    }

    public func createTeam(_ request: GaryxTeamRequest) async throws -> GaryxTeamSummary {
        try await post("/api/teams", body: request)
    }

    public func updateTeam(teamId: String, request: GaryxTeamRequest) async throws -> GaryxTeamSummary {
        try await put("/api/teams/\(teamId.urlPathEncoded)", body: request)
    }

    public func deleteTeam(teamId: String) async throws -> GaryxEmptyResponse {
        try await delete("/api/teams/\(teamId.urlPathEncoded)")
    }

    public func listSkills() async throws -> [GaryxSkillSummary] {
        let page: GaryxSkillsPage = try await get("/api/skills")
        return page.skills
    }

    public func createSkill(_ request: GaryxCreateSkillRequest) async throws -> GaryxSkillSummary {
        try await post("/api/skills", body: request)
    }

    public func updateSkill(
        skillId: String,
        request: GaryxUpdateSkillRequest
    ) async throws -> GaryxSkillSummary {
        try await patch("/api/skills/\(skillId.urlPathEncoded)", body: request)
    }

    public func toggleSkill(skillId: String) async throws -> GaryxSkillSummary {
        try await patch("/api/skills/\(skillId.urlPathEncoded)/toggle", body: GaryxEmptyBody())
    }

    public func deleteSkill(skillId: String) async throws -> GaryxDeleteResult {
        try await delete("/api/skills/\(skillId.urlPathEncoded)")
    }

    public func skillEditor(skillId: String) async throws -> GaryxSkillEditorState {
        try await get("/api/skills/\(skillId.urlPathEncoded)/tree")
    }

    public func readSkillFile(skillId: String, path: String) async throws -> GaryxSkillFileDocument {
        try await get(
            "/api/skills/\(skillId.urlPathEncoded)/file",
            queryItems: [URLQueryItem(name: "path", value: path)]
        )
    }

    public func saveSkillFile(
        skillId: String,
        request: GaryxSkillFileWriteRequest
    ) async throws -> GaryxSkillFileDocument {
        try await put("/api/skills/\(skillId.urlPathEncoded)/file", body: request)
    }

    public func createSkillEntry(
        skillId: String,
        request: GaryxSkillEntryCreateRequest
    ) async throws -> GaryxSkillEditorState {
        try await post("/api/skills/\(skillId.urlPathEncoded)/entries", body: request)
    }

    public func deleteSkillEntry(skillId: String, path: String) async throws -> GaryxSkillEditorState {
        try await delete(
            "/api/skills/\(skillId.urlPathEncoded)/entries",
            queryItems: [URLQueryItem(name: "path", value: path)]
        )
    }

    public func listTasks(includeDone: Bool = true, limit: Int = 100) async throws -> GaryxTasksPage {
        try await listTasks(filter: GaryxTaskListFilter(includeDone: includeDone, limit: limit))
    }

    public func listTasks(filter: GaryxTaskListFilter) async throws -> GaryxTasksPage {
        var queryItems = [
            URLQueryItem(name: "include_done", value: filter.includeDone ? "true" : "false"),
            URLQueryItem(name: "limit", value: String(filter.limit)),
            URLQueryItem(name: "offset", value: String(filter.offset)),
        ]
        if let status = filter.status {
            queryItems.append(URLQueryItem(name: "status", value: status.rawValue))
        }
        if let assignee = filter.assignee?.trimmingCharacters(in: .whitespacesAndNewlines), !assignee.isEmpty {
            queryItems.append(URLQueryItem(name: "assignee", value: assignee))
        }
        if let sourceBotId = filter.sourceBotId?.trimmingCharacters(in: .whitespacesAndNewlines), !sourceBotId.isEmpty {
            queryItems.append(URLQueryItem(name: "source_bot_id", value: sourceBotId))
        }
        return try await get("/api/tasks", queryItems: queryItems)
    }

    public func createTask(_ request: GaryxTaskCreateRequest) async throws -> GaryxTaskSummary {
        try await post("/api/tasks", body: request)
    }

    public func promoteTask(_ request: GaryxTaskPromoteRequest) async throws -> GaryxTaskSummary {
        try await post("/api/tasks/promote", body: request)
    }

    public func updateTaskStatus(
        taskId: String,
        request: GaryxTaskUpdateStatusRequest
    ) async throws -> GaryxTaskEnvelope {
        try await patch("/api/tasks/\(taskId.urlPathEncoded)/status", body: request)
    }

    public func assignTask(taskId: String, request: GaryxTaskAssignRequest) async throws -> GaryxTaskEnvelope {
        try await patch("/api/tasks/\(taskId.urlPathEncoded)/assign", body: request)
    }

    public func unassignTask(taskId: String) async throws -> GaryxTaskEnvelope {
        try await delete("/api/tasks/\(taskId.urlPathEncoded)/assign")
    }

    public func updateTaskTitle(taskId: String, title: String) async throws -> GaryxTaskEnvelope {
        try await patch(
            "/api/tasks/\(taskId.urlPathEncoded)/title",
            body: GaryxTaskUpdateTitleRequest(title: title)
        )
    }

    public func stopTask(taskId: String) async throws -> GaryxTaskEnvelope {
        try await post("/api/tasks/\(taskId.urlPathEncoded)/stop", body: GaryxEmptyBody())
    }

    public func deleteTask(taskId: String) async throws -> GaryxDeleteResult {
        try await delete("/api/tasks/\(taskId.urlPathEncoded)")
    }

    public func listAutomations() async throws -> [GaryxAutomationSummary] {
        let page: GaryxAutomationsPage = try await get("/api/automations")
        return page.automations
    }

    public func createAutomation(_ request: GaryxAutomationCreateRequest) async throws -> GaryxAutomationSummary {
        try await post("/api/automations", body: request)
    }

    public func updateAutomation(
        id: String,
        request: GaryxAutomationUpdateRequest
    ) async throws -> GaryxAutomationSummary {
        try await patch("/api/automations/\(id.urlPathEncoded)", body: request)
    }

    public func deleteAutomation(id: String) async throws -> GaryxEmptyResponse {
        try await delete("/api/automations/\(id.urlPathEncoded)")
    }

    public func automationActivity(id: String, limit: Int = 20) async throws -> GaryxAutomationActivityFeed {
        try await get(
            "/api/automations/\(id.urlPathEncoded)/activity",
            queryItems: [URLQueryItem(name: "limit", value: String(limit))]
        )
    }

    public func runAutomationNow(id: String) async throws -> GaryxAutomationActivityEntry {
        try await post("/api/automations/\(id.urlPathEncoded)/run-now", body: GaryxEmptyBody())
    }

    public func updateAutomationEnabled(
        id: String,
        enabled: Bool
    ) async throws -> GaryxAutomationSummary {
        try await updateAutomation(id: id, request: GaryxAutomationUpdateRequest(enabled: enabled))
    }

    public func workspaceGitStatus(workspaceDir: String) async throws -> GaryxWorkspaceGitStatus {
        try await get(
            "/api/workspaces/git-status",
            queryItems: [URLQueryItem(name: "workspace_dir", value: workspaceDir)]
        )
    }

    public func listWorkspaceFiles(
        workspaceDir: String,
        directoryPath: String? = nil
    ) async throws -> GaryxWorkspaceFileListing {
        var queryItems = [URLQueryItem(name: "workspaceDir", value: workspaceDir)]
        if let directoryPath, !directoryPath.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            queryItems.append(URLQueryItem(name: "path", value: directoryPath))
        }
        return try await get("/api/workspace-files", queryItems: queryItems)
    }

    public func previewWorkspaceFile(
        workspaceDir: String,
        path: String
    ) async throws -> GaryxWorkspaceFilePreview {
        try await get(
            "/api/workspace-files/preview",
            queryItems: [
                URLQueryItem(name: "workspaceDir", value: workspaceDir),
                URLQueryItem(name: "path", value: path),
            ]
        )
    }

    public func uploadWorkspaceFiles(
        _ request: GaryxUploadWorkspaceFilesRequest
    ) async throws -> GaryxUploadWorkspaceFilesResult {
        try await post("/api/workspace-files/upload", body: request)
    }

    public func uploadChatAttachments(
        _ request: GaryxUploadChatAttachmentsRequest
    ) async throws -> GaryxUploadChatAttachmentsResult {
        try await post("/api/chat/attachments/upload", body: request)
    }

    public func listSlashCommands() async throws -> [GaryxSlashCommand] {
        let page: GaryxSlashCommandsPage = try await get("/api/commands/shortcuts")
        return page.commands
    }

    public func createSlashCommand(_ request: GaryxSlashCommandRequest) async throws -> GaryxSlashCommand {
        try await post("/api/commands/shortcuts", body: request)
    }

    public func updateSlashCommand(
        currentName: String,
        request: GaryxSlashCommandRequest
    ) async throws -> GaryxSlashCommand {
        try await put("/api/commands/shortcuts/\(currentName.urlPathEncoded)", body: request)
    }

    public func deleteSlashCommand(name: String) async throws -> GaryxDeleteResult {
        try await delete("/api/commands/shortcuts/\(name.urlPathEncoded)")
    }

    public func listMcpServers() async throws -> [GaryxMcpServer] {
        let page: GaryxMcpServersPage = try await get("/api/mcp-servers")
        return page.servers
    }

    public func createMcpServer(_ request: GaryxMcpServerRequest) async throws -> GaryxMcpServer {
        try await post("/api/mcp-servers", body: request)
    }

    public func updateMcpServer(
        currentName: String,
        request: GaryxMcpServerRequest
    ) async throws -> GaryxMcpServer {
        try await put("/api/mcp-servers/\(currentName.urlPathEncoded)", body: request)
    }

    public func deleteMcpServer(name: String) async throws -> GaryxEmptyResponse {
        try await delete("/api/mcp-servers/\(name.urlPathEncoded)")
    }

    public func toggleMcpServer(name: String, enabled: Bool) async throws -> GaryxMcpServer {
        try await patch(
            "/api/mcp-servers/\(name.urlPathEncoded)/toggle",
            body: GaryxMcpServerToggleRequest(enabled: enabled)
        )
    }

    public func listAutoResearchRuns(limit: Int = 50) async throws -> [GaryxAutoResearchRun] {
        let page: GaryxAutoResearchRunsPage = try await get(
            "/api/auto-research/runs",
            queryItems: [URLQueryItem(name: "limit", value: String(limit))]
        )
        return page.items
    }

    public func createAutoResearchRun(
        _ request: GaryxAutoResearchCreateRequest
    ) async throws -> GaryxAutoResearchRun {
        try await post("/api/auto-research/runs", body: request)
    }

    public func getAutoResearchRun(runId: String) async throws -> GaryxAutoResearchDetail {
        try await get("/api/auto-research/runs/\(runId.urlPathEncoded)")
    }

    public func listAutoResearchIterations(runId: String) async throws -> [GaryxAutoResearchIteration] {
        let page: GaryxAutoResearchIterationsPage = try await get(
            "/api/auto-research/runs/\(runId.urlPathEncoded)/iterations"
        )
        return page.items
    }

    public func stopAutoResearchRun(runId: String, reason: String? = nil) async throws -> GaryxAutoResearchRun {
        try await post(
            "/api/auto-research/runs/\(runId.urlPathEncoded)/stop",
            body: GaryxAutoResearchStopRequest(reason: reason)
        )
    }

    public func deleteAutoResearchRun(runId: String) async throws -> GaryxEmptyResponse {
        try await delete("/api/auto-research/runs/\(runId.urlPathEncoded)")
    }

    public func listAutoResearchCandidates(runId: String) async throws -> GaryxAutoResearchCandidatesPage {
        try await get("/api/auto-research/runs/\(runId.urlPathEncoded)/candidates")
    }

    public func selectAutoResearchCandidate(
        runId: String,
        candidateId: String
    ) async throws -> GaryxAutoResearchRun {
        try await post(
            "/api/auto-research/runs/\(runId.urlPathEncoded)/select/\(candidateId.urlPathEncoded)",
            body: GaryxEmptyBody()
        )
    }

    public func sendAutoResearchFeedback(
        runId: String,
        request: GaryxAutoResearchFeedbackRequest
    ) async throws -> GaryxAutoResearchRun {
        try await post("/api/auto-research/runs/\(runId.urlPathEncoded)/feedback", body: request)
    }

    public func reverifyAutoResearchCandidate(
        runId: String,
        request: GaryxAutoResearchReverifyRequest
    ) async throws -> GaryxAutoResearchRun {
        try await post("/api/auto-research/runs/\(runId.urlPathEncoded)/reverify", body: request)
    }

    public func listChannelEndpoints() async throws -> [GaryxChannelEndpoint] {
        let page: GaryxChannelEndpointsPage = try await get("/api/channel-endpoints")
        return page.endpoints
    }

    public func listConfiguredBots() async throws -> [GaryxConfiguredBot] {
        let page: GaryxConfiguredBotsPage = try await get("/api/configured-bots")
        return page.bots
    }

    public func listBotConsoles() async throws -> [GaryxBotConsoleSummary] {
        let page: GaryxBotConsolesPage = try await get("/api/bot-consoles")
        return page.bots
    }

    public func botStatus(botId: String) async throws -> GaryxBotBindingResult {
        try await get(
            "/api/bot/status",
            queryItems: [URLQueryItem(name: "bot_id", value: botId)]
        )
    }

    public func bindBot(botId: String, threadId: String) async throws -> GaryxBotBindingResult {
        try await post(
            "/api/bot/bind",
            body: GaryxBotBindingRequest(botId: botId, threadId: threadId)
        )
    }

    public func unbindBot(botId: String) async throws -> GaryxBotBindingResult {
        try await post(
            "/api/bot/unbind",
            body: GaryxBotBindingRequest(botId: botId)
        )
    }

    public func listChannelPlugins() async throws -> [GaryxChannelPluginCatalogEntry] {
        let page: GaryxChannelPluginCatalogPage = try await get("/api/channels/plugins")
        return page.plugins
    }

    public func startChannelAuthFlow(
        pluginId: String,
        formState: [String: GaryxJSONValue] = [:]
    ) async throws -> GaryxChannelAuthSession {
        try await post(
            "/api/channels/plugins/\(pluginId.urlPathEncoded)/auth_flow/start",
            body: GaryxChannelAuthStartRequest(formState: formState)
        )
    }

    public func pollChannelAuthFlow(
        pluginId: String,
        sessionId: String
    ) async throws -> GaryxChannelAuthPollResult {
        try await post(
            "/api/channels/plugins/\(pluginId.urlPathEncoded)/auth_flow/poll",
            body: GaryxChannelAuthPollRequest(sessionId: sessionId)
        )
    }

    public func validateChannelAccount(
        pluginId: String,
        request: GaryxChannelAccountValidationRequest
    ) async throws -> GaryxChannelAccountValidationResult {
        try await post(
            "/api/channels/plugins/\(pluginId.urlPathEncoded)/validate_account",
            body: request
        )
    }

    public func bindChannelEndpoint(endpointKey: String, threadId: String) async throws -> GaryxEmptyResponse {
        try await post(
            "/api/channel-bindings/bind",
            body: GaryxChannelEndpointBindRequest(endpointKey: endpointKey, threadId: threadId)
        )
    }

    public func detachChannelEndpoint(endpointKey: String) async throws -> GaryxEmptyResponse {
        try await post(
            "/api/channel-bindings/detach",
            body: GaryxChannelEndpointDetachRequest(endpointKey: endpointKey)
        )
    }

    public func url(for path: String, queryItems: [URLQueryItem] = []) throws -> URL {
        guard var components = URLComponents(url: configuration.baseURL, resolvingAgainstBaseURL: false) else {
            throw GaryxGatewayError.invalidURL(configuration.baseURL.absoluteString)
        }
        let pathParts = path.split(separator: "?", maxSplits: 1, omittingEmptySubsequences: false)
        let requestedPath = String(pathParts.first ?? "")
        let requestedQuery = pathParts.dropFirst().first.map(String.init)
        var requestedQueryComponents = URLComponents()
        requestedQueryComponents.percentEncodedQuery = requestedQuery
        let basePath = components.percentEncodedPath.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        let nextPath = requestedPath.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        components.percentEncodedPath = [basePath, nextPath]
            .filter { !$0.isEmpty }
            .joined(separator: "/")
        if !components.percentEncodedPath.hasPrefix("/") {
            components.percentEncodedPath = "/" + components.percentEncodedPath
        }
        let mergedQueryItems = (requestedQueryComponents.queryItems ?? []) + queryItems
        components.queryItems = mergedQueryItems.isEmpty ? nil : mergedQueryItems

        guard let url = components.url else {
            throw GaryxGatewayError.invalidURL(path)
        }
        return url
    }

    public func chatWebSocketURL() throws -> URL {
        let httpURL = try url(for: "/api/chat/ws")
        guard var components = URLComponents(url: httpURL, resolvingAgainstBaseURL: false) else {
            throw GaryxGatewayError.invalidURL(httpURL.absoluteString)
        }
        switch components.scheme {
        case "https":
            components.scheme = "wss"
        case "http":
            components.scheme = "ws"
        default:
            throw GaryxGatewayError.invalidURL(httpURL.absoluteString)
        }
        if let token = configuration.authToken, !token.isEmpty {
            var items = components.queryItems ?? []
            items.append(URLQueryItem(name: "token", value: token))
            components.queryItems = items
        }
        guard let url = components.url else {
            throw GaryxGatewayError.invalidURL(httpURL.absoluteString)
        }
        return url
    }

    public func makeWebSocketTask() throws -> URLSessionWebSocketTask {
        session.webSocketTask(with: try chatWebSocketURL())
    }

    public func eventStreamRequest(historyLimit: Int = 50) throws -> URLRequest {
        var request = try makeRequest(
            path: "/api/stream",
            method: "GET",
            queryItems: [
                URLQueryItem(name: "history_limit", value: String(historyLimit)),
            ]
        )
        request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
        return request
    }

    public func encodeWebSocketCommand(_ command: GaryxChatWebSocketCommand) throws -> String {
        do {
            let data = try encoder.encode(command)
            guard let text = String(data: data, encoding: .utf8) else {
                throw GaryxGatewayError.encodingFailed("Unable to encode chat command as UTF-8.")
            }
            return text
        } catch let error as GaryxGatewayError {
            throw error
        } catch {
            throw GaryxGatewayError.encodingFailed(error.localizedDescription)
        }
    }

    public func decodeStreamEvent(_ text: String) throws -> GaryxChatStreamEvent {
        try decoder.decode(GaryxChatStreamEvent.self, from: Data(text.utf8))
    }

    private func get<Response: Decodable>(
        _ path: String,
        queryItems: [URLQueryItem] = []
    ) async throws -> Response {
        var request = try makeRequest(path: path, method: "GET", queryItems: queryItems)
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        return try await send(request)
    }

    private func post<Response: Decodable, Body: Encodable>(_ path: String, body: Body) async throws -> Response {
        var request = try makeRequest(path: path, method: "POST")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try encoder.encode(body)
        return try await send(request)
    }

    private func patch<Response: Decodable, Body: Encodable>(_ path: String, body: Body) async throws -> Response {
        var request = try makeRequest(path: path, method: "PATCH")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try encoder.encode(body)
        return try await send(request)
    }

    private func put<Response: Decodable, Body: Encodable>(_ path: String, body: Body) async throws -> Response {
        var request = try makeRequest(path: path, method: "PUT")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try encoder.encode(body)
        return try await send(request)
    }

    private func delete<Response: Decodable>(
        _ path: String,
        queryItems: [URLQueryItem] = []
    ) async throws -> Response {
        var request = try makeRequest(path: path, method: "DELETE", queryItems: queryItems)
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        return try await send(request)
    }

    private func makeRequest(
        path: String,
        method: String,
        queryItems: [URLQueryItem] = []
    ) throws -> URLRequest {
        var request = URLRequest(url: try url(for: path, queryItems: queryItems))
        request.httpMethod = method
        if let token = configuration.authToken, !token.isEmpty {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        return request
    }

    private func send<Response: Decodable>(_ request: URLRequest) async throws -> Response {
        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw GaryxGatewayError.invalidHTTPResponse
        }
        guard (200..<300).contains(http.statusCode) else {
            let body = String(data: data, encoding: .utf8) ?? ""
            throw GaryxGatewayError.httpStatus(http.statusCode, body)
        }
        if data.isEmpty, Response.self == GaryxEmptyResponse.self {
            return GaryxEmptyResponse() as! Response
        }
        return try decoder.decode(Response.self, from: data)
    }
}

private extension KeyedDecodingContainer {
    func decodeFirstString(_ keys: Key...) throws -> String? {
        for key in keys {
            if let value = try decodeIfPresent(String.self, forKey: key) {
                return value
            }
        }
        return nil
    }

    func decodeFirstBool(_ keys: Key...) throws -> Bool? {
        for key in keys {
            if let value = try decodeIfPresent(Bool.self, forKey: key) {
                return value
            }
        }
        return nil
    }

    func decodeFirstInt(_ keys: Key...) throws -> Int? {
        for key in keys {
            if let value = try decodeIfPresent(Int.self, forKey: key) {
                return value
            }
        }
        return nil
    }

    func decodeFirstStringArray(_ keys: Key...) throws -> [String]? {
        for key in keys {
            if let value = try decodeIfPresent([String].self, forKey: key) {
                return value
            }
        }
        return nil
    }
}

private extension GaryxJSONValue {
    static func decoded(from text: String) -> GaryxJSONValue? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.hasPrefix("{") || trimmed.hasPrefix("[") else { return nil }
        return try? JSONDecoder().decode(GaryxJSONValue.self, from: Data(trimmed.utf8))
    }

    var objectValue: [String: GaryxJSONValue]? {
        if case .object(let value) = self {
            return value
        }
        return nil
    }

    var jsonStringDecodedIfNeeded: GaryxJSONValue {
        if case .string(let value) = self,
           let decoded = GaryxJSONValue.decoded(from: value) {
            return decoded
        }
        return self
    }

    var stringValue: String? {
        switch self {
        case .string(let value):
            return value.garyxTrimmedNilIfEmpty
        case .number(let value):
            if value.rounded() == value,
               let exactInteger = Int(exactly: value) {
                return String(exactInteger)
            }
            return String(value).garyxTrimmedNilIfEmpty
        case .bool(let value):
            return value ? "true" : "false"
        case .null, .array, .object:
            return nil
        }
    }
}

private extension Dictionary where Key == String, Value == GaryxJSONValue {
    func stringValue(forKeys keys: [String]) -> String? {
        for key in keys {
            if let value = self[key]?.stringValue?.garyxTrimmedNilIfEmpty {
                return value
            }
        }
        return nil
    }

    func objectValue(forKeys keys: [String]) -> [String: GaryxJSONValue]? {
        for key in keys {
            if let value = self[key]?.objectValue {
                return value
            }
        }
        return nil
    }
}

private extension String {
    var lastPathComponent: String {
        (self as NSString).lastPathComponent
    }

    var garyxTrimmedNilIfEmpty: String? {
        let trimmed = trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    var urlPathEncoded: String {
        GaryxGatewayClient.encodePathSegment(self)
    }
}

private let garyxURLPathSegmentAllowed: CharacterSet = {
    CharacterSet(charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")
}()
