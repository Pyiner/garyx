import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
    @discardableResult
    func applyAgentTargets(
        agents nextAgents: [GaryxAgentSummary]?,
        teams nextTeams: [GaryxTeamSummary]?
    ) -> Bool {
        var didUpdateTargets = false
        let receivedTargets = nextAgents != nil || nextTeams != nil
        if let nextAgents {
            didUpdateTargets = GaryxEquatableAssignment.assignIfChanged(
                current: agents,
                next: nextAgents
            ) { agents = $0 } || didUpdateTargets
        }
        if let nextTeams {
            didUpdateTargets = GaryxEquatableAssignment.assignIfChanged(
                current: teams,
                next: nextTeams
            ) { teams = $0 } || didUpdateTargets
        }
        if receivedTargets, agentTargetsLoadPhase != .loaded {
            agentTargetsLoadPhase = .loaded
        }
        if receivedTargets {
            ensureSelectedAgentTarget()
        }
        if didUpdateTargets {
            if !threads.isEmpty {
                persistRecentThreadsWidgetSnapshot()
            }
        }
        return didUpdateTargets
    }

    func ensureSelectedAgentTarget() {
        let targets = agentTargets
        if targets.contains(where: { $0.id == selectedAgentTargetId }) {
            return
        }
        if let first = targets.first {
            setSelectedAgentTarget(first.id)
        }
    }

    func ensureSelectedWorkspace() {
        let paths = userWorkspacePaths
        let selected = selectedWorkspacePath.trimmingCharacters(in: .whitespacesAndNewlines)
        if !selected.isEmpty {
            selectedWorkspacePath = selected
            draftWorkspacePath = selected
            return
        }
        selectedWorkspacePath = paths.first ?? ""
        draftWorkspacePath = selectedWorkspacePath
    }

    func mergeMissingSidebarRequiredThreads(
        using gatewayClient: GaryxGatewayClient,
        extraThreadIds: [String?] = [],
        runtimeGeneration: UUID? = nil,
        remoteStateRefreshRequestId: UUID? = nil
    ) async {
        let observedGeneration = runtimeGeneration ?? gatewayRuntimeGeneration
        let requiredThreadIds = sidebarRequiredThreadIds(
            pinnedThreadIds: pinnedThreadIds,
            extraThreadIds: extraThreadIds
        )
        let missingThreads = await fetchMissingThreadSummaries(
            using: gatewayClient,
            requiredThreadIds: requiredThreadIds,
            existingThreadIds: Set(threads.map(\.id))
        )
        guard observedGeneration == gatewayRuntimeGeneration,
              isCurrentRemoteStateScopedRequest(remoteStateRefreshRequestId) else {
            return
        }
        if !missingThreads.isEmpty {
            threads = Self.mergedThreadSummaries(threads + missingThreads)
        }
    }

    func isCurrentRemoteStateScopedRequest(_ requestId: UUID?) -> Bool {
        guard let requestId else { return true }
        return remoteStateRefreshRequestId == requestId
    }

    func fetchMissingThreadSummaries(
        using gatewayClient: GaryxGatewayClient,
        requiredThreadIds: [String],
        existingThreadIds: Set<String>
    ) async -> [GaryxThreadSummary] {
        var visibleThreadIds = existingThreadIds
        var missingThreads: [GaryxThreadSummary] = []
        for threadId in requiredThreadIds where !visibleThreadIds.contains(threadId) {
            if let thread = try? await gatewayClient.getThread(threadId: threadId) {
                missingThreads.append(thread)
                visibleThreadIds.insert(thread.id)
            }
        }
        return missingThreads
    }

    func normalizedThreadIds(_ values: [String?]) -> [String] {
        var seen = Set<String>()
        return values.compactMap { value -> String? in
            let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            guard !trimmed.isEmpty, seen.insert(trimmed).inserted else { return nil }
            return trimmed
        }
    }

    func sidebarRequiredThreadIds(
        pinnedThreadIds: [String],
        extraThreadIds: [String?] = []
    ) -> [String] {
        var seen = Set<String>()
        var ids: [String] = []

        func append(_ value: String?) {
            let trimmed = value?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            guard !trimmed.isEmpty, seen.insert(trimmed).inserted else { return }
            ids.append(trimmed)
        }

        pinnedThreadIds.forEach { append($0) }
        extraThreadIds.forEach { append($0) }
        channelEndpoints.forEach { append($0.threadId) }
        configuredBots.forEach { bot in
            append(bot.mainThreadId)
            append(bot.defaultOpenThreadId)
        }
        botConsoles.forEach { console in
            append(console.mainThreadId)
            append(console.defaultOpenThreadId)
            console.conversationNodes.forEach { append($0.endpoint.threadId) }
        }

        return ids
    }

    static func mergedThreadSummaries(_ values: [GaryxThreadSummary]) -> [GaryxThreadSummary] {
        var indexesById: [String: Int] = [:]
        var merged: [GaryxThreadSummary] = []
        for value in values {
            guard !value.id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                continue
            }
            if let index = indexesById[value.id] {
                var next = value
                if next.threadRuntime == nil {
                    next.threadRuntime = merged[index].threadRuntime
                }
                merged[index] = next
            } else {
                indexesById[value.id] = merged.count
                merged.append(value)
            }
        }
        return merged
    }
}
