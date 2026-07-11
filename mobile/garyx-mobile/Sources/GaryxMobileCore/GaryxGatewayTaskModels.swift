import Foundation

public enum GaryxTaskStatus: String, Codable, Equatable, Sendable, CaseIterable {
    case todo
    case inProgress = "in_progress"
    case inReview = "in_review"
    case done
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
    public var executor: GaryxTaskExecutor?
    public var updatedBy: GaryxTaskPrincipal?
    public var runtimeAgentId: String
    public var replyCount: Int
    public var updatedAt: String?

    public init(
        id: String,
        threadId: String,
        number: Int = 0,
        title: String,
        status: GaryxTaskStatus = .todo,
        creator: GaryxTaskPrincipal? = nil,
        assignee: GaryxTaskPrincipal? = nil,
        assigneeLabel: String = "",
        source: GaryxTaskSource? = nil,
        executor: GaryxTaskExecutor? = nil,
        updatedBy: GaryxTaskPrincipal? = nil,
        runtimeAgentId: String = "",
        replyCount: Int = 0,
        updatedAt: String? = nil
    ) {
        self.id = id
        self.threadId = threadId
        self.number = number
        self.title = title
        self.status = status
        self.creator = creator
        self.assignee = assignee
        self.assigneeLabel = assigneeLabel.isEmpty ? assignee?.label ?? "" : assigneeLabel
        self.source = source
        self.executor = executor
        self.updatedBy = updatedBy
        self.runtimeAgentId = runtimeAgentId
        self.replyCount = replyCount
        self.updatedAt = updatedAt
    }

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
        case executor
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
            if let id = try container.garyxDecodeFirstString(.taskId, .taskIdCamel) {
                summary.id = id
            }
            if let threadId = try container.garyxDecodeFirstString(.threadId, .threadIdCamel) {
                summary.threadId = threadId
            }
            if let runtimeAgentId = try container.garyxDecodeFirstString(.runtimeAgentId, .runtimeAgentIdCamel) {
                summary.runtimeAgentId = runtimeAgentId
            }
            if let executor = try container.decodeIfPresent(GaryxTaskExecutor.self, forKey: .executor) {
                summary.executor = executor
            }
            self = summary
            return
        }
        number = try container.decodeIfPresent(Int.self, forKey: .number) ?? 0
        id = try container.garyxDecodeFirstString(.taskId, .taskIdCamel) ?? (number > 0 ? "#TASK-\(number)" : "")
        threadId = try container.garyxDecodeFirstString(.threadId, .threadIdCamel) ?? ""
        title = try container.garyxDecodeFirstString(.title) ?? (number > 0 ? "#TASK-\(number)" : "Untitled task")
        let rawStatus = try container.garyxDecodeFirstString(.status) ?? "todo"
        status = GaryxTaskStatus(rawValue: rawStatus) ?? .todo
        creator = try container.decodeIfPresent(GaryxTaskPrincipal.self, forKey: .creator)
        assignee = try container.decodeIfPresent(GaryxTaskPrincipal.self, forKey: .assignee)
        assigneeLabel = assignee?.label ?? ""
        source = try container.decodeIfPresent(GaryxTaskSource.self, forKey: .source)
        executor = try container.decodeIfPresent(GaryxTaskExecutor.self, forKey: .executor)
        updatedBy = try container.decodeIfPresent(GaryxTaskPrincipal.self, forKey: .updatedBy)
            ?? container.decodeIfPresent(GaryxTaskPrincipal.self, forKey: .updatedByCamel)
        runtimeAgentId = try container.garyxDecodeFirstString(.runtimeAgentId, .runtimeAgentIdCamel) ?? ""
        replyCount = try container.decodeIfPresent(Int.self, forKey: .replyCount)
            ?? container.decodeIfPresent(Int.self, forKey: .replyCountCamel)
            ?? 0
        updatedAt = try container.garyxDecodeFirstString(.updatedAt, .updatedAtCamel)
    }
}


public struct GaryxTaskExecutor: Decodable, Equatable, Sendable {
    public var type: String
    public var agentId: String?

    public init(
        type: String,
        agentId: String? = nil
    ) {
        self.type = type
        self.agentId = agentId
    }

    enum CodingKeys: String, CodingKey {
        case type
        case agentId = "agent_id"
        case agentIdCamel = "agentId"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        type = try container.garyxDecodeFirstString(.type) ?? ""
        agentId = try container.garyxDecodeFirstString(.agentId, .agentIdCamel)
    }
}


public struct GaryxTaskSource: Decodable, Equatable, Sendable {
    public var threadId: String?
    public var taskId: String?
    public var taskThreadId: String?
    public var botId: String?
    public var channel: String?
    public var accountId: String?

    public init(
        threadId: String? = nil,
        taskId: String? = nil,
        taskThreadId: String? = nil,
        botId: String? = nil,
        channel: String? = nil,
        accountId: String? = nil
    ) {
        self.threadId = threadId
        self.taskId = taskId
        self.taskThreadId = taskThreadId
        self.botId = botId
        self.channel = channel
        self.accountId = accountId
    }

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
        threadId = try container.garyxDecodeFirstString(.threadId, .threadIdCamel)
        taskId = try container.garyxDecodeFirstString(.taskId, .taskIdCamel)
        taskThreadId = try container.garyxDecodeFirstString(.taskThreadId, .taskThreadIdCamel)
        botId = try container.garyxDecodeFirstString(.botId, .botIdCamel)
        channel = try container.garyxDecodeFirstString(.channel)
        accountId = try container.garyxDecodeFirstString(.accountId, .accountIdCamel)
    }
}


public struct GaryxTaskPrincipal: Decodable, Equatable, Sendable {
    public var kind: String
    public var agentId: String?
    public var userId: String?

    public init(kind: String, agentId: String? = nil, userId: String? = nil) {
        self.kind = kind
        self.agentId = agentId
        self.userId = userId
    }

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
        kind = try container.garyxDecodeFirstString(.kind) ?? ""
        agentId = try container.garyxDecodeFirstString(.agentId, .agentIdCamel)
        userId = try container.garyxDecodeFirstString(.userId, .userIdCamel)
    }
}
