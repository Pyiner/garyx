import CoreGraphics

/// Pure decision logic for the iOS capsule full-screen detail's interactive
/// pull-to-dismiss gesture (#TASK-1470).
///
/// The capsule detail stays a `.fullScreenCover` (full-bleed, never a sheet) and
/// gains a Photos-style drag-down dismiss. The hard part is coexisting with the
/// inner `WKWebView` vertical scroll: a downward drag must dismiss only when the
/// page is scrolled to the top, and must never fight content scrolling.
///
/// This mirrors the sidebar gesture's "decide the axis once, then lock"
/// discipline (`GaryxMobileViews.openingSidebarGesture` decides
/// `sidebarDragAxis` on the first change and keeps returning for the off-axis):
/// the dismiss phase is decided exactly once at the start of a drag from
/// `decideInitialPhase` and never re-evaluated mid-drag. That guarantees a drag
/// started off-top is `.ignored` for its whole life even if the web view later
/// reaches the top, and a drag started at-top stays `.engaged` so a quick scroll
/// up can't cancel an in-progress dismiss.
///
/// Flick detection uses `DragGesture.Value.predictedEndTranslation` (the same
/// signal the sidebar gestures use), so it does not depend on the iOS 17-only
/// `velocity` accessor.
public enum GaryxCapsuleDragPhase: Equatable, Sendable {
    /// No active drag; the next change event decides the phase.
    case idle
    /// This drag owns dismissal: content follows the finger, release decides.
    case engaged
    /// This drag belongs to web-view scrolling (or an upward pull); the dismiss
    /// container stays inert for the rest of the drag.
    case ignored
}

public enum GaryxCapsuleDragDismiss {
    /// Release past this downward offset (points) dismisses.
    public static let dismissThreshold: CGFloat = 120
    /// A flick whose predicted end translation reaches this dismisses even when
    /// the live offset is short.
    public static let flickThreshold: CGFloat = 220
    /// Offset over which the drag is considered "fully pulled" for derived
    /// visuals (scrim/scale). Tuned in the view; kept here so progress is tested.
    public static let fullPullDistance: CGFloat = 240

    /// Decide the phase once, at the first change of a drag.
    ///
    /// Engages only when the web view is at the top **and** the drag is downward.
    /// Anything else (off-top start, upward pull) is `.ignored` and — because the
    /// caller locks the phase for the rest of the drag — stays ignored even if the
    /// web view later scrolls to the top.
    public static func decideInitialPhase(atTop: Bool, translationY: CGFloat) -> GaryxCapsuleDragPhase {
        guard atTop, translationY > 0 else { return .ignored }
        return .engaged
    }

    /// Live downward offset to apply to the content. Only an engaged drag moves
    /// it; an ignored drag never displaces the content (web scrolling stays
    /// untouched). Negative translations (overscroll up) clamp to 0.
    public static func resolvedOffset(phase: GaryxCapsuleDragPhase, translationY: CGFloat) -> CGFloat {
        guard phase == .engaged else { return 0 }
        return max(0, translationY)
    }

    /// Normalized pull progress in `0...1` for derived visuals (backdrop dim,
    /// content scale). `0` at rest, `1` once pulled `fullPullDistance`.
    public static func dragProgress(offset: CGFloat, fullPullDistance: CGFloat = fullPullDistance) -> Double {
        guard fullPullDistance > 0 else { return 0 }
        let ratio = Double(offset / fullPullDistance)
        return min(1, max(0, ratio))
    }

    /// Whether releasing the drag should dismiss. Only an engaged drag can
    /// dismiss; it does so when the live offset crosses `dismissThreshold` or the
    /// predicted (flicked) end translation crosses `flickThreshold`.
    public static func shouldDismiss(
        phase: GaryxCapsuleDragPhase,
        offset: CGFloat,
        predictedTranslationY: CGFloat
    ) -> Bool {
        guard phase == .engaged else { return false }
        return offset >= dismissThreshold || predictedTranslationY >= flickThreshold
    }
}
