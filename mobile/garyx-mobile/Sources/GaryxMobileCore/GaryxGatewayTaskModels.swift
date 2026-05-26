import Foundation

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
        hasMore = try container.garyxDecodeFirstBool(.hasMore, .hasMoreCamel) ?? false
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
        updatedBy = try container.decodeIfPresent(GaryxTaskPrincipal.self, forKey: .updatedBy)
            ?? container.decodeIfPresent(GaryxTaskPrincipal.self, forKey: .updatedByCamel)
        runtimeAgentId = try container.garyxDecodeFirstString(.runtimeAgentId, .runtimeAgentIdCamel) ?? ""
        replyCount = try container.decodeIfPresent(Int.self, forKey: .replyCount)
            ?? container.decodeIfPresent(Int.self, forKey: .replyCountCamel)
            ?? 0
        updatedAt = try container.garyxDecodeFirstString(.updatedAt, .updatedAtCamel)
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
