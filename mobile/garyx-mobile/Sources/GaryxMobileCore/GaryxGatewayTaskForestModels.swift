import Foundation

/// Anchored task forest page from `GET /api/tasks/forest?anchor_thread_id=`.
/// The gateway sends the source-conversation `thread` root first (when the
/// origin is known) followed by task nodes in DFS pre-order; `depth` and
/// `activeCount` are absent when talking to an older gateway.
public struct GaryxTaskForestPage: Decodable, Equatable, Sendable {
    public var nodes: [GaryxTaskForestNode]
    public var total: Int
    public var activeCount: Int?
    public var rootThreadIds: [String]

    public init(
        nodes: [GaryxTaskForestNode] = [],
        total: Int = 0,
        activeCount: Int? = nil,
        rootThreadIds: [String] = []
    ) {
        self.nodes = nodes
        self.total = total
        self.activeCount = activeCount
        self.rootThreadIds = rootThreadIds
    }

    enum CodingKeys: String, CodingKey {
        case tasks
        case total
        case activeCount = "active_count"
        case activeCountCamel = "activeCount"
        case rootThreadIds = "root_thread_ids"
        case rootThreadIdsCamel = "rootThreadIds"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        nodes = try container.decodeIfPresent([GaryxTaskForestNode].self, forKey: .tasks) ?? []
        total = try container.decodeIfPresent(Int.self, forKey: .total) ?? nodes.count
        activeCount = try container.garyxDecodeFirstInt(.activeCount, .activeCountCamel)
        rootThreadIds = try container.garyxDecodeFirstStringArray(
            .rootThreadIds, .rootThreadIdsCamel
        ) ?? []
    }
}

public enum GaryxTaskForestNode: Decodable, Equatable, Sendable, Identifiable {
    case thread(GaryxTaskForestThreadNode)
    case task(GaryxTaskForestTaskNode)

    public var id: String { nodeId }

    public var nodeId: String {
        switch self {
        case .thread(let node): node.nodeId
        case .task(let node): node.nodeId
        }
    }

    public var threadId: String {
        switch self {
        case .thread(let node): node.threadId
        case .task(let node): node.task.threadId
        }
    }

    public var depth: Int? {
        switch self {
        case .thread(let node): node.depth
        case .task(let node): node.depth
        }
    }

    public var taskNode: GaryxTaskForestTaskNode? {
        if case .task(let node) = self { return node }
        return nil
    }

    enum CodingKeys: String, CodingKey {
        case kind
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let kind = try container.garyxDecodeFirstString(.kind) ?? "task"
        switch kind.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() {
        case "thread":
            self = .thread(try GaryxTaskForestThreadNode(from: decoder))
        default:
            self = .task(try GaryxTaskForestTaskNode(from: decoder))
        }
    }
}

public struct GaryxTaskForestThreadNode: Decodable, Equatable, Sendable {
    public var nodeId: String
    public var threadId: String
    public var title: String
    public var threadType: String
    public var providerType: String?
    public var agentId: String?
    public var messageCount: Int
    public var lastMessagePreview: String
    public var activeRunId: String?
    public var runState: String
    public var updatedAt: String?
    public var lastActiveAt: String?
    public var depth: Int?

    public init(
        nodeId: String = "",
        threadId: String,
        title: String = "",
        threadType: String = "chat",
        providerType: String? = nil,
        agentId: String? = nil,
        messageCount: Int = 0,
        lastMessagePreview: String = "",
        activeRunId: String? = nil,
        runState: String = "idle",
        updatedAt: String? = nil,
        lastActiveAt: String? = nil,
        depth: Int? = nil
    ) {
        self.nodeId = nodeId.isEmpty ? "thread-root:\(threadId)" : nodeId
        self.threadId = threadId
        self.title = title.isEmpty ? threadId : title
        self.threadType = threadType
        self.providerType = providerType
        self.agentId = agentId
        self.messageCount = messageCount
        self.lastMessagePreview = lastMessagePreview
        self.activeRunId = activeRunId
        self.runState = runState
        self.updatedAt = updatedAt
        self.lastActiveAt = lastActiveAt
        self.depth = depth
    }

    enum CodingKeys: String, CodingKey {
        case nodeId = "node_id"
        case nodeIdCamel = "nodeId"
        case threadId = "thread_id"
        case threadIdCamel = "threadId"
        case title
        case threadType = "thread_type"
        case threadTypeCamel = "threadType"
        case providerType = "provider_type"
        case providerTypeCamel = "providerType"
        case agentId = "agent_id"
        case agentIdCamel = "agentId"
        case messageCount = "message_count"
        case messageCountCamel = "messageCount"
        case lastMessagePreview = "last_message_preview"
        case lastMessagePreviewCamel = "lastMessagePreview"
        case activeRunId = "active_run_id"
        case activeRunIdCamel = "activeRunId"
        case runState = "run_state"
        case runStateCamel = "runState"
        case updatedAt = "updated_at"
        case updatedAtCamel = "updatedAt"
        case lastActiveAt = "last_active_at"
        case lastActiveAtCamel = "lastActiveAt"
        case depth
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let threadId = try container.garyxDecodeFirstString(.threadId, .threadIdCamel) ?? ""
        self.threadId = threadId
        nodeId = try container.garyxDecodeFirstString(.nodeId, .nodeIdCamel)
            ?? "thread-root:\(threadId)"
        let rawTitle = try container.garyxDecodeFirstString(.title) ?? ""
        title = rawTitle.isEmpty ? threadId : rawTitle
        threadType = try container.garyxDecodeFirstString(.threadType, .threadTypeCamel) ?? "chat"
        providerType = try container.garyxDecodeFirstString(.providerType, .providerTypeCamel)
        agentId = try container.garyxDecodeFirstString(.agentId, .agentIdCamel)
        messageCount = try container.garyxDecodeFirstInt(.messageCount, .messageCountCamel) ?? 0
        lastMessagePreview = try container.garyxDecodeFirstString(
            .lastMessagePreview, .lastMessagePreviewCamel
        ) ?? ""
        activeRunId = try container.garyxDecodeFirstString(.activeRunId, .activeRunIdCamel)
        runState = try container.garyxDecodeFirstString(.runState, .runStateCamel) ?? "idle"
        updatedAt = try container.garyxDecodeFirstString(.updatedAt, .updatedAtCamel)
        lastActiveAt = try container.garyxDecodeFirstString(.lastActiveAt, .lastActiveAtCamel)
        depth = try container.garyxDecodeFirstInt(.depth)
    }
}

public struct GaryxTaskForestTaskNode: Decodable, Equatable, Sendable {
    public var nodeId: String
    public var parentNodeId: String?
    /// Flattened task fields decoded through the existing summary model.
    public var task: GaryxTaskSummary
    public var parentTaskNumber: Int?
    public var parentThreadId: String?
    public var activeRunId: String?
    public var runState: String
    public var lastActiveAt: String?
    public var depth: Int?

    public init(
        nodeId: String = "",
        task: GaryxTaskSummary,
        parentNodeId: String? = nil,
        parentTaskNumber: Int? = nil,
        parentThreadId: String? = nil,
        activeRunId: String? = nil,
        runState: String = "idle",
        lastActiveAt: String? = nil,
        depth: Int? = nil
    ) {
        self.nodeId = nodeId.isEmpty ? "task:\(task.threadId)" : nodeId
        self.task = task
        self.parentNodeId = parentNodeId
        self.parentTaskNumber = parentTaskNumber
        self.parentThreadId = parentThreadId
        self.activeRunId = activeRunId
        self.runState = runState
        self.lastActiveAt = lastActiveAt
        self.depth = depth
    }

    enum CodingKeys: String, CodingKey {
        case nodeId = "node_id"
        case nodeIdCamel = "nodeId"
        case parentNodeId = "parent_node_id"
        case parentNodeIdCamel = "parentNodeId"
        case parentTaskNumber = "parent_task_number"
        case parentTaskNumberCamel = "parentTaskNumber"
        case parentThreadId = "parent_thread_id"
        case parentThreadIdCamel = "parentThreadId"
        case activeRunId = "active_run_id"
        case activeRunIdCamel = "activeRunId"
        case runState = "run_state"
        case runStateCamel = "runState"
        case lastActiveAt = "last_active_at"
        case lastActiveAtCamel = "lastActiveAt"
        case depth
    }

    public init(from decoder: Decoder) throws {
        // The gateway flattens TaskSummary fields into the node object; reuse
        // the summary decoder over the same payload.
        task = try GaryxTaskSummary(from: decoder)
        let container = try decoder.container(keyedBy: CodingKeys.self)
        nodeId = try container.garyxDecodeFirstString(.nodeId, .nodeIdCamel)
            ?? "task:\(task.threadId)"
        parentNodeId = try container.garyxDecodeFirstString(.parentNodeId, .parentNodeIdCamel)
        parentTaskNumber = try container.garyxDecodeFirstInt(
            .parentTaskNumber, .parentTaskNumberCamel
        )
        parentThreadId = try container.garyxDecodeFirstString(
            .parentThreadId, .parentThreadIdCamel
        )
        activeRunId = try container.garyxDecodeFirstString(.activeRunId, .activeRunIdCamel)
        runState = try container.garyxDecodeFirstString(.runState, .runStateCamel) ?? "idle"
        lastActiveAt = try container.garyxDecodeFirstString(.lastActiveAt, .lastActiveAtCamel)
        depth = try container.garyxDecodeFirstInt(.depth)
    }
}
