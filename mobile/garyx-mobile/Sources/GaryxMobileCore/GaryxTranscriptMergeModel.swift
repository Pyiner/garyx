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
    /// for optimistic local sends. A committed assistant row in the current turn
    /// also adopts the matching live assistant row id once the committed text
    /// materializes that streamed prefix. Each local row is consumed at most once.
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
        let lastRemoteUserIndex = remoteMessages.lastIndex { $0.role == .user }
        let lastLocalUserIndex = localMessages.lastIndex { $0.role == .user }

        func reusableLiveAssistant(for remote: GaryxMobileMessage, remoteIndex: Int) -> GaryxMobileMessage? {
            guard remote.role == .assistant else { return nil }
            if let lastRemoteUserIndex, remoteIndex <= lastRemoteUserIndex {
                return nil
            }
            let remoteText = normalizedMergeText(remote.text)
            guard !remoteText.isEmpty else { return nil }
            guard let localIndex = localMessages.indices.first(where: { index in
                let local = localMessages[index]
                guard !consumedLocalIds.contains(local.id),
                      local.id != remote.id,
                      !remoteIds.contains(local.id),
                      local.role == .assistant,
                      local.localState != .remoteFinal,
                      local.isStreaming || local.localState == .remotePartial else {
                    return false
                }
                if let lastLocalUserIndex, index <= lastLocalUserIndex {
                    return false
                }
                let localText = normalizedMergeText(local.text)
                return !localText.isEmpty
                    && remoteText.count >= localText.count
                    && remoteText.hasPrefix(localText)
            }) else {
                return nil
            }
            return localMessages[localIndex]
        }

        func reusableLocal(for remote: GaryxMobileMessage, remoteIndex: Int) -> GaryxMobileMessage? {
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
            if let assistantMatch = reusableLiveAssistant(for: remote, remoteIndex: remoteIndex) {
                return assistantMatch
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

        return remoteMessages.enumerated().map { remoteIndex, remote in
            guard remote.role == .user || remote.role == .assistant,
                  let local = reusableLocal(for: remote, remoteIndex: remoteIndex) else {
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
        preserveRemoteBeforeIndex: Int? = nil,
        threadRunActive: Bool = true
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
        let remoteAssistantTexts = remoteMessages
            .filter { $0.role == .assistant }
            .map { normalizedMergeText($0.text) }
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
        var passedMaterializedLocalUser = false
        for local in localMessages {
            if let remoteIndex = merged.firstIndex(where: { $0.id == local.id }) {
                if local.role == .user {
                    isAfterUnmaterializedLocalUser = false
                    passedMaterializedLocalUser = true
                }
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
                if local.role == .user {
                    passedMaterializedLocalUser = true
                }
                continue
            }
            if !localPendingInputId.isEmpty,
               remotePendingInputIds.contains(localPendingInputId) {
                isAfterUnmaterializedLocalUser = false
                if local.role == .user {
                    passedMaterializedLocalUser = true
                }
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
                        passedMaterializedLocalUser = true
                        continue
                    }
                    merged.append(local)
                    isAfterUnmaterializedLocalUser = true
                    passedMaterializedLocalUser = false
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
                    // Defensive live-cache cleanup: if a previous-turn streaming
                    // assistant was already committed remotely, do not keep the
                    // stale local copy below a newly materialized user row.
                    let staleFromPreviousTurn = passedMaterializedLocalUser
                        && normalizedText.count >= 12
                        && remoteAssistantTexts.contains { remoteText in
                            !remoteText.isEmpty
                                && remoteText.count >= normalizedText.count
                                && remoteText.hasPrefix(normalizedText)
                        }
                    if !alreadyMaterialized && !staleFromPreviousTurn {
                        merged.append(local)
                    }
                }
            case .tool:
                // While the run is active a live tool group is an OVERLAY on the
                // committed structure, not a row that owns its own grouping: each
                // live entry updates the committed row that already holds that
                // call, and only calls with no committed row yet stay as their own
                // live row. The committed window thus stays the single source of
                // grouping. Folding the whole live group into one committed row
                // instead (the prior behavior) duplicated a later call the window
                // had split into its own row, and stranded a running call's name on
                // a result-only row as a generic "Used 1 tool".
                //
                // When the run is finished the canonical transcript already holds
                // every tool row in order, so a live group still claiming to run
                // (it lost its terminal events to a backgrounded stream or dropped
                // socket) is dropped rather than pinned after the final answer.
                guard threadRunActive,
                      local.isStreaming || local.toolTraceGroup?.isActive == true,
                      let liveGroup = local.toolTraceGroup else { break }
                overlayLiveToolGroup(liveGroup, fallbackRow: local, into: &merged)
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

    @discardableResult
    static func appendLiveAssistantText(
        _ text: String,
        targetId: String,
        into messages: inout [GaryxMobileMessage]
    ) -> String? {
        guard !text.isEmpty else { return nil }
        if let index = messages.firstIndex(where: { $0.id == targetId || $0.remoteId == targetId }) {
            messages[index].text += text
            messages[index].isStreaming = true
            return messages[index].id
        }
        if let index = currentTurnAssistantIndexMaterializingLiveText(text, in: messages) {
            let existingText = normalizedMergeText(messages[index].text)
            let pendingText = normalizedMergeText(text)
            if pendingText.count > existingText.count,
               pendingText.hasPrefix(existingText) {
                messages[index].text = text
                messages[index].isStreaming = true
            }
            return messages[index].id
        }
        messages.append(
            GaryxMobileMessage(
                id: targetId,
                role: .assistant,
                text: text,
                timestamp: nil,
                isStreaming: true,
                localState: .remotePartial
            )
        )
        return targetId
    }

    private static func currentTurnAssistantIndexMaterializingLiveText(
        _ text: String,
        in messages: [GaryxMobileMessage]
    ) -> Int? {
        let pendingText = normalizedMergeText(text)
        guard !pendingText.isEmpty else { return nil }
        let startIndex: Int
        if let lastUserIndex = messages.lastIndex(where: { $0.role == .user }) {
            startIndex = messages.index(after: lastUserIndex)
        } else {
            startIndex = messages.startIndex
        }
        guard startIndex < messages.endIndex else { return nil }
        return messages[startIndex...].indices.reversed().first { index in
            let message = messages[index]
            guard message.role == .assistant,
                  message.attachments.isEmpty else {
                return false
            }
            let existingText = normalizedMergeText(message.text)
            guard !existingText.isEmpty else { return false }
            if existingText == pendingText {
                return true
            }
            if existingText.count >= pendingText.count,
               existingText.hasPrefix(pendingText) {
                return true
            }
            if pendingText.count > existingText.count,
               pendingText.hasPrefix(existingText) {
                return true
            }
            return pendingText.count >= 4 && existingText.contains(pendingText)
        }
    }

    /// Whether `index` falls in the running turn — after the last user boundary.
    /// `ignoringPendingSteer` skips a steer the user queued mid-run (.optimistic
    /// just-sent, or .remotePartial acked-and-pending): that is a future turn the
    /// assistant has not started, so a stable-id call may reconcile across it. The
    /// id-less fingerprint path leaves the default (a steer stays a boundary),
    /// since it cannot prove two same-fingerprint calls are the same one.
    static func isInCurrentTurn(
        index: Int,
        messages: [GaryxMobileMessage],
        ignoringPendingSteer: Bool = false
    ) -> Bool {
        guard let lastUserIndex = messages.lastIndex(where: { message in
            guard message.role == .user else { return false }
            guard ignoringPendingSteer else { return true }
            return message.localState != .optimistic && message.localState != .remotePartial
        }) else {
            return true
        }
        return index > lastUserIndex
    }

    /// Overlay a live (in-flight) tool group onto the already-merged committed
    /// rows. Each live entry is reconciled against the committed row that already
    /// holds that call — adopting a running call's name + input onto a bare
    /// result-only row, or absorbing its result once the call ends — so the
    /// committed window stays the single source of grouping. Entries with no
    /// committed row yet are appended as one trailing live row.
    ///
    /// Appending (rather than splicing into live order) is both correct and
    /// stable. Correct: committed rows arrive in strict seq order, so an
    /// uncommitted call is always the most recent in the turn — any earlier call
    /// has already committed and is matched above, never left behind in the
    /// in-flight buffer. Stable: the flush feeds each merged result back as the
    /// next local input, and a trailing row stays trailing on every re-merge,
    /// whereas re-inserting at a recomputed index made an in-flight call jump
    /// position once its neighbor committed.
    static func overlayLiveToolGroup(
        _ liveGroup: GaryxMobileToolTraceGroup,
        fallbackRow: GaryxMobileMessage,
        into merged: inout [GaryxMobileMessage]
    ) {
        var uncommittedEntries: [GaryxMobileToolTraceEntry] = []
        for entry in liveGroup.entries {
            guard let rowIndex = merged.indices.first(where: { index in
                guard let committed = merged[index].toolTraceGroup?.entries else { return false }
                // A stable toolUseId is unique per call, so an id match is the same
                // call: reconcile it within the committed turn — a queued steer is
                // transparent (a future turn the assistant hasn't started), while a
                // real committed user turn still bounds it.
                if committed.contains(where: { toolTraceEntriesSameCall($0, entry, allowFingerprint: false) }) {
                    return isInCurrentTurn(index: index, messages: merged, ignoringPendingSteer: true)
                }
                // Id-less providers fall back to the tool+input fingerprint, which
                // can't tell two identical calls apart, so stay conservative: gate
                // to the same turn by any user row, leaving a queued steer a boundary.
                return isInCurrentTurn(index: index, messages: merged)
                    && committed.contains { toolTraceEntriesSameCall($0, entry, allowFingerprint: true) }
            }),
            var group = merged[rowIndex].toolTraceGroup,
            let entryIndex = group.entries.firstIndex(where: {
                toolTraceEntriesSameCall($0, entry, allowFingerprint: true)
            }) else {
                uncommittedEntries.append(entry)
                continue
            }
            if entry.status == .running {
                // The committed side can be a bare result-only row (generic "tool")
                // when the tool_use landed before the resumable window opened, so
                // the name + input live only on the still-running entry. Adopt that
                // identity without finalizing the not-yet-finished call.
                group.entries[entryIndex].adoptCallIdentity(from: entry)
            } else {
                group.entries[entryIndex].absorb(result: entry)
            }
            group.live = group.live || liveGroup.live
            merged[rowIndex].toolTraceGroup = group
            merged[rowIndex].text = group.summary
            merged[rowIndex].isStreaming = group.isActive
        }
        guard !uncommittedEntries.isEmpty else { return }
        var row = fallbackRow
        var group = liveGroup
        group.entries = uncommittedEntries
        row.toolTraceGroup = group
        row.text = group.summary
        row.isStreaming = group.isActive
        merged.append(row)
    }

    /// Whether two tool-trace entries are the same call. Entries that BOTH carry a
    /// stable `toolUseId` match only when those ids are equal (distinct ids are distinct
    /// calls, even with identical input). When at least one lacks an id, fall back to
    /// the call's identity — tool + input — which stays stable as the call goes running
    /// → completed/failed. Summary/result/isError are deliberately excluded: they change
    /// as a call ends, and including them stopped a live (running) row from matching its
    /// own committed (completed) row, rendering the call twice until the run finished.
    static func toolTraceEntriesSameCall(
        _ a: GaryxMobileToolTraceEntry,
        _ b: GaryxMobileToolTraceEntry,
        allowFingerprint: Bool
    ) -> Bool {
        let idA = a.toolUseId?.trimmingCharacters(in: .whitespacesAndNewlines)
        let idB = b.toolUseId?.trimmingCharacters(in: .whitespacesAndNewlines)
        if let idA, !idA.isEmpty, let idB, !idB.isEmpty {
            return idA == idB
        }
        guard allowFingerprint,
              let fingerprintA = toolTraceCallFingerprint(a),
              let fingerprintB = toolTraceCallFingerprint(b)
        else {
            return false
        }
        return fingerprintA == fingerprintB
    }

    /// Stable identity of a tool call absent a `toolUseId`: tool name + input. `nil`
    /// when either is empty, so input-less calls are not collapsed by fingerprint.
    static func toolTraceCallFingerprint(_ entry: GaryxMobileToolTraceEntry) -> String? {
        let tool = entry.toolName.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let input = entry.inputText?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !tool.isEmpty, !input.isEmpty else {
            return nil
        }
        return "\(tool):\(input)"
    }

    /// Absorb a `tool_result` entry into the tool-use entry it belongs to, by
    /// stable identity. An id'd result matches its call by `toolUseId`
    /// REGARDLESS of whether that call already carries a result: a duplicate or
    /// late result — e.g. the committed copy that raced ahead of the live event
    /// under the real-time committed stream — is absorbed idempotently, never
    /// rendered as its own row. id-less results (Gemini unkeyed calls, Codex
    /// empty ids) fall back to an open running entry matched by tool name. The
    /// return value is `false` ONLY when no entry in the group is the same call;
    /// the caller must then NOT render the result as a standalone tool row.
    static func absorbToolResult(
        _ result: GaryxMobileToolTraceEntry,
        into group: inout GaryxMobileToolTraceGroup,
        allowIdlessFallback: Bool = true
    ) -> Bool {
        if let resultId = result.toolUseId?.trimmingCharacters(in: .whitespacesAndNewlines),
           !resultId.isEmpty {
            if let index = group.entries.lastIndex(where: { entry in
                guard let entryId = entry.toolUseId?.trimmingCharacters(in: .whitespacesAndNewlines),
                      !entryId.isEmpty else { return false }
                return entryId == resultId
            }) {
                group.entries[index].absorb(result: result)
                return true
            }
            return false
        }

        // The id-less tool-name fallback is too weak to cross group boundaries
        // (it can attach a stray result to an unrelated generic group); callers
        // crossing flushed groups disable it and match by stable id only.
        guard allowIdlessFallback else { return false }
        if let index = group.entries.lastIndex(where: { canAbsorbToolResultFallback(result, into: $0) }) {
            group.entries[index].absorb(result: result)
            return true
        }
        return false
    }

    /// id-less fallback: a result with no usable `toolUseId` matches an OPEN
    /// running entry by tool name / title / summary. Mirrors the call's identity
    /// without an id; never matches an already-completed entry, so a second
    /// id-less result does not overwrite a finished call.
    static func canAbsorbToolResultFallback(
        _ result: GaryxMobileToolTraceEntry,
        into candidate: GaryxMobileToolTraceEntry
    ) -> Bool {
        guard candidate.status == .running, candidate.resultText == nil else {
            return false
        }
        let resultId = result.toolUseId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let candidateId = candidate.toolUseId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if !resultId.isEmpty, !candidateId.isEmpty, resultId != candidateId {
            return false
        }
        let resultTool = result.toolName.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let candidateTool = candidate.toolName.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if !resultTool.isEmpty, resultTool == candidateTool {
            return true
        }
        if candidateTool == "tool" || resultTool == "tool" {
            return true
        }
        if result.title.caseInsensitiveCompare(candidate.title) == .orderedSame {
            return true
        }
        if let resultSummary = result.summaryText,
           let candidateSummary = candidate.summaryText,
           resultSummary == candidateSummary {
            return true
        }
        return false
    }

    /// Absorb a `tool_result` into the most recent already-flushed tool group in
    /// the CURRENT turn whose matching tool_use it belongs to (matched by stable
    /// id). The committed builder uses this when an intervening text row flushed
    /// the call's group before its result arrived (a sub-agent runs while the
    /// parent narrates). Stops at the last `.user` so it never crosses turns, and
    /// disables the weak tool-name fallback, which is unsafe across groups.
    static func absorbResultIntoFlushedToolGroup(
        _ entry: GaryxMobileToolTraceEntry,
        in messages: inout [GaryxMobileMessage]
    ) -> Bool {
        for index in messages.indices.reversed() {
            if messages[index].role == .user { break }
            guard messages[index].role == .tool,
                  var group = messages[index].toolTraceGroup else { continue }
            if absorbToolResult(entry, into: &group, allowIdlessFallback: false) {
                messages[index].toolTraceGroup = group
                messages[index].text = group.summary
                return true
            }
        }
        return false
    }

    /// Fold a live tool-trace event (from the global event stream) into the
    /// displayed message list, grouping consecutive tool calls and keeping each
    /// call exactly once.
    ///
    /// Tool data reaches a thread from TWO sources during a run — the real-time
    /// committed stream (rendered into the window) and the global live stream
    /// (this path). Both must converge on one row per call:
    /// - `.toolResult` is absorbed into the matching open call by identity; an
    ///   unmatched result is dropped (never a lone "Used 1 tool").
    /// - `.toolUse` whose call is ALREADY shown in the current turn (the
    ///   committed copy raced ahead, or a prior live group already has it) is
    ///   ignored, instead of opening a second group for the same call — the gap
    ///   that left a duplicate "Used 1 tool" beside a complete command.
    /// Otherwise the entry extends the trailing tool group or opens a new one.
    static func appendLiveToolTraceEntry(
        _ entry: GaryxMobileToolTraceEntry,
        kind: GaryxMobileTranscriptToolTraceKind,
        into messages: inout [GaryxMobileMessage]
    ) {
        if kind == .toolResult {
            for index in messages.indices.reversed() {
                if messages[index].role == .user { break }
                guard var group = messages[index].toolTraceGroup else { continue }
                if absorbToolResult(entry, into: &group) {
                    messages[index].toolTraceGroup = group
                    messages[index].text = group.summary
                    messages[index].isStreaming = group.isActive
                    return
                }
            }
            return
        }

        // A tool_use whose call is already represented in the current turn is a
        // duplicate from the dual stream sources; ignore it rather than open a
        // second group for the same call.
        for index in messages.indices.reversed() {
            if messages[index].role == .user { break }
            guard let group = messages[index].toolTraceGroup else { continue }
            if group.entries.contains(where: { toolTraceEntriesSameCall($0, entry, allowFingerprint: true) }) {
                return
            }
        }

        if let index = messages.indices.last,
           messages[index].role == .tool,
           var group = messages[index].toolTraceGroup {
            group.live = true
            group.entries.append(entry)
            messages[index].toolTraceGroup = group
            messages[index].text = group.summary
            messages[index].isStreaming = group.isActive
            return
        }

        let group = GaryxMobileToolTraceGroup(entries: [entry], live: true)
        messages.append(
            GaryxMobileMessage(
                id: "tool-group:\(entry.id)",
                role: .tool,
                text: group.summary,
                timestamp: entry.timestamp,
                isStreaming: group.isActive,
                toolTraceGroup: group,
                localState: .remotePartial
            )
        )
    }
}
