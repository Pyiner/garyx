import Foundation

/// One visible row of the conversation task-tree sidebar.
public struct GaryxTaskTreeRow: Equatable, Identifiable, Sendable {
    public enum Kind: Equatable, Sendable {
        case sourceThread
        case task
    }

    public var id: String
    public var kind: Kind
    public var threadId: String
    public var title: String
    /// "#TASK-n" for task rows; nil on the source-thread root row.
    public var taskDisplayId: String?
    public var status: GaryxTaskStatus?
    /// Identity hint for the shared avatar helpers: executor team/agent,
    /// else agent assignee, else runtime agent (task rows); thread agent id
    /// on the root row. Empty when unknown.
    public var identityAgentId: String
    public var identityIsTeam: Bool
    public var providerType: String
    /// Visual indent level, clamped at 4 like the Mac popover.
    public var indentLevel: Int
    public var isCurrent: Bool
    public var isRunning: Bool
}

/// Pure mapping from the anchored forest wire page to sidebar rows plus the
/// sidebar's availability/badge/tap policies. No SwiftUI, fully testable.
public enum GaryxTaskTreeSidebarPresentation {
    public static let maxIndentLevel = 4

    public static func rows(
        page: GaryxTaskForestPage,
        currentThreadId: String?
    ) -> [GaryxTaskTreeRow] {
        let nodes = page.nodes
        guard !nodes.isEmpty else { return [] }
        let hasServerLayout = nodes.allSatisfy { $0.depth != nil }
        if hasServerLayout {
            return nodes.map { node in
                row(for: node, depth: node.depth ?? 0, currentThreadId: currentThreadId)
            }
        }
        return fallbackRows(nodes: nodes, currentThreadId: currentThreadId)
    }

    /// Badge count: prefer the server page count, recount locally against an
    /// old gateway.
    public static func activeBadgeCount(page: GaryxTaskForestPage) -> Int {
        if let activeCount = page.activeCount {
            return activeCount
        }
        return page.nodes.reduce(into: 0) { count, node in
            if let task = node.taskNode,
               task.task.status == .inProgress || task.task.status == .inReview {
                count += 1
            }
        }
    }

    /// The sidebar (header button and edge gesture) exists only when the tree
    /// has at least one task node.
    public static func isSidebarAvailable(page: GaryxTaskForestPage) -> Bool {
        page.nodes.contains { $0.taskNode != nil }
    }

    /// Row tap policy: the current thread's row only closes the panel.
    public static func shouldNavigate(currentThreadId: String?, targetThreadId: String) -> Bool {
        !targetThreadId.isEmpty && targetThreadId != currentThreadId
    }

    /// Known-empty trees stop the 5s poll until the anchor changes or a local
    /// task mutation invalidates the snapshot; an unknown tree keeps polling.
    public static func shouldContinuePolling(page: GaryxTaskForestPage?) -> Bool {
        guard let page else { return true }
        return isSidebarAvailable(page: page)
    }

    private static func row(
        for node: GaryxTaskForestNode,
        depth: Int,
        currentThreadId: String?
    ) -> GaryxTaskTreeRow {
        let indent = min(max(depth, 0), maxIndentLevel)
        switch node {
        case .thread(let thread):
            let isCurrent = isCurrent(threadId: thread.threadId, currentThreadId: currentThreadId)
            return GaryxTaskTreeRow(
                id: thread.nodeId,
                kind: .sourceThread,
                threadId: thread.threadId,
                title: thread.title,
                taskDisplayId: nil,
                status: nil,
                identityAgentId: thread.agentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? "",
                identityIsTeam: false,
                providerType: thread.providerType ?? "",
                indentLevel: indent,
                isCurrent: isCurrent,
                isRunning: isRunningState(thread.runState)
            )
        case .task(let node):
            let task = node.task
            let isCurrent = isCurrent(threadId: task.threadId, currentThreadId: currentThreadId)
            let identity = taskIdentity(task)
            return GaryxTaskTreeRow(
                id: node.nodeId,
                kind: .task,
                threadId: task.threadId,
                title: task.title,
                taskDisplayId: task.id.isEmpty ? "#TASK-\(task.number)" : task.id,
                status: task.status,
                identityAgentId: identity.agentId,
                identityIsTeam: identity.isTeam,
                providerType: "",
                indentLevel: indent,
                isCurrent: isCurrent,
                isRunning: isRunningState(node.runState)
            )
        }
    }

    /// Old-gateway fallback: rebuild the tree locally from `parentNodeId`.
    /// Orphan parents (parent id absent from the page) become roots; thread
    /// nodes sort before tasks and task siblings sort by number. Depth is the
    /// visual indent level: the thread root row and its top-level tasks both
    /// sit flush at 0 (the root row differs by styling, not indentation).
    private static func fallbackRows(
        nodes: [GaryxTaskForestNode],
        currentThreadId: String?
    ) -> [GaryxTaskTreeRow] {
        let ids = Set(nodes.map(\.nodeId))
        var originalIndex: [String: Int] = [:]
        for (index, node) in nodes.enumerated() {
            if originalIndex[node.nodeId] == nil {
                originalIndex[node.nodeId] = index
            }
        }
        var childrenByParent: [String: [GaryxTaskForestNode]] = [:]
        for node in nodes {
            let parent: String
            if let task = node.taskNode,
               let parentNodeId = task.parentNodeId,
               ids.contains(parentNodeId) {
                parent = parentNodeId
            } else {
                parent = ""
            }
            childrenByParent[parent, default: []].append(node)
        }
        for key in childrenByParent.keys {
            childrenByParent[key]?.sort { a, b in
                switch (a, b) {
                case (.thread, .task):
                    return true
                case (.task, .thread):
                    return false
                case (.task(let left), .task(let right)):
                    if left.task.number != right.task.number {
                        return left.task.number < right.task.number
                    }
                    return (originalIndex[a.nodeId] ?? 0) < (originalIndex[b.nodeId] ?? 0)
                case (.thread, .thread):
                    return (originalIndex[a.nodeId] ?? 0) < (originalIndex[b.nodeId] ?? 0)
                }
            }
        }

        var rows: [GaryxTaskTreeRow] = []
        var visited = Set<String>()
        func walk(parent: String, depth: Int) {
            for node in childrenByParent[parent] ?? [] {
                guard visited.insert(node.nodeId).inserted else { continue }
                rows.append(row(for: node, depth: depth, currentThreadId: currentThreadId))
                let childDepth = node.taskNode == nil ? depth : depth + 1
                walk(parent: node.nodeId, depth: childDepth)
            }
        }
        walk(parent: "", depth: 0)
        return rows
    }

    private static func isCurrent(threadId: String, currentThreadId: String?) -> Bool {
        guard let currentThreadId, !currentThreadId.isEmpty else { return false }
        return threadId == currentThreadId
    }

    private static func isRunningState(_ runState: String) -> Bool {
        runState.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() == "running"
    }

    private static func taskIdentity(_ task: GaryxTaskSummary) -> (agentId: String, isTeam: Bool) {
        if let executor = task.executor {
            if let teamId = executor.teamId?.trimmingCharacters(in: .whitespacesAndNewlines),
               !teamId.isEmpty {
                return (teamId, true)
            }
            if let agentId = executor.agentId?.trimmingCharacters(in: .whitespacesAndNewlines),
               !agentId.isEmpty {
                return (agentId, false)
            }
        }
        if let assignee = task.assignee, assignee.kind == "agent",
           let agentId = assignee.agentId?.trimmingCharacters(in: .whitespacesAndNewlines),
           !agentId.isEmpty {
            return (agentId, false)
        }
        let runtimeAgentId = task.runtimeAgentId.trimmingCharacters(in: .whitespacesAndNewlines)
        return (runtimeAgentId, false)
    }
}

/// Generation gate for forest loads: responses are accepted only when the
/// gateway + anchor they were requested for is still the active pair, so a
/// stale response from a previous thread or gateway can never overwrite the
/// current snapshot.
public struct GaryxTaskTreeRequestGate: Equatable, Sendable {
    public private(set) var generation = 0
    private var gatewayKey = ""
    private var anchorThreadId = ""

    public init() {}

    public mutating func begin(gatewayKey: String, anchorThreadId: String) -> Int {
        generation += 1
        self.gatewayKey = gatewayKey
        self.anchorThreadId = anchorThreadId
        return generation
    }

    public func accepts(token: Int, gatewayKey: String, anchorThreadId: String) -> Bool {
        token == generation
            && gatewayKey == self.gatewayKey
            && anchorThreadId == self.anchorThreadId
    }
}
