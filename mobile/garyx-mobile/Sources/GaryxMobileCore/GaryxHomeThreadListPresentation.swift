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

/// Agent-or-team identity projected from a thread summary for list rows and
/// widget snapshots.
struct GaryxWidgetAgentIdentity: Equatable, Sendable {
    var id: String?
    var name: String?
    var avatarDataUrl: String?
    var providerType: String?
    var isTeam: Bool
    var builtIn: Bool
}

/// Single source of truth for resolving a thread's display identity: team
/// first (catalog hit, then thread fallback), then agent (catalog hit, then
/// thread fallback), then provider-only.
enum GaryxWidgetAgentIdentityProjector {
    static func identity(
        for thread: GaryxThreadSummary,
        agents: [GaryxAgentSummary],
        teams: [GaryxTeamSummary]
    ) -> GaryxWidgetAgentIdentity {
        identity(
            for: thread,
            team: { teamId in teams.first { $0.id == teamId } },
            agent: { agentId in agents.first { $0.id == agentId } }
        )
    }

    static func identity(
        for thread: GaryxThreadSummary,
        agentsById: [String: GaryxAgentSummary],
        teamsById: [String: GaryxTeamSummary]
    ) -> GaryxWidgetAgentIdentity {
        identity(
            for: thread,
            team: { teamsById[$0] },
            agent: { agentsById[$0] }
        )
    }

    private static func identity(
        for thread: GaryxThreadSummary,
        team teamById: (String) -> GaryxTeamSummary?,
        agent agentById: (String) -> GaryxAgentSummary?
    ) -> GaryxWidgetAgentIdentity {
        let teamId = thread.teamId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !teamId.isEmpty {
            if let team = teamById(teamId) {
                return GaryxWidgetAgentIdentity(
                    id: team.id,
                    name: team.displayName,
                    avatarDataUrl: team.avatarDataUrl.isEmpty ? nil : team.avatarDataUrl,
                    providerType: nil,
                    isTeam: true,
                    builtIn: false
                )
            }
            return GaryxWidgetAgentIdentity(
                id: teamId,
                name: thread.teamName,
                avatarDataUrl: nil,
                providerType: nil,
                isTeam: true,
                builtIn: false
            )
        }

        let agentId = thread.agentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !agentId.isEmpty {
            if let agent = agentById(agentId) {
                return GaryxWidgetAgentIdentity(
                    id: agent.id,
                    name: agent.displayName,
                    avatarDataUrl: agent.avatarDataUrl.isEmpty ? nil : agent.avatarDataUrl,
                    providerType: agent.providerType,
                    isTeam: false,
                    builtIn: agent.builtIn
                )
            }
            return GaryxWidgetAgentIdentity(
                id: agentId,
                name: nil,
                avatarDataUrl: nil,
                providerType: thread.providerType,
                isTeam: false,
                builtIn: false
            )
        }

        return GaryxWidgetAgentIdentity(
            id: nil,
            name: nil,
            avatarDataUrl: nil,
            providerType: thread.providerType,
            isTeam: false,
            builtIn: false
        )
    }
}

struct GaryxSidebarThreadRowAvatar: Equatable, Sendable {
    let agentId: String
    let avatarDataUrl: String
    let kind: GaryxMobileAgentTarget.Kind
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

    private static func subtitle(for thread: GaryxThreadSummary) -> String? {
        let context: String? = {
            if let workspacePath = thread.workspacePath, !workspacePath.isEmpty {
                return workspacePath.garyxLastPathComponent
            }
            if let teamName = thread.teamName, !teamName.isEmpty {
                return teamName
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
    var teams: [GaryxTeamSummary]
    var automations: [GaryxAutomationSummary]
    var pinnedThreadIds: [String]
    var recentThreadIds: [String]
    var selectedThreadId: String?

    init(
        threads: [GaryxThreadSummary],
        agents: [GaryxAgentSummary],
        teams: [GaryxTeamSummary],
        automations: [GaryxAutomationSummary],
        pinnedThreadIds: [String],
        recentThreadIds: [String],
        selectedThreadId: String?
    ) {
        self.threads = threads
        self.agents = agents
        self.teams = teams
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

    init(
        sectionsInput: GaryxHomeThreadSectionsInput,
        runningThreadIds: Set<String>,
        isLoadingThreads: Bool,
        isHomeVisible: Bool
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
    }

    static func == (lhs: GaryxHomeThreadListInput, rhs: GaryxHomeThreadListInput) -> Bool {
        IdentityKey(lhs) == IdentityKey(rhs)
    }

    private struct IdentityKey: Equatable {
        var sections: GaryxHomeThreadSectionsIdentityKey
        var runningThreadIds: Set<String>
        var isLoadingThreads: Bool
        var isHomeVisible: Bool

        init(_ input: GaryxHomeThreadListInput) {
            sections = GaryxHomeThreadSectionsIdentityKey(input.sectionsInput)
            runningThreadIds = input.runningThreadIds
            isLoadingThreads = input.isLoadingThreads
            isHomeVisible = input.isHomeVisible
        }
    }
}

struct GaryxHomeThreadListSnapshot: Equatable, Sendable {
    var sections = GaryxHomeThreadSections()
    var isLoadingThreads = false
    var isHomeVisible = false

    var recentPlaceholder: GaryxHomeRecentPlaceholder {
        guard sections.recent.isEmpty else { return .none }
        return isLoadingThreads ? .loadingSkeleton(rowCount: 6) : .empty
    }

    static let empty = GaryxHomeThreadListSnapshot()
}

enum GaryxHomeRecentPlaceholder: Equatable, Sendable {
    case none
    case loadingSkeleton(rowCount: Int)
    case empty
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

enum GaryxHomeThreadSectionsBuilder {
    static func build(_ input: GaryxHomeThreadSectionsInput) -> GaryxHomeThreadSections {
        var threadsById: [String: GaryxThreadSummary] = [:]
        for thread in input.threads where threadsById[thread.id] == nil {
            threadsById[thread.id] = thread
        }
        let pinnedIds = normalizedPinnedThreadIds(input.pinnedThreadIds)
        let pinnedIdSet = Set(pinnedIds)
        let selectedThreadId = input.selectedThreadId
        var teamsById: [String: GaryxTeamSummary] = [:]
        for team in input.teams where teamsById[team.id] == nil {
            teamsById[team.id] = team
        }
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
                    teamsById: teamsById,
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
                    teamsById: teamsById,
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
        teamsById: [String: GaryxTeamSummary],
        agentsById: [String: GaryxAgentSummary],
        automationThreadIds: Set<String>
    ) -> GaryxHomeThreadRow {
        let identity = self.identity(for: thread, teamsById: teamsById, agentsById: agentsById)
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
                kind: identity.isTeam ? .team : .agent,
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
        teamsById: [String: GaryxTeamSummary],
        agentsById: [String: GaryxAgentSummary]
    ) -> GaryxWidgetAgentIdentity {
        GaryxWidgetAgentIdentityProjector.identity(
            for: thread,
            agentsById: agentsById,
            teamsById: teamsById
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
    var teams: [GaryxTeamSummary]
    var automationThreadIds: Set<String>
    var pinnedThreadIds: [String]
    var recentThreadIds: [String]
    var selectedThreadId: String?

    init(_ input: GaryxHomeThreadSectionsInput) {
        threads = input.threads.map(Self.displayThread)
        agents = input.agents
        teams = input.teams
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
    private(set) var latestActorAppliedSeq = 0
    private(set) var acceptedInputCount = 0
    private(set) var acceptedActorSnapshotCount = 0
    private(set) var publishCount = 0

    init(snapshot: GaryxHomeThreadListSnapshot = .empty) {
        self.snapshot = snapshot
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
            isHomeVisible: input.isHomeVisible
        )
        guard snapshot != next else {
            return false
        }
        snapshot = next
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
            isHomeVisible: actorSnapshot.isHomeVisible
        )
        guard snapshot != next else {
            return false
        }
        snapshot = next
        publishCount += 1
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
    var teams: [GaryxTeamSummary]
    var pinnedThreadIds: [String]
    var recentThreadIds: [String]
    var gatewayScopeId: String

    init(
        threads: [GaryxThreadSummary],
        agents: [GaryxAgentSummary],
        teams: [GaryxTeamSummary],
        pinnedThreadIds: [String],
        recentThreadIds: [String],
        gatewayScopeId: String = ""
    ) {
        self.threads = threads
        self.agents = agents
        self.teams = teams
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
        var teamsById: [String: GaryxTeamSummary] = [:]
        for team in input.teams where teamsById[team.id] == nil {
            teamsById[team.id] = team
        }

        return normalizedThreadIds(input.pinnedThreadIds + input.recentThreadIds).compactMap { threadId in
            guard let thread = summariesById[threadId] else { return nil }
            let workspaceName = thread.workspacePath?
                .garyxLastPathComponent
                .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            let identity = widgetAgentIdentity(for: thread, agentsById: agentsById, teamsById: teamsById)
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
                isTeam: identity.isTeam,
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
        let teamId = thread.teamId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !teamId.isEmpty {
            return GaryxAvatarIdentity(scope: scope, kind: .team, id: teamId)
        }
        let agentId = thread.agentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !agentId.isEmpty else { return nil }
        return GaryxAvatarIdentity(scope: scope, kind: .agent, id: agentId)
    }

    private static func avatarIdentity(identity: GaryxWidgetAgentIdentity, scope: String) -> GaryxAvatarIdentity? {
        let scope = scope.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !scope.isEmpty,
              let id = identity.id?.trimmingCharacters(in: .whitespacesAndNewlines),
              !id.isEmpty else {
            return nil
        }
        return GaryxAvatarIdentity(scope: scope, kind: identity.isTeam ? .team : .agent, id: id)
    }

    private static func widgetAgentIdentity(
        for thread: GaryxThreadSummary,
        agentsById: [String: GaryxAgentSummary],
        teamsById: [String: GaryxTeamSummary]
    ) -> GaryxWidgetAgentIdentity {
        GaryxWidgetAgentIdentityProjector.identity(
            for: thread,
            agentsById: agentsById,
            teamsById: teamsById
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
