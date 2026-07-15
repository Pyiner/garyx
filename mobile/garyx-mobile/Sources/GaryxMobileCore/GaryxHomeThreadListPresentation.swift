import Combine
import Foundation

enum GaryxThreadSummaryRunStateResolver {
    static func resolvedRunState(
        apiRunState: String?,
        recentRunId _: String?,
        committedState _: GaryxTranscriptRunState?
    ) -> String? {
        apiRunState
    }

    /// Whether a thread summary's API run state marks the thread as running.
    static func isRunning(_ thread: GaryxThreadSummary) -> Bool {
        let runState = thread.runState?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return runState == "running"
    }
}

/// Agent identity projected from a thread summary for list rows and widgets.
struct GaryxWidgetAgentIdentity: Equatable, Sendable {
    var id: String?
    var name: String?
    var avatarDataUrl: String?
    var providerType: String?
    var builtIn: Bool
}

/// Single source of truth for resolving a thread's display identity: catalog
/// agent, thread fallback, then provider-only.
enum GaryxWidgetAgentIdentityProjector {
    static func identity(
        for thread: GaryxThreadSummary,
        agents: [GaryxAgentSummary]
    ) -> GaryxWidgetAgentIdentity {
        identity(
            for: thread,
            agent: { agentId in agents.first { $0.id == agentId } }
        )
    }

    static func identity(
        for thread: GaryxThreadSummary,
        agentsById: [String: GaryxAgentSummary]
    ) -> GaryxWidgetAgentIdentity {
        identity(
            for: thread,
            agent: { agentsById[$0] }
        )
    }

    private static func identity(
        for thread: GaryxThreadSummary,
        agent agentById: (String) -> GaryxAgentSummary?
    ) -> GaryxWidgetAgentIdentity {
        let agentId = thread.agentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !agentId.isEmpty {
            if let agent = agentById(agentId) {
                return GaryxWidgetAgentIdentity(
                    id: agent.id,
                    name: agent.displayName,
                    avatarDataUrl: agent.avatarDataUrl.isEmpty ? nil : agent.avatarDataUrl,
                    providerType: agent.providerType,
                    builtIn: agent.builtIn
                )
            }
            return GaryxWidgetAgentIdentity(
                id: agentId,
                name: nil,
                avatarDataUrl: nil,
                providerType: thread.providerType,
                builtIn: false
            )
        }

        return GaryxWidgetAgentIdentity(
            id: nil,
            name: nil,
            avatarDataUrl: nil,
            providerType: thread.providerType,
            builtIn: false
        )
    }
}

struct GaryxSidebarThreadRowAvatar: Equatable, Sendable {
    let agentId: String
    let avatarDataUrl: String
    let label: String
    let providerType: String
    let builtIn: Bool
}

struct GaryxSidebarThreadRowPresentation: Equatable, Sendable {
    let title: String
    let subtitle: String?
    let trailingTimestamp: String?
    let isSelected: Bool
    let isPinned: Bool
    let isRunning: Bool

    init(
        thread: GaryxThreadSummary,
        isSelected: Bool,
        isPinned: Bool,
        trailingTimestamp: String?,
        showsRunningState: Bool = true
    ) {
        self.title = thread.title.isEmpty ? "Untitled" : thread.title
        self.subtitle = Self.subtitle(for: thread)
        self.trailingTimestamp = trailingTimestamp
        self.isSelected = isSelected
        self.isPinned = isPinned
        self.isRunning = showsRunningState && Self.isRunning(thread)
    }

    init(
        title: String,
        subtitle: String?,
        trailingTimestamp: String?,
        isSelected: Bool,
        isPinned: Bool,
        isRunning: Bool
    ) {
        self.title = title
        self.subtitle = subtitle
        self.trailingTimestamp = trailingTimestamp
        self.isSelected = isSelected
        self.isPinned = isPinned
        self.isRunning = isRunning
    }

    func withTrailingTimestamp(_ trailingTimestamp: String?) -> GaryxSidebarThreadRowPresentation {
        GaryxSidebarThreadRowPresentation(
            title: title,
            subtitle: subtitle,
            trailingTimestamp: trailingTimestamp,
            isSelected: isSelected,
            isPinned: isPinned,
            isRunning: isRunning
        )
    }

    func withRunningState(_ isRunning: Bool) -> GaryxSidebarThreadRowPresentation {
        GaryxSidebarThreadRowPresentation(
            title: title,
            subtitle: subtitle,
            trailingTimestamp: trailingTimestamp,
            isSelected: isSelected,
            isPinned: isPinned,
            isRunning: isRunning
        )
    }

    func withPinnedState(_ isPinned: Bool) -> GaryxSidebarThreadRowPresentation {
        GaryxSidebarThreadRowPresentation(
            title: title,
            subtitle: subtitle,
            trailingTimestamp: trailingTimestamp,
            isSelected: isSelected,
            isPinned: isPinned,
            isRunning: isRunning
        )
    }

    private static func subtitle(for thread: GaryxThreadSummary) -> String? {
        let context: String? = {
            if let workspacePath = thread.workspacePath, !workspacePath.isEmpty {
                return workspacePath.garyxLastPathComponent
            }
            return thread.agentId
        }()
        let parts = [context, compactedPreview(thread.lastMessagePreview)].compactMap { part -> String? in
            let trimmed = part?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            return trimmed.isEmpty ? nil : trimmed
        }
        return parts.isEmpty ? nil : parts.joined(separator: " \u{00B7} ")
    }

    /// Last-message previews can span lines; collapse to one display line.
    private static func compactedPreview(_ raw: String) -> String? {
        let collapsed = raw
            .components(separatedBy: .whitespacesAndNewlines)
            .filter { !$0.isEmpty }
            .joined(separator: " ")
        return collapsed.isEmpty ? nil : collapsed
    }

    private static func isRunning(_ thread: GaryxThreadSummary) -> Bool {
        GaryxThreadSummaryRunStateResolver.isRunning(thread)
    }
}

struct GaryxHomeThreadSectionsInput: Equatable, Sendable {
    var threads: [GaryxThreadSummary]
    var agents: [GaryxAgentSummary]
    var automations: [GaryxAutomationSummary]
    var pinnedThreadIds: [String]
    var recentThreadIds: [String]
    var selectedThreadId: String?

    init(
        threads: [GaryxThreadSummary],
        agents: [GaryxAgentSummary],
        automations: [GaryxAutomationSummary],
        pinnedThreadIds: [String],
        recentThreadIds: [String],
        selectedThreadId: String?
    ) {
        self.threads = threads
        self.agents = agents
        self.automations = automations
        self.pinnedThreadIds = pinnedThreadIds
        self.recentThreadIds = recentThreadIds
        self.selectedThreadId = selectedThreadId
    }
}

struct GaryxHomeThreadSections: Equatable, Sendable {
    var pinned: [GaryxHomeThreadRow] = []
    var recent: [GaryxHomeThreadRow] = []

    var allRows: [GaryxHomeThreadRow] {
        pinned + recent
    }
}

struct GaryxHomeThreadListInput: Equatable, Sendable {
    var sectionsInput: GaryxHomeThreadSectionsInput
    var runningThreadIds: Set<String>
    var isLoadingThreads: Bool
    var isHomeVisible: Bool
    var selectedRecentFilter: GaryxRecentThreadFilter
    var recentFeedPresentation: GaryxRecentThreadFeedPresentation

    init(
        sectionsInput: GaryxHomeThreadSectionsInput,
        runningThreadIds: Set<String>,
        isLoadingThreads: Bool,
        isHomeVisible: Bool,
        selectedRecentFilter: GaryxRecentThreadFilter = .all,
        recentFeedPresentation: GaryxRecentThreadFeedPresentation = .init(isPrimed: true)
    ) {
        self.sectionsInput = sectionsInput
        self.runningThreadIds = Set(
            runningThreadIds.compactMap { threadId -> String? in
                let normalized = threadId.trimmingCharacters(in: .whitespacesAndNewlines)
                return normalized.isEmpty ? nil : normalized
            }
        )
        self.isLoadingThreads = isLoadingThreads
        self.isHomeVisible = isHomeVisible
        self.selectedRecentFilter = selectedRecentFilter
        self.recentFeedPresentation = recentFeedPresentation
    }

    static func == (lhs: GaryxHomeThreadListInput, rhs: GaryxHomeThreadListInput) -> Bool {
        IdentityKey(lhs) == IdentityKey(rhs)
    }

    private struct IdentityKey: Equatable {
        var sections: GaryxHomeThreadSectionsIdentityKey
        var runningThreadIds: Set<String>
        var isLoadingThreads: Bool
        var isHomeVisible: Bool
        var selectedRecentFilter: GaryxRecentThreadFilter
        var recentFeedPresentation: GaryxRecentThreadFeedPresentation

        init(_ input: GaryxHomeThreadListInput) {
            sections = GaryxHomeThreadSectionsIdentityKey(input.sectionsInput)
            runningThreadIds = input.runningThreadIds
            isLoadingThreads = input.isLoadingThreads
            isHomeVisible = input.isHomeVisible
            selectedRecentFilter = input.selectedRecentFilter
            recentFeedPresentation = input.recentFeedPresentation
        }
    }
}

struct GaryxHomeThreadListSnapshot: Equatable, Sendable {
    var sections = GaryxHomeThreadSections()
    var isLoadingThreads = false
    var isHomeVisible = false
    var selectedRecentFilter: GaryxRecentThreadFilter = .all
    var recentFeedPresentation = GaryxRecentThreadFeedPresentation(isPrimed: true)

    var recentPlaceholder: GaryxHomeRecentPlaceholder {
        guard sections.recent.isEmpty else { return .none }
        if !recentFeedPresentation.isPrimed {
            return recentFeedPresentation.headFailure
                ? .unavailable
                : .loadingSkeleton(rowCount: 6)
        }
        return isLoadingThreads ? .loadingSkeleton(rowCount: 6) : .empty
    }

    static let empty = GaryxHomeThreadListSnapshot()
}

enum GaryxHomeRecentPlaceholder: Equatable, Sendable {
    case none
    case loadingSkeleton(rowCount: Int)
    case empty
    case unavailable
}

enum GaryxHomeThreadListRegion: Equatable, Sendable {
    case pinned
    case recent
}

/// One stable identity space for the native List. A thread keeps the same item
/// id while moving between Pinned and Recent, allowing UIKit to animate a move
/// instead of rendering an unrelated deletion and insertion.
enum GaryxHomeThreadListItem: Identifiable, Equatable, Sendable {
    case pinnedHeader
    case thread(GaryxHomeThreadRow, region: GaryxHomeThreadListRegion)
    case pinnedSpacer
    case recentHeader
    case recentPlaceholder(GaryxHomeRecentPlaceholder)

    var id: String {
        switch self {
        case .pinnedHeader:
            "header:pinned"
        case let .thread(row, _):
            "thread:\(row.id)"
        case .pinnedSpacer:
            "spacer:pinned"
        case .recentHeader:
            "header:recent"
        case .recentPlaceholder:
            "placeholder:recent"
        }
    }
}

enum GaryxHomeThreadListLayout {
    static func primaryItems(for snapshot: GaryxHomeThreadListSnapshot) -> [GaryxHomeThreadListItem] {
        var items: [GaryxHomeThreadListItem] = []
        if !snapshot.sections.pinned.isEmpty {
            items.append(.pinnedHeader)
            items += snapshot.sections.pinned.map { .thread($0, region: .pinned) }
            items.append(.pinnedSpacer)
        }
        items.append(.recentHeader)
        switch snapshot.recentPlaceholder {
        case .none:
            items += snapshot.sections.recent.map { .thread($0, region: .recent) }
        case let placeholder:
            items.append(.recentPlaceholder(placeholder))
        }
        return items
    }
}

struct GaryxHomeThreadRow: Identifiable, Equatable, Sendable {
    let id: String
    let thread: GaryxThreadSummary
    let presentation: GaryxSidebarThreadRowPresentation
    let avatar: GaryxSidebarThreadRowAvatar
    let timestampValue: String?
    let canArchive: Bool
    let showsDivider: Bool
}

enum GaryxHomeThreadRowMotion: Equatable, Sendable {
    case stable
    case archiving
    case pinning
    case leavingFilteredList
}

/// Ephemeral presentation state layered over the canonical Home snapshot.
/// It keeps archive rows physically present until the remote commit, while pin
/// overrides prevent stale refreshes from bouncing a row between groups.
struct GaryxHomeThreadTransitionState: Equatable, Sendable {
    private enum ArchivePhase: Equatable, Sendable {
        case requesting
        case committed
    }

    private struct PinTransition: Equatable, Sendable {
        var targetPinned: Bool
        let originalPinned: Bool
        let recentIndex: Int?
        let originalPinnedIndex: Int
        let originalPinnedOrder: [String]
        let originalRecentOrder: [String]
        let sourceRow: GaryxHomeThreadRow?
        let sequence: UInt64
        var remoteResolved: Bool

        var restoresOriginalPlacement: Bool {
            remoteResolved && targetPinned == originalPinned
        }
    }

    private var archivePhases: [String: ArchivePhase] = [:]
    private var pinTransitions: [String: PinTransition] = [:]
    private var nextSequence: UInt64 = 0

    var isEmpty: Bool {
        archivePhases.isEmpty && pinTransitions.isEmpty
    }

    mutating func beginArchive(threadId: String) -> Bool {
        guard let threadId = Self.normalizedThreadId(threadId),
              archivePhases[threadId] == nil,
              pinTransitions[threadId] == nil else {
            return false
        }
        archivePhases[threadId] = .requesting
        return true
    }

    mutating func commitArchive(threadId: String) {
        guard let threadId = Self.normalizedThreadId(threadId),
              archivePhases[threadId] != nil else {
            return
        }
        archivePhases[threadId] = .committed
    }

    mutating func cancelArchive(threadId: String) {
        guard let threadId = Self.normalizedThreadId(threadId) else { return }
        archivePhases[threadId] = nil
    }

    mutating func beginPin(
        threadId: String,
        pinned: Bool,
        originalPinned: Bool,
        recentIndex: Int?,
        originalPinnedIndex: Int? = nil,
        originalPinnedOrder: [String] = [],
        originalRecentOrder: [String] = [],
        sourceRow: GaryxHomeThreadRow? = nil
    ) -> Bool {
        guard let threadId = Self.normalizedThreadId(threadId),
              pinned != originalPinned,
              archivePhases[threadId] == nil,
              pinTransitions[threadId] == nil else {
            return false
        }
        nextSequence &+= 1
        pinTransitions[threadId] = PinTransition(
            targetPinned: pinned,
            originalPinned: originalPinned,
            recentIndex: recentIndex.map { max(0, $0) },
            originalPinnedIndex: max(0, originalPinnedIndex ?? 0),
            originalPinnedOrder: Self.normalizedThreadIds(originalPinnedOrder),
            originalRecentOrder: Self.normalizedThreadIds(originalRecentOrder),
            sourceRow: sourceRow,
            sequence: nextSequence,
            remoteResolved: false
        )
        return true
    }

    mutating func resolvePin(threadId: String, pinned: Bool) {
        guard let threadId = Self.normalizedThreadId(threadId),
              var transition = pinTransitions[threadId] else {
            return
        }
        transition.targetPinned = pinned
        transition.remoteResolved = true
        pinTransitions[threadId] = transition
    }

    mutating func rollbackPin(threadId: String) {
        guard let threadId = Self.normalizedThreadId(threadId),
              var transition = pinTransitions[threadId] else {
            return
        }
        transition.targetPinned = transition.originalPinned
        transition.remoteResolved = true
        pinTransitions[threadId] = transition
    }

    mutating func reset() {
        archivePhases = [:]
        pinTransitions = [:]
    }

    mutating func reconcile(with baseSections: GaryxHomeThreadSections) {
        let baseRows = Dictionary(uniqueKeysWithValues: baseSections.allRows.map { ($0.id, $0) })
        archivePhases = archivePhases.filter { threadId, phase in
            phase != .committed || baseRows[threadId] != nil
        }
        pinTransitions = pinTransitions.filter { threadId, transition in
            guard transition.remoteResolved else {
                return true
            }
            if transition.targetPinned {
                return baseRows[threadId]?.presentation.isPinned != true
            }
            if transition.recentIndex == nil {
                return baseRows[threadId] != nil
            }
            return baseRows[threadId]?.presentation.isPinned != false
        }
    }

    func motion(for threadId: String) -> GaryxHomeThreadRowMotion {
        guard let threadId = Self.normalizedThreadId(threadId) else { return .stable }
        if archivePhases[threadId] != nil {
            return .archiving
        }
        if let transition = pinTransitions[threadId] {
            if !transition.targetPinned, transition.recentIndex == nil {
                return .leavingFilteredList
            }
            return .pinning
        }
        return .stable
    }

    func effectivePinnedState(
        for threadId: String,
        baseSections: GaryxHomeThreadSections
    ) -> Bool? {
        guard let threadId = Self.normalizedThreadId(threadId) else { return nil }
        if let transition = pinTransitions[threadId] {
            return transition.targetPinned
        }
        return baseSections.allRows.first(where: { $0.id == threadId })?.presentation.isPinned
    }

    /// Applies every in-flight pin intent to an authoritative id list. This
    /// preserves unrelated optimistic requests when one concurrent request
    /// resolves or rolls back with an older full-list response.
    func presentedPinnedThreadIds(from baseIds: [String]) -> [String] {
        var seen = Set<String>()
        var ids = baseIds.compactMap { rawId -> String? in
            guard let threadId = Self.normalizedThreadId(rawId),
                  seen.insert(threadId).inserted else {
                return nil
            }
            return threadId
        }
        for (threadId, transition) in pinTransitions.sorted(by: { $0.value.sequence < $1.value.sequence }) {
            ids.removeAll { $0 == threadId }
            if transition.targetPinned {
                let targetIndex = transition.restoresOriginalPlacement
                    ? Self.restorationIndex(
                        threadId: threadId,
                        originalOrder: transition.originalPinnedOrder,
                        currentIds: ids,
                        fallbackIndex: transition.originalPinnedIndex
                    )
                    : 0
                ids.insert(threadId, at: targetIndex)
            }
        }
        return ids
    }

    func presentedSections(from baseSections: GaryxHomeThreadSections) -> GaryxHomeThreadSections {
        guard !pinTransitions.isEmpty else { return baseSections }
        var rowsById = Dictionary(uniqueKeysWithValues: baseSections.allRows.map { ($0.id, $0) })
        for (threadId, transition) in pinTransitions where rowsById[threadId] == nil {
            rowsById[threadId] = transition.sourceRow
        }
        var pinnedIds = baseSections.pinned.map(\.id)
        var recentIds = baseSections.recent.map(\.id)

        for (threadId, transition) in pinTransitions.sorted(by: { $0.value.sequence < $1.value.sequence }) {
            guard rowsById[threadId] != nil else { continue }
            pinnedIds.removeAll { $0 == threadId }
            recentIds.removeAll { $0 == threadId }
            if transition.targetPinned {
                let targetIndex = transition.restoresOriginalPlacement
                    ? Self.restorationIndex(
                        threadId: threadId,
                        originalOrder: transition.originalPinnedOrder,
                        currentIds: pinnedIds,
                        fallbackIndex: transition.originalPinnedIndex
                    )
                    : 0
                pinnedIds.insert(threadId, at: targetIndex)
            } else if let recentIndex = transition.recentIndex {
                let targetIndex = transition.restoresOriginalPlacement
                    ? Self.restorationIndex(
                        threadId: threadId,
                        originalOrder: transition.originalRecentOrder,
                        currentIds: recentIds,
                        fallbackIndex: recentIndex
                    )
                    : min(recentIndex, recentIds.count)
                recentIds.insert(threadId, at: targetIndex)
            } else {
                let targetIndex = Self.restorationIndex(
                    threadId: threadId,
                    originalOrder: transition.originalPinnedOrder,
                    currentIds: pinnedIds,
                    fallbackIndex: transition.originalPinnedIndex
                )
                pinnedIds.insert(threadId, at: targetIndex)
            }
        }

        return GaryxHomeThreadSections(
            pinned: Self.placedRows(ids: pinnedIds, rowsById: rowsById, pinned: true),
            recent: Self.placedRows(ids: recentIds, rowsById: rowsById, pinned: false)
        )
    }

    private static func placedRows(
        ids: [String],
        rowsById: [String: GaryxHomeThreadRow],
        pinned: Bool
    ) -> [GaryxHomeThreadRow] {
        ids.enumerated().compactMap { index, threadId in
            guard let row = rowsById[threadId] else { return nil }
            return GaryxHomeThreadRow(
                id: row.id,
                thread: row.thread,
                presentation: row.presentation.withPinnedState(pinned),
                avatar: row.avatar,
                timestampValue: row.timestampValue,
                canArchive: row.canArchive,
                showsDivider: index > 0
            )
        }
    }

    private static func normalizedThreadId(_ rawId: String) -> String? {
        let threadId = rawId.trimmingCharacters(in: .whitespacesAndNewlines)
        return threadId.isEmpty ? nil : threadId
    }

    private static func normalizedThreadIds(_ rawIds: [String]) -> [String] {
        var seen = Set<String>()
        return rawIds.compactMap { rawId in
            guard let threadId = normalizedThreadId(rawId),
                  seen.insert(threadId).inserted else {
                return nil
            }
            return threadId
        }
    }

    /// Restores relative to the nearest surviving stable neighbor instead of
    /// trusting an index that another optimistic transition may have shifted.
    private static func restorationIndex(
        threadId: String,
        originalOrder: [String],
        currentIds: [String],
        fallbackIndex: Int
    ) -> Int {
        guard let originalIndex = originalOrder.firstIndex(of: threadId) else {
            return min(max(0, fallbackIndex), currentIds.count)
        }

        for candidateId in originalOrder[..<originalIndex].reversed() {
            if let candidateIndex = currentIds.firstIndex(of: candidateId) {
                return candidateIndex + 1
            }
        }
        for candidateId in originalOrder.dropFirst(originalIndex + 1) {
            if let candidateIndex = currentIds.firstIndex(of: candidateId) {
                return candidateIndex
            }
        }
        return min(max(0, fallbackIndex), currentIds.count)
    }
}

enum GaryxHomeThreadSectionsBuilder {
    static func build(_ input: GaryxHomeThreadSectionsInput) -> GaryxHomeThreadSections {
        var threadsById: [String: GaryxThreadSummary] = [:]
        for thread in input.threads where threadsById[thread.id] == nil {
            threadsById[thread.id] = thread
        }
        let pinnedIds = normalizedPinnedThreadIds(input.pinnedThreadIds)
        let pinnedIdSet = Set(pinnedIds)
        let selectedThreadId = input.selectedThreadId
        var agentsById: [String: GaryxAgentSummary] = [:]
        for agent in input.agents where agentsById[agent.id] == nil {
            agentsById[agent.id] = agent
        }
        let automationThreadIds = self.automationThreadIds(input.automations)

        let pinnedRows = pinnedIds
            .compactMap { threadsById[$0] }
            .enumerated()
            .map { index, thread in
                row(
                    thread: thread,
                    isSelected: selectedThreadId == thread.id,
                    isPinned: true,
                    showsDivider: index > 0,
                    agentsById: agentsById,
                    automationThreadIds: automationThreadIds
                )
            }

        let recentRows = input.recentThreadIds
            .filter { !pinnedIdSet.contains($0) }
            .compactMap { threadsById[$0] }
            .enumerated()
            .map { index, thread in
                row(
                    thread: thread,
                    isSelected: selectedThreadId == thread.id,
                    isPinned: false,
                    showsDivider: index > 0,
                    agentsById: agentsById,
                    automationThreadIds: automationThreadIds
                )
            }

        return GaryxHomeThreadSections(pinned: pinnedRows, recent: recentRows)
    }

    static func normalizedPinnedThreadIds(_ ids: [String]) -> [String] {
        var seen = Set<String>()
        var normalized: [String] = []
        for rawId in ids {
            let id = rawId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !id.isEmpty, seen.insert(id).inserted else { continue }
            normalized.append(id)
        }
        return normalized
    }

    static func automationThreadIds(_ automations: [GaryxAutomationSummary]) -> Set<String> {
        Set(automations.compactMap { automation -> String? in
            let threadId = (automation.targetThreadId ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            return threadId.isEmpty ? nil : threadId
        })
    }

    private static func row(
        thread: GaryxThreadSummary,
        isSelected: Bool,
        isPinned: Bool,
        showsDivider: Bool,
        agentsById: [String: GaryxAgentSummary],
        automationThreadIds: Set<String>
    ) -> GaryxHomeThreadRow {
        let identity = self.identity(for: thread, agentsById: agentsById)
        return GaryxHomeThreadRow(
            id: thread.id,
            thread: thread,
            presentation: GaryxSidebarThreadRowPresentation(
                thread: thread,
                isSelected: isSelected,
                isPinned: isPinned,
                trailingTimestamp: nil,
                showsRunningState: false
            ),
            avatar: GaryxSidebarThreadRowAvatar(
                agentId: identity.id ?? "",
                avatarDataUrl: identity.avatarDataUrl ?? "",
                label: identity.name ?? thread.title,
                providerType: identity.providerType ?? "",
                builtIn: identity.builtIn
            ),
            timestampValue: thread.updatedAt ?? thread.createdAt,
            canArchive: !automationThreadIds.contains(thread.id),
            showsDivider: showsDivider
        )
    }

    private static func identity(
        for thread: GaryxThreadSummary,
        agentsById: [String: GaryxAgentSummary]
    ) -> GaryxWidgetAgentIdentity {
        GaryxWidgetAgentIdentityProjector.identity(
            for: thread,
            agentsById: agentsById
        )
    }
}

final class GaryxHomeThreadSectionsCache {
    private var previousKey: GaryxHomeThreadSectionsIdentityKey?
    private var previousSections = GaryxHomeThreadSections()
    private(set) var derivationCount = 0

    func sections(for input: GaryxHomeThreadSectionsInput) -> GaryxHomeThreadSections {
        let key = GaryxHomeThreadSectionsIdentityKey(input)
        if previousKey == key {
            return previousSections
        }
        previousKey = key
        previousSections = GaryxHomeThreadSectionsBuilder.build(input)
        derivationCount += 1
        return previousSections
    }
}

struct GaryxHomeThreadSectionsIdentityKey: Equatable {
    var threads: [GaryxThreadSummary]
    var agents: [GaryxAgentSummary]
    var automationThreadIds: Set<String>
    var pinnedThreadIds: [String]
    var recentThreadIds: [String]
    var selectedThreadId: String?

    init(_ input: GaryxHomeThreadSectionsInput) {
        threads = input.threads.map(Self.displayThread)
        agents = input.agents
        automationThreadIds = GaryxHomeThreadSectionsBuilder.automationThreadIds(input.automations)
        pinnedThreadIds = GaryxHomeThreadSectionsBuilder.normalizedPinnedThreadIds(input.pinnedThreadIds)
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

final class GaryxHomeThreadListStore: ObservableObject {
    @Published private(set) var snapshot: GaryxHomeThreadListSnapshot
    private var previousInput: GaryxHomeThreadListInput?
    private let sectionsCache = GaryxHomeThreadSectionsCache()
    private var transitionState = GaryxHomeThreadTransitionState()
    private(set) var pinnedOrderState: GaryxPinnedOrderState
    private(set) var latestActorAppliedSeq = 0
    private(set) var acceptedInputCount = 0
    private(set) var acceptedActorSnapshotCount = 0
    private(set) var publishCount = 0

    init(snapshot: GaryxHomeThreadListSnapshot = .empty) {
        self.snapshot = snapshot
        pinnedOrderState = GaryxPinnedOrderState(
            gatewayIdentity: "",
            initialOrder: snapshot.sections.pinned.map(\.id)
        )
    }

    var presentationSnapshot: GaryxHomeThreadListSnapshot {
        GaryxHomeThreadListSnapshot(
            sections: transitionState.presentedSections(from: snapshot.sections),
            isLoadingThreads: snapshot.isLoadingThreads,
            isHomeVisible: snapshot.isHomeVisible,
            selectedRecentFilter: snapshot.selectedRecentFilter,
            recentFeedPresentation: snapshot.recentFeedPresentation
        )
    }

    func rowMotion(threadId: String) -> GaryxHomeThreadRowMotion {
        transitionState.motion(for: threadId)
    }

    func effectivePinnedState(threadId: String) -> Bool? {
        transitionState.effectivePinnedState(for: threadId, baseSections: snapshot.sections)
    }

    func presentedPinnedThreadIds(from baseIds: [String]) -> [String] {
        transitionState.presentedPinnedThreadIds(from: baseIds)
    }

    /// Runs one pure pinned-order reduction while keeping the authority state
    /// owned by the home-list store. App-layer transport code executes the
    /// returned effects, but cannot replace the reducer domain piecemeal.
    @discardableResult
    func updatePinnedOrderState(
        _ update: (inout GaryxPinnedOrderState) -> GaryxPinnedOrderUpdate
    ) -> GaryxPinnedOrderUpdate {
        var next = pinnedOrderState
        let result = update(&next)
        guard next != pinnedOrderState else { return result }
        objectWillChange.send()
        pinnedOrderState = next
        return result
    }

    @discardableResult
    func beginArchiveTransition(threadId: String) -> Bool {
        return updateTransitionState { state in
            state.beginArchive(threadId: threadId)
        }
    }

    func commitArchiveTransition(threadId: String) {
        updateTransitionState { state in
            let before = state
            state.commitArchive(threadId: threadId)
            return state != before
        }
    }

    func cancelArchiveTransition(threadId: String) {
        updateTransitionState { state in
            let before = state
            state.cancelArchive(threadId: threadId)
            return state != before
        }
    }

    @discardableResult
    func beginPinTransition(
        threadId: String,
        pinned: Bool,
        originalPinned: Bool,
        recentIndex: Int?
    ) -> Bool {
        let presentedSections = transitionState.presentedSections(from: snapshot.sections)
        let sourceRow = presentedSections.allRows.first { $0.id == threadId }
        let originalPinnedIndex = presentedSections.pinned.firstIndex { $0.id == threadId }
        let originalPinnedOrder = presentedSections.pinned.map(\.id)
        let originalRecentOrder = presentedSections.recent.map(\.id)
        return updateTransitionState { state in
            state.beginPin(
                threadId: threadId,
                pinned: pinned,
                originalPinned: originalPinned,
                recentIndex: recentIndex,
                originalPinnedIndex: originalPinnedIndex,
                originalPinnedOrder: originalPinnedOrder,
                originalRecentOrder: originalRecentOrder,
                sourceRow: sourceRow
            )
        }
    }

    func resolvePinTransition(threadId: String, pinned: Bool) {
        updateTransitionState { state in
            let before = state
            state.resolvePin(threadId: threadId, pinned: pinned)
            state.reconcile(with: snapshot.sections)
            return state != before
        }
    }

    @discardableResult
    func rollbackPinTransition(
        threadId: String,
        basePinnedIds: [String]? = nil
    ) -> [String]? {
        let before = transitionState
        var next = before
        next.rollbackPin(threadId: threadId)
        // Derive the canonical rollback before reconciliation can retire a
        // fast-failing transition against an actor snapshot that never saw it.
        let presentedIds = basePinnedIds.map { next.presentedPinnedThreadIds(from: $0) }
        next.reconcile(with: snapshot.sections)
        if next != before {
            objectWillChange.send()
            transitionState = next
        }
        return presentedIds
    }

    func resetTransitions() {
        updateTransitionState { state in
            guard !state.isEmpty else { return false }
            state.reset()
            return true
        }
    }

    var sectionDerivationCount: Int {
        sectionsCache.derivationCount
    }

    @discardableResult
    func apply(_ input: GaryxHomeThreadListInput) -> Bool {
        if previousInput == input {
            return false
        }
        previousInput = input
        acceptedInputCount += 1

        let baseSections = sectionsCache.sections(for: input.sectionsInput)
        let next = GaryxHomeThreadListSnapshot(
            sections: Self.sections(baseSections, runningThreadIds: input.runningThreadIds),
            isLoadingThreads: input.isLoadingThreads,
            isHomeVisible: input.isHomeVisible,
            selectedRecentFilter: input.selectedRecentFilter,
            recentFeedPresentation: input.recentFeedPresentation
        )
        var reconciledTransitions = transitionState
        reconciledTransitions.reconcile(with: next.sections)
        let transitionsChanged = reconciledTransitions != transitionState
        guard snapshot != next || transitionsChanged else {
            return false
        }
        apply(next, reconciledTransitions: reconciledTransitions)
        publishCount += 1
        return true
    }

    @discardableResult
    func apply(actorSnapshot: HomeSnapshot, difference _: CollectionDifference<String>? = nil) -> Bool {
        guard actorSnapshot.appliedSeq > latestActorAppliedSeq else {
            return false
        }
        latestActorAppliedSeq = actorSnapshot.appliedSeq
        acceptedActorSnapshotCount += 1

        let next = GaryxHomeThreadListSnapshot(
            sections: actorSnapshot.sections,
            isLoadingThreads: actorSnapshot.isLoadingThreads,
            isHomeVisible: actorSnapshot.isHomeVisible,
            selectedRecentFilter: actorSnapshot.selectedRecentFilter,
            recentFeedPresentation: actorSnapshot.recentFeedPresentation
        )
        var reconciledTransitions = transitionState
        reconciledTransitions.reconcile(with: next.sections)
        let transitionsChanged = reconciledTransitions != transitionState
        guard snapshot != next || transitionsChanged else {
            return false
        }
        apply(next, reconciledTransitions: reconciledTransitions)
        publishCount += 1
        return true
    }

    private func apply(
        _ nextSnapshot: GaryxHomeThreadListSnapshot,
        reconciledTransitions: GaryxHomeThreadTransitionState
    ) {
        let snapshotChanged = snapshot != nextSnapshot
        if !snapshotChanged {
            objectWillChange.send()
        }
        transitionState = reconciledTransitions
        if snapshotChanged {
            snapshot = nextSnapshot
        }
    }

    @discardableResult
    private func updateTransitionState(
        _ update: (inout GaryxHomeThreadTransitionState) -> Bool
    ) -> Bool {
        var next = transitionState
        guard update(&next), next != transitionState else { return false }
        objectWillChange.send()
        transitionState = next
        return true
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
        let normalizedId = row.id.trimmingCharacters(in: .whitespacesAndNewlines)
        let isRunning = !normalizedId.isEmpty && runningThreadIds.contains(normalizedId)
        guard row.presentation.isRunning != isRunning else {
            return row
        }
        return GaryxHomeThreadRow(
            id: row.id,
            thread: row.thread,
            presentation: row.presentation.withRunningState(isRunning),
            avatar: row.avatar,
            timestampValue: row.timestampValue,
            canArchive: row.canArchive,
            showsDivider: row.showsDivider
        )
    }
}

struct GaryxRecentThreadsWidgetSnapshotInput: Equatable, Sendable {
    var threads: [GaryxThreadSummary]
    var agents: [GaryxAgentSummary]
    var pinnedThreadIds: [String]
    var recentThreadIds: [String]
    var gatewayScopeId: String

    init(
        threads: [GaryxThreadSummary],
        agents: [GaryxAgentSummary],
        pinnedThreadIds: [String],
        recentThreadIds: [String],
        gatewayScopeId: String = ""
    ) {
        self.threads = threads
        self.agents = agents
        self.pinnedThreadIds = pinnedThreadIds
        self.recentThreadIds = recentThreadIds
        self.gatewayScopeId = gatewayScopeId
    }
}

enum GaryxRecentThreadsWidgetSnapshotProjector {
    static func widgetThreads(
        from input: GaryxRecentThreadsWidgetSnapshotInput,
        avatarFallback: [GaryxAvatarIdentity: String] = [:]
    ) -> [GaryxMobileWidgetThread] {
        var summariesById: [String: GaryxThreadSummary] = [:]
        for thread in input.threads where summariesById[thread.id] == nil {
            summariesById[thread.id] = thread
        }
        var agentsById: [String: GaryxAgentSummary] = [:]
        for agent in input.agents where agentsById[agent.id] == nil {
            agentsById[agent.id] = agent
        }
        return normalizedThreadIds(input.pinnedThreadIds + input.recentThreadIds).compactMap { threadId in
            guard let thread = summariesById[threadId] else { return nil }
            let workspaceName = thread.workspacePath?
                .garyxLastPathComponent
                .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            let identity = widgetAgentIdentity(for: thread, agentsById: agentsById)
            return GaryxMobileWidgetThread(
                id: thread.id,
                title: thread.title,
                workspaceName: workspaceName,
                updatedAt: thread.updatedAt ?? thread.createdAt,
                activeRunId: thread.activeRunId,
                runState: thread.runState,
                agentId: identity.id,
                agentName: identity.name,
                avatarDataUrl: widgetAvatarDataUrl(
                    identity: identity,
                    gatewayScopeId: input.gatewayScopeId,
                    avatarFallback: avatarFallback
                ),
                avatarScope: widgetAvatarScope(
                    identity: identity,
                    gatewayScopeId: input.gatewayScopeId,
                    avatarFallback: avatarFallback
                ),
                avatarFingerprint: widgetAvatarFingerprint(
                    identity: identity,
                    gatewayScopeId: input.gatewayScopeId,
                    avatarFallback: avatarFallback
                ),
                providerType: identity.providerType,
                builtIn: identity.builtIn
            )
        }
    }

    static func avatarIdentities(from input: GaryxRecentThreadsWidgetSnapshotInput) -> [GaryxAvatarIdentity] {
        let scope = input.gatewayScopeId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !scope.isEmpty else { return [] }
        var summariesById: [String: GaryxThreadSummary] = [:]
        for thread in input.threads where summariesById[thread.id] == nil {
            summariesById[thread.id] = thread
        }
        var identities: [GaryxAvatarIdentity] = []
        var seen = Set<String>()
        for threadId in normalizedThreadIds(input.pinnedThreadIds + input.recentThreadIds) {
            guard let thread = summariesById[threadId],
                  let identity = avatarIdentity(for: thread, scope: scope) else {
                continue
            }
            guard seen.insert(identity.storageKey).inserted else { continue }
            identities.append(identity)
        }
        return identities
    }

    private static func normalizedThreadIds(_ ids: [String]) -> [String] {
        var seen = Set<String>()
        var normalized: [String] = []
        for rawId in ids {
            let id = rawId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !id.isEmpty, seen.insert(id).inserted else { continue }
            normalized.append(id)
        }
        return normalized
    }

    private static func widgetAvatarDataUrl(
        identity: GaryxWidgetAgentIdentity,
        gatewayScopeId: String,
        avatarFallback: [GaryxAvatarIdentity: String]
    ) -> String? {
        let scope = gatewayScopeId.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !scope.isEmpty else {
            return identity.avatarDataUrl
        }
        guard avatarIdentity(identity: identity, scope: scope) != nil else {
            return identity.avatarDataUrl
        }
        return nil
    }

    private static func widgetAvatarScope(
        identity: GaryxWidgetAgentIdentity,
        gatewayScopeId: String,
        avatarFallback: [GaryxAvatarIdentity: String]
    ) -> String? {
        guard let avatarIdentity = avatarIdentity(identity: identity, scope: gatewayScopeId),
              avatarFallback[avatarIdentity] != nil else {
            return nil
        }
        return avatarIdentity.scope
    }

    private static func widgetAvatarFingerprint(
        identity: GaryxWidgetAgentIdentity,
        gatewayScopeId: String,
        avatarFallback: [GaryxAvatarIdentity: String]
    ) -> String? {
        guard let avatarIdentity = avatarIdentity(identity: identity, scope: gatewayScopeId) else {
            return nil
        }
        return avatarFallback[avatarIdentity]
    }

    private static func avatarIdentity(for thread: GaryxThreadSummary, scope: String) -> GaryxAvatarIdentity? {
        let agentId = thread.agentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !agentId.isEmpty else { return nil }
        return GaryxAvatarIdentity(scope: scope, id: agentId)
    }

    private static func avatarIdentity(identity: GaryxWidgetAgentIdentity, scope: String) -> GaryxAvatarIdentity? {
        let scope = scope.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !scope.isEmpty,
              let id = identity.id?.trimmingCharacters(in: .whitespacesAndNewlines),
              !id.isEmpty else {
            return nil
        }
        return GaryxAvatarIdentity(scope: scope, id: id)
    }

    private static func widgetAgentIdentity(
        for thread: GaryxThreadSummary,
        agentsById: [String: GaryxAgentSummary]
    ) -> GaryxWidgetAgentIdentity {
        GaryxWidgetAgentIdentityProjector.identity(
            for: thread,
            agentsById: agentsById
        )
    }
}

final class GaryxRecentThreadsWidgetPersistencePlanner {
    enum Decision: Equatable {
        case skipUnchanged
        case write([GaryxMobileWidgetThread])
    }

    private var lastWrittenThreads: [GaryxMobileWidgetThread]?

    func nextWrite(for threads: [GaryxMobileWidgetThread]) -> Decision {
        guard threads != lastWrittenThreads else {
            return .skipUnchanged
        }
        lastWrittenThreads = threads
        return .write(threads)
    }
}

/// Whether a recent-thread row that just left the running state should have
/// its committed history hydrated in the background: the thread is not the
/// open conversation, is no longer running, and was observed running (or
/// remote-busy) on the previous refresh.
enum GaryxCompletedThreadHydrationPolicy {
    static func shouldHydrate(
        previousThread: GaryxThreadSummary?,
        previousRemoteBusyThreadIds: Set<String>,
        refreshedThread: GaryxThreadSummary,
        selectedThreadId: String?
    ) -> Bool {
        let threadId = refreshedThread.id.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !threadId.isEmpty,
              selectedThreadId != threadId,
              !GaryxThreadSummaryRunStateResolver.isRunning(refreshedThread) else {
            return false
        }
        return previousThread.map(GaryxThreadSummaryRunStateResolver.isRunning) == true
            || previousRemoteBusyThreadIds.contains(threadId)
    }
}

struct GaryxBackgroundCommittedRunReconcileDecision: Equatable, Sendable {
    var candidateThreadIds: [String]
    var refreshesThreads: Bool
    var hydratesCandidateThreads: Bool

    static let idle = GaryxBackgroundCommittedRunReconcileDecision(
        candidateThreadIds: [],
        refreshesThreads: false,
        hydratesCandidateThreads: false
    )
}

final class GaryxBackgroundCommittedRunReconcilePlanner {
    private let minimumRefreshInterval: TimeInterval
    private var lastRefreshAt: Date?
    private var lastCandidateThreadIds: [String] = []

    init(minimumRefreshInterval: TimeInterval) {
        self.minimumRefreshInterval = minimumRefreshInterval
    }

    /// Threads the background reconcile loop should watch: locally tracked
    /// runs, committed-busy threads, and summary-running threads without a
    /// committed state — excluding the open conversation.
    static func candidateThreadIds(
        locallyTrackedThreadIds: Set<String>,
        runStateByThread: [String: GaryxTranscriptRunState],
        threads: [GaryxThreadSummary],
        selectedThreadId: String?
    ) -> [String] {
        let selectedId = selectedThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        var ids = locallyTrackedThreadIds
        ids.formUnion(runStateByThread.compactMap { threadId, state in
            state.busy ? threadId : nil
        })
        ids.formUnion(threads.compactMap { thread in
            if let committedState = runStateByThread[thread.id] {
                return committedState.busy ? thread.id : nil
            }
            return GaryxThreadSummaryRunStateResolver.isRunning(thread) ? thread.id : nil
        })
        return ids
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty && $0 != selectedId }
            .sorted()
    }

    func nextDecision(
        candidateThreadIds: [String],
        now: Date = Date(),
        forceRefresh: Bool = false
    ) -> GaryxBackgroundCommittedRunReconcileDecision {
        let normalizedIds = normalizedThreadIds(candidateThreadIds)
        let intervalElapsed = lastRefreshAt.map { now.timeIntervalSince($0) >= minimumRefreshInterval } ?? true
        let shouldRefresh = forceRefresh || intervalElapsed
        guard !normalizedIds.isEmpty else {
            lastCandidateThreadIds = []
            if shouldRefresh {
                lastRefreshAt = now
            }
            return GaryxBackgroundCommittedRunReconcileDecision(
                candidateThreadIds: [],
                refreshesThreads: shouldRefresh,
                hydratesCandidateThreads: false
            )
        }

        let candidatesChanged = normalizedIds != lastCandidateThreadIds
        let refreshesThreads = shouldRefresh || candidatesChanged
        if refreshesThreads {
            lastRefreshAt = now
        }
        lastCandidateThreadIds = normalizedIds

        return GaryxBackgroundCommittedRunReconcileDecision(
            candidateThreadIds: normalizedIds,
            refreshesThreads: refreshesThreads,
            hydratesCandidateThreads: true
        )
    }

    private func normalizedThreadIds(_ ids: [String]) -> [String] {
        var seen = Set<String>()
        var normalized: [String] = []
        for rawId in ids {
            let id = rawId.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !id.isEmpty, seen.insert(id).inserted else { continue }
            normalized.append(id)
        }
        return normalized.sorted()
    }
}

enum GaryxEquatableAssignment {
    @discardableResult
    static func assignIfChanged<Value: Equatable>(
        current: Value,
        next: Value,
        assign: (Value) -> Void
    ) -> Bool {
        guard current != next else { return false }
        assign(next)
        return true
    }
}
