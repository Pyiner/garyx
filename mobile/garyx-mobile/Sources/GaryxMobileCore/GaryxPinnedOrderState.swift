import Foundation

struct GaryxPinnedOrderPage: Equatable, Sendable {
    var threadIds: [String]
    var revision: Int64

    init(threadIds: [String], revision: Int64) {
        self.threadIds = GaryxPinnedOrderState.normalized(threadIds)
        self.revision = max(0, revision)
    }
}

struct GaryxPinnedOrderRequestStamp: Equatable, Sendable {
    var gatewayIdentity: String
    var epoch: UInt64
}

struct GaryxPinnedOrderOutbox: Codable, Equatable, Sendable {
    var gatewayIdentity: String
    var desiredOrder: [String]
    var lastKnownRevision: Int64

    init(gatewayIdentity: String, desiredOrder: [String], lastKnownRevision: Int64) {
        self.gatewayIdentity = gatewayIdentity
        self.desiredOrder = GaryxPinnedOrderState.normalized(desiredOrder)
        self.lastKnownRevision = max(0, lastKnownRevision)
    }
}

protocol GaryxPinnedOrderOutboxPersisting {
    func loadPinnedOrderOutbox(gatewayIdentity: String) -> GaryxPinnedOrderOutbox?
    func savePinnedOrderOutbox(_ outbox: GaryxPinnedOrderOutbox?, gatewayIdentity: String)
}

struct GaryxPinnedOrderReorderRequest: Equatable, Sendable {
    var token: UInt64
    var stamp: GaryxPinnedOrderRequestStamp
    var threadIds: [String]
    var expectedRevision: Int64
}

struct GaryxPinnedOrderMembershipRequest: Equatable, Sendable {
    var token: UInt64
    var stamp: GaryxPinnedOrderRequestStamp
    var threadId: String
    var pinned: Bool
}

enum GaryxPinnedOrderAcceptanceOutcome: Equatable, Sendable {
    case discardedBelowFloor
    case merged
    case authoritative
}

enum GaryxPinnedOrderSyncState: Equatable, Sendable {
    case settled
    case ready
    case inFlight
    case waitingForMembership
    case coalescedBehindFlight
    case retryScheduled(attempt: Int, notBefore: TimeInterval)
    case pausedPermanent(statusCode: Int?)
}

enum GaryxPinnedOrderReorderFailure: Equatable, Sendable {
    case retryable(delay: TimeInterval)
    case permanent(statusCode: Int?)
    case cancelled
}

enum GaryxPinnedOrderEffect: Equatable, Sendable {
    case publish([String])
    case persist(GaryxPinnedOrderOutbox?, gatewayIdentity: String)
    case sendReorder(GaryxPinnedOrderReorderRequest)
    case noteLocalMutation
}

struct GaryxPinnedOrderUpdate: Equatable, Sendable {
    var identityAccepted = true
    var acceptance: GaryxPinnedOrderAcceptanceOutcome?
    var membershipRequest: GaryxPinnedOrderMembershipRequest?
    var effects: [GaryxPinnedOrderEffect] = []

    static var wrongIdentity: GaryxPinnedOrderUpdate {
        GaryxPinnedOrderUpdate(identityAccepted: false)
    }
}

/// Pure, gateway-scoped authority for pinned membership and order.
///
/// Transport owners feed complete response events into this value. Each event
/// enforces identity -> transport completion -> revision acceptance ->
/// publication -> one drain, so effects cannot accidentally dispatch with a
/// floor that the same response is about to supersede.
struct GaryxPinnedOrderState: Equatable, Sendable {
    private enum MembershipPhase: Equatable, Sendable {
        case live
        case retiredPin(completionRevision: Int64)
    }

    private struct MembershipIntent: Equatable, Sendable {
        var token: UInt64
        var targetPinned: Bool
        var originallyPinned: Bool
        var rollbackOrder: [String]
        var phase: MembershipPhase
    }

    private struct DragSession: Equatable, Sendable {
        var baseline: [String]
        var preview: [String]
        var previewChanged: Bool
        var acceptedBuffer: [String]?
    }

    private(set) var gatewayIdentity: String
    private(set) var desiredOrder: [String]
    private(set) var epoch: UInt64 = 0
    private(set) var highestObservedRevision: Int64
    private(set) var outbox: GaryxPinnedOrderOutbox?
    private(set) var pendingSync: GaryxPinnedOrderSyncState
    private(set) var activeReorderFlight: GaryxPinnedOrderReorderRequest?
    private(set) var wakeRequested = false

    private var resolvedOrder: [String]
    private var publishedOrder: [String]
    private var latestAcceptedRawOrder: [String]?
    private var membershipIntents: [String: MembershipIntent] = [:]
    private var dragSession: DragSession?
    private var nextToken: UInt64 = 0
    private var retryAttempt = 0
    private var retryNotBefore: TimeInterval?
    private var permanentPauseStatus: Int?

    init(
        gatewayIdentity: String,
        initialOrder: [String] = [],
        revision: Int64 = 0,
        restoredOutbox: GaryxPinnedOrderOutbox? = nil
    ) {
        let initial = Self.normalized(initialOrder)
        let restored = restoredOutbox.flatMap { candidate in
            candidate.gatewayIdentity == gatewayIdentity ? candidate : nil
        }
        let floor = max(0, revision, restored?.lastKnownRevision ?? 0)
        let restoredDomainOutbox = restored.map {
            GaryxPinnedOrderOutbox(
                gatewayIdentity: gatewayIdentity,
                desiredOrder: $0.desiredOrder,
                lastKnownRevision: floor
            )
        }
        self.gatewayIdentity = gatewayIdentity
        highestObservedRevision = floor
        outbox = restoredDomainOutbox
        desiredOrder = outbox?.desiredOrder ?? initial
        resolvedOrder = outbox?.desiredOrder ?? initial
        publishedOrder = outbox?.desiredOrder ?? initial
        pendingSync = outbox == nil ? .settled : .ready
    }

    var presentedOrder: [String] {
        dragSession?.preview ?? resolvedOrder
    }

    var isDragging: Bool { dragSession != nil }
    var isUnsettled: Bool { outbox != nil }
    var hasPendingSync: Bool { outbox != nil }
    var liveMembershipIntentCount: Int {
        membershipIntents.values.filter { intent in
            if case .live = intent.phase { return true }
            return false
        }.count
    }

    func requestStamp() -> GaryxPinnedOrderRequestStamp {
        GaryxPinnedOrderRequestStamp(gatewayIdentity: gatewayIdentity, epoch: epoch)
    }

    mutating func beginDrag() -> GaryxPinnedOrderUpdate {
        guard dragSession == nil else { return GaryxPinnedOrderUpdate() }
        dragSession = DragSession(
            baseline: presentedOrder,
            preview: presentedOrder,
            previewChanged: false,
            acceptedBuffer: nil
        )
        return GaryxPinnedOrderUpdate()
    }

    mutating func previewDrag(order: [String]) -> GaryxPinnedOrderUpdate {
        guard var session = dragSession else { return GaryxPinnedOrderUpdate() }
        let preview = Self.overlay(order: order, onMembershipOrder: resolvedOrder)
        session.preview = preview
        session.previewChanged = session.previewChanged || preview != session.baseline
        dragSession = session
        return GaryxPinnedOrderUpdate()
    }

    mutating func acceptDrop(now: TimeInterval = 0) -> GaryxPinnedOrderUpdate {
        guard let session = dragSession else { return GaryxPinnedOrderUpdate() }
        guard session.previewChanged else { return cancelDrag() }

        var effects: [GaryxPinnedOrderEffect] = []
        let committed = Self.overlay(order: session.preview, onMembershipOrder: resolvedOrder)
        dragSession = nil
        resolvedOrder = committed
        desiredOrder = committed
        epoch &+= 1
        effects.append(.noteLocalMutation)

        outbox = makeOutbox()
        effects.append(.persist(outbox, gatewayIdentity: gatewayIdentity))
        wakeRequested = true
        appendPublicationIfChanged(effects: &effects)
        drain(now: now, effects: &effects)
        return GaryxPinnedOrderUpdate(effects: effects)
    }

    mutating func cancelDrag() -> GaryxPinnedOrderUpdate {
        guard dragSession != nil else { return GaryxPinnedOrderUpdate() }
        dragSession = nil
        var effects: [GaryxPinnedOrderEffect] = []
        appendPublicationIfChanged(effects: &effects)
        return GaryxPinnedOrderUpdate(effects: effects)
    }

    mutating func beginMembershipChange(
        threadId rawThreadId: String,
        pinned: Bool,
        now: TimeInterval = 0
    ) -> GaryxPinnedOrderUpdate {
        guard let threadId = Self.normalizedId(rawThreadId),
              membershipIntents[threadId] == nil,
              presentedOrder.contains(threadId) != pinned else {
            return GaryxPinnedOrderUpdate()
        }

        let rollbackOrder = membershipIntents.values.first(where: { intent in
            if case .live = intent.phase { return true }
            return false
        })?.rollbackOrder ?? desiredOrder
        nextToken &+= 1
        let stamp = requestStamp()
        let request = GaryxPinnedOrderMembershipRequest(
            token: nextToken,
            stamp: stamp,
            threadId: threadId,
            pinned: pinned
        )
        membershipIntents[threadId] = MembershipIntent(
            token: request.token,
            targetPinned: pinned,
            originallyPinned: desiredOrder.contains(threadId),
            rollbackOrder: rollbackOrder,
            phase: .live
        )

        var next = desiredOrder.filter { $0 != threadId }
        if pinned { next.insert(threadId, at: 0) }
        resolvedOrder = next
        desiredOrder = next
        epoch &+= 1

        var effects: [GaryxPinnedOrderEffect] = [.noteLocalMutation]
        if outbox != nil {
            outbox = makeOutbox()
            effects.append(.persist(outbox, gatewayIdentity: gatewayIdentity))
            wakeRequested = true
        }
        appendPublicationIfChanged(effects: &effects)
        drain(now: now, effects: &effects)
        return GaryxPinnedOrderUpdate(membershipRequest: request, effects: effects)
    }

    mutating func receivePage(
        _ page: GaryxPinnedOrderPage,
        stamp: GaryxPinnedOrderRequestStamp,
        now: TimeInterval = 0
    ) -> GaryxPinnedOrderUpdate {
        guard stamp.gatewayIdentity == gatewayIdentity else { return .wrongIdentity }
        var effects: [GaryxPinnedOrderEffect] = []
        let outcome = acceptPage(page, stamp: stamp, effects: &effects)
        if outcome != .discardedBelowFloor, outbox != nil {
            wakeRequested = true
        }
        drain(now: now, effects: &effects)
        return GaryxPinnedOrderUpdate(acceptance: outcome, effects: effects)
    }

    mutating func completeMembership(
        _ request: GaryxPinnedOrderMembershipRequest,
        page: GaryxPinnedOrderPage,
        now: TimeInterval = 0
    ) -> GaryxPinnedOrderUpdate {
        guard request.stamp.gatewayIdentity == gatewayIdentity,
              var intent = membershipIntents[request.threadId],
              intent.token == request.token,
              intent.phase == .live else {
            return .wrongIdentity
        }

        // Pipeline step 2: resolve transport only. Dispatch remains deferred
        // until after this response has raised (or failed to raise) the floor.
        if intent.targetPinned {
            intent.phase = .retiredPin(completionRevision: page.revision)
            membershipIntents[request.threadId] = intent
        } else {
            membershipIntents[request.threadId] = nil
        }
        epoch &+= 1
        if outbox != nil { wakeRequested = true }

        var effects: [GaryxPinnedOrderEffect] = []
        let outcome = acceptPage(page, stamp: request.stamp, effects: &effects)
        let retiredIntentRemoved = cleanupRetiredPinIntents()
        if outcome == .discardedBelowFloor,
           (!intent.targetPinned || retiredIntentRemoved),
           let latestAcceptedRawOrder {
            let merged = Self.mergeLocalOrder(
                desiredOrder,
                membershipOrder: membershipOrder(rawOrder: latestAcceptedRawOrder)
            )
            desiredOrder = merged
            resolvedOrder = merged
            appendPublicationIfChanged(effects: &effects)
        }
        if outbox != nil {
            outbox = makeOutbox()
            effects.append(.persist(outbox, gatewayIdentity: gatewayIdentity))
            wakeRequested = true
        }
        drain(now: now, effects: &effects)
        return GaryxPinnedOrderUpdate(acceptance: outcome, effects: effects)
    }

    mutating func failMembership(
        _ request: GaryxPinnedOrderMembershipRequest,
        now: TimeInterval = 0
    ) -> GaryxPinnedOrderUpdate {
        guard request.stamp.gatewayIdentity == gatewayIdentity,
              let intent = membershipIntents[request.threadId],
              intent.token == request.token,
              intent.phase == .live else {
            return .wrongIdentity
        }
        membershipIntents[request.threadId] = nil

        if intent.originallyPinned {
            desiredOrder = Self.restoring(
                threadId: request.threadId,
                baseline: intent.rollbackOrder,
                into: desiredOrder
            )
        } else {
            desiredOrder.removeAll { $0 == request.threadId }
        }
        resolvedOrder = desiredOrder
        epoch &+= 1

        var effects: [GaryxPinnedOrderEffect] = [.noteLocalMutation]
        if outbox != nil {
            outbox = makeOutbox()
            effects.append(.persist(outbox, gatewayIdentity: gatewayIdentity))
            wakeRequested = true
        }
        appendPublicationIfChanged(effects: &effects)
        drain(now: now, effects: &effects)
        return GaryxPinnedOrderUpdate(effects: effects)
    }

    mutating func completeReorder(
        _ request: GaryxPinnedOrderReorderRequest,
        page: GaryxPinnedOrderPage,
        now: TimeInterval = 0
    ) -> GaryxPinnedOrderUpdate {
        guard request.stamp.gatewayIdentity == gatewayIdentity else { return .wrongIdentity }
        guard activeReorderFlight?.token == request.token else {
            return GaryxPinnedOrderUpdate()
        }

        // Pipeline step 2 only closes this transport token. A page accepted in
        // step 3 may settle the outbox before the single post-response drain.
        activeReorderFlight = nil
        if outbox != nil { wakeRequested = true }
        var effects: [GaryxPinnedOrderEffect] = []
        let outcome = acceptPage(page, stamp: request.stamp, effects: &effects)
        if outbox != nil { wakeRequested = true }
        drain(now: now, effects: &effects)
        return GaryxPinnedOrderUpdate(acceptance: outcome, effects: effects)
    }

    mutating func failReorder(
        _ request: GaryxPinnedOrderReorderRequest,
        failure: GaryxPinnedOrderReorderFailure,
        now: TimeInterval = 0
    ) -> GaryxPinnedOrderUpdate {
        guard request.stamp.gatewayIdentity == gatewayIdentity else { return .wrongIdentity }
        guard activeReorderFlight?.token == request.token else {
            return GaryxPinnedOrderUpdate()
        }
        activeReorderFlight = nil
        guard outbox != nil else {
            pendingSync = .settled
            return GaryxPinnedOrderUpdate()
        }

        switch failure {
        case .retryable(let delay):
            retryAttempt += 1
            retryNotBefore = now + max(0, delay)
            wakeRequested = false
            pendingSync = .retryScheduled(
                attempt: retryAttempt,
                notBefore: retryNotBefore ?? now
            )
        case .permanent(let statusCode):
            permanentPauseStatus = statusCode
            wakeRequested = false
            pendingSync = .pausedPermanent(statusCode: statusCode)
        case .cancelled:
            wakeRequested = true
            var effects: [GaryxPinnedOrderEffect] = []
            drain(now: now, effects: &effects)
            return GaryxPinnedOrderUpdate(effects: effects)
        }
        return GaryxPinnedOrderUpdate()
    }

    mutating func retryTick(now: TimeInterval) -> GaryxPinnedOrderUpdate {
        guard outbox != nil, permanentPauseStatus == nil else {
            return GaryxPinnedOrderUpdate()
        }
        guard retryNotBefore.map({ now >= $0 }) ?? true else {
            return GaryxPinnedOrderUpdate()
        }
        retryNotBefore = nil
        wakeRequested = true
        var effects: [GaryxPinnedOrderEffect] = []
        drain(now: now, effects: &effects)
        return GaryxPinnedOrderUpdate(effects: effects)
    }

    mutating func resumePausedSync(now: TimeInterval = 0) -> GaryxPinnedOrderUpdate {
        guard outbox != nil else { return GaryxPinnedOrderUpdate() }
        permanentPauseStatus = nil
        retryNotBefore = nil
        retryAttempt = 0
        wakeRequested = true
        var effects: [GaryxPinnedOrderEffect] = []
        drain(now: now, effects: &effects)
        return GaryxPinnedOrderUpdate(effects: effects)
    }

    mutating func switchGateway(
        to newIdentity: String,
        restoredOutbox: GaryxPinnedOrderOutbox? = nil
    ) -> GaryxPinnedOrderUpdate {
        let oldIdentity = gatewayIdentity
        self = GaryxPinnedOrderState(
            gatewayIdentity: newIdentity,
            restoredOutbox: restoredOutbox
        )
        return GaryxPinnedOrderUpdate(
            effects: [.persist(nil, gatewayIdentity: oldIdentity)]
        )
    }

    private mutating func acceptPage(
        _ page: GaryxPinnedOrderPage,
        stamp: GaryxPinnedOrderRequestStamp,
        effects: inout [GaryxPinnedOrderEffect]
    ) -> GaryxPinnedOrderAcceptanceOutcome {
        guard page.revision >= highestObservedRevision else {
            return .discardedBelowFloor
        }

        highestObservedRevision = page.revision
        latestAcceptedRawOrder = page.threadIds
        _ = cleanupRetiredPinIntents()

        if outbox != nil,
           desiredOrder.isEmpty,
           liveMembershipIntentCount == 0 {
            // A projected-empty outbox may survive a process death while its
            // membership requests do not. With no live intents after restore,
            // clear it before fetched membership can revive the order debt.
            settleOutbox(effects: &effects)
        } else if let outbox, outbox.desiredOrder == page.threadIds {
            resolvedOrder = outbox.desiredOrder
            desiredOrder = outbox.desiredOrder
            settleOutbox(effects: &effects)
        }

        let needsMerge = stamp.epoch < epoch
            || outbox != nil
            || !membershipIntents.isEmpty
        let outcome: GaryxPinnedOrderAcceptanceOutcome
        if needsMerge {
            let membershipOrder = membershipOrder(rawOrder: page.threadIds)
            let merged = Self.mergeLocalOrder(
                desiredOrder,
                membershipOrder: membershipOrder
            )
            resolvedOrder = merged
            desiredOrder = merged
            if outbox != nil {
                outbox = makeOutbox()
                effects.append(.persist(outbox, gatewayIdentity: gatewayIdentity))
            }
            outcome = .merged
        } else {
            resolvedOrder = page.threadIds
            desiredOrder = page.threadIds
            outcome = .authoritative
        }

        if var session = dragSession {
            session.acceptedBuffer = resolvedOrder
            dragSession = session
        } else {
            appendPublicationIfChanged(effects: &effects)
        }
        return outcome
    }

    private mutating func drain(
        now: TimeInterval,
        effects: inout [GaryxPinnedOrderEffect]
    ) {
        guard outbox != nil else {
            wakeRequested = false
            pendingSync = .settled
            return
        }
        guard wakeRequested else {
            refreshPendingState(now: now)
            return
        }
        if activeReorderFlight != nil {
            pendingSync = .coalescedBehindFlight
            return
        }
        if liveMembershipIntentCount > 0 {
            if desiredOrder.isEmpty, latestAcceptedRawOrder?.isEmpty == true {
                clearOutbox(effects: &effects)
                return
            }
            pendingSync = .waitingForMembership
            wakeRequested = false
            return
        }
        if desiredOrder.isEmpty {
            // No live membership intent remains, so an empty desired order
            // cannot be sent and is safe to clear even if the last raw page
            // predates the completed unpins.
            clearOutbox(effects: &effects)
            return
        }
        if latestAcceptedRawOrder == desiredOrder {
            settleOutbox(effects: &effects)
            return
        }
        if let permanentPauseStatus {
            pendingSync = .pausedPermanent(statusCode: permanentPauseStatus)
            wakeRequested = false
            return
        }
        if let retryNotBefore, now < retryNotBefore {
            pendingSync = .retryScheduled(attempt: retryAttempt, notBefore: retryNotBefore)
            wakeRequested = false
            return
        }

        nextToken &+= 1
        let request = GaryxPinnedOrderReorderRequest(
            token: nextToken,
            stamp: requestStamp(),
            threadIds: desiredOrder,
            expectedRevision: highestObservedRevision
        )
        activeReorderFlight = request
        wakeRequested = false
        pendingSync = .inFlight
        effects.append(.sendReorder(request))
    }

    private mutating func refreshPendingState(now: TimeInterval) {
        if let permanentPauseStatus {
            pendingSync = .pausedPermanent(statusCode: permanentPauseStatus)
        } else if let retryNotBefore, now < retryNotBefore {
            pendingSync = .retryScheduled(attempt: retryAttempt, notBefore: retryNotBefore)
        } else if activeReorderFlight != nil {
            pendingSync = .inFlight
        } else if liveMembershipIntentCount > 0 {
            pendingSync = .waitingForMembership
        } else {
            pendingSync = .ready
        }
    }

    private mutating func settleOutbox(effects: inout [GaryxPinnedOrderEffect]) {
        guard outbox != nil else { return }
        outbox = nil
        retryAttempt = 0
        retryNotBefore = nil
        permanentPauseStatus = nil
        wakeRequested = false
        epoch &+= 1
        pendingSync = .settled
        effects.append(.persist(nil, gatewayIdentity: gatewayIdentity))
    }

    private mutating func clearOutbox(effects: inout [GaryxPinnedOrderEffect]) {
        settleOutbox(effects: &effects)
    }

    private func makeOutbox() -> GaryxPinnedOrderOutbox {
        GaryxPinnedOrderOutbox(
            gatewayIdentity: gatewayIdentity,
            desiredOrder: desiredOrder,
            lastKnownRevision: highestObservedRevision
        )
    }

    private mutating func appendPublicationIfChanged(
        effects: inout [GaryxPinnedOrderEffect]
    ) {
        let order = presentedOrder
        guard publishedOrder != order else { return }
        publishedOrder = order
        effects.append(.publish(order))
    }

    @discardableResult
    private mutating func cleanupRetiredPinIntents() -> Bool {
        let before = membershipIntents.count
        membershipIntents = membershipIntents.filter { _, intent in
            guard case .retiredPin(let completionRevision) = intent.phase else {
                return true
            }
            return highestObservedRevision < completionRevision
        }
        return membershipIntents.count != before
    }

    private func membershipOrder(rawOrder: [String]) -> [String] {
        var membership = rawOrder
        for (threadId, intent) in membershipIntents.sorted(by: { $0.value.token < $1.value.token }) {
            switch intent.phase {
            case .live where intent.targetPinned:
                if !membership.contains(threadId) { membership.append(threadId) }
            case .live:
                membership.removeAll { $0 == threadId }
            case .retiredPin:
                if !membership.contains(threadId) { membership.append(threadId) }
            }
        }
        return Self.normalized(membership)
    }

    static func normalized(_ values: [String]) -> [String] {
        var seen = Set<String>()
        return values.compactMap { value in
            guard let id = normalizedId(value), seen.insert(id).inserted else { return nil }
            return id
        }
    }

    private static func normalizedId(_ raw: String) -> String? {
        let id = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        return id.isEmpty ? nil : id
    }

    private static func mergeLocalOrder(
        _ localOrder: [String],
        membershipOrder: [String]
    ) -> [String] {
        let membership = Set(membershipOrder)
        let local = normalized(localOrder).filter(membership.contains)
        let localSet = Set(local)
        let newAtHead = membershipOrder.filter { !localSet.contains($0) }
        return normalized(newAtHead + local)
    }

    private static func overlay(
        order: [String],
        onMembershipOrder membershipOrder: [String]
    ) -> [String] {
        mergeLocalOrder(order, membershipOrder: membershipOrder)
    }

    private static func restoring(
        threadId: String,
        baseline: [String],
        into current: [String]
    ) -> [String] {
        var result = normalized(current).filter { $0 != threadId }
        let baseline = normalized(baseline)
        guard let originalIndex = baseline.firstIndex(of: threadId) else {
            result.insert(threadId, at: 0)
            return result
        }
        for predecessor in baseline[..<originalIndex].reversed() {
            if let index = result.firstIndex(of: predecessor) {
                result.insert(threadId, at: index + 1)
                return result
            }
        }
        for successor in baseline[baseline.index(after: originalIndex)...] {
            if let index = result.firstIndex(of: successor) {
                result.insert(threadId, at: index)
                return result
            }
        }
        result.insert(threadId, at: min(originalIndex, result.count))
        return result
    }
}
