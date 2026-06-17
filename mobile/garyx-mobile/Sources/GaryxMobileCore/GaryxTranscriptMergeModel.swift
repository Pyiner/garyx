import Foundation

/// Pure transcript merge for the iOS app: reconciles a fetched remote
/// transcript page with locally known messages (optimistic sends, failure
/// annotations, previously loaded older pages).
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
///   (optimistic sends and failure annotations) are preserved.
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
            guard remote.role == .user else { return nil }
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
            guard remote.role == .user,
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

        for local in localMessages {
            if merged.contains(where: { $0.id == local.id }) {
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
                continue
            }
            if !localPendingInputId.isEmpty,
               remotePendingInputIds.contains(localPendingInputId) {
                continue
            }
            switch local.role {
            case .user:
                if local.localState == .optimistic {
                    let mergeKey = userMergeKey(local)
                    if let count = remoteUserTextCounts[mergeKey],
                       count > 0 {
                        remoteUserTextCounts[mergeKey] = count - 1
                        continue
                    }
                    merged.append(local)
                }
            case .assistant:
                break
            case .tool:
                break
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
}
