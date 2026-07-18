import Foundation
import SwiftUI
import UniformTypeIdentifiers
import WidgetKit

extension GaryxMobileModel {
    @discardableResult
    func applyAgentCatalog(_ catalog: GaryxAgentsPage) -> Bool {
        let rawDefault = catalog.defaultAgentId?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let effectiveDefault = catalog.effectiveDefaultAgentId?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        gatewayDefaultAgentId = rawDefault.isEmpty ? nil : rawDefault
        effectiveDefaultAgentId = effectiveDefault.isEmpty ? nil : effectiveDefault
        return applyAgentTargets(agents: catalog.agents)
    }

    @discardableResult
    func applyAgentTargets(
        agents nextAgents: [GaryxAgentSummary]
    ) -> Bool {
        let didUpdateTargets = GaryxEquatableAssignment.assignIfChanged(
            current: agents,
            next: nextAgents
        ) { agents = $0 }
        if agentTargetsLoadPhase != .loaded {
            agentTargetsLoadPhase = .loaded
        }
        if didUpdateTargets {
            if !residentRecentThreadSummaries.isEmpty {
                persistRecentThreadsWidgetSnapshot()
            }
        }
        return didUpdateTargets
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
        runtimeGeneration: GaryxGatewayRequestToken? = nil,
        remoteStateRefreshRequestId: UUID? = nil
    ) async {
        let observedGeneration = runtimeGeneration ?? gatewayRequestToken
        let requiredThreadIds = sidebarRequiredThreadIds(
            pinnedThreadIds: pinnedThreadIds,
            extraThreadIds: extraThreadIds
        )
        let missingThreads = await fetchMissingThreadSummaries(
            using: gatewayClient,
            requiredThreadIds: requiredThreadIds,
            existingThreadIds: Set(
                requiredThreadIds.filter { threadSummaryCache.summary(for: $0) != nil }
            )
        )
        guard observedGeneration == gatewayRequestToken,
              isCurrentRemoteStateScopedRequest(remoteStateRefreshRequestId) else {
            return
        }
        if !missingThreads.isEmpty {
            cacheThreadSummaries(missingThreads)
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
