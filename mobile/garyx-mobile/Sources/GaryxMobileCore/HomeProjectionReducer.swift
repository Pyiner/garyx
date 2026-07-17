import Foundation

enum HomeProjectionRunStateSource: Int, CaseIterable, Sendable {
    case runTracker = 0
    case committedRunState = 1
    case recentThreadSummary = 2
}

enum HomeProjectionRunStateStatus: Equatable, Sendable {
    case running
    case idle
    case unknown

    init(isRunning: Bool) {
        self = isRunning ? .running : .idle
    }
}

enum HomeProjectionEvent: Sendable {
    case recentThreadsIngested(
        threads: [GaryxThreadSummary],
        recentThreadIds: [String],
        agents: [GaryxAgentSummary],
        automations: [GaryxAutomationSummary],
        selectedRecentFilter: GaryxRecentThreadFilter,
        recentFeedPresentation: GaryxRecentThreadFeedPresentation,
        recentRunStateEpoch: Int
    )
    case pinsChanged(pinnedThreadIds: [String])
    case favoritesChanged(favoritedThreadIds: [String])
    case runStateDelta(
        source: HomeProjectionRunStateSource,
        threadId: String,
        status: HomeProjectionRunStateStatus,
        basedOnSeq: Int
    )
    case selectedThreadChanged(threadId: String?)
    case loadingChanged(isLoading: Bool)
    case homeVisibilityChanged(isVisible: Bool)
}

struct HomeSnapshot: Equatable, Sendable {
    var appliedSeq: Int
    var sections: GaryxHomeThreadSections
    var isLoadingThreads: Bool
    var isHomeVisible: Bool
    var selectedRecentFilter: GaryxRecentThreadFilter
    var recentFeedPresentation: GaryxRecentThreadFeedPresentation

    init(
        appliedSeq: Int = 0,
        sections: GaryxHomeThreadSections = GaryxHomeThreadSections(),
        isLoadingThreads: Bool = false,
        isHomeVisible: Bool = false,
        selectedRecentFilter: GaryxRecentThreadFilter = .all,
        recentFeedPresentation: GaryxRecentThreadFeedPresentation = .init(isPrimed: true)
    ) {
        self.appliedSeq = appliedSeq
        self.sections = sections
        self.isLoadingThreads = isLoadingThreads
        self.isHomeVisible = isHomeVisible
        self.selectedRecentFilter = selectedRecentFilter
        self.recentFeedPresentation = recentFeedPresentation
    }
}

struct HomeProjectionState: Equatable, Sendable {
    var threads: [GaryxThreadSummary] = []
    var agents: [GaryxAgentSummary] = []
    var automations: [GaryxAutomationSummary] = []
    var pinnedThreadIds: [String] = []
    var favoritedThreadIds: [String] = []
    var recentThreadIds: [String] = []
    var selectedThreadId: String?
    var isLoadingThreads = false
    var isHomeVisible = false
    var selectedRecentFilter: GaryxRecentThreadFilter = .all
    var recentFeedPresentation = GaryxRecentThreadFeedPresentation(isPrimed: true)

    fileprivate(set) var appliedSeq = 0
    fileprivate(set) var baseSections = GaryxHomeThreadSections()
    fileprivate(set) var snapshot = HomeSnapshot()
    fileprivate(set) var baseSectionBuildCount = 0
    fileprivate(set) var displayRebuildEventCount = 0
    fileprivate(set) var runStatePatchCount = 0
    fileprivate(set) var sectionIdentityEvaluationCount = 0
    fileprivate(set) var rowDifferenceEvaluationCount = 0

    fileprivate var lastBuiltSignature: HomeProjectionDisplaySignature?
    fileprivate var rowLocationsById: [String: HomeProjectionRowLocation] = [:]
    fileprivate var lastRowIds: [String] = []
    fileprivate var runStateSlotsByThread: [String: [HomeProjectionRunStateSource: HomeProjectionRunStateSlot]] = [:]

    init() {}

    var resolvedRunningThreadIds: Set<String> {
        var ids = Set<String>()
        for thread in threads {
            let threadId = Self.normalizedId(thread.id)
            guard !threadId.isEmpty, resolvedRunningState(for: threadId) else { continue }
            ids.insert(threadId)
        }
        return ids
    }

    /// Test-oracle support: builds the legacy fallback input for reducer equivalence checks.
    func legacyCheckpointInput() -> GaryxHomeThreadListInput {
        GaryxHomeThreadListInput(
            sectionsInput: sectionsInput,
            runningThreadIds: resolvedRunningThreadIds,
            isLoadingThreads: isLoadingThreads,
            isHomeVisible: isHomeVisible,
            selectedRecentFilter: selectedRecentFilter,
            recentFeedPresentation: recentFeedPresentation
        )
    }

    fileprivate var sectionsInput: GaryxHomeThreadSectionsInput {
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

    fileprivate func resolvedRunningState(for rawThreadId: String) -> Bool {
        let threadId = Self.normalizedId(rawThreadId)
        guard let slots = runStateSlotsByThread[threadId] else { return false }
        for source in HomeProjectionRunStateSource.allCases.sorted(by: { $0.rawValue < $1.rawValue }) {
            guard let slot = slots[source] else { continue }
            return slot.status == .running
        }
        return false
    }

    fileprivate static func normalizedId(_ id: String) -> String {
        id.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

enum HomeProjectionReducer {
    typealias Result = (
        state: HomeProjectionState,
        snapshot: HomeSnapshot,
        difference: CollectionDifference<String>?
    )

    static func reduce(_ state: HomeProjectionState, _ event: HomeProjectionEvent) -> Result {
        var next = state
        next.appliedSeq += 1
        var evaluatesRowDifference = false

        switch event {
        case let .recentThreadsIngested(
            threads,
            recentThreadIds,
            agents,
            automations,
            selectedRecentFilter,
            recentFeedPresentation,
            recentRunStateEpoch
        ):
            next.threads = threads
            next.recentThreadIds = normalizedThreadIds(recentThreadIds)
            next.agents = agents
            next.automations = automations
            next.selectedRecentFilter = selectedRecentFilter
            next.recentFeedPresentation = recentFeedPresentation
            applyRecentRunStateSlots(to: &next, threads: threads, epoch: recentRunStateEpoch)
            rebuildBaseSectionsIfNeeded(&next)
            rebuildSnapshotFromBase(&next)
            evaluatesRowDifference = true

        case let .pinsChanged(pinnedThreadIds):
            next.pinnedThreadIds = normalizedThreadIds(pinnedThreadIds)
            rebuildBaseSectionsIfNeeded(&next)
            rebuildSnapshotFromBase(&next)
            evaluatesRowDifference = true

        case let .favoritesChanged(favoritedThreadIds):
            next.favoritedThreadIds = normalizedThreadIds(favoritedThreadIds)
            rebuildBaseSectionsIfNeeded(&next)
            rebuildSnapshotFromBase(&next)
            evaluatesRowDifference = true

        case let .runStateDelta(source, threadId, status, basedOnSeq):
            let accepted = applyRunStateDelta(
                to: &next,
                source: source,
                threadId: threadId,
                status: status,
                basedOnSeq: basedOnSeq
            )
            if accepted {
                patchRunningRowIfVisible(&next, threadId: threadId)
            }
            next.snapshot.appliedSeq = next.appliedSeq

        case let .selectedThreadChanged(threadId):
            next.selectedThreadId = normalizedOptionalId(threadId)
            rebuildBaseSectionsIfNeeded(&next)
            rebuildSnapshotFromBase(&next)
            evaluatesRowDifference = true

        case let .loadingChanged(isLoading):
            next.isLoadingThreads = isLoading
            next.snapshot = HomeSnapshot(
                appliedSeq: next.appliedSeq,
                sections: next.snapshot.sections,
                isLoadingThreads: next.isLoadingThreads,
                isHomeVisible: next.isHomeVisible,
                selectedRecentFilter: next.selectedRecentFilter,
                recentFeedPresentation: next.recentFeedPresentation
            )

        case let .homeVisibilityChanged(isVisible):
            next.isHomeVisible = isVisible
            next.snapshot = HomeSnapshot(
                appliedSeq: next.appliedSeq,
                sections: next.snapshot.sections,
                isLoadingThreads: next.isLoadingThreads,
                isHomeVisible: next.isHomeVisible,
                selectedRecentFilter: next.selectedRecentFilter,
                recentFeedPresentation: next.recentFeedPresentation
            )
        }

        let difference: CollectionDifference<String>?
        if evaluatesRowDifference {
            next.rowDifferenceEvaluationCount += 1
            let rowIds = next.snapshot.sections.allRows.map(\.id)
            difference = next.lastRowIds == rowIds
                ? nil
                : rowIds.difference(from: next.lastRowIds).inferringMoves()
            next.lastRowIds = rowIds
        } else {
            difference = nil
        }
        return (next, next.snapshot, difference)
    }

    private static func rebuildBaseSectionsIfNeeded(_ state: inout HomeProjectionState) {
        let signature = HomeProjectionDisplaySignature(input: state.sectionsInput)
        guard state.lastBuiltSignature != signature else { return }

        state.displayRebuildEventCount += 1
        state.baseSections = GaryxHomeThreadSectionsBuilder.build(state.sectionsInput)
        state.baseSectionBuildCount += 1
        state.lastBuiltSignature = signature
        state.rowLocationsById = rowLocations(for: state.baseSections)
    }

    private static func rebuildSnapshotFromBase(_ state: inout HomeProjectionState) {
        state.snapshot = HomeSnapshot(
            appliedSeq: state.appliedSeq,
            sections: sections(
                state.baseSections,
                runningThreadIds: state.resolvedRunningThreadIds
            ),
            isLoadingThreads: state.isLoadingThreads,
            isHomeVisible: state.isHomeVisible,
            selectedRecentFilter: state.selectedRecentFilter,
            recentFeedPresentation: state.recentFeedPresentation
        )
    }

    @discardableResult
    private static func applyRunStateDelta(
        to state: inout HomeProjectionState,
        source: HomeProjectionRunStateSource,
        threadId rawThreadId: String,
        status: HomeProjectionRunStateStatus,
        basedOnSeq: Int
    ) -> Bool {
        let threadId = HomeProjectionState.normalizedId(rawThreadId)
        guard !threadId.isEmpty else { return false }

        var slots = state.runStateSlotsByThread[threadId] ?? [:]
        if let current = slots[source], basedOnSeq < current.basedOnSeq {
            return false
        }

        switch status {
        case .running, .idle:
            slots[source] = HomeProjectionRunStateSlot(status: status, basedOnSeq: basedOnSeq)
        case .unknown:
            slots[source] = nil
        }
        state.runStateSlotsByThread[threadId] = slots.isEmpty ? nil : slots
        return true
    }

    private static func applyRecentRunStateSlots(
        to state: inout HomeProjectionState,
        threads: [GaryxThreadSummary],
        epoch: Int
    ) {
        for thread in threads {
            let status: HomeProjectionRunStateStatus = isThreadSummaryRunning(thread) ? .running : .idle
            _ = applyRunStateDelta(
                to: &state,
                source: .recentThreadSummary,
                threadId: thread.id,
                status: status,
                basedOnSeq: epoch
            )
        }
    }

    private static func patchRunningRowIfVisible(
        _ state: inout HomeProjectionState,
        threadId rawThreadId: String
    ) {
        let threadId = HomeProjectionState.normalizedId(rawThreadId)
        guard let location = state.rowLocationsById[threadId] else { return }
        let isRunning = state.resolvedRunningState(for: threadId)
        switch location.section {
        case .pinned:
            guard state.snapshot.sections.pinned.indices.contains(location.index) else { return }
            state.snapshot.sections.pinned[location.index] = row(
                state.snapshot.sections.pinned[location.index],
                isRunning: isRunning
            )
        case .recent:
            guard state.snapshot.sections.recent.indices.contains(location.index) else { return }
            state.snapshot.sections.recent[location.index] = row(
                state.snapshot.sections.recent[location.index],
                isRunning: isRunning
            )
        }
        state.snapshot.appliedSeq = state.appliedSeq
        state.runStatePatchCount += 1
    }

    private static func sections(
        _ sections: GaryxHomeThreadSections,
        runningThreadIds: Set<String>
    ) -> GaryxHomeThreadSections {
        GaryxHomeThreadSections(
            pinned: sections.pinned.map { row($0, runningThreadIds: runningThreadIds) },
            recent: sections.recent.map { row($0, runningThreadIds: runningThreadIds) }
        )
    }

    private static func row(
        _ row: GaryxHomeThreadRow,
        runningThreadIds: Set<String>
    ) -> GaryxHomeThreadRow {
        let id = HomeProjectionState.normalizedId(row.id)
        return self.row(row, isRunning: !id.isEmpty && runningThreadIds.contains(id))
    }

    private static func row(_ row: GaryxHomeThreadRow, isRunning: Bool) -> GaryxHomeThreadRow {
        var capabilities = row.capabilities
        capabilities.canArchive = row.canArchive && !isRunning
        capabilities.archiveStrategy = capabilities.canArchive ? .thread : .none
        guard row.presentation.isRunning != isRunning
                || row.capabilities != capabilities else { return row }
        return GaryxHomeThreadRow(
            id: row.id,
            thread: row.thread,
            presentation: row.presentation.withRunningState(isRunning),
            avatar: row.avatar,
            timestampValue: row.timestampValue,
            canArchive: row.canArchive,
            capabilities: capabilities,
            showsDivider: row.showsDivider
        )
    }

    private static func rowLocations(for sections: GaryxHomeThreadSections) -> [String: HomeProjectionRowLocation] {
        var locations: [String: HomeProjectionRowLocation] = [:]
        for (index, row) in sections.pinned.enumerated() {
            let id = HomeProjectionState.normalizedId(row.id)
            guard !id.isEmpty, locations[id] == nil else { continue }
            locations[id] = HomeProjectionRowLocation(section: .pinned, index: index)
        }
        for (index, row) in sections.recent.enumerated() {
            let id = HomeProjectionState.normalizedId(row.id)
            guard !id.isEmpty, locations[id] == nil else { continue }
            locations[id] = HomeProjectionRowLocation(section: .recent, index: index)
        }
        return locations
    }

    private static func normalizedThreadIds(_ ids: [String]) -> [String] {
        var seen = Set<String>()
        var normalized: [String] = []
        for rawId in ids {
            let id = HomeProjectionState.normalizedId(rawId)
            guard !id.isEmpty, seen.insert(id).inserted else { continue }
            normalized.append(id)
        }
        return normalized
    }

    private static func normalizedOptionalId(_ id: String?) -> String? {
        let normalized = HomeProjectionState.normalizedId(id ?? "")
        return normalized.isEmpty ? nil : normalized
    }

    private static func isThreadSummaryRunning(_ thread: GaryxThreadSummary) -> Bool {
        thread.runState?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() == "running"
    }
}

private struct HomeProjectionRunStateSlot: Equatable, Sendable {
    var status: HomeProjectionRunStateStatus
    var basedOnSeq: Int
}

private enum HomeProjectionRowSection: Equatable, Sendable {
    case pinned
    case recent
}

private struct HomeProjectionRowLocation: Equatable, Sendable {
    var section: HomeProjectionRowSection
    var index: Int
}

private struct HomeProjectionDisplaySignature: Equatable, Sendable {
    var threads: [GaryxThreadSummary]
    var agents: [GaryxAgentSummary]
    var automationThreadIds: Set<String>
    var pinnedThreadIds: [String]
    var favoritedThreadIds: [String]
    var recentThreadIds: [String]
    var selectedThreadId: String?

    init(input: GaryxHomeThreadSectionsInput) {
        threads = input.threads.map(Self.displayThread)
        agents = input.agents
        automationThreadIds = GaryxHomeThreadSectionsBuilder.automationThreadIds(input.automations)
        pinnedThreadIds = GaryxHomeThreadSectionsBuilder.normalizedPinnedThreadIds(input.pinnedThreadIds)
        favoritedThreadIds = GaryxHomeThreadSectionsBuilder.normalizedPinnedThreadIds(
            input.favoritedThreadIds
        )
        recentThreadIds = input.recentThreadIds
        selectedThreadId = input.selectedThreadId?.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func displayThread(_ thread: GaryxThreadSummary) -> GaryxThreadSummary {
        var copy = thread
        copy.activeRunId = nil
        copy.runState = nil
        if var runtime = copy.threadRuntime {
            runtime.activeRun = nil
            copy.threadRuntime = runtime
        }
        return copy
    }
}
