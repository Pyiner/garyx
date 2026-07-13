import Foundation

public enum GaryxAvatarGenerationFailureCategory: String, Equatable, Sendable {
    case unreachable
    case timeout
    case provider
    case unusable
    case unknown
}

public struct GaryxAvatarGenerationFailure: Equatable, Sendable {
    public let category: GaryxAvatarGenerationFailureCategory
    public let message: String

    public init(
        category: GaryxAvatarGenerationFailureCategory,
        message: String? = nil
    ) {
        self.category = category
        self.message = message ?? category.userMessage
    }
}

public enum GaryxAvatarGenerationOutcome: Equatable, Sendable {
    case success(dataUrl: String)
    case failure(GaryxAvatarGenerationFailure)
    case cancelled
    case superseded
}

public extension GaryxAvatarGenerationOutcome {
    static func from(error: Error) -> Self {
        if GaryxGatewayRetryClassifier.isCancellation(error) {
            return .cancelled
        }
        if case GaryxGatewayError.httpStatus(let status, _) = error {
            switch status {
            case 504:
                return .failure(GaryxAvatarGenerationFailure(category: .timeout))
            case 502:
                return .failure(GaryxAvatarGenerationFailure(category: .provider))
            default:
                return .failure(GaryxAvatarGenerationFailure(category: .unknown))
            }
        }
        let nsError = error as NSError
        if nsError.domain == NSURLErrorDomain {
            if nsError.code == NSURLErrorTimedOut {
                return .failure(GaryxAvatarGenerationFailure(category: .timeout))
            }
            return .failure(GaryxAvatarGenerationFailure(category: .unreachable))
        }
        return .failure(GaryxAvatarGenerationFailure(category: .unknown))
    }
}

public enum GaryxAvatarGenerationPhase: Equatable, Sendable {
    case choosing
    case generating
    case candidate
    case failed(GaryxAvatarGenerationFailure)
}

public enum GaryxAvatarGenerationPrimaryAction: Equatable, Sendable {
    case generate
    case disabled
    case use
    case retry
}

public enum GaryxAvatarGenerationLeadingAction: Equatable, Sendable {
    case cancel
    case cancelGeneration
}

/// Pure state machine for the focused avatar-generation transaction.
///
/// `currentAvatarDataUrl` is the form draft snapshot and never changes until
/// `acceptCandidate()` is called. Request IDs make late success, failure, and
/// completion callbacks harmless after cancel or retry.
public struct GaryxMobileAvatarEditorState: Equatable, Sendable {
    public private(set) var phase: GaryxAvatarGenerationPhase
    public private(set) var currentAvatarDataUrl: String
    public private(set) var candidateAvatarDataUrl: String?
    public private(set) var requestId: UUID?
    public var selectedStyleId: String
    public var customStyle: String

    public init(
        currentAvatarDataUrl: String = "",
        candidateAvatarDataUrl: String? = nil,
        phase: GaryxAvatarGenerationPhase = .choosing,
        requestId: UUID? = nil,
        selectedStyleId: String = GaryxAvatarStyleOption.defaultId,
        customStyle: String = ""
    ) {
        self.phase = phase
        self.currentAvatarDataUrl = currentAvatarDataUrl
        self.candidateAvatarDataUrl = candidateAvatarDataUrl
        self.requestId = requestId
        self.selectedStyleId = selectedStyleId
        self.customStyle = customStyle
    }

    public var isGenerating: Bool {
        phase == .generating
    }

    public var hasCandidate: Bool {
        !(candidateAvatarDataUrl ?? "").isEmpty
    }

    public var primaryAction: GaryxAvatarGenerationPrimaryAction {
        switch phase {
        case .choosing:
            return .generate
        case .generating:
            return .disabled
        case .candidate:
            return .use
        case .failed:
            return .retry
        }
    }

    public var leadingAction: GaryxAvatarGenerationLeadingAction {
        phase == .generating ? .cancelGeneration : .cancel
    }

    public var activeStylePrompt: String {
        if selectedStyleId == "custom" {
            return customStyle.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return GaryxAvatarStyleOption.builtIn.first(where: { $0.id == selectedStyleId })?.prompt
            ?? GaryxAvatarStyleOption.builtIn.first?.prompt
            ?? ""
    }

    public var canGenerate: Bool {
        !activeStylePrompt.isEmpty && !isGenerating
    }

    @discardableResult
    public mutating func beginGeneration(requestId: UUID = UUID()) -> UUID? {
        guard !isGenerating, !activeStylePrompt.isEmpty else { return nil }
        self.requestId = requestId
        phase = .generating
        return requestId
    }

    /// Applies a terminal result only when it belongs to the active request.
    /// Returns false for every late/superseded callback.
    @discardableResult
    public mutating func resolve(
        _ outcome: GaryxAvatarGenerationOutcome,
        requestId: UUID
    ) -> Bool {
        guard self.requestId == requestId, phase == .generating else { return false }
        self.requestId = nil
        switch outcome {
        case .success(let dataUrl):
            let trimmed = dataUrl.trimmingCharacters(in: .whitespacesAndNewlines)
            if trimmed.isEmpty {
                phase = .failed(
                    GaryxAvatarGenerationFailure(category: .unusable)
                )
            } else {
                candidateAvatarDataUrl = trimmed
                phase = .candidate
            }
        case .failure(let failure):
            phase = .failed(failure)
        case .cancelled, .superseded:
            phase = .choosing
        }
        return true
    }

    @discardableResult
    public mutating func cancelGeneration(requestId: UUID? = nil) -> Bool {
        guard phase == .generating else { return false }
        if let requestId, self.requestId != requestId { return false }
        self.requestId = nil
        phase = .choosing
        return true
    }

    /// L1: failed → choosing while preserving style text and any prior
    /// candidate so the user can revise style without losing useful work.
    public mutating func changeStyle() {
        guard case .failed = phase else { return }
        requestId = nil
        phase = .choosing
    }

    @discardableResult
    public mutating func acceptCandidate() -> String? {
        guard phase == .candidate,
              let candidateAvatarDataUrl,
              !candidateAvatarDataUrl.isEmpty else {
            return nil
        }
        currentAvatarDataUrl = candidateAvatarDataUrl
        requestId = nil
        phase = .choosing
        return candidateAvatarDataUrl
    }

    public mutating func reset(currentAvatarDataUrl: String) {
        phase = .choosing
        self.currentAvatarDataUrl = currentAvatarDataUrl
        candidateAvatarDataUrl = nil
        requestId = nil
        selectedStyleId = GaryxAvatarStyleOption.defaultId
        customStyle = ""
    }
}

public extension GaryxAvatarGenerationFailureCategory {
    var userMessage: String {
        switch self {
        case .unreachable:
            return "Couldn’t reach the gateway."
        case .timeout:
            return "Avatar generation took too long."
        case .provider:
            return "The image provider couldn’t generate an avatar."
        case .unusable:
            return "The generated image couldn’t be used."
        case .unknown:
            return "Couldn’t generate an avatar."
        }
    }
}
