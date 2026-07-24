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

    public init() {}

    /// Start a session for a freshly presented send. An unmeasured viewport
    /// (zero) starts an empty floor; the first real measurement raises it
    /// through `reconcile`.
    @discardableResult
    public mutating func begin(
        anchorRowId: String,
        viewportHeight: CGFloat,
        bottomChromeClearance: CGFloat,
        contentBelowAnchorHeight: CGFloat
    ) -> CGFloat {
        self.anchorRowId = anchorRowId
        runSpaceFloor = Self.effectiveRunSpace(
            viewportHeight: viewportHeight,
            bottomChromeClearance: bottomChromeClearance
        )
        height = max(0, runSpaceFloor - Self.valid(contentBelowAnchorHeight))
        updateExhaustion(contentBelowAnchorHeight: contentBelowAnchorHeight)
        return height
    }

    /// Reconcile a new layout sample. The floor only rises within a session;
    /// the height always tracks `floor - contentBelowAnchor` exactly.
    @discardableResult
    public mutating func reconcile(
        viewportHeight: CGFloat,
        bottomChromeClearance: CGFloat,
        contentBelowAnchorHeight: CGFloat
    ) -> CGFloat {
        guard anchorRowId != nil else {
            height = 0
            runSpaceFloor = 0
            return height
        }
        runSpaceFloor = max(
            runSpaceFloor,
            Self.effectiveRunSpace(
                viewportHeight: viewportHeight,
                bottomChromeClearance: bottomChromeClearance
            )
        )
        height = max(0, runSpaceFloor - Self.valid(contentBelowAnchorHeight))
        updateExhaustion(contentBelowAnchorHeight: contentBelowAnchorHeight)
        return height
    }

    public mutating func reset() {
        self = Self()
    }

    private mutating func updateExhaustion(contentBelowAnchorHeight: CGFloat) {
        isExhausted = runSpaceFloor > 0
            && Self.valid(contentBelowAnchorHeight) >= runSpaceFloor
    }

    private static func effectiveRunSpace(
        viewportHeight: CGFloat,
        bottomChromeClearance: CGFloat
    ) -> CGFloat {
        max(0, valid(viewportHeight) - valid(bottomChromeClearance))
    }

    private static func valid(_ value: CGFloat) -> CGFloat {
        guard value.isFinite else { return 0 }
        return max(0, value)
    }
}
