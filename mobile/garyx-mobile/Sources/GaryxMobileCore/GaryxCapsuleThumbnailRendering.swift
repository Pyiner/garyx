import Foundation

/// A capsule thumbnail's target shape. The gallery card is 16:10; the chat card
/// is 16:9. Aspect is part of the rendered-image cache key so a snapshot taken
/// for one surface is never served cropped-wrong to the other.
public struct GaryxCapsuleThumbnailRendition: Hashable, Sendable {
    public let aspectWidth: Int
    public let aspectHeight: Int

    public init(aspectWidth: Int, aspectHeight: Int) {
        self.aspectWidth = max(1, aspectWidth)
        self.aspectHeight = max(1, aspectHeight)
    }

    /// Gallery card preview (`GaryxCapsuleGalleryCard`, 16:10).
    public static let gallery = GaryxCapsuleThumbnailRendition(aspectWidth: 16, aspectHeight: 10)
    /// Chat transcript capsule card (`GaryxMobileCapsuleChatCard`, 16:9).
    public static let chatCard = GaryxCapsuleThumbnailRendition(aspectWidth: 16, aspectHeight: 9)

    /// Stable, filesystem-safe token, e.g. `16x10`.
    public var token: String { "\(aspectWidth)x\(aspectHeight)" }

    public var aspectRatio: Double { Double(aspectWidth) / Double(aspectHeight) }
}

/// Cache key for a *rendered* capsule thumbnail image.
///
/// Unlike the HTML text cache (`GaryxCapsuleHTMLCacheKey`, keyed only by
/// `id + revision` because raw HTML is surface-independent), a rendered image is
/// surface-shaped: the gallery (16:10) and chat card (16:9) crop the same
/// capsule differently. So the rendition is part of the key — a bare
/// `id:revision` key would let a 16:10 image be served into a 16:9 card.
public struct GaryxCapsuleThumbnailCacheKey: Hashable, Sendable {
    public let id: String
    public let revision: Int
    public let rendition: GaryxCapsuleThumbnailRendition

    public init(id: String, revision: Int, rendition: GaryxCapsuleThumbnailRendition) {
        self.id = id.trimmingCharacters(in: .whitespacesAndNewlines)
        self.revision = revision
        self.rendition = rendition
    }

    /// Stable, filesystem-safe storage token, e.g. `<id>.r3.16x10`.
    public var storageToken: String { "\(id).r\(revision).\(rendition.token)" }
}

/// Pure geometry for rendering a capsule thumbnail snapshot.
///
/// The capsule HTML lays out at a fixed `layoutWidth` with a device-width
/// viewport injected, so a capsule that declares no viewport does **not** fall
/// back to the ~980px desktop width and leave side gutters (the A2 root cause).
/// The snapshot captures the top `rendition`-tall band (cover, top-anchored):
/// content taller than the band is cropped at the bottom; shorter content shows
/// the renderer's opaque backing color, never the card's translucent fill.
public struct GaryxCapsuleThumbnailSnapshotPlan: Equatable, Sendable {
    /// CSS layout viewport the capsule is rendered into (points).
    public let layoutWidth: Double
    public let layoutHeight: Double
    /// Output image size (pixels) after applying `scale`.
    public let pixelWidth: Double
    public let pixelHeight: Double

    /// Default layout width matches the long-standing thumbnail virtual canvas
    /// (760) — close to the ~760–780 width most capsules are authored for — and
    /// renders at `scale` for crisp downscaling into small cards.
    public init(rendition: GaryxCapsuleThumbnailRendition, layoutWidth: Double = 760, scale: Double = 2) {
        let w = max(1, layoutWidth)
        let s = max(1, scale)
        let h = (w * Double(rendition.aspectHeight) / Double(rendition.aspectWidth)).rounded()
        self.layoutWidth = w
        self.layoutHeight = h
        self.pixelWidth = (w * s).rounded()
        self.pixelHeight = (h * s).rounded()
    }
}

/// Pure invalidation logic for the rendered-thumbnail image cache, mirroring
/// `GaryxCapsuleHTMLCachePruner` but rendition-aware. The disk store owns the
/// keys; these functions decide which to keep so the "did anything get evicted"
/// signal (which bumps the thumbnail cache epoch) is headless-testable.
public enum GaryxCapsuleThumbnailCachePruner {
    /// Keep only keys whose `(id, revision)` is still authoritative. A deleted
    /// capsule (id absent) or a superseded revision drops out — across every
    /// rendition of that capsule.
    public static func pruned(
        keys: [GaryxCapsuleThumbnailCacheKey],
        validCapsules: [GaryxCapsuleSummary]
    ) -> (keep: [GaryxCapsuleThumbnailCacheKey], evict: [GaryxCapsuleThumbnailCacheKey]) {
        var validRevisions: [String: Int] = [:]
        for capsule in validCapsules {
            validRevisions[capsule.id.trimmingCharacters(in: .whitespacesAndNewlines)] = capsule.revision
        }
        var keep: [GaryxCapsuleThumbnailCacheKey] = []
        var evict: [GaryxCapsuleThumbnailCacheKey] = []
        for key in keys {
            if validRevisions[key.id] == key.revision {
                keep.append(key)
            } else {
                evict.append(key)
            }
        }
        return (keep, evict)
    }

    /// Evict every cached rendition/revision of one capsule. A `/serve` 404 means
    /// the whole capsule is gone, so all `(id, *, *)` entries drop.
    public static func evictingCapsule(
        keys: [GaryxCapsuleThumbnailCacheKey],
        capsuleId: String
    ) -> (keep: [GaryxCapsuleThumbnailCacheKey], evict: [GaryxCapsuleThumbnailCacheKey]) {
        let id = capsuleId.trimmingCharacters(in: .whitespacesAndNewlines)
        var keep: [GaryxCapsuleThumbnailCacheKey] = []
        var evict: [GaryxCapsuleThumbnailCacheKey] = []
        for key in keys {
            if key.id == id { evict.append(key) } else { keep.append(key) }
        }
        return (keep, evict)
    }
}
