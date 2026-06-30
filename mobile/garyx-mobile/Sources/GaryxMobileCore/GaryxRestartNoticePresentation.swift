import Foundation

public struct GaryxRestartNotice: Equatable, Sendable {
    public var message: String

    public init(message: String) {
        self.message = message
    }
}

public enum GaryxRestartNoticePresentation {
    public static let defaultMessage = "Garyx has restarted. Continue your task."

    /// Parse a `<garyx_restarted>…</garyx_restarted>` restart-notice message into
    /// a card model. Returns nil when the text is not a restart notice so callers
    /// fall back to plain rendering. Mirrors `GaryxTaskNotificationPresentation`.
    public static func parse(_ text: String) -> GaryxRestartNotice? {
        guard let body = stripOuterEnvelope(from: text) else { return nil }
        return GaryxRestartNotice(message: body.isEmpty ? defaultMessage : body)
    }

    private static func stripOuterEnvelope(from text: String) -> String? {
        guard let openMatch = firstMatch(#"^\s*<garyx_restarted\b([^>]*)>\s*"#, in: text) else {
            return nil
        }
        let closeTag = "</garyx_restarted>"
        guard let closeRange = text.range(of: closeTag, options: .backwards) else {
            return nil
        }
        guard let openRange = Range(openMatch.range, in: text) else { return nil }
        let openEnd = openRange.upperBound
        guard openEnd <= closeRange.lowerBound else { return nil }
        return text[openEnd..<closeRange.lowerBound]
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func firstMatch(
        _ pattern: String,
        in text: String
    ) -> NSTextCheckingResult? {
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return nil }
        return regex.firstMatch(
            in: text,
            range: NSRange(text.startIndex..<text.endIndex, in: text)
        )
    }
}
