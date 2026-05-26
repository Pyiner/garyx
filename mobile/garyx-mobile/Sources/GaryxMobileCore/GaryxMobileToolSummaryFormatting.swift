import Foundation

extension String {
    func garyxSingleLineTruncated(limit: Int) -> String {
        let normalized = replacingOccurrences(of: "\r", with: "\n")
            .split(whereSeparator: \.isNewline)
            .map { String($0).trimmingCharacters(in: .whitespacesAndNewlines) }
            .first { !$0.isEmpty } ?? trimmingCharacters(in: .whitespacesAndNewlines)
        guard normalized.count > limit else { return normalized }
        let end = normalized.index(normalized.startIndex, offsetBy: max(0, limit - 1))
        return "\(normalized[..<end])…"
    }

    var garyxSafeToolSummary: String? {
        let summary = garyxSingleLineTruncated(limit: 120)
        guard !summary.isEmpty, summary != "{", summary != "[", !summary.hasPrefix("{\"") else {
            return nil
        }
        return summary
    }

    var garyxPathTail: String {
        let normalized = replacingOccurrences(of: "\\", with: "/")
        let parts = normalized.split(separator: "/").map(String.init)
        guard parts.count > 2 else { return normalized }
        return parts.suffix(2).joined(separator: "/")
    }

    var garyxShellSummary: String {
        var normalized = trimmingCharacters(in: .whitespacesAndNewlines)
        let launchers = [
            "/bin/bash -lc ",
            "bash -lc ",
            "/bin/sh -lc ",
            "sh -lc ",
            "/bin/zsh -lc ",
            "zsh -lc ",
        ]
        for launcher in launchers where normalized.hasPrefix(launcher) {
            normalized = String(normalized.dropFirst(launcher.count)).garyxUnwrappedQuotes
            break
        }
        normalized = normalized
            .replacingOccurrences(of: #" 2>&1\b"#, with: "", options: .regularExpression)
            .replacingOccurrences(of: #"\s+"#, with: " ", options: .regularExpression)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return normalized.garyxSingleLineTruncated(limit: 112)
    }

    private var garyxUnwrappedQuotes: String {
        let trimmed = trimmingCharacters(in: .whitespacesAndNewlines)
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
