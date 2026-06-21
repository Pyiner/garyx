import Combine
import XCTest
@testable import GaryxMobileCore

final class GaryxHomeThreadListPerformanceDiagnosticsTests: XCTestCase {
    func testHomeThreadSectionDerivationCurrentCostAtHomeScale() {
        let fixture = DiagnosticHomeThreadFixture.make()
        let deriver = DiagnosticHomeThreadSectionDeriver()

        let sections = deriver.sections(input: fixture.input)
        XCTAssertEqual(sections.totalRows, 50)
        XCTAssertEqual(sections.pinned.count, 8)
        XCTAssertEqual(sections.recent.count, 42)

        let timing = DiagnosticTiming.measure(iterations: 1_000) {
            deriver.sections(input: fixture.input).totalRows
        }
        print("DIAGNOSTIC home_sections_current_avg_ms=\(timing.averageMilliseconds)")

        var measuredRows = 0
        measure(metrics: [XCTClockMetric()]) {
            measuredRows = deriver.sections(input: fixture.input).totalRows
        }
        XCTAssertEqual(measuredRows, 50)
    }

    func testCachedHomeThreadSectionDerivationReusesSectionsForIdenticalInput() {
        let fixture = DiagnosticHomeThreadFixture.make()
        let cache = DiagnosticHomeThreadSectionCache()

        XCTAssertEqual(cache.sections(input: fixture.input).totalRows, 50)
        XCTAssertEqual(cache.derivationCount, 1)

        let repeated = cache.sections(input: fixture.input)
        XCTAssertEqual(repeated.totalRows, 50)
        XCTAssertEqual(cache.derivationCount, 1)

        var nextMinute = fixture.input
        nextMinute.relativeTimeBucket += 60
        XCTAssertEqual(cache.sections(input: nextMinute).totalRows, 50)
        XCTAssertEqual(cache.derivationCount, 2)

        let warmCache = DiagnosticHomeThreadSectionCache()
        _ = warmCache.sections(input: fixture.input)
        let timing = DiagnosticTiming.measure(iterations: 10_000) {
            warmCache.sections(input: fixture.input).totalRows
        }
        print("DIAGNOSTIC home_sections_cached_hit_avg_ms=\(timing.averageMilliseconds)")

        var measuredRows = 0
        measure(metrics: [XCTClockMetric()]) {
            measuredRows = warmCache.sections(input: fixture.input).totalRows
        }
        XCTAssertEqual(measuredRows, 50)
        XCTAssertEqual(warmCache.derivationCount, 1)
    }

    func testIdenticalCatalogRefreshPublishesAndRecomputesWithoutEquatableGuard() {
        let fixture = DiagnosticHomeThreadFixture.make()
        let deriver = DiagnosticHomeThreadSectionDeriver()

        let current = DiagnosticAgentTargetPublicationBox(
            agents: fixture.agents,
            teams: fixture.teams,
            automations: fixture.automations
        )
        var currentPublishes = 0
        var currentDerivedRows = 0
        var currentCancellables: Set<AnyCancellable> = []
        current.$agents.dropFirst().sink { _ in
            currentPublishes += 1
            currentDerivedRows += deriver.sections(input: fixture.input).totalRows
        }.store(in: &currentCancellables)
        current.$teams.dropFirst().sink { _ in
            currentPublishes += 1
            currentDerivedRows += deriver.sections(input: fixture.input).totalRows
        }.store(in: &currentCancellables)
        current.$automations.dropFirst().sink { _ in
            currentPublishes += 1
            currentDerivedRows += deriver.sections(input: fixture.input).totalRows
        }.store(in: &currentCancellables)

        current.applyCurrent(
            agents: fixture.agents,
            teams: fixture.teams,
            automations: fixture.automations
        )

        XCTAssertEqual(currentPublishes, 3)
        XCTAssertEqual(currentDerivedRows, 150)

        let target = DiagnosticAgentTargetPublicationBox(
            agents: fixture.agents,
            teams: fixture.teams,
            automations: fixture.automations
        )
        var targetPublishes = 0
        var targetDerivedRows = 0
        var targetCancellables: Set<AnyCancellable> = []
        target.$agents.dropFirst().sink { _ in
            targetPublishes += 1
            targetDerivedRows += deriver.sections(input: fixture.input).totalRows
        }.store(in: &targetCancellables)
        target.$teams.dropFirst().sink { _ in
            targetPublishes += 1
            targetDerivedRows += deriver.sections(input: fixture.input).totalRows
        }.store(in: &targetCancellables)
        target.$automations.dropFirst().sink { _ in
            targetPublishes += 1
            targetDerivedRows += deriver.sections(input: fixture.input).totalRows
        }.store(in: &targetCancellables)

        target.applyDeduped(
            agents: fixture.agents,
            teams: fixture.teams,
            automations: fixture.automations
        )

        XCTAssertEqual(targetPublishes, 0)
        XCTAssertEqual(targetDerivedRows, 0)
    }

    func testIdenticalThreadRefreshIsAlreadyGuardedBeforePublication() {
        let fixture = DiagnosticHomeThreadFixture.make()
        let box = DiagnosticThreadPublicationBox(threads: fixture.threads)
        var publishes = 0
        var cancellables: Set<AnyCancellable> = []
        box.$threads.dropFirst().sink { _ in
            publishes += 1
        }.store(in: &cancellables)

        box.applyCurrentRefresh(nextThreads: fixture.threads)
        XCTAssertEqual(publishes, 0)

        var changed = fixture.threads
        changed[0].title = "Changed Synthetic Title"
        box.applyCurrentRefresh(nextThreads: changed)
        XCTAssertEqual(publishes, 1)
    }

    func testWidgetSnapshotProjectionStillRunsOnEveryIdenticalThreadRefresh() {
        let fixture = DiagnosticHomeThreadFixture.make()
        let deriver = DiagnosticWidgetSnapshotDeriver()

        let snapshot = deriver.widgetThreads(input: fixture.input)
        XCTAssertEqual(snapshot.count, 50)

        let timing = DiagnosticTiming.measure(iterations: 5_000) {
            deriver.widgetThreads(input: fixture.input).count
        }
        print("DIAGNOSTIC widget_snapshot_projection_avg_ms=\(timing.averageMilliseconds)")

        var measuredRows = 0
        measure(metrics: [XCTClockMetric()]) {
            measuredRows = deriver.widgetThreads(input: fixture.input).count
        }
        XCTAssertEqual(measuredRows, 50)
    }

    func testSelectionChangeRebuildsAllRowsEvenThoughOnlyTwoRowsChangeSemantically() {
        let fixture = DiagnosticHomeThreadFixture.make()
        let deriver = DiagnosticHomeThreadSectionDeriver()

        var beforeInput = fixture.input
        beforeInput.selectedThreadId = "thread-010"
        var afterInput = fixture.input
        afterInput.selectedThreadId = "thread-011"

        let beforeRows = deriver.sections(input: beforeInput).allRows
        let afterRows = deriver.sections(input: afterInput).allRows
        let changedRows = zip(beforeRows, afterRows).filter { $0 != $1 }.count

        XCTAssertEqual(beforeRows.count, 50)
        XCTAssertEqual(afterRows.count, 50)
        XCTAssertEqual(changedRows, 2)
    }

    func testBackgroundReconcileCadenceCallsThreadRefreshFortyTimesPerMinute() {
        let gateway = DiagnosticCountingThreadRefreshGateway()
        let loop = DiagnosticBackgroundReconcileLoop(intervalSeconds: 1.5)

        loop.run(forSeconds: 6.0, gateway: gateway)

        XCTAssertEqual(gateway.refreshThreadsCallCount, 4)
        XCTAssertEqual(loop.refreshesPerMinute, 40.0, accuracy: 0.001)
    }

    func testOptimizedBackgroundReconcileWouldSkipRefreshesWhileThreadListInteracting() {
        let gateway = DiagnosticCountingThreadRefreshGateway()
        let loop = DiagnosticBackgroundReconcileLoop(intervalSeconds: 1.5)

        loop.run(
            forSeconds: 6.0,
            gateway: gateway,
            isThreadListInteracting: { _ in true }
        )

        XCTAssertEqual(gateway.refreshThreadsCallCount, 0)
    }

    func testTypingBadgeCostAtHomeScale() {
        let fixture = DiagnosticHomeThreadFixture.make()
        let runningRows = fixture.threads.filter { $0.runState == "running" }.count

        XCTAssertEqual(runningRows, 12)
        XCTAssertEqual(
            DiagnosticTypingBadgeProbe.timelineInvalidationsPerSecond(runningRows: runningRows),
            360
        )
        XCTAssertEqual(
            DiagnosticTypingBadgeProbe.sinCallsPerSecond(runningRows: runningRows),
            1_080
        )

        let timing = DiagnosticTiming.measure(iterations: 10_000) {
            Int(DiagnosticTypingBadgeProbe.opacitySampleSum(runningRows: runningRows, frames: 30))
        }
        print("DIAGNOSTIC typing_badge_12_rows_30fps_math_avg_ms=\(timing.averageMilliseconds)")

        var measured = 0
        measure(metrics: [XCTClockMetric()]) {
            measured = Int(DiagnosticTypingBadgeProbe.opacitySampleSum(runningRows: runningRows, frames: 30))
        }
        XCTAssertGreaterThan(measured, 0)
    }
}

private struct DiagnosticHomeThreadSectionsInput: Equatable {
    var threads: [GaryxThreadSummary]
    var pinnedThreadIds: [String]
    var recentThreadIds: [String]
    var selectedThreadId: String?
    var agents: [GaryxAgentSummary]
    var teams: [GaryxTeamSummary]
    var automations: [GaryxAutomationSummary]
    var busyThreadIds: Set<String>
    var relativeTimeBucket: Int
}

private struct DiagnosticHomeThreadSections: Equatable {
    var pinned: [DiagnosticHomeThreadRow] = []
    var recent: [DiagnosticHomeThreadRow] = []

    var totalRows: Int {
        pinned.count + recent.count
    }

    var allRows: [DiagnosticHomeThreadRow] {
        pinned + recent
    }
}

private struct DiagnosticHomeThreadRow: Equatable, Identifiable {
    let id: String
    let thread: GaryxThreadSummary
    let presentation: GaryxSidebarThreadRowPresentation
    let avatar: DiagnosticHomeThreadRowAvatar
    let canArchive: Bool
    let showsDivider: Bool
}

private struct DiagnosticHomeThreadRowAvatar: Equatable {
    let agentId: String
    let avatarDataUrl: String
    let kind: GaryxMobileAgentTarget.Kind
    let label: String
    let providerType: String
    let builtIn: Bool
}

private final class DiagnosticHomeThreadSectionDeriver {
    func sections(input: DiagnosticHomeThreadSectionsInput) -> DiagnosticHomeThreadSections {
        var threadsById: [String: GaryxThreadSummary] = [:]
        for thread in input.threads where threadsById[thread.id] == nil {
            threadsById[thread.id] = thread
        }

        let pinnedIds = normalizedThreadIds(input.pinnedThreadIds)
        let pinnedIdSet = Set(pinnedIds)

        var teamsById: [String: GaryxTeamSummary] = [:]
        for team in input.teams where teamsById[team.id] == nil {
            teamsById[team.id] = team
        }

        var agentsById: [String: GaryxAgentSummary] = [:]
        for agent in input.agents where agentsById[agent.id] == nil {
            agentsById[agent.id] = agent
        }

        let automationThreadIds = Set(input.automations.compactMap { automation -> String? in
            let threadId = (automation.targetThreadId ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            return threadId.isEmpty ? nil : threadId
        })

        let pinnedRows = pinnedIds
            .compactMap { threadsById[$0] }
            .enumerated()
            .map { index, thread in
                row(
                    thread: thread,
                    input: input,
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
                    input: input,
                    isPinned: false,
                    showsDivider: index > 0,
                    teamsById: teamsById,
                    agentsById: agentsById,
                    automationThreadIds: automationThreadIds
                )
            }

        return DiagnosticHomeThreadSections(pinned: pinnedRows, recent: recentRows)
    }

    private func row(
        thread: GaryxThreadSummary,
        input: DiagnosticHomeThreadSectionsInput,
        isPinned: Bool,
        showsDivider: Bool,
        teamsById: [String: GaryxTeamSummary],
        agentsById: [String: GaryxAgentSummary],
        automationThreadIds: Set<String>
    ) -> DiagnosticHomeThreadRow {
        let identity = identity(for: thread, teamsById: teamsById, agentsById: agentsById)
        let timestamp = Self.formattedTimestamp(
            thread.updatedAt ?? thread.createdAt,
            relativeTimeBucket: input.relativeTimeBucket
        )

        return DiagnosticHomeThreadRow(
            id: thread.id,
            thread: thread,
            presentation: GaryxSidebarThreadRowPresentation(
                thread: thread,
                isSelected: input.selectedThreadId == thread.id,
                isPinned: isPinned,
                trailingTimestamp: timestamp
            ),
            avatar: DiagnosticHomeThreadRowAvatar(
                agentId: identity.id ?? "",
                avatarDataUrl: identity.avatarDataUrl ?? "",
                kind: identity.isTeam ? .team : .agent,
                label: identity.name ?? thread.title,
                providerType: identity.providerType ?? "",
                builtIn: identity.builtIn
            ),
            canArchive: !input.busyThreadIds.contains(thread.id) && !automationThreadIds.contains(thread.id),
            showsDivider: showsDivider
        )
    }

    private func identity(
        for thread: GaryxThreadSummary,
        teamsById: [String: GaryxTeamSummary],
        agentsById: [String: GaryxAgentSummary]
    ) -> DiagnosticWidgetAgentIdentity {
        let teamId = thread.teamId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !teamId.isEmpty {
            if let team = teamsById[teamId] {
                return DiagnosticWidgetAgentIdentity(
                    id: team.id,
                    name: team.displayName,
                    avatarDataUrl: team.avatarDataUrl.isEmpty ? nil : team.avatarDataUrl,
                    providerType: nil,
                    isTeam: true,
                    builtIn: false
                )
            }
            return DiagnosticWidgetAgentIdentity(
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
                return DiagnosticWidgetAgentIdentity(
                    id: agent.id,
                    name: agent.displayName,
                    avatarDataUrl: agent.avatarDataUrl.isEmpty ? nil : agent.avatarDataUrl,
                    providerType: agent.providerType,
                    isTeam: false,
                    builtIn: agent.builtIn
                )
            }
            return DiagnosticWidgetAgentIdentity(
                id: agentId,
                name: nil,
                avatarDataUrl: nil,
                providerType: thread.providerType,
                isTeam: false,
                builtIn: false
            )
        }

        return DiagnosticWidgetAgentIdentity(
            id: nil,
            name: nil,
            avatarDataUrl: nil,
            providerType: thread.providerType,
            isTeam: false,
            builtIn: false
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
        return normalized
    }

    private static func formattedTimestamp(_ value: String?, relativeTimeBucket: Int) -> String {
        guard let value, let date = iso8601Date(from: value) else {
            return ""
        }
        let now = Date(timeIntervalSince1970: TimeInterval(relativeTimeBucket))
        let diff = max(0, now.timeIntervalSince(date))
        let minutes = Int(diff / 60)
        let hours = Int(diff / 3_600)
        let days = Int(diff / 86_400)
        let months = days / 30
        if minutes < 1 { return "now" }
        if minutes < 60 { return "\(minutes)m" }
        if hours < 24 { return "\(hours)h" }
        if days < 30 { return "\(days)d" }
        if months < 12 { return "\(months)mo" }
        return "\(days / 365)y"
    }

    private static func iso8601Date(from value: String) -> Date? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        let cacheKey = trimmed as NSString
        if let cached = iso8601DateCache.object(forKey: cacheKey) {
            return cached as Date
        }
        let parsed = iso8601FractionalFormatter.date(from: trimmed)
            ?? iso8601StandardFormatter.date(from: trimmed)
        if let parsed {
            iso8601DateCache.setObject(parsed as NSDate, forKey: cacheKey)
        }
        return parsed
    }

    private static let iso8601FractionalFormatter: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter
    }()

    private static let iso8601StandardFormatter: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        return formatter
    }()

    private static let iso8601DateCache: NSCache<NSString, NSDate> = {
        let cache = NSCache<NSString, NSDate>()
        cache.countLimit = 4096
        return cache
    }()
}

private final class DiagnosticHomeThreadSectionCache {
    private var previousInput: DiagnosticHomeThreadSectionsInput?
    private var previousSections: DiagnosticHomeThreadSections?
    private let deriver = DiagnosticHomeThreadSectionDeriver()
    private(set) var derivationCount = 0

    func sections(input: DiagnosticHomeThreadSectionsInput) -> DiagnosticHomeThreadSections {
        if input == previousInput, let previousSections {
            return previousSections
        }
        let next = deriver.sections(input: input)
        previousInput = input
        previousSections = next
        derivationCount += 1
        return next
    }
}

private final class DiagnosticWidgetSnapshotDeriver {
    func widgetThreads(input: DiagnosticHomeThreadSectionsInput) -> [GaryxMobileWidgetThread] {
        var summariesById: [String: GaryxThreadSummary] = [:]
        for thread in input.threads where summariesById[thread.id] == nil {
            summariesById[thread.id] = thread
        }

        let orderedThreadIds = normalizedThreadIds(input.pinnedThreadIds + input.recentThreadIds)
        return orderedThreadIds.compactMap { threadId -> GaryxMobileWidgetThread? in
            guard let thread = summariesById[threadId] else { return nil }
            let workspaceName = thread.workspacePath?
                .garyxLastPathComponent
                .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            let identity = widgetAgentIdentity(for: thread, agents: input.agents, teams: input.teams)
            return GaryxMobileWidgetThread(
                id: thread.id,
                title: thread.title,
                workspaceName: workspaceName,
                updatedAt: thread.updatedAt ?? thread.createdAt,
                activeRunId: thread.activeRunId,
                runState: thread.runState,
                agentId: identity.id,
                agentName: identity.name,
                avatarDataUrl: identity.avatarDataUrl,
                providerType: identity.providerType,
                isTeam: identity.isTeam,
                builtIn: identity.builtIn
            )
        }
    }

    private func widgetAgentIdentity(
        for thread: GaryxThreadSummary,
        agents: [GaryxAgentSummary],
        teams: [GaryxTeamSummary]
    ) -> DiagnosticWidgetAgentIdentity {
        let teamId = thread.teamId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !teamId.isEmpty {
            if let team = teams.first(where: { $0.id == teamId }) {
                return DiagnosticWidgetAgentIdentity(
                    id: team.id,
                    name: team.displayName,
                    avatarDataUrl: team.avatarDataUrl.isEmpty ? nil : team.avatarDataUrl,
                    providerType: nil,
                    isTeam: true,
                    builtIn: false
                )
            }
            return DiagnosticWidgetAgentIdentity(
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
            if let agent = agents.first(where: { $0.id == agentId }) {
                return DiagnosticWidgetAgentIdentity(
                    id: agent.id,
                    name: agent.displayName,
                    avatarDataUrl: agent.avatarDataUrl.isEmpty ? nil : agent.avatarDataUrl,
                    providerType: agent.providerType,
                    isTeam: false,
                    builtIn: agent.builtIn
                )
            }
            return DiagnosticWidgetAgentIdentity(
                id: agentId,
                name: nil,
                avatarDataUrl: nil,
                providerType: thread.providerType,
                isTeam: false,
                builtIn: false
            )
        }

        return DiagnosticWidgetAgentIdentity(
            id: nil,
            name: nil,
            avatarDataUrl: nil,
            providerType: thread.providerType,
            isTeam: false,
            builtIn: false
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
        return normalized
    }
}

private struct DiagnosticWidgetAgentIdentity: Equatable {
    var id: String?
    var name: String?
    var avatarDataUrl: String?
    var providerType: String?
    var isTeam: Bool
    var builtIn: Bool
}

private final class DiagnosticAgentTargetPublicationBox: ObservableObject {
    @Published var agents: [GaryxAgentSummary]
    @Published var teams: [GaryxTeamSummary]
    @Published var automations: [GaryxAutomationSummary]

    init(
        agents: [GaryxAgentSummary],
        teams: [GaryxTeamSummary],
        automations: [GaryxAutomationSummary]
    ) {
        self.agents = agents
        self.teams = teams
        self.automations = automations
    }

    func applyCurrent(
        agents nextAgents: [GaryxAgentSummary],
        teams nextTeams: [GaryxTeamSummary],
        automations nextAutomations: [GaryxAutomationSummary]
    ) {
        agents = nextAgents
        teams = nextTeams
        automations = nextAutomations
    }

    func applyDeduped(
        agents nextAgents: [GaryxAgentSummary],
        teams nextTeams: [GaryxTeamSummary],
        automations nextAutomations: [GaryxAutomationSummary]
    ) {
        if agents != nextAgents {
            agents = nextAgents
        }
        if teams != nextTeams {
            teams = nextTeams
        }
        if automations != nextAutomations {
            automations = nextAutomations
        }
    }
}

private final class DiagnosticThreadPublicationBox: ObservableObject {
    @Published var threads: [GaryxThreadSummary]

    init(threads: [GaryxThreadSummary]) {
        self.threads = threads
    }

    func applyCurrentRefresh(nextThreads: [GaryxThreadSummary]) {
        if threads != nextThreads {
            threads = nextThreads
        }
    }
}

private final class DiagnosticCountingThreadRefreshGateway {
    private(set) var refreshThreadsCallCount = 0

    func refreshThreads() {
        refreshThreadsCallCount += 1
    }
}

private struct DiagnosticBackgroundReconcileLoop {
    let intervalSeconds: TimeInterval

    var refreshesPerMinute: Double {
        60.0 / intervalSeconds
    }

    func run(
        forSeconds duration: TimeInterval,
        gateway: DiagnosticCountingThreadRefreshGateway,
        isThreadListInteracting: (TimeInterval) -> Bool = { _ in false }
    ) {
        var elapsed = intervalSeconds
        while elapsed <= duration + 0.000_001 {
            if !isThreadListInteracting(elapsed) {
                gateway.refreshThreads()
            }
            elapsed += intervalSeconds
        }
    }
}

private enum DiagnosticTypingBadgeProbe {
    static func timelineInvalidationsPerSecond(runningRows: Int) -> Int {
        runningRows * 30
    }

    static func sinCallsPerSecond(runningRows: Int) -> Int {
        timelineInvalidationsPerSecond(runningRows: runningRows) * 3
    }

    static func opacitySampleSum(runningRows: Int, frames: Int) -> Double {
        guard runningRows > 0, frames > 0 else { return 0 }
        let cycle = 1.05
        var sum = 0.0
        for frame in 0..<frames {
            let timestamp = Double(frame) / 30.0
            let progress = timestamp.truncatingRemainder(dividingBy: cycle) / cycle
            for _ in 0..<runningRows {
                for index in 0..<3 {
                    let phase = progress * 2 * .pi - Double(index) * (.pi / 4)
                    sum += 0.35 + 0.65 * max(0, sin(phase))
                }
            }
        }
        return sum
    }
}

private struct DiagnosticTiming {
    let averageMilliseconds: Double

    static func measure(iterations: Int, _ operation: () -> Int) -> DiagnosticTiming {
        var accumulator = 0
        let start = CFAbsoluteTimeGetCurrent()
        for _ in 0..<iterations {
            accumulator &+= operation()
        }
        let elapsed = CFAbsoluteTimeGetCurrent() - start
        XCTAssertNotEqual(accumulator, Int.min)
        return DiagnosticTiming(averageMilliseconds: elapsed * 1_000.0 / Double(iterations))
    }
}

private struct DiagnosticHomeThreadFixture {
    let threads: [GaryxThreadSummary]
    let pinnedThreadIds: [String]
    let recentThreadIds: [String]
    let agents: [GaryxAgentSummary]
    let teams: [GaryxTeamSummary]
    let automations: [GaryxAutomationSummary]
    let busyThreadIds: Set<String>
    let relativeTimeBucket: Int

    var input: DiagnosticHomeThreadSectionsInput {
        DiagnosticHomeThreadSectionsInput(
            threads: threads,
            pinnedThreadIds: pinnedThreadIds,
            recentThreadIds: recentThreadIds,
            selectedThreadId: "thread-004",
            agents: agents,
            teams: teams,
            automations: automations,
            busyThreadIds: busyThreadIds,
            relativeTimeBucket: relativeTimeBucket
        )
    }

    static func make() -> DiagnosticHomeThreadFixture {
        let relativeTimeBucket = 1_800_000_000
        let now = Date(timeIntervalSince1970: TimeInterval(relativeTimeBucket))
        let agents = makeAgents(count: 72)
        let teams = makeTeams(count: 28)
        let threads = makeThreads(
            count: 50,
            agents: agents,
            teams: teams,
            now: now
        )
        let pinnedThreadIds = (0..<8).map { threadId($0) }
        let recentThreadIds = (0..<50).map { threadId($0) }
        let automations = makeAutomations(count: 24, threadCount: threads.count)
        let busyThreadIds = Set((0..<12).map { threadId($0) })

        return DiagnosticHomeThreadFixture(
            threads: threads,
            pinnedThreadIds: pinnedThreadIds,
            recentThreadIds: recentThreadIds,
            agents: agents,
            teams: teams,
            automations: automations,
            busyThreadIds: busyThreadIds,
            relativeTimeBucket: relativeTimeBucket
        )
    }

    private static func makeAgents(count: Int) -> [GaryxAgentSummary] {
        (0..<count).map { index in
            GaryxAgentSummary(
                id: agentId(index),
                displayName: "Test Agent \(index)",
                providerType: index.isMultiple(of: 2) ? "codex" : "claude",
                model: "test-model-\(index % 5)",
                defaultWorkspaceDir: "/Users/test/workspace-\(index % 6)",
                avatarDataUrl: "",
                builtIn: index < 4
            )
        }
    }

    private static func makeTeams(count: Int) -> [GaryxTeamSummary] {
        (0..<count).map { index in
            GaryxTeamSummary(
                id: teamId(index),
                displayName: "Test Team \(index)",
                leaderAgentId: agentId(index),
                memberAgentIds: [
                    agentId(index),
                    agentId(index + 1),
                    agentId(index + 2),
                ],
                avatarDataUrl: ""
            )
        }
    }

    private static func makeThreads(
        count: Int,
        agents: [GaryxAgentSummary],
        teams: [GaryxTeamSummary],
        now: Date
    ) -> [GaryxThreadSummary] {
        (0..<count).map { index in
            let usesTeam = index.isMultiple(of: 5)
            let agent = agents[index % agents.count]
            let team = teams[index % teams.count]
            return GaryxThreadSummary(
                id: threadId(index),
                title: "Synthetic Thread \(index)",
                createdAt: iso8601String(from: now.addingTimeInterval(Double(-(index + 80) * 600))),
                updatedAt: iso8601String(from: now.addingTimeInterval(Double(-(index + 1) * 180))),
                lastMessagePreview: "Synthetic preview text for row \(index) with enough words to exercise compaction.",
                workspacePath: "/Users/test/workspace-\(index % 6)",
                messageCount: 10 + index,
                agentId: usesTeam ? nil : agent.id,
                teamId: usesTeam ? team.id : nil,
                teamName: usesTeam ? team.displayName : nil,
                providerType: agent.providerType,
                recentRunId: index < 12 ? "run-\(index)" : nil,
                activeRunId: index < 12 ? "run-\(index)" : nil,
                runState: index < 12 ? "running" : "completed",
                worktreePath: "/Users/test/workspace-\(index % 6)"
            )
        }
    }

    private static func makeAutomations(count: Int, threadCount: Int) -> [GaryxAutomationSummary] {
        (0..<count).map { index in
            GaryxAutomationSummary(
                id: "automation-\(index)",
                label: "Synthetic Automation \(index)",
                prompt: "Summarize synthetic updates.",
                agentId: agentId(index),
                workspacePath: "/Users/test/workspace-\(index % 6)",
                targetThreadId: threadId((index * 2) % threadCount),
                nextRun: "2027-01-15T09:00:00Z"
            )
        }
    }

    private static func threadId(_ index: Int) -> String {
        String(format: "thread-%03d", index)
    }

    private static func agentId(_ index: Int) -> String {
        String(format: "agent-%03d", index)
    }

    private static func teamId(_ index: Int) -> String {
        String(format: "team-%03d", index)
    }

    private static func iso8601String(from date: Date) -> String {
        iso8601Formatter.string(from: date)
    }

    private static let iso8601Formatter: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        return formatter
    }()
}
