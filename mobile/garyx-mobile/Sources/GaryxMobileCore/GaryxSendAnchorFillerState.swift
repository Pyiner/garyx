import Foundation

/// Route-occurrence-scoped signal published in the same presentation
/// transaction that appends an optimistic user row.
public struct GaryxConversationLocalSendPresentation: Equatable, Sendable {
    public let scopeIdentity: String
    public let anchorRowId: String
    public let generation: UInt64

    public init(scopeIdentity: String, anchorRowId: String, generation: UInt64) {
        self.scopeIdentity = scopeIdentity
        self.anchorRowId = anchorRowId
        self.generation = generation
    }
}

/// Pure run-space state for a send-anchored transcript.
///
/// The spacer sits after intrinsic transcript content and before the existing
/// bottom chrome clearance, guaranteeing at least one viewport of scrollable
/// space below the anchored user row so the anchor-to-top scroll is always
/// reachable.
///
/// v2 floor rule: each session maintains `runSpaceFloor`, the largest
/// effective viewport observed during the session (monotonic nondecreasing),
/// and the spacer height is always `max(0, floor - contentBelowAnchor)`.
///
/// - Reply growth consumes run space one-for-one: total height below the
///   anchor stays pinned at the floor, so the anchored row never moves.
/// - A growing viewport (keyboard dismissal) raises the floor, so the filler
///   is allowed to grow back and the anchor stays reachable — the v1
///   monotonic-nonincreasing rule clamped the anchor short in exactly this
///   case.
/// - Filler changes happen strictly below the anchored row and outside the
///   visible viewport; they never displace the anchored content above.
public struct GaryxSendAnchorFillerState: Equatable, Sendable {
    public private(set) var anchorRowId: String?
    public private(set) var runSpaceFloor: CGFloat = 0
    public private(set) var height: CGFloat = 0
    /// True once intrinsic content below the anchor filled the session
    /// floor: the reply is growing below the screen. Content-space
    /// measurement, so it cannot false-positive from the viewport still
    /// sitting at the pre-anchor scroll position at send time. The view
    /// hands this off to the scroll state machine (anchored session →
    /// tail following) and ends the session.
    public private(set) var isExhausted = false
    /// v2.1.1: a reader gesture ends the anchored session but must never
    /// cause a clamp jump — instantly removing the run space shrinks
    /// contentSize below the current offset and UIKit snaps a full screen.
    /// Instead the session enters retiring mode: the spacer shrink-wraps to
    /// the viewport bottom, trimming exactly the scrollable excess each
    /// measurement frame (monotonic), so the content bottom stays glued to
    /// the viewport edge until the blank is gone.
    public private(set) var isRetiring = false

    public init() {}

    /// Start a session for a freshly presented send. An unmeasured viewport
    /// (zero) starts an empty floor; the first real measurement raises it
    /// through `reconcile`.
    @discardableResult
    public mutating func begin(
        anchorRowId: String,
        viewportHeight: CGFloat,
        bottomChromeClearance: CGFloat,
        anchorTopInset: CGFloat = 0,
        contentBelowAnchorHeight: CGFloat
    ) -> CGFloat {
        self.anchorRowId = anchorRowId
        // A fresh send owns a fresh session: a still-retiring previous
        // session must not leak in, or reconcile short-circuits forever and
        // exhaustion (the long-reply handoff) never fires (#TASK-2698).
        isRetiring = false
        runSpaceFloor = Self.effectiveRunSpace(
            viewportHeight: viewportHeight,
            bottomChromeClearance: bottomChromeClearance,
            anchorTopInset: anchorTopInset
        )
        height = max(0, runSpaceFloor - Self.valid(contentBelowAnchorHeight))
        updateExhaustion(contentBelowAnchorHeight: contentBelowAnchorHeight)
        return height
    }

    /// Reconcile a new layout sample. The floor only rises within a session;
    /// the height always tracks `floor - contentBelowAnchor` exactly.
    /// A retiring session is owned by `trim` — floor reconciliation must not
    /// regrow a spacer that is shrink-wrapping away.
    @discardableResult
    public mutating func reconcile(
        viewportHeight: CGFloat,
        bottomChromeClearance: CGFloat,
        anchorTopInset: CGFloat = 0,
        contentBelowAnchorHeight: CGFloat
    ) -> CGFloat {
        guard !isRetiring else { return height }
        guard anchorRowId != nil else {
            height = 0
            runSpaceFloor = 0
            return height
        }
        runSpaceFloor = max(
            runSpaceFloor,
            Self.effectiveRunSpace(
                viewportHeight: viewportHeight,
                bottomChromeClearance: bottomChromeClearance,
                anchorTopInset: anchorTopInset
            )
        )
        height = max(0, runSpaceFloor - Self.valid(contentBelowAnchorHeight))
        updateExhaustion(contentBelowAnchorHeight: contentBelowAnchorHeight)
        return height
    }

    /// The session ended under an active reader gesture: switch to
    /// shrink-wrap retirement instead of an instant collapse.
    public mutating func beginRetiring() {
        guard anchorRowId != nil else { return }
        isRetiring = true
        isExhausted = false
    }

    /// Trim the retiring spacer by the currently scrollable excess below the
    /// viewport bottom (`metrics.distanceFromBottom` when positive). Upward
    /// reading motion trims one-for-one; the spacer never regrows, and the
    /// session clears itself once the blank is fully consumed.
    @discardableResult
    public mutating func trim(scrollableExcessBelowViewport excess: CGFloat) -> CGFloat {
        guard isRetiring, anchorRowId != nil else { return height }
        height = max(0, height - Self.valid(excess))
        if height == 0 {
            reset()
        }
        return height
    }

    public mutating func reset() {
        self = Self()
    }

    private mutating func updateExhaustion(contentBelowAnchorHeight: CGFloat) {
        isExhausted = runSpaceFloor > 0
            && Self.valid(contentBelowAnchorHeight) >= runSpaceFloor
    }

    /// Run space required below the anchored row: the viewport minus the
    /// bottom chrome clearance and the anchor's top inset (the anchored row
    /// sits `anchorTopInset` below the viewport top, so that much less space
    /// is needed underneath it).
    private static func effectiveRunSpace(
        viewportHeight: CGFloat,
        bottomChromeClearance: CGFloat,
        anchorTopInset: CGFloat
    ) -> CGFloat {
        max(
            0,
            valid(viewportHeight)
                - valid(bottomChromeClearance)
                - valid(anchorTopInset)
        )
    }

    private static func valid(_ value: CGFloat) -> CGFloat {
        guard value.isFinite else { return 0 }
        return max(0, value)
    }
}
