import Foundation

public enum GaryxSelectedThreadStreamAction: Equatable {
    case none
    case start(String)
    case stop
}

public enum GaryxSelectedThreadStreamPolicy {
    public static func action(previousThreadId: String?, selectedThreadId: String?) -> GaryxSelectedThreadStreamAction {
        let previous = previousThreadId?.trimmingCharacters(in: .whitespacesAndNewlines)
        let selected = selectedThreadId?.trimmingCharacters(in: .whitespacesAndNewlines)

        guard let selected, !selected.isEmpty else {
            return previous?.isEmpty == false ? .stop : .none
        }
        return .start(selected)
    }
}

public enum GaryxVisibleConversationStreamPolicy {
    public static func shouldStart(
        isConversationVisible: Bool,
        selectedThreadId: String?,
        streamOwnedThreadId: String?,
        hasStreamTask: Bool
    ) -> Bool {
        guard isConversationVisible else { return false }
        let selected = selectedThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !selected.isEmpty else { return false }
        let owned = streamOwnedThreadId?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return owned != selected || !hasStreamTask
    }
}
