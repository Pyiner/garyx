import Foundation

public struct GaryxTaskNotification: Equatable, Hashable, Sendable {
    public let event: String
    public let status: String
    public let taskId: String
    public let title: String
    public let finalMessage: String

    public init(
        event: String,
        status: String,
        taskId: String,
        title: String,
        finalMessage: String
    ) {
        self.event = event
        self.status = status
        self.taskId = taskId
        self.title = title
        self.finalMessage = finalMessage
    }
}

public enum GaryxTaskNotificationPresentation {
    /// Structural decoder only. Semantic identity and header fields come from
    /// the server-owned presentation object, never from this text envelope.
    public static func stripEnvelope(from text: String) -> String? {
        guard let openStart = text.range(of: "<garyx_task_notification") else {
            return nil
        }
        guard let openEnd = text[openStart.lowerBound...].firstIndex(of: ">") else {
            return nil
        }
        guard let closeRange = text.range(
            of: "</garyx_task_notification>",
            options: .backwards
        ), openEnd < closeRange.lowerBound else {
            return nil
        }
        return text[text.index(after: openEnd)..<closeRange.lowerBound]
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    public static func statusLabel(for status: String) -> String {
        if status == "in_review" {
            return "In review"
        }
        return status
            .split { $0 == "_" || $0 == "-" }
            .map { part in
                part.prefix(1).uppercased() + String(part.dropFirst())
            }
            .joined(separator: " ")
    }
}

public enum GaryxTaskNotificationOverflow {
    public static func overflows(
        naturalHeight: Double,
        clampHeight: Double,
        epsilon: Double
    ) -> Bool {
        naturalHeight > clampHeight + epsilon
    }
}

public struct GaryxTaskNotificationSelection: Identifiable, Equatable, Sendable {
    public struct ID: Hashable, Sendable {
        public let messageId: String
        public let messageSeq: Int?

        public init(messageId: String, messageSeq: Int?) {
            self.messageId = messageId
            self.messageSeq = messageSeq
        }
    }

    public let id: ID
    public let notification: GaryxTaskNotification

    public init(
        messageId: String,
        messageSeq: Int?,
        notification: GaryxTaskNotification
    ) {
        id = ID(messageId: messageId, messageSeq: messageSeq)
        self.notification = notification
    }
}

public struct GaryxTaskNotificationPresentationScope: Equatable, Sendable {
    public let threadIdentity: String
    public let gatewayIdentity: String
    public let occurrenceIdentity: String

    public init(
        threadIdentity: String,
        gatewayIdentity: String,
        occurrenceIdentity: String
    ) {
        self.threadIdentity = threadIdentity
        self.gatewayIdentity = gatewayIdentity
        self.occurrenceIdentity = occurrenceIdentity
    }
}

public struct GaryxTaskNotificationSelectionState: Equatable, Sendable {
    public private(set) var scope: GaryxTaskNotificationPresentationScope?
    public private(set) var selection: GaryxTaskNotificationSelection?

    public init() {}

    @discardableResult
    public mutating func synchronize(
        scope nextScope: GaryxTaskNotificationPresentationScope
    ) -> Bool {
        let changed = scope != nil && scope != nextScope
        scope = nextScope
        if changed {
            selection = nil
        }
        return changed
    }

    public mutating func present(
        _ nextSelection: GaryxTaskNotificationSelection,
        scope nextScope: GaryxTaskNotificationPresentationScope
    ) {
        _ = synchronize(scope: nextScope)
        selection = nextSelection
    }

    public mutating func dismiss() {
        selection = nil
    }
}
