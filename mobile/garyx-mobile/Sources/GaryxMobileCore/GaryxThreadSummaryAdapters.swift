import Foundation

/// Strict wire model for `GET /api/thread-summaries` and the enhanced
/// favorites snapshot. The route is snake_case-only; keeping this DTO
/// separate prevents the compatibility rules of the legacy point-read route
/// from silently widening the new contract.
public struct GaryxThreadSummaryRowDTO: Decodable, Equatable, Sendable {
    public var threadId: String
    public var title: String?
    public var workspaceDir: String?
    public var threadType: String
    public var providerType: String?
    public var agentId: String?
    public var createdAt: String?
    public var updatedAt: String?
    public var messageCount: Int
    public var lastUserMessage: String?
    public var lastAssistantMessage: String?
    public var lastMessagePreview: String?
    public var recentRunId: String?
    public var activeRunId: String?
    public var worktree: GaryxThreadSummaryWorktreeDTO?

    enum CodingKeys: String, CodingKey {
        case threadId = "thread_id"
        case title
        case workspaceDir = "workspace_dir"
        case threadType = "thread_type"
        case providerType = "provider_type"
        case agentId = "agent_id"
        case createdAt = "created_at"
        case updatedAt = "updated_at"
        case messageCount = "message_count"
        case lastUserMessage = "last_user_message"
        case lastAssistantMessage = "last_assistant_message"
        case lastMessagePreview = "last_message_preview"
        case recentRunId = "recent_run_id"
        case activeRunId = "active_run_id"
        case worktree
    }
}

public struct GaryxThreadSummaryWorktreeDTO: Decodable, Equatable, Sendable {
    public var path: String?
    public var worktreeDir: String?

    enum CodingKeys: String, CodingKey {
        case path
        case worktreeDir = "worktree_dir"
    }

    var visiblePath: String? {
        Self.nonEmpty(worktreeDir) ?? Self.nonEmpty(path)
    }

    private static func nonEmpty(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.isEmpty ? nil : trimmed
    }
}

/// Raw canonical record returned by `GET /api/threads/:id`. This point-read
/// route still returns canonical records whose display title is stored under
/// `label`; keep that compatibility inside this adapter only.
public struct GaryxLegacyThreadRecordDTO: Decodable, Equatable, Sendable {
    public var payload: [String: GaryxJSONValue]

    public init(payload: [String: GaryxJSONValue]) {
        self.payload = payload
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        payload = try container.decode([String: GaryxJSONValue].self)
    }
}

/// Camel-case summary embedded by `/api/automations/{id}/threads`.
/// This is a third adapter, not a compatibility branch in either canonical
/// adapter: the automation endpoint deliberately owns a different wire shape.
public struct GaryxAutomationThreadSummaryDTO: Decodable, Equatable, Sendable {
    public var id: String
    public var title: String?
    public var workspaceDir: String?
    public var threadType: String?
    public var providerType: String?
    public var agentId: String?
    public var createdAt: String?
    public var updatedAt: String?
    public var messageCount: Int?
    public var lastUserMessage: String?
    public var lastAssistantMessage: String?
    public var recentRunId: String?
    public var activeRunId: String?
    public var automationId: String?
    public var automationThreadMode: String?

    enum CodingKeys: String, CodingKey {
        case id
        case threadId
        case title
        case label
        case workspaceDir
        case threadType
        case providerType
        case agentId
        case createdAt
        case updatedAt
        case messageCount
        case lastUserMessage
        case lastAssistantMessage
        case recentRunId
        case activeRunId
        case automationId
        case automationThreadMode
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.garyxDecodeFirstString(.id, .threadId) ?? ""
        title = try container.garyxDecodeFirstString(.title, .label)
        workspaceDir = try container.garyxDecodeFirstString(.workspaceDir)
        threadType = try container.garyxDecodeFirstString(.threadType)
        providerType = try container.garyxDecodeFirstString(.providerType)
        agentId = try container.garyxDecodeFirstString(.agentId)
        createdAt = try container.garyxDecodeFirstString(.createdAt)
        updatedAt = try container.garyxDecodeFirstString(.updatedAt)
        messageCount = try container.decodeIfPresent(Int.self, forKey: .messageCount)
        lastUserMessage = try container.garyxDecodeFirstString(.lastUserMessage)
        lastAssistantMessage = try container.garyxDecodeFirstString(.lastAssistantMessage)
        recentRunId = try container.garyxDecodeFirstString(.recentRunId)
        activeRunId = try container.garyxDecodeFirstString(.activeRunId)
        automationId = try container.garyxDecodeFirstString(.automationId)
        automationThreadMode = try container.garyxDecodeFirstString(.automationThreadMode)
    }
}

public enum GaryxThreadSummaryAdapter {
    public static func summary(_ row: GaryxThreadSummaryRowDTO) -> GaryxThreadSummary {
        GaryxThreadSummary(
            id: row.threadId,
            title: nonEmpty(row.title) ?? "New Thread",
            createdAt: row.createdAt,
            updatedAt: row.updatedAt,
            lastMessagePreview: nonEmpty(row.lastMessagePreview)
                ?? nonEmpty(row.lastUserMessage)
                ?? nonEmpty(row.lastAssistantMessage)
                ?? "",
            workspacePath: row.workspaceDir,
            messageCount: row.messageCount,
            agentId: row.agentId,
            providerType: row.providerType,
            recentRunId: row.recentRunId,
            activeRunId: row.activeRunId,
            runState: runState(activeRunId: row.activeRunId, recentRunId: row.recentRunId),
            worktreePath: row.worktree?.visiblePath
        )
    }

    public static func summary(_ record: GaryxLegacyThreadRecordDTO) -> GaryxThreadSummary {
        let payload = record.payload
        let id = string(payload, keys: ["thread_id", "thread_key", "id"]) ?? ""
        let recentRunId = string(payload, keys: ["recent_run_id"])
            ?? payload["history"]?.garyxGatewayObjectValue?
                .garyxGatewayArrayLastString(forKey: "recent_committed_run_ids")
        let activeRunId = string(payload, keys: ["active_run_id"])
        return GaryxThreadSummary(
            id: id,
            title: string(payload, keys: ["label", "title"]) ?? "New Thread",
            createdAt: string(payload, keys: ["created_at"]),
            updatedAt: string(payload, keys: ["updated_at", "last_active_at"]),
            lastMessagePreview: legacyPreview(payload),
            workspacePath: string(payload, keys: ["workspace_dir", "workspace_path"]),
            messageCount: integer(payload["message_count"])
                ?? payload["history"]?.garyxGatewayObjectValue?
                    .garyxGatewayArrayCount(forKey: "messages"),
            agentId: string(payload, keys: ["agent_id"]),
            providerType: string(payload, keys: ["provider_type"]),
            recentRunId: recentRunId,
            activeRunId: activeRunId,
            runState: string(payload, keys: ["run_state"])
                ?? runState(activeRunId: activeRunId, recentRunId: recentRunId),
            worktreePath: legacyWorktreePath(payload),
            automationId: string(payload, keys: ["automation_id"]),
            automationThreadMode: string(payload, keys: ["automation_thread_mode"]),
            threadRuntime: decode(GaryxThreadRuntimeSummary.self, from: payload["thread_runtime"])
        )
    }

    public static func summary(_ row: GaryxAutomationThreadSummaryDTO) -> GaryxThreadSummary {
        GaryxThreadSummary(
            id: row.id,
            title: nonEmpty(row.title) ?? "New Thread",
            createdAt: row.createdAt,
            updatedAt: row.updatedAt,
            lastMessagePreview: nonEmpty(row.lastUserMessage)
                ?? nonEmpty(row.lastAssistantMessage)
                ?? "",
            workspacePath: row.workspaceDir,
            messageCount: row.messageCount,
            agentId: row.agentId,
            providerType: row.providerType,
            recentRunId: row.recentRunId,
            activeRunId: row.activeRunId,
            runState: runState(activeRunId: row.activeRunId, recentRunId: row.recentRunId),
            worktreePath: nil,
            automationId: row.automationId,
            automationThreadMode: row.automationThreadMode
        )
    }

    private static func legacyPreview(_ payload: [String: GaryxJSONValue]) -> String {
        string(
            payload,
            keys: [
                "last_message_preview",
                "last_user_message",
                "last_assistant_message",
                "last_user_preview",
                "last_assistant_preview",
            ]
        ) ?? ""
    }

    private static func legacyWorktreePath(_ payload: [String: GaryxJSONValue]) -> String? {
        guard let worktree = payload["worktree"]?.garyxGatewayObjectValue else { return nil }
        return string(worktree, keys: ["worktree_dir", "path"])
    }

    private static func string(
        _ payload: [String: GaryxJSONValue],
        keys: [String]
    ) -> String? {
        payload.garyxGatewayStringValue(forKeys: keys)
    }

    private static func integer(_ value: GaryxJSONValue?) -> Int? {
        guard case .number(let number)? = value,
              number.rounded() == number else { return nil }
        return Int(exactly: number)
    }

    private static func decode<Value: Decodable>(
        _ type: Value.Type,
        from value: GaryxJSONValue?
    ) -> Value? {
        guard let value, let data = try? JSONEncoder().encode(value) else { return nil }
        return try? JSONDecoder().decode(type, from: data)
    }

    private static func nonEmpty(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.isEmpty ? nil : trimmed
    }

    private static func runState(activeRunId: String?, recentRunId: String?) -> String? {
        if nonEmpty(activeRunId) != nil { return "running" }
        if nonEmpty(recentRunId) != nil { return "idle" }
        return nil
    }
}

private extension Dictionary where Key == String, Value == GaryxJSONValue {
    func garyxGatewayArrayCount(forKey key: String) -> Int? {
        guard case .array(let values)? = self[key] else { return nil }
        return values.count
    }

    func garyxGatewayArrayLastString(forKey key: String) -> String? {
        guard case .array(let values)? = self[key] else { return nil }
        return values.reversed().compactMap(\.garyxGatewayStringValue).first
    }
}

public enum GaryxThreadFavoriteCapability: Equatable, Sendable {
    case addAndRemove
    case none
}

public enum GaryxThreadArchiveStrategy: Equatable, Sendable {
    case thread
    case botEndpoint
    case none
}

public struct GaryxThreadRowCapabilities: Equatable, Sendable {
    public var canOpen: Bool
    public var canPin: Bool
    public var canArchive: Bool
    public var favorite: GaryxThreadFavoriteCapability
    public var archiveStrategy: GaryxThreadArchiveStrategy

    public init(
        canOpen: Bool,
        canPin: Bool,
        canArchive: Bool,
        favorite: GaryxThreadFavoriteCapability,
        archiveStrategy: GaryxThreadArchiveStrategy
    ) {
        self.canOpen = canOpen
        self.canPin = canPin
        self.canArchive = canArchive
        self.favorite = favorite
        self.archiveStrategy = archiveStrategy
    }
}

public struct GaryxThreadRowCapabilityContext: Equatable, Sendable {
    public var openable: Bool
    public var automationTargetThreadIds: Set<String>
    public var hasActiveRun: Bool
    public var botEndpointRow: Bool
    public var botEndpointCanArchive: Bool

    public init(
        openable: Bool = true,
        automationTargetThreadIds: Set<String> = [],
        hasActiveRun: Bool = false,
        botEndpointRow: Bool = false,
        botEndpointCanArchive: Bool = true
    ) {
        self.openable = openable
        self.automationTargetThreadIds = automationTargetThreadIds
        self.hasActiveRun = hasActiveRun
        self.botEndpointRow = botEndpointRow
        self.botEndpointCanArchive = botEndpointCanArchive
    }
}

/// The only row-action capability derivation used by list surfaces.
public enum GaryxThreadRowCapabilityDeriver {
    public static func capabilities(
        for summary: GaryxThreadSummary?,
        context: GaryxThreadRowCapabilityContext
    ) -> GaryxThreadRowCapabilities {
        guard let summary,
              !summary.id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              context.openable else {
            return GaryxThreadRowCapabilities(
                canOpen: false,
                canPin: false,
                canArchive: false,
                favorite: .none,
                archiveStrategy: .none
            )
        }

        let isAutomationTarget = context.automationTargetThreadIds.contains(summary.id)
        // Active run is a runtime overlay input, not cached summary content.
        // Home's cached section builder passes false; its run-state projection
        // patches the capability alongside the running dot without rebuilding
        // static section identity.
        let activeRun = context.hasActiveRun
        let canArchive = !isAutomationTarget
            && !activeRun
            && (!context.botEndpointRow || context.botEndpointCanArchive)
        let strategy: GaryxThreadArchiveStrategy
        if !canArchive {
            strategy = .none
        } else {
            strategy = context.botEndpointRow ? .botEndpoint : .thread
        }
        return GaryxThreadRowCapabilities(
            canOpen: true,
            canPin: true,
            canArchive: canArchive,
            favorite: .addAndRemove,
            archiveStrategy: strategy
        )
    }
}
