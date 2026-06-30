import Foundation

/// The horizontal transform that makes rendered capsule content fill the
/// thumbnail frame flush-left, with no side gutters and **no injected backing**.
public struct GaryxCapsuleThumbnailFillTransform: Equatable, Sendable {
    /// Uniform scale (both axes — height overflow is handled by the top-anchored
    /// cover crop, so the page is never distorted).
    public let scale: Double
    /// Horizontal shift (points) applied after scaling, top-left origin, so the
    /// content's left edge lands at x = 0.
    public let translateX: Double
}

/// Makes a capsule thumbnail's content fill the render frame horizontally.
///
/// Rendering at the device logical width already makes most capsules fill (an
/// author `max-width` ≥ device width caps to the viewport). The remaining gutter
/// sources are a `max-width` *smaller* than the device width and an un-reset
/// `body { margin }`. For those, the rendered content is measured and the whole
/// page is uniformly scaled (top-left) so the content spans the full width.
///
/// This is the approved fix direction: fill by scaling the page, never by
/// painting a backing color behind centered content.
public enum GaryxCapsuleThumbnailFill {
    /// The fill transform for content measured to occupy `[contentLeft,
    /// contentLeft + contentWidth]` within a `viewportWidth`-wide layout, or
    /// `nil` when the content already fills (no left gutter, within 1px) so the
    /// renderer skips the transform entirely.
    ///
    /// Kept byte-for-byte in sync with the arithmetic in `fillScript`.
    public static func fillTransform(
        contentLeft: Double,
        contentWidth: Double,
        viewportWidth: Double
    ) -> GaryxCapsuleThumbnailFillTransform? {
        guard viewportWidth > 0, contentWidth > 0 else { return nil }
        let left = max(0, contentLeft)
        let scale = viewportWidth / contentWidth
        // Already fills: no scale-up needed and no left gutter to absorb.
        if scale <= 1.005 && left <= 1 { return nil }
        return GaryxCapsuleThumbnailFillTransform(scale: scale, translateX: -left * scale)
    }

    /// JavaScript injected into the rendered page (after layout settles) that
    /// measures the visible content's horizontal extent and applies
    /// `fillTransform` to `document.documentElement`. Self-contained, never
    /// throws, and is a no-op when the content already fills the width.
    ///
    /// The measurement (which needs a live layout) runs in-page; the arithmetic
    /// mirrors `fillTransform` and is cross-engine verified by the Chromium
    /// reproduction harness (Chromium == Electron and matches WKWebView CSS).
    public static let fillScript: String = """
    (function () {
      try {
        if (!document.body) { return; }
        var vw = window.innerWidth;
        if (!(vw > 0)) { return; }
        var minL = Infinity, maxR = -Infinity;
        var els = document.body.querySelectorAll('*');
        for (var i = 0; i < els.length; i++) {
          var el = els[i];
          var r = el.getBoundingClientRect();
          if (r.width <= 0 || r.height <= 0) { continue; }
          var cs = getComputedStyle(el);
          if (cs.visibility === 'hidden' || cs.display === 'none') { continue; }
          if (r.right <= 0 || r.left >= vw) { continue; }
          var l = Math.max(0, r.left);
          var rr = Math.min(vw, r.right);
          if (l < minL) { minL = l; }
          if (rr > maxR) { maxR = rr; }
        }
        if (!isFinite(minL) || !isFinite(maxR)) { return; }
        var left = Math.max(0, minL);
        var width = Math.max(1, maxR - left);
        var scale = vw / width;
        if (scale <= 1.005 && left <= 1) { return; }
        var root = document.documentElement;
        root.style.transformOrigin = 'top left';
        root.style.transform = 'translateX(' + (-left * scale) + 'px) scale(' + scale + ')';
      } catch (e) { /* best-effort fill: leave the page untransformed on error */ }
    })();
    """
}
