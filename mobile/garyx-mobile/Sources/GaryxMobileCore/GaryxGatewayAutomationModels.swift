import Foundation

public struct GaryxAutomationsPage: Decodable, Equatable, Sendable {
    public var automations: [GaryxAutomationSummary]
}

public enum GaryxAutomationAgentResolution: String, Codable, Equatable, Sendable {
    case resolved
    case followThread = "follow_thread"
    case targetMissing = "target_missing"
}

public enum GaryxAutomationValidationState: String, Codable, Equatable, Sendable {
    case valid
    case invalid
}

public struct GaryxAutomationSummary: Decodable, Identifiable, Equatable, Sendable {
    public var id: String
    public var label: String
    public var prompt: String
    public var agentId: String?
    public var agentResolution: GaryxAutomationAgentResolution
    public var effectiveAgentId: String?
    public var enabled: Bool
    public var workspacePath: String
    public var targetThreadId: String?
    public var threadId: String?
    public var threadMode: String
    public var nextRun: String
    public var lastRunAt: String?
    public var lastStatus: String
    public var schedule: GaryxAutomationSchedule
    public var validationState: GaryxAutomationValidationState
    public var validationError: String?

    public init(
        id: String,
        label: String,
        prompt: String,
        agentId: String?,
        agentResolution: GaryxAutomationAgentResolution = .resolved,
        effectiveAgentId: String? = nil,
        enabled: Bool = true,
        workspacePath: String,
        targetThreadId: String? = nil,
        threadId: String? = nil,
        threadMode: String = "target",
        nextRun: String = "",
        lastRunAt: String? = nil,
        lastStatus: String = "success",
        schedule: GaryxAutomationSchedule = .interval(hours: 24),
        validationState: GaryxAutomationValidationState = .valid,
        validationError: String? = nil
    ) {
        self.id = id
        self.label = label
        self.prompt = prompt
        self.agentId = agentId
        self.agentResolution = agentResolution
        self.effectiveAgentId = effectiveAgentId
        self.enabled = enabled
        self.workspacePath = workspacePath
        self.targetThreadId = targetThreadId
        self.threadId = threadId
        self.threadMode = threadMode
        self.nextRun = nextRun
        self.lastRunAt = lastRunAt
        self.lastStatus = lastStatus
        self.schedule = schedule
        self.validationState = validationState
        self.validationError = validationError
    }

    enum CodingKeys: String, CodingKey {
        case id
        case label
        case prompt
        case agentId = "agent_id"
        case agentIdCamel = "agentId"
        case agentResolution = "agent_resolution"
        case agentResolutionCamel = "agentResolution"
        case effectiveAgentId = "effective_agent_id"
        case effectiveAgentIdCamel = "effectiveAgentId"
        case enabled
        case workspaceDir = "workspace_dir"
        case workspaceDirCamel = "workspaceDir"
        case targetThreadId = "target_thread_id"
        case targetThreadIdCamel = "targetThreadId"
        case threadId = "thread_id"
        case threadIdCamel = "threadId"
        case threadMode = "thread_mode"
        case threadModeCamel = "threadMode"
        case nextRun = "next_run"
        case nextRunCamel = "nextRun"
        case lastRunAt = "last_run_at"
        case lastRunAtCamel = "lastRunAt"
        case lastStatus = "last_status"
        case lastStatusCamel = "lastStatus"
        case schedule
        case validationState = "validation_state"
        case validationStateCamel = "validationState"
        case validationError = "validation_error"
        case validationErrorCamel = "validationError"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.garyxDecodeFirstString(.id) ?? ""
        label = try container.garyxDecodeFirstString(.label) ?? id
        prompt = try container.garyxDecodeFirstString(.prompt) ?? ""
        agentId = try container.garyxDecodeFirstString(.agentId, .agentIdCamel)
        agentResolution = try container.garyxDecodeFirstString(.agentResolution, .agentResolutionCamel)
            .flatMap(GaryxAutomationAgentResolution.init(rawValue:)) ?? .resolved
        effectiveAgentId = try container.garyxDecodeFirstString(.effectiveAgentId, .effectiveAgentIdCamel)
        enabled = try container.decodeIfPresent(Bool.self, forKey: .enabled) ?? true
        workspacePath = try container.garyxDecodeFirstString(.workspaceDir, .workspaceDirCamel) ?? ""
        targetThreadId = try container.garyxDecodeFirstString(.targetThreadId, .targetThreadIdCamel)
        threadId = try container.garyxDecodeFirstString(.threadId, .threadIdCamel)
        threadMode = try container.garyxDecodeFirstString(.threadMode, .threadModeCamel)
            ?? (targetThreadId == nil ? "generated" : "target")
        nextRun = try container.garyxDecodeFirstString(.nextRun, .nextRunCamel) ?? ""
        lastRunAt = try container.garyxDecodeFirstString(.lastRunAt, .lastRunAtCamel)
        lastStatus = try container.garyxDecodeFirstString(.lastStatus, .lastStatusCamel) ?? "success"
        schedule = try container.decodeIfPresent(GaryxAutomationSchedule.self, forKey: .schedule) ?? .interval(hours: 24)
        validationState = try container.garyxDecodeFirstString(.validationState, .validationStateCamel)
            .flatMap(GaryxAutomationValidationState.init(rawValue:)) ?? .valid
        validationError = try container.garyxDecodeFirstString(.validationError, .validationErrorCamel)
    }

    public var isGeneratedThreadMode: Bool {
        threadMode.trimmingCharacters(in: .whitespacesAndNewlines) == "generated"
    }
}


public struct GaryxAutomationSchedule: Codable, Equatable, Sendable {
    public enum Kind: String, Codable, Equatable, Sendable {
        case daily
        case interval
        case monthly
        case once
    }

    public var kind: Kind
    public var time: String?
    public var weekdays: [String]
    public var timezone: String?
    public var hours: Int?
    public var at: String?
    public var day: Int?

    public static func daily(
        time: String = "09:00",
        weekdays: [String] = ["mo", "tu", "we", "th", "fr"],
        timezone: String = "UTC"
    ) -> Self {
        Self(kind: .daily, time: time, weekdays: weekdays, timezone: timezone, hours: nil, at: nil, day: nil)
    }

    public static func interval(hours: Int = 24) -> Self {
        Self(kind: .interval, time: nil, weekdays: [], timezone: nil, hours: max(1, hours), at: nil, day: nil)
    }

    public static func monthly(
        day: Int = 1,
        time: String = "09:00",
        timezone: String = "UTC"
    ) -> Self {
        Self(kind: .monthly, time: time, weekdays: [], timezone: timezone, hours: nil, at: nil, day: min(max(day, 1), 31))
    }

    public static func once(at: String) -> Self {
        Self(kind: .once, time: nil, weekdays: [], timezone: nil, hours: nil, at: at, day: nil)
    }

    public init(
        kind: Kind,
        time: String? = nil,
        weekdays: [String] = [],
        timezone: String? = nil,
        hours: Int? = nil,
        at: String? = nil,
        day: Int? = nil
    ) {
        self.kind = kind
        self.time = time
        self.weekdays = weekdays
        self.timezone = timezone
        self.hours = hours
        self.at = at
        self.day = day
    }

    enum CodingKeys: String, CodingKey {
        case kind
        case time
        case weekdays
        case timezone
        case hours
        case at
        case day
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        kind = try container.decodeIfPresent(Kind.self, forKey: .kind) ?? .interval
        time = try container.decodeIfPresent(String.self, forKey: .time)
        weekdays = try container.decodeIfPresent([String].self, forKey: .weekdays) ?? []
        timezone = try container.decodeIfPresent(String.self, forKey: .timezone)
        hours = try container.decodeIfPresent(Int.self, forKey: .hours)
        at = try container.decodeIfPresent(String.self, forKey: .at)
        day = try container.decodeIfPresent(Int.self, forKey: .day)
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
        case .monthly:
            try container.encode(min(max(day ?? 1, 1), 31), forKey: .day)
            try container.encode(time ?? "09:00", forKey: .time)
            try container.encode(timezone ?? "UTC", forKey: .timezone)
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
        runId = try container.garyxDecodeFirstString(.runId, .runIdCamel) ?? ""
        status = try container.garyxDecodeFirstString(.status) ?? "success"
        startedAt = try container.garyxDecodeFirstString(.startedAt, .startedAtCamel) ?? ""
        excerpt = try container.garyxDecodeFirstString(.excerpt)
        threadId = try container.garyxDecodeFirstString(.threadId, .threadIdCamel) ?? ""
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
        threadId = try container.garyxDecodeFirstString(.threadId, .threadIdSnake) ?? ""
        count = try container.decodeIfPresent(Int.self, forKey: .count) ?? items.count
    }
}

public struct GaryxAutomationThreadEntry: Decodable, Identifiable, Equatable, Sendable {
    public var automationId: String
    public var runId: String
    public var threadId: String
    public var workspacePath: String?
    public var agentId: String?
    public var automationLabel: String
    public var automationDeleted: Bool
    public var status: String
    public var startedAt: String
    public var finishedAt: String?
    public var thread: GaryxThreadSummary?

    public var id: String {
        runId.isEmpty ? threadId : runId
    }

    enum CodingKeys: String, CodingKey {
        case automationId = "automation_id"
        case automationIdCamel = "automationId"
        case runId = "run_id"
        case runIdCamel = "runId"
        case threadId = "thread_id"
        case threadIdCamel = "threadId"
        case workspaceDir = "workspace_dir"
        case workspaceDirCamel = "workspaceDir"
        case agentId = "agent_id"
        case agentIdCamel = "agentId"
        case automationLabel = "automation_label"
        case automationLabelCamel = "automationLabel"
        case automationDeleted = "automation_deleted"
        case automationDeletedCamel = "automationDeleted"
        case status
        case startedAt = "started_at"
        case startedAtCamel = "startedAt"
        case finishedAt = "finished_at"
        case finishedAtCamel = "finishedAt"
        case thread
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        automationId = try container.garyxDecodeFirstString(.automationId, .automationIdCamel) ?? ""
        runId = try container.garyxDecodeFirstString(.runId, .runIdCamel) ?? ""
        threadId = try container.garyxDecodeFirstString(.threadId, .threadIdCamel) ?? ""
        workspacePath = try container.garyxDecodeFirstString(.workspaceDir, .workspaceDirCamel)
        agentId = try container.garyxDecodeFirstString(.agentId, .agentIdCamel)
        automationLabel = try container.garyxDecodeFirstString(.automationLabel, .automationLabelCamel) ?? automationId
        automationDeleted = try container.decodeIfPresent(Bool.self, forKey: .automationDeletedCamel)
            ?? container.decodeIfPresent(Bool.self, forKey: .automationDeleted)
            ?? false
        status = try container.garyxDecodeFirstString(.status) ?? "success"
        startedAt = try container.garyxDecodeFirstString(.startedAt, .startedAtCamel) ?? ""
        finishedAt = try container.garyxDecodeFirstString(.finishedAt, .finishedAtCamel)
        thread = try container
            .decodeIfPresent(GaryxAutomationThreadSummaryDTO.self, forKey: .thread)
            .map(GaryxThreadSummaryAdapter.summary)
    }
}

public struct GaryxAutomationThreadsPage: Decodable, Equatable, Sendable {
    public var automationId: String
    public var automationLabel: String
    public var automationDeleted: Bool
    public var items: [GaryxAutomationThreadEntry]
    public var count: Int
    public var total: Int
    public var limit: Int
    public var offset: Int
    public var hasMore: Bool

    enum CodingKeys: String, CodingKey {
        case automationId = "automation_id"
        case automationIdCamel = "automationId"
        case automationLabel = "automation_label"
        case automationLabelCamel = "automationLabel"
        case automationDeleted = "automation_deleted"
        case automationDeletedCamel = "automationDeleted"
        case items
        case count
        case total
        case limit
        case offset
        case hasMore = "has_more"
        case hasMoreCamel = "hasMore"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        automationId = try container.garyxDecodeFirstString(.automationId, .automationIdCamel) ?? ""
        automationLabel = try container.garyxDecodeFirstString(.automationLabel, .automationLabelCamel) ?? automationId
        automationDeleted = try container.decodeIfPresent(Bool.self, forKey: .automationDeletedCamel)
            ?? container.decodeIfPresent(Bool.self, forKey: .automationDeleted)
            ?? false
        items = try container.decodeIfPresent([GaryxAutomationThreadEntry].self, forKey: .items) ?? []
        count = try container.decodeIfPresent(Int.self, forKey: .count) ?? items.count
        total = try container.decodeIfPresent(Int.self, forKey: .total) ?? count
        limit = try container.decodeIfPresent(Int.self, forKey: .limit) ?? count
        offset = try container.decodeIfPresent(Int.self, forKey: .offset) ?? 0
        hasMore = try container.decodeIfPresent(Bool.self, forKey: .hasMoreCamel)
            ?? container.decodeIfPresent(Bool.self, forKey: .hasMore)
            ?? (offset + count < total)
    }
}
