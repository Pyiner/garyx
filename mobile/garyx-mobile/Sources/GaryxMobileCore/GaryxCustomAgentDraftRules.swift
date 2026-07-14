import Foundation

public enum GaryxCustomAgentDraftMode: Equatable, Sendable {
    case create
    case edit(agentId: String, expectedUpdatedAt: String)
}

public enum GaryxCustomAgentAvatarIntent: Equatable, Sendable {
    case unchanged
    case replace(String)
    case remove
}

public enum GaryxCustomAgentValidationIssue: Equatable, Sendable {
    case nameRequired
    case derivedAgentIdRequired
    case providerRequired
    case invalidEnvironmentKey
    case missingExpectedUpdatedAt
}

public struct GaryxCustomAgentCreateCollision: Equatable, Sendable {
    public let derivedAgentId: String

    public init(derivedAgentId: String) {
        self.derivedAgentId = derivedAgentId
    }
}

public enum GaryxCustomAgentMutationFailure: Equatable, Sendable {
    case createConflict
    case editConflict(currentUpdatedAt: String?)
    case deleted
    case other(message: String)
}

public enum GaryxCustomAgentMutationResult: Equatable, Sendable {
    case saved(GaryxAgentSummary)
    case failed(GaryxCustomAgentMutationFailure)
    case superseded
}

public enum GaryxCustomAgentLoadResult: Equatable, Sendable {
    case loaded(GaryxAgentSummary)
    case deleted
    case failed(message: String)
    case superseded
}

public enum GaryxCustomAgentEditStatus: Equatable, Sendable {
    case loading
    case ready
    case conflict(currentUpdatedAt: String?)
    case deleted
    case loadFailed(message: String)
}

/// Cross-platform identity semantics for custom-agent create/edit forms.
///
/// Create IDs are a pure projection of `displayName`; edit IDs are the
/// immutable authoritative profile ID captured by `mode`. There is no mutable
/// ID field and therefore no touched/programmatic-binding state to reconcile.
public struct GaryxCustomAgentDraft: Equatable, Sendable {
    public private(set) var mode: GaryxCustomAgentDraftMode
    public var displayName: String {
        didSet {
            clearCreateCollisionWhenIdentityChanges()
        }
    }
    public var providerType: String
    public var model: String
    public var modelReasoningEffort: String
    public var modelServiceTier: String
    public var defaultWorkspaceDir: String
    public private(set) var avatarDataUrl: String
    public private(set) var avatarIntent: GaryxCustomAgentAvatarIntent
    public var systemPrompt: String
    public var env: GaryxAgentEnvDraft
    public private(set) var createCollision: GaryxCustomAgentCreateCollision?

    public init(
        mode: GaryxCustomAgentDraftMode,
        displayName: String = "",
        providerType: String,
        model: String = "",
        modelReasoningEffort: String = "",
        modelServiceTier: String = "",
        defaultWorkspaceDir: String = "",
        avatarDataUrl: String = "",
        systemPrompt: String = "",
        env: GaryxAgentEnvDraft = .empty,
        createCollision: GaryxCustomAgentCreateCollision? = nil
    ) {
        self.mode = mode
        self.displayName = displayName
        self.providerType = providerType
        self.model = model
        self.modelReasoningEffort = modelReasoningEffort
        self.modelServiceTier = modelServiceTier
        self.defaultWorkspaceDir = defaultWorkspaceDir
        self.avatarDataUrl = avatarDataUrl
        self.avatarIntent = mode.isCreate && !avatarDataUrl.isEmpty
            ? .replace(avatarDataUrl)
            : .unchanged
        self.systemPrompt = systemPrompt
        self.env = env
        self.createCollision = createCollision
    }

    public static func create(defaultProviderType: String = "codex_app_server") -> Self {
        Self(mode: .create, providerType: defaultProviderType)
    }

    public static func edit(authoritative agent: GaryxAgentSummary) -> Self {
        Self(
            mode: .edit(
                agentId: agent.id,
                expectedUpdatedAt: agent.updatedAt ?? ""
            ),
            displayName: agent.displayName,
            providerType: agent.providerType,
            model: agent.model,
            modelReasoningEffort: agent.modelReasoningEffort,
            modelServiceTier: agent.modelServiceTier,
            defaultWorkspaceDir: agent.defaultWorkspaceDir,
            avatarDataUrl: agent.avatarDataUrl,
            systemPrompt: agent.systemPrompt,
            env: .seeded(from: agent.providerEnv)
        )
    }

    public var agentId: String {
        switch mode {
        case .create:
            return GaryxCustomAgentDraftRules.deriveId(from: displayName)
        case .edit(let agentId, _):
            return agentId
        }
    }

    public var validationIssues: [GaryxCustomAgentValidationIssue] {
        var issues: [GaryxCustomAgentValidationIssue] = []
        if GaryxCustomAgentDraftRules.javascriptTrim(displayName).isEmpty {
            issues.append(.nameRequired)
        } else if mode.isCreate && agentId.isEmpty {
            issues.append(.derivedAgentIdRequired)
        }
        if providerType.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            issues.append(.providerRequired)
        }
        if env.hasInvalidKey {
            issues.append(.invalidEnvironmentKey)
        }
        if case .edit(_, let expectedUpdatedAt) = mode,
           expectedUpdatedAt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            issues.append(.missingExpectedUpdatedAt)
        }
        return issues
    }

    public var canSubmit: Bool {
        validationIssues.isEmpty && createCollision == nil
    }

    public var nameValidationMessage: String? {
        if validationIssues.contains(.nameRequired) {
            return "Name is required."
        }
        if validationIssues.contains(.derivedAgentIdRequired) {
            return "Name must include at least one English letter or number."
        }
        if let createCollision {
            let name = GaryxCustomAgentDraftRules.javascriptTrim(displayName)
            return "An agent named \u{201C}\(name)\u{201D} already uses the ID \u{201C}\(createCollision.derivedAgentId)\u{201D}. Change the name and try again."
        }
        return nil
    }

    public var environmentValidationMessage: String? {
        validationIssues.contains(.invalidEnvironmentKey)
            ? "Environment variable names must match [A-Za-z_][A-Za-z0-9_]*."
            : nil
    }

    public mutating func setAvatarDataUrl(_ value: String) {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        avatarDataUrl = trimmed
        if trimmed.isEmpty {
            avatarIntent = mode.isCreate ? .unchanged : .remove
        } else {
            avatarIntent = .replace(trimmed)
        }
    }

    public mutating func removeAvatar() {
        setAvatarDataUrl("")
    }

    public mutating func recordCreateConflict() {
        guard mode.isCreate, !agentId.isEmpty else { return }
        createCollision = GaryxCustomAgentCreateCollision(derivedAgentId: agentId)
    }

    public mutating func clearServerIssue() {
        createCollision = nil
    }

    public func makeRequest() -> GaryxCustomAgentRequest? {
        guard validationIssues.isEmpty else { return nil }

        let trimmedName = GaryxCustomAgentDraftRules.javascriptTrim(displayName)
        let trimmedProvider = providerType.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedWorkspace = defaultWorkspaceDir.trimmingCharacters(in: .whitespacesAndNewlines)
        let providerEnv: [String: String]?
        switch mode {
        case .create:
            let map = env.currentEnvMap()
            providerEnv = map.isEmpty ? nil : map
        case .edit:
            switch env.resolvedIntent() {
            case .unchanged:
                providerEnv = nil
            case .replace(let map):
                providerEnv = map
            case .clear:
                providerEnv = [:]
            }
        }

        let avatarDataUrl: String?
        switch avatarIntent {
        case .unchanged:
            avatarDataUrl = nil
        case .replace(let value):
            avatarDataUrl = value
        case .remove:
            avatarDataUrl = ""
        }

        let expectedUpdatedAt: String?
        let workspaceValue: String?
        switch mode {
        case .create:
            expectedUpdatedAt = nil
            workspaceValue = trimmedWorkspace.isEmpty ? nil : trimmedWorkspace
        case .edit(_, let token):
            expectedUpdatedAt = token.trimmingCharacters(in: .whitespacesAndNewlines)
            // Empty is an explicit clear on update; nil would preserve storage.
            workspaceValue = trimmedWorkspace
        }

        return GaryxCustomAgentRequest(
            agentId: agentId,
            displayName: trimmedName,
            providerType: trimmedProvider,
            model: model.trimmingCharacters(in: .whitespacesAndNewlines),
            modelReasoningEffort: modelReasoningEffort.trimmingCharacters(in: .whitespacesAndNewlines),
            modelServiceTier: modelServiceTier.trimmingCharacters(in: .whitespacesAndNewlines),
            providerEnv: providerEnv,
            defaultWorkspaceDir: workspaceValue,
            avatarDataUrl: avatarDataUrl,
            systemPrompt: systemPrompt.trimmingCharacters(in: .whitespacesAndNewlines),
            expectedUpdatedAt: expectedUpdatedAt
        )
    }

    private mutating func clearCreateCollisionWhenIdentityChanges() {
        guard let collision = createCollision else { return }
        if GaryxCustomAgentDraftRules.deriveId(from: displayName) != collision.derivedAgentId {
            createCollision = nil
        }
    }
}

public enum GaryxCustomAgentDraftRules {
    /// Reproduces the current Mac `deriveId(name)` operation order and ASCII
    /// regex semantics. Unicode letters are deliberately not transliterated.
    public static func deriveId(from name: String) -> String {
        let lowered = javascriptTrim(name).lowercased()
        var result = ""
        var hasPendingSeparator = false
        for scalar in lowered.unicodeScalars {
            let isLowercaseASCII = scalar.value >= 0x61 && scalar.value <= 0x7A
            let isDigit = scalar.value >= 0x30 && scalar.value <= 0x39
            if isLowercaseASCII || isDigit {
                if hasPendingSeparator && !result.isEmpty {
                    result.append("-")
                }
                result.unicodeScalars.append(scalar)
                hasPendingSeparator = false
            } else if !result.isEmpty {
                hasPendingSeparator = true
            }
        }
        return result
    }

    /// ECMAScript `String.prototype.trim` uses the WhiteSpace and
    /// LineTerminator code-point sets, including U+FEFF but excluding U+0085
    /// and the retired U+180E whitespace classification.
    public static func javascriptTrim(_ value: String) -> String {
        let scalars = Array(value.unicodeScalars)
        var lowerBound = 0
        var upperBound = scalars.count
        while lowerBound < upperBound, isJavaScriptTrimScalar(scalars[lowerBound]) {
            lowerBound += 1
        }
        while upperBound > lowerBound, isJavaScriptTrimScalar(scalars[upperBound - 1]) {
            upperBound -= 1
        }
        var result = ""
        for scalar in scalars[lowerBound..<upperBound] {
            result.unicodeScalars.append(scalar)
        }
        return result
    }

    public static func mutationFailure(
        for error: GaryxGatewayError,
        mode: GaryxCustomAgentDraftMode
    ) -> GaryxCustomAgentMutationFailure {
        switch error {
        case .httpStatus(409, let body, _):
            if mode.isCreate {
                return .createConflict
            }
            return .editConflict(currentUpdatedAt: currentUpdatedAt(from: body))
        case .httpStatus(404, _, _):
            if !mode.isCreate {
                return .deleted
            }
            return .other(message: error.localizedDescription)
        default:
            return .other(message: error.localizedDescription)
        }
    }

    private static func isJavaScriptTrimScalar(_ scalar: UnicodeScalar) -> Bool {
        switch scalar.value {
        case 0x0009...0x000D,
             0x0020,
             0x00A0,
             0x1680,
             0x2000...0x200A,
             0x2028,
             0x2029,
             0x202F,
             0x205F,
             0x3000,
             0xFEFF:
            return true
        default:
            return false
        }
    }

    private static func currentUpdatedAt(from body: String) -> String? {
        guard let data = body.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let value = object["current_updated_at"] as? String else {
            return nil
        }
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}

private extension GaryxCustomAgentDraftMode {
    var isCreate: Bool {
        if case .create = self { return true }
        return false
    }
}
