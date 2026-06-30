import Foundation

/// Viewport handling for the iOS capsule **full-screen detail** web view
/// (#TASK-1453 problem B).
///
/// The gateway serves capsule HTML with only a CSP meta injected
/// (`inject_csp_meta`); the self-contained card markup carries no viewport meta.
/// A `WKWebView` with no `width=device-width` viewport lays the page out at its
/// ~980pt desktop default and shrinks it to fit the screen, so a centered
/// `max-width` card renders with wide side gutters and stays pinch-zoomable. The
/// detail page should instead behave like a browser rendering a web page: fill
/// the device width and disable user zoom.
///
/// This is a webview-side concern, not a card-authoring one — the same served
/// HTML is also rendered as a deliberately desktop-width, shrunk thumbnail
/// preview, so the viewport must be applied only where the full-screen,
/// fill-the-width behavior is wanted.
public enum GaryxCapsuleViewport {
    /// Fills the device width and disables user zoom.
    public static let mobileViewportMeta =
        #"<meta name="viewport" content="width=device-width, initial-scale=1, maximum-scale=1, user-scalable=no">"#

    /// Returns `html` guaranteed to carry the mobile viewport meta. Inserts it
    /// right after an existing `<head …>` open tag, otherwise prepends it
    /// (mirroring the gateway's CSP injection). HTML that already declares a
    /// viewport is returned unchanged so an author-chosen viewport is respected.
    public static func ensuringMobileViewport(in html: String) -> String {
        guard !declaresViewport(html) else { return html }
        if let headEnd = headOpenTagEnd(in: html) {
            var output = html
            output.insert(contentsOf: mobileViewportMeta, at: headEnd)
            return output
        }
        return mobileViewportMeta + html
    }

    /// Whether the markup already declares a `<meta name="viewport" …>`.
    static func declaresViewport(_ html: String) -> Bool {
        guard let metaRegex = try? NSRegularExpression(
            pattern: #"<meta[^>]*name\s*=\s*["']?viewport["']?"#,
            options: [.caseInsensitive]
        ) else { return false }
        let range = NSRange(html.startIndex..., in: html)
        return metaRegex.firstMatch(in: html, range: range) != nil
    }

    // MARK: Thumbnail capture (#TASK-1478)

    /// Marker id on the injected style so a double-prepare is a no-op.
    static let scrollbarHideStyleID = "garyx-thumbnail-scrollbar-hide"

    /// Style that hides every scrollbar while a capsule is captured into a
    /// thumbnail. A capsule is authored for a full phone screen, so its content
    /// is far taller than the short band the thumbnail captures; WebKit would
    /// otherwise paint a root vertical scrollbar (and any inner `overflow:auto`
    /// container's scrollbar) straight into the static PNG. `::-webkit-scrollbar`
    /// is a universal selector (root + every inner element);
    /// `scrollbar-width:none` covers the standard property. `!important` so an
    /// author's own scrollbar styling can't re-enable it. This only suppresses
    /// the scrollbar chrome — content still overflows and the top band is
    /// captured top-anchored (cover), so the visible crop is unchanged. Kept in
    /// sync with the desktop `capsuleThumbnailScrollbarHidingStyle`.
    public static let scrollbarHidingStyle =
        #"<style id="garyx-thumbnail-scrollbar-hide">html{scrollbar-width:none!important}::-webkit-scrollbar{display:none!important;width:0!important;height:0!important}</style>"#

    /// Returns `html` with the scrollbar-hiding style injected (right after an
    /// existing `<head …>` open tag, otherwise prepended — mirroring
    /// `ensuringMobileViewport`). A no-op when the style is already present.
    public static func hidingScrollbars(in html: String) -> String {
        guard !html.contains(scrollbarHideStyleID) else { return html }
        if let headEnd = headOpenTagEnd(in: html) {
            var output = html
            output.insert(contentsOf: scrollbarHidingStyle, at: headEnd)
            return output
        }
        return scrollbarHidingStyle + html
    }

    /// Prepare served capsule HTML for a one-shot thumbnail capture: guarantee a
    /// device-width viewport and hide scrollbars. Used only by the thumbnail
    /// renderer — the full-screen detail web view keeps its scrollbars. Mirrors
    /// the desktop `prepareThumbnailHtml`.
    public static func preparingForThumbnail(in html: String) -> String {
        hidingScrollbars(in: ensuringMobileViewport(in: html))
    }

    /// The index just after the first `<head …>` open tag, if any.
    private static func headOpenTagEnd(in html: String) -> String.Index? {
        guard let headRegex = try? NSRegularExpression(
            pattern: #"<head\b[^>]*>"#,
            options: [.caseInsensitive]
        ) else { return nil }
        let range = NSRange(html.startIndex..., in: html)
        guard let match = headRegex.firstMatch(in: html, range: range),
              let matchRange = Range(match.range, in: html) else { return nil }
        return matchRange.upperBound
    }
}
