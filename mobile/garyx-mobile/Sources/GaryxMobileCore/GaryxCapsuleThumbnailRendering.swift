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

/// Version of the thumbnail *render output*. Bump whenever the renderer changes
/// how a capsule is turned into a PNG — render viewport width, fill transform,
/// crop, or backing — so every previously cached thumbnail (whose storage token
/// embeds this version) misses and re-renders under the new logic instead of
/// serving a stale image (e.g. an old white-edged render).
///
/// - `1`: original wide (760pt) render viewport with an opaque dark backing.
/// - `2`: device-width (390pt) render + horizontal content-fill, no backing
///   (#TASK-1458) — content fills the frame instead of being letterboxed.
/// - `3`: scrollbars hidden during capture (#TASK-1478) — content taller than
///   the captured band no longer paints a root/inner overflow scrollbar.
public enum GaryxCapsuleThumbnailRenderSchema {
    public static let version = 3
}

/// Cache key for a *rendered* capsule thumbnail image.
///
/// Unlike the HTML text cache (`GaryxCapsuleHTMLCacheKey`, keyed only by
/// `id + revision` because raw HTML is surface-independent), a rendered image is
/// surface-shaped: the gallery (16:10) and chat card (16:9) crop the same
/// capsule differently. So the rendition is part of the key — a bare
/// `id:revision` key would let a 16:10 image be served into a 16:9 card.
///
/// The render-schema version is also part of the key so a renderer change
/// invalidates every old cached image without bumping the capsule's revision.
public struct GaryxCapsuleThumbnailCacheKey: Hashable, Sendable {
    public let id: String
    public let revision: Int
    public let rendition: GaryxCapsuleThumbnailRendition
    public let schemaVersion: Int

    public init(
        id: String,
        revision: Int,
        rendition: GaryxCapsuleThumbnailRendition,
        schemaVersion: Int = GaryxCapsuleThumbnailRenderSchema.version
    ) {
        self.id = id.trimmingCharacters(in: .whitespacesAndNewlines)
        self.revision = revision
        self.rendition = rendition
        self.schemaVersion = schemaVersion
    }

    /// Stable, filesystem-safe storage token, e.g. `<id>.r3.16x10.s3`.
    public var storageToken: String { "\(id).r\(revision).\(rendition.token).s\(schemaVersion)" }
}

/// Pure geometry for rendering a capsule thumbnail snapshot.
///
/// The capsule HTML lays out at a **device-width** `layoutWidth`, so it renders
/// exactly as it would full-screen on a phone — content fills the width instead
/// of being centered with side gutters. (The old wide 760pt viewport let an
/// author `max-width` container sit centered, leaving white side gutters — the
/// #TASK-1458 root cause.) The thumbnail is that full-screen render scaled down.
/// The snapshot captures the top `rendition`-tall band (cover, top-anchored):
/// content taller than the band is cropped at the bottom. There is no injected
/// backing — content fills the frame via `GaryxCapsuleThumbnailFill`.
public struct GaryxCapsuleThumbnailSnapshotPlan: Equatable, Sendable {
    /// Standard device logical width (points) the capsule is laid out into, so
    /// it fills like a phone full-screen render rather than a desktop-wide one.
    public static let deviceLayoutWidth: Double = 390

    /// CSS layout viewport the capsule is rendered into (points).
    public let layoutWidth: Double
    public let layoutHeight: Double
    /// Output image size (pixels) after applying `scale`.
    public let pixelWidth: Double
    public let pixelHeight: Double

    /// Lays out at the device logical width and renders at `scale` so the
    /// downscaled card stays crisp on 2x/3x displays (390×3 = 1170px wide).
    public init(
        rendition: GaryxCapsuleThumbnailRendition,
        layoutWidth: Double = GaryxCapsuleThumbnailSnapshotPlan.deviceLayoutWidth,
        scale: Double = 3
    ) {
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

    /// Evict entries whose on-disk token no longer matches the token the current
    /// schema would produce — i.e. renders from a previous schema version (or a
    /// legacy token with no schema suffix). Run on cache warm so a renderer
    /// change (which bumped `GaryxCapsuleThumbnailRenderSchema.version`) drops
    /// every stale image instead of letting it linger until LRU. Each entry's
    /// `key` is reconstructed from its stored metadata and therefore carries the
    /// *current* schema, so a stored token built under an older schema differs.
    public static func evictingStaleSchema(
        entries: [(token: String, key: GaryxCapsuleThumbnailCacheKey)]
    ) -> (keepTokens: [String], evictTokens: [String]) {
        var keep: [String] = []
        var evict: [String] = []
        for entry in entries {
            if entry.token == entry.key.storageToken {
                keep.append(entry.token)
            } else {
                evict.append(entry.token)
            }
        }
        return (keep, evict)
    }
}
