import Foundation

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
        from = try container.garyxDecodeFirstString(.from) ?? ""
        to = try container.garyxDecodeFirstString(.to) ?? ""
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
        dreamId = try container.garyxDecodeFirstString(.dreamIdSnake, .dreamId) ?? ""
        title = try container.garyxDecodeFirstString(.title) ?? "Untitled Dream"
        summary = try container.garyxDecodeFirstString(.summary) ?? ""
        firstMessageAt = try container.garyxDecodeFirstString(.firstMessageAtSnake, .firstMessageAt) ?? ""
        lastMessageAt = try container.garyxDecodeFirstString(.lastMessageAtSnake, .lastMessageAt) ?? ""
        updatedAt = try container.garyxDecodeFirstString(.updatedAtSnake, .updatedAt) ?? ""
        source = try container.garyxDecodeFirstString(.source) ?? "unknown"
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
        spanId = try container.garyxDecodeFirstString(.spanIdSnake, .spanId) ?? ""
        dreamId = try container.garyxDecodeFirstString(.dreamIdSnake, .dreamId) ?? ""
        threadId = try container.garyxDecodeFirstString(.threadIdSnake, .threadId) ?? ""
        workspacePath = try container.garyxDecodeFirstString(.workspaceDir, .workspacePath)
        startSeq = try container.decodeIfPresent(Int.self, forKey: .startSeqSnake)
            ?? container.decodeIfPresent(Int.self, forKey: .startSeq)
            ?? 0
        endSeq = try container.decodeIfPresent(Int.self, forKey: .endSeqSnake)
            ?? container.decodeIfPresent(Int.self, forKey: .endSeq)
            ?? 0
        startAt = try container.garyxDecodeFirstString(.startAtSnake, .startAt) ?? ""
        endAt = try container.garyxDecodeFirstString(.endAtSnake, .endAt) ?? ""
        excerpt = try container.garyxDecodeFirstString(.excerpt) ?? ""
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
        runId = try container.garyxDecodeFirstString(.runIdSnake, .runId) ?? ""
        scannedFrom = try container.garyxDecodeFirstString(.scannedFromSnake, .scannedFrom) ?? ""
        scannedTo = try container.garyxDecodeFirstString(.scannedToSnake, .scannedTo) ?? ""
        createdAt = try container.garyxDecodeFirstString(.createdAtSnake, .createdAt) ?? ""
        source = try container.garyxDecodeFirstString(.source) ?? "unknown"
        status = try container.garyxDecodeFirstString(.status) ?? "unknown"
        topicsCount = try container.decodeIfPresent(Int.self, forKey: .topicsCountSnake)
            ?? container.decodeIfPresent(Int.self, forKey: .topicsCount)
            ?? 0
        spansCount = try container.decodeIfPresent(Int.self, forKey: .spansCountSnake)
            ?? container.decodeIfPresent(Int.self, forKey: .spansCount)
            ?? 0
        error = try container.garyxDecodeFirstString(.error)
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
