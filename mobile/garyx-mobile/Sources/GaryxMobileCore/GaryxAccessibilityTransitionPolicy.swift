public enum GaryxAccessibilityTransitionPolicy {
    /// Existing Reduce Motion fallbacks already remove spatial movement. The
    /// dedicated cross-fade preference requests the same presentation even if
    /// SwiftUI has not surfaced Reduce Motion in the current environment yet.
    public static func usesCrossFade(
        reduceMotion: Bool,
        prefersCrossFadeTransitions: Bool
    ) -> Bool {
        reduceMotion || prefersCrossFadeTransitions
    }

    /// Reduce Motion keeps transitions immediate unless the user explicitly
    /// asks for a cross-fade, in which case a short opacity animation is the
    /// requested accessible transition.
    public static func animatesTransition(
        reduceMotion: Bool,
        prefersCrossFadeTransitions: Bool
    ) -> Bool {
        !reduceMotion || prefersCrossFadeTransitions
    }
}
