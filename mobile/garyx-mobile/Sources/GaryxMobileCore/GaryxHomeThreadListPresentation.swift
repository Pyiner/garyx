import Foundation

enum GaryxThreadSummaryRunStateResolver {
    static func resolvedRunState(
        apiRunState: String?,
        recentRunId: String?,
        committedState: GaryxTranscriptRunState?
    ) -> String? {
        guard let committedState else {
            return apiRunState
        }

        if committedState.busy {
            return "running"
        }

        if let terminal = trimmed(committedState.terminalStatus) {
            return terminal
        }

        return hasValue(recentRunId) ? "completed" : "idle"
    }

    private static func trimmed(_ value: String?) -> String? {
        let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.isEmpty ? nil : trimmed
    }

    private static func hasValue(_ value: String?) -> Bool {
        !(value?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ?? true)
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
        let state = thread.runState?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return state == "running"
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
    ) -> AgentIdentity {
        let teamId = thread.teamId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !teamId.isEmpty {
            if let team = teamsById[teamId] {
                return AgentIdentity(
                    id: team.id,
                    name: team.displayName,
                    avatarDataUrl: team.avatarDataUrl.isEmpty ? nil : team.avatarDataUrl,
                    providerType: nil,
                    isTeam: true,
                    builtIn: false
                )
            }
            return AgentIdentity(
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
            if let agent = agentsById[agentId] {
                return AgentIdentity(
                    id: agent.id,
                    name: agent.displayName,
                    avatarDataUrl: agent.avatarDataUrl.isEmpty ? nil : agent.avatarDataUrl,
                    providerType: agent.providerType,
                    isTeam: false,
                    builtIn: agent.builtIn
                )
            }
            return AgentIdentity(
                id: agentId,
                name: nil,
                avatarDataUrl: nil,
                providerType: thread.providerType,
                isTeam: false,
                builtIn: false
            )
        }

        return AgentIdentity(
            id: nil,
            name: nil,
            avatarDataUrl: nil,
            providerType: thread.providerType,
            isTeam: false,
            builtIn: false
        )
    }

    private struct AgentIdentity {
        var id: String?
        var name: String?
        var avatarDataUrl: String?
        var providerType: String?
        var isTeam: Bool
        var builtIn: Bool
    }
}

final class GaryxHomeThreadSectionsCache {
    private var previousKey: IdentityKey?
    private var previousSections = GaryxHomeThreadSections()
    private(set) var derivationCount = 0

    func sections(for input: GaryxHomeThreadSectionsInput) -> GaryxHomeThreadSections {
        let key = IdentityKey(input)
        if previousKey == key {
            return previousSections
        }
        previousKey = key
        previousSections = GaryxHomeThreadSectionsBuilder.build(input)
        derivationCount += 1
        return previousSections
    }

    private struct IdentityKey: Equatable {
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

enum GaryxBackgroundThreadReconcilePolicy {
    static func shouldRefreshThreads(
        isThreadListInteracting: Bool,
        candidateThreadIds: [String]
    ) -> Bool {
        guard !isThreadListInteracting else { return false }
        return candidateThreadIds.contains { threadId in
            !threadId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }
}
