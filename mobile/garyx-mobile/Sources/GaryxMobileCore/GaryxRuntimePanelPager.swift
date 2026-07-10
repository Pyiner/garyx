import CoreGraphics
import Foundation

/// Pure state machine behind the thread runtime panel's two-phase page
/// swap: the current page fades out fully, then — unless a newer request
/// or a collapse invalidated it — the page swaps and fades back in. Only
/// one page is ever mounted; tokens make stale completions harmless.
///
/// The view layer owns clocks and animation curves; this type owns the
/// ordering rules so they stay testable without SwiftUI.
public struct GaryxRuntimePanelPager<Element: Equatable>: Equatable {
    public private(set) var page: Element
    public private(set) var isContentVisible: Bool
    public private(set) var transitionToken: Int

    public init(page: Element) {
        self.page = page
        self.isContentVisible = true
        self.transitionToken = 0
    }

    /// Starts a transition by hiding the current page. Returns the token
    /// the caller must present to `complete(token:to:)` after the exit
    /// delay, or nil when the target already shows.
    public mutating func begin(to next: Element) -> Int? {
        guard page != next else { return nil }
        transitionToken += 1
        isContentVisible = false
        return transitionToken
    }

    /// Swaps to the target page if the token is still current. A stale
    /// token (newer request or reset happened meanwhile) is a no-op:
    /// latest-wins.
    @discardableResult
    public mutating func complete(token: Int, to next: Element) -> Bool {
        guard token == transitionToken else { return false }
        page = next
        isContentVisible = true
        return true
    }

    /// Instant reset (panel collapse): shows `element` immediately and
    /// invalidates every pending completion.
    public mutating func reset(to element: Element) {
        transitionToken += 1
        page = element
        isContentVisible = true
    }
}

/// Viewport sizing for the runtime panel's options page: estimated from
/// the default-size row metrics, corrected by the measured content height
/// (Dynamic Type growth), floored and capped.
public enum GaryxRuntimeOptionsViewportMetrics {
    public static let rowHeight: CGFloat = 44
    public static let verticalPadding: CGFloat = 16
    public static let minHeight: CGFloat = 96

    public static func height(
        rowCount: Int,
        hairlineHeight: CGFloat,
        measuredContentHeight: CGFloat?,
        maxHeight: CGFloat
    ) -> CGFloat {
        let hairlines = CGFloat(max(rowCount - 1, 0)) * hairlineHeight
        let estimate = CGFloat(rowCount) * rowHeight + hairlines + verticalPadding
        let content = measuredContentHeight ?? estimate
        return min(max(content, minHeight), maxHeight)
    }
}
