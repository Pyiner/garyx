import Foundation

// MARK: - Transaction ownership

public enum GaryxPresentationTransactionPhase: String, CaseIterable, Codable, Sendable {
    case active
    case preCommit
    case cancelSettle
    case commitSettle
    case terminal
}

public enum GaryxPresentationRouteOwner: Equatable, Sendable {
    case source
    case destination
}

public enum GaryxPresentationInteractionOwner: Equatable, Sendable {
    case source
    case destination
    case frozen
    case nextTransaction
}

public enum GaryxPresentationFocusOwner: Equatable, Sendable {
    case source
    case destination
    case sourceFrozen
    case deferredDestination
    case nextTransaction
}

public enum GaryxPresentationTransitionControl: Equatable, Sendable {
    case edgePanEligible
    case tracking
    case coordinatorRegrabOnly
    case locked
}

public struct GaryxPresentationOwnerSnapshot: Equatable, Sendable {
    public let canonical: GaryxPresentationRouteOwner
    public let data: GaryxPresentationRouteOwner
    public let pageInteraction: GaryxPresentationInteractionOwner
    public let focusAndAccessibility: GaryxPresentationFocusOwner
    public let transitionControl: GaryxPresentationTransitionControl

    public init(
        canonical: GaryxPresentationRouteOwner,
        data: GaryxPresentationRouteOwner,
        pageInteraction: GaryxPresentationInteractionOwner,
        focusAndAccessibility: GaryxPresentationFocusOwner,
        transitionControl: GaryxPresentationTransitionControl
    ) {
        self.canonical = canonical
        self.data = data
        self.pageInteraction = pageInteraction
        self.focusAndAccessibility = focusAndAccessibility
        self.transitionControl = transitionControl
    }
}

public enum GaryxPresentationTerminalOutcome: String, CaseIterable, Codable, Sendable {
    case committed
    case cancelled
}

public enum GaryxPresentationVisibility: String, CaseIterable, Codable, Sendable {
    case visible
    case superseded
    case inactive
}

public struct GaryxPresentationTerminalState: Equatable, Sendable {
    public let outcome: GaryxPresentationTerminalOutcome
    public let visibility: GaryxPresentationVisibility

    public init(outcome: GaryxPresentationTerminalOutcome, visibility: GaryxPresentationVisibility) {
        self.outcome = outcome
        self.visibility = visibility
    }
}

public enum GaryxPresentationFocusDisposition: Equatable, Sendable {
    case activateDestinationWhenInputReady
    case deferDestinationUntilActive
    case none
}

public enum GaryxPresentationScreenChangedDisposition: Equatable, Sendable {
    case emitExactlyOnce
    case deferUntilActive
    case none
}

public enum GaryxPresentationModalDisposition: Equatable, Sendable {
    case destinationEligible
    case restoreSourceEligibility
    case remainFrozenUntilActive
    case handoffToNextTransaction
}

public struct GaryxPresentationTerminalDisposition: Equatable, Sendable {
    public let focus: GaryxPresentationFocusDisposition
    public let screenChanged: GaryxPresentationScreenChangedDisposition
    public let modal: GaryxPresentationModalDisposition

    public init(
        focus: GaryxPresentationFocusDisposition,
        screenChanged: GaryxPresentationScreenChangedDisposition,
        modal: GaryxPresentationModalDisposition
    ) {
        self.focus = focus
        self.screenChanged = screenChanged
        self.modal = modal
    }

    public static func resolve(_ terminal: GaryxPresentationTerminalState) -> Self {
        switch (terminal.outcome, terminal.visibility) {
        case (.committed, .visible):
            Self(
                focus: .activateDestinationWhenInputReady,
                screenChanged: .emitExactlyOnce,
                modal: .destinationEligible
            )
        case (.committed, .inactive):
            Self(
                focus: .deferDestinationUntilActive,
                screenChanged: .deferUntilActive,
                modal: .remainFrozenUntilActive
            )
        case (.committed, .superseded):
            Self(focus: .none, screenChanged: .none, modal: .handoffToNextTransaction)
        case (.cancelled, .visible):
            Self(focus: .none, screenChanged: .none, modal: .restoreSourceEligibility)
        case (.cancelled, .inactive):
            Self(focus: .none, screenChanged: .none, modal: .remainFrozenUntilActive)
        case (.cancelled, .superseded):
            Self(focus: .none, screenChanged: .none, modal: .handoffToNextTransaction)
        }
    }
}

public enum GaryxPresentationCoordinatorEvent: Equatable, Sendable {
    case recognizerCancelled
    case sceneInactive
    case geometryChanged
    case keyboardGeometryChanged
    case routeInvalidated
    case gatewayForced
}

public enum GaryxPresentationEventEffect: Equatable, Sendable {
    case transitioned(GaryxPresentationTransactionPhase)
    case reachedTerminal(GaryxPresentationTerminalState)
    case rederiveGeometry
    case ignored
}

/// Pure coordinator for the four transition phases. Canonical commit is
/// represented by the owner switch at `commitSettle`; the renderer performs
/// the actual MainActor path write at that same release boundary in A4.
public struct GaryxPresentationTransactionCoordinator: Equatable, Sendable {
    public private(set) var phase: GaryxPresentationTransactionPhase
    public private(set) var terminalState: GaryxPresentationTerminalState?

    public init(phase: GaryxPresentationTransactionPhase = .active) {
        self.phase = phase
        terminalState = nil
    }

    public var owners: GaryxPresentationOwnerSnapshot {
        switch phase {
        case .active:
            return GaryxPresentationOwnerSnapshot(
                canonical: .source,
                data: .source,
                pageInteraction: .source,
                focusAndAccessibility: .source,
                transitionControl: .edgePanEligible
            )
        case .preCommit:
            return GaryxPresentationOwnerSnapshot(
                canonical: .source,
                data: .source,
                pageInteraction: .frozen,
                focusAndAccessibility: .source,
                transitionControl: .tracking
            )
        case .cancelSettle:
            return GaryxPresentationOwnerSnapshot(
                canonical: .source,
                data: .source,
                pageInteraction: .frozen,
                focusAndAccessibility: .source,
                transitionControl: .coordinatorRegrabOnly
            )
        case .commitSettle:
            return GaryxPresentationOwnerSnapshot(
                canonical: .destination,
                data: .destination,
                pageInteraction: .frozen,
                focusAndAccessibility: .sourceFrozen,
                transitionControl: .locked
            )
        case .terminal:
            return terminalOwners()
        }
    }

    public mutating func begin() -> Bool {
        guard phase == .active || phase == .terminal else { return false }
        phase = .preCommit
        terminalState = nil
        return true
    }

    public mutating func release(commit: Bool) -> Bool {
        guard phase == .preCommit else { return false }
        phase = commit ? .commitSettle : .cancelSettle
        return true
    }

    public mutating func regrabCancelSettle() -> Bool {
        guard phase == .cancelSettle else { return false }
        phase = .preCommit
        return true
    }

    @discardableResult
    public mutating func finish(visibility: GaryxPresentationVisibility) -> Bool {
        let outcome: GaryxPresentationTerminalOutcome
        switch phase {
        case .cancelSettle:
            outcome = .cancelled
        case .commitSettle:
            outcome = .committed
        case .active, .preCommit, .terminal:
            return false
        }
        terminalState = GaryxPresentationTerminalState(outcome: outcome, visibility: visibility)
        phase = .terminal
        return true
    }

    @discardableResult
    public mutating func handle(_ event: GaryxPresentationCoordinatorEvent) -> GaryxPresentationEventEffect {
        switch event {
        case .geometryChanged:
            return phase == .active || phase == .terminal ? .ignored : .rederiveGeometry
        case .keyboardGeometryChanged:
            return .ignored
        case .recognizerCancelled:
            guard phase == .preCommit else { return .ignored }
            phase = .cancelSettle
            return .transitioned(.cancelSettle)
        case .sceneInactive:
            switch phase {
            case .preCommit, .cancelSettle:
                return forceTerminal(.init(outcome: .cancelled, visibility: .inactive))
            case .commitSettle:
                return forceTerminal(.init(outcome: .committed, visibility: .inactive))
            case .active, .terminal:
                return .ignored
            }
        case .routeInvalidated, .gatewayForced:
            switch phase {
            case .preCommit, .cancelSettle:
                return forceTerminal(.init(outcome: .cancelled, visibility: .superseded))
            case .commitSettle:
                return forceTerminal(.init(outcome: .committed, visibility: .superseded))
            case .active, .terminal:
                return .ignored
            }
        }
    }

    private mutating func forceTerminal(
        _ state: GaryxPresentationTerminalState
    ) -> GaryxPresentationEventEffect {
        terminalState = state
        phase = .terminal
        return .reachedTerminal(state)
    }

    private func terminalOwners() -> GaryxPresentationOwnerSnapshot {
        guard let terminalState else {
            return GaryxPresentationOwnerSnapshot(
                canonical: .source,
                data: .source,
                pageInteraction: .source,
                focusAndAccessibility: .source,
                transitionControl: .edgePanEligible
            )
        }
        let committed = terminalState.outcome == .committed
        let routeOwner: GaryxPresentationRouteOwner = committed ? .destination : .source
        let interaction: GaryxPresentationInteractionOwner
        let focus: GaryxPresentationFocusOwner
        switch (terminalState.outcome, terminalState.visibility) {
        case (.committed, .visible):
            interaction = .destination
            focus = .destination
        case (.cancelled, .visible):
            interaction = .source
            focus = .source
        case (_, .inactive):
            interaction = .frozen
            focus = committed ? .deferredDestination : .sourceFrozen
        case (_, .superseded):
            interaction = .nextTransaction
            focus = .nextTransaction
        }
        return GaryxPresentationOwnerSnapshot(
            canonical: routeOwner,
            data: routeOwner,
            pageInteraction: interaction,
            focusAndAccessibility: focus,
            transitionControl: .edgePanEligible
        )
    }
}

// MARK: - Host lifecycle

public enum GaryxRouteHostLifecyclePhase: String, Codable, Sendable {
    case mounted
    case appeared
    case active
    case inactive
    case disappeared
}

public struct GaryxRouteHostLifecycle: Equatable, Sendable {
    public private(set) var phase: GaryxRouteHostLifecyclePhase

    public init(phase: GaryxRouteHostLifecyclePhase = .mounted) {
        self.phase = phase
    }

    @discardableResult
    public mutating func transition(to next: GaryxRouteHostLifecyclePhase) -> Bool {
        let valid: Bool = switch (phase, next) {
        case (.mounted, .appeared), (.appeared, .active), (.active, .inactive),
             (.inactive, .active), (.inactive, .disappeared):
            true
        default:
            false
        }
        guard valid else { return false }
        phase = next
        return true
    }
}

// MARK: - Presentation lease tree

public struct GaryxPresentationLeaseToken: RawRepresentable, Hashable, Codable, Sendable {
    public let rawValue: String

    public init(rawValue: String) {
        precondition(!rawValue.isEmpty, "presentation lease token must not be empty")
        self.rawValue = rawValue
    }
}

public enum GaryxPresentationLeaseJoinState: Equatable, Sendable {
    case requested
    case presented
    case dismissing
    case dismissedAwaitingResult
    case resultRecordedAwaitingDismissal
    case released
}

public enum GaryxPresentationResultDisposition: Equatable, Sendable {
    case notRequired
    case pending
    case recorded
    case explicitNoResult
}

public struct GaryxPresentationLeaseRecord: Equatable, Sendable {
    public let token: GaryxPresentationLeaseToken
    public let parent: GaryxPresentationLeaseToken?
    public let resultBearing: Bool
    public fileprivate(set) var requested: Bool
    public fileprivate(set) var presented: Bool
    public fileprivate(set) var dismissing: Bool
    public fileprivate(set) var dismissalCompleted: Bool
    public fileprivate(set) var result: GaryxPresentationResultDisposition
    public fileprivate(set) var released: Bool
    public fileprivate(set) var releaseCount: Int

    fileprivate init(
        token: GaryxPresentationLeaseToken,
        parent: GaryxPresentationLeaseToken?,
        resultBearing: Bool
    ) {
        self.token = token
        self.parent = parent
        self.resultBearing = resultBearing
        requested = true
        presented = false
        dismissing = false
        dismissalCompleted = false
        result = resultBearing ? .pending : .notRequired
        released = false
        releaseCount = 0
    }

    public var joinState: GaryxPresentationLeaseJoinState {
        if released { return .released }
        if dismissalCompleted, result == .pending { return .dismissedAwaitingResult }
        if !dismissalCompleted, result == .recorded || result == .explicitNoResult {
            return .resultRecordedAwaitingDismissal
        }
        if dismissing { return .dismissing }
        if presented { return .presented }
        return .requested
    }

    fileprivate mutating func releaseOnce() {
        guard !released else { return }
        released = true
        releaseCount += 1
    }
}

public struct GaryxPresentationLeaseTree: Equatable, Sendable {
    public private(set) var records: [GaryxPresentationLeaseToken: GaryxPresentationLeaseRecord]

    public init() {
        records = [:]
    }

    public var hasBarrier: Bool { records.values.contains(where: { !$0.released }) }

    /// Synchronous acquisition: callers invoke this before setting a
    /// presentation binding or otherwise making a presentation request visible.
    @discardableResult
    public mutating func acquire(
        _ token: GaryxPresentationLeaseToken,
        parent: GaryxPresentationLeaseToken? = nil,
        resultBearing: Bool = false
    ) -> Bool {
        guard records[token] == nil else { return false }
        if let parent {
            guard let parentRecord = records[parent], !parentRecord.released else { return false }
        }
        records[token] = GaryxPresentationLeaseRecord(
            token: token,
            parent: parent,
            resultBearing: resultBearing
        )
        return true
    }

    public mutating func markPresented(_ token: GaryxPresentationLeaseToken) {
        guard var record = records[token], !record.released else { return }
        record.presented = true
        records[token] = record
    }

    public mutating func markDismissing(_ token: GaryxPresentationLeaseToken) {
        guard var record = records[token], !record.released else { return }
        record.dismissing = true
        records[token] = record
    }

    public mutating func recordResult(_ token: GaryxPresentationLeaseToken) {
        guard var record = records[token], record.resultBearing, !record.released else { return }
        record.result = .recorded
        records[token] = record
        releaseIfJoined(token)
    }

    public mutating func recordNoResult(_ token: GaryxPresentationLeaseToken) {
        guard var record = records[token], record.resultBearing, !record.released else { return }
        record.result = .explicitNoResult
        records[token] = record
        releaseIfJoined(token)
    }

    /// Programmatic and interactive callbacks share this exactly-once path.
    public mutating func dismissalCompleted(_ token: GaryxPresentationLeaseToken) {
        guard records[token] != nil else { return }
        let subtree = subtreeTokens(root: token)
        for member in subtree {
            guard var record = records[member], !record.released else { continue }
            record.dismissalCompleted = true
            records[member] = record
            if member != token, record.resultBearing, record.result == .pending {
                // Dismissing an ancestor is an explicit no-result terminal for
                // nested result-bearing presentations.
                recordNoResult(member)
            }
        }
        for member in subtree.reversed() {
            releaseIfJoined(member)
        }
    }

    public mutating func presentationFailed(_ token: GaryxPresentationLeaseToken) {
        forceDismissSubtree(token)
    }

    public mutating func forceDismissSubtree(_ token: GaryxPresentationLeaseToken) {
        let subtree = subtreeTokens(root: token)
        for member in subtree {
            guard var record = records[member], !record.released else { continue }
            record.dismissing = true
            record.dismissalCompleted = true
            if record.resultBearing, record.result == .pending {
                record.result = .explicitNoResult
            }
            records[member] = record
        }
        for member in subtree.reversed() {
            releaseIfJoined(member)
        }
    }

    /// Released records are bounded audit state, not barriers. Once callers no
    /// longer need the exactly-once counters they can reclaim the entire
    /// released forest; late callbacks then follow the existing unknown-token
    /// idempotent path.
    @discardableResult
    public mutating func garbageCollectReleased() -> Int {
        let released = records.values.filter(\.released).map(\.token)
        for token in released { records.removeValue(forKey: token) }
        return released.count
    }

    private mutating func releaseIfJoined(_ token: GaryxPresentationLeaseToken) {
        guard var record = records[token], !record.released else { return }
        let resultTerminal = !record.resultBearing || record.result == .recorded
            || record.result == .explicitNoResult
        guard record.dismissalCompleted, resultTerminal else { return }
        record.releaseOnce()
        records[token] = record
    }

    private func subtreeTokens(root: GaryxPresentationLeaseToken) -> [GaryxPresentationLeaseToken] {
        guard records[root] != nil else { return [] }
        var result: [GaryxPresentationLeaseToken] = []
        var queue = [root]
        while !queue.isEmpty {
            let current = queue.removeFirst()
            result.append(current)
            queue.append(contentsOf: records.values
                .filter { $0.parent == current }
                .map(\.token)
                .sorted { $0.rawValue < $1.rawValue })
        }
        return result
    }
}

// MARK: - Path diff decision table

public enum GaryxPathMutationSource: Equatable, Sendable {
    case ordinary
    case declaredWholeChainReplacement
}

public enum GaryxPathDiffDecision: Equatable, Sendable {
    case noChange
    case push
    case pop
    case popMultiple(Int)
    case replaceTop
    case promoteInPlace
    case inPlacePayloadUpdate
    case popToHome
    case wholeChainReplacement
    case normalizeIllegalMutationAndLogFault
}

public enum GaryxPathDiffPlanner {
    public static func decide(
        from old: [GaryxRouteEntry],
        to new: [GaryxRouteEntry],
        source: GaryxPathMutationSource = .ordinary
    ) -> GaryxPathDiffDecision {
        if old == new { return .noChange }
        if new.isEmpty, !old.isEmpty { return .popToHome }
        if source == .declaredWholeChainReplacement { return .wholeChainReplacement }

        if new.count == old.count + 1, Array(new.dropLast()) == old {
            return .push
        }
        if old.count > new.count, Array(old.prefix(new.count)) == new {
            let count = old.count - new.count
            return count == 1 ? .pop : .popMultiple(count)
        }
        if old.count == new.count, !old.isEmpty,
           Array(old.dropLast()) == Array(new.dropLast()),
           old.last?.id != new.last?.id {
            return .replaceTop
        }
        if old.count == new.count, old.map(\.id) == new.map(\.id) {
            let changedIndices = old.indices.filter { old[$0] != new[$0] }
            if changedIndices.count == 1,
               let changedIndex = changedIndices.first,
               case .conversationDraft = old[changedIndex].destination,
               case .conversation = new[changedIndex].destination {
                return .promoteInPlace
            }
            if changedIndices == [old.index(before: old.endIndex)] {
                return .inPlacePayloadUpdate
            }
            return .normalizeIllegalMutationAndLogFault
        }
        return .normalizeIllegalMutationAndLogFault
    }
}
