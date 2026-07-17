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
    public var changed: Bool?
    public var operationId: String?
    public var outcome: String?
    public var id: String?
    public var taskId: String?
    public var threadId: String?
    public var detachedEndpointKeys: [String]?

    enum CodingKeys: String, CodingKey {
        case deleted
        case changed
        case operationId = "operation_id"
        case outcome
        case id
        case taskId = "task_id"
        case taskIdCamel = "taskId"
        case threadId = "thread_id"
        case threadIdCamel = "threadId"
        case detachedEndpointKeys = "detached_endpoint_keys"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        deleted = try container.decodeIfPresent(Bool.self, forKey: .deleted)
        changed = try container.decodeIfPresent(Bool.self, forKey: .changed)
        operationId = try container.garyxDecodeFirstString(.operationId)
        outcome = try container.decodeIfPresent(String.self, forKey: .outcome)
        id = try container.garyxDecodeFirstString(.id)
        taskId = try container.garyxDecodeFirstString(.taskId, .taskIdCamel)
        threadId = try container.garyxDecodeFirstString(.threadId, .threadIdCamel)
        detachedEndpointKeys = try container.garyxDecodeFirstStringArray(.detachedEndpointKeys)
    }
}


public struct GaryxDeleteThreadRequest: Encodable, Equatable, Sendable {
    public var operationId: String
    public var expectedStoreIncarnation: String

    public init(operationId: String, expectedStoreIncarnation: String) {
        self.operationId = operationId
        self.expectedStoreIncarnation = expectedStoreIncarnation
    }
}


public struct GaryxArchiveThreadRequest: Encodable, Equatable, Sendable {
    public var operationId: String
    public var expectedStoreIncarnation: String
    public var endpointKeys: [String]

    public init(
        operationId: String,
        expectedStoreIncarnation: String,
        endpointKeys: [String] = []
    ) {
        self.operationId = operationId
        self.expectedStoreIncarnation = expectedStoreIncarnation
        self.endpointKeys = endpointKeys
    }
}


public struct GaryxArchiveThreadResult: Decodable, Equatable, Sendable {
    public var archived: Bool?
    public var deleted: Bool?
    public var changed: Bool?
    public var operationId: String?
    public var outcome: String?
    public var threadId: String?
    public var detachedEndpointKeys: [String]?

    enum CodingKeys: String, CodingKey {
        case archived
        case deleted
        case changed
        case operationId = "operation_id"
        case outcome
        case threadId = "thread_id"
        case threadIdCamel = "threadId"
        case detachedEndpointKeys = "detached_endpoint_keys"
        case detachedEndpointKeysCamel = "detachedEndpointKeys"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        archived = try container.decodeIfPresent(Bool.self, forKey: .archived)
        deleted = try container.decodeIfPresent(Bool.self, forKey: .deleted)
        changed = try container.decodeIfPresent(Bool.self, forKey: .changed)
        operationId = try container.garyxDecodeFirstString(.operationId)
        outcome = try container.decodeIfPresent(String.self, forKey: .outcome)
        threadId = try container.garyxDecodeFirstString(.threadId, .threadIdCamel)
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
