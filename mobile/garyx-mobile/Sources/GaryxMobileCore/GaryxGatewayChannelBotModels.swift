import Foundation

public struct GaryxChannelEndpointsPage: Decodable, Equatable, Sendable {
    public var endpoints: [GaryxChannelEndpoint]
}


public struct GaryxChannelEndpoint: Decodable, Identifiable, Equatable, Sendable {
    public var id: String { endpointKey }
    public var endpointKey: String
    public var channel: String
    public var accountId: String
    public var displayLabel: String
    public var threadId: String?
    public var threadLabel: String?
    public var workspaceDir: String?
    public var lastInboundAt: String?
    public var lastDeliveryAt: String?
    public var conversationKind: String?
    public var conversationLabel: String?

    public init(
        endpointKey: String,
        channel: String,
        accountId: String,
        displayLabel: String,
        threadId: String? = nil,
        threadLabel: String? = nil,
        workspaceDir: String? = nil,
        lastInboundAt: String? = nil,
        lastDeliveryAt: String? = nil,
        conversationKind: String? = nil,
        conversationLabel: String? = nil
    ) {
        self.endpointKey = endpointKey
        self.channel = channel
        self.accountId = accountId
        self.displayLabel = displayLabel
        self.threadId = threadId
        self.threadLabel = threadLabel
        self.workspaceDir = workspaceDir
        self.lastInboundAt = lastInboundAt
        self.lastDeliveryAt = lastDeliveryAt
        self.conversationKind = conversationKind
        self.conversationLabel = conversationLabel
    }

    enum CodingKeys: String, CodingKey {
        case endpointKey = "endpoint_key"
        case endpointKeyCamel = "endpointKey"
        case channel
        case accountId = "account_id"
        case accountIdCamel = "accountId"
        case displayLabel = "display_label"
        case displayLabelCamel = "displayLabel"
        case threadId = "thread_id"
        case threadIdCamel = "threadId"
        case threadLabel = "thread_label"
        case threadLabelCamel = "threadLabel"
        case workspaceDir = "workspace_dir"
        case workspaceDirCamel = "workspaceDir"
        case lastInboundAt = "last_inbound_at"
        case lastInboundAtCamel = "lastInboundAt"
        case lastDeliveryAt = "last_delivery_at"
        case lastDeliveryAtCamel = "lastDeliveryAt"
        case conversationKind = "conversation_kind"
        case conversationKindCamel = "conversationKind"
        case conversationLabel = "conversation_label"
        case conversationLabelCamel = "conversationLabel"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        endpointKey = try container.garyxDecodeFirstString(.endpointKey, .endpointKeyCamel) ?? ""
        channel = try container.garyxDecodeFirstString(.channel) ?? ""
        accountId = try container.garyxDecodeFirstString(.accountId, .accountIdCamel) ?? ""
        displayLabel = try container.garyxDecodeFirstString(.displayLabel, .displayLabelCamel) ?? endpointKey
        threadId = try container.garyxDecodeFirstString(.threadId, .threadIdCamel)
        threadLabel = try container.garyxDecodeFirstString(.threadLabel, .threadLabelCamel)
        workspaceDir = try container.garyxDecodeFirstString(.workspaceDir, .workspaceDirCamel)
        lastInboundAt = try container.garyxDecodeFirstString(.lastInboundAt, .lastInboundAtCamel)
        lastDeliveryAt = try container.garyxDecodeFirstString(.lastDeliveryAt, .lastDeliveryAtCamel)
        conversationKind = try container.garyxDecodeFirstString(.conversationKind, .conversationKindCamel)
        conversationLabel = try container.garyxDecodeFirstString(.conversationLabel, .conversationLabelCamel)
    }
}


public struct GaryxConfiguredBotsPage: Decodable, Equatable, Sendable {
    public var bots: [GaryxConfiguredBot]
}


public struct GaryxConfiguredBot: Decodable, Identifiable, Equatable, Sendable {
    public var id: String { "\(channel):\(accountId)" }
    public var channel: String
    public var accountId: String
    public var displayName: String
    public var enabled: Bool
    public var agentId: String?
    public var effectiveAgentId: String?
    public var workspaceDir: String?
    public var workspaceMode: String?
    public var rootBehavior: String
    public var mainEndpointStatus: String
    public var mainThreadId: String?
    public var defaultOpenThreadId: String?

    public init(
        channel: String,
        accountId: String,
        displayName: String,
        enabled: Bool = true,
        agentId: String? = nil,
        effectiveAgentId: String? = nil,
        workspaceDir: String? = nil,
        workspaceMode: String? = nil,
        rootBehavior: String = "open_default",
        mainEndpointStatus: String = "unresolved",
        mainThreadId: String? = nil,
        defaultOpenThreadId: String? = nil
    ) {
        self.channel = channel
        self.accountId = accountId
        self.displayName = displayName
        self.enabled = enabled
        self.agentId = agentId
        self.effectiveAgentId = effectiveAgentId
        self.workspaceDir = workspaceDir
        self.workspaceMode = workspaceMode
        self.rootBehavior = rootBehavior
        self.mainEndpointStatus = mainEndpointStatus
        self.mainThreadId = mainThreadId
        self.defaultOpenThreadId = defaultOpenThreadId
    }

    enum CodingKeys: String, CodingKey {
        case channel
        case accountId = "account_id"
        case accountIdCamel = "accountId"
        case displayName = "display_name"
        case displayNameCamel = "displayName"
        case name
        case enabled
        case agentId = "agent_id"
        case agentIdCamel = "agentId"
        case effectiveAgentId = "effective_agent_id"
        case effectiveAgentIdCamel = "effectiveAgentId"
        case workspaceDir = "workspace_dir"
        case workspaceDirCamel = "workspaceDir"
        case workspaceMode = "workspace_mode"
        case workspaceModeCamel = "workspaceMode"
        case rootBehavior = "root_behavior"
        case rootBehaviorCamel = "rootBehavior"
        case mainEndpointStatus = "main_endpoint_status"
        case mainEndpointStatusCamel = "mainEndpointStatus"
        case mainThreadId = "main_endpoint_thread_id"
        case mainThreadIdCamel = "mainEndpointThreadId"
        case defaultOpenThreadId = "default_open_thread_id"
        case defaultOpenThreadIdCamel = "defaultOpenThreadId"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        channel = try container.garyxDecodeFirstString(.channel) ?? ""
        accountId = try container.garyxDecodeFirstString(.accountId, .accountIdCamel) ?? ""
        displayName = try container.garyxDecodeFirstString(.displayName, .displayNameCamel, .name) ?? "\(channel):\(accountId)"
        enabled = try container.decodeIfPresent(Bool.self, forKey: .enabled) ?? true
        agentId = try container.garyxDecodeFirstString(.agentId, .agentIdCamel)
        effectiveAgentId = try container.garyxDecodeFirstString(.effectiveAgentId, .effectiveAgentIdCamel)
        workspaceDir = try container.garyxDecodeFirstString(.workspaceDir, .workspaceDirCamel)
        workspaceMode = try container.garyxDecodeFirstString(.workspaceMode, .workspaceModeCamel)
        rootBehavior = try container.garyxDecodeFirstString(.rootBehavior, .rootBehaviorCamel) ?? "open_default"
        mainEndpointStatus = try container.garyxDecodeFirstString(.mainEndpointStatus, .mainEndpointStatusCamel) ?? "unresolved"
        mainThreadId = try container.garyxDecodeFirstString(.mainThreadId, .mainThreadIdCamel)
        defaultOpenThreadId = try container.garyxDecodeFirstString(.defaultOpenThreadId, .defaultOpenThreadIdCamel)
    }
}


public struct GaryxBotConsolesPage: Decodable, Equatable, Sendable {
    public var bots: [GaryxBotConsoleSummary]
}


public struct GaryxBotConversationNode: Decodable, Identifiable, Equatable, Sendable {
    public var id: String
    public var endpoint: GaryxChannelEndpoint
    public var kind: String
    public var title: String
    public var badge: String?
    public var latestActivity: String?
    public var openable: Bool

    public init(
        id: String,
        endpoint: GaryxChannelEndpoint,
        kind: String,
        title: String,
        badge: String? = nil,
        latestActivity: String? = nil,
        openable: Bool = false
    ) {
        self.id = id
        self.endpoint = endpoint
        self.kind = kind
        self.title = title
        self.badge = badge
        self.latestActivity = latestActivity
        self.openable = openable
    }

    enum CodingKeys: String, CodingKey {
        case id
        case endpoint
        case kind
        case title
        case badge
        case latestActivity = "latest_activity"
        case latestActivityCamel = "latestActivity"
        case openable
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.garyxDecodeFirstString(.id) ?? ""
        endpoint = try container.decode(GaryxChannelEndpoint.self, forKey: .endpoint)
        kind = try container.garyxDecodeFirstString(.kind) ?? ""
        title = try container.garyxDecodeFirstString(.title) ?? endpoint.displayLabel
        badge = try container.garyxDecodeFirstString(.badge)
        latestActivity = try container.garyxDecodeFirstString(.latestActivity, .latestActivityCamel)
        openable = try container.decodeIfPresent(Bool.self, forKey: .openable) ?? false
    }
}


public struct GaryxBotConsoleSummary: Decodable, Identifiable, Equatable, Sendable {
    public var id: String
    public var channel: String
    public var accountId: String
    public var title: String
    public var subtitle: String
    public var agentId: String?
    public var effectiveAgentId: String?
    public var rootBehavior: String
    public var status: String
    public var latestActivity: String?
    public var endpointCount: Int
    public var boundEndpointCount: Int
    public var workspaceDir: String?
    public var mainThreadId: String?
    public var defaultOpenThreadId: String?
    public var conversationNodes: [GaryxBotConversationNode]

    public init(
        id: String,
        channel: String,
        accountId: String,
        title: String,
        subtitle: String = "",
        agentId: String? = nil,
        effectiveAgentId: String? = nil,
        rootBehavior: String = "open_default",
        status: String = "idle",
        latestActivity: String? = nil,
        endpointCount: Int = 0,
        boundEndpointCount: Int = 0,
        workspaceDir: String? = nil,
        mainThreadId: String? = nil,
        defaultOpenThreadId: String? = nil,
        conversationNodes: [GaryxBotConversationNode] = []
    ) {
        self.id = id
        self.channel = channel
        self.accountId = accountId
        self.title = title
        self.subtitle = subtitle
        self.agentId = agentId
        self.effectiveAgentId = effectiveAgentId
        self.rootBehavior = rootBehavior
        self.status = status
        self.latestActivity = latestActivity
        self.endpointCount = endpointCount
        self.boundEndpointCount = boundEndpointCount
        self.workspaceDir = workspaceDir
        self.mainThreadId = mainThreadId
        self.defaultOpenThreadId = defaultOpenThreadId
        self.conversationNodes = conversationNodes
    }

    enum CodingKeys: String, CodingKey {
        case id
        case channel
        case accountId = "account_id"
        case accountIdCamel = "accountId"
        case title
        case subtitle
        case agentId = "agent_id"
        case agentIdCamel = "agentId"
        case effectiveAgentId = "effective_agent_id"
        case effectiveAgentIdCamel = "effectiveAgentId"
        case rootBehavior = "root_behavior"
        case rootBehaviorCamel = "rootBehavior"
        case status
        case latestActivity = "latest_activity"
        case latestActivityCamel = "latestActivity"
        case endpointCount = "endpoint_count"
        case endpointCountCamel = "endpointCount"
        case boundEndpointCount = "bound_endpoint_count"
        case boundEndpointCountCamel = "boundEndpointCount"
        case workspaceDir = "workspace_dir"
        case workspaceDirCamel = "workspaceDir"
        case mainThreadId = "main_endpoint_thread_id"
        case mainThreadIdCamel = "mainEndpointThreadId"
        case defaultOpenThreadId = "default_open_thread_id"
        case defaultOpenThreadIdCamel = "defaultOpenThreadId"
        case conversationNodes = "conversation_nodes"
        case conversationNodesCamel = "conversationNodes"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.garyxDecodeFirstString(.id) ?? ""
        channel = try container.garyxDecodeFirstString(.channel) ?? ""
        accountId = try container.garyxDecodeFirstString(.accountId, .accountIdCamel) ?? ""
        title = try container.garyxDecodeFirstString(.title) ?? id
        subtitle = try container.garyxDecodeFirstString(.subtitle) ?? ""
        agentId = try container.garyxDecodeFirstString(.agentId, .agentIdCamel)
        effectiveAgentId = try container.garyxDecodeFirstString(.effectiveAgentId, .effectiveAgentIdCamel)
        rootBehavior = try container.garyxDecodeFirstString(.rootBehavior, .rootBehaviorCamel) ?? "open_default"
        status = try container.garyxDecodeFirstString(.status) ?? "idle"
        latestActivity = try container.garyxDecodeFirstString(.latestActivity, .latestActivityCamel)
        endpointCount = try container.garyxDecodeFirstInt(.endpointCount, .endpointCountCamel) ?? 0
        boundEndpointCount = try container.garyxDecodeFirstInt(.boundEndpointCount, .boundEndpointCountCamel) ?? 0
        workspaceDir = try container.garyxDecodeFirstString(.workspaceDir, .workspaceDirCamel)
        mainThreadId = try container.garyxDecodeFirstString(.mainThreadId, .mainThreadIdCamel)
        defaultOpenThreadId = try container.garyxDecodeFirstString(.defaultOpenThreadId, .defaultOpenThreadIdCamel)
        let snakeConversationNodes = try container.decodeIfPresent(
            [GaryxBotConversationNode].self,
            forKey: .conversationNodes
        )
        let camelConversationNodes = try container.decodeIfPresent(
            [GaryxBotConversationNode].self,
            forKey: .conversationNodesCamel
        )
        conversationNodes = snakeConversationNodes ?? camelConversationNodes ?? []
    }
}


public struct GaryxChannelPluginCatalogPage: Decodable, Equatable, Sendable {
    public var plugins: [GaryxChannelPluginCatalogEntry]
}


public struct GaryxChannelPluginConfigMethod: Decodable, Equatable, Sendable {
    public var kind: String
    public var title: String?
    public var description: String?

    public init(kind: String, title: String? = nil, description: String? = nil) {
        self.kind = kind
        self.title = title
        self.description = description
    }

    enum CodingKeys: String, CodingKey {
        case kind
        case title
        case description
    }
}


public struct GaryxChannelPluginCatalogEntry: Decodable, Identifiable, Equatable, Sendable {
    public var id: String
    public var displayName: String
    public var description: String?
    public var iconDataUrl: String?
    public var schema: [String: GaryxJSONValue]
    public var configMethods: [GaryxChannelPluginConfigMethod]

    public init(
        id: String,
        displayName: String,
        description: String? = nil,
        iconDataUrl: String? = nil,
        schema: [String: GaryxJSONValue] = [:],
        configMethods: [GaryxChannelPluginConfigMethod] = []
    ) {
        self.id = id
        self.displayName = displayName
        self.description = description
        self.iconDataUrl = iconDataUrl
        self.schema = schema
        self.configMethods = configMethods
    }

    enum CodingKeys: String, CodingKey {
        case id
        case displayName = "display_name"
        case displayNameCamel = "displayName"
        case description
        case iconDataUrl = "icon_data_url"
        case iconDataUrlCamel = "iconDataUrl"
        case schema
        case configMethods = "config_methods"
        case configMethodsCamel = "configMethods"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.garyxDecodeFirstString(.id) ?? ""
        displayName = try container.garyxDecodeFirstString(.displayName, .displayNameCamel) ?? id
        description = try container.garyxDecodeFirstString(.description)
        iconDataUrl = try container.garyxDecodeFirstString(.iconDataUrl, .iconDataUrlCamel)
        schema = try container.decodeIfPresent([String: GaryxJSONValue].self, forKey: .schema) ?? [:]
        configMethods = try container.decodeIfPresent([GaryxChannelPluginConfigMethod].self, forKey: .configMethods)
            ?? container.decodeIfPresent([GaryxChannelPluginConfigMethod].self, forKey: .configMethodsCamel)
            ?? []
    }
}


public struct GaryxChannelAuthStartRequest: Encodable, Equatable, Sendable {
    public var formState: [String: GaryxJSONValue]

    public init(formState: [String: GaryxJSONValue] = [:]) {
        self.formState = formState
    }

    enum CodingKeys: String, CodingKey {
        case formState = "form_state"
    }
}


public struct GaryxChannelAuthSession: Decodable, Equatable, Sendable {
    public var sessionId: String
    public var display: [GaryxJSONValue]
    public var expiresInSecs: Int
    public var pollIntervalSecs: Int

    enum CodingKeys: String, CodingKey {
        case sessionId = "session_id"
        case sessionIdCamel = "sessionId"
        case display
        case expiresInSecs = "expires_in_secs"
        case expiresInSecsCamel = "expiresInSecs"
        case pollIntervalSecs = "poll_interval_secs"
        case pollIntervalSecsCamel = "pollIntervalSecs"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        sessionId = try container.garyxDecodeFirstString(.sessionId, .sessionIdCamel) ?? ""
        display = try container.decodeIfPresent([GaryxJSONValue].self, forKey: .display) ?? []
        expiresInSecs = try container.garyxDecodeFirstInt(.expiresInSecs, .expiresInSecsCamel) ?? 0
        pollIntervalSecs = max(1, try container.garyxDecodeFirstInt(.pollIntervalSecs, .pollIntervalSecsCamel) ?? 5)
    }
}


public struct GaryxChannelAuthPollRequest: Encodable, Equatable, Sendable {
    public var sessionId: String

    public init(sessionId: String) {
        self.sessionId = sessionId
    }

    enum CodingKeys: String, CodingKey {
        case sessionId = "session_id"
    }
}


public struct GaryxChannelAuthPollResult: Decodable, Equatable, Sendable {
    public var status: String
    public var display: [GaryxJSONValue]
    public var nextIntervalSecs: Int?
    public var values: [String: GaryxJSONValue]?
    public var reason: String?

    enum CodingKeys: String, CodingKey {
        case status
        case display
        case nextIntervalSecs = "next_interval_secs"
        case nextIntervalSecsCamel = "nextIntervalSecs"
        case values
        case reason
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        status = try container.garyxDecodeFirstString(.status) ?? ""
        display = try container.decodeIfPresent([GaryxJSONValue].self, forKey: .display) ?? []
        nextIntervalSecs = try container.garyxDecodeFirstInt(.nextIntervalSecs, .nextIntervalSecsCamel)
        values = try container.decodeIfPresent([String: GaryxJSONValue].self, forKey: .values)
        reason = try container.garyxDecodeFirstString(.reason)
    }
}


public struct GaryxChannelAccountValidationRequest: Encodable, Equatable, Sendable {
    public var accountId: String
    public var enabled: Bool
    public var config: [String: GaryxJSONValue]

    public init(accountId: String, enabled: Bool = true, config: [String: GaryxJSONValue]) {
        self.accountId = accountId
        self.enabled = enabled
        self.config = config
    }

    enum CodingKeys: String, CodingKey {
        case accountId = "account_id"
        case enabled
        case config
    }
}


public struct GaryxChannelAccountValidationResult: Decodable, Equatable, Sendable {
    public var validated: Bool
    public var message: String
}


public struct GaryxBotBindingRequest: Encodable, Equatable, Sendable {
    public var botId: String
    public var threadId: String?

    public init(botId: String, threadId: String? = nil) {
        self.botId = botId
        self.threadId = threadId
    }
}


public struct GaryxBotBindingResult: Decodable, Equatable, Sendable {
    public var ok: Bool
    public var botId: String
    public var channel: String
    public var accountId: String
    public var workspaceMode: String?
    public var mainEndpointStatus: String
    public var currentThreadStatus: String
    public var currentThreadId: String?
    public var action: String?
    public var threadId: String?
    public var previousThreadId: String?
    public var endpointKey: String?
    public var error: String?
    public var reason: String?

    enum CodingKeys: String, CodingKey {
        case ok
        case botId = "bot_id"
        case botIdCamel = "botId"
        case channel
        case accountId = "account_id"
        case accountIdCamel = "accountId"
        case workspaceMode = "workspace_mode"
        case workspaceModeCamel = "workspaceMode"
        case mainEndpointStatus = "main_endpoint_status"
        case mainEndpointStatusCamel = "mainEndpointStatus"
        case currentThreadStatus = "current_thread_status"
        case currentThreadStatusCamel = "currentThreadStatus"
        case currentThreadId = "current_thread_id"
        case currentThreadIdCamel = "currentThreadId"
        case action
        case threadId = "thread_id"
        case threadIdCamel = "threadId"
        case previousThreadId = "previous_thread_id"
        case previousThreadIdCamel = "previousThreadId"
        case endpointKey = "endpoint_key"
        case endpointKeyCamel = "endpointKey"
        case error
        case reason
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        ok = try container.garyxDecodeFirstBool(.ok) ?? false
        botId = try container.garyxDecodeFirstString(.botId, .botIdCamel) ?? ""
        channel = try container.garyxDecodeFirstString(.channel) ?? ""
        accountId = try container.garyxDecodeFirstString(.accountId, .accountIdCamel) ?? ""
        workspaceMode = try container.garyxDecodeFirstString(.workspaceMode, .workspaceModeCamel)
        mainEndpointStatus = try container.garyxDecodeFirstString(.mainEndpointStatus, .mainEndpointStatusCamel) ?? "unknown"
        currentThreadStatus = try container.garyxDecodeFirstString(.currentThreadStatus, .currentThreadStatusCamel) ?? "unknown"
        currentThreadId = try container.garyxDecodeFirstString(.currentThreadId, .currentThreadIdCamel)
        action = try container.garyxDecodeFirstString(.action)
        threadId = try container.garyxDecodeFirstString(.threadId, .threadIdCamel)
        previousThreadId = try container.garyxDecodeFirstString(.previousThreadId, .previousThreadIdCamel)
        endpointKey = try container.garyxDecodeFirstString(.endpointKey, .endpointKeyCamel)
        error = try container.garyxDecodeFirstString(.error)
        reason = try container.garyxDecodeFirstString(.reason)
    }
}


public struct GaryxChannelEndpointBindRequest: Encodable, Equatable, Sendable {
    public var endpointKey: String
    public var threadId: String

    public init(endpointKey: String, threadId: String) {
        self.endpointKey = endpointKey
        self.threadId = threadId
    }
}


public struct GaryxChannelEndpointDetachRequest: Encodable, Equatable, Sendable {
    public var endpointKey: String

    public init(endpointKey: String) {
        self.endpointKey = endpointKey
    }
}
