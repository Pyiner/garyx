import Foundation

/// Pure transcript merge for the iOS app: reconciles a fetched remote
/// transcript page with locally known messages (optimistic sends, streaming
/// partials, failure annotations, previously loaded older pages).
///
/// Implements the conversation state contract's reconciliation rules
/// (docs/agents/conversation-state.md, mirroring the desktop
/// `mergeRemoteTranscriptWithLocal`):
/// - provenance branches on `GaryxMobileMessage.localState`, never on id
///   prefixes;
/// - identity reuse: a remote row that materializes a known local message
///   keeps the local row's id, so list row identity stays stable and the
///   transcript does not flicker on reconcile;
/// - older loaded pages survive via `historyIndex` and
///   `preserveRemoteBeforeIndex`;
/// - local entries that the remote transcript does not cover yet
///   (optimistic sends, streaming partials, failure annotations) are
///   preserved.
enum GaryxTranscriptMerge {
    // MARK: Identity reuse

    /// Rewrites remote rows that materialize a known local message so they
    /// adopt the local row's id. Matching mirrors the legacy semantics:
    /// `clientIntentId` or `pendingInputId` first, then normalized user text
    /// for optimistic local sends. Each local row is consumed at most once.
    static func remoteReusingLocalIdentities(
        _ remoteMessages: [GaryxMobileMessage],
        local localMessages: [GaryxMobileMessage]
    ) -> [GaryxMobileMessage] {
        guard !remoteMessages.isEmpty, !localMessages.isEmpty else {
            return remoteMessages
        }
        let remoteIds = Set(remoteMessages.map(\.id))
        var consumedLocalIds = Set<String>()
        // Remote user occurrences already represented by a non-optimistic
        // local row (pending inputs, remote-derived rows) are claimed first;
        // only unclaimed occurrences may materialize an optimistic send by
        // text. Mirrors the merge's occurrence accounting below.
        var nonOptimisticUserClaims = Dictionary(
            grouping: localMessages
                .filter { $0.role == .user && $0.localState != .optimistic && !remoteIds.contains($0.id) }
                .map(userMergeKey),
            by: { $0 }
        )
        .mapValues(\.count)

        func reusableLocal(for remote: GaryxMobileMessage) -> GaryxMobileMessage? {
            let remoteClientIntentId = remote.clientIntentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            let remotePendingInputId = remote.pendingInputId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            if let identityMatch = localMessages.first(where: { local in
                guard !consumedLocalIds.contains(local.id),
                      local.id != remote.id,
                      !remoteIds.contains(local.id),
                      local.role == remote.role,
                      local.localState != .remoteFinal else {
                    return false
                }
                let localClientIntentId = local.clientIntentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
                let localPendingInputId = local.pendingInputId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
                if !remoteClientIntentId.isEmpty, localClientIntentId == remoteClientIntentId {
                    return true
                }
                return !remotePendingInputId.isEmpty && localPendingInputId == remotePendingInputId
            }) {
                return identityMatch
            }
            guard remote.role == .user else { return nil }
            let remoteKey = userMergeKey(remote)
            if let claims = nonOptimisticUserClaims[remoteKey], claims > 0 {
                nonOptimisticUserClaims[remoteKey] = claims - 1
                return nil
            }
            let remoteText = normalizedMergeText(remote.text)
            guard !remoteText.isEmpty else { return nil }
            return localMessages.first { local in
                !consumedLocalIds.contains(local.id)
                    && local.id != remote.id
                    && !remoteIds.contains(local.id)
                    && local.role == .user
                    && local.localState == .optimistic
                    && normalizedMergeText(local.text) == remoteText
            }
        }

        return remoteMessages.map { remote in
            guard remote.role == .user || remote.role == .assistant,
                  let local = reusableLocal(for: remote) else {
                return remote
            }
            consumedLocalIds.insert(local.id)
            var materialized = GaryxMobileMessage(
                id: local.id,
                role: remote.role,
                text: remote.text,
                attachments: remote.attachments.isEmpty ? local.attachments : remote.attachments,
                timestamp: remote.timestamp ?? local.timestamp,
                isStreaming: false,
                statusText: nil,
                clientIntentId: remote.clientIntentId ?? local.clientIntentId,
                pendingInputId: remote.pendingInputId ?? local.pendingInputId,
                localState: remote.localState ?? .remoteFinal,
                historyIndex: remote.historyIndex
            )
            materialized.remoteId = remote.id
            return materialized
        }
    }

    // MARK: Merge

    static func mergedMessages(
        _ incomingRemoteMessages: [GaryxMobileMessage],
        withLocal localMessages: [GaryxMobileMessage],
        preserveRemoteBeforeIndex: Int? = nil
    ) -> [GaryxMobileMessage] {
        guard !incomingRemoteMessages.isEmpty else {
            return localMessages
        }

        let remoteMessages = remoteReusingLocalIdentities(incomingRemoteMessages, local: localMessages)
        var merged = remoteMessages
        var preservedOlderRemoteMessages: [GaryxMobileMessage] = []
        var preservedOlderRemoteIds = Set<String>()
        let localClientIntentIds = Set(
            localMessages
                .compactMap { $0.clientIntentId?.trimmingCharacters(in: .whitespacesAndNewlines) }
                .filter { !$0.isEmpty }
        )
        let localPendingInputIds = Set(
            localMessages
                .compactMap { $0.pendingInputId?.trimmingCharacters(in: .whitespacesAndNewlines) }
                .filter { !$0.isEmpty }
        )
        // Text-dedup counts only remote occurrences that are not already
        // spent on a specific local row (identity reuse or intent-id match).
        // Without this exclusion two identical optimistic sends collapse to
        // one row when the first materializes.
        var remoteUserTextCounts = Dictionary(
            grouping: remoteMessages
                .filter { remote in
                    guard remote.role == .user, remote.remoteId == nil else { return false }
                    let clientIntentId = remote.clientIntentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
                    if !clientIntentId.isEmpty, localClientIntentIds.contains(clientIntentId) {
                        return false
                    }
                    let pendingInputId = remote.pendingInputId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
                    return pendingInputId.isEmpty || !localPendingInputIds.contains(pendingInputId)
                }
                .map(userMergeKey),
            by: { $0 }
        )
        .mapValues(\.count)
        // Remote occurrences already represented by a non-optimistic local
        // row (history rows, pending inputs, materialized sends) must not be
        // double-spent on optimistic dedup below.
        for localRemoteUserText in localMessages
            .filter({ $0.role == .user && $0.localState != .optimistic })
            .map(userMergeKey) {
            if let count = remoteUserTextCounts[localRemoteUserText], count > 0 {
                remoteUserTextCounts[localRemoteUserText] = count - 1
            }
        }
        let currentTurnRemoteAssistantTexts = currentTurnAssistantTexts(in: remoteMessages)
        let remoteClientIntentIds = Set(
            remoteMessages
                .compactMap { $0.clientIntentId?.trimmingCharacters(in: .whitespacesAndNewlines) }
                .filter { !$0.isEmpty }
        )
        let remotePendingInputIds = Set(
            remoteMessages
                .compactMap { $0.pendingInputId?.trimmingCharacters(in: .whitespacesAndNewlines) }
                .filter { !$0.isEmpty }
        )

        var isAfterUnmaterializedLocalUser = false
        for local in localMessages {
            if let remoteIndex = merged.firstIndex(where: { $0.id == local.id }) {
                if local.role == .assistant,
                   local.isStreaming,
                   merged[remoteIndex].role == .assistant,
                   normalizedMergeText(local.text).count > normalizedMergeText(merged[remoteIndex].text).count {
                    merged[remoteIndex] = local
                }
                continue
            }
            if let preserveRemoteBeforeIndex,
               let historyIndex = local.historyIndex,
               historyIndex < preserveRemoteBeforeIndex,
               preservedOlderRemoteIds.insert(local.id).inserted {
                preservedOlderRemoteMessages.append(local)
                continue
            }
            let localClientIntentId = local.clientIntentId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            let localPendingInputId = local.pendingInputId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            if !localClientIntentId.isEmpty,
               remoteClientIntentIds.contains(localClientIntentId) {
                isAfterUnmaterializedLocalUser = false
                continue
            }
            if !localPendingInputId.isEmpty,
               remotePendingInputIds.contains(localPendingInputId) {
                isAfterUnmaterializedLocalUser = false
                continue
            }
            let normalizedText = normalizedMergeText(local.text)
            switch local.role {
            case .user:
                if local.localState == .optimistic {
                    let mergeKey = userMergeKey(local)
                    if let count = remoteUserTextCounts[mergeKey],
                       count > 0 {
                        remoteUserTextCounts[mergeKey] = count - 1
                        isAfterUnmaterializedLocalUser = false
                        continue
                    }
                    merged.append(local)
                    isAfterUnmaterializedLocalUser = true
                } else {
                    isAfterUnmaterializedLocalUser = false
                }
            case .assistant:
                if local.isStreaming || local.localState == .remotePartial {
                    if isAfterUnmaterializedLocalUser {
                        merged.append(local)
                        continue
                    }
                    let alreadyMaterialized = currentTurnRemoteAssistantTexts.contains { remoteText in
                        !normalizedText.isEmpty
                            && !remoteText.isEmpty
                            && remoteText.count >= normalizedText.count
                            && remoteText.hasPrefix(normalizedText)
                    }
                    if !alreadyMaterialized {
                        merged.append(local)
                    }
                }
            case .tool:
                if local.isStreaming || local.toolTraceGroup?.isActive == true {
                    if let localGroup = local.toolTraceGroup,
                       let remoteIndex = merged.indices.first(where: { remoteIndex in
                           let remote = merged[remoteIndex]
                           guard let remoteGroup = remote.toolTraceGroup else { return false }
                           return toolTraceGroupsOverlap(
                               remoteGroup,
                               localGroup,
                               allowFingerprint: isInCurrentTurn(index: remoteIndex, messages: merged)
                           )
                       }) {
                        if var remoteGroup = merged[remoteIndex].toolTraceGroup {
                            remoteGroup = mergedToolTraceGroup(remoteGroup, with: localGroup)
                            merged[remoteIndex].toolTraceGroup = remoteGroup
                            merged[remoteIndex].text = remoteGroup.summary
                            merged[remoteIndex].isStreaming = remoteGroup.isActive
                        }
                        continue
                    }
                    merged.append(local)
                }
            case .system:
                if local.statusText != nil
                    || (local.localState != nil && local.localState != .remoteFinal) {
                    merged.append(local)
                }
            }
        }

        if !preservedOlderRemoteMessages.isEmpty {
            merged = preservedOlderRemoteMessages + merged
        }
        return merged
    }

    // MARK: Helpers

    static func normalizedMergeText(_ text: String) -> String {
        text.trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: "\r\n", with: "\n")
    }

    static func userMergeKey(_ message: GaryxMobileMessage) -> String {
        GaryxStructuredContentRenderer.userMergeKey(
            text: message.text,
            attachments: message.attachments.map(\.contentDescriptor)
        )
    }

    static func currentTurnAssistantTexts(in messages: [GaryxMobileMessage]) -> [String] {
        let startIndex: Array<GaryxMobileMessage>.Index
        if let lastUserIndex = messages.lastIndex(where: { $0.role == .user }) {
            startIndex = messages.index(after: lastUserIndex)
        } else {
            startIndex = messages.startIndex
        }
        return messages[startIndex...]
            .filter { $0.role == .assistant }
            .map { normalizedMergeText($0.text) }
    }

    static func isInCurrentTurn(index: Int, messages: [GaryxMobileMessage]) -> Bool {
        guard let lastUserIndex = messages.lastIndex(where: { $0.role == .user }) else {
            return true
        }
        return index > lastUserIndex
    }

    static func toolTraceGroupsOverlap(
        _ left: GaryxMobileToolTraceGroup,
        _ right: GaryxMobileToolTraceGroup,
        allowFingerprint: Bool
    ) -> Bool {
        let leftKeys = Set(left.entries.compactMap { toolTraceMergeKey($0, includeFingerprint: allowFingerprint) })
        let rightKeys = Set(right.entries.compactMap { toolTraceMergeKey($0, includeFingerprint: allowFingerprint) })
        return !leftKeys.isDisjoint(with: rightKeys)
    }

    static func mergedToolTraceGroup(
        _ remote: GaryxMobileToolTraceGroup,
        with local: GaryxMobileToolTraceGroup
    ) -> GaryxMobileToolTraceGroup {
        var merged = remote
        merged.live = remote.live || local.live
        for localEntry in local.entries {
            if let localKey = toolTraceMergeKey(localEntry),
               let index = merged.entries.firstIndex(where: { toolTraceMergeKey($0) == localKey }) {
                if localEntry.status != .running {
                    merged.entries[index].absorb(result: localEntry)
                }
                continue
            }
            merged.entries.append(localEntry)
        }
        return merged
    }

    static func toolTraceMergeKey(
        _ entry: GaryxMobileToolTraceEntry,
        includeFingerprint: Bool = true
    ) -> String? {
        if let toolUseId = entry.toolUseId?.trimmingCharacters(in: .whitespacesAndNewlines),
           !toolUseId.isEmpty {
            return "id:\(toolUseId)"
        }
        guard includeFingerprint else {
            return nil
        }
        let normalizedTool = entry.toolName.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let input = entry.inputText?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let summary = entry.summaryText?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !normalizedTool.isEmpty, !input.isEmpty || !summary.isEmpty else {
            return nil
        }
        return "fp:\(normalizedTool):\(input):\(summary):\(entry.isError)"
    }
}
