import Foundation

public enum GaryxChannelIconResolver {
    public static func displayName(
        for channel: String,
        plugins: [GaryxChannelPluginCatalogEntry]
    ) -> String? {
        let normalizedChannel = channel.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !normalizedChannel.isEmpty else { return nil }
        for plugin in plugins {
            guard plugin.id.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() == normalizedChannel else {
                continue
            }
            let value = plugin.displayName.trimmingCharacters(in: .whitespacesAndNewlines)
            return value.isEmpty ? nil : value
        }
        return nil
    }

    public static func iconDataUrl(
        for channel: String,
        plugins: [GaryxChannelPluginCatalogEntry]
    ) -> String? {
        let normalizedChannel = channel.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !normalizedChannel.isEmpty else { return nil }
        for plugin in plugins {
            guard plugin.id.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() == normalizedChannel else {
                continue
            }
            let value = plugin.iconDataUrl?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            return value.isEmpty ? nil : value
        }
        return nil
    }
}

public enum GaryxThreadActivitySignature {
    public static func make(from transcript: GaryxThreadTranscript) -> String {
        make(
            messages: transcript.messages,
            pendingUserInputs: transcript.pendingUserInputs,
            threadRuntime: transcript.threadRuntime
        )
    }

    public static func make(
        messages: [GaryxTranscriptMessage],
        pendingUserInputs: [GaryxPendingUserInput],
        threadRuntime: GaryxThreadRuntimeSummary?
    ) -> String {
        let lastMessage = messages.last
        let lastPendingInput = pendingUserInputs.last
        let activeRun = threadRuntime?.activeRun
        return [
            "messageCount=\(messages.count)",
            "lastMessage.id=\(lastMessage?.id ?? "")",
            "lastMessage.role=\(lastMessage?.role.rawValue ?? "")",
            "lastMessage.text=\(lastMessage?.text ?? "")",
            "lastMessage.timestamp=\(lastMessage?.timestamp ?? "")",
            "lastMessage.kind=\(lastMessage?.kind ?? "")",
            "pendingInputCount=\(pendingUserInputs.count)",
            "lastPendingInput.id=\(lastPendingInput?.id ?? "")",
            "lastPendingInput.status=\(lastPendingInput?.status ?? "")",
            "lastPendingInput.active=\(lastPendingInput?.active == true ? "true" : "false")",
            "lastPendingInput.text=\(lastPendingInput?.text ?? "")",
            "activeRun.runId=\(activeRun?.runId ?? "")",
            "activeRun.updatedAt=\(activeRun?.updatedAt ?? "")",
            "activeRun.assistantResponse=\(activeRun?.assistantResponse ?? "")",
            "activeRun.pendingUserInputCount=\(activeRun?.pendingUserInputCount ?? 0)",
        ].joined(separator: "\u{1F}")
    }
}
