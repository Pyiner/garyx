import Foundation

internal enum GaryxMobileToolSummaryFormatter {
    internal static func singleLineTruncated(_ text: String, limit: Int) -> String {
        let normalized = text.replacingOccurrences(of: "\r", with: "\n")
            .split(whereSeparator: \.isNewline)
            .map { String($0).trimmingCharacters(in: .whitespacesAndNewlines) }
            .first { !$0.isEmpty } ?? text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard normalized.count > limit else { return normalized }
        let end = normalized.index(normalized.startIndex, offsetBy: max(0, limit - 1))
        return "\(normalized[..<end])…"
    }

    internal static func safeSummary(_ text: String) -> String? {
        let summary = singleLineTruncated(text, limit: 120)
        guard !summary.isEmpty, summary != "{", summary != "[", !summary.hasPrefix("{\"") else {
            return nil
        }
        return summary
    }

    internal static func pathTail(_ text: String) -> String {
        let normalized = text.replacingOccurrences(of: "\\", with: "/")
        let parts = normalized.split(separator: "/").map(String.init)
        guard parts.count > 2 else { return normalized }
        return parts.suffix(2).joined(separator: "/")
    }

    internal static func shellSummary(_ text: String) -> String {
        var normalized = text.trimmingCharacters(in: .whitespacesAndNewlines)
        let launchers = [
            "/bin/bash -lc ",
            "bash -lc ",
            "/bin/sh -lc ",
            "sh -lc ",
            "/bin/zsh -lc ",
            "zsh -lc ",
        ]
        for launcher in launchers where normalized.hasPrefix(launcher) {
            normalized = unwrappedQuotes(String(normalized.dropFirst(launcher.count)))
            break
        }
        normalized = normalized
            .replacingOccurrences(of: #" 2>&1\b"#, with: "", options: .regularExpression)
            .replacingOccurrences(of: #"\s+"#, with: " ", options: .regularExpression)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return singleLineTruncated(normalized, limit: 112)
    }

    private static func unwrappedQuotes(_ text: String) -> String {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.count >= 2,
              let first = trimmed.first,
              let last = trimmed.last,
              (first == "\"" || first == "'"),
              first == last else {
            return trimmed
        }
        return String(trimmed.dropFirst().dropLast()).trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
