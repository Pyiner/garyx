import Foundation

/// One bare file path detected inside rendered markdown text, expressed in
/// character offsets so view code can map it onto an `AttributedString`.
public struct GaryxDetectedFilePathLink: Equatable, Sendable {
    public let characterOffset: Int
    public let characterCount: Int
    public let target: String

    public init(characterOffset: Int, characterCount: Int, target: String) {
        self.characterOffset = characterOffset
        self.characterCount = characterCount
        self.target = target
    }

    public var isAbsolute: Bool {
        target.hasPrefix("/")
    }
}

/// Detects bare file-system paths in plain message text (paths the author did
/// not wrap in a markdown link) so transcript surfaces can make them tappable
/// through the shared file-preview route.
public enum GaryxFilePathLinkDetector {
    public static let linkScheme = "garyx-path"

    /// Absolute (`/a/b/file.ext`) or workspace-relative (`docs/a/file.ext`)
    /// paths. At least one directory separator and a short trailing file
    /// extension are required to keep prose like `and/or` or `TCP/IP` out.
    private static let pathPattern =
        #"(?<![\w/.~-])/?(?:[\w.+@%-]+/)+[\w.+@%-]+\.[A-Za-z0-9]{1,8}\b"#

    private static let pathRegex = try? NSRegularExpression(pattern: pathPattern)

    public static func detect(in text: String) -> [GaryxDetectedFilePathLink] {
        guard !text.isEmpty, text.contains("/"), let pathRegex else { return [] }
        let nsText = text as NSString
        let fullRange = NSRange(location: 0, length: nsText.length)
        var links: [GaryxDetectedFilePathLink] = []
        pathRegex.enumerateMatches(in: text, range: fullRange) { match, _, _ in
            guard let match, let range = Range(match.range, in: text) else { return }
            let target = String(text[range])
            guard isLikelyFilePath(target) else { return }
            links.append(
                GaryxDetectedFilePathLink(
                    characterOffset: text.distance(from: text.startIndex, to: range.lowerBound),
                    characterCount: text.distance(from: range.lowerBound, to: range.upperBound),
                    target: target
                )
            )
        }
        return links
    }

    public static func linkURL(forTarget target: String) -> URL? {
        let trimmed = target.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        var components = URLComponents()
        components.scheme = linkScheme
        components.host = "file"
        components.queryItems = [URLQueryItem(name: "target", value: trimmed)]
        return components.url
    }

    public static func target(from url: URL) -> String? {
        guard url.scheme?.lowercased() == linkScheme,
              let components = URLComponents(url: url, resolvingAgainstBaseURL: false),
              let target = components.queryItems?.first(where: { $0.name == "target" })?.value,
              !target.isEmpty else {
            return nil
        }
        return target
    }

    private static func isLikelyFilePath(_ target: String) -> Bool {
        if target.hasPrefix("/") {
            return true
        }
        // Domain-shaped prefixes (`example.com/a/file.html`) are bare URLs,
        // not workspace-relative paths.
        guard let firstComponent = target.split(separator: "/").first else { return false }
        return !firstComponent.contains(".")
    }
}
