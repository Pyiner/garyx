import Foundation

/// Which placeholder the chat composer shows.
public enum GaryxComposerPlaceholderKind: Equatable, Sendable {
    /// Idle thread (new or with history): invite a fresh message.
    case prompt
    /// A run is in flight: the next message queues as a follow-up.
    case followUp
}

/// The chat composer's affordances, as a pure function of the open thread's
/// **real run state** and the local draft — never of whether a thread is merely
/// open, and never of the tail transcript row (#TASK-1453 problem A).
public struct GaryxComposerPresentation: Equatable, Sendable {
    public var placeholder: GaryxComposerPlaceholderKind
    public var showsStopButton: Bool
    public var showsSendButton: Bool

    public init(placeholder: GaryxComposerPlaceholderKind, showsStopButton: Bool, showsSendButton: Bool) {
        self.placeholder = placeholder
        self.showsStopButton = showsStopButton
        self.showsSendButton = showsSendButton
    }
}

public enum GaryxComposerPresentationResolver {
    /// Resolve the composer's placeholder + action buttons.
    ///
    /// The follow-up placeholder and the stop button are the busy/active-run
    /// affordances; an idle thread shows the prompt placeholder and the send
    /// button. This mirrors the Mac composer, which keys its placeholder on
    /// whether a run is sending — not on whether a thread is open. The send
    /// button also stays available while busy when there is a draft, so a
    /// follow-up can be queued.
    ///
    /// - Parameters:
    ///   - isThreadBusy: the thread's real run state — the server-derived
    ///     committed run state unioned with the local optimistic run tracker.
    ///     Deliberately not a function of the tail row (a capsule card must not
    ///     make the composer look busy).
    ///   - hasLocalPayload: the local draft carries text or attachments.
    public static func resolve(isThreadBusy: Bool, hasLocalPayload: Bool) -> GaryxComposerPresentation {
        GaryxComposerPresentation(
            placeholder: isThreadBusy ? .followUp : .prompt,
            showsStopButton: isThreadBusy,
            showsSendButton: !isThreadBusy || hasLocalPayload
        )
    }
}
