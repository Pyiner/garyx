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
    public var defaultReasoningEffort: String?
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
        case defaultReasoningEffort
        case defaultReasoningEffortSnake = "default_reasoning_effort"
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
        defaultReasoningEffort = try container.garyxDecodeFirstString(
            .defaultReasoningEffort,
            .defaultReasoningEffortSnake
        )
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

public enum GaryxAgentAvatarKind: String, Equatable, Sendable {
    case agent
    case team
}

public struct GaryxAvatarStyleOption: Identifiable, Equatable, Sendable {
    public var id: String
    public var label: String
    public var prompt: String

    public init(id: String, label: String, prompt: String) {
        self.id = id
        self.label = label
        self.prompt = prompt
    }

    public static let defaultId = "clean_glyph"

    public static let builtIn: [GaryxAvatarStyleOption] = [
        GaryxAvatarStyleOption(
            id: "clean_glyph",
            label: "Clean glyph",
            prompt: "minimal vector glyph, simple geometric mark, balanced negative space, charcoal base with one sharp accent color"
        ),
        GaryxAvatarStyleOption(
            id: "soft_3d",
            label: "Soft 3D",
            prompt: "soft 3D clay icon, rounded abstract forms, gentle studio lighting, compact and friendly without looking childish"
        ),
        GaryxAvatarStyleOption(
            id: "glass_icon",
            label: "Glass icon",
            prompt: "translucent glassmorphism icon, crisp inner symbol, subtle refraction, clean depth, restrained blue green accent"
        ),
        GaryxAvatarStyleOption(
            id: "pixel_badge",
            label: "Pixel badge",
            prompt: "premium pixel-art badge, 32-bit style, readable blocky silhouette, limited palette, modern developer-tool feel"
        ),
        GaryxAvatarStyleOption(
            id: "ink_line",
            label: "Ink line",
            prompt: "monoline ink icon, expressive black linework, small accent fill, simple abstract agent signal, high legibility"
        ),
        GaryxAvatarStyleOption(
            id: "paper_cut",
            label: "Paper cut",
            prompt: "layered paper-cut icon, crisp stacked shapes, soft shadow, warm neutral base with a bright teal accent, high contrast silhouette"
        ),
        GaryxAvatarStyleOption(
            id: "blueprint",
            label: "Blueprint",
            prompt: "technical blueprint emblem, precise line grid, subtle cyan ink on deep charcoal, schematic but simple, readable at small sizes"
        ),
        GaryxAvatarStyleOption(
            id: "enamel_sticker",
            label: "Enamel sticker",
            prompt: "polished enamel sticker badge, bold flat shapes, thick clean outline, optimistic coral and mint accents, crisp app-icon finish"
        ),
    ]
}

public enum GaryxAvatarPromptBuilder {
    public static func prompt(
        displayName: String,
        identifier: String? = nil,
        kind: GaryxAgentAvatarKind,
        stylePrompt: String? = nil
    ) -> String {
        let name = avatarName(displayName: displayName, identifier: identifier)
        let quotedName = jsonQuoted(name)
        let isTeam = kind == .team
        let style = stylePrompt?.trimmingCharacters(in: .whitespacesAndNewlines)
            ?? "minimal vector glyph, simple geometry, balanced negative space, one confident accent color"
        return [
            "Create a square app avatar for an AI \(isTeam ? "agent team" : "agent") named \(quotedName).",
            "Visual style: \(style).",
            "Composition: one centered abstract \(isTeam ? "team" : "agent") mark, clean silhouette, readable at 32px, restrained palette, polished macOS developer-tool finish.",
            "Do not include text, letters, watermarks, screenshots, people, or UI chrome.",
        ].joined(separator: "\n")
    }

    private static func avatarName(displayName: String, identifier: String?) -> String {
        let display = displayName.trimmingCharacters(in: .whitespacesAndNewlines)
        if !display.isEmpty {
            return display
        }
        let id = identifier?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return id.isEmpty ? "Agent" : id
    }

    private static func jsonQuoted(_ value: String) -> String {
        if let data = try? JSONEncoder().encode(value),
           let encoded = String(data: data, encoding: .utf8) {
            return encoded
        }
        let escaped = value
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
        return "\"\(escaped)\""
    }
}

public enum GaryxMobileAvatarEditorActivity: Equatable, Sendable {
    case generate
    case upload
}

public struct GaryxMobileAvatarEditorState: Equatable, Sendable {
    public private(set) var requestId: UUID?
    public private(set) var activity: GaryxMobileAvatarEditorActivity?
    public private(set) var fingerprint: String

    public init(
        requestId: UUID? = nil,
        activity: GaryxMobileAvatarEditorActivity? = nil,
        fingerprint: String = ""
    ) {
        self.requestId = requestId
        self.activity = activity
        self.fingerprint = fingerprint
    }

    public var isGenerating: Bool {
        activity == .generate
    }

    public var isUploading: Bool {
        activity == .upload
    }

    public var isBusy: Bool {
        activity != nil
    }

    @discardableResult
    public mutating func begin(
        _ activity: GaryxMobileAvatarEditorActivity,
        fingerprint: String,
        requestId: UUID = UUID()
    ) -> UUID {
        self.requestId = requestId
        self.activity = activity
        self.fingerprint = fingerprint
        return requestId
    }

    public func canApply(requestId: UUID, fingerprint: String) -> Bool {
        self.requestId == requestId && self.fingerprint == fingerprint
    }

    public mutating func finish(requestId: UUID) {
        guard self.requestId == requestId else { return }
        self.requestId = nil
        activity = nil
        fingerprint = ""
    }

    public mutating func reset() {
        requestId = nil
        activity = nil
        fingerprint = ""
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
    /// Concurrency token for updates: the `updatedAt` of the agent this edit
    /// was based on. Required by the gateway on PUT; omitted on POST.
    public var expectedUpdatedAt: String?

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
        systemPrompt: String? = nil,
        expectedUpdatedAt: String? = nil
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
        self.expectedUpdatedAt = expectedUpdatedAt
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
        case expectedUpdatedAt = "expected_updated_at"
    }
}


public struct GaryxTeamRequest: Encodable, Equatable, Sendable {
    public var teamId: String
    public var displayName: String
    public var leaderAgentId: String
    public var memberAgentIds: [String]
    public var workflowText: String
    public var avatarDataUrl: String?
    /// Concurrency token for updates: the `updatedAt` of the team this edit
    /// was based on. Required by the gateway on PUT; omitted on POST.
    public var expectedUpdatedAt: String?

    public init(
        teamId: String,
        displayName: String,
        leaderAgentId: String,
        memberAgentIds: [String],
        workflowText: String,
        avatarDataUrl: String? = nil,
        expectedUpdatedAt: String? = nil
    ) {
        self.teamId = teamId
        self.displayName = displayName
        self.leaderAgentId = leaderAgentId
        self.memberAgentIds = memberAgentIds
        self.workflowText = workflowText
        self.avatarDataUrl = avatarDataUrl
        self.expectedUpdatedAt = expectedUpdatedAt
    }
}
