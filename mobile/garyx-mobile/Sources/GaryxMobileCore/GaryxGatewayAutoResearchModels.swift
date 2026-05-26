import Foundation

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
        runId = try container.garyxDecodeFirstString(.runId, .runIdCamel) ?? ""
        state = try container.garyxDecodeFirstString(.state) ?? "queued"
        goal = try container.garyxDecodeFirstString(.goal) ?? ""
        workspaceDir = try container.garyxDecodeFirstString(.workspaceDir, .workspaceDirCamel)
        maxIterations = try container.garyxDecodeFirstInt(.maxIterations, .maxIterationsCamel) ?? 0
        timeBudgetSecs = try container.garyxDecodeFirstInt(.timeBudgetSecs, .timeBudgetSecsCamel)
        iterationsUsed = try container.garyxDecodeFirstInt(.iterationsUsed, .iterationsUsedCamel) ?? 0
        createdAt = try container.garyxDecodeFirstString(.createdAt, .createdAtCamel) ?? ""
        updatedAt = try container.garyxDecodeFirstString(.updatedAt, .updatedAtCamel) ?? ""
        terminalReason = try container.garyxDecodeFirstString(.terminalReason, .terminalReasonCamel)
        selectedCandidate = try container.garyxDecodeFirstString(.selectedCandidate, .selectedCandidateCamel)
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
        feedback = try container.garyxDecodeFirstString(.feedback) ?? ""
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
        candidateId = try container.garyxDecodeFirstString(.candidateId, .candidateIdCamel) ?? ""
        iteration = try container.garyxDecodeFirstInt(.iteration) ?? 0
        output = try container.garyxDecodeFirstString(.output) ?? ""
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
        bestCandidateId = try container.garyxDecodeFirstString(.bestCandidateId, .bestCandidateIdSnake)
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
        runId = try container.garyxDecodeFirstString(.runId, .runIdCamel) ?? ""
        iterationIndex = try container.garyxDecodeFirstInt(.iterationIndex, .iterationIndexCamel) ?? 0
        state = try container.garyxDecodeFirstString(.state) ?? ""
        workThreadId = try container.garyxDecodeFirstString(.workThreadId, .workThreadIdCamel)
        verifyThreadId = try container.garyxDecodeFirstString(.verifyThreadId, .verifyThreadIdCamel)
        startedAt = try container.garyxDecodeFirstString(.startedAt, .startedAtCamel) ?? ""
        completedAt = try container.garyxDecodeFirstString(.completedAt, .completedAtCamel)
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
        activeThreadId = try container.garyxDecodeFirstString(.activeThreadId, .activeThreadIdCamel)
    }
}
