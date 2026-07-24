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
/// bottom chrome clearance. Once a session has a real viewport measurement,
/// its height may only shrink. Reply growth therefore consumes blank run space
/// without moving the user row anchored above it.
public struct GaryxSendAnchorFillerState: Equatable, Sendable {
    public private(set) var anchorRowId: String?
    public private(set) var height: CGFloat = 0
    public private(set) var hasMeasuredViewport = false

    public init() {}

    @discardableResult
    public mutating func begin(
        anchorRowId: String,
        viewportHeight: CGFloat,
        bottomChromeClearance: CGFloat,
        contentBelowAnchorHeight: CGFloat
    ) -> CGFloat {
        self.anchorRowId = anchorRowId
        hasMeasuredViewport = Self.valid(viewportHeight) > 0
        height = hasMeasuredViewport
            ? Self.requiredHeight(
                viewportHeight: viewportHeight,
                bottomChromeClearance: bottomChromeClearance,
                contentBelowAnchorHeight: contentBelowAnchorHeight
            )
            : 0
        return height
    }

    /// Reconcile a new layout sample. The first valid viewport establishes an
    /// unmeasured session; every established session is monotonic nonincreasing.
    @discardableResult
    public mutating func reconcile(
        viewportHeight: CGFloat,
        bottomChromeClearance: CGFloat,
        contentBelowAnchorHeight: CGFloat
    ) -> CGFloat {
        guard anchorRowId != nil else {
            height = 0
            hasMeasuredViewport = false
            return height
        }
        let measuredViewport = Self.valid(viewportHeight)
        guard measuredViewport > 0 else { return height }
        let required = Self.requiredHeight(
            viewportHeight: measuredViewport,
            bottomChromeClearance: bottomChromeClearance,
            contentBelowAnchorHeight: contentBelowAnchorHeight
        )
        if hasMeasuredViewport {
            height = min(height, required)
        } else {
            height = required
            hasMeasuredViewport = true
        }
        return height
    }

    public mutating func reset() {
        self = Self()
    }

    private static func requiredHeight(
        viewportHeight: CGFloat,
        bottomChromeClearance: CGFloat,
        contentBelowAnchorHeight: CGFloat
    ) -> CGFloat {
        max(
            0,
            valid(viewportHeight)
                - valid(bottomChromeClearance)
                - valid(contentBelowAnchorHeight)
        )
    }

    private static func valid(_ value: CGFloat) -> CGFloat {
        guard value.isFinite else { return 0 }
        return max(0, value)
    }
}
