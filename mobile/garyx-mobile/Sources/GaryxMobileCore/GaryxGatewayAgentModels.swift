import Foundation

public struct GaryxAgentsPage: Decodable, Equatable, Sendable {
    public var agents: [GaryxAgentSummary]
}


public struct GaryxAgentSummary: Decodable, Identifiable, Equatable, Sendable {
    public var id: String
    public var displayName: String
    public var providerType: String
    public var model: String
    public var modelReasoningEffort: String
    public var modelServiceTier: String
    public var providerEnv: [String: String]
    public var authSource: String
    public var baseUrl: String
    public var codexHome: String
    public var maxToolIterations: Int?
    public var requestTimeoutSeconds: Int?
    public var defaultWorkspaceDir: String
    public var avatarDataUrl: String
    public var systemPrompt: String
    public var builtIn: Bool
    public var standalone: Bool
    public var createdAt: String?
    public var updatedAt: String?

    public init(
        id: String,
        displayName: String,
        providerType: String,
        model: String,
        modelReasoningEffort: String = "",
        modelServiceTier: String = "",
        providerEnv: [String: String] = [:],
        authSource: String = "",
        baseUrl: String = "",
        codexHome: String = "",
        maxToolIterations: Int? = nil,
        requestTimeoutSeconds: Int? = nil,
        defaultWorkspaceDir: String = "",
        avatarDataUrl: String = "",
        systemPrompt: String = "",
        builtIn: Bool = false,
        standalone: Bool = true,
        createdAt: String? = nil,
        updatedAt: String? = nil
    ) {
        self.id = id
        self.displayName = displayName
        self.providerType = providerType
        self.model = model
        self.modelReasoningEffort = modelReasoningEffort
        self.modelServiceTier = modelServiceTier
        self.providerEnv = providerEnv
        self.authSource = authSource
        self.baseUrl = baseUrl
        self.codexHome = codexHome
        self.maxToolIterations = maxToolIterations
        self.requestTimeoutSeconds = requestTimeoutSeconds
        self.defaultWorkspaceDir = defaultWorkspaceDir
        self.avatarDataUrl = avatarDataUrl
        self.systemPrompt = systemPrompt
        self.builtIn = builtIn
        self.standalone = standalone
        self.createdAt = createdAt
        self.updatedAt = updatedAt
    }

    enum CodingKeys: String, CodingKey {
        case agentId = "agent_id"
        case agentIdCamel = "agentId"
        case displayName = "display_name"
        case displayNameCamel = "displayName"
        case providerType = "provider_type"
        case providerTypeCamel = "providerType"
        case model
        case modelReasoningEffort = "model_reasoning_effort"
        case modelReasoningEffortCamel = "modelReasoningEffort"
        case modelServiceTier = "model_service_tier"
        case modelServiceTierCamel = "modelServiceTier"
        case providerEnv = "provider_env"
        case providerEnvCamel = "providerEnv"
        case env
        case authSource = "auth_source"
        case authSourceCamel = "authSource"
        case baseUrl = "base_url"
        case baseUrlCamel = "baseUrl"
        case codexHome = "codex_home"
        case codexHomeCamel = "codexHome"
        case maxToolIterations = "max_tool_iterations"
        case maxToolIterationsCamel = "maxToolIterations"
        case requestTimeoutSeconds = "request_timeout_seconds"
        case requestTimeoutSecondsCamel = "requestTimeoutSeconds"
        case defaultWorkspaceDir = "default_workspace_dir"
        case defaultWorkspaceDirCamel = "defaultWorkspaceDir"
        case workspaceDir = "workspace_dir"
        case workspaceDirCamel = "workspaceDir"
        case avatarDataUrl = "avatar_data_url"
        case avatarDataUrlCamel = "avatarDataUrl"
        case avatarURL
        case avatarUrl = "avatar_url"
        case systemPrompt = "system_prompt"
        case systemPromptCamel = "systemPrompt"
        case builtIn = "built_in"
        case builtInCamel = "builtIn"
        case standalone
        case createdAt = "created_at"
        case createdAtCamel = "createdAt"
        case updatedAt = "updated_at"
        case updatedAtCamel = "updatedAt"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.garyxDecodeFirstString(.agentId, .agentIdCamel) ?? ""
        displayName = try container.garyxDecodeFirstString(.displayName, .displayNameCamel) ?? id
        providerType = try container.garyxDecodeFirstString(.providerType, .providerTypeCamel) ?? ""
        model = try container.garyxDecodeFirstString(.model) ?? ""
        modelReasoningEffort = try container.garyxDecodeFirstString(.modelReasoningEffort, .modelReasoningEffortCamel) ?? ""
        modelServiceTier = try container.garyxDecodeFirstString(.modelServiceTier, .modelServiceTierCamel) ?? ""
        providerEnv = try container.decodeIfPresent([String: String].self, forKey: .providerEnv)
            ?? container.decodeIfPresent([String: String].self, forKey: .providerEnvCamel)
            ?? container.decodeIfPresent([String: String].self, forKey: .env)
            ?? [:]
        authSource = try container.garyxDecodeFirstString(.authSource, .authSourceCamel) ?? ""
        baseUrl = try container.garyxDecodeFirstString(.baseUrl, .baseUrlCamel) ?? ""
        codexHome = try container.garyxDecodeFirstString(.codexHome, .codexHomeCamel) ?? ""
        maxToolIterations = try container.garyxDecodeFirstInt(.maxToolIterations, .maxToolIterationsCamel)
        requestTimeoutSeconds = try container.garyxDecodeFirstInt(.requestTimeoutSeconds, .requestTimeoutSecondsCamel)
        defaultWorkspaceDir = try container.garyxDecodeFirstString(
            .defaultWorkspaceDir,
            .defaultWorkspaceDirCamel,
            .workspaceDir,
            .workspaceDirCamel
        ) ?? ""
        avatarDataUrl = try container.garyxDecodeFirstString(
            .avatarDataUrl,
            .avatarDataUrlCamel,
            .avatarURL,
            .avatarUrl
        ) ?? ""
        systemPrompt = try container.garyxDecodeFirstString(.systemPrompt, .systemPromptCamel) ?? ""
        builtIn = try container.garyxDecodeFirstBool(.builtIn, .builtInCamel) ?? false
        standalone = try container.garyxDecodeFirstBool(.standalone) ?? true
        createdAt = try container.garyxDecodeFirstString(.createdAt, .createdAtCamel)
        updatedAt = try container.garyxDecodeFirstString(.updatedAt, .updatedAtCamel)
    }
}


public struct GaryxTeamsPage: Decodable, Equatable, Sendable {
    public var teams: [GaryxTeamSummary]
}


public struct GaryxTeamSummary: Decodable, Identifiable, Equatable, Sendable {
    public var id: String
    public var displayName: String
    public var leaderAgentId: String
    public var memberAgentIds: [String]
    public var workflowText: String
    public var avatarDataUrl: String
    public var createdAt: String?
    public var updatedAt: String?

    public init(
        id: String,
        displayName: String,
        leaderAgentId: String,
        memberAgentIds: [String],
        workflowText: String = "",
        avatarDataUrl: String = "",
        createdAt: String? = nil,
        updatedAt: String? = nil
    ) {
        self.id = id
        self.displayName = displayName
        self.leaderAgentId = leaderAgentId
        self.memberAgentIds = memberAgentIds
        self.workflowText = workflowText
        self.avatarDataUrl = avatarDataUrl
        self.createdAt = createdAt
        self.updatedAt = updatedAt
    }

    enum CodingKeys: String, CodingKey {
        case teamId = "team_id"
        case teamIdCamel = "teamId"
        case displayName = "display_name"
        case displayNameCamel = "displayName"
        case leaderAgentId = "leader_agent_id"
        case leaderAgentIdCamel = "leaderAgentId"
        case memberAgentIds = "member_agent_ids"
        case memberAgentIdsCamel = "memberAgentIds"
        case workflowText = "workflow_text"
        case workflowTextCamel = "workflowText"
        case avatarDataUrl = "avatar_data_url"
        case avatarDataUrlCamel = "avatarDataUrl"
        case avatarURL
        case avatarUrl = "avatar_url"
        case createdAt = "created_at"
        case createdAtCamel = "createdAt"
        case updatedAt = "updated_at"
        case updatedAtCamel = "updatedAt"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.garyxDecodeFirstString(.teamId, .teamIdCamel) ?? ""
        displayName = try container.garyxDecodeFirstString(.displayName, .displayNameCamel) ?? id
        leaderAgentId = try container.garyxDecodeFirstString(.leaderAgentId, .leaderAgentIdCamel) ?? ""
        memberAgentIds = try container.garyxDecodeFirstStringArray(.memberAgentIds, .memberAgentIdsCamel) ?? []
        workflowText = try container.garyxDecodeFirstString(.workflowText, .workflowTextCamel) ?? ""
        avatarDataUrl = try container.garyxDecodeFirstString(
            .avatarDataUrl,
            .avatarDataUrlCamel,
            .avatarURL,
            .avatarUrl
        ) ?? ""
        createdAt = try container.garyxDecodeFirstString(.createdAt, .createdAtCamel)
        updatedAt = try container.garyxDecodeFirstString(.updatedAt, .updatedAtCamel)
    }
}


public struct GaryxProviderModelOption: Decodable, Identifiable, Equatable, Sendable {
    public var id: String
    public var label: String
    public var description: String?
    public var recommended: Bool
    public var defaultReasoningEffort: String?
    public var supportedReasoningEfforts: [GaryxProviderModelOption]
    public var serviceTiers: [GaryxProviderModelOption]

    enum CodingKeys: String, CodingKey {
        case id
        case label
        case description
        case recommended
        case defaultReasoningEffort
        case defaultReasoningEffortSnake = "default_reasoning_effort"
        case supportedReasoningEfforts
        case supportedReasoningEffortsSnake = "supported_reasoning_efforts"
        case serviceTiers
        case serviceTiersSnake = "service_tiers"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.garyxDecodeFirstString(.id) ?? ""
        label = try container.garyxDecodeFirstString(.label) ?? id
        description = try container.garyxDecodeFirstString(.description)
        recommended = try container.garyxDecodeFirstBool(.recommended) ?? false
        defaultReasoningEffort = try container.garyxDecodeFirstString(
            .defaultReasoningEffort,
            .defaultReasoningEffortSnake
        )
        supportedReasoningEfforts = try container.decodeIfPresent(
            [GaryxProviderModelOption].self,
            forKey: .supportedReasoningEfforts
        ) ?? container.decodeIfPresent(
            [GaryxProviderModelOption].self,
            forKey: .supportedReasoningEffortsSnake
        ) ?? []
        serviceTiers = try container.decodeIfPresent([GaryxProviderModelOption].self, forKey: .serviceTiers)
            ?? container.decodeIfPresent([GaryxProviderModelOption].self, forKey: .serviceTiersSnake)
            ?? []
    }
}


public struct GaryxProviderModels: Decodable, Equatable, Sendable {
    public var providerType: String
    public var supportsModelSelection: Bool
    public var models: [GaryxProviderModelOption]
    public var supportsReasoningEffortSelection: Bool
    public var reasoningEfforts: [GaryxProviderModelOption]
    public var supportsServiceTierSelection: Bool
    public var serviceTiers: [GaryxProviderModelOption]
    public var defaultModel: String?
    public var source: String
    public var error: String?

    enum CodingKeys: String, CodingKey {
        case providerType
        case providerTypeSnake = "provider_type"
        case supportsModelSelection
        case supportsModelSelectionSnake = "supports_model_selection"
        case models
        case supportsReasoningEffortSelection
        case supportsReasoningEffortSelectionSnake = "supports_reasoning_effort_selection"
        case reasoningEfforts
        case reasoningEffortsSnake = "reasoning_efforts"
        case supportsServiceTierSelection
        case supportsServiceTierSelectionSnake = "supports_service_tier_selection"
        case serviceTiers
        case serviceTiersSnake = "service_tiers"
        case defaultModel
        case defaultModelSnake = "default_model"
        case source
        case error
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        providerType = try container.garyxDecodeFirstString(.providerType, .providerTypeSnake) ?? ""
        supportsModelSelection = try container.garyxDecodeFirstBool(
            .supportsModelSelection,
            .supportsModelSelectionSnake
        ) ?? false
        models = try container.decodeIfPresent([GaryxProviderModelOption].self, forKey: .models) ?? []
        supportsReasoningEffortSelection = try container.garyxDecodeFirstBool(
            .supportsReasoningEffortSelection,
            .supportsReasoningEffortSelectionSnake
        ) ?? false
        reasoningEfforts = try container.decodeIfPresent(
            [GaryxProviderModelOption].self,
            forKey: .reasoningEfforts
        ) ?? container.decodeIfPresent([GaryxProviderModelOption].self, forKey: .reasoningEffortsSnake) ?? []
        supportsServiceTierSelection = try container.garyxDecodeFirstBool(
            .supportsServiceTierSelection,
            .supportsServiceTierSelectionSnake
        ) ?? false
        serviceTiers = try container.decodeIfPresent([GaryxProviderModelOption].self, forKey: .serviceTiers)
            ?? container.decodeIfPresent([GaryxProviderModelOption].self, forKey: .serviceTiersSnake)
            ?? []
        defaultModel = try container.garyxDecodeFirstString(.defaultModel, .defaultModelSnake)
        source = try container.garyxDecodeFirstString(.source) ?? ""
        error = try container.garyxDecodeFirstString(.error)
    }
}


public struct GaryxGeneratedAvatar: Decodable, Equatable, Sendable {
    public var avatarDataUrl: String
    public var mediaType: String

    enum CodingKeys: String, CodingKey {
        case avatarDataUrl
        case avatarDataUrlSnake = "avatar_data_url"
        case dataBase64
        case dataBase64Snake = "data_base64"
        case mediaType
        case mediaTypeSnake = "media_type"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        mediaType = try container.garyxDecodeFirstString(.mediaType, .mediaTypeSnake) ?? "image/png"
        if let dataUrl = try container.garyxDecodeFirstString(.avatarDataUrl, .avatarDataUrlSnake) {
            avatarDataUrl = dataUrl
        } else if let encoded = try container.garyxDecodeFirstString(.dataBase64, .dataBase64Snake), !encoded.isEmpty {
            avatarDataUrl = "data:\(mediaType);base64,\(encoded)"
        } else {
            avatarDataUrl = ""
        }
    }
}


public struct GaryxGenerateAvatarRequest: Encodable, Equatable, Sendable {
    public var prompt: String
    public var timeoutSecs: Int

    public init(prompt: String, timeoutSecs: Int = 600) {
        self.prompt = prompt
        self.timeoutSecs = timeoutSecs
    }

    enum CodingKeys: String, CodingKey {
        case prompt
        case timeoutSecs = "timeout_secs"
    }
}


public struct GaryxCustomAgentRequest: Encodable, Equatable, Sendable {
    public var agentId: String
    public var displayName: String
    public var providerType: String
    public var model: String?
    public var modelReasoningEffort: String?
    public var modelServiceTier: String?
    public var providerEnv: [String: String]?
    public var authSource: String?
    public var baseUrl: String?
    public var codexHome: String?
    public var maxToolIterations: Int?
    public var requestTimeoutSeconds: Int?
    public var defaultWorkspaceDir: String?
    public var avatarDataUrl: String?
    public var systemPrompt: String?

    public init(
        agentId: String,
        displayName: String,
        providerType: String,
        model: String? = nil,
        modelReasoningEffort: String? = nil,
        modelServiceTier: String? = nil,
        providerEnv: [String: String]? = nil,
        authSource: String? = nil,
        baseUrl: String? = nil,
        codexHome: String? = nil,
        maxToolIterations: Int? = nil,
        requestTimeoutSeconds: Int? = nil,
        defaultWorkspaceDir: String? = nil,
        avatarDataUrl: String? = nil,
        systemPrompt: String? = nil
    ) {
        self.agentId = agentId
        self.displayName = displayName
        self.providerType = providerType
        self.model = model
        self.modelReasoningEffort = modelReasoningEffort
        self.modelServiceTier = modelServiceTier
        self.providerEnv = providerEnv
        self.authSource = authSource
        self.baseUrl = baseUrl
        self.codexHome = codexHome
        self.maxToolIterations = maxToolIterations
        self.requestTimeoutSeconds = requestTimeoutSeconds
        self.defaultWorkspaceDir = defaultWorkspaceDir
        self.avatarDataUrl = avatarDataUrl
        self.systemPrompt = systemPrompt
    }

    enum CodingKeys: String, CodingKey {
        case agentId = "agent_id"
        case displayName = "display_name"
        case providerType = "provider_type"
        case model
        case modelReasoningEffort = "model_reasoning_effort"
        case modelServiceTier = "model_service_tier"
        case providerEnv = "provider_env"
        case authSource = "auth_source"
        case baseUrl = "base_url"
        case codexHome = "codex_home"
        case maxToolIterations = "max_tool_iterations"
        case requestTimeoutSeconds = "request_timeout_seconds"
        case defaultWorkspaceDir = "default_workspace_dir"
        case avatarDataUrl = "avatar_data_url"
        case systemPrompt = "system_prompt"
    }
}


public struct GaryxTeamRequest: Encodable, Equatable, Sendable {
    public var teamId: String
    public var displayName: String
    public var leaderAgentId: String
    public var memberAgentIds: [String]
    public var workflowText: String
    public var avatarDataUrl: String?

    public init(
        teamId: String,
        displayName: String,
        leaderAgentId: String,
        memberAgentIds: [String],
        workflowText: String,
        avatarDataUrl: String? = nil
    ) {
        self.teamId = teamId
        self.displayName = displayName
        self.leaderAgentId = leaderAgentId
        self.memberAgentIds = memberAgentIds
        self.workflowText = workflowText
        self.avatarDataUrl = avatarDataUrl
    }
}
