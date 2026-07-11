import Foundation
@testable import GaryxMobileCore

// MARK: - Diagnostic support for TASK-1037 (iOS home-list scroll jank)
//
// The home thread list derives its rows from `GaryxMobileModel.homeThreadSections`,
// a `computed property` that currently lives in the App target
// (`App/GaryxMobile/GaryxMobileSidebarViews.swift`). Because it is a private
// extension on an `@EnvironmentObject` model it is NOT unit-testable today.
//
// This file is a FAITHFUL 1:1 PORT of that derivation (homeThreadSections +
// homeThreadRow + homeThreadIdentity + the per-row presentation/avatar/timestamp
// construction) onto the Core types it already consumes. It exists only in the
// test target — no product code is changed. Porting it here is itself part of
// the diagnosis/design: it proves the derivation is a pure function of Core
// inputs and can be sunk into `GaryxMobileCore` behind an Equatable input gate.
//
// Source of truth at port time (read in this investigation):
//   - homeThreadSections      GaryxMobileSidebarViews.swift:324-377
//   - homeThreadRow           GaryxMobileSidebarViews.swift:379-410
//   - homeThreadIdentity      GaryxMobileSidebarViews.swift:412-469
//   - presentation init       Sources/.../GaryxMobilePresentationModels.swift:42 (reused as-is)
//   - timestamp formatting    GaryxMobileDesignSystem.swift:407-422 (ported with injectable `now`)
//   - normalizedPinnedThreadIds GaryxMobileModel+Threads.swift:157

// MARK: Ported view-model row/section types

/// Port of the App-target `GaryxMobileModel.WidgetAgentIdentity`.
struct RefAgentIdentity: Equatable {
    var id: String?
    var name: String?
    var avatarDataUrl: String?
    var providerType: String?
    var builtIn: Bool
}

/// Port of the App-target `GaryxSidebarThreadRowAvatar`.
struct RefThreadRowAvatar: Equatable {
    let agentId: String
    let avatarDataUrl: String
    let label: String
    let providerType: String
    let builtIn: Bool
}

/// Port of the App-target private `GaryxHomeThreadRow`. Made `Equatable` so the
/// diagnostic can assert derivation purity and cache correctness. Reuses the
/// Core `GaryxSidebarThreadRowPresentation` exactly as the App target does.
struct RefHomeThreadRow: Identifiable, Equatable {
    let id: String
    let presentation: GaryxSidebarThreadRowPresentation
    let avatar: RefThreadRowAvatar
    let canArchive: Bool
    let showsDivider: Bool
}

/// Port of the App-target private `GaryxHomeThreadSections`.
struct RefHomeThreadSections: Equatable {
    var pinned: [RefHomeThreadRow] = []
    var recent: [RefHomeThreadRow] = []

    var rowCount: Int { pinned.count + recent.count }
}

// MARK: Ported derivation

enum HomeThreadSectionsReference {
    /// The exact set of `@Published` model fields the App-target computed
    /// property reads. `busyThreadIds` mirrors `isThreadBusy(_:)`
    /// (`runTracker.isThreadBusy || runStateByThread[id]?.busy`).
    struct Inputs: Equatable {
        var threads: [GaryxThreadSummary]
        var agents: [GaryxAgentSummary]
        var automations: [GaryxAutomationSummary]
        var pinnedThreadIds: [String]
        var recentThreadIds: [String]
        var selectedThreadId: String?
        var busyThreadIds: Set<String>
        /// Wall-clock used by the relative-time formatter. Excluded from the
        /// section-identity key (timestamps are a render-time concern).
        var now: Date
    }

    static func build(_ input: Inputs) -> RefHomeThreadSections {
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
                    isSelected: selectedThreadId == thread.id,
                    isPinned: true,
                    showsDivider: index > 0,
                    agentsById: agentsById,
                    automationThreadIds: automationThreadIds,
                    busyThreadIds: input.busyThreadIds,
                    now: input.now
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
                    automationThreadIds: automationThreadIds,
                    busyThreadIds: input.busyThreadIds,
                    now: input.now
                )
            }

        return RefHomeThreadSections(pinned: pinnedRows, recent: recentRows)
    }

    private static func row(
        thread: GaryxThreadSummary,
        isSelected: Bool,
        isPinned: Bool,
        showsDivider: Bool,
        agentsById: [String: GaryxAgentSummary],
        automationThreadIds: Set<String>,
        busyThreadIds: Set<String>,
        now: Date
    ) -> RefHomeThreadRow {
        let identity = self.identity(for: thread, agentsById: agentsById)
        let canArchive = !busyThreadIds.contains(thread.id) && !automationThreadIds.contains(thread.id)
        return RefHomeThreadRow(
            id: thread.id,
            presentation: GaryxSidebarThreadRowPresentation(
                thread: thread,
                isSelected: isSelected,
                isPinned: isPinned,
                trailingTimestamp: formattedTimestamp(thread.updatedAt ?? thread.createdAt, now: now)
            ),
            avatar: RefThreadRowAvatar(
                agentId: identity.id ?? "",
                avatarDataUrl: identity.avatarDataUrl ?? "",
                label: identity.name ?? thread.title,
                providerType: identity.providerType ?? "",
                builtIn: identity.builtIn
            ),
            canArchive: canArchive,
            showsDivider: showsDivider
        )
    }

    private static func identity(
        for thread: GaryxThreadSummary,
        agentsById: [String: GaryxAgentSummary]
    ) -> RefAgentIdentity {
        let agentId = thread.agentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !agentId.isEmpty {
            if let agent = agentsById[agentId] {
                return RefAgentIdentity(
                    id: agent.id,
                    name: agent.displayName,
                    avatarDataUrl: agent.avatarDataUrl.isEmpty ? nil : agent.avatarDataUrl,
                    providerType: agent.providerType,
                    builtIn: agent.builtIn
                )
            }
            return RefAgentIdentity(
                id: agentId,
                name: nil,
                avatarDataUrl: nil,
                providerType: thread.providerType,
                builtIn: false
            )
        }

        return RefAgentIdentity(
            id: nil,
            name: nil,
            avatarDataUrl: nil,
            providerType: thread.providerType,
            builtIn: false
        )
    }

    // Port of GaryxMobileModel.normalizedPinnedThreadIds.
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

    // Port of garyxFormattedTaskTimestamp with injectable `now`. Uses the same
    // shared-formatter + bounded parse cache strategy as the production code so
    // the measured per-row cost is representative (not inflated by per-row
    // formatter construction).
    static func formattedTimestamp(_ value: String?, now: Date) -> String {
        guard let value, let date = parsedISO8601(value) else { return "" }
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

    private static let iso8601: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter
    }()
    private static let iso8601Plain: ISO8601DateFormatter = {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        return formatter
    }()
    private static let parseCache: NSCache<NSString, NSDate> = {
        let cache = NSCache<NSString, NSDate>()
        cache.countLimit = 4096
        return cache
    }()

    private static func parsedISO8601(_ value: String) -> Date? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        let key = trimmed as NSString
        if let cached = parseCache.object(forKey: key) { return cached as Date }
        guard let parsed = iso8601.date(from: trimmed) ?? iso8601Plain.date(from: trimmed) else { return nil }
        parseCache.setObject(parsed as NSDate, forKey: key)
        return parsed
    }
}

// MARK: - Section-identity keys (proposed Equatable input gate)

/// The PROPOSED Equatable section-identity key. Captures everything that affects
/// row identity/order/content EXCEPT the volatile per-thread run state
/// (`runState`/`activeRunId`) and the wall clock (`now`). Run state is delivered
/// to the affected row as a row-scoped signal instead of being baked into the
/// section model, so run-state churn does not bust the section cache.
struct HomeSectionsIdentityKey: Equatable {
    struct ThreadIdentity: Equatable {
        let id: String
        let title: String
        let agentId: String?
        let providerType: String?
        let workspacePath: String?
        let lastMessagePreview: String
        let updatedAt: String?
        let createdAt: String?

        init(_ t: GaryxThreadSummary) {
            id = t.id
            title = t.title
            agentId = t.agentId
            providerType = t.providerType
            workspacePath = t.workspacePath
            lastMessagePreview = t.lastMessagePreview
            updatedAt = t.updatedAt
            createdAt = t.createdAt
        }
    }

    let threads: [ThreadIdentity]
    let agents: [GaryxAgentSummary]
    let automationThreadIds: Set<String>
    let pinnedThreadIds: [String]
    let recentThreadIds: [String]
    let selectedThreadId: String?

    init(_ input: HomeThreadSectionsReference.Inputs) {
        threads = input.threads.map(ThreadIdentity.init)
        agents = input.agents
        automationThreadIds = Set(input.automations.compactMap {
            let id = ($0.targetThreadId ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            return id.isEmpty ? nil : id
        })
        pinnedThreadIds = input.pinnedThreadIds
        recentThreadIds = input.recentThreadIds
        selectedThreadId = input.selectedThreadId
    }
}

/// A naive content key that bakes the full thread summaries (INCLUDING run
/// state) into the identity — this models what a cache keyed on the *current*
/// `threads` array would see, where every run-state delta busts the cache.
struct HomeSectionsNaiveKey: Equatable {
    let threads: [GaryxThreadSummary]
    let agents: [GaryxAgentSummary]
    let automations: [GaryxAutomationSummary]
    let pinnedThreadIds: [String]
    let recentThreadIds: [String]
    let selectedThreadId: String?
    let busyThreadIds: Set<String>

    init(_ input: HomeThreadSectionsReference.Inputs) {
        threads = input.threads
        agents = input.agents
        automations = input.automations
        pinnedThreadIds = input.pinnedThreadIds
        recentThreadIds = input.recentThreadIds
        selectedThreadId = input.selectedThreadId
        busyThreadIds = input.busyThreadIds
    }
}

/// The PROPOSED memoized section builder: recompute only when the Equatable
/// identity key changes. `computeCount` records genuine recomputes.
final class HomeThreadSectionsCache {
    private(set) var computeCount = 0
    private var key: HomeSectionsIdentityKey?
    private var cached = RefHomeThreadSections()

    func sections(for input: HomeThreadSectionsReference.Inputs) -> RefHomeThreadSections {
        let nextKey = HomeSectionsIdentityKey(input)
        if key == nextKey {
            return cached
        }
        computeCount += 1
        cached = HomeThreadSectionsReference.build(input)
        key = nextKey
        return cached
    }
}

// MARK: - Synthetic fixtures (public-repo safe placeholders only)

enum GaryxHomeListFixture {
    /// Build a realistic home-list state: pinned + recent threads, a catalog of
    /// agents, and automations. All data is synthetic.
    static func makeInputs(
        threadCount: Int = 50,
        agentCount: Int = 80,
        automationCount: Int = 25,
        pinnedCount: Int = 6,
        runningCount: Int = 4,
        now: Date = Date(timeIntervalSince1970: 1_750_000_000)
    ) -> HomeThreadSectionsReference.Inputs {
        let agents = (0..<agentCount).map { i in
            GaryxAgentSummary(
                id: "agent-\(i)",
                displayName: "Test Agent \(i)",
                providerType: ["claude_code", "codex_app_server", "google"][i % 3],
                model: "model-\(i % 5)",
                avatarDataUrl: i % 4 == 0 ? "data:image/png;base64,AAAA" : "",
                builtIn: i % 7 == 0
            )
        }
        let threads = (0..<threadCount).map { i -> GaryxThreadSummary in
            let isRunning = i < runningCount
            return GaryxThreadSummary(
                id: "thread-\(i)",
                title: "Conversation about topic number \(i)",
                createdAt: iso(now.addingTimeInterval(Double(-i) * 3_600)),
                updatedAt: iso(now.addingTimeInterval(Double(-i) * 600)),
                lastMessagePreview: "This is a representative multi-word last message preview for row \(i) with enough text to exercise the compacted-preview string work.",
                workspacePath: "/Users/test/workspaces/project-\(i % 12)",
                messageCount: 10 + i,
                agentId: "agent-\(i % max(1, agentCount))",
                providerType: ["claude_code", "codex_app_server", "google"][i % 3],
                recentRunId: "run-\(i)",
                activeRunId: isRunning ? "run-\(i)" : nil,
                runState: isRunning ? "running" : "idle",
                worktreePath: nil
            )
        }

        let pinnedThreadIds = (0..<min(pinnedCount, threadCount)).map { "thread-\($0)" }
        let recentThreadIds = (0..<threadCount).map { "thread-\($0)" }
        let busy = Set((0..<runningCount).map { "thread-\($0)" })
        let automations = (0..<automationCount).map { i in
            GaryxAutomationSummary(
                id: "automation-\(i)",
                label: "Automation \(i)",
                prompt: "Do the scheduled thing \(i)",
                agentId: "agent-\(i % max(1, agentCount))",
                workspacePath: "/Users/test/workspaces/project-\(i % 12)",
                targetThreadId: i % 2 == 0 ? "thread-\(threadCount - 1 - (i % threadCount))" : nil
            )
        }

        return HomeThreadSectionsReference.Inputs(
            threads: threads,
            agents: agents,
            automations: automations,
            pinnedThreadIds: pinnedThreadIds,
            recentThreadIds: recentThreadIds,
            selectedThreadId: "thread-1",
            busyThreadIds: busy,
            now: now
        )
    }

    private static let isoFormatter: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        return f
    }()

    static func iso(_ date: Date) -> String { isoFormatter.string(from: date) }
}

// MARK: - Timing helper

enum GaryxBench {
    /// Median wall-clock milliseconds of `iterations` runs of `body`.
    static func medianMillis(iterations: Int, _ body: () -> Void) -> Double {
        var samples: [Double] = []
        samples.reserveCapacity(iterations)
        for _ in 0..<iterations {
            let start = DispatchTime.now().uptimeNanoseconds
            body()
            let end = DispatchTime.now().uptimeNanoseconds
            samples.append(Double(end - start) / 1_000_000.0)
        }
        samples.sort()
        return samples[samples.count / 2]
    }
}
