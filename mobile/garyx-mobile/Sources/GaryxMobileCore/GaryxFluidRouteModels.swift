import Foundation

// MARK: - Canonical route identity

public struct GaryxRouteInstanceID: RawRepresentable, Hashable, Codable, Sendable {
    public let rawValue: String

    public init(rawValue: String) {
        precondition(!rawValue.isEmpty, "route instance identity must not be empty")
        self.rawValue = rawValue
    }
}

public enum GaryxComposerKey: Hashable, Codable, Sendable {
    case thread(String)
    case draft(String)
}

public enum GaryxWorkspaceDrilldownIdentity: Hashable, Codable, Sendable {
    case workspace(path: String)
    case bot(accountID: String)
    case automationThreads(automationID: String)
}

/// The immutable identity carried by a canonical route occurrence.
///
/// Conversation and draft cases deliberately carry their complete domain
/// identity. Live data is looked up by this value instead of being copied into
/// the navigation stack.
public enum GaryxRouteDestination: Hashable, Codable, Sendable {
    case conversation(threadID: String)
    case conversationDraft(draftID: String)
    case panel(String)
    case settingsDetail(String)
    case workspaceDrilldown(GaryxWorkspaceDrilldownIdentity)

    public var composerKey: GaryxComposerKey? {
        switch self {
        case .conversation(let threadID):
            .thread(threadID)
        case .conversationDraft(let draftID):
            .draft(draftID)
        case .panel, .settingsDetail, .workspaceDrilldown:
            nil
        }
    }
}

public struct GaryxRouteEntry: Hashable, Codable, Sendable {
    /// Occurrence identity. It is intentionally independent from composerKey.
    public let id: GaryxRouteInstanceID
    public private(set) var destination: GaryxRouteDestination
    public private(set) var payloadRevision: UInt64

    public init(
        id: GaryxRouteInstanceID,
        destination: GaryxRouteDestination,
        payloadRevision: UInt64 = 0
    ) {
        self.id = id
        self.destination = destination
        self.payloadRevision = payloadRevision
    }

    public mutating func replacePayload(with destination: GaryxRouteDestination) {
        guard self.destination != destination else { return }
        self.destination = destination
        payloadRevision &+= 1
    }
}

public enum GaryxRoutePresentationNode: Hashable, Codable, Sendable {
    case home
    case entry(GaryxRouteEntry)
}

public enum GaryxRouteOpenResult: Equatable, Sendable {
    case appended(GaryxRouteInstanceID)
    case focusedExistingDraft(GaryxRouteInstanceID)
}

/// Canonical path state for the future fluid-navigation container.
///
/// This type is not wired into the application in A3. The legacy app path
/// remains untouched until A4.
public struct GaryxCanonicalRouteState: Equatable, Sendable {
    public private(set) var path: [GaryxRouteEntry]
    public private(set) var stackRevision: UInt64

    public init(path: [GaryxRouteEntry] = [], stackRevision: UInt64 = 0) {
        precondition(Set(path.map(\.id)).count == path.count, "route occurrence IDs must be unique")
        self.path = path
        self.stackRevision = stackRevision
    }

    public var topNode: GaryxRoutePresentationNode {
        path.last.map(GaryxRoutePresentationNode.entry) ?? .home
    }

    public var predecessorNode: GaryxRoutePresentationNode {
        guard path.count > 1 else { return .home }
        return .entry(path[path.count - 2])
    }

    /// Existing conversations always create a fresh occurrence. Drafts focus
    /// their single existing occurrence by trimming descendants.
    @discardableResult
    public mutating func open(_ entry: GaryxRouteEntry) -> GaryxRouteOpenResult {
        if case .conversationDraft(let draftID) = entry.destination,
           let index = path.firstIndex(where: {
               $0.destination == .conversationDraft(draftID: draftID)
           }) {
            if index != path.index(before: path.endIndex) {
                path.removeSubrange(path.index(after: index)..<path.endIndex)
                stackRevision &+= 1
            }
            return .focusedExistingDraft(path[index].id)
        }

        precondition(!path.contains(where: { $0.id == entry.id }), "route occurrence ID reused")
        path.append(entry)
        stackRevision &+= 1
        return .appended(entry.id)
    }

    @discardableResult
    public mutating func pop(count: Int = 1) -> [GaryxRouteEntry] {
        guard count > 0, !path.isEmpty else { return [] }
        let removalCount = min(count, path.count)
        let removed = Array(path.suffix(removalCount))
        path.removeLast(removalCount)
        stackRevision &+= 1
        return removed
    }

    public mutating func replacePath(_ replacement: [GaryxRouteEntry]) {
        precondition(
            Set(replacement.map(\.id)).count == replacement.count,
            "route occurrence IDs must be unique"
        )
        guard replacement != path else { return }
        path = replacement
        stackRevision &+= 1
    }

    public mutating func promoteVisibleDraft(
        instanceID: GaryxRouteInstanceID,
        draftID: String,
        threadID: String
    ) -> Bool {
        replaceRoutePayload(
            instanceID: instanceID,
            expected: .conversationDraft(draftID: draftID),
            with: .conversation(threadID: threadID)
        )
    }

    @discardableResult
    public mutating func replaceRoutePayload(
        instanceID: GaryxRouteInstanceID,
        expected: GaryxRouteDestination,
        with replacement: GaryxRouteDestination
    ) -> Bool {
        guard let index = path.firstIndex(where: { entry in
            entry.id == instanceID && entry.destination == expected
        }) else { return false }
        path[index].replacePayload(with: replacement)
        // Payload replacement is not a topology change: stackRevision stays put.
        return true
    }
}

// MARK: - Draft promotion

public enum GaryxDraftPromotionSendStage: Equatable, Sendable {
    case threadCreatedButNotDispatched
    case dispatchInFlight
    case serverAcknowledged
}

public enum GaryxDraftPromotionOutboxAdmission: Equatable, Sendable {
    case succeeded
    case failed(code: String)
}

public struct GaryxDraftPromotionRequest: Equatable, Sendable {
    public let instanceID: GaryxRouteInstanceID
    public let draftID: String
    public let threadID: String
    public let originScope: GaryxGatewayScope
    public let clientIntentID: String
    public let sendStage: GaryxDraftPromotionSendStage

    public init(
        instanceID: GaryxRouteInstanceID,
        draftID: String,
        threadID: String,
        originScope: GaryxGatewayScope,
        clientIntentID: String,
        sendStage: GaryxDraftPromotionSendStage
    ) {
        self.instanceID = instanceID
        self.draftID = draftID
        self.threadID = threadID
        self.originScope = originScope
        self.clientIntentID = clientIntentID
        self.sendStage = sendStage
    }
}

public enum GaryxDraftPromotionNavigationDisposition: Equatable, Sendable {
    /// The matching occurrence was changed in place without topology mutation.
    case updatedInPlace
    /// The occurrence has already left the stack. Domain state migrates only.
    case domainOnlyLate
    /// The event belongs to a non-current origin scope and cannot touch the path.
    case originScopePartitionOnly
    /// The origin epoch is at or below the revocation watermark. No partition
    /// or outbox mutation may be created from the late event.
    case originScopeRevoked
    /// Outbox durability failed, so the origin-scope draft remains authoritative
    /// without mutating whichever scope currently owns the visible path.
    case draftRestored
}

public enum GaryxDraftPromotionSendDisposition: Equatable, Sendable {
    case failedRetryableOutbox
    case typedFailure(code: String)
    case reconcileAmbiguous
    case acknowledged
    case rejectedRevokedScope
}

public struct GaryxDraftPromotionResult: Equatable, Sendable {
    public let navigation: GaryxDraftPromotionNavigationDisposition
    public let send: GaryxDraftPromotionSendDisposition
    public let migratedDomainInOriginScope: Bool
    public let preservedPresentationLease: Bool
    public let keptOptimisticThread: Bool
    public let outboxInsertCount: Int
    public let dispatchCountDelta: Int

    public init(
        navigation: GaryxDraftPromotionNavigationDisposition,
        send: GaryxDraftPromotionSendDisposition,
        migratedDomainInOriginScope: Bool,
        preservedPresentationLease: Bool = true,
        keptOptimisticThread: Bool,
        outboxInsertCount: Int,
        dispatchCountDelta: Int = 0
    ) {
        self.navigation = navigation
        self.send = send
        self.migratedDomainInOriginScope = migratedDomainInOriginScope
        self.preservedPresentationLease = preservedPresentationLease
        self.keptOptimisticThread = keptOptimisticThread
        self.outboxInsertCount = outboxInsertCount
        self.dispatchCountDelta = dispatchCountDelta
    }
}

/// Navigation-layer protocol used by A4 orchestration. A3 supplies the pure
/// canonical implementation and leaves storage/transport wiring out of scope.
public protocol GaryxDraftPromoting {
    mutating func promoteDraft(
        _ request: GaryxDraftPromotionRequest,
        scopes: GaryxGatewayScopeRegistry,
        outboxAdmission: GaryxDraftPromotionOutboxAdmission
    ) -> GaryxDraftPromotionResult
}

extension GaryxCanonicalRouteState: GaryxDraftPromoting {
    public mutating func promoteDraft(
        _ request: GaryxDraftPromotionRequest,
        scopes: GaryxGatewayScopeRegistry,
        outboxAdmission: GaryxDraftPromotionOutboxAdmission = .succeeded
    ) -> GaryxDraftPromotionResult {
        guard scopes.lifecycle(of: request.originScope) != .revoked else {
            return GaryxDraftPromotionResult(
                navigation: .originScopeRevoked,
                send: .rejectedRevokedScope,
                migratedDomainInOriginScope: false,
                keptOptimisticThread: false,
                outboxInsertCount: 0
            )
        }

        let sendDisposition: GaryxDraftPromotionSendDisposition
        let outboxInsertCount: Int
        let shouldCommitPromotion: Bool

        switch request.sendStage {
        case .threadCreatedButNotDispatched:
            switch outboxAdmission {
            case .succeeded:
                sendDisposition = .failedRetryableOutbox
                outboxInsertCount = 1
                shouldCommitPromotion = true
            case .failed(let code):
                sendDisposition = .typedFailure(code: code)
                outboxInsertCount = 0
                shouldCommitPromotion = false
            }
        case .dispatchInFlight:
            sendDisposition = .reconcileAmbiguous
            outboxInsertCount = 0
            shouldCommitPromotion = true
        case .serverAcknowledged:
            sendDisposition = .acknowledged
            outboxInsertCount = 0
            shouldCommitPromotion = true
        }

        guard shouldCommitPromotion else {
            return GaryxDraftPromotionResult(
                navigation: .draftRestored,
                send: sendDisposition,
                migratedDomainInOriginScope: false,
                keptOptimisticThread: false,
                outboxInsertCount: outboxInsertCount
            )
        }

        guard scopes.activeScope == request.originScope else {
            return GaryxDraftPromotionResult(
                navigation: .originScopePartitionOnly,
                send: sendDisposition,
                migratedDomainInOriginScope: true,
                keptOptimisticThread: true,
                outboxInsertCount: outboxInsertCount
            )
        }

        let updated = promoteVisibleDraft(
            instanceID: request.instanceID,
            draftID: request.draftID,
            threadID: request.threadID
        )
        return GaryxDraftPromotionResult(
            navigation: updated ? .updatedInPlace : .domainOnlyLate,
            send: sendDisposition,
            migratedDomainInOriginScope: true,
            keptOptimisticThread: true,
            outboxInsertCount: outboxInsertCount
        )
    }
}
