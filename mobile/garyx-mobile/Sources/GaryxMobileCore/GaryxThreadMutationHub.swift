import Foundation

public struct GaryxThreadMutationID: Equatable, Hashable, Sendable, ExpressibleByStringLiteral {
    public var rawValue: String

    public init(_ rawValue: String) {
        self.rawValue = rawValue
    }

    public init(stringLiteral value: String) {
        rawValue = value
    }
}

public enum GaryxThreadMutationKind: Equatable, Sendable {
    case archive(threadId: String)
    case pin(threadId: String, pinned: Bool)
    case insert(threadId: String)
    case rename(threadId: String)
    case favoriteDownstream(threadId: String, favorited: Bool)

    public var threadId: String {
        switch self {
        case .archive(let threadId), .pin(let threadId, _), .insert(let threadId),
             .rename(let threadId), .favoriteDownstream(let threadId, _):
            return threadId
        }
    }
}

public enum GaryxThreadMutationPhase: Equatable, Sendable {
    case began
    case committed
    case rolledBack(message: String?)
    case ambiguous
}

public struct GaryxThreadMutationRecord: Equatable, Sendable {
    public var id: GaryxThreadMutationID
    public var kind: GaryxThreadMutationKind
    public var gatewayRuntimeEpoch: UInt64
    public var phase: GaryxThreadMutationPhase
    /// Terminal server truth retained with the transaction event so every
    /// subscriber (including the summary-cache owner) observes the same
    /// revision/membership payload.
    public var authority: GaryxThreadMutationAuthority?
}

public enum GaryxThreadMutationMembershipAuthority: Equatable, Sendable {
    case unchanged
    case remove(threadId: String)
    case upsertAtHead(threadId: String)
    case replace(orderedThreadIds: [String], revision: Int64?)
}

public struct GaryxThreadMutationAuthority: Equatable, Sendable {
    public var membership: GaryxThreadMutationMembershipAuthority
    public var summary: GaryxThreadSummary?
    public var favoriteRevision: Int64?

    public init(
        membership: GaryxThreadMutationMembershipAuthority = .unchanged,
        summary: GaryxThreadSummary? = nil,
        favoriteRevision: Int64? = nil
    ) {
        self.membership = membership
        self.summary = summary
        self.favoriteRevision = favoriteRevision
    }
}

public struct GaryxThreadMutationPendingState: Equatable, Sendable {
    public var kind: GaryxThreadMutationKind
    /// Archive motion is cancelled at ambiguity while the logical pending
    /// state remains behind the reconstruction barrier.
    public var showsMotion: Bool
    public var ambiguous: Bool
}

public enum GaryxThreadReconstructionState: Equatable, Sendable {
    case pending
    case failed(message: String)
}

public struct GaryxThreadReconstructionBarrier: Equatable, Sendable {
    public var generation: UInt64
    public var coveredMutationIds: Set<GaryxThreadMutationID>
    public var state: GaryxThreadReconstructionState
}

public struct GaryxThreadMutationResidentState: Equatable, Sendable {
    public var storeId: String
    public var instanceId: UInt64
    public var orderedThreadIds: [String]
    public var pending: [GaryxThreadMutationID: GaryxThreadMutationPendingState]
    public var barrier: GaryxThreadReconstructionBarrier?

    public init(storeId: String, instanceId: UInt64, orderedThreadIds: [String]) {
        self.storeId = storeId
        self.instanceId = instanceId
        self.orderedThreadIds = Self.uniqueIds(orderedThreadIds)
        pending = [:]
        barrier = nil
    }

    private static func uniqueIds(_ rawIds: [String]) -> [String] {
        var seen = Set<String>()
        return rawIds.compactMap { rawId in
            let id = rawId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !id.isEmpty, seen.insert(id).inserted else { return nil }
            return id
        }
    }
}

public struct GaryxThreadReconstructionTicket: Equatable, Hashable, Sendable {
    public var storeId: String
    public var instanceId: UInt64
    public var generation: UInt64
    public var gatewayRuntimeEpoch: UInt64
}

public enum GaryxThreadReconstructionOutcome: Equatable, Sendable {
    case authoritative(orderedThreadIds: [String])
    case failed(message: String)
}

public enum GaryxThreadReconstructionCompletion: Equatable, Sendable {
    case accepted
    case rejectedStaleTicket
}

/// Transactional fan-out shared by all resident list stores. Favorites owns
/// its own ambiguity/CAS reducer and enters only through terminal downstream
/// fan-out methods below.
public struct GaryxThreadMutationHub: Equatable, Sendable {
    public private(set) var gatewayRuntimeEpoch: UInt64
    public private(set) var transactions: [GaryxThreadMutationID: GaryxThreadMutationRecord]
    public private(set) var residents: [String: GaryxThreadMutationResidentState]
    private var nextBarrierGeneration: UInt64

    public init(gatewayRuntimeEpoch: UInt64 = 0) {
        self.gatewayRuntimeEpoch = gatewayRuntimeEpoch
        transactions = [:]
        residents = [:]
        nextBarrierGeneration = 1
    }

    public mutating func registerStore(
        storeId: String,
        instanceId: UInt64,
        orderedThreadIds: [String]
    ) {
        if var current = residents[storeId], current.instanceId == instanceId {
            current.orderedThreadIds = Self.uniqueIds(orderedThreadIds)
            residents[storeId] = current
            return
        }
        // A re-entered, evicted store is a cold authoritative load. No old
        // barrier or pending overlay crosses the instance boundary.
        residents[storeId] = GaryxThreadMutationResidentState(
            storeId: storeId,
            instanceId: instanceId,
            orderedThreadIds: orderedThreadIds
        )
    }

    public mutating func evictStore(storeId: String, instanceId: UInt64) {
        guard residents[storeId]?.instanceId == instanceId else { return }
        // Eviction itself completes this resident's barrier: re-entry must
        // cold-load authoritative membership under a fresh instance.
        residents[storeId] = nil
    }

    @discardableResult
    public mutating func began(
        mutationId: GaryxThreadMutationID,
        kind: GaryxThreadMutationKind,
        gatewayRuntimeEpoch: UInt64,
        affectedStoreIds: Set<String>? = nil
    ) -> Bool {
        guard gatewayRuntimeEpoch == self.gatewayRuntimeEpoch,
              transactions[mutationId] == nil else { return false }
        transactions[mutationId] = GaryxThreadMutationRecord(
            id: mutationId,
            kind: kind,
            gatewayRuntimeEpoch: gatewayRuntimeEpoch,
            phase: .began,
            authority: nil
        )
        for storeId in residents.keys.sorted()
            where affectedStoreIds?.contains(storeId) != false {
            residents[storeId]?.pending[mutationId] = GaryxThreadMutationPendingState(
                kind: kind,
                showsMotion: true,
                ambiguous: false
            )
        }
        return true
    }

    @discardableResult
    public mutating func committed(
        mutationId: GaryxThreadMutationID,
        gatewayRuntimeEpoch: UInt64,
        authority: GaryxThreadMutationAuthority = GaryxThreadMutationAuthority()
    ) -> Bool {
        guard owns(mutationId, epoch: gatewayRuntimeEpoch) else { return false }
        transactions[mutationId]?.phase = .committed
        transactions[mutationId]?.authority = authority
        for storeId in residents.keys {
            guard residents[storeId]?.pending[mutationId] != nil else { continue }
            apply(authority.membership, to: &residents[storeId]!.orderedThreadIds)
            residents[storeId]?.pending[mutationId] = nil
            removeFromBarrier(mutationId, storeId: storeId)
        }
        return true
    }

    @discardableResult
    public mutating func rolledBack(
        mutationId: GaryxThreadMutationID,
        gatewayRuntimeEpoch: UInt64,
        message: String? = nil
    ) -> Bool {
        guard owns(mutationId, epoch: gatewayRuntimeEpoch) else { return false }
        transactions[mutationId]?.phase = .rolledBack(message: message)
        transactions[mutationId]?.authority = nil
        for storeId in residents.keys {
            residents[storeId]?.pending[mutationId] = nil
            removeFromBarrier(mutationId, storeId: storeId)
        }
        return true
    }

    /// Marks a transaction unknowable and returns one replacement ticket per
    /// resident. A newly queued ambiguity always supersedes the previous
    /// generation, so an older completion cannot clear it.
    @discardableResult
    public mutating func ambiguous(
        mutationId: GaryxThreadMutationID,
        gatewayRuntimeEpoch: UInt64
    ) -> [GaryxThreadReconstructionTicket] {
        guard owns(mutationId, epoch: gatewayRuntimeEpoch) else { return [] }
        transactions[mutationId]?.phase = .ambiguous
        transactions[mutationId]?.authority = nil
        var tickets: [GaryxThreadReconstructionTicket] = []
        for storeId in residents.keys.sorted() {
            guard var pending = residents[storeId]?.pending[mutationId] else { continue }
            pending.ambiguous = true
            if case .archive = pending.kind {
                pending.showsMotion = false
            }
            residents[storeId]?.pending[mutationId] = pending
            let covered = (residents[storeId]?.barrier?.coveredMutationIds ?? [])
                .union([mutationId])
            let generation = nextBarrierGeneration
            nextBarrierGeneration &+= 1
            residents[storeId]?.barrier = GaryxThreadReconstructionBarrier(
                generation: generation,
                coveredMutationIds: covered,
                state: .pending
            )
            if let instanceId = residents[storeId]?.instanceId {
                tickets.append(
                    GaryxThreadReconstructionTicket(
                        storeId: storeId,
                        instanceId: instanceId,
                        generation: generation,
                        gatewayRuntimeEpoch: gatewayRuntimeEpoch
                    )
                )
            }
        }
        return tickets
    }

    @discardableResult
    public mutating func completeReconstruction(
        _ ticket: GaryxThreadReconstructionTicket,
        outcome: GaryxThreadReconstructionOutcome
    ) -> GaryxThreadReconstructionCompletion {
        guard ticket.gatewayRuntimeEpoch == gatewayRuntimeEpoch,
              var resident = residents[ticket.storeId],
              resident.instanceId == ticket.instanceId,
              let barrier = resident.barrier,
              barrier.generation == ticket.generation,
              barrier.state == .pending else {
            return .rejectedStaleTicket
        }
        switch outcome {
        case .failed(let message):
            resident.barrier?.state = .failed(message: message)
        case .authoritative(let orderedThreadIds):
            resident.orderedThreadIds = Self.uniqueIds(orderedThreadIds)
            for mutationId in barrier.coveredMutationIds {
                resident.pending[mutationId] = nil
            }
            resident.barrier = nil
        }
        residents[ticket.storeId] = resident
        return .accepted
    }

    /// Sticky failures retry only through an explicitly newer generation.
    public mutating func retryReconstruction(
        storeId: String
    ) -> GaryxThreadReconstructionTicket? {
        guard var resident = residents[storeId],
              resident.barrier != nil else { return nil }
        let generation = nextBarrierGeneration
        nextBarrierGeneration &+= 1
        resident.barrier?.generation = generation
        resident.barrier?.state = .pending
        residents[storeId] = resident
        return GaryxThreadReconstructionTicket(
            storeId: storeId,
            instanceId: resident.instanceId,
            generation: generation,
            gatewayRuntimeEpoch: gatewayRuntimeEpoch
        )
    }

    /// Favorites reducer is the sole judge of CAS/verify/ambiguity. Once it
    /// has committed a result, the hub performs downstream fan-out only.
    public mutating func fanOutFavoritesCommitted(
        mutationId: GaryxThreadMutationID,
        threadId: String,
        favorited: Bool,
        authority: GaryxThreadMutationAuthority = GaryxThreadMutationAuthority()
    ) {
        let kind = GaryxThreadMutationKind.favoriteDownstream(
            threadId: threadId,
            favorited: favorited
        )
        transactions[mutationId] = GaryxThreadMutationRecord(
            id: mutationId,
            kind: kind,
            gatewayRuntimeEpoch: gatewayRuntimeEpoch,
            phase: .committed,
            authority: authority
        )
        for storeId in residents.keys {
            apply(authority.membership, to: &residents[storeId]!.orderedThreadIds)
        }
    }

    public mutating func resetGatewayScope(runtimeEpoch: UInt64) {
        gatewayRuntimeEpoch = runtimeEpoch
        transactions = [:]
        residents = [:]
        nextBarrierGeneration &+= 1
    }

    private func owns(_ mutationId: GaryxThreadMutationID, epoch: UInt64) -> Bool {
        guard epoch == gatewayRuntimeEpoch,
              let transaction = transactions[mutationId] else { return false }
        return transaction.gatewayRuntimeEpoch == epoch
            && (transaction.phase == .began || transaction.phase == .ambiguous)
    }

    private mutating func removeFromBarrier(
        _ mutationId: GaryxThreadMutationID,
        storeId: String
    ) {
        guard var barrier = residents[storeId]?.barrier else { return }
        barrier.coveredMutationIds.remove(mutationId)
        residents[storeId]?.barrier = barrier.coveredMutationIds.isEmpty ? nil : barrier
    }

    private func apply(
        _ authority: GaryxThreadMutationMembershipAuthority,
        to ids: inout [String]
    ) {
        switch authority {
        case .unchanged:
            break
        case .remove(let rawThreadId):
            let threadId = rawThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
            ids.removeAll { $0 == threadId }
        case .upsertAtHead(let rawThreadId):
            let threadId = rawThreadId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !threadId.isEmpty else { return }
            ids.removeAll { $0 == threadId }
            ids.insert(threadId, at: 0)
        case .replace(let orderedThreadIds, _):
            ids = Self.uniqueIds(orderedThreadIds)
        }
    }

    private static func uniqueIds(_ rawIds: [String]) -> [String] {
        var seen = Set<String>()
        return rawIds.compactMap { rawId in
            let id = rawId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !id.isEmpty, seen.insert(id).inserted else { return nil }
            return id
        }
    }
}
