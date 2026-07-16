import Foundation

public struct GaryxFavoritePage: Equatable, Sendable {
    public var storeIncarnationId: String
    public var serverBootId: String
    public var revision: Int64
    public var threadIds: [String]

    public init(
        storeIncarnationId: String,
        serverBootId: String,
        revision: Int64,
        threadIds: [String]
    ) {
        self.storeIncarnationId = storeIncarnationId
        self.serverBootId = serverBootId
        self.revision = revision
        self.threadIds = threadIds
    }

    public init(_ page: GaryxThreadFavoritesPage) {
        self.init(
            storeIncarnationId: page.storeIncarnationId,
            serverBootId: page.serverBootId,
            revision: page.revision,
            threadIds: page.threadIds
        )
    }
}

public struct GaryxFavoriteSnapshot: Equatable, Sendable {
    public var page: GaryxFavoritePage
    public var rows: [GaryxThreadSummary]
    public var truncated: Bool

    public init(
        page: GaryxFavoritePage,
        rows: [GaryxThreadSummary],
        truncated: Bool = false
    ) {
        self.page = page
        self.rows = rows
        self.truncated = truncated
    }

    public init(_ snapshot: GaryxThreadFavoritesSnapshot) {
        page = GaryxFavoritePage(
            storeIncarnationId: snapshot.storeIncarnationId,
            serverBootId: snapshot.serverBootId,
            revision: snapshot.revision,
            threadIds: snapshot.threadIds
        )
        rows = snapshot.recent.threads
        truncated = snapshot.recent.truncated
    }
}

public enum GaryxFavoriteIntentPhase: Equatable, Sendable {
    case active
    case retryScheduled(effectToken: UInt64, cause: GaryxFavoriteRetryCause)
    case awaitVerify(effectToken: UInt64)

    fileprivate var effectToken: UInt64? {
        switch self {
        case .active: return nil
        case .retryScheduled(let token, _), .awaitVerify(let token): return token
        }
    }
}

public enum GaryxFavoriteRetryCause: Equatable, Sendable {
    case notSent
    case rejected
}

public struct GaryxFavoriteIntent: Equatable, Sendable {
    public var generation: UInt64
    public var desired: Bool
    public var phase: GaryxFavoriteIntentPhase
}

public enum GaryxFavoriteFlightOrigin: Equatable, Sendable {
    case ordinary
    case verify
}

public struct GaryxFavoriteMutationTicket: Equatable, Sendable {
    public var gatewayScope: String
    public var runtimeEpoch: UInt64
    public var requestToken: UInt64
    public var threadId: String
    public var target: Bool
    public var flightGeneration: UInt64
    public var expectedRevision: Int64
    public var expectedStoreIncarnation: String
    public var origin: GaryxFavoriteFlightOrigin
}

public struct GaryxFavoriteBackoffStamp: Equatable, Sendable {
    public var gatewayScope: String
    public var runtimeEpoch: UInt64
    public var threadId: String
    public var generation: UInt64
    public var effectToken: UInt64
}

public struct GaryxFavoritesSnapshotTicket: Equatable, Sendable {
    public var gatewayScope: String
    public var runtimeEpoch: UInt64
    public var requestToken: UInt64
}

public enum GaryxFavoritesEffect: Equatable, Sendable {
    case mutate(GaryxFavoriteMutationTicket)
    case backoff(stamp: GaryxFavoriteBackoffStamp, delayNanoseconds: UInt64)
    case snapshot(GaryxFavoritesSnapshotTicket)
    case surfaceError(threadId: String, message: String)
}

public struct GaryxStoreResponseStamp: Equatable, Sendable {
    public var gatewayScope: String
    public var runtimeEpoch: UInt64
    public var owned: Bool

    public init(gatewayScope: String, runtimeEpoch: UInt64, owned: Bool) {
        self.gatewayScope = gatewayScope
        self.runtimeEpoch = runtimeEpoch
        self.owned = owned
    }
}

public enum GaryxStoreIdentityDecision: Equatable, Sendable {
    case accept
    case drop
    case scopeClear
}

public enum GaryxFavoriteMutationSettlement: Equatable, Sendable {
    case ok(GaryxFavoritePage)
    case definitive(
        status: Int,
        code: String,
        message: String?,
        page: GaryxFavoritePage?
    )
    case ambiguous(message: String)
    case notSent(message: String)
}

public struct GaryxFavoritesState: Equatable, Sendable {
    public private(set) var gatewayScope: String
    public private(set) var runtimeEpoch: UInt64
    public private(set) var storeIncarnationId: String?
    public private(set) var rawRevision: Int64?
    public private(set) var rawThreadIds: [String]
    public private(set) var highestObservedRevision: Int64?
    public private(set) var intents: [String: GaryxFavoriteIntent]
    public private(set) var inFlight: [String: GaryxFavoriteMutationTicket]
    public private(set) var unresolvedFence: [String: Int64]
    public private(set) var favoriteRows: [GaryxThreadSummary]
    public private(set) var favoritesServerBootId: String?
    public private(set) var favoritesSnapshotTruncated: Bool
    public private(set) var activeSnapshotTicket: GaryxFavoritesSnapshotTicket?
    public private(set) var snapshotTrailingDirty: Bool

    private var nextGeneration: UInt64
    private var nextRequestToken: UInt64
    private var nextEffectToken: UInt64

    public init(gatewayScope: String = "") {
        self.gatewayScope = gatewayScope
        runtimeEpoch = 0
        storeIncarnationId = nil
        rawRevision = nil
        rawThreadIds = []
        highestObservedRevision = nil
        intents = [:]
        inFlight = [:]
        unresolvedFence = [:]
        favoriteRows = []
        favoritesServerBootId = nil
        favoritesSnapshotTruncated = false
        activeSnapshotTicket = nil
        snapshotTrailingDirty = false
        nextGeneration = 1
        nextRequestToken = 1
        nextEffectToken = 1
    }

    public func isPresented(threadId rawThreadId: String) -> Bool {
        let threadId = rawThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
        return intents[threadId]?.desired ?? rawThreadIds.contains(threadId)
    }

    public var presentedRows: [GaryxThreadSummary] {
        favoriteRows.filter { isPresented(threadId: $0.id) }
    }

    @discardableResult
    public mutating func replaceGatewayScope(
        _ scope: String,
        requestSnapshot shouldRequestSnapshot: Bool = true
    ) -> [GaryxFavoritesEffect] {
        if gatewayScope != scope {
            clearDomain(gatewayScope: scope)
        }
        return shouldRequestSnapshot ? requestSnapshot() : []
    }

    /// v24 §7.1 response judgment. Ownership/epoch are checked before the
    /// incarnation id so an old response cannot switch a new domain back.
    @discardableResult
    public mutating func observeStoreIdentity(
        stamp: GaryxStoreResponseStamp,
        responseStoreIncarnationId: String
    ) -> (decision: GaryxStoreIdentityDecision, effects: [GaryxFavoritesEffect]) {
        guard stamp.owned,
              stamp.gatewayScope == gatewayScope,
              stamp.runtimeEpoch == runtimeEpoch else {
            return (.drop, [])
        }
        guard let current = storeIncarnationId else {
            storeIncarnationId = responseStoreIncarnationId
            return (.accept, [])
        }
        guard current != responseStoreIncarnationId else {
            return (.accept, [])
        }
        clearDomain(gatewayScope: gatewayScope)
        return (.scopeClear, requestSnapshot())
    }

    @discardableResult
    public mutating func toggle(
        threadId rawThreadId: String,
        desired: Bool
    ) -> [GaryxFavoritesEffect] {
        let threadId = rawThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !threadId.isEmpty else { return [] }
        let generation = nextGeneration
        nextGeneration &+= 1
        intents[threadId] = GaryxFavoriteIntent(
            generation: generation,
            desired: desired,
            phase: .active
        )
        return drain(threadId: threadId, origin: .ordinary)
    }

    @discardableResult
    public mutating func requestSnapshot() -> [GaryxFavoritesEffect] {
        guard !gatewayScope.isEmpty else { return [] }
        guard activeSnapshotTicket == nil else {
            snapshotTrailingDirty = true
            return []
        }
        let ticket = GaryxFavoritesSnapshotTicket(
            gatewayScope: gatewayScope,
            runtimeEpoch: runtimeEpoch,
            requestToken: nextRequestToken
        )
        nextRequestToken &+= 1
        activeSnapshotTicket = ticket
        snapshotTrailingDirty = false
        return [.snapshot(ticket)]
    }

    @discardableResult
    public mutating func completeSnapshot(
        ticket: GaryxFavoritesSnapshotTicket,
        snapshot: GaryxFavoriteSnapshot
    ) -> [GaryxFavoritesEffect] {
        let identity = observeStoreIdentity(
            stamp: GaryxStoreResponseStamp(
                gatewayScope: ticket.gatewayScope,
                runtimeEpoch: ticket.runtimeEpoch,
                owned: snapshotTicketIsOwned(ticket)
            ),
            responseStoreIncarnationId: snapshot.page.storeIncarnationId
        )
        guard identity.decision == .accept else { return identity.effects }
        let trailing = snapshotTrailingDirty
        activeSnapshotTicket = nil
        snapshotTrailingDirty = false
        if let highestObservedRevision,
           snapshot.page.revision < highestObservedRevision {
            snapshotTrailingDirty = true
            return requestSnapshot()
        }
        _ = acceptRawWithoutReconcile(snapshot.page)
        favoriteRows = snapshot.rows
        favoritesServerBootId = snapshot.page.serverBootId
        favoritesSnapshotTruncated = snapshot.truncated
        var effects = reconcileAllIdleIntents()
        if trailing {
            effects += requestSnapshot()
        }
        return effects
    }

    @discardableResult
    public mutating func failSnapshot(
        ticket: GaryxFavoritesSnapshotTicket
    ) -> [GaryxFavoritesEffect] {
        guard snapshotTicketIsOwned(ticket) else { return [] }
        let trailing = snapshotTrailingDirty
        activeSnapshotTicket = nil
        snapshotTrailingDirty = false
        return trailing ? requestSnapshot() : []
    }

    @discardableResult
    public mutating func acceptReadPage(
        stamp: GaryxStoreResponseStamp,
        page: GaryxFavoritePage
    ) -> [GaryxFavoritesEffect] {
        let identity = observeStoreIdentity(
            stamp: stamp,
            responseStoreIncarnationId: page.storeIncarnationId
        )
        guard identity.decision == .accept else { return identity.effects }
        guard highestObservedRevision == nil || page.revision >= highestObservedRevision! else {
            return []
        }
        let bootChanged = favoritesServerBootId.map { $0 != page.serverBootId } ?? false
        _ = acceptRawWithoutReconcile(page)
        var effects = reconcileAllIdleIntents()
        if bootChanged {
            effects += requestSnapshot()
        }
        return effects
    }

    @discardableResult
    public mutating func settle(
        ticket: GaryxFavoriteMutationTicket,
        settlement: GaryxFavoriteMutationSettlement
    ) -> [GaryxFavoritesEffect] {
        guard mutationTicketIsOwned(ticket) else { return [] }
        let responsePage: GaryxFavoritePage?
        switch settlement {
        case .ok(let page): responsePage = page
        case .definitive(_, _, _, let page): responsePage = page
        case .ambiguous, .notSent: responsePage = nil
        }
        if let responsePage {
            let identity = observeStoreIdentity(
                stamp: GaryxStoreResponseStamp(
                    gatewayScope: ticket.gatewayScope,
                    runtimeEpoch: ticket.runtimeEpoch,
                    owned: true
                ),
                responseStoreIncarnationId: responsePage.storeIncarnationId
            )
            guard identity.decision == .accept else { return identity.effects }
        }

        let bootChanged = responsePage.flatMap { page in
            favoritesServerBootId.map { $0 != page.serverBootId }
        } ?? false
        let effects: [GaryxFavoritesEffect]
        switch settlement {
        case .ok(let page):
            effects = settleApplied(ticket: ticket, page: page)
        case .ambiguous:
            effects = settleDeferred(ticket: ticket, cause: .ambiguous)
        case .notSent:
            effects = settleDeferred(ticket: ticket, cause: .notSent)
        case .definitive(let status, let code, let message, let page):
            if code == "wrong_incarnation" {
                effects = settleDeferred(ticket: ticket, cause: .rejected)
            } else if status == 409, let page {
                effects = settleConflict(ticket: ticket, page: page)
            } else if status == 404 {
                effects = settleNotFound(ticket: ticket, page: page)
            } else if status == 429 || code == "unavailable" || code == "temporarily_unavailable" {
                effects = settleDeferred(ticket: ticket, cause: .rejected)
            } else {
                effects = settleTerminalRejection(
                    ticket: ticket,
                    message: message ?? (code.isEmpty ? "Failed to update favorite." : code)
                )
            }
        }
        guard bootChanged else { return effects }
        return effects + requestSnapshot()
    }

    @discardableResult
    public mutating func fireBackoff(
        _ stamp: GaryxFavoriteBackoffStamp
    ) -> [GaryxFavoritesEffect] {
        guard stamp.gatewayScope == gatewayScope,
              stamp.runtimeEpoch == runtimeEpoch,
              inFlight[stamp.threadId] == nil,
              var intent = intents[stamp.threadId],
              intent.generation == stamp.generation,
              intent.phase.effectToken == stamp.effectToken else {
            return []
        }
        let origin: GaryxFavoriteFlightOrigin
        switch intent.phase {
        case .awaitVerify: origin = .verify
        case .retryScheduled: origin = .ordinary
        case .active: return []
        }
        intent.phase = .active
        intents[stamp.threadId] = intent
        return drain(threadId: stamp.threadId, origin: origin)
    }

    private enum DeferredCause: Equatable {
        case ambiguous
        case notSent
        case rejected
    }

    private mutating func clearDomain(gatewayScope scope: String) {
        let generation = nextGeneration
        let requestToken = nextRequestToken
        let effectToken = nextEffectToken
        let nextEpoch = runtimeEpoch &+ 1
        self = GaryxFavoritesState(gatewayScope: scope)
        runtimeEpoch = nextEpoch
        nextGeneration = generation
        nextRequestToken = requestToken
        nextEffectToken = effectToken
    }

    private func snapshotTicketIsOwned(_ ticket: GaryxFavoritesSnapshotTicket) -> Bool {
        activeSnapshotTicket == ticket
    }

    private func mutationTicketIsOwned(_ ticket: GaryxFavoriteMutationTicket) -> Bool {
        inFlight[ticket.threadId] == ticket
    }

    @discardableResult
    private mutating func acceptRawWithoutReconcile(_ page: GaryxFavoritePage) -> Bool {
        if let highestObservedRevision, page.revision < highestObservedRevision {
            return false
        }
        rawRevision = page.revision
        rawThreadIds = Self.uniqueIds(page.threadIds)
        highestObservedRevision = max(highestObservedRevision ?? page.revision, page.revision)
        for (threadId, fence) in unresolvedFence where page.revision > fence {
            unresolvedFence[threadId] = nil
        }
        return true
    }

    private mutating func settleApplied(
        ticket: GaryxFavoriteMutationTicket,
        page: GaryxFavoritePage
    ) -> [GaryxFavoritesEffect] {
        guard acceptRawWithoutReconcile(page), page.revision > ticket.expectedRevision else {
            return settleDeferred(ticket: ticket, cause: .ambiguous)
        }
        inFlight[ticket.threadId] = nil
        var effects = reconcileAllIdleIntents(excluding: ticket.threadId)
        guard let intent = intents[ticket.threadId],
              intent.generation > ticket.flightGeneration else {
            intents[ticket.threadId] = nil
            return effects
        }
        effects += resolveCurrentIntentAfterRaw(
            threadId: ticket.threadId,
            forceActiveDrain: true
        )
        return effects
    }

    private mutating func settleConflict(
        ticket: GaryxFavoriteMutationTicket,
        page: GaryxFavoritePage
    ) -> [GaryxFavoritesEffect] {
        guard acceptRawWithoutReconcile(page) else {
            return settleDeferred(ticket: ticket, cause: .ambiguous)
        }
        inFlight[ticket.threadId] = nil
        var effects = reconcileAllIdleIntents(excluding: ticket.threadId)
        guard let intent = intents[ticket.threadId] else { return effects }
        if intent.desired != rawContains(ticket.threadId) {
            intents[ticket.threadId]?.phase = .active
            effects += drain(threadId: ticket.threadId, origin: .ordinary)
            return effects
        }
        if retirementGatePasses(ticket.threadId) {
            intents[ticket.threadId] = nil
            return effects
        }
        effects += schedule(threadId: ticket.threadId, awaitVerify: true, cause: .rejected)
        return effects
    }

    private mutating func settleNotFound(
        ticket: GaryxFavoriteMutationTicket,
        page: GaryxFavoritePage?
    ) -> [GaryxFavoritesEffect] {
        var effects: [GaryxFavoritesEffect] = []
        if let page {
            _ = acceptRawWithoutReconcile(page)
            effects += reconcileAllIdleIntents(excluding: ticket.threadId)
        }
        inFlight[ticket.threadId] = nil
        intents[ticket.threadId] = nil
        unresolvedFence[ticket.threadId] = nil
        return effects
    }

    private mutating func settleTerminalRejection(
        ticket: GaryxFavoriteMutationTicket,
        message: String
    ) -> [GaryxFavoritesEffect] {
        inFlight[ticket.threadId] = nil
        guard let intent = intents[ticket.threadId] else { return [] }
        if intent.generation == ticket.flightGeneration {
            intents[ticket.threadId] = nil
            return [.surfaceError(threadId: ticket.threadId, message: message)]
        }
        intents[ticket.threadId]?.phase = .active
        return resolveCurrentIntentAfterRaw(
            threadId: ticket.threadId,
            forceActiveDrain: true
        )
    }

    private mutating func settleDeferred(
        ticket: GaryxFavoriteMutationTicket,
        cause: DeferredCause
    ) -> [GaryxFavoritesEffect] {
        inFlight[ticket.threadId] = nil
        if cause == .ambiguous {
            unresolvedFence[ticket.threadId] = min(
                unresolvedFence[ticket.threadId] ?? ticket.expectedRevision,
                ticket.expectedRevision
            )
        }
        guard let intent = intents[ticket.threadId] else { return [] }
        if intent.generation != ticket.flightGeneration {
            intents[ticket.threadId]?.phase = .active
            return resolveCurrentIntentAfterRaw(
                threadId: ticket.threadId,
                forceActiveDrain: true
            )
        }
        return schedule(
            threadId: ticket.threadId,
            awaitVerify: cause == .ambiguous,
            cause: cause == .notSent ? .notSent : .rejected
        )
    }

    private mutating func schedule(
        threadId: String,
        awaitVerify: Bool,
        cause: GaryxFavoriteRetryCause
    ) -> [GaryxFavoritesEffect] {
        guard var intent = intents[threadId] else { return [] }
        let effectToken = nextEffectToken
        nextEffectToken &+= 1
        intent.phase = awaitVerify
            ? .awaitVerify(effectToken: effectToken)
            : .retryScheduled(effectToken: effectToken, cause: cause)
        intents[threadId] = intent
        return [
            .backoff(
                stamp: GaryxFavoriteBackoffStamp(
                    gatewayScope: gatewayScope,
                    runtimeEpoch: runtimeEpoch,
                    threadId: threadId,
                    generation: intent.generation,
                    effectToken: effectToken
                ),
                delayNanoseconds: 750_000_000
            ),
        ]
    }

    private mutating func reconcileAllIdleIntents(
        excluding excludedThreadId: String? = nil
    ) -> [GaryxFavoritesEffect] {
        var effects: [GaryxFavoritesEffect] = []
        for threadId in intents.keys.sorted()
            where threadId != excludedThreadId && inFlight[threadId] == nil {
            effects += resolveCurrentIntentAfterRaw(
                threadId: threadId,
                forceActiveDrain: false
            )
        }
        return effects
    }

    private mutating func resolveCurrentIntentAfterRaw(
        threadId: String,
        forceActiveDrain: Bool
    ) -> [GaryxFavoritesEffect] {
        guard let intent = intents[threadId], rawRevision != nil,
              inFlight[threadId] == nil else { return [] }
        let equal = rawContains(threadId) == intent.desired
        if equal, retirementGatePasses(threadId) {
            intents[threadId] = nil
            return []
        }
        if forceActiveDrain || intent.phase == .active {
            // Equality cannot retire an intent while an ambiguous older
            // flight still fences this revision. Issue a compensating CAS so
            // the next accepted page advances the baseline past that fence.
            intents[threadId]?.phase = .active
            return drain(threadId: threadId, origin: .ordinary)
        }
        switch intent.phase {
        case .awaitVerify:
            guard unresolvedFence[threadId] == nil else { return [] }
            if equal {
                intents[threadId] = nil
                return []
            }
            intents[threadId]?.phase = .active
            return drain(threadId: threadId, origin: .ordinary)
        case .retryScheduled:
            // R11-8: a raw mismatch cannot bypass the existing backoff timer.
            return []
        case .active:
            return []
        }
    }

    private mutating func drain(
        threadId: String,
        origin: GaryxFavoriteFlightOrigin
    ) -> [GaryxFavoritesEffect] {
        guard let intent = intents[threadId],
              inFlight[threadId] == nil,
              let rawRevision,
              let storeIncarnationId else { return [] }
        let ticket = GaryxFavoriteMutationTicket(
            gatewayScope: gatewayScope,
            runtimeEpoch: runtimeEpoch,
            requestToken: nextRequestToken,
            threadId: threadId,
            target: intent.desired,
            flightGeneration: intent.generation,
            expectedRevision: rawRevision,
            expectedStoreIncarnation: storeIncarnationId,
            origin: origin
        )
        nextRequestToken &+= 1
        inFlight[threadId] = ticket
        return [.mutate(ticket)]
    }

    private func rawContains(_ threadId: String) -> Bool {
        rawThreadIds.contains(threadId)
    }

    private func retirementGatePasses(_ threadId: String) -> Bool {
        guard let fence = unresolvedFence[threadId] else { return true }
        return rawRevision.map { $0 > fence } ?? false
    }

    private static func uniqueIds(_ ids: [String]) -> [String] {
        var seen = Set<String>()
        return ids.compactMap { rawId in
            let id = rawId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !id.isEmpty, seen.insert(id).inserted else { return nil }
            return id
        }
    }
}
