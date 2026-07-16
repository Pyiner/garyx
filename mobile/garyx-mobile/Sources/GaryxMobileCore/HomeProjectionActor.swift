import Foundation

struct HomeProjectionCapture: Equatable, Sendable {
    var threads: [GaryxThreadSummary]
    var recentThreadIds: [String]
    var agents: [GaryxAgentSummary]
    var automations: [GaryxAutomationSummary]
    var pinnedThreadIds: [String]
    var favoritedThreadIds: [String]
    var selectedThreadId: String?
    var isLoadingThreads: Bool
    var isHomeVisible: Bool
    var selectedRecentFilter: GaryxRecentThreadFilter
    var recentFeedPresentation: GaryxRecentThreadFeedPresentation
    var runTrackerBusyThreadIds: Set<String>
    var committedRunStateBusyByThreadId: [String: Bool]

    init(
        threads: [GaryxThreadSummary],
        recentThreadIds: [String],
        agents: [GaryxAgentSummary],
        automations: [GaryxAutomationSummary],
        pinnedThreadIds: [String],
        favoritedThreadIds: [String] = [],
        selectedThreadId: String?,
        isLoadingThreads: Bool,
        isHomeVisible: Bool,
        selectedRecentFilter: GaryxRecentThreadFilter = .all,
        recentFeedPresentation: GaryxRecentThreadFeedPresentation = .init(isPrimed: true),
        runTrackerBusyThreadIds: Set<String> = [],
        committedRunStateBusyByThreadId: [String: Bool] = [:]
    ) {
        self.threads = threads
        self.recentThreadIds = recentThreadIds
        self.agents = agents
        self.automations = automations
        self.pinnedThreadIds = pinnedThreadIds
        self.favoritedThreadIds = favoritedThreadIds
        self.selectedThreadId = selectedThreadId
        self.isLoadingThreads = isLoadingThreads
        self.isHomeVisible = isHomeVisible
        self.selectedRecentFilter = selectedRecentFilter
        self.recentFeedPresentation = recentFeedPresentation
        self.runTrackerBusyThreadIds = Self.normalizedThreadIdSet(runTrackerBusyThreadIds)
        self.committedRunStateBusyByThreadId = Self.normalizedBusyMap(committedRunStateBusyByThreadId)
    }

    init(
        legacyInput input: GaryxHomeThreadListInput,
        runTrackerBusyThreadIds: Set<String> = [],
        committedRunStateBusyByThreadId: [String: Bool] = [:]
    ) {
        self.init(
            threads: input.sectionsInput.threads,
            recentThreadIds: input.sectionsInput.recentThreadIds,
            agents: input.sectionsInput.agents,
            automations: input.sectionsInput.automations,
            pinnedThreadIds: input.sectionsInput.pinnedThreadIds,
            favoritedThreadIds: input.sectionsInput.favoritedThreadIds,
            selectedThreadId: input.sectionsInput.selectedThreadId,
            isLoadingThreads: input.isLoadingThreads,
            isHomeVisible: input.isHomeVisible,
            selectedRecentFilter: input.selectedRecentFilter,
            recentFeedPresentation: input.recentFeedPresentation,
            runTrackerBusyThreadIds: runTrackerBusyThreadIds,
            committedRunStateBusyByThreadId: committedRunStateBusyByThreadId
        )
    }

    var sectionsInput: GaryxHomeThreadSectionsInput {
        GaryxHomeThreadSectionsInput(
            threads: threads,
            agents: agents,
            automations: automations,
            pinnedThreadIds: pinnedThreadIds,
            favoritedThreadIds: favoritedThreadIds,
            recentThreadIds: recentThreadIds,
            selectedThreadId: selectedThreadId
        )
    }

    fileprivate func events(comparedTo previous: HomeProjectionCapture?, epoch: Int) -> [HomeProjectionEvent] {
        var events: [HomeProjectionEvent] = []
        if previous?.displayPayload != displayPayload {
            events.append(.recentThreadsIngested(
                threads: threads,
                recentThreadIds: recentThreadIds,
                agents: agents,
                automations: automations,
                selectedRecentFilter: selectedRecentFilter,
                recentFeedPresentation: recentFeedPresentation,
                recentRunStateEpoch: epoch
            ))
        }
        if previous?.pinnedThreadIds != pinnedThreadIds {
            events.append(.pinsChanged(pinnedThreadIds: pinnedThreadIds))
        }
        if previous?.favoritedThreadIds != favoritedThreadIds {
            events.append(.favoritesChanged(favoritedThreadIds: favoritedThreadIds))
        }
        if previous?.selectedThreadId != selectedThreadId {
            events.append(.selectedThreadChanged(threadId: selectedThreadId))
        }
        if previous?.isLoadingThreads != isLoadingThreads {
            events.append(.loadingChanged(isLoading: isLoadingThreads))
        }
        if previous?.isHomeVisible != isHomeVisible {
            events.append(.homeVisibilityChanged(isVisible: isHomeVisible))
        }

        appendRunTrackerEvents(comparedTo: previous, epoch: epoch, into: &events)
        appendCommittedRunStateEvents(comparedTo: previous, epoch: epoch, into: &events)
        return events
    }

    private var displayPayload: HomeProjectionDisplayPayload {
        HomeProjectionDisplayPayload(
            threads: threads,
            recentThreadIds: recentThreadIds,
            agents: agents,
            automations: automations,
            selectedRecentFilter: selectedRecentFilter,
            recentFeedPresentation: recentFeedPresentation
        )
    }

    private func appendRunTrackerEvents(
        comparedTo previous: HomeProjectionCapture?,
        epoch: Int,
        into events: inout [HomeProjectionEvent]
    ) {
        let previousIds = previous?.runTrackerBusyThreadIds ?? []
        for threadId in runTrackerBusyThreadIds.subtracting(previousIds).sorted() {
            events.append(.runStateDelta(
                source: .runTracker,
                threadId: threadId,
                status: .running,
                basedOnSeq: epoch
            ))
        }
        for threadId in previousIds.subtracting(runTrackerBusyThreadIds).sorted() {
            events.append(.runStateDelta(
                source: .runTracker,
                threadId: threadId,
                status: .unknown,
                basedOnSeq: epoch
            ))
        }
    }

    private func appendCommittedRunStateEvents(
        comparedTo previous: HomeProjectionCapture?,
        epoch: Int,
        into events: inout [HomeProjectionEvent]
    ) {
        let previousMap = previous?.committedRunStateBusyByThreadId ?? [:]
        let allIds = Set(previousMap.keys).union(committedRunStateBusyByThreadId.keys).sorted()
        for threadId in allIds {
            switch (previousMap[threadId], committedRunStateBusyByThreadId[threadId]) {
            case let (previous?, current?) where previous != current:
                events.append(.runStateDelta(
                    source: .committedRunState,
                    threadId: threadId,
                    status: HomeProjectionRunStateStatus(isRunning: current),
                    basedOnSeq: epoch
                ))
            case (nil, let current?):
                events.append(.runStateDelta(
                    source: .committedRunState,
                    threadId: threadId,
                    status: HomeProjectionRunStateStatus(isRunning: current),
                    basedOnSeq: epoch
                ))
            case (_?, nil):
                events.append(.runStateDelta(
                    source: .committedRunState,
                    threadId: threadId,
                    status: .unknown,
                    basedOnSeq: epoch
                ))
            default:
                break
            }
        }
    }

    private static func normalizedThreadIdSet(_ ids: Set<String>) -> Set<String> {
        Set(ids.compactMap { rawId in
            let id = rawId.trimmingCharacters(in: .whitespacesAndNewlines)
            return id.isEmpty ? nil : id
        })
    }

    private static func normalizedBusyMap(_ map: [String: Bool]) -> [String: Bool] {
        var normalized: [String: Bool] = [:]
        for (rawId, isBusy) in map {
            let id = rawId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !id.isEmpty else { continue }
            normalized[id] = isBusy
        }
        return normalized
    }
}

struct HomeProjectionBoundaryResult: Equatable, Sendable {
    var transactionId: UInt64
    var appliedSeq: Int
    var snapshot: HomeSnapshot
    var difference: CollectionDifference<String>?
    var snapshotEmitCount: Int
}

actor HomeProjectionActor {
    private var state = HomeProjectionState()
    private var previousCapture: HomeProjectionCapture?
    private var boundaryEpoch = 0
    private(set) var snapshotEmitCount = 0

    func applyBoundary(
        capture: HomeProjectionCapture,
        transactionId: UInt64
    ) -> HomeProjectionBoundaryResult {
        boundaryEpoch += 1
        let events = capture.events(comparedTo: previousCapture, epoch: boundaryEpoch)
        var latestDifference: CollectionDifference<String>?
        for event in events {
            let result = HomeProjectionReducer.reduce(state, event)
            state = result.state
            latestDifference = result.difference ?? latestDifference
        }
        previousCapture = capture
        return finishBoundary(
            transactionId: transactionId,
            latestDifference: latestDifference
        )
    }

    func applyCommittedRunStateDelta(
        threadId: String,
        isRunning: Bool,
        transactionId: UInt64
    ) -> HomeProjectionBoundaryResult {
        boundaryEpoch += 1
        let result = HomeProjectionReducer.reduce(
            state,
            .runStateDelta(
                source: .committedRunState,
                threadId: threadId,
                status: HomeProjectionRunStateStatus(isRunning: isRunning),
                basedOnSeq: boundaryEpoch
            )
        )
        state = result.state
        return finishBoundary(
            transactionId: transactionId,
            latestDifference: result.difference
        )
    }

    private func finishBoundary(
        transactionId: UInt64,
        latestDifference: CollectionDifference<String>?
    ) -> HomeProjectionBoundaryResult {
        snapshotEmitCount += 1
        return HomeProjectionBoundaryResult(
            transactionId: transactionId,
            appliedSeq: state.snapshot.appliedSeq,
            snapshot: state.snapshot,
            difference: latestDifference,
            snapshotEmitCount: snapshotEmitCount
        )
    }
}

@MainActor
final class HomeProjectionGateway {
    typealias ResultHandler = @MainActor (HomeProjectionBoundaryResult) -> Void

    private enum BoundaryPayload: Sendable {
        case capture(HomeProjectionCapture)
        case committedRunStateDelta(threadId: String, isRunning: Bool)
    }

    private struct Boundary: Sendable {
        var transactionId: UInt64
        var payload: BoundaryPayload
    }

    private let actor: HomeProjectionActor
    private let isEnabled: Bool
    private var nextTransactionId: UInt64 = 0
    private var transactionDepth = 0
    private var activeTransactionId: UInt64?
    private var pendingTransactionBoundary: Boundary?
    private var queuedBoundary: Boundary?
    private var inFlightTask: Task<Void, Never>?
    private var resultHandler: ResultHandler?

    private(set) var latestResult: HomeProjectionBoundaryResult?
    private(set) var snapshotEmitCount = 0

    init(
        actor: HomeProjectionActor = HomeProjectionActor(),
        isEnabled: Bool = HomeProjectionGatewayConfiguration.isEnabled
    ) {
        self.actor = actor
        self.isEnabled = isEnabled
    }

    func setResultHandler(_ handler: ResultHandler?) {
        resultHandler = handler
    }

    @discardableResult
    func beginTransaction(label _: String? = nil) -> UInt64? {
        guard isEnabled else { return nil }
        if transactionDepth == 0 {
            activeTransactionId = allocateTransactionId()
            pendingTransactionBoundary = nil
        }
        transactionDepth += 1
        return activeTransactionId
    }

    func endTransaction(_ transactionId: UInt64? = nil) {
        guard isEnabled, transactionDepth > 0 else { return }
        if let transactionId, let activeTransactionId, transactionId != activeTransactionId {
            return
        }
        transactionDepth -= 1
        guard transactionDepth == 0 else { return }
        activeTransactionId = nil
        if let boundary = pendingTransactionBoundary {
            pendingTransactionBoundary = nil
            enqueue(boundary)
        }
    }

    func capture(_ capture: HomeProjectionCapture) {
        guard isEnabled else { return }
        if transactionDepth > 0, let transactionId = activeTransactionId {
            pendingTransactionBoundary = Boundary(
                transactionId: transactionId,
                payload: .capture(capture)
            )
            return
        }
        enqueue(Boundary(
            transactionId: allocateTransactionId(),
            payload: .capture(capture)
        ))
    }

    func captureCommittedRunStateDelta(threadId: String, isRunning: Bool) {
        guard isEnabled else { return }
        enqueue(Boundary(
            transactionId: allocateTransactionId(),
            payload: .committedRunStateDelta(threadId: threadId, isRunning: isRunning)
        ))
    }

    func waitForIdleForTesting() async {
        while inFlightTask != nil || queuedBoundary != nil {
            await inFlightTask?.value
            await Task.yield()
        }
    }

    private func allocateTransactionId() -> UInt64 {
        nextTransactionId &+= 1
        return nextTransactionId
    }

    private func enqueue(_ boundary: Boundary) {
        queuedBoundary = boundary
        startDrainIfNeeded()
    }

    private func startDrainIfNeeded() {
        guard inFlightTask == nil, let boundary = queuedBoundary else { return }
        queuedBoundary = nil
        inFlightTask = Task { [actor] in
            let result: HomeProjectionBoundaryResult
            switch boundary.payload {
            case let .capture(capture):
                result = await actor.applyBoundary(
                    capture: capture,
                    transactionId: boundary.transactionId
                )
            case let .committedRunStateDelta(threadId, isRunning):
                result = await actor.applyCommittedRunStateDelta(
                    threadId: threadId,
                    isRunning: isRunning,
                    transactionId: boundary.transactionId
                )
            }
            await MainActor.run { [weak self] in
                self?.finishDrain(result)
            }
        }
    }

    private func finishDrain(_ result: HomeProjectionBoundaryResult) {
        latestResult = result
        snapshotEmitCount = result.snapshotEmitCount
        resultHandler?(result)
        inFlightTask = nil
        startDrainIfNeeded()
    }
}

enum HomeProjectionGatewayConfiguration {
    static var isEnabled: Bool {
        HomeProjectionLiveSourceConfiguration.usesActorSnapshots
            || HomeProjectionShadowConfiguration.isEnabled
    }
}

enum HomeProjectionLiveSourceConfiguration {
    static var usesActorSnapshots: Bool {
        let rawValue = ProcessInfo.processInfo.environment["GARYX_MOBILE_HOME_PROJECTION_CUTOVER"]?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
        switch rawValue {
        case "0", "false", "no", "off":
            return false
        default:
            return true
        }
    }
}

enum HomeProjectionShadowConfiguration {
    static var isEnabled: Bool {
        let rawValue = ProcessInfo.processInfo.environment["GARYX_MOBILE_HOME_PROJECTION_SHADOW"]?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
        switch rawValue {
        case "0", "false", "no", "off":
            return false
        default:
            return true
        }
    }
}

private struct HomeProjectionDisplayPayload: Equatable, Sendable {
    var threads: [GaryxThreadSummary]
    var recentThreadIds: [String]
    var agents: [GaryxAgentSummary]
    var automations: [GaryxAutomationSummary]
    var selectedRecentFilter: GaryxRecentThreadFilter
    var recentFeedPresentation: GaryxRecentThreadFeedPresentation
}
