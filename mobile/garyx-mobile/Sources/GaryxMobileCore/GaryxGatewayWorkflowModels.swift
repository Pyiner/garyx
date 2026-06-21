import Foundation

public struct GaryxWorkflowRunDrilldown: Decodable, Equatable, Sendable {
    public var workflow: GaryxWorkflowRunRecord
    public var children: [GaryxWorkflowChildRunRecord]
    public var events: [GaryxWorkflowEventRecord]
    public var presentation: GaryxWorkflowPresentation

    public init(
        workflow: GaryxWorkflowRunRecord,
        children: [GaryxWorkflowChildRunRecord] = [],
        events: [GaryxWorkflowEventRecord] = [],
        presentation: GaryxWorkflowPresentation
    ) {
        self.workflow = workflow
        self.children = children
        self.events = events
        self.presentation = presentation
    }
}

public struct GaryxWorkflowRunRecord: Decodable, Equatable, Sendable {
    public var workflowRunId: String
    public var threadId: String
    public var workflowDefinitionId: String?
    public var taskId: String?
    public var taskThreadId: String?
    public var name: String
    public var description: String?
    public var status: String
    public var currentPhaseIndex: Int?
    public var outputText: String?
    public var result: GaryxJSONValue?
    public var error: String?
    public var workspaceDir: String?
    public var totalChildren: Int
    public var completedChildren: Int
    public var failedChildren: Int
    public var createdAt: String?
    public var startedAt: String?
    public var finishedAt: String?
    public var updatedAt: String?

    enum CodingKeys: String, CodingKey {
        case workflowRunId
        case workflowRunIdSnake = "workflow_run_id"
        case workflowId
        case workflowIdSnake = "workflow_id"
        case threadId
        case threadIdSnake = "thread_id"
        case workflowDefinitionId
        case workflowDefinitionIdSnake = "workflow_definition_id"
        case taskId
        case taskIdSnake = "task_id"
        case taskThreadId
        case taskThreadIdSnake = "task_thread_id"
        case name
        case title
        case description
        case status
        case currentPhaseIndex
        case currentPhaseIndexSnake = "current_phase_index"
        case outputText
        case outputTextSnake = "output_text"
        case result
        case error
        case workspaceDir
        case workspaceDirSnake = "workspace_dir"
        case totalChildren
        case totalChildrenSnake = "total_children"
        case completedChildren
        case completedChildrenSnake = "completed_children"
        case failedChildren
        case failedChildrenSnake = "failed_children"
        case createdAt
        case createdAtSnake = "created_at"
        case startedAt
        case startedAtSnake = "started_at"
        case finishedAt
        case finishedAtSnake = "finished_at"
        case updatedAt
        case updatedAtSnake = "updated_at"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        workflowRunId = try container.garyxDecodeFirstString(
            .workflowRunId,
            .workflowRunIdSnake,
            .workflowId,
            .workflowIdSnake,
            .threadId,
            .threadIdSnake
        ) ?? ""
        threadId = try container.garyxDecodeFirstString(.threadId, .threadIdSnake) ?? workflowRunId
        workflowDefinitionId = try container.garyxDecodeFirstString(.workflowDefinitionId, .workflowDefinitionIdSnake)
        taskId = try container.garyxDecodeFirstString(.taskId, .taskIdSnake)
        taskThreadId = try container.garyxDecodeFirstString(.taskThreadId, .taskThreadIdSnake)
        name = try container.garyxDecodeFirstString(.name, .title) ?? "Workflow run"
        description = try container.garyxDecodeFirstString(.description)
        status = try container.garyxDecodeFirstString(.status) ?? "queued"
        currentPhaseIndex = try container.decodeIfPresent(Int.self, forKey: .currentPhaseIndex)
            ?? container.decodeIfPresent(Int.self, forKey: .currentPhaseIndexSnake)
        outputText = try container.garyxDecodeFirstString(.outputText, .outputTextSnake)
        result = try container.decodeIfPresent(GaryxJSONValue.self, forKey: .result)
        error = try container.garyxDecodeFirstString(.error)
        workspaceDir = try container.garyxDecodeFirstString(.workspaceDir, .workspaceDirSnake)
        totalChildren = try container.garyxDecodeFirstInt(.totalChildren, .totalChildrenSnake) ?? 0
        completedChildren = try container.garyxDecodeFirstInt(.completedChildren, .completedChildrenSnake) ?? 0
        failedChildren = try container.garyxDecodeFirstInt(.failedChildren, .failedChildrenSnake) ?? 0
        createdAt = try container.garyxDecodeFirstString(.createdAt, .createdAtSnake)
        startedAt = try container.garyxDecodeFirstString(.startedAt, .startedAtSnake)
        finishedAt = try container.garyxDecodeFirstString(.finishedAt, .finishedAtSnake)
        updatedAt = try container.garyxDecodeFirstString(.updatedAt, .updatedAtSnake)
    }
}

public struct GaryxWorkflowChildRunRecord: Decodable, Equatable, Sendable {
    public var workflowChildRunId: String
    public var threadId: String
    public var phaseIndex: Int
    public var phaseTitle: String
    public var label: String
    public var agentId: String?
    public var status: String
    public var resultPreview: String?
    public var error: String?
    public var updatedAt: String?

    enum CodingKeys: String, CodingKey {
        case workflowChildRunId
        case workflowChildRunIdSnake = "workflow_child_run_id"
        case threadId
        case threadIdSnake = "thread_id"
        case phaseIndex
        case phaseIndexSnake = "phase_index"
        case phaseTitle
        case phaseTitleSnake = "phase_title"
        case label
        case agentId
        case agentIdSnake = "agent_id"
        case status
        case resultPreview
        case resultPreviewSnake = "result_preview"
        case error
        case updatedAt
        case updatedAtSnake = "updated_at"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        workflowChildRunId = try container.garyxDecodeFirstString(.workflowChildRunId, .workflowChildRunIdSnake) ?? ""
        threadId = try container.garyxDecodeFirstString(.threadId, .threadIdSnake) ?? ""
        phaseIndex = try container.garyxDecodeFirstInt(.phaseIndex, .phaseIndexSnake) ?? 0
        phaseTitle = try container.garyxDecodeFirstString(.phaseTitle, .phaseTitleSnake) ?? "Phase"
        label = try container.garyxDecodeFirstString(.label) ?? "Child run"
        agentId = try container.garyxDecodeFirstString(.agentId, .agentIdSnake)
        status = try container.garyxDecodeFirstString(.status) ?? "queued"
        resultPreview = try container.garyxDecodeFirstString(.resultPreview, .resultPreviewSnake)
        error = try container.garyxDecodeFirstString(.error)
        updatedAt = try container.garyxDecodeFirstString(.updatedAt, .updatedAtSnake)
    }
}

public struct GaryxWorkflowEventRecord: Decodable, Equatable, Sendable {
    public var eventSeq: UInt64
    public var eventId: String
    public var eventType: String
    public var payload: GaryxJSONValue?
    public var createdAt: String?

    enum CodingKeys: String, CodingKey {
        case eventSeq
        case eventSeqSnake = "event_seq"
        case eventId
        case eventIdSnake = "event_id"
        case eventType
        case eventTypeSnake = "event_type"
        case payload
        case createdAt
        case createdAtSnake = "created_at"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        eventSeq = UInt64(try container.garyxDecodeFirstInt(.eventSeq, .eventSeqSnake) ?? 0)
        eventId = try container.garyxDecodeFirstString(.eventId, .eventIdSnake) ?? ""
        eventType = try container.garyxDecodeFirstString(.eventType, .eventTypeSnake) ?? ""
        payload = try container.decodeIfPresent(GaryxJSONValue.self, forKey: .payload)
        createdAt = try container.garyxDecodeFirstString(.createdAt, .createdAtSnake)
    }
}

public struct GaryxWorkflowPresentation: Decodable, Equatable, Sendable {
    public var version: UInt64
    public var workflowRunId: String
    public var threadId: String
    public var workflowDefinitionId: String?
    public var taskId: String?
    public var taskThreadId: String?
    public var title: String
    public var description: String?
    public var status: String
    public var counts: GaryxWorkflowCounts
    public var activePhase: GaryxWorkflowPhaseIdentity?
    public var phaseStatus: [GaryxWorkflowPhaseStatus]
    public var phases: [GaryxWorkflowPhase]
    public var childCards: [GaryxWorkflowChildCard]
    public var outcome: GaryxWorkflowOutcome
    public var outputText: String?
    public var result: GaryxJSONValue?
    public var error: String?
    public var terminalComplete: Bool
    public var stale: Bool?
    public var staleReason: String?
    public var snapshotVersion: UInt64
    public var latestEventSeq: UInt64
    public var eventsSeed: GaryxWorkflowEventsSeed

    public var progressFraction: Double {
        guard counts.total > 0 else { return terminalComplete ? 1 : 0 }
        return min(1, max(0, Double(counts.completed) / Double(counts.total)))
    }
}

public struct GaryxWorkflowCounts: Decodable, Equatable, Sendable {
    public var total: Int
    public var completed: Int
    public var failedChildren: Int
    public var runningChildren: Int
    public var queuedChildren: Int
    public var skippedChildren: Int
    public var totalPhases: Int
    public var completedPhases: Int
    public var totalInputTokens: Int
    public var totalOutputTokens: Int
    public var totalToolCalls: Int
    public var costUsd: Double
}

public struct GaryxWorkflowPhaseIdentity: Decodable, Equatable, Sendable {
    public var phaseId: String
    public var index: Int?
    public var title: String
    public var detail: String?
}

public struct GaryxWorkflowPhaseStatus: Decodable, Equatable, Sendable {
    public var phaseId: String
    public var index: Int?
    public var title: String
    public var status: String
    public var active: Bool
    public var completedChildren: Int
    public var totalChildren: Int
    public var failedChildren: Int
}

public struct GaryxWorkflowPhase: Decodable, Equatable, Sendable {
    public var phaseId: String
    public var index: Int?
    public var title: String
    public var detail: String?
    public var status: String
    public var active: Bool
    public var counts: GaryxWorkflowPhaseCounts
    public var children: [GaryxWorkflowChildCard]
}

public struct GaryxWorkflowPhaseCounts: Decodable, Equatable, Sendable {
    public var completed: Int
    public var total: Int
    public var failedChildren: Int
}

public struct GaryxWorkflowChildCard: Decodable, Identifiable, Equatable, Sendable {
    public var workflowChildRunId: String
    public var threadId: String
    public var phaseIndex: Int
    public var phaseTitle: String
    public var label: String
    public var agentId: String?
    public var status: String
    public var prompt: String?
    public var resultMode: String?
    public var resultText: String?
    public var result: GaryxJSONValue?
    public var resultPreview: String?
    public var error: String?
    public var inputTokens: Int
    public var outputTokens: Int
    public var tokens: Int
    public var toolCalls: Int
    public var costUsd: Double
    public var queuedAt: String?
    public var startedAt: String?
    public var finishedAt: String?
    public var updatedAt: String?

    public var id: String { workflowChildRunId.isEmpty ? threadId : workflowChildRunId }
}

public struct GaryxWorkflowOutcome: Decodable, Equatable, Sendable {
    public var kind: String
    public var status: String
    public var hasOutputText: Bool
    public var hasResult: Bool
    public var error: String?
}

public struct GaryxWorkflowEventsSeed: Decodable, Equatable, Sendable {
    public var count: Int
    public var latestSeedEventSeq: UInt64
    public var truncated: Bool
}

public enum GaryxWorkflowStatusPresentation {
    public static func isTerminal(_ status: String) -> Bool {
        switch normalized(status) {
        case "succeeded", "failed", "cancelled", "skipped":
            return true
        default:
            return false
        }
    }

    public static func label(for status: String) -> String {
        switch normalized(status) {
        case "in_progress", "running":
            return "Running"
        case "succeeded":
            return "Succeeded"
        case "failed":
            return "Failed"
        case "cancelled":
            return "Cancelled"
        case "skipped":
            return "Skipped"
        case "queued":
            return "Queued"
        default:
            return status.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "Unknown" : status
        }
    }

    public static func normalized(_ status: String) -> String {
        status.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    }
}
