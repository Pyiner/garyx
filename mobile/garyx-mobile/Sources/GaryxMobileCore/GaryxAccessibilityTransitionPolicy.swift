public enum GaryxAccessibilityTransitionMode: String, CaseIterable, Codable, Sendable {
    case spatial
    case crossFade
    case immediate
}

public enum GaryxAccessibilityTransitionPolicy {
    public static func mode(
        reduceMotion: Bool,
        prefersCrossFadeTransitions: Bool
    ) -> GaryxAccessibilityTransitionMode {
        if prefersCrossFadeTransitions {
            return .crossFade
        }
        return reduceMotion ? .immediate : .spatial
    }

    /// Existing Reduce Motion fallbacks already remove spatial movement. The
    /// dedicated cross-fade preference requests the same presentation even if
    /// SwiftUI has not surfaced Reduce Motion in the current environment yet.
    public static func usesCrossFade(
        reduceMotion: Bool,
        prefersCrossFadeTransitions: Bool
    ) -> Bool {
        mode(
            reduceMotion: reduceMotion,
            prefersCrossFadeTransitions: prefersCrossFadeTransitions
        ) != .spatial
    }

    /// Reduce Motion keeps transitions immediate unless the user explicitly
    /// asks for a cross-fade, in which case a short opacity animation is the
    /// requested accessible transition.
    public static func animatesTransition(
        reduceMotion: Bool,
        prefersCrossFadeTransitions: Bool
    ) -> Bool {
        mode(
            reduceMotion: reduceMotion,
            prefersCrossFadeTransitions: prefersCrossFadeTransitions
        ) != .immediate
    }
}
