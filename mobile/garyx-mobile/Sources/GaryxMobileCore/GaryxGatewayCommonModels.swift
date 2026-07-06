import Foundation

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
        id = try container.garyxDecodeFirstString(.id)
        taskId = try container.garyxDecodeFirstString(.taskId, .taskIdCamel)
        threadId = try container.garyxDecodeFirstString(.threadId, .threadIdCamel)
    }
}


public struct GaryxArchiveThreadRequest: Encodable, Equatable, Sendable {
    public var endpointKeys: [String]

    public init(endpointKeys: [String] = []) {
        self.endpointKeys = endpointKeys
    }
}


public struct GaryxArchiveThreadResult: Decodable, Equatable, Sendable {
    public var archived: Bool?
    public var deleted: Bool?
    public var threadId: String?
    public var staleProjection: Bool?
    public var detachedEndpointKeys: [String]?

    enum CodingKeys: String, CodingKey {
        case archived
        case deleted
        case threadId = "thread_id"
        case threadIdCamel = "threadId"
        case staleProjection = "stale_projection"
        case staleProjectionCamel = "staleProjection"
        case detachedEndpointKeys = "detached_endpoint_keys"
        case detachedEndpointKeysCamel = "detachedEndpointKeys"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        archived = try container.decodeIfPresent(Bool.self, forKey: .archived)
        deleted = try container.decodeIfPresent(Bool.self, forKey: .deleted)
        threadId = try container.garyxDecodeFirstString(.threadId, .threadIdCamel)
        staleProjection = try container.garyxDecodeFirstBool(.staleProjection, .staleProjectionCamel)
        detachedEndpointKeys = try container.garyxDecodeFirstStringArray(
            .detachedEndpointKeys,
            .detachedEndpointKeysCamel
        )
    }
}


public struct GaryxEmptyResponse: Decodable, Equatable, Sendable {
    public init() {}
    public init(from decoder: Decoder) throws {
        self.init()
    }
}


public struct GaryxEmptyBody: Encodable, Equatable, Sendable {
    public init() {}
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
        ok = try container.garyxDecodeFirstBool(.ok) ?? false
        message = try container.garyxDecodeFirstString(.message)
        warnings = try container.decodeIfPresent([String].self, forKey: .warnings) ?? []
        errors = try container.decodeIfPresent([String].self, forKey: .errors) ?? []
    }
}
